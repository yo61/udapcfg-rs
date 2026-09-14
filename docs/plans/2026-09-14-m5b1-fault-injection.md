# udapcfg-rs M5-B1 Implementation Plan — synchronous fault injection

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `mocksbr` the fault-injection knobs that need no clock, so the client's failure paths can be driven from a test.

**Architecture:** Four knobs on `DeviceConfig`, checked in `Network::receive` in go-udap's precedence order — `Unreachable` first, then `FailOn`, then the operation's own handler, inside which `DropGetIP`/`DropGetUUID` suppress a reply. `Malformed` changes the *shape* of a `get_data` reply rather than suppressing it, which is what makes the client's decode error paths reachable. Everything stays synchronous.

**Tech Stack:** Rust 1.98.1. No new dependencies.

**Spec:** [`docs/specs/2026-09-08-rust-port-spec.md`](../specs/2026-09-08-rust-port-spec.md) — milestone M5
**Reference:** go-udap `mocksbr/{device,handlers,responses}.go`
**Prior plan:** [`2026-09-14-m5a-stateful-mocksbr.md`](2026-09-14-m5a-stateful-mocksbr.md)

## Global Constraints

- **Source of truth is go-udap v2.4.8 (`43864a5`)** at `~/code/github.com/yo61/go-udap`. Where this plan and the Go disagree, the Go wins — read it and fix the plan.
- **The toolchain is mise-managed and NOT on PATH.** Prefix every cargo command: `mise exec -- cargo ...` from the repo root.
- **Zero warnings.** `cargo clippy --all-targets --all-features` and `cargo fmt --all --check` clean. `RUSTFLAGS=-D warnings` is set in `mise.toml`.
- **No `#[allow(...)]`.** Suppress only with `#[expect(lint, reason = "...")]`. Where an item is used only by tests, scope it: `#[cfg_attr(not(test), expect(dead_code, reason = "..."))]` — and expect it to fire as *unfulfilled* the moment a real caller appears, which is the point.
- **No `.unwrap()`/`.expect()` in non-test code.** `clippy.toml` exempts tests, but only in frames carrying `#[test]`/`#[tokio::test]` — a fallible call in a plain helper fn still fires. A bounded wait in a test must be a **macro**, so the `expect` expands inside the test frame.
- **`clippy::panic` is denied everywhere, including tests.** Use `assert!`/`assert_eq!`, never `panic!`.
- **Device-supplied values are bytes.** ADR-6: parameter values are `Vec<u8>`, never `String`.
- **TDD.** Failing test first, watch it fail, then implement. Where `-D warnings` forbids an empty stub, make it *complete but wrong* — a better red anyway.
- **Mutation-check each behaviour.** Break it, confirm a test fails, restore. **Verify the edit actually landed** (`rg` for the changed text) before believing a "survived" result — rustfmt reflows code, and a mutation that silently failed to apply looks exactly like a passing one.
- **Commit per task**, conventional-commit format, on a feature branch. Never commit to `main`.
- **Pushing runs the lastlight gate.** Batch all edits, then review once, then push.

## Why these four, and not the other three

M5's acceptance is "go-udap's `mocksbr` integration tests pass in Rust", so those tests define what is required. Counting their uses:

| Knob | Uses | In this plan? |
|---|---|---|
| `FailOn` | 16 | yes |
| `Malformed` | 7 | yes |
| `NVRAM` | 6 | yes |
| `Unreachable` | 5 | yes |
| `DropGetIP` / `DropGetUUID` | 2 each | yes |
| `Slow` | 7 | **no** — the only knob needing a clock; its own plan |
| `RebootDelay` | 0 | no — for the CLI's tests, M6 |
| `DropGetData` | 0 | no — for the CLI's tests, M6 |
| `SuppressDiscoveryUUID` | 0 | no — for the CLI's tests, M6 |

The three with zero uses say so in their own doc comments: they exist for
CLI tests that do not exist yet. Deferring them also removes the reboot
window, so nothing here needs a clock.

## The knob this replaces

M4 added `DeviceConfig.error_reply: Option<String>` ad hoc, to test the
device-error paths. It fails **every** directed request with a
caller-chosen message. `FailOn` is go-udap's equivalent and is
per-operation, which `error_reply` cannot express — a test needing
"`get_data` fails but `get_ip` works" has no way to say so today.

