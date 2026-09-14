# udapcfg-rs M5-A Implementation Plan — a stateful mocksbr

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `mocksbr` real per-device state, so a `set` followed by a `get` returns what was written instead of a factory default.

**Architecture:** Each virtual device gains two parameter maps — *working memory*, which `set_data` writes and `get_data` reads, and *NVRAM*, which `save` copies working memory into and `reset` reloads it from. go-udap's mock saves on every set, so a later reset observes the most recent values. Everything here is synchronous: `reset` reloads immediately, and the reboot *delay* is deferred to plan B along with the other fault-injection knobs.

**Tech Stack:** Rust 1.98.1. No new dependencies.

**Spec:** [`docs/specs/2026-09-08-rust-port-spec.md`](../specs/2026-09-08-rust-port-spec.md) — milestone M5
**Reference:** go-udap `mocksbr/{device,handlers,responses}.go`
**Prior plan:** [`2026-09-14-m4-remaining-operations.md`](2026-09-14-m4-remaining-operations.md)

## Global Constraints

- **Source of truth is go-udap v2.4.8 (`43864a5`)** at `~/code/github.com/yo61/go-udap`. Where this plan and the Go disagree, the Go wins — read it and fix the plan.
- **The toolchain is mise-managed and NOT on PATH.** Prefix every cargo command: `mise exec -- cargo ...` from the repo root.
- **Zero warnings.** `cargo clippy --all-targets --all-features` and `cargo fmt --all --check` clean. `RUSTFLAGS=-D warnings` is set in `mise.toml`.
- **No `#[allow(...)]`.** Suppress only with `#[expect(lint, reason = "...")]`, and only when a rewrite genuinely cannot satisfy the lint. Where an item is used only by tests, scope it: `#[cfg_attr(not(test), expect(dead_code, reason = "..."))]`.
- **No `.unwrap()`/`.expect()` in non-test code.** `clippy.toml` exempts tests, but only in frames carrying `#[test]`/`#[tokio::test]` — a fallible call in a plain helper fn still fires. If a test needs a bounded wait, use a **macro** so the `expect` expands inside the test frame.
- **`clippy::panic` is denied everywhere, including tests.** Use `assert!`/`assert_eq!`, never `panic!`.
- **Device-supplied values are bytes.** ADR-6: parameter values are `Vec<u8>`, never `String`, on both sides of the wire.
- **TDD.** Failing test first, watch it fail, then implement. Where `-D warnings` forbids a stub (an unused constant or import), make the stub *complete but wrong* rather than empty — that is a better red anyway.
- **Mutation-check each behaviour.** Break it, confirm a test fails, restore. A mutation that "survives" in 0.00 s usually did not apply — rustfmt reflows code, so verify the edit landed before believing the result.
- **Commit per task**, conventional-commit format, on a feature branch. Never commit to `main`.
- **Pushing runs the lastlight gate.** Batch all edits, then review once, then push.

## Why this is worth doing

Today `mocksbr` answers `get_data` by encoding each requested parameter's
`factory_default`, and discards `set_data` entirely. Two consequences:

- **No test can catch a `set` that encodes wrongly.** `crates/udap/tests/ops_config.rs::set_records_the_new_value_after_the_device_acknowledges` asserts only that the *client's* cache updated — the device never stored anything, so the wire encoding is unverified in both directions.
- **`reset` is untestable.** It acknowledges and changes nothing, so "reset restores factory values" has no observable meaning.

## File Structure

```
crates/mocksbr/src/
  state.rs        CREATE  DeviceState: working memory + NVRAM, and the
                          three transitions (set, save, reset)
  wire.rs         CREATE  decode_param_value, and the set_data request
                          parser. Encoding back to wire bytes is not here
                          -- `udap::parameters::Parameter::encode` already
                          does it, and Task 3 calls it directly
  network.rs      MODIFY  hold a DeviceState per device; route writes
  responses.rs    MODIFY  get_data answers from state; set_data acks
  lib.rs          MODIFY  export the new modules

crates/udap/tests/
  ops_config.rs   MODIFY  add the round-trip and reset assertions
```

