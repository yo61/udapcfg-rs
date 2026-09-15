//! The fault-injection knobs, driven through the real client.

use mocksbr::{DeviceConfig, Malformed, MockTransport, Network, Op};
use std::collections::BTreeMap;
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

/// Asserts an operation never completes, because the device is silent.
///
/// Pair with `#[tokio::test(start_paused = true)]`: with nothing left to
/// run, the runtime advances virtual time to the deadline, so this is
/// instant.
///
/// Why not a pre-cancelled token? `MockTransport::recv` selects over the
/// cancellation and the queued reply *without* `biased`, so when a reply
/// exists both branches are ready and the winner is random. A test built
/// that way passes whether or not the device stayed silent — measured at
/// a 70% catch rate against a mutant that removed the suppression. Here
/// a reply resolves the future immediately and fails the assertion.
macro_rules! silent {
    ($fut:expr) => {
        tokio::time::timeout(Duration::from_secs(5), $fut)
            .await
            .expect_err("a silent device must not produce a reply")
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

#[tokio::test(start_paused = true)]
async fn an_unreachable_device_answers_nothing() {
    let (session, device) = fixture_with(|cfg| cfg.unreachable = true);
    silent!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ));
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

#[tokio::test(start_paused = true)]
async fn drop_get_ip_silences_only_get_ip() {
    let (session, device) = fixture_with(|cfg| cfg.drop_get_ip = true);
    silent!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ));

    within!(ops::getuuid::get_uuid(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_uuid is unaffected");
}

#[tokio::test(start_paused = true)]
async fn drop_get_uuid_silences_only_get_uuid() {
    let (session, device) = fixture_with(|cfg| cfg.drop_get_uuid = true);
    silent!(ops::getuuid::get_uuid(
        &session,
        &CancellationToken::new(),
        &device
    ));

    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_ip is unaffected");
}

#[tokio::test]
async fn an_oversized_count_is_caught_by_the_per_item_bounds_check() {
    // The device promises 65535 items and writes no bodies.
    let (session, device) = fixture_with(|cfg| cfg.malformed = Malformed::OversizedCount);
    let err = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect_err("the reply is malformed");
    assert!(
        err.to_string().contains("truncated header for item 0"),
        "expected the bounds check to fire, got: {err}"
    );
}

#[tokio::test]
async fn an_item_longer_than_the_payload_is_rejected() {
    // One item declaring length 1000, with nothing following it.
    let (session, device) = fixture_with(|cfg| cfg.malformed = Malformed::LengthExceedsPayload);
    let err = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect_err("the reply is malformed");
    assert!(
        err.to_string().contains("exceeds payload"),
        "expected the item-length check to fire, got: {err}"
    );
}

#[tokio::test]
async fn an_unknown_reply_method_is_reported_with_its_value() {
    let (session, device) = fixture_with(|cfg| cfg.malformed = Malformed::UnknownMethod);
    let err = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect_err("0x9999 is not a reply we accept");
    assert_eq!(
        err.to_string(),
        "device 00:04:20:16:17:18: unexpected response method 0x9999"
    );
}

#[tokio::test]
async fn a_well_formed_device_decodes_cleanly() {
    // The control: without the knob the same request succeeds, so the
    // three tests above are attributable to the malformation.
    let (session, device) = fixture_with(|_| {});
    within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect("a well-formed reply decodes");
}

#[tokio::test]
async fn a_seeded_device_reports_the_seeded_value() {
    let (session, device) = fixture_with(|cfg| {
        cfg.nvram
            .insert("wireless_channel".to_owned(), b"9".to_vec());
    });
    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_channel"]
    ))
    .expect("get succeeds");
    assert_eq!(
        values.get("wireless_channel").map(Vec::as_slice),
        Some(b"9".as_slice()),
        "the seed must override the factory default"
    );
}

#[tokio::test]
async fn seeding_one_parameter_leaves_the_rest_at_factory() {
    let (session, device) = fixture_with(|cfg| {
        cfg.nvram
            .insert("wireless_channel".to_owned(), b"9".to_vec());
    });
    let values = within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["wireless_region_id"]
    ))
    .expect("get succeeds");
    assert_eq!(
        values.get("wireless_region_id").map(Vec::as_slice),
        Some(b"4".as_slice())
    );
}

#[tokio::test]
async fn a_seed_reaches_nvram_so_a_reset_reloads_it() {
    // The seed is the device's persisted state, not merely its working
    // memory: a reset must find it still there.
    let (session, device) = fixture_with(|cfg| {
        cfg.nvram
            .insert("wireless_channel".to_owned(), b"9".to_vec());
    });
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
        Some(b"9".as_slice())
    );
}

#[tokio::test]
async fn fail_on_discover_skips_the_device_from_discovery() {
    // go-udap's TestFailOnDiscoverDevicesAreSkipped. Discovery is a
    // broadcast, so there is no requester to send an error back to: the
    // device stays silent instead of replying with 0x0007.
    let ok = DeviceConfig::default_with_mac(Mac::from_bytes(MAC));
    let mut bad =
        DeviceConfig::default_with_mac(Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x19]));
    bad.fail_on = vec![Op::Discover];
    let network = Network::new(vec![ok, bad]);

    let replies = network.receive(&discovery_request());
    assert_eq!(
        replies.len(),
        1,
        "the fail-on-discover device must be skipped, not answered with an error"
    );
    assert_eq!(replies[0].src, "00:04:20:16:17:18");
}