`error_reply` is therefore replaced, not kept alongside. But a straight
swap would lose coverage: `failing_fixture("")` currently drives the
*no-TLV* error path (`OpError::DeviceNoMessage`,
`OpError::ResetRejectedNoMessage`), and `FailOn`'s message is fixed.
go-udap never tests that path — `"returned error response"` appears at
its four production sites and in no test — so the coverage is ours to
keep. Hence two fields rather than one:

- `fail_on: Vec<Op>` — which operations fail
- `fail_message: Option<String>` — `None` gives go-udap's
  `mocksbr: configured to fail <op>`; `Some("")` gives an error reply
  with no TLV; `Some(text)` gives that text

## File Structure

```
crates/mocksbr/src/
  device.rs       MODIFY  Op enum; DeviceConfig gains fail_on,
                          fail_message, unreachable, drop_get_ip,
                          drop_get_uuid, malformed, nvram; loses
                          error_reply
  network.rs      MODIFY  precedence: unreachable -> fail_on -> handler
  responses.rs    MODIFY  malformed shapes for get_data
  state.rs        MODIFY  factory_with(seed) for the NVRAM knob

crates/udap/tests/
  ops_config.rs   MODIFY  rewrite the five failing_fixture call sites
  ops_faults.rs   CREATE  the new knobs, driven through the real client
```

**Not in this plan:** `Slow` (plan B2), and `RebootDelay`,
`DropGetData`, `SuppressDiscoveryUUID` (M6, when the CLI needs them).

---

### Task 1: `Op` and `FailOn`

Replaces `error_reply`. First because five existing tests depend on the
knob it removes, and leaving them broken across tasks would make every
later red ambiguous.

**Files:**
- Modify: `crates/mocksbr/src/device.rs`, `crates/mocksbr/src/network.rs`, `crates/udap/tests/ops_config.rs`

**Interfaces:**
- Produces:
  - `pub enum Op { Discover, Get, Set, Save, Reset, GetIp, GetUuid }`, deriving `Debug, Clone, Copy, PartialEq, Eq`
  - `DeviceConfig.fail_on: Vec<Op>`
  - `DeviceConfig.fail_message: Option<String>`
  - `pub(crate) fn Op::from_method(method: u16) -> Option<Op>`

- [ ] **Step 1: Add the `Op` enum and the two fields**

In `crates/mocksbr/src/device.rs`:

```rust
/// A UDAP operation, for the failure-injection knobs.
///
/// `Set` and `Save` are the same wire method (0x0006) — a real device
/// does both on one request — so `from_method` reports `Set` and a
/// config naming `Save` has no effect on its own. Both variants exist to
/// match go-udap's surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Discover,
    Get,
    Set,
    Save,
    Reset,
    GetIp,
    GetUuid,
}

impl Op {
    /// The operation a UCP method denotes, if this mock models it.
    pub(crate) fn from_method(method: u16) -> Option<Op> {
        use udap::protocol::method;
        match method {
            method::ADV_DISC => Some(Op::Discover),
            method::GET_DATA => Some(Op::Get),
            method::SET_DATA => Some(Op::Set),
            method::RESET => Some(Op::Reset),
            method::GET_IP => Some(Op::GetIp),
            method::GET_UUID => Some(Op::GetUuid),
            _ => None,
        }
    }

    /// The name go-udap uses in its failure message.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Op::Discover => "discover",
            Op::Get => "get",
            Op::Set => "set",
            Op::Save => "save",
            Op::Reset => "reset",
            Op::GetIp => "getip",
            Op::GetUuid => "getuuid",
        }
    }
}
```

On `DeviceConfig`, delete `pub error_reply: Option<String>` and add:

```rust
    /// Fault injection: operations the device rejects with UCP 0x0007.
    pub fail_on: Vec<Op>,
    /// The rejection message.
    ///
    /// `None` uses go-udap's `mocksbr: configured to fail <op>`.
    /// `Some("")` sends an error reply carrying **no** TLV at all, which
    /// is a distinct client path (`OpError::DeviceNoMessage`) that
    /// go-udap has no way to produce and does not test.
    pub fail_message: Option<String>,
```

Default both in `default_with_mac`: `fail_on: Vec::new(), fail_message: None`.

- [ ] **Step 2: Check `fail_on` in the dispatch**

In `crates/mocksbr/src/network.rs`, replace the `error_reply` block. The
message is built from the *requested* operation, so a device failing only
`GetIp` says `getip`:

