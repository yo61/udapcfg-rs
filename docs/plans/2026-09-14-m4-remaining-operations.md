# udapcfg-rs M4 Implementation Plan — remaining UCP operations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the five remaining UCP operations — `get_data`, `set_data`, `reset`, `get_ip`, `get_uuid` — so `udap` becomes a complete, standalone Rust UDAP library.

**Architecture:** Split `Client`'s two jobs. A new `Session` owns the transport, the retry count and the sequence counter, and knows how to send a request and wait for *one named device's* reply. Operations become free functions in `ops/` taking `(&Session, &CancellationToken, &mut Device, …)`. `Client` keeps discovery and the device registry, hands devices out with `take_device`, and wraps each operation so call sites still read like go-udap's. This is what the borrow checker wants — `&self` plus `&mut self.devices[…]` is two overlapping borrows — and it happens to match what the code actually needs (see "Why the seam" below).

**Tech Stack:** Rust 1.98.1, tokio (current_thread), `thiserror`, `tokio-util::CancellationToken`. No new dependencies.

**Spec:** [`docs/specs/2026-09-08-rust-port-spec.md`](../specs/2026-09-08-rust-port-spec.md)
**Reference:** [`docs/port-map.md`](../port-map.md)
**Prior plan:** [`2026-09-08-m3-real-udp-transport.md`](2026-09-08-m3-real-udp-transport.md)

## Global Constraints

- **Source of truth is go-udap v2.4.8 (`43864a5`)** at `~/code/github.com/yo61/go-udap`. Where this plan and the Go disagree, the Go wins — read it and fix the plan. (`v2.4.9` and later are byte-identical in `*.go`; verified in M3.)
- **The toolchain is mise-managed and NOT on PATH.** Prefix every cargo command: `mise exec -- cargo ...` from the repo root.
- **Zero warnings.** `cargo clippy --all-targets --all-features` and `cargo fmt --all --check` clean. `RUSTFLAGS=-D warnings` is set in `mise.toml`.
- **No `#[allow(...)]`.** Suppress only with `#[expect(lint, reason = "...")]`, and only when a rewrite genuinely cannot satisfy the lint.
- **No `.unwrap()`/`.expect()` in non-test code.** `clippy.toml` exempts tests, but only in frames carrying `#[test]`/`#[tokio::test]` — a fallible call in a plain helper fn still fires.
- **`clippy::panic` is denied everywhere, including tests.** Use `assert!`/`assert_eq!`, never `panic!`.
- **`clippy::print_stdout`/`print_stderr` are denied.** All output through injected writers.
- **`crates/udap/src/lib.rs` is append-only.** Widening a `pub use` list is fine; losing a name is not.
- **Device-supplied values are bytes.** ADR-6: NVRAM values and discovery TLV strings are `Vec<u8>`, never `String`. User *input* (CLI flags, config files) is `&str`.
- **Error text is fidelity-contract text.** Every message this plan quotes from go-udap is user-visible and must match byte-for-byte. Where the plan gives a literal string, copy it exactly.
- **TDD.** Failing test first, watch it fail, then implement.
- **Commit per task**, conventional-commit format, on a feature branch. Never commit to `main`.
- **Pushing runs the lastlight gate.** Each push needs an independent review recorded for that exact SHA. Batch all edits, then review once, then push.

## Why the seam (read before Task 2)

Measured in go-udap, not assumed:

- **Nothing re-reads the client's map after an operation.** The only non-test references to `c.devices` are a lookup *before* an operation (`udap/client.go:368`) and the write during discovery (`:396`). The pointer aliasing Go relies on is real but **not load-bearing** — so Rust does not need to reproduce it.
- **The `Device` genuinely is stateful across calls**, in three distinct ways, all of which must be ported:
  1. **Output channel.** `GetAllDeviceConfigWithContext` returns only `error`; it writes results into `device.Parameters`, and `cli/read.go:64` reads them back.
  2. **Cross-call cache.** `cli/set.go:137` pre-populates `Parameters` so `SetDeviceConfig` skips its read-modify-write prelude — one round-trip instead of two.
  3. **Commit barrier.** `udap/config.go:151` merges the caller's overrides into `Parameters` **only after** the device acknowledges. The comment records why: mutating earlier "left device.Parameters showing values that were never persisted to NVRAM if the round-trip failed."
- `waitForDeviceReply` and `sendRetried` need only the transport, the retry count and a **read-only** view of the device. Neither touches the registry. That is exactly `Session`.

## File Structure

```
crates/udap/src/
  session.rs        CREATE  Session: transport + retries + sequence;
                            send_retried(), wait_for_reply(), header()
  ops/mod.rs        CREATE  OpError; re-exports
  ops/config.rs     CREATE  get(), get_all(), set(), reset()
  ops/getip.rs      CREATE  get_ip(), parse_response()
  ops/getuuid.rs    CREATE  get_uuid(), parse_response()
  netconfig.rs      CREATE  NetworkConfig + Display
  validation.rs     CREATE  validate_parameter()
  device.rs         MODIFY  add `parameters: BTreeMap<String, Vec<u8>>`
  client.rs         MODIFY  hold a Session; take_device(); op wrappers
  lib.rs            MODIFY  append `mod`/`pub use` entries

crates/udap/tests/
  fixtures/*.bin    CREATE  five captures copied from go-udap
  golden_capture.rs MODIFY  add fixture assertions per operation
```

**Not in this plan:** the CLI subcommands (`read`, `get`, `set`, `reboot`, `getip`, `info`) — that is M6. M4 stops at the library boundary, with `mocksbr` as the only consumer.

---

### Task 1: `netconfig` and `validation`

Two pure modules, no I/O and no async. Deliberately first: they are mechanical, they unblock later tasks, and they get the error-text fidelity work done while it is the only thing to think about.

**Files:**
- Create: `crates/udap/src/netconfig.rs`, `crates/udap/src/validation.rs`
- Modify: `crates/udap/src/lib.rs`

**Interfaces:**
- Consumes: `crate::parameters::{by_name, Parameter}` (exists)
- Produces:
  - `pub struct NetworkConfig { pub ip: Option<Ipv4Addr>, pub subnet_mask: Option<Ipv4Addr>, pub gateway: Option<Ipv4Addr> }`
  - `impl Display for NetworkConfig`
  - `pub fn validate_parameter(name: &str, value: &str) -> Result<(), ValidationError>`
  - `pub enum ValidationError` (thiserror)

- [ ] **Step 1: Write the failing tests for `netconfig`**

