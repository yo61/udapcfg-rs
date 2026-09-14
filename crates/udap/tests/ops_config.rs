//! `get`, `get_all` and `reset` against `mocksbr`.

use mocksbr::{DeviceConfig, MockTransport, Network};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

/// Wraps a transport and counts sends, so a test can prove how many
/// round trips an operation made.
struct CountingTransport {
    inner: MockTransport,
    sends: Arc<AtomicUsize>,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait::async_trait]
impl udap::transport::Transport for CountingTransport {
    async fn send(&self, packet: &[u8]) -> Result<(), udap::transport::TransportError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut sent) = self.sent.lock() {
            sent.push(packet.to_vec());
        }
        self.inner.send(packet).await
    }
    async fn recv(
        &self,
        cancel: &CancellationToken,
    ) -> Result<(Vec<u8>, String), udap::transport::TransportError> {
        self.inner.recv(cancel).await
    }
    async fn close(&self) -> Result<(), udap::transport::TransportError> {
        self.inner.close().await
    }
}

type Sent = Arc<Mutex<Vec<Vec<u8>>>>;

fn counting_fixture() -> (Session, Device, Arc<AtomicUsize>, Sent) {
    let mac = Mac::from_bytes(MAC);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let sends = Arc::new(AtomicUsize::new(0));
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let transport = CountingTransport {
        inner: MockTransport::new(network),
        sends: Arc::clone(&sends),
        sent: Arc::clone(&sent),
    };
    let device = Device {
        mac,
        ..Device::default()
    };
    (Session::new(Box::new(transport)), device, sends, sent)
}

/// The item count a request body declares, or `None` if it is not the
/// method asked for. Layout: 27-byte header, 32 credential bytes, then a
/// big-endian u16 count.
fn item_count(packet: &[u8], want_method: u16) -> Option<u16> {
    const HEADER: usize = 27;
    const CREDENTIALS: usize = 32;
    let (parsed, _) = udap::Packet::from_bytes(packet).ok()?;
    if parsed.ucp_method != want_method {
        return None;
    }
    let at = HEADER + CREDENTIALS;
    Some(u16::from_be_bytes([packet[at], packet[at + 1]]))
}

fn change(name: &str, value: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut map = BTreeMap::new();
    map.insert(name.to_owned(), value.to_vec());
    map
}

#[tokio::test]
async fn set_writes_every_parameter_not_only_the_ones_asked_for() {
    // Omitting a parameter from a set_data request writes zeros over the
    // neighbouring NVRAM region, which is the whole reason set does a
    // read-modify-write.
    //
    // This inspects the packet, not device.parameters: the cache is
    // populated by the prelude read either way, so asserting on it passes
    // even when the request carried a single item. Found by mutation
    // testing -- the earlier version of this test could not tell the
    // difference.
    let (session, mut device, _sends, sent) = counting_fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    let packets = sent.lock().expect("not poisoned");
    let count = packets
        .iter()
        .find_map(|p| item_count(p, 0x0006))
        .expect("a SetData request was sent");
    assert!(
        count > 1,
        "SetData carried {count} item(s): a single item means the caller's \
         change alone, zeroing every neighbouring NVRAM field"
    );
    assert_eq!(
        usize::from(count),
        udap::parameters::names().count(),
        "the merged set is every known parameter"
    );
}

#[tokio::test]
async fn set_records_the_new_value_after_the_device_acknowledges() {
    let (session, mut device) = fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    assert_eq!(
        device.parameters.get("wireless_channel").map(Vec::as_slice),
        Some(b"11".as_slice()),
        "an acknowledged write updates the cache"
    );
}

#[tokio::test]
async fn set_skips_the_prelude_read_when_parameters_are_cached() {
    // cli/set.go pre-populates parameters so this is one round trip, not
    // two. Counting sends is what proves the prelude was skipped.
    let (session, mut device, sends, _sent) = counting_fixture();
    within!(ops::config::get_all(
        &session,
        &CancellationToken::new(),
        &mut device
    ))
    .expect("prime the cache");
    let after_priming = sends.load(Ordering::SeqCst);

    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    assert_eq!(
        sends.load(Ordering::SeqCst) - after_priming,
        1,
        "a primed cache means one send: the SetData itself"
    );
}

#[tokio::test]
async fn set_reads_first_when_the_cache_is_empty() {
    // The converse: without a primed cache it is two sends, the prelude
    // GetData and the SetData.
    let (session, mut device, sends, _sent) = counting_fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    assert_eq!(sends.load(Ordering::SeqCst), 2, "prelude read, then write");
}