```rust
                // Failure injection short-circuits the operation's own
                // reply, but not discovery: a device that cannot answer
                // get_data is still discoverable.
                let op = Op::from_method(request.ucp_method);
                if let Some(op) = op
                    && op != Op::Discover
                    && cfg.fail_on.contains(&op)
                {
                    let message = cfg.fail_message.clone().unwrap_or_else(|| {
                        format!("mocksbr: configured to fail {}", op.as_str())
                    });
                    return Some((
                        responses::error_response(&request, cfg, &message),
                        cfg.mac.to_string(),
                    ));
                }
```

`responses::error_response` already sends no TLV for an empty message —
that behaviour was added in M4 and is what `Some("")` relies on.

- [ ] **Step 3: Rewrite the five `failing_fixture` call sites**

`crates/udap/tests/ops_config.rs` has one helper and four users. Replace
the helper:

```rust
/// A device that rejects `ops` with `message`.
///
/// An empty `message` sends an error reply with no TLV, which is a
/// distinct client path from one carrying an explanation.
fn failing_fixture(ops: &[mocksbr::Op], message: &str) -> (Session, Device) {
    let mac = Mac::from_bytes(MAC);
    let mut cfg = DeviceConfig::default_with_mac(mac);
    cfg.fail_on = ops.to_vec();
    cfg.fail_message = Some(message.to_owned());
    let session = Session::new(Box::new(MockTransport::new(Arc::new(Network::new(vec![
        cfg,
    ])))));
    let device = Device {
        mac,
        ..Device::default()
    };
    (session, device)
}
```

Then update each call, keeping every existing assertion unchanged —
the messages are still caller-chosen, so the expected strings do not move:

- `get_reports_an_error_reply_without_decoding_its_message`:
  `failing_fixture(&[mocksbr::Op::Get], "no such offset")`
- `reset_reports_the_devices_reason_for_refusing`:
  `failing_fixture(&[mocksbr::Op::Reset], "locked")`
- `reset_reports_a_refusal_with_no_explanation`:
  `failing_fixture(&[mocksbr::Op::Reset], "")`
- `set_does_not_record_a_value_the_device_never_acknowledged`:
  `failing_fixture(&[mocksbr::Op::Set], "locked")`

Export `Op` from the crate root so tests can name it: add
`pub use device::Op;` to `crates/mocksbr/src/lib.rs`.

- [ ] **Step 4: Run and watch the suite stay green**

Run: `mise exec -- cargo test --workspace`
Expected: all pass. **This task changes no behaviour any test asserts** —
it changes how the fault is requested, not what it does. A failure here
means the replacement is not equivalent.

- [ ] **Step 5: Write the failing test for what `FailOn` adds**

The whole point of the replacement — per-operation precision, which
`error_reply` could not express. `crates/udap/tests/ops_faults.rs`:

```rust
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
```

- [ ] **Step 6: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: `the_default_failure_message_names_the_operation` fails if the
message is not built from the requested op.

- [ ] **Step 7: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src crates/udap/tests
git commit -m "feat(mocksbr): replace error_reply with per-operation FailOn"
```

---

### Task 2: `Unreachable` and the `Drop` pair

Reply suppression. Three knobs, one mechanism: the device stays silent
and the client's timeout path runs.

**Files:**
- Modify: `crates/mocksbr/src/device.rs`, `crates/mocksbr/src/network.rs`
- Test: `crates/udap/tests/ops_faults.rs`

**Interfaces:**
- Consumes: `Op`, `fixture_with` from Task 1
- Produces: `DeviceConfig.unreachable: bool`, `.drop_get_ip: bool`, `.drop_get_uuid: bool`

- [ ] **Step 1: Write the failing tests**

A silent device means the operation never returns, so each test cancels
the token to stand in for `--timeout`. Without that they would hang, and
`within!` would report a five-second block rather than the behaviour.

```rust
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
async fn an_unreachable_device_is_still_not_discoverable_by_accident() {
    // Unreachable means unreachable: unlike fail_on, it suppresses the
    // discovery reply too, because the device is off the network rather
    // than refusing a request.
    let mac = Mac::from_bytes(MAC);
    let mut cfg = DeviceConfig::default_with_mac(mac);
    cfg.unreachable = true;
    let network = Arc::new(Network::new(vec![cfg]));
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

    within!(ops::getip::get_ip(&session, &cancel, &device))
        .expect_err("get_ip is dropped");

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

    within!(ops::getuuid::get_uuid(&session, &cancel, &device))
        .expect_err("get_uuid is dropped");

    within!(ops::getip::get_ip(
        &session,
        &CancellationToken::new(),
        &device
    ))
    .expect("get_ip is unaffected");
}
```

Add the helper that builds a discovery request, beside `fixture_with`:

```rust
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
```

- [ ] **Step 2: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: compile error — the fields do not exist.

- [ ] **Step 3: Add the fields and the checks**

Three `bool` fields on `DeviceConfig`, all defaulting to `false`:

```rust
    /// Fault injection: the device answers nothing at all, including
    /// discovery. Models a device that is off the network, as distinct
    /// from `fail_on`, which models one that refuses a request.
    pub unreachable: bool,
    /// Fault injection: `get_ip` requests get no reply.
    pub drop_get_ip: bool,
    /// Fault injection: `get_uuid` requests get no reply.
    pub drop_get_uuid: bool,