`crates/udap/src/netconfig.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_fields_render_as_a_dash() {
        let nc = NetworkConfig::default();
        assert_eq!(nc.to_string(), "IP:      -\nSubnet:  -\nGateway: -");
    }

    #[test]
    fn an_unspecified_address_also_renders_as_a_dash() {
        // go-udap's ipOrDash treats 0.0.0.0 as absent, not as an address.
        // A device that omits the gateway TLV and one that reports
        // 0.0.0.0 must print identically.
        let nc = NetworkConfig {
            ip: Some(Ipv4Addr::new(192, 168, 1, 50)),
            subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            gateway: Some(Ipv4Addr::UNSPECIFIED),
        };
        assert_eq!(
            nc.to_string(),
            "IP:      192.168.1.50\nSubnet:  255.255.255.0\nGateway: -"
        );
    }

    #[test]
    fn present_fields_render_as_dotted_quads() {
        let nc = NetworkConfig {
            ip: Some(Ipv4Addr::new(10, 0, 0, 5)),
            subnet_mask: Some(Ipv4Addr::new(255, 0, 0, 0)),
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        };
        assert_eq!(
            nc.to_string(),
            "IP:      10.0.0.5\nSubnet:  255.0.0.0\nGateway: 10.0.0.1"
        );
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `mise exec -- cargo test -p udap --lib netconfig`
Expected: compile error — `NetworkConfig` not found. Add the struct with `todo!()`-free stubs (a `Display` impl returning `String::new()`) until the failures are assertion failures rather than compile errors.

- [ ] **Step 3: Implement `netconfig`**

```rust
//! The result of a `get_ip` (0x0002) query.
//!
//! Distinct from `Device`: `Device` is what discovery passively
//! observed; `NetworkConfig` is what the device reports when asked.

use std::fmt;
use std::net::Ipv4Addr;

/// Every field is optional — devices omit TLVs, notably `gateway` on a
/// static address with no gateway configured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkConfig {
    pub ip: Option<Ipv4Addr>,
    pub subnet_mask: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
}

/// Renders an address, or `-` when it is absent *or* unspecified.
///
/// go-udap's `ipOrDash` collapses both cases, so a device that omits the
/// TLV and one that reports `0.0.0.0` print identically. `Option` alone
/// would not reproduce that.
fn or_dash(addr: Option<Ipv4Addr>) -> String {
    match addr {
        Some(a) if !a.is_unspecified() => a.to_string(),
        _ => "-".to_owned(),
    }
}

impl fmt::Display for NetworkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IP:      {}\nSubnet:  {}\nGateway: {}",
            or_dash(self.ip),
            or_dash(self.subnet_mask),
            or_dash(self.gateway)
        )
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `mise exec -- cargo test -p udap --lib netconfig`
Expected: 3 passed.

- [ ] **Step 5: Write the failing tests for `validation`**

Error strings are copied verbatim from `udap/validation.go:35-97`. They are user-visible; do not reword them.

`crates/udap/src/validation.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_parameter_is_allowed() {
        // go-udap: "Unknown parameter, but we'll allow it".
        assert!(validate_parameter("not_a_real_param", "anything").is_ok());
    }

    #[test]
    fn a_one_byte_parameter_rejects_non_numeric_input() {
        let err = validate_parameter("lan_ip_mode", "1abc")
            .expect_err("must reject");
        assert_eq!(err.to_string(), r#"expected numeric value (0-255), got "1abc""#);
    }

    #[test]
    fn a_one_byte_parameter_rejects_values_above_255() {
        let err = validate_parameter("lan_ip_mode", "256")
            .expect_err("must reject");
        assert_eq!(err.to_string(), r#"expected numeric value (0-255), got "256""#);
    }

    #[test]
    fn a_numeric_parameter_rejects_a_leading_plus() {
        // Rust's FromStr accepts "+1" for unsigned ints; Go's ParseUint
        // does not. The error text is fidelity-contract, so the two
        // implementations must agree on what is valid.
        let err = validate_parameter("lan_ip_mode", "+1")
            .expect_err("Go rejects a signed value here");
        assert_eq!(err.to_string(), r#"expected numeric value (0-255), got "+1""#);
    }

    #[test]
    fn an_ip_parameter_rejects_a_non_address() {
        let err = validate_parameter("lan_gateway", "not-an-ip")
            .expect_err("must reject");
        assert_eq!(
            err.to_string(),
            r#"expected valid IPv4 address, got "not-an-ip""#
        );
    }

    #[test]
    fn an_ip_parameter_accepts_a_dotted_quad() {
        assert!(validate_parameter("lan_gateway", "192.168.1.1").is_ok());
    }

    #[test]
    fn a_string_parameter_rejects_an_overlong_value() {
        // squeezecenter_name is a 33-byte field; 34 chars must not fit.
        let long = "x".repeat(34);
        let err = validate_parameter("squeezecenter_name", &long)
            .expect_err("must reject");
        assert_eq!(err.to_string(), "value too long (max 33 chars), got 34");
    }

    #[test]
    fn wireless_channel_must_be_1_to_13() {
        assert!(validate_parameter("wireless_channel", "6").is_ok());
        let err = validate_parameter("wireless_channel", "14")
            .expect_err("must reject");
        assert_eq!(err.to_string(), "must be between 1 and 13");
    }

    #[test]
    fn wireless_mode_must_be_0_or_1() {
        assert!(validate_parameter("wireless_mode", "0").is_ok());
        let err = validate_parameter("wireless_mode", "2").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 0 (infrastructure) or 1 (ad-hoc)");
    }

    #[test]
    fn wireless_keylen_must_be_5_or_13() {
        let err = validate_parameter("wireless_keylen", "8").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 5 or 13 for WEP keys");
    }

    #[test]
    fn wireless_wpa_psk_must_be_8_to_63_characters() {
        let err = validate_parameter("wireless_wpa_psk", "short")
            .expect_err("must reject");
        assert_eq!(err.to_string(), "must be 8-63 characters");
    }

    #[test]
    fn wireless_ssid_must_be_1_to_32_characters() {
        let err = validate_parameter("wireless_SSID", "").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 1-32 characters");
    }
}
```

- [ ] **Step 6: Run the tests and watch them fail**

Run: `mise exec -- cargo test -p udap --lib validation`
Expected: every test fails; the messages are the point.

- [ ] **Step 7: Implement `validation`**

Note the ordering: the width check runs first, then the parameter-specific rule, exactly as `validateParameter` does. A value can fail either.