#[tokio::test]
async fn naming_save_in_fail_on_also_rejects_a_set() {
    // go-udap's failsOn aliases Set and Save in both directions, because
    // they are the same wire method (0x0006).
    let (session, mut device) = fixture_with(|cfg| {
        cfg.fail_on = vec![Op::Save];
        cfg.fail_message = Some("locked".to_owned());
    });
    let mut updates = BTreeMap::new();
    updates.insert("wireless_channel".to_owned(), b"11".to_vec());

    let err = within!(ops::config::set(
        &session,
        &CancellationToken::new(),
        &mut device,
        &updates
    ))
    .expect_err("naming Save must reject a Set");
    assert!(
        err.to_string().contains("locked"),
        "expected the configured rejection, got: {err}"
    );
}

#[tokio::test]
async fn receive_reports_the_configured_delay() {
    // Network stays synchronous: it does not wait, it reports what the
    // wait would be. The transport is what turns this into elapsed time.
    const SLOW: Duration = Duration::from_millis(80);
    let mut cfg = DeviceConfig::default_with_mac(Mac::from_bytes(MAC));
    cfg.slow = SLOW;
    let network = Network::new(vec![cfg]);

    let replies = network.receive(&discovery_request());
    assert_eq!(replies.len(), 1);
    assert_eq!(
        replies[0].delay, SLOW,
        "the reply carries its device's delay"
    );
    assert_eq!(replies[0].src, "00:04:20:16:17:18");
}

#[tokio::test]
async fn a_device_with_no_configured_delay_reports_zero() {
    let cfg = DeviceConfig::default_with_mac(Mac::from_bytes(MAC));
    let network = Network::new(vec![cfg]);
    let replies = network.receive(&discovery_request());
    assert_eq!(replies[0].delay, Duration::ZERO);
}

/// T1. go-udap's `TestSlowDeviceReplyDelayedByConfiguredDuration`, but
/// exact: its version brackets with a 200ms skew tolerance
/// (failure_injection_test.go:96-101) because it uses a real clock.
#[tokio::test(start_paused = true)]
async fn a_slow_device_replies_after_exactly_its_delay() {
    const SLOW: Duration = Duration::from_millis(80);
    let (session, device) = fixture_with(|cfg| cfg.slow = SLOW);

    // tokio's Instant, not std's: std would report real elapsed time and
    // defeat the whole strategy.
    let start = tokio::time::Instant::now();
    within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect("the device answers, just late");
    assert_eq!(start.elapsed(), SLOW);
}

/// T2. go-udap's `TestSlowDeviceTimesOutWhenDeadlineShorter`. ADR-2: the
/// `CancellationToken` carries cancellation, `timeout` carries the
/// deadline, together standing in for Go's context.
#[tokio::test(start_paused = true)]
async fn a_deadline_shorter_than_the_delay_times_out() {
    const SLOW: Duration = Duration::from_millis(200);
    const BUDGET: Duration = Duration::from_millis(40);
    let (session, device) = fixture_with(|cfg| cfg.slow = SLOW);

    tokio::time::timeout(
        BUDGET,
        ops::config::get(&session, &CancellationToken::new(), &device, &["hostname"]),
    )
    .await
    .expect_err("the deadline expires before the device answers");
}

/// T4. go-udap sets Slow on the error path too
/// (mocksbr/handlers.go:142) but never tests it: a device that refuses
/// slowly is still slow.
#[tokio::test(start_paused = true)]
async fn slow_delays_an_error_reply_too() {
    const SLOW: Duration = Duration::from_millis(120);
    let (session, device) = fixture_with(|cfg| {
        cfg.slow = SLOW;
        cfg.fail_on = vec![Op::Get];
    });

    let start = tokio::time::Instant::now();
    within!(ops::config::get(
        &session,
        &CancellationToken::new(),
        &device,
        &["hostname"]
    ))
    .expect_err("configured to fail");
    assert_eq!(start.elapsed(), SLOW, "a refusal is delayed like any reply");
}

/// T3. `unreachable` wins: there is no reply for `slow` to delay, and it
/// must not conjure one late.
#[tokio::test(start_paused = true)]
async fn slow_does_not_resurrect_an_unreachable_device() {
    let (session, device) = fixture_with(|cfg| {
        cfg.unreachable = true;
        cfg.slow = Duration::from_millis(50);
    });
    silent!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ));
}

/// T6. Each reply carries its own device's delay, so a fast device
/// overtakes a slow one in the same fan-out — which is what real
/// hardware does.
///
/// Asserts each arrival *time*, not merely the order: order alone would
/// still pass if every reply in the batch were given one shared delay.
#[tokio::test(start_paused = true)]
async fn a_fan_out_delivers_each_reply_at_its_own_delay() {
    use udap::transport::Transport;

    const FAST: Duration = Duration::from_millis(20);
    const SLOW: Duration = Duration::from_millis(90);

    // The slow device is listed FIRST, so config order cannot explain
    // the arrival order.
    let mut slow =
        DeviceConfig::default_with_mac(Mac::from_bytes([0x00, 0x04, 0x20, 0x00, 0x00, 0x02]));
    slow.slow = SLOW;
    let mut fast =
        DeviceConfig::default_with_mac(Mac::from_bytes([0x00, 0x04, 0x20, 0x00, 0x00, 0x01]));
    fast.slow = FAST;

    let transport = MockTransport::new(Arc::new(Network::new(vec![slow, fast])));
    let cancel = CancellationToken::new();
    let start = tokio::time::Instant::now();
    transport.send(&discovery_request()).await.expect("send");

    let (_, first) = transport.recv(&cancel).await.expect("first reply");
    assert_eq!(start.elapsed(), FAST, "the fast device answers first");
    assert_eq!(first, "00:04:20:00:00:01");

    let (_, second) = transport.recv(&cancel).await.expect("second reply");
    assert_eq!(
        start.elapsed(),
        SLOW,
        "the slow device answers at its own delay"
    );
    assert_eq!(second, "00:04:20:00:00:02");
}
