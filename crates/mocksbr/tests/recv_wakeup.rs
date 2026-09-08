//! Deterministic regression coverage for the lost-wakeup race described
//! in Task 7's brief and its override.
//!
//! An earlier version of this file tried to catch the race by racing
//! `recv` against `send` on a multi-threaded runtime, 2000-50,000 times
//! per run, hoping scheduling luck would land a `send` inside the
//! buggy window. Independent review reconstructed the brief's racy
//! `Mutex<VecDeque> + Notify` version and ran that test against it —
//! zero failures across 64,000+ iterations on a 14-core machine. The
//! buggy window is a handful of CPU instructions with no `.await`
//! inside it, so nothing can preempt a thread there short of a genuine
//! cross-core race landing at exactly the wrong nanosecond; iteration
//! count and worker-thread count do not reliably buy odds of hitting
//! it. That version is gone. See the task-7-report.md "Fix round 1"
//! section for the full reconciliation of the discrepancy.
//!
//! What replaces it:
//!
//! - [`racy_pattern_loses_a_push_landed_in_the_gap`] and
//!   [`enable_before_check_pattern_does_not_lose_a_push`] reproduce the
//!   exact bug *deterministically*, on a single-threaded runtime, using
//!   a local, test-only copy of the buggy pattern instrumented with an
//!   explicit rendezvous hook that pauses execution precisely between
//!   the empty-queue check and the `Notify` registration — forcing the
//!   interleaving instead of hoping for it. They don't exercise
//!   `MockTransport` (which no longer contains this pattern at all —
//!   see below); they exist to prove, on demand and without scheduler
//!   luck, that the pattern the brief specified is genuinely broken,
//!   and that the override's suggested `enable()`-first fix genuinely
//!   isn't.
//! - [`recv_started_before_send_still_receives_the_reply`] is a plain
//!   correctness test against the real `MockTransport`: `recv` is
//!   confirmed genuinely parked (via `yield_now`, deterministic on the
//!   single-threaded test runtime — no multi-threading, no loop)
//!   before `send` runs. It is **not** a race regression guard by
//!   itself: `MockTransport` is built on `tokio::sync::mpsc`, whose
//!   `Receiver` checks its queue and registers the waker as one atomic
//!   step inside the channel implementation, so there is no gap here
//!   left for any test, deterministic or not, to land a message inside
//!   of. This test instead documents and guards the ordinary contract
//!   ("a reply sent after recv is waiting still arrives").
//! - [`close_wakes_a_pending_recv`] closes a small parity gap flagged
//!   in review: go-udap's `MockTransport.Close` wakes a blocked `Recv`
//!   immediately; the brief's trivial `close` did not.

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::Mac;
use udap::protocol::{ADDR_TYPE_ETH, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
use udap::transport::Transport;

fn adv_discovery_request() -> Vec<u8> {
    Packet {
        dst_broadcast: 1,
        dst_type: ADDR_TYPE_ETH,
        dst_address: Mac::ZERO,
        src_broadcast: 0,
        src_type: ADDR_TYPE_ETH,
        src_address: Mac::ZERO,
        sequence: 1,
        udap_type: UDAP_TYPE_UCP,
        ucp_flags: 0x01,
        uap_class: UAP_CLASS_UCP,
        ucp_method: method::ADV_DISC,
    }
    .to_bytes()
    .to_vec()
}

/// A minimal, test-only reproduction of the brief's `Mutex<VecDeque> +
/// Notify` pattern, instrumented with a rendezvous (`at_gap` /
/// `proceed`) that lets a test pause execution deterministically at
/// the exact point a real scheduler race would need to hit by luck:
/// immediately after the queue is observed empty, before the `Notify`
/// registration that would let a subsequent `push` wake this waiter.
///
/// `at_gap` and `proceed` are `notify_one`, not `notify_waiters`:
/// `notify_one` buffers a single permit for a call that hasn't started
/// waiting yet, so the rendezvous itself can't suffer the same
/// lost-wakeup problem it's built to demonstrate.
struct HookedQueue {
    pending: std::sync::Mutex<std::collections::VecDeque<u32>>,
    notify: tokio::sync::Notify,
    at_gap: tokio::sync::Notify,
    proceed: tokio::sync::Notify,
}

impl HookedQueue {
    fn new() -> Self {
        HookedQueue {
            pending: std::sync::Mutex::new(std::collections::VecDeque::new()),
            notify: tokio::sync::Notify::new(),
            at_gap: tokio::sync::Notify::new(),
            proceed: tokio::sync::Notify::new(),
        }
    }

    fn pop(&self) -> Option<u32> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }

    fn push(&self, value: u32) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(value);
        self.notify.notify_waiters();
    }

    /// The brief's `MockTransport::recv` loop body, verbatim in
    /// structure, with the test rendezvous inserted after the empty
    /// check and before the `notified()` registration — exactly the
    /// gap the override identified.
    async fn recv_racy(&self) -> u32 {
        loop {
            if let Some(value) = self.pop() {
                return value;
            }
            self.at_gap.notify_one();
            self.proceed.notified().await;
            self.notify.notified().await;
        }
    }

    /// The override's suggested fix: register interest with
    /// `enable()` *before* checking the queue. The same rendezvous is
    /// inserted after the check (mirroring the racy version above) to
    /// prove a `push` landed there is no longer lost — registration
    /// already happened.
    async fn recv_enabled(&self) -> u32 {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if let Some(value) = self.pop() {
                return value;
            }
            self.at_gap.notify_one();
            self.proceed.notified().await;
            notified.await;
        }
    }
}