```rust
//! Per-parameter input rules for values the *user* supplies.
//!
//! Input is `&str`, not bytes: ADR-6 makes device-supplied values
//! `Vec<u8>`, but these rules run on CLI flags and config-file entries,
//! which are text by construction.

use crate::parameters;
use std::net::Ipv4Addr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("expected numeric value (0-255), got {0:?}")]
    NotU8(String),
    #[error("expected numeric value (0-65535), got {0:?}")]
    NotU16(String),
    #[error("expected valid IPv4 address, got {0:?}")]
    NotIpv4(String),
    #[error("value too long (max {max} chars), got {got}")]
    TooLong { max: u16, got: usize },
    #[error("must be 0 (infrastructure) or 1 (ad-hoc)")]
    WirelessMode,
    #[error("must be between 1 and 13")]
    WirelessChannel,
    #[error("must be 5 or 13 for WEP keys")]
    WirelessKeylen,
    #[error("must be 8-63 characters")]
    WpaPskLength,
    #[error("must be 1-32 characters")]
    SsidLength,
}

/// Validates a user-supplied value for `name`.
///
/// An unrecognised `name` is accepted, matching go-udap: the CLI can
/// carry parameters this table does not know about.
///
/// # Errors
/// [`ValidationError`] describing the first rule the value fails.
pub fn validate_parameter(name: &str, value: &str) -> Result<(), ValidationError> {
    let Some(param) = parameters::by_name(name) else {
        return Ok(());
    };

    match param.length {
        1 => {
            if !parses_as_unsigned::<u8>(value) {
                return Err(ValidationError::NotU8(value.to_owned()));
            }
        }
        2 => {
            if !parses_as_unsigned::<u16>(value) {
                return Err(ValidationError::NotU16(value.to_owned()));
            }
        }
        4 => {
            if value.parse::<Ipv4Addr>().is_err() {
                return Err(ValidationError::NotIpv4(value.to_owned()));
            }
        }
        max => {
            if value.len() > usize::from(max) {
                return Err(ValidationError::TooLong {
                    max,
                    got: value.len(),
                });
            }
        }
    }

    match name {
        "wireless_mode" => {
            if value != "0" && value != "1" {
                return Err(ValidationError::WirelessMode);
            }
        }
        "wireless_channel" => {
            let channel = if parses_as_unsigned::<u32>(value) {
                value.parse::<u32>().ok()
            } else {
                None
            };
            match channel {
                Some(ch) if (1..=13).contains(&ch) => {}
                _ => return Err(ValidationError::WirelessChannel),
            }
        }
        "wireless_keylen" => {
            if value != "5" && value != "13" {
                return Err(ValidationError::WirelessKeylen);
            }
        }
        "wireless_wpa_psk" => {
            if value.len() < 8 || value.len() > 63 {
                return Err(ValidationError::WpaPskLength);
            }
        }
        "wireless_SSID" => {
            if value.is_empty() || value.len() > 32 {
                return Err(ValidationError::SsidLength);
            }
        }
        _ => {}
    }

    Ok(())
}
```

**Two fidelity notes for the implementer:**

1. `{0:?}` on a `String` produces Rust's debug quoting. Go's `%q` is Go-syntax quoting. They agree for ASCII — which every value in the tests is — but diverge on non-ASCII and some escapes. If a test ever exercises a non-ASCII value here, pin the expected bytes against `go-udap` rather than trusting the formats to match.
2. **`value.parse::<u8>()` is not equivalent to Go's `ParseUint`.** It agrees
   on `"1abc"` (rejected) and `"256"` (rejected), but Rust's integer `FromStr`
   accepts an optional leading `+` for unsigned types — `"+1".parse::<u8>()`
   is `Ok(1)`, verified. Go's `ParseUint` has no sign handling at all, so
   `ParseUint("+1", 10, 8)` is a syntax error. Reject the sign explicitly:

   ```rust
   /// Go's `strconv.ParseUint` semantics: digits only, no sign.
   ///
   /// Rust's `FromStr` accepts a leading `+` for unsigned integers; Go's
   /// does not, and this value reaches a fidelity-contract error message.
   fn parses_as_unsigned<T: std::str::FromStr>(value: &str) -> bool {
       !value.starts_with('+') && value.parse::<T>().is_ok()
   }
   ```

   Use it for the 1-byte, 2-byte and `wireless_channel` checks alike. The Go
   comment explicitly notes `ParseUint` was chosen over `fmt.Sscanf` because
   the latter accepts `"1abc"` — so do not reach for a permissive scan either.

- [ ] **Step 8: Run the tests and watch them pass**

Run: `mise exec -- cargo test -p udap --lib validation`
Expected: 12 passed.

- [ ] **Step 9: Wire both modules into the crate**

Append to `crates/udap/src/lib.rs` (append-only — do not reorder or remove):

```rust
pub mod netconfig;
pub mod validation;

pub use netconfig::NetworkConfig;
pub use validation::{ValidationError, validate_parameter};
```