**Not in this plan:** the fault-injection knobs (`FailOn`, `Slow`,
`Unreachable`, `Drop*`, `SuppressDiscoveryUUID`, `Malformed`) and the
reboot *delay* — plan B. The standalone binary — plan C.

---

### Task 1: `wire` — value codec and request parser

Pure functions, no state. First because everything else consumes them, and
because they are the part where a fidelity mistake is silent.

**Files:**
- Create: `crates/mocksbr/src/wire.rs`
- Modify: `crates/mocksbr/src/lib.rs`

**Interfaces:**
- Consumes: `udap::parameters::{by_offset, Parameter}` (exists)
- Produces:
  - `pub(crate) fn decode_param_value(value: &[u8]) -> Vec<u8>`
  - `pub(crate) struct SetDataItem { pub offset: u16, pub length: u16, pub value: Vec<u8>, pub name: Option<&'static str> }`
  - `pub(crate) fn parse_set_data_request(payload: &[u8]) -> Vec<SetDataItem>`

- [ ] **Step 1: Write the failing tests**

`crates/mocksbr/src/wire.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_byte_value_decodes_as_a_decimal_number() {
        assert_eq!(decode_param_value(&[6]), b"6");
        assert_eq!(decode_param_value(&[255]), b"255");
    }

    #[test]
    fn a_two_byte_value_decodes_as_a_big_endian_number() {
        assert_eq!(decode_param_value(&[0x01, 0x2c]), b"300");
    }

    #[test]
    fn a_four_byte_value_decodes_as_a_dotted_quad() {
        assert_eq!(decode_param_value(&[192, 168, 1, 50]), b"192.168.1.50");
    }

    #[test]
    fn a_string_value_stops_at_the_first_nul() {
        // NVRAM string fields are NUL-padded to their full width.
        assert_eq!(decode_param_value(b"hello\0\0\0"), b"hello");
    }

    #[test]
    fn a_string_value_with_no_nul_is_taken_whole() {
        assert_eq!(decode_param_value(b"abcde"), b"abcde");
    }

    #[test]
    fn a_non_utf8_string_value_survives_byte_exact() {
        // ADR-6: 802.11 does not require an SSID to be valid UTF-8, and
        // the mock must not be the thing that mangles it.
        let raw = [0xff, 0xfe, 0x41, 0x00, 0x00];
        assert_eq!(decode_param_value(&raw), vec![0xff, 0xfe, 0x41]);
    }

    #[test]
    fn a_request_decodes_into_its_items() {
        // 32 credential bytes, count=1, then offset/length/value.
        // Offset 4 is lan_ip_mode, a 1-byte parameter.
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&4u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(1);

        let items = parse_set_data_request(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].offset, 4);
        assert_eq!(items[0].length, 1);
        assert_eq!(items[0].value, vec![1]);
        assert_eq!(items[0].name, Some("lan_ip_mode"));
    }

    #[test]
    fn an_unknown_offset_parses_but_has_no_name() {
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&9999u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(7);

        let items = parse_set_data_request(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, None, "offset 9999 is not in the table");
    }

    #[test]
    fn a_truncated_item_stops_the_walk_without_panicking() {
        // count promises 2, only one complete item follows.
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&2u16.to_be_bytes());
        payload.extend_from_slice(&4u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(1);
        payload.extend_from_slice(&4u16.to_be_bytes()); // header cut short

        assert_eq!(parse_set_data_request(&payload).len(), 1);
    }

    #[test]
    fn a_payload_too_short_for_the_count_yields_nothing() {
        assert!(parse_set_data_request(&[0u8; 10]).is_empty());
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `mise exec -- cargo test -p mocksbr --lib wire`
Expected: compile error — the module does not exist. Add it with
`decode_param_value` returning `value.to_vec()` unconditionally and
`parse_set_data_request` returning `Vec::new()`; both compile cleanly and
both are wrong, so the failures become assertions.

- [ ] **Step 3: Implement the codec and parser**

```rust
//! Wire encoding for the mock's parameter values, and the `set_data`
//! request parser.
//!
//! Deliberately a separate implementation from `udap`'s `format_value`,
//! not a shared one. go-udap duplicates it the same way — but the better
//! reason is independence: if the mock decoded with the client's own
//! decoder, a bug in that decoder would be invisible, because both sides
//! of a round-trip would agree on the wrong answer.