```

In `network.rs`, `unreachable` is checked **before** everything, including
the discovery arm — it is the only knob that suppresses discovery:

```rust
            .filter(|(_, cfg)| !cfg.unreachable)
```

placed on the existing `.filter(...)` chain, before the addressing filter.
The `Drop` pair lives in the method match, returning `None`:

```rust
                    method::GET_IP if cfg.drop_get_ip => return None,
                    method::GET_UUID if cfg.drop_get_uuid => return None,
```

Place both arms **above** the corresponding reply arms; a match arm order
mistake here silently disables the knob.

- [ ] **Step 4: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: 7 passed (3 from Task 1, 4 new).

- [ ] **Step 5: Mutation-check the precedence**

Move the `unreachable` filter *after* the addressing filter: no test
should change, because both filters are conjunctive — confirming the
placement is about clarity, not behaviour. Then delete it entirely:
`an_unreachable_device_answers_nothing` must fail. Verify each edit
landed before believing the result.

- [ ] **Step 6: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src crates/udap/tests/ops_faults.rs
git commit -m "feat(mocksbr): add Unreachable and the per-operation Drop knobs"
```

---

### Task 3: `Malformed`

The most valuable knob here: it makes `OpError::Decode` reachable from an
integration test for the first time. `GetDataError`'s bounds checks were
ported at M1 and have only ever been tested against hand-built byte
arrays — never against a device producing them.

**Files:**
- Modify: `crates/mocksbr/src/device.rs`, `crates/mocksbr/src/responses.rs`
- Test: `crates/udap/tests/ops_faults.rs`

**Interfaces:**
- Consumes: `fixture_with` from Task 1
- Produces: `pub enum Malformed { None, OversizedCount, LengthExceedsPayload, UnknownMethod }`, `DeviceConfig.malformed: Malformed`

- [ ] **Step 1: Write the failing tests**

```rust
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
    let (session, device) =
        fixture_with(|cfg| cfg.malformed = Malformed::LengthExceedsPayload);
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
```

Import `Malformed` in the test file's `use mocksbr::{...}` list.

- [ ] **Step 2: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: compile error — `Malformed` does not exist.

- [ ] **Step 3: Add the enum and the field**

```rust
/// A deliberately broken reply shape, for exercising the client's
/// decode error paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Malformed {
    /// Well-formed replies.
    #[default]
    None,
    /// Declare 65535 items and write no bodies, so the client's
    /// per-item bounds check fires on the first one.
    OversizedCount,
    /// Declare one item of length 1000 and write nothing, so the
    /// client's "item exceeds payload" check fires.
    LengthExceedsPayload,
    /// Reply with an unrecognised UCP method.
    UnknownMethod,
}
```

On `DeviceConfig`: `pub malformed: Malformed,` defaulting to
`Malformed::None`.

- [ ] **Step 4: Apply it in `get_data_response`**

The method substitution happens when the header is built; the payload
shapes replace the item list. Both are `get_data`-only — go-udap applies
`Malformed` nowhere else.

```rust
    let method = if cfg.malformed == Malformed::UnknownMethod {
        0x9999
    } else {
        request.ucp_method
    };
    let mut out = build_header(request, cfg, method).to_bytes().to_vec();

    match cfg.malformed {
        Malformed::OversizedCount => {
            out.extend_from_slice(&0xFFFFu16.to_be_bytes());
            return out;
        }
        Malformed::LengthExceedsPayload => {
            out.extend_from_slice(&1u16.to_be_bytes()); // one item
            out.extend_from_slice(&0u16.to_be_bytes()); // offset
            out.extend_from_slice(&1000u16.to_be_bytes()); // length, no body
            return out;
        }
        Malformed::None | Malformed::UnknownMethod => {}
    }
```