#[tokio::test]
async fn racy_pattern_loses_a_push_landed_in_the_gap() {
    let queue = Arc::new(HookedQueue::new());
    let recv_queue = Arc::clone(&queue);
    let recv_task = tokio::spawn(async move { recv_queue.recv_racy().await });

    // Deterministically wait until `recv_racy` has observed the queue
    // empty and is parked at the rendezvous, then land the push in the
    // exact gap and release it -- forcing the interleaving instead of
    // hoping a scheduler race produces it.
    queue.at_gap.notified().await;
    queue.push(42);
    queue.proceed.notify_one();

    let outcome = tokio::time::timeout(Duration::from_millis(200), recv_task).await;
    assert!(
        outcome.is_err(),
        "expected the brief's racy pattern to lose the push and hang forever"
    );
}

#[tokio::test]
async fn enable_before_check_pattern_does_not_lose_a_push() {
    let queue = Arc::new(HookedQueue::new());
    let recv_queue = Arc::clone(&queue);
    let recv_task = tokio::spawn(async move { recv_queue.recv_enabled().await });

    queue.at_gap.notified().await;
    queue.push(42);
    queue.proceed.notify_one();

    let outcome = tokio::time::timeout(Duration::from_millis(200), recv_task)
        .await
        .expect("the enable()-first pattern must not lose a push landed after registration");
    assert_eq!(outcome.unwrap(), 42);
}

#[tokio::test]
async fn recv_started_before_send_still_receives_the_reply() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = Arc::new(mocksbr::MockTransport::new(network));
    let cancel = CancellationToken::new();

    let recv_transport = Arc::clone(&transport);
    let recv_cancel = cancel.clone();
    let recv_task = tokio::spawn(async move { recv_transport.recv(&recv_cancel).await });

    // Single-threaded runtime: yielding once hands control to
    // `recv_task`, which runs until it genuinely parks awaiting the
    // channel (there is nothing queued yet), then hands control back
    // here. Deterministic -- no multi-threading, no iteration count.
    tokio::task::yield_now().await;

    transport.send(&adv_discovery_request()).await.unwrap();

    let (reply, _src) = tokio::time::timeout(Duration::from_millis(200), recv_task)
        .await
        .expect("recv() did not resolve after send()")
        .unwrap()
        .unwrap();
    let (packet, _) = Packet::from_bytes(&reply).unwrap();
    assert_eq!(packet.src_address.to_string(), "00:04:20:00:00:01");
}

#[tokio::test]
async fn close_wakes_a_pending_recv() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = Arc::new(mocksbr::MockTransport::new(network));
    let cancel = CancellationToken::new();

    let recv_transport = Arc::clone(&transport);
    let recv_task = tokio::spawn(async move { recv_transport.recv(&cancel).await });

    tokio::task::yield_now().await; // let recv_task genuinely park first
    transport.close().await.unwrap();

    let result = tokio::time::timeout(Duration::from_millis(200), recv_task)
        .await
        .expect("close() did not wake the pending recv()")
        .unwrap();
    assert!(matches!(
        result,
        Err(udap::transport::TransportError::Cancelled)
    ));
}
