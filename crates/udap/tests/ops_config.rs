//! `get`, `get_all` and `reset` against `mocksbr`.

use mocksbr::{DeviceConfig, MockTransport, Network};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::{Device, Mac, Session, ops};

/// Bounds an operation so a missing reply fails rather than hanging.
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

fn fixture() -> (Session, Device) {
    let mac = Mac::from_bytes(MAC);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(network)));
    let device = Device {
        mac,
        ..Device::default()
    };
    (session, device)
}

#[tokio::test]
async fn get_returns_only_the_parameters_asked_for() {
    let (session, device) = fixture();
    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_channel", "lan_ip_mode"]
    ))
    .expect("the mock answers get_data");

    assert_eq!(values.len(), 2, "exactly the two requested");
    assert_eq!(
        values.get("wireless_channel").map(Vec::as_slice),
        Some(b"6".as_slice())
    );
}

#[tokio::test]
async fn get_skips_a_name_the_parameter_table_does_not_know() {
    // go-udap warns and continues rather than failing the request.
    let (session, device) = fixture();
    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_channel", "not_a_real_parameter"]
    ))
    .expect("an unknown name must not fail the request");
    assert_eq!(values.len(), 1, "only the known parameter is requested");
}

#[tokio::test]
async fn get_refuses_a_device_with_no_mac() {
    let (session, _) = fixture();
    let err = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &Device::default(),
        &["wireless_channel"]
    ))
    .expect_err("a zero MAC cannot be addressed");
    assert_eq!(
        err.to_string(),
        "cannot build GetData packet: device has zero MAC address"
    );
}

#[tokio::test]
async fn get_all_writes_into_the_device() {
    // The mutation IS the return channel: get_all returns only ().
    let (session, mut device) = fixture();
    assert!(device.parameters.is_empty(), "starts empty");

    within!(ops::config::get_all(
        &session,
        &CancellationToken::new(),
        &mut device
    ))
    .expect("get_all against the mock succeeds");

    assert!(
        !device.parameters.is_empty(),
        "get_all returns (), so the device is where the values land"
    );
    assert_eq!(
        device.parameters.get("wireless_channel").map(Vec::as_slice),
        Some(b"6".as_slice()),
        "factory default for wireless_channel"
    );
}

#[tokio::test]
async fn get_all_drops_stale_offset_entries() {
    // parse_response emits offset_NNN keys for offsets the table does not
    // know. go-udap clears them before merging, so a repeated get_all on
    // a consumer spanning firmware variations does not grow without bound.
    let (session, mut device) = fixture();
    device
        .parameters
        .insert("offset_999".to_owned(), b"stale".to_vec());
    // Not in the parameter table, so the device never returns it. This
    // is what distinguishes go-udap's maps.Copy merge from an overwrite:
    // a key the device omits keeps its value.
    device
        .parameters
        .insert("local_note".to_owned(), b"keep-me".to_vec());

    within!(ops::config::get_all(
        &session,
        &CancellationToken::new(),
        &mut device
    ))
    .expect("get_all succeeds");

    assert!(
        !device.parameters.contains_key("offset_999"),
        "stale offset_* keys must be cleared before the merge"
    );
    assert_eq!(
        device.parameters.get("local_note").map(Vec::as_slice),
        Some(b"keep-me".as_slice()),
        "a key the device did not return must survive: get_all merges, \
         it does not replace"
    );
}

#[tokio::test]
async fn reset_succeeds_when_the_device_acknowledges() {
    let (session, device) = fixture();
    within!(ops::config::reset(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("the mock acknowledges reset");
}

#[tokio::test]
async fn reset_treats_no_acknowledgement_as_success() {
    // The device may reset before it can reply. go-udap logs and returns
    // nil rather than reporting a failure.
    let mac = Mac::from_bytes(MAC);
    // An empty network: nothing answers, so the wait is cancelled.
    let network = Arc::new(Network::new(vec![]));
    let session = Session::new(Box::new(MockTransport::new(network)));
    let device = Device {
        mac,
        ..Device::default()
    };

    let cancel = CancellationToken::new();
    cancel.cancel();

    ops::config::reset(&session, &cancel, &device)
        .await
        .expect("a silent device means the reset probably landed");
}

#[tokio::test]
async fn reset_refuses_a_device_with_no_mac() {
    // Without this guard the request goes to nobody, the wait is
    // cancelled, and reset's "no acknowledgement means success" branch
    // reports a successful reset of a device that was never addressed.
    let (session, _) = fixture();
    let err = within!(ops::config::reset(
        &session,
        &CancellationToken::new(),
        &Device::default()
    ))
    .expect_err("a zero MAC cannot be addressed");
    assert_eq!(
        err.to_string(),
        "cannot build Reset packet: device has zero MAC address"
    );
}

/// A device that answers every directed request with an error reply.
fn failing_fixture(message: &str) -> (Session, Device) {
    let mac = Mac::from_bytes(MAC);
    let mut cfg = DeviceConfig::default_with_mac(mac);
    cfg.error_reply = Some(message.to_owned());
    let session = Session::new(Box::new(MockTransport::new(Arc::new(Network::new(vec![
        cfg,
    ])))));
    let device = Device {
        mac,
        ..Device::default()
    };
    (session, device)
}

#[tokio::test]
async fn get_reports_an_error_reply_without_decoding_its_message() {
    // Deliberately unlike get_ip: go-udap's get does not read the
    // error-message TLV, so the message is dropped even when present.
    // Carried forward rather than improved.
    let (session, device) = failing_fixture("no such offset");
    let err = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect_err("the device refused");
    assert_eq!(
        err.to_string(),
        "device 00:04:20:16:17:18 returned error response",
        "get drops the device's explanation; get_ip would report it"
    );
}

#[tokio::test]
async fn reset_reports_the_devices_reason_for_refusing() {
    let (session, device) = failing_fixture("locked");
    let err = within!(ops::config::reset(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect_err("the device refused");
    assert_eq!(
        err.to_string(),
        "device 00:04:20:16:17:18 rejected reset: locked",
        "reset words this as 'rejected reset', not 'error'"
    );
}

#[tokio::test]
async fn reset_reports_a_refusal_with_no_explanation() {
    let (session, device) = failing_fixture("");
    let err = within!(ops::config::reset(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect_err("the device refused");
    assert_eq!(err.to_string(), "device 00:04:20:16:17:18 rejected reset");
}