use udap::parameters;

/// Credential fields preceding the item list in a `set_data` request.
const CREDENTIAL_FIELDS: usize = 32;

/// Renders a raw NVRAM value as its display form, dispatching on width.
///
/// Mirrors go-udap's `decodeParamValue`: 1 byte is a decimal number,
/// 2 bytes a big-endian decimal number, 4 bytes a dotted quad, anything
/// else a NUL-terminated string.
///
/// Returns `Vec<u8>`, not `String`: NVRAM strings carry no encoding
/// guarantee (ADR-6), and the mock must not be what mangles them.
pub(crate) fn decode_param_value(value: &[u8]) -> Vec<u8> {
    match value.len() {
        1 => value[0].to_string().into_bytes(),
        2 => u16::from_be_bytes([value[0], value[1]]).to_string().into_bytes(),
        4 => format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]).into_bytes(),
        _ => {
            let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
            value[..end].to_vec()
        }
    }
}

/// One offset/length/value triple from a `set_data` request.
pub(crate) struct SetDataItem {
    pub offset: u16,
    pub length: u16,
    pub value: Vec<u8>,
    /// `None` when the offset is not in the parameter table.
    pub name: Option<&'static str>,
}

/// Decodes a `set_data` request body.
///
/// A truncated item ends the walk and keeps what was read, matching
/// go-udap: a malformed tail must not discard a well-formed prefix.
pub(crate) fn parse_set_data_request(payload: &[u8]) -> Vec<SetDataItem> {
    if payload.len() < CREDENTIAL_FIELDS + 2 {
        return Vec::new();
    }
    let mut pos = CREDENTIAL_FIELDS;
    let count = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
    pos += 2;

    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if pos + 4 > payload.len() {
            break;
        }
        let offset = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let length = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]);
        pos += 4;
        let end = pos + usize::from(length);
        if end > payload.len() {
            break;
        }
        out.push(SetDataItem {
            offset,
            length,
            value: payload[pos..end].to_vec(),
            name: parameters::by_offset(offset).map(|p| p.name),
        });
        pos = end;
    }
    out
}
```

`.unwrap_or(value.len())` here is `Option::unwrap_or`, not
`Result::unwrap` — it is not the denied lint and needs no suppression.

- [ ] **Step 4: Run the tests and watch them pass**

Run: `mise exec -- cargo test -p mocksbr --lib wire`
Expected: 10 passed.

- [ ] **Step 5: Export the module**

In `crates/mocksbr/src/lib.rs`, add `mod wire;` (private — nothing
outside the crate needs it).

- [ ] **Step 6: Mutation-check the two rules most likely to rot**

```bash
# The NUL truncation. Expect a_string_value_stops_at_the_first_nul to fail.
# The dispatch widths. Change `2 =>` to `3 =>`; expect the two-byte test to fail.
```

Verify each edit actually landed (`rg` for the changed text) before
believing a "survived" result, then restore.

- [ ] **Step 7: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src/wire.rs crates/mocksbr/src/lib.rs
git commit -m "feat(mocksbr): add the parameter value codec and set_data parser"
```

---

### Task 2: `state` — working memory and NVRAM

The state model, still with no wiring into the network. Testable entirely
on its own.

**Files:**
- Create: `crates/mocksbr/src/state.rs`
- Modify: `crates/mocksbr/src/lib.rs`

**Interfaces:**
- Consumes: `udap::parameters::PARAMETERS` (exists)
- Produces:
  - `pub(crate) struct DeviceState`
  - `pub(crate) fn DeviceState::factory() -> Self`
  - `pub(crate) fn DeviceState::get(&self, name: &str) -> Option<&[u8]>`
  - `pub(crate) fn DeviceState::apply_set(&mut self, updates: BTreeMap<String, Vec<u8>>)`
  - `pub(crate) fn DeviceState::apply_save(&mut self)`
  - `pub(crate) fn DeviceState::apply_reset(&mut self)`

- [ ] **Step 1: Write the failing tests**