- [ ] **Step 10: Verify the whole gate**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
```
Expected: fmt clean, clippy silent, all tests pass.

- [ ] **Step 11: Commit**

```bash
git add crates/udap/src/netconfig.rs crates/udap/src/validation.rs crates/udap/src/lib.rs
git commit -m "feat(udap): add netconfig and parameter validation"
```

---

### Task 2: The `Session` seam

Extract the wire half of `Client` so operations can borrow it independently of the device registry. **No behaviour changes** — this task must leave every existing test passing untouched. That is the point: it proves the seam before anything depends on it.

**Files:**
- Create: `crates/udap/src/session.rs`, `crates/udap/src/ops/mod.rs`
- Modify: `crates/udap/src/client.rs`, `crates/udap/src/lib.rs`

`ops/mod.rs` is created **here**, not in Task 3: `wait_for_reply` returns
`OpError`, so the type must exist before this task compiles. Task 3 adds the
`pub mod config/getip/getuuid` lines and the remaining variants.

**Interfaces:**
- Consumes: `crate::transport::{Transport, TransportError}`, `crate::protocol::{Packet, …}`, `crate::device::Device`
- Produces:
  - `pub struct Session { transport: Box<dyn Transport>, retries: usize, sequence: AtomicU16 }`

    **`sequence` must be an `AtomicU16`, not a plain field.** `client.rs:91`
    increments it per request (`self.sequence.wrapping_add(1)`) from a
    `&mut self` method, but every operation in Tasks 3-5 holds only
    `&Session` — `Client::get_ip(&self, …)` passes `&self.session`. A plain
    field cannot be incremented through a shared borrow. Use
    `sequence.fetch_add(1, Ordering::Relaxed)` in `header()`; `Relaxed` is
    right because nothing orders other memory against it. Note the width:
    the existing field is `u16`, matching the wire format.

    **Mind the off-by-one.** `client.rs:91-99` increments *then* reads, so the
    first packet a client sends carries `sequence = 1`. `fetch_add` returns the
    value from *before* the addition, which would make the first packet
    `sequence = 0` and shift every later one down by one. Write it as:

    ```rust
    let sequence = self.sequence.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    ```

    This is not cosmetic: the golden captures in Task 4 and Task 5 were taken
    with `sequence=1`, so an off-by-one here fails those fixture comparisons
    with a one-byte diff that is easy to misread as an encoding bug.
  - `pub fn Session::new(transport: Box<dyn Transport>) -> Self`
  - `pub fn Session::set_retries(&mut self, n: usize)` / `pub fn retries(&self) -> usize`
  - `pub(crate) fn Session::header(&self, dst: Mac, ucp_method: u16, broadcast: bool) -> Packet`

    Keep the `broadcast` parameter. The method being moved is
    `Client::next_packet(&mut self, dst, ucp_method, broadcast)`
    (`client.rs:90`), it sets `dst_broadcast: u8::from(broadcast)`, and
    `discover()` calls it with `true` (`client.rs:143`). Dropping the flag
    would silently unset the broadcast bit on the discovery packet. The five
    M4 operations are all device-directed and pass `false`.
  - `pub(crate) async fn Session::send_retried(&self, packet: &[u8]) -> Result<(), TransportError>`
  - `pub(crate) async fn Session::wait_for_reply(&self, cancel: &CancellationToken, device: &Device) -> Result<(Packet, Vec<u8>), OpError>`
  - `pub async fn Session::close(&self) -> Result<(), TransportError>`

- [ ] **Step 1: Write the failing test for reply matching**

This is the only genuinely new behaviour in the task. `wait_for_reply` must skip replies from other devices and keep waiting, not return them.

`crates/udap/src/session.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportError;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Hands out a scripted sequence of packets, one per `recv`.
    struct ScriptedTransport {
        replies: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait]
    impl Transport for ScriptedTransport {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            Ok(())
        }
        async fn recv(
            &self,
            _cancel: &CancellationToken,
        ) -> Result<(Vec<u8>, String), TransportError> {
            let mut replies = self.replies.lock().map_err(|_| {
                TransportError::Io(std::io::Error::other("poisoned"))
            })?;
            if replies.is_empty() {
                return Err(TransportError::Cancelled);
            }
            Ok((replies.remove(0), "mock".to_owned()))
        }
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    fn reply_from(mac: Mac) -> Vec<u8> {
        let packet = Packet {
            dst_broadcast: 0,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: mac,
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0, // reply: request bit clear
            uap_class: UAP_CLASS_UCP,
            ucp_method: crate::protocol::method::GET_DATA,
        };
        packet.to_bytes().to_vec()
    }

    #[tokio::test]
    async fn wait_for_reply_skips_other_devices() {
        let wanted = Mac::from_bytes([0x00, 0x04, 0x20, 0x11, 0x11, 0x11]);
        let other = Mac::from_bytes([0x00, 0x04, 0x20, 0x22, 0x22, 0x22]);

        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![reply_from(other), reply_from(wanted)]),
        }));
        let device = Device {
            mac: wanted,
            ..Device::default()
        };

        let (packet, _payload) = session
            .wait_for_reply(&CancellationToken::new(), &device)
            .await
            .expect("the second reply matches");
        assert_eq!(packet.src_address, wanted, "must skip the other device");
    }

    #[tokio::test]
    async fn wait_for_reply_reports_a_transport_error() {
        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![]),
        }));
        let device = Device { mac: Mac::ZERO, ..Device::default() };
        let err = session
            .wait_for_reply(&CancellationToken::new(), &device)
            .await
            .expect_err("an exhausted transport must not hang");
        assert!(matches!(err, OpError::Recv(_)));
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `mise exec -- cargo test -p udap --lib session`
Expected: compile error — `Session` not found.

- [ ] **Step 3: Create `Session`, moving code out of `client.rs`**

Move `send_retried`, the header builder and the `sequence` field from `client.rs` verbatim. Add `wait_for_reply`, ported from `udap/config.go:24-48`:

```rust
/// Waits for a reply from `device`, discarding anything else.
///
/// Three filters, matching go-udap's `waitForDeviceReply`:
/// unparseable packets are skipped, replies whose source MAC is not
/// `device.mac` are skipped, and — when `device.ip` is known — replies
/// from a different source address are skipped. Each skip logs and
/// continues; only a transport error ends the loop.
pub(crate) async fn wait_for_reply(
    &self,
    cancel: &CancellationToken,
    device: &Device,
) -> Result<(Packet, Vec<u8>), OpError> {
    loop {
        let (buf, src) = self
            .transport
            .recv(cancel)
            .await
            .map_err(OpError::Recv)?;
        let Ok((packet, payload)) = Packet::from_bytes(&buf) else {
            debug!("ignoring unparseable reply");
            continue;
        };
        if packet.src_address != device.mac {
            debug!(from = %packet.src_address, want = %device.mac,
                   "ignoring reply from a different device");
            continue;
        }
        if !device.ip.is_empty() && src != device.ip {
            warn!(mac = %packet.src_address, %src, expected = %device.ip,
                  "ignoring reply with mismatched source");
            continue;
        }
        return Ok((packet, payload.to_vec()));
    }
}
```

- [ ] **Step 4: Re-point `Client` at the `Session`**

`Client` gains a `session: Session` field and delegates. `discover` changes only in that it calls `self.session.send_retried(...)` and `self.session.header(...)`. Its behaviour, including "a cancelled receive ends discovery successfully", is unchanged.

Add the registry accessor:

```rust
/// Removes a device from the registry and hands it to the caller.
///
/// The registry is a *discovery result*, not a live mirror: nothing in
/// go-udap re-reads it after an operation, and operations mutate the
/// `Device` the caller holds. Moving the device out rather than cloning
/// makes that explicit — there is no second copy to fall out of date.
pub fn take_device(&mut self, mac: Mac) -> Option<Device> {
    self.devices.remove(&mac)
}
```

- [ ] **Step 5: Run the full suite and watch it pass unchanged**

