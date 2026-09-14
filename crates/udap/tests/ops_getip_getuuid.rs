//! `get_ip` and `get_uuid` round-tripped against `mocksbr`.
//!
//! The decoders have their own unit tests; these exercise the wire path
//! — header, send, and the reply matching in `Session`.

use mocksbr::{DeviceConfig, MockTransport, Network};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::{Device, Mac, Session, ops};

/// Bounds an operation so a missing reply fails the test rather than
/// hanging the suite.
///
/// `MockTransport::recv` blocks until a packet arrives or the token is
/// cancelled, and these tests pass a token that never fires — so a
/// regression that stops the device replying would otherwise wedge CI
/// instead of reporting. Found by mutation-testing: pointing `get_ip` at
/// the wrong MAC hung for ten minutes rather than failing.
///
/// A macro, not a function: `clippy.toml` exempts `expect` only inside
/// frames carrying `#[test]`/`#[tokio::test]`, so the same code in a
/// plain helper trips `expect_used`. Expanding at the call site puts it
/// in the test's own frame.
macro_rules! within {
    ($fut:expr) => {
        tokio::time::timeout(Duration::from_secs(5), $fut)
            .await
            .expect("operation blocked: a missing reply is a failure, not a hang")
    };
}

fn device(mac: Mac) -> Device {
    Device {
        mac,
        ..Device::default()
    }
}

fn session_for(macs: &[Mac]) -> Session {
    let network = Arc::new(Network::new(
        macs.iter()
            .copied()
            .map(DeviceConfig::default_with_mac)
            .collect(),
    ));
    Session::new(Box::new(MockTransport::new(network)))
}

#[tokio::test]
async fn get_ip_round_trips_against_mocksbr() {
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let session = session_for(&[mac]);

    let config = within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device(mac)
    ))
    .expect("the mock answers get_ip");

    // A setup-mode device reports unspecified addresses, which render as
    // dashes rather than 0.0.0.0 — see netconfig's or_dash.
    assert_eq!(config.to_string(), "IP:      -\nSubnet:  -\nGateway: -");
}

#[tokio::test]
async fn get_uuid_round_trips_against_mocksbr() {
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let session = session_for(&[mac]);

    let uuid = within!(ops::getuuid::get_uuid(
        &session,
        &CancellationToken::new(),
        &device(mac)
    ))
    .expect("the mock answers get_uuid");

    assert_eq!(uuid, "0123456789abcdeffedcba9876543210");
    assert!(
        uuid.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hex must be lowercase, matching go-udap's hex.EncodeToString"
    );
}

#[tokio::test]
async fn a_directed_request_names_the_device_it_is_for() {
    // UDAP always sends to the limited broadcast, so every device on the
    // segment receives a directed request and decides for itself whether
    // it is the addressee. mocksbr models that: it answers only when
    // dst_address matches. So this passes only if `header` put the right
    // MAC in the packet — with two devices present, the wrong MAC gets
    // silence rather than the wrong answer.
    //
    // Session's own MAC filter is covered by its unit test
    // (`wait_for_reply_skips_other_devices`); it is not what this
    // exercises, because the unasked device never replies.
    let wanted = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let other = Mac::from_bytes([0x00, 0x04, 0x20, 0x99, 0x99, 0x99]);
    let session = session_for(&[other, wanted]);

    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device(wanted)
    ))
    .expect("the addressed device answers");
}