`crates/mocksbr/src/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str, value: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut map = BTreeMap::new();
        map.insert(name.to_owned(), value.to_vec());
        map
    }

    #[test]
    fn a_factory_device_reports_the_table_defaults() {
        let state = DeviceState::factory();
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
        assert_eq!(state.get("lan_subnet_mask"), Some(b"255.255.255.0".as_slice()));
    }

    #[test]
    fn a_factory_device_knows_every_parameter() {
        let state = DeviceState::factory();
        for name in udap::parameters::names() {
            assert!(state.get(name).is_some(), "{name} missing from factory state");
        }
    }

    #[test]
    fn set_changes_what_get_reports() {
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        assert_eq!(state.get("wireless_channel"), Some(b"11".as_slice()));
    }

    #[test]
    fn set_leaves_other_parameters_alone() {
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        assert_eq!(state.get("wireless_region_id"), Some(b"4".as_slice()));
    }

    #[test]
    fn reset_without_a_save_discards_the_change() {
        // Working memory reloads from NVRAM, which never saw the write.
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        state.apply_reset();
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
    }

    #[test]
    fn reset_after_a_save_keeps_the_change() {
        // This is the pair that gives save a meaning: the same reset
        // yields a different answer depending on whether save ran.
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_channel", b"11"));
        state.apply_save();
        state.apply_reset();
        assert_eq!(state.get("wireless_channel"), Some(b"11".as_slice()));
    }

    #[test]
    fn save_does_not_disturb_working_memory() {
        let mut state = DeviceState::factory();
        state.apply_set(one("hostname", b"bedroom"));
        state.apply_save();
        assert_eq!(state.get("hostname"), Some(b"bedroom".as_slice()));
    }

    #[test]
    fn a_non_utf8_value_survives_the_full_cycle() {
        // ADR-6, end to end through the state model.
        let ssid = vec![0xff, 0xfe, 0x41];
        let mut state = DeviceState::factory();
        state.apply_set(one("wireless_SSID", &ssid));
        state.apply_save();
        state.apply_reset();
        assert_eq!(state.get("wireless_SSID"), Some(ssid.as_slice()));
    }

    #[test]
    fn an_unknown_name_is_stored_but_does_not_displace_a_known_one() {
        let mut state = DeviceState::factory();
        state.apply_set(one("not_a_real_parameter", b"x"));
        assert_eq!(state.get("not_a_real_parameter"), Some(b"x".as_slice()));
        assert_eq!(state.get("wireless_channel"), Some(b"6".as_slice()));
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `mise exec -- cargo test -p mocksbr --lib state`
Expected: compile error, then assertion failures once a stub exists.
Stub `factory()` as two empty maps — every `get` returns `None`, which
fails on content rather than on compilation.

- [ ] **Step 3: Implement the state model**

```rust
//! A virtual device's parameter state.
//!
//! Two tiers, mirroring a real Squeezebox: *working memory* is what the
//! device is running, *NVRAM* is what survives a reboot. `set_data`
//! writes working memory, `save` copies it to NVRAM, and `reset` reloads
//! working memory from NVRAM.
//!
//! go-udap's mock saves on every set, so a later reset observes the most
//! recent values — matching real-SBR behaviour on the test bench. The
//! two transitions stay separate here so a test can exercise a set
//! *without* a save and see the change discarded.

use std::collections::BTreeMap;
use udap::parameters;

type Params = BTreeMap<String, Vec<u8>>;

pub(crate) struct DeviceState {
    working: Params,
    nvram: Params,
}

impl DeviceState {
    /// A device in factory condition: both tiers hold the parameter
    /// table's factory defaults.
    pub(crate) fn factory() -> Self {
        let defaults: Params = parameters::PARAMETERS
            .iter()
            .map(|p| (p.name.to_owned(), p.factory_default.as_bytes().to_vec()))
            .collect();
        DeviceState {
            working: defaults.clone(),
            nvram: defaults,
        }
    }

    /// The value the device is currently running.
    pub(crate) fn get(&self, name: &str) -> Option<&[u8]> {
        self.working.get(name).map(Vec::as_slice)
    }