Run: `mise exec -- cargo test --workspace`
Expected: every pre-existing test still passes, plus the two new session tests. **If any existing test changed behaviour, the extraction is wrong — revert and redo it.**

- [ ] **Step 6: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
git add crates/udap/src/session.rs crates/udap/src/client.rs crates/udap/src/lib.rs
git commit -m "refactor(udap): extract Session from Client"
```

---

### Task 3: `ops/getip` and `ops/getuuid`

The two smallest operations. Both are request-then-decode with no device mutation, so they exercise the `Session` seam end to end without the read-modify-write complexity.

**Files:**
- Create: `crates/udap/src/ops/getip.rs`, `crates/udap/src/ops/getuuid.rs`
- Modify: `crates/udap/src/ops/mod.rs` (created in Task 2 — add the submodule declarations and any missing `OpError` variants)
- Modify: `crates/udap/src/lib.rs`, `crates/udap/src/client.rs`

**Interfaces:**
- Consumes: `Session::{header, send_retried, wait_for_reply}`, `NetworkConfig`
- Produces:
  - `pub enum OpError` (thiserror)
  - `pub async fn getip::get_ip(session: &Session, cancel: &CancellationToken, device: &Device) -> Result<NetworkConfig, OpError>`
  - `pub async fn getuuid::get_uuid(session: &Session, cancel: &CancellationToken, device: &Device) -> Result<String, OpError>`
  - `fn getip::parse_response(data: &[u8]) -> NetworkConfig`
  - `fn getuuid::parse_response(data: &[u8]) -> Result<String, OpError>`

- [ ] **Step 1: Extend `OpError` and declare the submodules**

`crates/udap/src/ops/mod.rs` already exists from Task 2. Add the `pub mod` lines and any variants it lacks:

```rust
//! The five UCP operations, as free functions over a [`Session`] and a
//! caller-held [`Device`].
//!
//! Free functions rather than `Client` methods because an operation
//! needs the transport and *one* device, never the registry — and
//! `&Client` plus `&mut Client.devices[..]` is two overlapping borrows.
//! `Client` wraps each of these so call sites still read like go-udap's.

pub mod config;
pub mod getip;
pub mod getuuid;

