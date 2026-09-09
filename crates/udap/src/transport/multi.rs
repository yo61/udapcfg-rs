//! Composes several transports: send fans out, recv merges.
//!
//! Used by `--all-interfaces`, one `UdpTransport` per usable interface.
//! `Client` cannot tell the difference — both satisfy `Transport`.
//!
//! `recv` merges children with [`select_all`]: each call builds one fresh
//! `recv` future per child and returns whichever settles first. Nothing is
//! pre-fetched or buffered between calls, which rules out the M2 mock
//! transport's lost-wakeup bug by construction rather than by careful
//! bookkeeping. That bug was a queue checked for a pending item, and only
//! *then* had the receiving task's waker registered — a `send` landing in
//! that gap between the check and the registration was missed forever.
//! The failure mode requires exactly that shape: a persistent queue with a
//! check-then-register step done by hand. `select_all` has neither. There
//! is no queue for a reply to land in unobserved: every child's `recv` is
//! polled by a task that is, at that instant, also polling `select_all`
//! itself, so "checked" and "registered as waiting" are the same poll —
//! there is no gap between them for a reply to land in.
//!
//! The alternative (spawning one long-lived pump task per child, feeding
//! a shared `mpsc` channel) was considered and rejected: the children
//! live behind `&self`, so they cannot be moved into `tokio::spawn`ed
//! tasks without also wrapping them in `Arc` and adding a `Once` to
//! start the pumps lazily and a token to stop them on `close` — real
//! state whose lifecycle has to be gotten right. `select_all` needs none
//! of it: an empty `Vec<Box<dyn Transport>>` behind a shared reference is
//! enough.
//!
//! A child whose `recv` returns a real error (a downed VPN interface
//! mid-discovery, say) is retired rather than allowed to end the whole
//! fan-out: `multi_transport.go`'s `pumpChild` logs it at warn and lets
//! that one pump exit, leaving every other child's pump feeding the
//! merged channel. `recv` here matches that by tracking which children
//! have errored and excluding them from the next `select_all`, looping
//! rather than returning, so one bad interface cannot take discovery
//! down with it. `TransportError::Cancelled` is not this kind of
//! failure — every child shares the caller's `cancel` token, so a child
//! reporting `Cancelled` means the whole operation was cancelled, and
//! that propagates immediately without retiring anything. Once every
//! child has retired, `recv` waits on `cancel` rather than returning an
//! error, matching `Recv`'s merged channel simply never yielding again
//! and leaving the caller's own context deadline to end the wait.

use crate::transport::{Transport, TransportError};
use async_trait::async_trait;
use futures_util::future::select_all;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Fans a single logical transport out across several children.
///
/// Used by `--all-interfaces`: one [`UdpTransport`](super::UdpTransport)
/// per usable network interface, composed here so `Client` sees one
/// `Transport` regardless of how many sockets back it.
pub struct MultiTransport {
    children: Vec<Box<dyn Transport>>,
    /// One flag per entry in `children`, set once that child's `recv`
    /// has reported a real (non-cancellation) error. Retirement only
    /// affects `recv` — `send` still tries every child every time,
    /// matching `multi_transport.go`, where retirement is a `pumpChild`
    /// concept `Send` never consults.
    retired: Vec<AtomicBool>,
}

impl MultiTransport {
    /// Composes `children` into one fan-out transport.
    ///
    /// An empty `children` is accepted (so this constructor never
    /// fails): `send` then has nothing to fan out to, and `recv` waits
    /// on its `cancel` token exactly as it would once every child had
    /// retired. Callers that build children from interface enumeration —
    /// [`Client::for_all_interfaces`](crate::Client::for_all_interfaces)
    /// — reject an empty list themselves so the error they surface can
    /// say "no usable interfaces".
    #[must_use]
    pub fn new(children: Vec<Box<dyn Transport>>) -> Self {
        let retired = children.iter().map(|_| AtomicBool::new(false)).collect();
        MultiTransport { children, retired }
    }
}

#[async_trait]
impl Transport for MultiTransport {
    /// Sends `packet` on every child. Succeeds if any child succeeded;
    /// fails only if every child did. Per-child failures are logged at
    /// warn, matching `multi_transport.go`'s `Send`.
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError> {
        let mut failures = Vec::new();
        let mut successes = 0usize;
        for (index, child) in self.children.iter().enumerate() {
            match child.send(packet).await {
                Ok(()) => successes += 1,
                Err(e) => {
                    warn!(child = index, error = %e, "MultiTransport: child send failed");
                    failures.push(e.to_string());
                }
            }
        }
        if successes == 0 {
            return Err(TransportError::Io(std::io::Error::other(format!(
                "all children failed: {failures:?}"
            ))));
        }
        Ok(())
    }