    /// Applies a write to working memory. Unrecognised names are stored
    /// as given — the mock does not second-guess the client.
    pub(crate) fn apply_set(&mut self, updates: Params) {
        self.working.extend(updates);
    }

    /// Commits working memory to NVRAM.
    pub(crate) fn apply_save(&mut self) {
        self.nvram = self.working.clone();
    }

    /// Reloads working memory from NVRAM, discarding uncommitted writes.
    pub(crate) fn apply_reset(&mut self) {
        self.working = self.nvram.clone();
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `mise exec -- cargo test -p mocksbr --lib state`
Expected: 9 passed.

- [ ] **Step 5: Mutation-check the tier separation**

Make `apply_reset` a no-op: `reset_without_a_save_discards_the_change`
must fail. Make `apply_save` a no-op: `reset_after_a_save_keeps_the_change`
must fail. If either survives, the two tiers are not actually distinct.

- [ ] **Step 6: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src/state.rs crates/mocksbr/src/lib.rs
git commit -m "feat(mocksbr): add working memory and NVRAM state"
```

---

### Task 3: Wire the state into the network

Where the mock stops being a stub. `Network` gains a `DeviceState` per
device, `get_data` answers from working memory, and `set_data` writes it.

**Files:**
- Modify: `crates/mocksbr/src/network.rs`, `crates/mocksbr/src/responses.rs`
- Test: `crates/udap/tests/ops_config.rs`

**Interfaces:**
- Consumes: `DeviceState`, `parse_set_data_request`, `decode_param_value`
- Produces:
  - `responses::get_data_response(request: &Packet, cfg: &DeviceConfig, state: &DeviceState, payload: &[u8]) -> Vec<u8>`
  - `responses::set_data_response(request: &Packet, cfg: &DeviceConfig, accepted: u16) -> Vec<u8>`

**`Network::receive` takes `&self` today and the state must now change,
so the per-device state goes behind a `std::sync::Mutex`.** `receive` is
synchronous and holds the lock only while building one reply, so there is
no await across it and `clippy::await_holding_lock` does not fire. Do not
reach for `tokio::sync::Mutex` — it would force `receive` async and
ripple through `MockTransport`.

- [ ] **Step 1: Write the failing round-trip test**

Append to `crates/udap/tests/ops_config.rs`:

```rust
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
```

- [ ] **Step 2: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: `a_set_is_visible_to_a_later_get` fails with `Some("6")` — the
factory default, proving the write went nowhere.

- [ ] **Step 3: Give `Network` per-device state**

In `crates/mocksbr/src/network.rs`, hold the state beside each config:

```rust
use crate::state::DeviceState;
use std::sync::Mutex;

pub struct Network {
    devices: Vec<DeviceConfig>,
    /// Per-device parameter state, indexed in step with `devices`.
    ///
    /// A `std::sync::Mutex`, not tokio's: `receive` is synchronous and
    /// holds the lock only while building one reply, so nothing awaits
    /// across it. tokio's would force `receive` async and ripple into
    /// `MockTransport`.
    state: Mutex<Vec<DeviceState>>,
}
```

Build one `DeviceState::factory()` per device in `Network::new` and
`with_auto_devices`.

- [ ] **Step 4: Answer `get_data` from working memory**

Change `responses::get_data_response` to take `&DeviceState` and read
each requested offset's value from it rather than from
`param.factory_default`:

```rust
if let Some(param) = udap::parameters::by_offset(offset) {
    let display = state.get(param.name).unwrap_or(b"");
    if let Ok(encoded) = param.encode(display) {
        items.push((offset, encoded));
    }
}
```

`unwrap_or` on an `Option<&[u8]>` is not the denied `Result::unwrap`.

- [ ] **Step 5: Make `set_data` write, and ack with the count**

In `network.rs`'s dispatch, `set_data` now mutates. go-udap applies the
set *and* the save on every write, so a later reset observes the most
recent values:

```rust
method::SET_DATA => {
    let items = crate::wire::parse_set_data_request(payload);
    let mut updates = BTreeMap::new();
    for item in &items {
        if let Some(name) = item.name {
            updates.insert(name.to_owned(), crate::wire::decode_param_value(&item.value));
        }
    }
    let accepted = u16::try_from(items.len()).unwrap_or(u16::MAX);
    // lock, apply_set(updates), apply_save(), drop the guard
    responses::set_data_response(&request, cfg, accepted)
}
```

The ack carries a 2-byte big-endian count of items accepted, appended to
the header — go-udap's `buildSetDataAck`. Note it counts **items parsed**,
including ones whose offset is unknown, not the number applied.

- [ ] **Step 6: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: all pass, including the three new ones.

- [ ] **Step 7: Mutation-check the round trip**

Revert `get_data_response` to answering from `factory_default`:
`a_set_is_visible_to_a_later_get` must fail. If it does not, the test is
reading the client's cache rather than the device's answer — the same
trap that made M4's clobbering test toothless.

- [ ] **Step 8: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src crates/udap/tests/ops_config.rs
git commit -m "feat(mocksbr): answer get_data from device state"
```

---

### Task 4: `reset` reloads from NVRAM

The last piece of the state model, and the one that gives `save` an
observable meaning.

**Files:**
- Modify: `crates/mocksbr/src/network.rs`
- Test: `crates/udap/tests/ops_config.rs`

**Interfaces:**
- Consumes: `DeviceState::apply_reset`
- Produces: no new API; `reset` becomes stateful

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn reset_restores_the_saved_values() {
    // mocksbr saves on every set, matching go-udap, so a reset reloads
    // the most recent write rather than the factory default. The
    // distinction that matters: reset is no longer a no-op.
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
```

- [ ] **Step 2: Run and watch it fail**

Run: `mise exec -- cargo test -p udap --test ops_config::reset_restores_the_saved_values`
Expected: passes *accidentally* — working memory already holds `11` and
reset currently does nothing. **That is a failing test in the sense that
matters: it does not discriminate.** Prove it by temporarily making
`apply_reset` restore factory defaults instead of NVRAM; the test must
then fail. Restore, then implement properly. Record this in the commit —
a test that cannot fail is not yet a test.

- [ ] **Step 3: Make `reset` reload**

In `network.rs`'s dispatch, before building the ack:

```rust
method::RESET => {
    // Reload working memory from NVRAM. go-udap serves the ack first
    // and then enters the reboot window; the window itself is plan B.
    // lock, apply_reset(), drop the guard
    responses::reset_response(&request, cfg)
}
```

- [ ] **Step 4: Run the whole suite**

Run: `mise exec -- cargo test --workspace`
Expected: all pass.

- [ ] **Step 5: Mutation-check**

Make `apply_reset` a no-op and confirm a test notices. If none does, add
one that sets, saves, sets again *without* saving, resets, and asserts
the second write was discarded — that is the assertion `apply_reset`
cannot satisfy by doing nothing.

- [ ] **Step 6: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/mocksbr/src/network.rs crates/udap/tests/ops_config.rs
git commit -m "feat(mocksbr): reload working memory from NVRAM on reset"
```

---

## Acceptance

M5-A is done when a `set` is observable through a later `get`, and a
`reset` reloads from NVRAM:

```bash
mise exec -- cargo test --workspace          # all green
mise exec -- cargo clippy --all-targets --all-features   # silent
mise exec -- cargo fmt --all --check          # clean
```

The M4 test `set_records_the_new_value_after_the_device_acknowledges`
becomes meaningfully stronger for free: it asserted only that the
client's cache updated, and the device now genuinely holds the value.

## Deferred

- **Fault-injection knobs** — `FailOn`, `Slow`, `Unreachable`,
  `DropGetData`/`DropGetIP`/`DropGetUUID`, `SuppressDiscoveryUUID`,
  `Malformed`. Plan B. `Slow` and `RebootDelay` need scheduled replies
  (go-udap returns `ScheduledReply { Bytes, Delay }`), which is the one
  piece of M5 that forces a time dimension into the mock — worth its own
  design pass rather than being smuggled in here.
- **The reboot window** after `reset`, during which a real device drops
  every packet. Plan B, with `Slow`.
- **The standalone binary.** Plan C.
- **`DeviceConfig.nvram` seed overrides** — letting a test start a device
  in a non-factory state. Not needed by any test this plan writes; add it
  when one wants it.
