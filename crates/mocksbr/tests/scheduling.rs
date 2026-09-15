//! Transport-level timing: what `DeviceConfig.slow` does to the wire.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use udap::Mac;
use udap::protocol::{ADDR_TYPE_ETH, FLAG_REQUEST, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
use udap::transport::Transport;

const MAC: [u8; 6] = [0x00, 0x04, 0x20, 0x16, 0x17, 0x18];

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
        ucp_flags: FLAG_REQUEST,
        uap_class: UAP_CLASS_UCP,
        ucp_method: method::ADV_DISC,
    }
    .to_bytes()
    .to_vec()
}

fn transport_with_slow(slow: Duration) -> mocksbr::MockTransport {
    let mut cfg = mocksbr::DeviceConfig::default_with_mac(Mac::from_bytes(MAC));
    cfg.slow = slow;
    mocksbr::MockTransport::new(Arc::new(mocksbr::Network::new(vec![cfg])))
}

/// T5. Asserts the reply is queued *before `send` returns*, not merely
/// that it arrives at time zero.
///
/// `elapsed == ZERO` would not discriminate: a spawned task with a zero
/// sleep is still polled before the runtime auto-advances, so the reply
/// lands at 0ns either way. A zero-duration timeout is a "ready right
/// now?" probe, because `tokio::time::timeout` polls the inner future
/// before consulting its deadline.
#[tokio::test(start_paused = true)]
async fn a_reply_with_no_delay_is_queued_before_send_returns() {
    let transport = transport_with_slow(Duration::ZERO);
    transport.send(&adv_discovery_request()).await.unwrap();

    tokio::time::timeout(Duration::ZERO, transport.recv(&CancellationToken::new()))
        .await
        .expect("a device with no delay has its reply queued when send returns")
        .unwrap();
}

/// A slow device's reply is NOT queued synchronously.
#[tokio::test(start_paused = true)]
async fn a_slow_reply_is_not_queued_when_send_returns() {
    let transport = transport_with_slow(Duration::from_millis(50));
    transport.send(&adv_discovery_request()).await.unwrap();

    tokio::time::timeout(Duration::ZERO, transport.recv(&CancellationToken::new()))
        .await
        .expect_err("a slow device's reply must not be available yet");
}

/// The delay is real, and exact under virtual time.
#[tokio::test(start_paused = true)]
async fn a_slow_reply_arrives_after_exactly_its_delay() {
    const SLOW: Duration = Duration::from_millis(50);
    let transport = transport_with_slow(SLOW);
    let start = Instant::now();
    transport.send(&adv_discovery_request()).await.unwrap();
    transport.recv(&CancellationToken::new()).await.unwrap();
    assert_eq!(start.elapsed(), SLOW);
}