    /// Returns the next packet from whichever live child produces one
    /// first, or the cancellation error if `cancel` fires first — see
    /// the module docs for why this cannot lose or duplicate a reply,
    /// and for how a child that errors is retired rather than ending
    /// the fan-out.
    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
        loop {
            let live: Vec<usize> = (0..self.children.len())
                .filter(|&i| !self.retired[i].load(Ordering::SeqCst))
                .collect();
            if live.is_empty() {
                cancel.cancelled().await;
                return Err(TransportError::Cancelled);
            }
            let futures = live.iter().map(|&i| self.children[i].recv(cancel));
            let (result, winner, _still_pending) = select_all(futures).await;
            match result {
                Ok(packet) => return Ok(packet),
                Err(TransportError::Cancelled) => return Err(TransportError::Cancelled),
                Err(e) => {
                    let child_index = live[winner];
                    warn!(
                        child = child_index,
                        error = %e,
                        "MultiTransport: child recv failed, retiring"
                    );
                    self.retired[child_index].store(true, Ordering::SeqCst);
                }
            }
        }
    }

    /// Closes every child. Returns the first error, if any, matching
    /// `multi_transport.go`'s `Close`.
    async fn close(&self) -> Result<(), TransportError> {
        let mut first_err = None;
        for child in &self.children {
            if let Err(e) = child.close().await
                && first_err.is_none()
            {
                first_err = Some(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Chatty {
        sends: Arc<AtomicUsize>,
        reply: Vec<u8>,
        fail_send: bool,
        fail_recv: bool,
    }

    #[async_trait]
    impl Transport for Chatty {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.fail_send {
                return Err(TransportError::Io(std::io::Error::other("nope")));
            }
            Ok(())
        }
        async fn recv(
            &self,
            cancel: &CancellationToken,
        ) -> Result<(Vec<u8>, String), TransportError> {
            if self.fail_recv {
                return Err(TransportError::Io(std::io::Error::other("recv boom")));
            }
            if self.reply.is_empty() {
                cancel.cancelled().await;
                return Err(TransportError::Cancelled);
            }
            Ok((self.reply.clone(), "10.0.0.1".to_owned()))
        }
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    fn child(sends: &Arc<AtomicUsize>, reply: &[u8], fail_send: bool) -> Box<dyn Transport> {
        Box::new(Chatty {
            sends: Arc::clone(sends),
            reply: reply.to_vec(),
            fail_send,
            fail_recv: false,
        })
    }

    /// A child whose `recv` always returns a plain I/O error — the path
    /// `Chatty`'s original two recv behaviours (an immediate reply, or
    /// blocking until cancelled) cannot reach, and the one this round's
    /// fix is about: a real per-child failure must retire that child,
    /// not end the whole fan-out.
    fn erroring_child(sends: &Arc<AtomicUsize>) -> Box<dyn Transport> {
        Box::new(Chatty {
            sends: Arc::clone(sends),
            reply: Vec::new(),
            fail_send: false,
            fail_recv: true,
        })
    }

    #[tokio::test]
    async fn send_fans_out_to_every_child() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![
            child(&sends, b"", false),
            child(&sends, b"", false),
            child(&sends, b"", false),
        ]);
        m.send(b"x").await.expect("send");
        assert_eq!(sends.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn send_succeeds_when_any_child_succeeds() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", false)]);
        assert!(m.send(b"x").await.is_ok(), "one success is enough");
    }

    #[tokio::test]
    async fn send_fails_only_when_every_child_fails() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", true)]);
        assert!(m.send(b"x").await.is_err(), "all failed, so send must fail");
    }

    #[tokio::test]
    async fn recv_merges_replies_from_children() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"reply-a", false)]);
        let cancel = CancellationToken::new();
        let (pkt, src) = m.recv(&cancel).await.expect("a merged reply");
        assert_eq!(pkt, b"reply-a");
        assert_eq!(src, "10.0.0.1");
    }

    #[tokio::test]
    async fn recv_returns_cancelled_when_the_token_fires() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", false)]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = m.recv(&cancel).await.expect_err("must not block");
        assert!(matches!(err, TransportError::Cancelled));
    }

    /// One child errors, another has a reply: the error must not abort
    /// the fan-out. Whichever child's future `select_all` happens to
    /// settle first, `recv`'s retry loop must retire the erroring one
    /// and come back with the healthy reply rather than surfacing the
    /// error to the caller.
    #[tokio::test]
    async fn recv_retires_an_erroring_child_and_returns_the_healthy_reply() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![
            erroring_child(&sends),
            child(&sends, b"reply-a", false),
        ]);
        let cancel = CancellationToken::new();
        let (pkt, src) = m
            .recv(&cancel)
            .await
            .expect("the healthy child's reply, not the other child's error");
        assert_eq!(pkt, b"reply-a");
        assert_eq!(src, "10.0.0.1");
    }

    /// One child errors, another blocks until cancelled: `recv` must
    /// wait rather than returning the error immediately (proved with a
    /// short timeout while `cancel` is still live), and cancellation
    /// must still resolve it once fired.
    #[tokio::test]
    async fn recv_waits_past_an_erroring_child_for_a_blocking_one_to_be_cancelled() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![erroring_child(&sends), child(&sends, b"", false)]);
        let cancel = CancellationToken::new();

        let mut recv_fut = m.recv(&cancel);
        let too_soon = tokio::time::timeout(Duration::from_millis(20), &mut recv_fut).await;
        assert!(
            too_soon.is_err(),
            "recv resolved before cancellation, despite a live blocking child"
        );

        cancel.cancel();
        let err = recv_fut
            .await
            .expect_err("must resolve once cancelled, not with the retired child's error");
        assert!(matches!(err, TransportError::Cancelled));
    }

    /// Every child errors: `recv` must wait for cancellation rather than
    /// returning the last child's error, matching `Recv`'s merged
    /// channel simply never yielding again once every pump has exited.
    #[tokio::test]
    async fn recv_waits_for_cancellation_once_every_child_has_retired() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![erroring_child(&sends), erroring_child(&sends)]);
        let cancel = CancellationToken::new();

        let mut recv_fut = m.recv(&cancel);
        let too_soon = tokio::time::timeout(Duration::from_millis(20), &mut recv_fut).await;
        assert!(
            too_soon.is_err(),
            "recv resolved with an error instead of waiting once every child had retired"
        );

        cancel.cancel();
        let err = recv_fut.await.expect_err("must resolve once cancelled");
        assert!(matches!(err, TransportError::Cancelled));
    }
}