`UnknownMethod` falls through to the normal payload — only the method
differs, so the client rejects it before decoding.

- [ ] **Step 5: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: 11 passed.

- [ ] **Step 6: Mutation-check that the modes are distinct**

Make `OversizedCount` emit the `LengthExceedsPayload` shape: the first
test must fail while the second still passes. If both pass, the two
tests are not distinguishing the modes and one of them is redundant.

- [ ] **Step 7: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src crates/udap/tests/ops_faults.rs
git commit -m "feat(mocksbr): add the Malformed reply knob"
```

---

### Task 4: `NVRAM` seed overrides

Lets a test start a device in a non-factory state. Deferred from M5-A as
having no consumer; it has one now.

**Files:**
- Modify: `crates/mocksbr/src/device.rs`, `crates/mocksbr/src/state.rs`, `crates/mocksbr/src/network.rs`
- Test: `crates/udap/tests/ops_faults.rs`

**Interfaces:**
- Consumes: `DeviceState::factory` from M5-A
- Produces:
  - `DeviceConfig.nvram: BTreeMap<String, Vec<u8>>`
  - `pub(crate) fn DeviceState::factory_with(seed: &BTreeMap<String, Vec<u8>>) -> Self`

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: compile error — `DeviceConfig` has no `nvram` field.

- [ ] **Step 3: Add the field and the seeded constructor**

On `DeviceConfig`:

```rust
    /// Fault injection: NVRAM values overriding the factory defaults.
    ///
    /// Seeds **both** tiers, so a device starts as though it had been
    /// configured and saved — a reset reloads these, not the factory
    /// table.
    pub nvram: BTreeMap<String, Vec<u8>>,
```

defaulting to `BTreeMap::new()`. In `state.rs`:

```rust
    /// A device whose NVRAM carries `seed` over the factory defaults.
    ///
    /// Both tiers get it: the seed models a device that was configured
    /// and saved before the test began, so a reset must find it.
    pub(crate) fn factory_with(seed: &BTreeMap<String, Vec<u8>>) -> Self {
        let mut state = Self::factory();
        state.working.extend(seed.iter().map(|(k, v)| (k.clone(), v.clone())));
        state.nvram.extend(seed.iter().map(|(k, v)| (k.clone(), v.clone())));
        state
    }
```

In `network.rs`, both constructors build state from the config rather
than unconditionally from factory:

```rust
        let state = devices.iter().map(|c| DeviceState::factory_with(&c.nvram)).collect();
```

- [ ] **Step 4: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_faults`
Expected: 14 passed.

- [ ] **Step 5: Mutation-check that the seed reaches NVRAM**

Make `factory_with` seed only `working` and not `nvram`:
`a_seed_reaches_nvram_so_a_reset_reloads_it` must fail while the other
two still pass. That is the assertion distinguishing "seeded the running
config" from "seeded what survives a reboot".

- [ ] **Step 6: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src crates/udap/tests/ops_faults.rs
git commit -m "feat(mocksbr): seed device NVRAM from DeviceConfig"
```

---

## Acceptance

```bash
mise exec -- cargo test --workspace          # all green
mise exec -- cargo clippy --all-targets --all-features   # silent
mise exec -- cargo fmt --all --check          # clean
```

Beyond the new tests, this plan closes a real coverage gap:
`OpError::Decode` and `GetDataError`'s two bounds checks become
reachable from an integration test for the first time. They were ported
at M1 and have since been exercised only against hand-built byte arrays.

## Deferred

- **`Slow`** — the only remaining knob needing a clock. Plan B2. go-udap
  returns `ScheduledReply { Bytes, Delay }` and schedules non-zero delays
  through `time.AfterFunc`; the Rust equivalent is a `tokio::spawn` plus
  `sleep` feeding `MockTransport`'s existing channel. Worth its own pass
  because it changes `Network::receive`'s contract from immediate replies
  to scheduled ones, and because the test strategy — real delays versus
  `#[tokio::test(start_paused = true)]` — is a genuine decision.
- **`RebootDelay`, `DropGetData`, `SuppressDiscoveryUUID`** — zero uses
  in go-udap's `mocksbr` tests; their doc comments say they exist for the
  CLI's tests. M6.
- **The standalone binary.** Plan C.