use crate::transport::TransportError;

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("send request: {0}")]
    Send(#[source] TransportError),
    #[error("recv reply: {0}")]
    Recv(#[source] TransportError),
    #[error("build packet: {0}")]
    Encode(#[from] crate::error::EncodeError),
    #[error("decode response: {0}")]
    Decode(#[from] crate::error::GetDataError),
    /// The device answered with UCP method 0x0007 and an error TLV.
    ///
    /// Text is fidelity-contract: go-udap formats this as
    /// `device %s error: %s` (`udap/config.go`).
    #[error("device {mac} error: {message}")]
    Device { mac: String, message: String },
    #[error("get_uuid response missing UUID TLV")]
    MissingUuid,
}
```

- [ ] **Step 2: Write the failing decoder tests**

These are pure functions — test them before any async plumbing.

`crates/udap/src/ops/getip.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// TLV: tag, length, value. Codes per Net::UDAP Constant.pm —
    /// 0x05 ip_addr, 0x06 subnet_mask, 0x07 gateway_addr.
    #[test]
    fn decodes_all_three_addresses() {
        let data = [
            0x05, 4, 192, 168, 1, 50,
            0x06, 4, 255, 255, 255, 0,
            0x07, 4, 192, 168, 1, 1,
        ];
        let nc = parse_response(&data);
        assert_eq!(nc.ip, Some(Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(nc.subnet_mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(nc.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn an_omitted_gateway_stays_none() {
        let data = [0x05, 4, 10, 0, 0, 5, 0x06, 4, 255, 0, 0, 0];
        let nc = parse_response(&data);
        assert_eq!(nc.gateway, None, "a missing TLV must not invent an address");
    }

    #[test]
    fn a_wrong_length_value_is_ignored() {
        // go-udap only assigns when length == 4.
        let data = [0x05, 3, 10, 0, 0];
        assert_eq!(parse_response(&data).ip, None);
    }

    #[test]
    fn a_truncated_tlv_stops_decoding_without_panicking() {
        // Length claims 4 bytes but only 2 follow.
        let data = [0x05, 4, 10, 0];
        assert_eq!(parse_response(&data), NetworkConfig::default());
    }

    #[test]
    fn empty_input_yields_an_empty_config() {
        assert_eq!(parse_response(&[]), NetworkConfig::default());
    }
}
```

`crates/udap/src/ops/getuuid.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_16_byte_uuid_as_lowercase_hex() {
        let mut data = vec![0x0d, 16];
        data.extend_from_slice(&[
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
            0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ]);
        let uuid = parse_response(&data).expect("a 16-byte UUID TLV decodes");
        assert_eq!(uuid, "0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn a_wrong_length_uuid_is_not_accepted() {
        let data = [0x0d, 8, 1, 2, 3, 4, 5, 6, 7, 8];
        assert!(matches!(
            parse_response(&data),
            Err(OpError::MissingUuid)
        ));
    }

    #[test]
    fn a_missing_uuid_tlv_is_an_error() {
        let data = [0x05, 4, 10, 0, 0, 1];
        assert!(matches!(parse_response(&data), Err(OpError::MissingUuid)));
    }
}
```

- [ ] **Step 3: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --lib ops::`
Expected: compile errors — the modules do not exist yet.

- [ ] **Step 4: Implement both decoders**

**Reuse `tlv::decode`; do not hand-roll the walk.** `crates/udap/src/tlv.rs:19`
already implements exactly this `tag(u8)/len(u8)/value` loop with the same
"break on overrun, keep the good prefix" semantics the truncation test pins,
and it is already proptest-fuzzed for panic-safety and bounds
(`decode_never_panics`, `decode_never_reads_past_the_buffer`). `client.rs`'s
discovery parser reuses it for the same shape of stream. go-udap hand-rolls a
separate walk per file, but that is internal structure, not observable
behaviour — the wart is not worth carrying, and copying it would mean two
un-fuzzed duplicates.

```rust
// getip.rs
use crate::tlv;
use std::net::Ipv4Addr;

/// Net::UDAP Constant.pm: UCP_CODE_IP_ADDR / SUBNET_MASK / GATEWAY_ADDR.
const TLV_IP_ADDR: u8 = 0x05;
const TLV_SUBNET_MASK: u8 = 0x06;
const TLV_GATEWAY_ADDR: u8 = 0x07;

/// Decodes a `get_ip` reply.
///
/// Unrecognised tags are skipped, and anything whose value is not exactly
/// four bytes is ignored — go-udap only assigns when `length == 4`. Never
/// fails: go-udap returns a nil error here, and an empty config is a
/// legitimate answer from a device with no address yet.
fn parse_response(data: &[u8]) -> NetworkConfig {
    let mut config = NetworkConfig::default();
    for entry in tlv::decode(data) {
        let Ok(octets) = <[u8; 4]>::try_from(entry.value) else {
            continue;
        };
        let addr = Ipv4Addr::from(octets);
        match entry.tag {
            TLV_IP_ADDR => config.ip = Some(addr),
            TLV_SUBNET_MASK => config.subnet_mask = Some(addr),
            TLV_GATEWAY_ADDR => config.gateway = Some(addr),
            _ => {}
        }
    }
    config
}
```

```rust
// getuuid.rs
use crate::{hex, tlv};

/// Net::UDAP Constant.pm: UCP_CODE_UUID, the same code discovery uses.
const TLV_UUID: u8 = 0x0d;
const UUID_LEN: usize = 16;

/// Decodes a `get_uuid` reply to lowercase hex.
///
/// # Errors
/// [`OpError::MissingUuid`] if no 16-byte UUID TLV is present.
fn parse_response(data: &[u8]) -> Result<String, OpError> {
    for entry in tlv::decode(data) {
        if entry.tag == TLV_UUID && entry.value.len() == UUID_LEN {
            return Ok(hex::encode(entry.value));
        }
    }
    Err(OpError::MissingUuid)
}
```

`hex::encode` is already `pub(crate)`, so it is reachable from `ops/`.

- [ ] **Step 5: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --lib ops::`
Expected: 8 passed.

- [ ] **Step 6: Teach `mocksbr` to answer `get_ip` and `get_uuid`**

`Network::receive` (`crates/mocksbr/src/network.rs:45`) currently returns no
reply for anything but `ADV_DISC`:

```rust
if request.ucp_method != method::ADV_DISC {
    return Vec::new();
}
```

Replace that early return with a match, and add the two responders beside
`responses::discovery_response`. A `get_ip` reply carries TLVs `0x05`/`0x06`/
`0x07` from the `DeviceConfig`; a `get_uuid` reply carries one 16-byte `0x0d`.
Both are addressed to the requester with the request bit clear, exactly as
`discovery_response` builds its header — copy that shape rather than
re-deriving it.

Extend `DeviceConfig` with the fields the responses need (`ip`, `subnet_mask`,
`gateway`, `uuid`), defaulting to the factory values a setup-mode device
reports: unspecified addresses, and a fixed UUID for reproducibility.

- [ ] **Step 7: Write the failing round-trip tests**

The decoders are already covered; these pin the async wrappers, which is where
the header, the send and the reply-matching all meet.

`crates/udap/tests/ops_getip_getuuid.rs`:

```rust
#[tokio::test]
async fn get_ip_round_trips_against_mocksbr() {
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let device = Device {
        mac,
        ..Device::default()
    };

    let config = ops::getip::get_ip(&session, &CancellationToken::new(), &device)
        .await
        .expect("the mock answers get_ip");

    // A setup-mode device reports unspecified addresses, which render as
    // dashes rather than 0.0.0.0 -- see netconfig's or_dash.
    assert_eq!(config.to_string(), "IP:      -\nSubnet:  -\nGateway: -");
}

#[tokio::test]
async fn get_uuid_round_trips_against_mocksbr() {
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let device = Device {
        mac,
        ..Device::default()
    };

    let uuid = ops::getuuid::get_uuid(&session, &CancellationToken::new(), &device)
        .await
        .expect("the mock answers get_uuid");

    assert_eq!(uuid.len(), 32, "16 bytes render as 32 hex characters");
    assert!(
        uuid.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "hex must be lowercase, matching go-udap's hex.EncodeToString"
    );
}

#[tokio::test]
async fn get_ip_ignores_a_reply_from_another_device() {
    // Two devices on the mock network; only one is asked. Session's
    // reply-matching must discard the other.
    let wanted = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let other = Mac::from_bytes([0x00, 0x04, 0x20, 0x99, 0x99, 0x99]);
    let network = Arc::new(Network::new(vec![
        DeviceConfig::default_with_mac(other),
        DeviceConfig::default_with_mac(wanted),
    ]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let device = Device {
        mac: wanted,
        ..Device::default()
    };

    ops::getip::get_ip(&session, &CancellationToken::new(), &device)
        .await
        .expect("the wanted device's reply is found among the others");
}
```

- [ ] **Step 8: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_getip_getuuid`
Expected: failures — the mock answers nothing, so these time out or return
`OpError::Recv`. That is the correct red: it proves the test exercises the
wire path rather than the decoder.

- [ ] **Step 9: Add the async operation wrappers**

```rust
/// Queries a device's active network configuration (UCP 0x0002).
///
/// # Errors
/// [`OpError::Send`], [`OpError::Recv`], or [`OpError::Device`] if the
/// device answers with an error TLV.
pub async fn get_ip(
    session: &Session,
    cancel: &CancellationToken,
    device: &Device,
) -> Result<NetworkConfig, OpError> {
    let header = session.header(device.mac, method::GET_IP, false);
    session
        .send_retried(&header.to_bytes())
        .await
        .map_err(OpError::Send)?;
    let (packet, payload) = session.wait_for_reply(cancel, device).await?;
    if packet.ucp_method == method::ERROR {
        return Err(device_error(device, &payload));
    }
    Ok(parse_response(&payload))
}
```

`device_error` lives in `ops/mod.rs`: it decodes the payload with `tlv::decode`, finds the error-message TLV, and builds `OpError::Device { mac, message }`. Check `udap/config.go:157-166` for the exact TLV type constant and the `String::from_utf8_lossy` treatment of the message.

- [ ] **Step 10: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_getip_getuuid`
Expected: 3 passed.

- [ ] **Step 11: Add `Client` wrappers, verify the gate, commit**

```rust
impl Client {
    /// See [`ops::getip::get_ip`].
    ///
    /// # Errors
    /// Propagates [`OpError`].
    pub async fn get_ip(
        &self,
        cancel: &CancellationToken,
        device: &Device,
    ) -> Result<NetworkConfig, OpError> {
        ops::getip::get_ip(&self.session, cancel, device).await
    }
}
```

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/udap/src/ops crates/udap/src/lib.rs crates/udap/src/client.rs crates/mocksbr/src crates/udap/tests/
git commit -m "feat(udap): add get_ip and get_uuid operations"
```

---

### Task 4: `ops/config` — `get`, `get_all`, `reset`

The read side of configuration, plus the simplest write. `set` is deliberately deferred to Task 5 so its read-modify-write logic gets a task of its own.

**Files:**
- Create: `crates/udap/src/ops/config.rs`
- Modify: `crates/udap/src/device.rs`, `crates/udap/src/client.rs`

**Interfaces:**
- Consumes: `Session`, `getdata::parse_response`, `parameters::{by_name, names, Parameter}`
- Produces:
  - `pub async fn get(session, cancel, device: &Device, params: &[&str]) -> Result<BTreeMap<String, Vec<u8>>, OpError>`
  - `pub async fn get_all(session, cancel, device: &mut Device) -> Result<(), OpError>`
  - `pub async fn reset(session, cancel, device: &Device) -> Result<(), OpError>`
  - `Device.parameters: BTreeMap<String, Vec<u8>>`

- [ ] **Step 1: Add the `parameters` field to `Device`**

```rust
/// NVRAM values most recently read from the device.
///
/// Three roles, all load-bearing (see the M4 plan's "Why the seam"):
/// the output channel for [`ops::config::get_all`], a cache that lets
/// [`ops::config::set`] skip its read-modify-write prelude, and — because
/// it is only written after the device acknowledges — a record of what is
/// actually persisted rather than what was attempted.
///
/// Values are bytes, not `String`: ADR-6.
pub parameters: BTreeMap<String, Vec<u8>>,
```

- [ ] **Step 2: Write the failing test for `get_all` as an output channel**

Use `mocksbr` — this is the first operation test that needs a device that answers `get_data`. If `mocksbr` does not yet handle 0x0005, extend it here; that work belongs to this task, not a separate one.

`crates/udap/tests/ops_config.rs`:

```rust
#[tokio::test]
async fn get_all_writes_into_the_device() {
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let mut device = Device { mac, ..Device::default() };

    assert!(device.parameters.is_empty(), "starts empty");

    ops::config::get_all(&session, &CancellationToken::new(), &mut device)
        .await
        .expect("get_all against the mock succeeds");

    assert!(
        !device.parameters.is_empty(),
        "the mutation is the return channel: get_all returns only ()"
    );
    assert_eq!(
        device.parameters.get("wireless_channel").map(Vec::as_slice),
        Some(b"6".as_slice()),
        "factory default for wireless_channel"
    );
}

#[tokio::test]
async fn get_all_drops_stale_offset_entries() {
    // go-udap deletes offset_* keys before merging, so an unrecognised
    // offset from a previous read does not survive a later one.
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let mut device = Device { mac, ..Device::default() };
    device
        .parameters
        .insert("offset_999".to_owned(), b"stale".to_vec());

    ops::config::get_all(&session, &CancellationToken::new(), &mut device)
        .await
        .expect("get_all succeeds");

    assert!(
        !device.parameters.contains_key("offset_999"),
        "stale offset_* keys must be cleared before the merge"
    );
}
```

- [ ] **Step 3: Run and watch them fail**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: compile error, then assertion failures once stubs exist.

- [ ] **Step 4: Implement `get`, `get_all`, `reset`**

Port from `udap/config.go:53-90` (`get`), `:91-111` (`get_all`), `:176-217` (`reset`). The three behaviours to get right:

1. `get` builds a `get_data` payload of `(offset, length)` pairs for the named parameters, sorted by offset. `parameters::by_name` gives both.
2. `get_all` calls `get` with `parameters::names()`, clears `offset_`-prefixed keys from `device.parameters`, then merges. It returns `Result<(), OpError>` — **not** the map.
3. `reset` sends UCP 0x0004 and waits for the ack. It does not touch `device.parameters`.

- [ ] **Step 5: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: 2 passed.

- [ ] **Step 6: Add the fixture assertions**

Copy `reset-ack.bin` from `~/code/github.com/yo61/go-udap/mocksbr/testdata/captures/` into `crates/udap/tests/fixtures/` and assert the reset path parses it. **The captures were taken with `sequence=1` and an all-zeros source MAC** — build requests the same way or the bytes will not match.

- [ ] **Step 7: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/udap/src/ops/config.rs crates/udap/src/device.rs crates/udap/src/client.rs crates/udap/tests/
git commit -m "feat(udap): add get, get_all and reset operations"
```

---

### Task 5: `ops/config::set` — read-modify-write

The hard one, and the reason it gets its own task. Three behaviours must be ported deliberately; each gets its own test.

**Files:**
- Modify: `crates/udap/src/ops/config.rs`, `crates/udap/src/client.rs`
- Test: `crates/udap/tests/ops_config.rs`

**Interfaces:**
- Consumes: everything from Task 4
- Produces: `pub async fn set(session, cancel, device: &mut Device, config: &BTreeMap<String, Vec<u8>>) -> Result<(), OpError>`

- [ ] **Step 1: Write the three failing behaviour tests**

```rust
#[tokio::test]
async fn set_reads_before_writing_to_avoid_clobbering_neighbours() {
    // Omitted parameters would write zeros over neighbouring NVRAM, so
    // set must read current values first and merge the caller's on top.
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(MockTransport::new(Arc::clone(&network))));
    let mut device = Device { mac, ..Device::default() };

    let mut change = BTreeMap::new();
    change.insert("wireless_channel".to_owned(), b"11".to_vec());

    ops::config::set(&session, &CancellationToken::new(), &mut device, &change)
        .await
        .expect("set succeeds");

    // A parameter the caller never mentioned must still hold its value.
    assert_eq!(
        device.parameters.get("wireless_region_id").map(Vec::as_slice),
        Some(b"4".as_slice()),
        "the prelude read must preserve untouched parameters"
    );
}

#[tokio::test]
async fn set_skips_the_prelude_read_when_parameters_are_cached() {
    // cli/set.go pre-populates Parameters so this is one round-trip, not
    // two. Count sends to prove the prelude was skipped.
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let counting = CountingTransport::new(MockTransport::new(Arc::clone(&network)));
    let sends = counting.sends();
    let session = Session::new(Box::new(counting));

    let mut device = Device { mac, ..Device::default() };
    ops::config::get_all(&session, &CancellationToken::new(), &mut device)
        .await
        .expect("prime the cache");
    let after_priming = sends.load(Ordering::SeqCst);

    let mut change = BTreeMap::new();
    change.insert("wireless_channel".to_owned(), b"11".to_vec());
    ops::config::set(&session, &CancellationToken::new(), &mut device, &change)
        .await
        .expect("set succeeds");

    assert_eq!(
        sends.load(Ordering::SeqCst) - after_priming,
        1,
        "a primed cache means one send: the SetData itself"
    );
}

#[tokio::test]
async fn set_does_not_record_values_the_device_never_acknowledged() {
    // The commit barrier. go-udap moved this merge after the ack because
    // doing it earlier left Parameters advertising values that were never
    // persisted when the round-trip failed.
    let mac = Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x17, 0x18]);
    let network = Arc::new(Network::new(vec![DeviceConfig::default_with_mac(mac)]));
    let session = Session::new(Box::new(FailingAfterSend::new(
        MockTransport::new(Arc::clone(&network)),
    )));
    let mut device = Device { mac, ..Device::default() };
    device
        .parameters
        .insert("wireless_channel".to_owned(), b"6".to_vec());

    let mut change = BTreeMap::new();
    change.insert("wireless_channel".to_owned(), b"11".to_vec());

    let result =
        ops::config::set(&session, &CancellationToken::new(), &mut device, &change).await;

    assert!(result.is_err(), "the round-trip failed");
    assert_eq!(
        device.parameters.get("wireless_channel").map(Vec::as_slice),
        Some(b"6".as_slice()),
        "a failed write must leave the cached value untouched, not show 11"
    );
}
```

`CountingTransport` and `FailingAfterSend` are test-only wrappers around `MockTransport`; put them in the same test file. `CountingTransport` holds an `Arc<AtomicUsize>` incremented in `send`; `FailingAfterSend` delegates `send` and returns `TransportError::Cancelled` from `recv`.

- [ ] **Step 2: Run and watch all three fail**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: compile error on `set`, then three assertion failures.

- [ ] **Step 3: Implement `set`**

Port from `udap/config.go:120-175`. The order is the specification:

```
1. If device.parameters is empty, run the prelude read (get_all).
   If that read fails, abort the whole operation — go-udap's comment
   records that the previous warn-and-continue path produced exactly
   the partial write the read was meant to prevent.
2. allParams = device.parameters, overlaid with config.
3. Build and send the SetData packet.
4. Wait for the reply.
5. On SET_DATA / GET_DATA / GET_IP: merge config into device.parameters
   and return Ok. NOT BEFORE — this is the commit barrier.
6. On ERROR: decode the error TLV and return OpError::Device.
```

- [ ] **Step 4: Run and watch them pass**

Run: `mise exec -- cargo test -p udap --test ops_config`
Expected: 5 passed (2 from Task 4, 3 new).

- [ ] **Step 5: Add the remaining fixtures**

Copy `setdata-status-ack.bin`, `setdata-empty-ack.bin` and `savedata-status-ack.bin` into `crates/udap/tests/fixtures/` and assert each parses through the `set` reply path.

- [ ] **Step 6: Add the `Client` wrapper**

```rust
/// See [`ops::config::set`].
///
/// # Errors
/// Propagates [`OpError`].
pub async fn set_config(
    &self,
    cancel: &CancellationToken,
    device: &mut Device,
    config: &BTreeMap<String, Vec<u8>>,
) -> Result<(), OpError> {
    ops::config::set(&self.session, cancel, device, config).await
}
```

- [ ] **Step 7: Verify the gate and commit**

```bash
mise exec -- cargo fmt --all --check
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo test --workspace
git add crates/udap/src/ops/config.rs crates/udap/src/client.rs crates/udap/tests/
git commit -m "feat(udap): add set operation with read-modify-write"
```

---

## Spec bookkeeping

OQ-6 is marked *Blocks M4* and says to resolve it by reading
`mocksbr/fixture_test.go` when M4 starts. That has been done — `readFixture` is
a bare `os.ReadFile`, and the captures are compared with `bytes.Equal` against
whatever `Network.Receive` produces, so there is no framing or offset
assumption to port. **Update the spec in Task 4**, alongside the first fixture
copy, so a reader of the spec does not still see M4 as blocked:

- Rewrite OQ-6's opening as `**OQ-6 — mocksbr capture fixtures. RESOLVED: they
  copy verbatim.**`
- Record the one genuine coupling: the captures were taken with `sequence=1`
  and an all-zeros source MAC, so tests must build requests the same way.

## Milestone acceptance

M4 is done when all six UCP operations round-trip against `mocksbr`. Verify:

```bash
mise exec -- cargo test --workspace          # all green
mise exec -- cargo clippy --all-targets --all-features   # silent
mise exec -- cargo fmt --all --check          # clean
```

Then update the spec: mark M4's *Done when* condition met, as M3's was.

**Hardware acceptance is available and worth taking.** A Squeezebox in setup mode answers `read`. Once M6 exposes the CLI, `udapcfg read --all MAC` must reproduce `go-udap read --all MAC` byte-for-byte. Recorded from the live device on 2026-09-14 (MAC `00:04:20:16:17:18`, factory state) — 26 parameters, all ASCII, `wireless_SSID` empty:

```
bridging=0
hostname=
interface=128
lan_gateway=0.0.0.0
lan_ip_mode=1
lan_network_address=0.0.0.0
lan_subnet_mask=255.255.255.0
lms_address=0.0.0.0
primary_dns=0.0.0.0
secondary_dns=0.0.0.0
server_address=0.0.0.0
squeezecenter_name=
wireless_SSID=
wireless_channel=6
wireless_keylen=0
wireless_mode=0
wireless_region_id=4
wireless_wep_key=
wireless_wep_key_1=
wireless_wep_key_2=
wireless_wep_key_3=
wireless_wep_on=0
wireless_wpa_cipher=3
wireless_wpa_mode=1
wireless_wpa_on=0
wireless_wpa_psk=
```

This also partially informs **OQ-8**: a factory device carries no non-UTF-8 values, so the question is only live for a *configured* device with a non-ASCII SSID. Resolving it properly needs such a device, or a deliberate `go-udap set` of one.

## Deferred

- **`Device::validate`, `Packet::validate`, `TLVData::validate`, `Client::validate`** (`udap/validation.go:99-200`). Only `ValidateParameter` is called from the CLI; the other four are unexercised outside the Go's own tests. Port them when something needs them, per YAGNI.
- **The CLI subcommands.** M6.
- **OQ-8's on-disk half.** The INI layer is M6; ADR-6 already settles the in-memory half.
