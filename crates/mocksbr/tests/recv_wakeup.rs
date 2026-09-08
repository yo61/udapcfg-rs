//! Regression test for the lost-wakeup race in `MockTransport::recv`.
//!
//! `Notify::notify_waiters()` wakes only tasks that are already
//! registered; a `send` that lands between `recv`'s queue-check and its
//! registration is silently dropped. This test drives the transport
//! through exactly that ordering — `recv` starts first, genuinely has
//! nothing queued, and only then does `send` land — on a multi-threaded
//! runtime so the two really do run concurrently on separate OS
//! threads. It's looped and timed out per iteration rather than relying
//! on one shot: the buggy window is a handful of CPU instructions wide,
//! so a single attempt has very low odds of landing inside it, and
//! without the timeout a lost wakeup hangs forever instead of failing.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recv_started_before_send_still_receives_the_reply() {
    for attempt in 0..2000 {
        let network = Arc::new(mocksbr::Network::with_auto_devices(1));
        let transport = Arc::new(mocksbr::MockTransport::new(network));
        let cancel = CancellationToken::new();

        let recv_transport = Arc::clone(&transport);
        let recv_cancel = cancel.clone();
        let recv_task = tokio::spawn(async move { recv_transport.recv(&recv_cancel).await });

        // No delay: `send` races `recv` on separate OS threads. A
        // correct implementation cannot lose the wakeup regardless of
        // how this interleaves; the brief's racy version occasionally
        // can, in which case the timeout below turns the hang into a
        // clean failure instead of wedging the whole test run.
        transport.send(&adv_discovery_request()).await.unwrap();

        let outcome = tokio::time::timeout(Duration::from_millis(500), recv_task).await;
        assert!(outcome.is_ok(), "attempt {attempt}: recv() lost its wakeup");
        let (reply, _src) = outcome.unwrap().unwrap().unwrap();
        let (packet, _) = Packet::from_bytes(&reply).unwrap();
        assert_eq!(packet.src_address.to_string(), "00:04:20:00:00:01");
    }
}