#[tokio::test]
async fn set_does_not_record_a_value_the_device_never_acknowledged() {
    // The commit barrier. go-udap moved this merge after the ack because
    // doing it earlier left parameters advertising values that were
    // never persisted when the round trip failed.
    let (session, mut device) = failing_fixture("locked");
    device
        .parameters
        .insert("wireless_channel".to_owned(), b"6".to_vec());

    let result = within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ));

    assert!(result.is_err(), "the device refused the write");
    assert_eq!(
        device.parameters.get("wireless_channel").map(Vec::as_slice),
        Some(b"6".as_slice()),
        "a refused write must leave the cached value alone, not show 11"
    );
}

#[tokio::test]
async fn set_refuses_a_device_with_no_mac() {
    let (session, _) = fixture();
    let err = within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut Device::default(),
        &change("wireless_channel", b"11")
    ))
    .expect_err("a zero MAC cannot be addressed");
    assert_eq!(
        err.to_string(),
        "cannot build SetData packet: device has zero MAC address"
    );
}

#[tokio::test]
async fn set_caches_a_name_it_never_sent() {
    // A carried-forward wart, pinned so a change to it is deliberate.
    //
    // set_data_payload skips names the parameter table does not know, so
    // they never reach the device — but the post-ack merge copies the
    // caller's whole map, so the cache reports them anyway. go-udap does
    // the same. See the spec's known warts.
    let (session, mut device, _sends, sent) = counting_fixture();
    let mut config = change("wireless_channel", b"11");
    config.insert("not_a_real_parameter".to_owned(), b"whatever".to_vec());

    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &config
    ))
    .expect("the unknown name does not fail the write");

    let packets = sent.lock().expect("not poisoned");
    let count = packets
        .iter()
        .find_map(|p| item_count(p, 0x0006))
        .expect("a SetData request was sent");
    assert_eq!(
        usize::from(count),
        udap::parameters::names().count(),
        "the unknown name was not sent to the device"
    );
    assert_eq!(
        device
            .parameters
            .get("not_a_real_parameter")
            .map(Vec::as_slice),
        Some(b"whatever".as_slice()),
        "yet it is cached: the wart this test documents"
    );
}

#[tokio::test]
async fn a_set_is_visible_to_a_later_get() {
    // The point of a stateful mock: until now the device discarded
    // writes and always answered with factory defaults, so nothing
    // could catch a set that encoded wrongly.
    let (session, mut device) = fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_channel"]
    ))
    .expect("get succeeds");

    assert_eq!(
        values.get("wireless_channel").map(Vec::as_slice),
        Some(b"11".as_slice()),
        "the device must report what was written, not the factory default"
    );
}

#[tokio::test]
async fn a_set_does_not_disturb_neighbouring_parameters() {
    let (session, mut device) = fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_region_id"]
    ))
    .expect("get succeeds");

    assert_eq!(
        values.get("wireless_region_id").map(Vec::as_slice),
        Some(b"4".as_slice()),
        "the read-modify-write must have preserved this"
    );
}

#[tokio::test]
async fn a_non_utf8_value_round_trips_byte_exact() {
    // ADR-6 end to end: client -> wire -> device state -> wire -> client.
    let (session, mut device) = fixture();
    let ssid = vec![0xffu8, 0xfe, 0x41];
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_SSID", &ssid)
    ))
    .expect("set succeeds");

    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_SSID"]
    ))
    .expect("get succeeds");

    assert_eq!(
        values.get("wireless_SSID").map(Vec::as_slice),
        Some(ssid.as_slice()),
        "a non-UTF-8 SSID must survive the full round trip unchanged"
    );
}

#[tokio::test]
async fn reset_restores_the_saved_values() {
    // mocksbr saves on every set, matching go-udap, so a reset reloads
    // the most recent write rather than the factory default.
    //
    // Know what this can and cannot catch. It fails if reset restores
    // factory defaults -- the plausible wrong implementation. It does
    // NOT fail if reset does nothing at all, and no test at this level
    // could: save-on-every-set means working memory and NVRAM are always
    // identical when a request arrives, so `working <- nvram` is
    // observationally a no-op through the wire. The difference is only
    // visible where working and NVRAM diverge, which needs a set without
    // a save -- something method 0x0006 cannot express, since it does
    // both. That case is covered by state.rs's unit tests.
    let (session, mut device) = fixture();
    within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &change("wireless_channel", b"11")
    ))
    .expect("set succeeds");

    within!(ops::config::reset(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("reset succeeds");

    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_channel"]
    ))
    .expect("get succeeds");

    assert_eq!(
        values.get("wireless_channel").map(Vec::as_slice),
        Some(b"11".as_slice()),
        "the set was saved, so the reset reload must observe it"
    );
}
