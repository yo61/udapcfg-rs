//! The fault-injection knobs, driven through the real client.

use mocksbr::{DeviceConfig, MockTransport, Network, Op};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::{Device, Mac, Session, ops};

/// Bounds an operation so a silent device fails rather than hanging.
///
/// A macro, not a function: `clippy.toml` exempts `expect` only inside
/// frames carrying `#[test]`/`#[tokio::test]`.
macro_rules! within {
    ($fut:expr) => {
        tokio::time::timeout(Duration::from_secs(5), $fut)
            .await
            .expect("operation blocked: a missing reply is a failure, not a hang")
    };
}

const MAC: [u8; 6] = [0x00, 0x04, 0x20, 0x16, 0x17, 0x18];

fn fixture_with(configure: impl FnOnce(&mut DeviceConfig)) -> (Session, Device) {
    let mac = Mac::from_bytes(MAC);
    let mut cfg = DeviceConfig::default_with_mac(mac);
    configure(&mut cfg);
    let session = Session::new(Box::new(MockTransport::new(Arc::new(Network::new(vec![
        cfg,
    ])))));
    let device = Device {
        mac,
        ..Device::default()
    };
    (session, device)
}

/// A well-formed advanced-discovery request, for tests that drive
/// `Network::receive` directly rather than through a client.
fn discovery_request() -> Vec<u8> {
    use udap::protocol::{ADDR_TYPE_ETH, FLAG_REQUEST, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
    udap::Packet {
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

#[tokio::test]
async fn fail_on_selects_a_single_operation() {
    // What error_reply could not express: one operation fails while its
    // neighbour still works.
    let (session, device) = fixture_with(|cfg| cfg.fail_on = vec![Op::GetIp]);

    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect_err("get_ip was configured to fail");

    within!(ops::getuuid::get_uuid(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_uuid was not configured to fail");
}

#[tokio::test]
async fn the_default_failure_message_names_the_operation() {
    // go-udap's wording, and it must name the requested op rather than a
    // fixed one — a device failing only get_ip must not say "reset".
    let (session, device) = fixture_with(|cfg| cfg.fail_on = vec![Op::GetIp]);
    let err = within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect_err("configured to fail");
    assert_eq!(
        err.to_string(),
        "device 00:04:20:16:17:18 error: mocksbr: configured to fail getip"
    );
}

#[tokio::test]
async fn failing_nothing_leaves_every_operation_working() {
    let (session, device) = fixture_with(|_| {});
    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("no failure configured");
}

#[tokio::test]
async fn an_unreachable_device_answers_nothing() {
    let (session, device) = fixture_with(|cfg| cfg.unreachable = true);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = within!(ops::getip::get_ip(&session, &cancel, &device))
        .expect_err("an unreachable device never replies");
    assert!(
        matches!(err, udap::OpError::Recv(_)),
        "the wait ends by cancellation, not by a device error: {err}"
    );
}

#[tokio::test]
async fn an_unreachable_device_is_not_discoverable() {
    // Unreachable means unreachable: unlike fail_on, it suppresses the
    // discovery reply too, because the device is off the network rather
    // than refusing a request.
    let mut cfg = DeviceConfig::default_with_mac(Mac::from_bytes(MAC));
    cfg.unreachable = true;
    let network = Network::new(vec![cfg]);
    assert!(
        network.receive(&discovery_request()).is_empty(),
        "an unreachable device must not answer discovery"
    );
}

#[tokio::test]
async fn drop_get_ip_silences_only_get_ip() {
    let (session, device) = fixture_with(|cfg| cfg.drop_get_ip = true);
    let cancel = CancellationToken::new();
    cancel.cancel();

    within!(ops::getip::get_ip(&session, &cancel, &device)).expect_err("get_ip is dropped");

    // A fresh token: the first is already cancelled.
    within!(ops::getuuid::get_uuid(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_uuid is unaffected");
}

#[tokio::test]
async fn drop_get_uuid_silences_only_get_uuid() {
    let (session, device) = fixture_with(|cfg| cfg.drop_get_uuid = true);
    let cancel = CancellationToken::new();
    cancel.cancel();

    within!(ops::getuuid::get_uuid(&session, &cancel, &device)).expect_err("get_uuid is dropped");

    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_ip is unaffected");
}
