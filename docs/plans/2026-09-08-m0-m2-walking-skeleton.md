# udapcfg-rs M0–M2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Rust protocol core and a walking skeleton, ending with `udapcfg discover` printing the same MAC list as `go-udap discover` against an in-process mock device.

**Architecture:** Cargo workspace of three crates — `udap` (protocol + transport + client), `mocksbr` (fake Squeezebox Receiver), `udap-cli` (the `udapcfg` binary). Ported faithfully from go-udap v2.4.8. Async on tokio; the `Transport` trait is the seam that lets the same client run against a real UDP socket or an in-process mock.

**Tech Stack:** Rust 1.98.1, tokio (current_thread), clap 4, socket2, netdev, thiserror/anyhow, tracing, indicatif. Test with `cargo nextest`, `insta`, `proptest`.

**Spec:** [`docs/specs/2026-09-08-rust-port-spec.md`](../specs/2026-09-08-rust-port-spec.md)
**Reference:** [`docs/port-map.md`](../port-map.md) — module-by-module Go→Rust mapping

## Global Constraints

- **Source of truth is go-udap v2.4.8 (`43864a5`)**, at `~/code/github.com/yo61/go-udap`. When this plan and the Go disagree, read the Go and fix the plan.
- **Wire bytes are pinned.** All multi-byte protocol fields are **big-endian**. The UDAP header is exactly **27 bytes**. Any change to encoding is a spec amendment, not an implementation choice.
- **Binary is `udapcfg`**, not `go-udap` and not `udapcfg-rs`. Output must match go-udap byte-for-byte *except* where the program name appears (help, usage errors, `--version`).
- **Toolchain is pinned in `mise.toml`.** Run everything through `mise exec --` or with mise activated. Rust 1.98.1.
- **Zero warnings.** `mise.toml` sets `RUSTFLAGS = "-D warnings"`. `cargo clippy --all-targets --all-features` must be clean. Any `#[allow(...)]` needs a justification comment.
- **Pin exact dependency versions** (`=1.2.3` style is not required, but do not use `^` ranges loosely — write the two-component version and let Cargo.lock pin the rest; commit `Cargo.lock`).
- **No `unwrap()` in non-test code.** `unwrap_used = "deny"` is on. Tests may use `unwrap`/`expect`.
- **TDD.** Every task writes the failing test first, watches it fail, then implements.
- **Commit per task**, conventional-commit format, on branch `main` of `udapcfg-rs` *only after* creating a feature branch — never commit directly to `main`.

## File Structure

```
Cargo.toml                          workspace manifest, shared [workspace.lints]
Cargo.lock                          committed
mise.toml                           already exists — toolchain pin
deny.toml                           cargo-deny config
.github/workflows/ci.yaml           fmt + clippy + nextest

crates/udap/
  Cargo.toml
  src/lib.rs                        re-exports; crate docs
  src/error.rs                      Error enum (thiserror) for the whole crate
  src/mac.rs                        Mac value object            [Task 2]
  src/tlv.rs                        TLV encode/decode           [Task 3]
  src/protocol.rs                   Packet, constants, Method   [Task 4]
  src/parameters.rs                 PARAMETERS const table      [Task 5]
  src/getdata.rs                    GetData response decoder    [Task 6]
  src/transport/mod.rs              Transport trait             [Task 7]
  src/device.rs                     Device struct               [Task 8]
  src/client.rs                     Client + discovery          [Task 8]

crates/mocksbr/
  Cargo.toml
  src/lib.rs
  src/device.rs                     DeviceConfig + one device   [Task 7]
  src/network.rs                    Network of devices          [Task 7]
  src/responses.rs                  reply builders              [Task 7]
  src/transport.rs                  MockTransport               [Task 7]

crates/udap-cli/
  Cargo.toml                        [[bin]] name = "udapcfg"
  src/main.rs                       entry, exit codes           [Task 9]
  src/cli.rs                        clap definitions            [Task 9]
  src/cmd/mod.rs
  src/cmd/discover.rs               discover subcommand         [Task 9]
```

**Not in this plan** (later milestones): `transport/udp.rs`, `transport/multi.rs`, `interfaces.rs`, `netconfig.rs`, `validation.rs`, `ops/*`, the remaining subcommands, `progress.rs`, man pages, completions.

**Reordering note:** Tasks 5 and 6 (parameters, getdata) are M1 per the spec but are *not* on the `discover` critical path. If you want a running binary sooner, do them after Task 9. Nothing else depends on them within this plan.

---

### Task 1: Workspace foundations

**Files:**
- Create: `Cargo.toml`, `deny.toml`, `.github/workflows/ci.yaml`
- Create: `crates/udap/Cargo.toml`, `crates/udap/src/lib.rs`
- Create: `crates/mocksbr/Cargo.toml`, `crates/mocksbr/src/lib.rs`
- Create: `crates/udap-cli/Cargo.toml`, `crates/udap-cli/src/main.rs`

**Interfaces:**
- Consumes: nothing
- Produces: a workspace where `cargo build`, `cargo clippy`, `cargo nextest run` all succeed on empty crates. Every later task adds to this.

- [ ] **Step 1: Create the feature branch**

```bash
cd ~/code/github.com/yo61/udapcfg-rs
git switch -c feat/m0-workspace
```

- [ ] **Step 2: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/udap", "crates/mocksbr", "crates/udap-cli"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.98"
license = "MIT"
repository = "https://github.com/yo61/udapcfg-rs"

[workspace.dependencies]
tokio = { version = "1.53", features = ["rt", "net", "time", "sync", "macros"] }
tokio-util = "0.7"  # CancellationToken needs no features
async-trait = "0.1"
thiserror = "2.0"
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
clap = { version = "4.6", features = ["derive", "wrap_help"] }
socket2 = { version = "0.6", features = ["all"] }
netdev = { version = "0.46", default-features = false }
indicatif = "0.18"
# dev
insta = "1.48"
rstest = "0.27"
proptest = "1.11"
serial_test = "4.0"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "warn"
panic = "deny"
panic_in_result_fn = "deny"
unimplemented = "deny"
allow_attributes = "deny"
dbg_macro = "deny"
todo = "deny"
print_stdout = "deny"
print_stderr = "deny"
await_holding_lock = "deny"
large_futures = "deny"
exit = "deny"
mem_forget = "deny"
module_name_repetitions = "allow"
similar_names = "allow"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

- [ ] **Step 3: Write the three crate manifests**

`crates/udap/Cargo.toml`:

```toml
[package]
name = "udap"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Squeezebox UDAP protocol client"

[dependencies]
async-trait.workspace = true
thiserror.workspace = true
tokio.workspace = true
tokio-util.workspace = true
tracing.workspace = true

[dev-dependencies]
proptest.workspace = true
rstest.workspace = true
tokio = { workspace = true, features = ["rt", "macros", "time", "test-util"] }

[lints]
workspace = true
```

`crates/mocksbr/Cargo.toml`:

```toml
[package]
name = "mocksbr"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "In-process mock Squeezebox Receiver for testing udap"

[dependencies]
udap = { path = "../udap" }
async-trait.workspace = true
thiserror.workspace = true
tokio.workspace = true
tokio-util.workspace = true

[lints]
workspace = true
```

`crates/udap-cli/Cargo.toml`:

```toml
[package]
name = "udap-cli"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
description = "Command-line tool for configuring Squeezebox devices over UDAP"

[[bin]]
name = "udapcfg"
path = "src/main.rs"

[dependencies]
udap = { path = "../udap" }
anyhow.workspace = true
clap.workspace = true
tokio = { workspace = true, features = ["rt", "macros", "time"] }
tokio-util.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true

[dev-dependencies]
mocksbr = { path = "../mocksbr" }
insta.workspace = true

[lints]
workspace = true
```

- [ ] **Step 4: Write the three crate roots**

`crates/udap/src/lib.rs`:

```rust
//! Squeezebox UDAP (Universal Device Access Protocol) client.
//!
//! Ported from <https://github.com/yo61/go-udap> v2.4.8.
```

`crates/mocksbr/src/lib.rs`:

```rust
//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.
```

`crates/udap-cli/src/main.rs`:

```rust
fn main() {}
```

- [ ] **Step 5: Write the cargo-deny config**

`deny.toml`:

```toml
[advisories]
yanked = "deny"

[licenses]
allow = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib"]

[bans]
multiple-versions = "warn"
wildcards = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

- [ ] **Step 6: Write CI**

`.github/workflows/ci.yaml`:

```yaml
name: CI
on:
  push: { branches: [main] }
  pull_request:

permissions:
  contents: read

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v7.0.1
        with:
          persist-credentials: false
      - uses: jdx/mise-action@c2a87611a18de5b3828c5652fe268e992400cb5c  # v4.3.0
        with:
          experimental: true
      - name: Format
        run: cargo fmt --all --check
      - name: Clippy
        run: cargo clippy --all-targets --all-features
      - name: Test
        run: cargo nextest run --all-features
      - name: Supply chain
        run: cargo deny check
```

> SHAs verified 2026-09-08 against the latest release of each action.
> `mise-action` reads `mise.toml`, so CI uses the same pinned toolchain as
> local development — one source of truth, not two.

- [ ] **Step 7: Verify the workspace builds clean**

```bash
mise exec -- cargo build --workspace
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo fmt --all --check
mise exec -- cargo nextest run --workspace
```

Expected: all four succeed. `nextest` reports 0 tests, which is correct at this point.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -S -m "feat: scaffold cargo workspace with three crates

Adds the udap, mocksbr and udap-cli crates, the shared clippy lint
table from the project standard, cargo-deny config, and a CI job
running fmt, clippy, nextest and deny through mise."
```

---

### Task 2: `Mac` value object

Port of `udap/mac.go`. Read that file and `udap/mac_test.go` before starting.

**Files:**
- Create: `crates/udap/src/mac.rs`
- Modify: `crates/udap/src/lib.rs` (add `pub mod mac; pub use mac::Mac;`)

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Mac([u8; 6])` — `Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default`
  - `Mac::ZERO: Mac`
  - `Mac::from_bytes(b: [u8; 6]) -> Mac`
  - `Mac::as_bytes(&self) -> &[u8; 6]`
  - `Mac::is_zero(&self) -> bool`
  - `impl FromStr for Mac { type Err = MacParseError; }`
  - `impl Display for Mac` — lowercase `aa:bb:cc:dd:ee:ff`
  - `pub enum MacParseError` with variants `Length`, `NonHex`, `MissingColon`

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/mac.rs` (test module only for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_lowercase() {
        let m: Mac = "00:04:20:16:05:8f".parse().unwrap();
        assert_eq!(m.as_bytes(), &[0x00, 0x04, 0x20, 0x16, 0x05, 0x8f]);
    }

    #[test]
    fn parses_uppercase_and_mixed_case() {
        let upper: Mac = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let mixed: Mac = "aA:Bb:cC:Dd:eE:Ff".parse().unwrap();
        assert_eq!(upper, mixed);
        assert_eq!(upper.as_bytes(), &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    }

    #[test]
    fn display_is_canonical_lowercase() {
        let m = Mac::from_bytes([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(m.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn roundtrips_through_string() {
        let input = "00:04:20:16:05:8f";
        let m: Mac = input.parse().unwrap();
        assert_eq!(m.to_string(), input);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!("00:04:20:16:05".parse::<Mac>().is_err());
        assert!("00:04:20:16:05:8f:aa".parse::<Mac>().is_err());
    }

    // go-udap tightened this: fmt.Sscanf used to accept trailing space.
    #[test]
    fn rejects_leading_and_trailing_whitespace() {
        assert!(" 00:04:20:16:05:8f".parse::<Mac>().is_err());
        assert!("00:04:20:16:05:8f ".parse::<Mac>().is_err());
    }

    #[test]
    fn rejects_wrong_separator() {
        assert!("00-04-20-16-05-8f".parse::<Mac>().is_err());
    }

    #[test]
    fn rejects_non_hex_digits() {
        assert!("zz:04:20:16:05:8f".parse::<Mac>().is_err());
    }

    #[test]
    fn zero_is_detected() {
        assert!(Mac::ZERO.is_zero());
        assert!(!Mac::from_bytes([0, 0, 0, 0, 0, 1]).is_zero());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap
```

Expected: compilation failure — `Mac` is not defined.

- [ ] **Step 3: Implement `Mac`**

Prepend to `crates/udap/src/mac.rs`, above the test module:

```rust
//! The `Mac` value object: a 48-bit IEEE 802 hardware address.
//!
//! Parsing and formatting rules live here so validation happens once at
//! the boundary and the type carries the guarantee downstream.

use std::fmt;
use std::str::FromStr;

/// Length of a MAC address in bytes.
pub const MAC_LEN: usize = 6;

/// Canonical string form is exactly this many characters: `aa:bb:cc:dd:ee:ff`.
const MAC_STR_LEN: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Mac([u8; MAC_LEN]);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MacParseError {
    #[error("invalid MAC address {input:?}: want {MAC_STR_LEN} chars, got {got}")]
    Length { input: String, got: usize },
    #[error("invalid MAC address {input:?}: non-hex digit at {pos}")]
    NonHex { input: String, pos: usize },
    #[error("invalid MAC address {input:?}: missing colon at {pos}")]
    MissingColon { input: String, pos: usize },
}

impl Mac {
    /// The all-zeros MAC, used as the broadcast destination and as the
    /// source placeholder in outgoing packets.
    pub const ZERO: Mac = Mac([0; MAC_LEN]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; MAC_LEN]) -> Self {
        Mac(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; MAC_LEN] {
        &self.0
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; MAC_LEN]
    }
}

/// Converts one ASCII hex digit to its 0-15 value.
///
/// Hand-rolled rather than pulling in a hex crate for a single-byte decode,
/// matching go-udap's `hexNibble`.
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl FromStr for Mac {
    type Err = MacParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != MAC_STR_LEN {
            return Err(MacParseError::Length {
                input: s.to_owned(),
                got: bytes.len(),
            });
        }
        let mut out = [0u8; MAC_LEN];
        for i in 0..MAC_LEN {
            let base = i * 3;
            let hi = hex_nibble(bytes[base]).ok_or_else(|| MacParseError::NonHex {
                input: s.to_owned(),
                pos: base,
            })?;
            let lo = hex_nibble(bytes[base + 1]).ok_or_else(|| MacParseError::NonHex {
                input: s.to_owned(),
                pos: base + 1,
            })?;
            if i < MAC_LEN - 1 && bytes[base + 2] != b':' {
                return Err(MacParseError::MissingColon {
                    input: s.to_owned(),
                    pos: base + 2,
                });
            }
            out[i] = (hi << 4) | lo;
        }
        Ok(Mac(out))
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ":")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Wire it into the crate root**

`crates/udap/src/lib.rs`:

```rust
//! Squeezebox UDAP (Universal Device Access Protocol) client.
//!
//! Ported from <https://github.com/yo61/go-udap> v2.4.8.

pub mod mac;

pub use mac::{Mac, MacParseError};
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
```

Expected: 8 tests pass, clippy clean.

- [ ] **Step 6: Add a property test for the round-trip**

Append to the test module in `crates/udap/src/mac.rs`:

```rust
#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn display_parse_roundtrip(bytes in proptest::array::uniform6(any::<u8>())) {
            let m = Mac::from_bytes(bytes);
            let parsed: Mac = m.to_string().parse().unwrap();
            prop_assert_eq!(m, parsed);
        }

        #[test]
        fn never_panics_on_arbitrary_input(s in ".*") {
            let _ = s.parse::<Mac>();
        }
    }
}
```

- [ ] **Step 7: Run the property tests**

```bash
mise exec -- cargo nextest run -p udap
```

Expected: 10 tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/udap/src/mac.rs crates/udap/src/lib.rs
git commit -S -m "feat(udap): add Mac value object

Ports udap/mac.go. Strict parsing: exactly 17 characters, colon
separators, hex digits only, no leading or trailing whitespace.
Display renders the canonical lowercase form."
```

---

### Task 3: TLV codec

Port of `EncodeTLV` / `DecodeTLV` in `udap/protocol.go`, plus `writeTLV` from `mocksbr/responses.go`.

**Files:**
- Create: `crates/udap/src/tlv.rs`
- Modify: `crates/udap/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Tlv<'a> { pub tag: u8, pub value: &'a [u8] }`
  - `pub fn decode(data: &[u8]) -> Vec<Tlv<'_>>`
  - `pub fn encode_into(tag: u8, value: &[u8], out: &mut Vec<u8>)`

**Behaviour note:** Go's `DecodeTLV` **silently stops** on a truncated entry rather than erroring — it `break`s out of the loop and returns what it parsed. Preserve that exactly; a malformed tail must not discard the well-formed prefix.

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/tlv.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_single_entry() {
        let data = [0x02, 0x03, b'a', b'b', b'c'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].tag, 0x02);
        assert_eq!(tlvs[0].value, b"abc");
    }

    #[test]
    fn decodes_multiple_entries_in_order() {
        let data = [0x02, 0x01, b'x', 0x09, 0x02, b'7', b'7'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 2);
        assert_eq!(tlvs[0].tag, 0x02);
        assert_eq!(tlvs[0].value, b"x");
        assert_eq!(tlvs[1].tag, 0x09);
        assert_eq!(tlvs[1].value, b"77");
    }

    #[test]
    fn decodes_zero_length_value() {
        let data = [0x0c, 0x00];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].value, b"");
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(decode(&[]).is_empty());
    }

    // Matches Go's DecodeTLV: a truncated tail is dropped, the good
    // prefix is kept, and no error is raised.
    #[test]
    fn truncated_value_keeps_the_good_prefix() {
        let data = [0x02, 0x01, b'x', 0x09, 0x05, b'a', b'b'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].tag, 0x02);
    }

    #[test]
    fn dangling_tag_byte_is_dropped() {
        let data = [0x02, 0x01, b'x', 0x09];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
    }

    #[test]
    fn encode_then_decode_roundtrips() {
        let mut buf = Vec::new();
        encode_into(0x0c, b"connected", &mut buf);
        encode_into(0x09, b"77", &mut buf);
        let tlvs = decode(&buf);
        assert_eq!(tlvs.len(), 2);
        assert_eq!(tlvs[0].value, b"connected");
        assert_eq!(tlvs[1].value, b"77");
    }

    // Matches mocksbr's writeTLV: the length field is one byte, so
    // over-long values are truncated rather than corrupting the stream.
    #[test]
    fn encode_truncates_values_over_255_bytes() {
        let long = vec![b'z'; 300];
        let mut buf = Vec::new();
        encode_into(0x02, &long, &mut buf);
        assert_eq!(buf[1], 255);
        assert_eq!(buf.len(), 2 + 255);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap tlv
```

Expected: compilation failure — `decode` and `encode_into` are not defined.

- [ ] **Step 3: Implement the codec**

Prepend to `crates/udap/src/tlv.rs`:

```rust
//! Type-Length-Value codec used by UDAP discovery and error payloads.
//!
//! Wire form per entry: one tag byte, one length byte, then that many
//! value bytes. Length is `u8`, so a value is at most 255 bytes.

/// One decoded TLV entry. Borrows its value from the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
}

/// Decodes a TLV sequence.
///
/// A truncated trailing entry is dropped and the well-formed prefix is
/// returned — matching go-udap's `DecodeTLV`, which breaks rather than
/// erroring. Callers that need strictness must check the returned count.
#[must_use]
pub fn decode(data: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 2 <= data.len() {
        let tag = data[pos];
        let len = usize::from(data[pos + 1]);
        pos += 2;
        if pos + len > data.len() {
            break;
        }
        out.push(Tlv {
            tag,
            value: &data[pos..pos + len],
        });
        pos += len;
    }
    out
}

/// Appends one TLV entry to `out`.
///
/// Values longer than 255 bytes are truncated, because the length field
/// is a single byte. UDAP's own TLVs never approach that.
pub fn encode_into(tag: u8, value: &[u8], out: &mut Vec<u8>) {
    let len = value.len().min(255);
    out.push(tag);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "len is clamped to 255 on the line above"
    )]
    out.push(len as u8);
    out.extend_from_slice(&value[..len]);
}
```

- [ ] **Step 4: Wire it into the crate root**

Add to `crates/udap/src/lib.rs`:

```rust
pub mod tlv;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
```

Expected: 8 new tests pass (18 total), clippy clean.

- [ ] **Step 6: Add a property test**

Append to the test module:

```rust
#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // The decoder runs on network input. It must never panic.
        #[test]
        fn decode_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode(&data);
        }

        #[test]
        fn decode_never_reads_past_the_buffer(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let total: usize = decode(&data).iter().map(|t| 2 + t.value.len()).sum();
            prop_assert!(total <= data.len());
        }
    }
}
```

- [ ] **Step 7: Run and commit**

```bash
mise exec -- cargo nextest run -p udap
git add crates/udap/src/tlv.rs crates/udap/src/lib.rs
git commit -S -m "feat(udap): add TLV codec

Ports EncodeTLV/DecodeTLV from udap/protocol.go. Decoding stops
silently at a truncated entry and keeps the well-formed prefix,
matching the Go. Encoding truncates values at 255 bytes because the
length field is one byte."
```

---

### Task 4: `Packet` header codec

Port of the `Packet` struct, `ParsePacket`, the protocol constants, and `isUDAPRequestPacket` (from `udap/loopback.go`).

**Files:**
- Create: `crates/udap/src/protocol.rs`
- Create: `crates/udap/src/error.rs`
- Modify: `crates/udap/src/lib.rs`

**Interfaces:**
- Consumes: `Mac`, `Mac::from_bytes`, `Mac::as_bytes` (Task 2)
- Produces:
  - `pub const HEADER_SIZE: usize = 27`
  - `pub const PORT: u16 = 17784`
  - `pub const UDAP_TYPE_UCP: u16 = 0xC001`
  - `pub const ADDR_TYPE_ETH: u8 = 0x01`
  - `pub const UAP_CLASS_UCP: [u8; 4] = [0x00, 0x01, 0x00, 0x01]`
  - `pub const UCP_FLAGS_OFFSET: usize = 20`
  - `pub mod method` with `pub const ADV_DISC: u16 = 0x0009` etc.
  - `pub struct Packet { .. }` with all 11 fields public
  - `Packet::to_bytes(&self) -> [u8; HEADER_SIZE]`
  - `Packet::from_bytes(buf: &[u8]) -> Result<(Packet, &[u8]), ProtocolError>`
  - `pub fn is_request_packet(buf: &[u8]) -> bool`
  - `pub enum ProtocolError` with `TooShort { got, min }` and `NotUcp { udap_type }`

**Design note:** `ucp_method` stays a raw `u16`, not an enum. `udap/config.go` reports `"unexpected response method 0x%04x"` for unrecognised methods, which an enum could not represent. Constants in a `method` module give readable call sites without losing the raw value.

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/protocol.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mac;

    fn sample() -> Packet {
        Packet {
            dst_broadcast: 1,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x05, 0x8f]),
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0x01,
            uap_class: UAP_CLASS_UCP,
            ucp_method: method::ADV_DISC,
        }
    }

    #[test]
    fn header_is_27_bytes() {
        assert_eq!(HEADER_SIZE, 27);
        assert_eq!(sample().to_bytes().len(), 27);
    }

    #[test]
    fn field_offsets_match_the_wire_layout() {
        let b = sample().to_bytes();
        assert_eq!(b[0], 1, "dst_broadcast");
        assert_eq!(b[1], ADDR_TYPE_ETH, "dst_type");
        assert_eq!(&b[2..8], &[0u8; 6], "dst_address");
        assert_eq!(b[8], 0, "src_broadcast");
        assert_eq!(b[9], ADDR_TYPE_ETH, "src_type");
        assert_eq!(&b[10..16], &[0x00, 0x04, 0x20, 0x16, 0x05, 0x8f], "src_address");
        assert_eq!(&b[16..18], &[0x00, 0x01], "sequence, big-endian");
        assert_eq!(&b[18..20], &[0xC0, 0x01], "udap_type, big-endian");
        assert_eq!(b[20], 0x01, "ucp_flags");
        assert_eq!(&b[21..25], &[0x00, 0x01, 0x00, 0x01], "uap_class");
        assert_eq!(&b[25..27], &[0x00, 0x09], "ucp_method, big-endian");
    }

    #[test]
    fn ucp_flags_offset_constant_matches_the_layout() {
        let b = sample().to_bytes();
        assert_eq!(b[UCP_FLAGS_OFFSET], 0x01);
    }

    #[test]
    fn roundtrips_through_bytes() {
        let original = sample();
        let bytes = original.to_bytes();
        let (parsed, payload) = Packet::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, original);
        assert!(payload.is_empty());
    }

    #[test]
    fn returns_the_payload_after_the_header() {
        let mut bytes = sample().to_bytes().to_vec();
        bytes.extend_from_slice(&[0x02, 0x03, b'a', b'b', b'c']);
        let (_, payload) = Packet::from_bytes(&bytes).unwrap();
        assert_eq!(payload, &[0x02, 0x03, b'a', b'b', b'c']);
    }

    #[test]
    fn rejects_short_packets() {
        let bytes = [0u8; 26];
        let err = Packet::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ProtocolError::TooShort { got: 26, min: 27 }));
    }

    #[test]
    fn rejects_non_ucp_packets() {
        let mut bytes = sample().to_bytes();
        bytes[18] = 0xAA;
        bytes[19] = 0xBB;
        let err = Packet::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ProtocolError::NotUcp { udap_type: 0xAABB }));
    }

    // We broadcast with the request bit set; the kernel loops our own
    // packet back to us. The capture path uses this to skip it.
    #[test]
    fn identifies_our_own_looped_back_request() {
        let bytes = sample().to_bytes();
        assert!(is_request_packet(&bytes));
    }

    #[test]
    fn device_replies_are_not_requests() {
        let mut p = sample();
        p.ucp_flags = 0x00;
        assert!(!is_request_packet(&p.to_bytes()));
    }

    #[test]
    fn short_buffers_are_not_requests() {
        assert!(!is_request_packet(&[0u8; 10]));
        assert!(!is_request_packet(&[]));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap protocol
```

Expected: compilation failure — `Packet` is not defined.

- [ ] **Step 3: Write the error type**

`crates/udap/src/error.rs`:

```rust
//! Error types for the `udap` crate.

/// Errors from decoding a UDAP packet header.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("packet too short: {got} bytes (minimum {min})")]
    TooShort { got: usize, min: usize },
    #[error("not a UDAP/UCP packet: UDAPType=0x{udap_type:04x}")]
    NotUcp { udap_type: u16 },
}
```

- [ ] **Step 4: Implement the packet codec**

Prepend to `crates/udap/src/protocol.rs`:

```rust
//! UDAP packet header and protocol constants.
//!
//! All multi-byte fields are network byte order (big-endian), matching
//! the Net::UDAP Perl reference implementation.

use crate::error::ProtocolError;
use crate::Mac;

/// UDAP listens on this UDP port.
pub const PORT: u16 = 17784;

/// Serialized size of the packet header: the sum of its fields, no padding.
pub const HEADER_SIZE: usize = 27;

/// UDAPType value identifying a UCP packet.
pub const UDAP_TYPE_UCP: u16 = 0xC001;

/// Ethernet addressing. Real devices always use this.
pub const ADDR_TYPE_ETH: u8 = 0x01;

/// The only UAP class UDAP uses.
pub const UAP_CLASS_UCP: [u8; 4] = [0x00, 0x01, 0x00, 0x01];

/// Byte index of `ucp_flags` within the serialized header.
///
/// `dst_broadcast(1) + dst_type(1) + dst_address(6) + src_broadcast(1)
/// + src_type(1) + src_address(6) + sequence(2) + udap_type(2) = 20`
pub const UCP_FLAGS_OFFSET: usize = 20;

/// The request bit in `ucp_flags`. We send with it set; devices reply
/// with it clear.
pub const FLAG_REQUEST: u8 = 0x01;

/// UCP method numbers, per Net::UDAP `Constant.pm`.
pub mod method {
    pub const DISCOVER: u16 = 0x0001;
    pub const GET_IP: u16 = 0x0002;
    pub const RESET: u16 = 0x0004;
    pub const GET_DATA: u16 = 0x0005;
    pub const SET_DATA: u16 = 0x0006;
    pub const ERROR: u16 = 0x0007;
    pub const CREDENTIALS_ERROR: u16 = 0x0008;
    pub const ADV_DISC: u16 = 0x0009;
    pub const GET_UUID: u16 = 0x000b;
}

/// A UDAP packet header.
///
/// `ucp_method` is deliberately a raw `u16` rather than an enum: the
/// client reports unrecognised methods verbatim ("unexpected response
/// method 0x%04x"), which an enum could not represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub dst_broadcast: u8,
    pub dst_type: u8,
    pub dst_address: Mac,
    pub src_broadcast: u8,
    pub src_type: u8,
    pub src_address: Mac,
    pub sequence: u16,
    pub udap_type: u16,
    pub ucp_flags: u8,
    pub uap_class: [u8; 4],
    pub ucp_method: u16,
}

impl Packet {
    /// Serializes the header. Field order and offsets are the wire contract.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[0] = self.dst_broadcast;
        b[1] = self.dst_type;
        b[2..8].copy_from_slice(self.dst_address.as_bytes());
        b[8] = self.src_broadcast;
        b[9] = self.src_type;
        b[10..16].copy_from_slice(self.src_address.as_bytes());
        b[16..18].copy_from_slice(&self.sequence.to_be_bytes());
        b[18..20].copy_from_slice(&self.udap_type.to_be_bytes());
        b[20] = self.ucp_flags;
        b[21..25].copy_from_slice(&self.uap_class);
        b[25..27].copy_from_slice(&self.ucp_method.to_be_bytes());
        b
    }

    /// Parses a header, returning it alongside the remaining payload.
    ///
    /// Rejects anything shorter than [`HEADER_SIZE`] or whose UDAPType is
    /// not [`UDAP_TYPE_UCP`] — such packets are junk on our socket (mDNS
    /// leakage, stray broadcasts), not data to interpret.
    ///
    /// # Errors
    /// [`ProtocolError::TooShort`] or [`ProtocolError::NotUcp`].
    pub fn from_bytes(buf: &[u8]) -> Result<(Packet, &[u8]), ProtocolError> {
        if buf.len() < HEADER_SIZE {
            return Err(ProtocolError::TooShort {
                got: buf.len(),
                min: HEADER_SIZE,
            });
        }
        let mut dst = [0u8; 6];
        dst.copy_from_slice(&buf[2..8]);
        let mut src = [0u8; 6];
        src.copy_from_slice(&buf[10..16]);
        let mut uap_class = [0u8; 4];
        uap_class.copy_from_slice(&buf[21..25]);

        let packet = Packet {
            dst_broadcast: buf[0],
            dst_type: buf[1],
            dst_address: Mac::from_bytes(dst),
            src_broadcast: buf[8],
            src_type: buf[9],
            src_address: Mac::from_bytes(src),
            sequence: u16::from_be_bytes([buf[16], buf[17]]),
            udap_type: u16::from_be_bytes([buf[18], buf[19]]),
            ucp_flags: buf[20],
            uap_class,
            ucp_method: u16::from_be_bytes([buf[25], buf[26]]),
        };
        if packet.udap_type != UDAP_TYPE_UCP {
            return Err(ProtocolError::NotUcp {
                udap_type: packet.udap_type,
            });
        }
        Ok((packet, &buf[HEADER_SIZE..]))
    }
}

/// Reports whether `buf` is a UDAP packet with the request bit set.
///
/// The capture path uses this to skip our own kernel-looped broadcast:
/// we send with the request bit set, devices reply with it clear.
/// Returns `false` for buffers too short to contain the flags byte.
#[must_use]
pub fn is_request_packet(buf: &[u8]) -> bool {
    match buf.get(UCP_FLAGS_OFFSET) {
        Some(flags) => flags & FLAG_REQUEST != 0,
        None => false,
    }
}
```

- [ ] **Step 5: Wire into the crate root**

Add to `crates/udap/src/lib.rs`:

```rust
pub mod error;
pub mod protocol;

pub use error::ProtocolError;
pub use protocol::{Packet, HEADER_SIZE, PORT};
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
```

Expected: 10 new tests pass, clippy clean.

- [ ] **Step 7: Add a golden-bytes test against a real capture**

Copy a fixture from go-udap and assert we parse it identically:

```bash
mkdir -p crates/udap/tests/fixtures
cp ~/code/github.com/yo61/go-udap/mocksbr/testdata/captures/discovery-factory.bin \
   crates/udap/tests/fixtures/
```

Create `crates/udap/tests/golden_capture.rs`:

```rust
//! Parses real captured device responses, so a refactor that changes
//! decoding is caught against hardware-observed bytes rather than
//! against our own encoder.

use udap::protocol::{method, Packet, UDAP_TYPE_UCP};

const DISCOVERY_FACTORY: &[u8] = include_bytes!("fixtures/discovery-factory.bin");

#[test]
fn parses_a_real_discovery_response() {
    let (packet, payload) = Packet::from_bytes(DISCOVERY_FACTORY).expect("fixture must parse");
    assert_eq!(packet.udap_type, UDAP_TYPE_UCP);
    assert_eq!(packet.ucp_method, method::ADV_DISC);
    assert_eq!(packet.ucp_flags & 0x01, 0, "a device reply has the request bit clear");
    assert!(!payload.is_empty(), "discovery responses carry TLVs");
}

#[test]
fn decodes_the_discovery_tlvs() {
    let (_, payload) = Packet::from_bytes(DISCOVERY_FACTORY).expect("fixture must parse");
    let tlvs = udap::tlv::decode(payload);
    assert!(!tlvs.is_empty());
    // 0x09 is firmware_rev; every real device reports one.
    assert!(tlvs.iter().any(|t| t.tag == 0x09), "expected a firmware_rev TLV");
}
```

- [ ] **Step 8: Run the golden test**

```bash
mise exec -- cargo nextest run -p udap
```

Expected: passes. If `ucp_method` is not `ADV_DISC`, print the fixture's bytes and check which method the capture actually used before changing the assertion — the fixture is ground truth, the plan is not.

- [ ] **Step 9: Commit**

```bash
git add crates/udap/src/protocol.rs crates/udap/src/error.rs crates/udap/src/lib.rs crates/udap/tests/
git commit -S -m "feat(udap): add packet header codec and protocol constants

Ports the Packet struct, ParsePacket and isUDAPRequestPacket. Go used
binary.Read reflection over a no-padding struct; this writes the field
offsets out explicitly so the 27-byte layout is checked rather than
asserted in a comment.

Adds a golden test against go-udap's captured discovery-factory.bin."
```

---

### Task 5: Parameter table

Port of `udap/parameters.go`. **Not on the `discover` critical path** — see the reordering note above.

**Files:**
- Create: `crates/udap/src/parameters.rs`
- Modify: `crates/udap/src/lib.rs`, `crates/udap/src/error.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct Parameter { pub name, offset, length, placeholder, help, factory_default }` — all `&'static str` except `offset: u16` and `length: u16`
  - `pub const PARAMETERS: [Parameter; 26]`
  - `Parameter::flag_name(&self) -> String`
  - `Parameter::encode(&self, value: &str) -> Result<Vec<u8>, EncodeError>`
  - `pub fn by_name(name: &str) -> Option<&'static Parameter>` — resolves aliases
  - `pub fn by_offset(offset: u16) -> Option<&'static Parameter>`
  - `pub fn names() -> impl Iterator<Item = &'static str>`
  - `pub enum EncodeError`

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/parameters.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_26_entries() {
        assert_eq!(PARAMETERS.len(), 26);
    }

    #[test]
    fn every_entry_is_self_consistent() {
        for p in &PARAMETERS {
            assert!(!p.name.is_empty(), "{p:?} has an empty name");
            assert!(p.length > 0, "{} has zero length", p.name);
            assert!(p.length <= 256, "{} length {} exceeds max", p.name, p.length);
        }
    }

    #[test]
    fn offsets_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in &PARAMETERS {
            assert!(seen.insert(p.offset), "duplicate offset {} ({})", p.offset, p.name);
        }
    }

    #[test]
    fn lookup_by_name_works() {
        let p = by_name("wireless_SSID").expect("known parameter");
        assert_eq!(p.offset, 183);
        assert_eq!(p.length, 33);
    }

    #[test]
    fn unknown_names_return_none() {
        assert!(by_name("no_such_parameter").is_none());
    }

    #[test]
    fn aliases_resolve_to_the_canonical_entry() {
        let canonical = by_name("server_address").expect("known");
        assert_eq!(by_name("squeezecenter_address").map(|p| p.offset), Some(canonical.offset));
        assert_eq!(by_name("slimserver_address").map(|p| p.offset), Some(canonical.offset));
    }

    #[test]
    fn lookup_by_offset_works() {
        assert_eq!(by_offset(4).map(|p| p.name), Some("lan_ip_mode"));
        assert!(by_offset(9999).is_none());
    }

    #[test]
    fn flag_name_lowercases_and_hyphenates() {
        assert_eq!(by_name("wireless_SSID").unwrap().flag_name(), "wireless-ssid");
        assert_eq!(by_name("lan_ip_mode").unwrap().flag_name(), "lan-ip-mode");
    }

    #[test]
    fn encodes_one_byte_values() {
        let p = by_name("lan_ip_mode").unwrap();
        assert_eq!(p.encode("1").unwrap(), vec![1]);
        assert!(p.encode("256").is_err());
        assert!(p.encode("nope").is_err());
    }

    #[test]
    fn encodes_ipv4_values_as_four_bytes() {
        let p = by_name("lan_network_address").unwrap();
        assert_eq!(p.encode("192.168.1.50").unwrap(), vec![192, 168, 1, 50]);
        assert!(p.encode("not-an-ip").is_err());
        assert!(p.encode("::1").is_err(), "IPv6 must be rejected");
    }

    #[test]
    fn encodes_strings_zero_padded_to_length() {
        let p = by_name("hostname").unwrap();
        let out = p.encode("bedroom").unwrap();
        assert_eq!(out.len(), 33);
        assert_eq!(&out[..7], b"bedroom");
        assert!(out[7..].iter().all(|&b| b == 0), "remainder must be zero-padded");
    }

    #[test]
    fn encode_always_returns_exactly_length_bytes() {
        for p in &PARAMETERS {
            let sample = match p.length {
                1 => "1",
                2 => "1",
                4 => "192.168.1.1",
                _ => "x",
            };
            let out = p.encode(sample).unwrap();
            assert_eq!(out.len(), usize::from(p.length), "{} encoded to the wrong width", p.name);
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap parameters
```

Expected: compilation failure.

- [ ] **Step 3: Add the encode error variant**

Append to `crates/udap/src/error.rs`:

```rust
/// Errors from encoding a parameter value to its NVRAM wire form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("{value:?} is not a valid u8")]
    NotU8 { value: String },
    #[error("{value:?} is not a valid u16")]
    NotU16 { value: String },
    #[error("cannot parse {value:?} as an IPv4 address")]
    NotIpv4 { value: String },
}
```

- [ ] **Step 4: Implement the table**

Prepend to `crates/udap/src/parameters.rs`. **Copy the 26 rows verbatim from `udap/parameters.go`** — offsets and lengths come from the squeezeplay Lua reference and are wire contract:

```rust
//! The canonical table of UDAP NVRAM parameters.
//!
//! Single source of truth: the CLI flag table, `read` coverage, and the
//! offset reverse-lookup are all derived from `PARAMETERS`. To add a
//! parameter, add one row here.

use crate::error::EncodeError;
use std::net::Ipv4Addr;

/// One NVRAM-resident parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    /// Canonical wire name, used in protocol messages and INI files.
    pub name: &'static str,
    /// NVRAM byte offset.
    pub offset: u16,
    /// NVRAM field width in bytes. Drives the encoding.
    pub length: u16,
    /// Value form shown after the flag in `--help` (e.g. "IP", "0|1").
    pub placeholder: &'static str,
    /// End-user help text.
    pub help: &'static str,
    /// Value the device reports after a hardware reset. Captured from a
    /// real Squeezebox Receiver; used by `read` to filter uninteresting
    /// values.
    pub factory_default: &'static str,
}

/// Ordered list of every known parameter. Order is intentional and stable.
pub const PARAMETERS: [Parameter; 26] = [
    Parameter { name: "lan_ip_mode", offset: 4, length: 1, placeholder: "0|1", help: "0=static, 1=DHCP", factory_default: "1" },
    Parameter { name: "lan_network_address", offset: 5, length: 4, placeholder: "IP", help: "Static IPv4 address (e.g. 192.168.1.50)", factory_default: "0.0.0.0" },
    Parameter { name: "lan_subnet_mask", offset: 9, length: 4, placeholder: "MASK", help: "Subnet mask (e.g. 255.255.255.0)", factory_default: "255.255.255.0" },
    Parameter { name: "lan_gateway", offset: 13, length: 4, placeholder: "IP", help: "Default gateway IPv4 address", factory_default: "0.0.0.0" },
    Parameter { name: "hostname", offset: 17, length: 33, placeholder: "NAME", help: "Device hostname (max 33 chars)", factory_default: "" },
    Parameter { name: "bridging", offset: 50, length: 1, placeholder: "0|1", help: "0=disabled, 1=enabled", factory_default: "0" },
    Parameter { name: "interface", offset: 52, length: 1, placeholder: "0|1", help: "0=wireless, 1=wired (Ethernet)", factory_default: "128" },
    Parameter { name: "primary_dns", offset: 59, length: 4, placeholder: "IP", help: "Primary DNS server IPv4 address", factory_default: "0.0.0.0" },
    Parameter { name: "secondary_dns", offset: 67, length: 4, placeholder: "IP", help: "Secondary DNS server IPv4 address", factory_default: "0.0.0.0" },
    Parameter { name: "server_address", offset: 71, length: 4, placeholder: "IP", help: "Logitech Media Server IPv4 address", factory_default: "0.0.0.0" },
    Parameter { name: "lms_address", offset: 79, length: 4, placeholder: "IP", help: "Alternative LMS server IPv4 address", factory_default: "0.0.0.0" },
    Parameter { name: "squeezecenter_name", offset: 83, length: 33, placeholder: "NAME", help: "Squeezecenter / LMS server name (max 33 chars)", factory_default: "" },
    Parameter { name: "wireless_mode", offset: 173, length: 1, placeholder: "0|1", help: "0=infrastructure, 1=ad-hoc", factory_default: "0" },
    Parameter { name: "wireless_SSID", offset: 183, length: 33, placeholder: "SSID", help: "Wireless SSID (1-32 chars)", factory_default: "" },
    Parameter { name: "wireless_channel", offset: 216, length: 1, placeholder: "N", help: "Wireless channel (1-13)", factory_default: "6" },
    Parameter { name: "wireless_region_id", offset: 218, length: 1, placeholder: "ID", help: "Wireless region identifier (4=US, 6=CA, 7=AU, 13=FR, 14=EU, 16=JP, 21=TW, 23=CH)", factory_default: "4" },
    Parameter { name: "wireless_keylen", offset: 220, length: 1, placeholder: "5|13", help: "WEP key length", factory_default: "0" },
    Parameter { name: "wireless_wep_key", offset: 222, length: 13, placeholder: "HEX", help: "Primary WEP key", factory_default: "" },
    Parameter { name: "wireless_wep_key_1", offset: 235, length: 13, placeholder: "HEX", help: "WEP key slot 1", factory_default: "" },
    Parameter { name: "wireless_wep_key_2", offset: 248, length: 13, placeholder: "HEX", help: "WEP key slot 2", factory_default: "" },
    Parameter { name: "wireless_wep_key_3", offset: 261, length: 13, placeholder: "HEX", help: "WEP key slot 3", factory_default: "" },
    Parameter { name: "wireless_wep_on", offset: 274, length: 1, placeholder: "0|1", help: "0=disabled, 1=enabled", factory_default: "0" },
    Parameter { name: "wireless_wpa_cipher", offset: 275, length: 1, placeholder: "1|2|3", help: "1=TKIP, 2=AES (CCMP), 3=TKIP+AES", factory_default: "3" },
    Parameter { name: "wireless_wpa_mode", offset: 276, length: 1, placeholder: "1|2", help: "1=WPA, 2=WPA2", factory_default: "1" },
    Parameter { name: "wireless_wpa_on", offset: 277, length: 1, placeholder: "0|1", help: "0=disabled, 1=enabled", factory_default: "0" },
    Parameter { name: "wireless_wpa_psk", offset: 278, length: 64, placeholder: "PSK", help: "WPA pre-shared key (8-63 chars)", factory_default: "" },
];

/// Legacy and third-party names that refer to an existing parameter.
/// These get no `read` slot and no CLI flag; they resolve on lookup only.
const ALIASES: [(&str, &str); 2] = [
    ("slimserver_address", "server_address"),
    ("squeezecenter_address", "server_address"),
];

impl Parameter {
    /// The CLI flag form: lowercased, underscores to hyphens.
    #[must_use]
    pub fn flag_name(&self) -> String {
        self.name.to_lowercase().replace('_', "-")
    }

    /// Encodes `value` to exactly `self.length` bytes.
    ///
    /// Width drives the encoding: 1 is `u8`, 2 is big-endian `u16`, 4 is
    /// IPv4, anything else is zero-padded UTF-8 (truncated if too long).
    ///
    /// # Errors
    /// [`EncodeError`] if the value does not parse for this width.
    pub fn encode(&self, value: &str) -> Result<Vec<u8>, EncodeError> {
        match self.length {
            1 => {
                let n: u8 = value.parse().map_err(|_| EncodeError::NotU8 {
                    value: value.to_owned(),
                })?;
                Ok(vec![n])
            }
            2 => {
                let n: u16 = value.parse().map_err(|_| EncodeError::NotU16 {
                    value: value.to_owned(),
                })?;
                Ok(n.to_be_bytes().to_vec())
            }
            4 => {
                let ip: Ipv4Addr = value.parse().map_err(|_| EncodeError::NotIpv4 {
                    value: value.to_owned(),
                })?;
                Ok(ip.octets().to_vec())
            }
            width => {
                let mut out = vec![0u8; usize::from(width)];
                let src = value.as_bytes();
                let take = src.len().min(out.len());
                out[..take].copy_from_slice(&src[..take]);
                Ok(out)
            }
        }
    }
}

/// Looks up a parameter by canonical name, resolving aliases.
///
/// Linear scan: the table has 26 entries, so this beats hashing and
/// avoids the lazily-initialised global maps the Go version needs.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Parameter> {
    if let Some(p) = PARAMETERS.iter().find(|p| p.name == name) {
        return Some(p);
    }
    let canonical = ALIASES.iter().find(|(alias, _)| *alias == name)?.1;
    PARAMETERS.iter().find(|p| p.name == canonical)
}

/// Looks up a parameter by its NVRAM offset.
#[must_use]
pub fn by_offset(offset: u16) -> Option<&'static Parameter> {
    PARAMETERS.iter().find(|p| p.offset == offset)
}

/// Every canonical parameter name, in table order.
pub fn names() -> impl Iterator<Item = &'static str> {
    PARAMETERS.iter().map(|p| p.name)
}
```

- [ ] **Step 5: Wire in and run**

Add `pub mod parameters;` and `pub use error::EncodeError;` to `crates/udap/src/lib.rs`, then:

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
```

Expected: 12 new tests pass, clippy clean.

- [ ] **Step 6: Cross-check the table against the Go**

```bash
cd ~/code/github.com/yo61/go-udap
rg -o '\{"[a-z_A-Z]+", [0-9]+, [0-9]+' udap/parameters.go | sed 's/{//; s/"//g' > /tmp/go-params.txt
cd ~/code/github.com/yo61/udapcfg-rs
rg -o 'name: "[a-zA-Z_]+", offset: [0-9]+, length: [0-9]+' crates/udap/src/parameters.rs \
  | sed 's/name: //; s/offset: //; s/length: //; s/"//g' > /tmp/rs-params.txt
diff <(tr -d ' ' < /tmp/go-params.txt) <(tr -d ' ' < /tmp/rs-params.txt) && echo "TABLES MATCH"
```

Expected: `TABLES MATCH`. If not, the Go is right — fix the Rust.

- [ ] **Step 7: Commit**

```bash
git add crates/udap/src/parameters.rs crates/udap/src/error.rs crates/udap/src/lib.rs
git commit -S -m "feat(udap): add the NVRAM parameter table

Ports udap/parameters.go as a const array. The two lazily-initialised
lookup maps become linear scans: at 26 entries a scan beats hashing and
removes two pieces of global mutable state.

Offsets and lengths cross-checked against the Go table."
```

---

### Task 6: GetData response decoder

Port of `udap/getdata_response.go`. **Not on the `discover` critical path.**

**Files:**
- Create: `crates/udap/src/getdata.rs`
- Modify: `crates/udap/src/lib.rs`, `crates/udap/src/error.rs`

**Interfaces:**
- Consumes: `parameters::by_offset` (Task 5)
- Produces:
  - `pub fn parse_response(data: &[u8]) -> Result<BTreeMap<String, String>, GetDataError>`
  - `pub enum GetDataError` with `PayloadTooShort`, `TruncatedHeader`, `ItemExceedsPayload`

**Why `BTreeMap`:** the Go returns `map[string]string` and every consumer sorts before printing. A `BTreeMap` is sorted by construction, so the sort disappears and output ordering is deterministic without extra work.

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/getdata.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a response payload: u16 count, then count x (u16 offset,
    /// u16 length, value bytes).
    fn payload(items: &[(u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        #[allow(clippy::cast_possible_truncation, reason = "test data is small")]
        out.extend_from_slice(&(items.len() as u16).to_be_bytes());
        for (offset, value) in items {
            out.extend_from_slice(&offset.to_be_bytes());
            #[allow(clippy::cast_possible_truncation, reason = "test data is small")]
            out.extend_from_slice(&(value.len() as u16).to_be_bytes());
            out.extend_from_slice(value);
        }
        out
    }

    #[test]
    fn decodes_a_one_byte_numeric() {
        let got = parse_response(&payload(&[(4, &[1])])).unwrap();
        assert_eq!(got.get("lan_ip_mode").map(String::as_str), Some("1"));
    }

    #[test]
    fn decodes_a_four_byte_value_as_dotted_quad() {
        let got = parse_response(&payload(&[(5, &[192, 168, 1, 50])])).unwrap();
        assert_eq!(got.get("lan_network_address").map(String::as_str), Some("192.168.1.50"));
    }

    #[test]
    fn decodes_a_string_and_trims_at_the_first_nul() {
        let mut value = b"bedroom".to_vec();
        value.resize(33, 0);
        let got = parse_response(&payload(&[(17, &value)])).unwrap();
        assert_eq!(got.get("hostname").map(String::as_str), Some("bedroom"));
    }

    #[test]
    fn unknown_offsets_become_synthetic_hex_keys() {
        let got = parse_response(&payload(&[(9999, &[0xde, 0xad])])).unwrap();
        assert_eq!(got.get("offset_9999").map(String::as_str), Some("dead"));
    }

    #[test]
    fn decodes_multiple_items() {
        let got = parse_response(&payload(&[(4, &[1]), (5, &[10, 0, 0, 5])])).unwrap();
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn rejects_a_payload_too_short_for_the_count() {
        assert!(matches!(parse_response(&[0x00]), Err(GetDataError::PayloadTooShort { .. })));
    }

    #[test]
    fn rejects_a_truncated_item_header() {
        // count=1 but only two bytes of the four-byte item header follow
        let data = [0x00, 0x01, 0x00, 0x04];
        assert!(matches!(parse_response(&data), Err(GetDataError::TruncatedHeader { .. })));
    }

    #[test]
    fn rejects_an_item_longer_than_the_payload() {
        // count=1, offset=4, length=100, but no value bytes follow
        let data = [0x00, 0x01, 0x00, 0x04, 0x00, 0x64];
        assert!(matches!(parse_response(&data), Err(GetDataError::ItemExceedsPayload { .. })));
    }

    // A crafted count of 65535 with a tiny body must not pre-allocate a
    // huge map. Go clamps the size hint; we must too.
    #[test]
    fn oversized_count_does_not_allocate_wildly() {
        let data = [0xff, 0xff, 0x00, 0x04, 0x00, 0x01, 0x07];
        let err = parse_response(&data).unwrap_err();
        assert!(matches!(err, GetDataError::TruncatedHeader { .. }));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run -p udap getdata
```

Expected: compilation failure.

- [ ] **Step 3: Add the error variants**

Append to `crates/udap/src/error.rs`:

```rust
/// Errors from decoding a GetData response payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GetDataError {
    #[error("getdata response: payload too short ({got} bytes)")]
    PayloadTooShort { got: usize },
    #[error("getdata response: truncated header for item {index} at offset {pos}")]
    TruncatedHeader { index: usize, pos: usize },
    #[error(
        "getdata response: item {index} (NVRAM offset {offset}, length {length}) \
         exceeds payload ({remaining} bytes left)"
    )]
    ItemExceedsPayload {
        index: usize,
        offset: u16,
        length: u16,
        remaining: usize,
    },
}
```

- [ ] **Step 4: Implement the decoder**

Prepend to `crates/udap/src/getdata.rs`:

```rust
//! Decoder for the GetData (0x0005) response payload.
//!
//! Wire format: `u16 count`, then `count` items of
//! `u16 offset, u16 length, length bytes`. Verified against Net::UDAP
//! wire captures.

use crate::error::GetDataError;
use crate::parameters;
use std::collections::BTreeMap;

/// Decodes a GetData response payload — everything after the 27-byte header.
///
/// Offsets are mapped back to parameter names via the parameter table.
/// Unrecognised offsets are recorded under a synthetic `offset_<decimal>`
/// key with the raw bytes hex-encoded, matching go-udap.
///
/// # Errors
/// [`GetDataError`] if the payload is malformed.
pub fn parse_response(data: &[u8]) -> Result<BTreeMap<String, String>, GetDataError> {
    if data.len() < 2 {
        return Err(GetDataError::PayloadTooShort { got: data.len() });
    }
    let count = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let mut pos = 2usize;
    let mut out = BTreeMap::new();

    for index in 0..count {
        if pos + 4 > data.len() {
            return Err(GetDataError::TruncatedHeader { index, pos });
        }
        let offset = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);
        pos += 4;
        if pos + usize::from(length) > data.len() {
            return Err(GetDataError::ItemExceedsPayload {
                index,
                offset,
                length,
                remaining: data.len() - pos,
            });
        }
        let value = &data[pos..pos + usize::from(length)];
        pos += usize::from(length);

        match parameters::by_offset(offset) {
            Some(p) => {
                out.insert(p.name.to_owned(), format_value(value));
            }
            None => {
                out.insert(format!("offset_{offset}"), hex_encode(value));
            }
        }
    }
    Ok(out)
}

/// Renders a raw NVRAM value so it round-trips back through
/// `Parameter::encode`.
fn format_value(value: &[u8]) -> String {
    match value.len() {
        1 => value[0].to_string(),
        2 => u16::from_be_bytes([value[0], value[1]]).to_string(),
        4 => format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]),
        _ => {
            let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
            String::from_utf8_lossy(&value[..end]).into_owned()
        }
    }
}

fn hex_encode(value: &[u8]) -> String {
    let mut s = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write;
        // Writing to a String is infallible.
        let _ = write!(s, "{byte:02x}");
    }
    s
}
```

- [ ] **Step 5: Wire in, run, commit**

Add `pub mod getdata;` and `pub use error::GetDataError;` to `crates/udap/src/lib.rs`.

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
git add crates/udap/src/getdata.rs crates/udap/src/error.rs crates/udap/src/lib.rs
git commit -S -m "feat(udap): add GetData response decoder

Ports udap/getdata_response.go. Returns a BTreeMap so ordering is
deterministic without the caller sorting. Bounds checks match the Go,
including refusing to trust a declared item count larger than the
payload can hold."
```

- [ ] **Step 6: Add a fuzzing property test**

Append to the test module:

```rust
#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // This decoder runs on network input from an untrusted LAN peer.
        #[test]
        fn never_panics_on_arbitrary_input(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = parse_response(&data);
        }
    }
}
```

```bash
mise exec -- cargo nextest run -p udap
git add crates/udap/src/getdata.rs
git commit -S -m "test(udap): fuzz the GetData decoder against arbitrary input"
```

---

### Task 7: Transport trait and mock device

The M2 walking skeleton starts here. Build the `Transport` seam and the minimum `mocksbr` that answers advanced discovery.

**Files:**
- Create: `crates/udap/src/transport/mod.rs`
- Create: `crates/mocksbr/src/device.rs`, `network.rs`, `responses.rs`, `transport.rs`
- Modify: `crates/udap/src/lib.rs`, `crates/mocksbr/src/lib.rs`

**Interfaces:**
- Consumes: `Packet`, `Mac`, `tlv::encode_into`, `protocol::method` (Tasks 2–4)
- Produces:
  - `#[async_trait] pub trait Transport: Send + Sync { async fn send(&self, packet: &[u8]) -> Result<(), TransportError>; async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError>; async fn close(&self) -> Result<(), TransportError>; }`
  - `pub enum TransportError { Cancelled, Io(std::io::Error) }`
  - `mocksbr::DeviceConfig { mac: Mac, name: String, model: String, device_id: String, firmware: String, hardware: String }`
  - `mocksbr::DeviceConfig::default_with_mac(mac: Mac) -> DeviceConfig`
  - `mocksbr::Network::new(devices: Vec<DeviceConfig>) -> Network`
  - `mocksbr::Network::with_auto_devices(n: usize) -> Network` — MACs `00:04:20:00:00:01`..
  - `mocksbr::MockTransport::new(network: Arc<Network>) -> MockTransport`

- [ ] **Step 1: Write the failing test**

`crates/mocksbr/tests/discovery.rs`:

```rust
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use udap::protocol::{method, Packet, ADDR_TYPE_ETH, UAP_CLASS_UCP, UDAP_TYPE_UCP};
use udap::transport::Transport;
use udap::Mac;

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

#[tokio::test]
async fn mock_answers_advanced_discovery() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    transport.send(&adv_discovery_request()).await.unwrap();
    let (reply, _src) = transport.recv(&cancel).await.unwrap();

    let (packet, payload) = Packet::from_bytes(&reply).unwrap();
    assert_eq!(packet.ucp_method, method::ADV_DISC);
    assert_eq!(packet.ucp_flags, 0x00, "replies clear the request bit");
    assert_eq!(packet.src_address.to_string(), "00:04:20:00:00:01");
    assert_eq!(packet.sequence, 1, "sequence is echoed");
    assert!(!payload.is_empty());
}

#[tokio::test]
async fn every_device_replies() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(3));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    transport.send(&adv_discovery_request()).await.unwrap();
    let mut macs = Vec::new();
    for _ in 0..3 {
        let (reply, _) = transport.recv(&cancel).await.unwrap();
        let (packet, _) = Packet::from_bytes(&reply).unwrap();
        macs.push(packet.src_address.to_string());
    }
    macs.sort();
    assert_eq!(macs, ["00:04:20:00:00:01", "00:04:20:00:00:02", "00:04:20:00:00:03"]);
}

#[tokio::test]
async fn recv_returns_cancelled_when_the_token_fires() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = transport.recv(&cancel).await.unwrap_err();
    assert!(matches!(err, udap::transport::TransportError::Cancelled));
}

#[tokio::test]
async fn non_discovery_methods_are_ignored_for_now() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    let mut request = adv_discovery_request();
    request[25..27].copy_from_slice(&method::GET_DATA.to_be_bytes());
    transport.send(&request).await.unwrap();

    cancel.cancel();
    assert!(transport.recv(&cancel).await.is_err(), "no reply should be queued");
}
```

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run -p mocksbr
```

Expected: compilation failure — `mocksbr::Network` does not exist.

- [ ] **Step 3: Define the `Transport` trait**

`crates/udap/src/transport/mod.rs`:

```rust
//! The network abstraction beneath `Client`.
//!
//! Addressing is encoded in the packets themselves, not at this layer:
//! `send` broadcasts, and the destination MAC lives inside the packet.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("transport I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Send and receive raw UDAP packets.
///
/// Implemented by the real UDP transport and by `mocksbr::MockTransport`
/// for hermetic in-process tests.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Dispatches a packet. The destination is encoded in the packet.
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError>;

    /// Waits for the next packet, or until `cancel` fires.
    ///
    /// Returns the raw bytes and an informational source identifier —
    /// an IP for the UDP transport, a MAC for the mock. The source is
    /// for logging and reply validation; routing uses packet contents.
    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError>;

    /// Releases transport resources.
    async fn close(&self) -> Result<(), TransportError>;
}
```

Add to `crates/udap/src/lib.rs`:

```rust
pub mod transport;
```

- [ ] **Step 4: Implement the mock device and its reply builder**

`crates/mocksbr/src/device.rs`:

```rust
//! One virtual Squeezebox Receiver.

use udap::Mac;

/// Per-device configuration. This is the M2 subset; fault-injection
/// knobs arrive with the full mocksbr port.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub mac: Mac,
    /// Reported as TLV 0x02 (device_name).
    pub name: String,
    /// Reported as TLV 0x03 (device_type).
    pub model: String,
    /// Reported as TLV 0x0b (device_id). "07" is a Receiver.
    pub device_id: String,
    /// Reported as TLV 0x09 (firmware_rev).
    pub firmware: String,
    /// Reported as TLV 0x0a (hardware_rev).
    pub hardware: String,
    /// Reported as TLV 0x0c (device_status).
    pub state: String,
}

impl DeviceConfig {
    /// Builds a device with go-udap's mocksbr defaults.
    #[must_use]
    pub fn default_with_mac(mac: Mac) -> Self {
        DeviceConfig {
            mac,
            name: "Mock SBR".to_owned(),
            model: "squeezebox".to_owned(),
            device_id: "07".to_owned(),
            firmware: "77".to_owned(),
            hardware: "0005".to_owned(),
            state: "wait_slimserver".to_owned(),
        }
    }
}
```

`crates/mocksbr/src/responses.rs`:

```rust
//! Reply builders. Layout and TLV order match go-udap's mocksbr so the
//! committed wire captures stay valid.

use crate::device::DeviceConfig;
use udap::protocol::{Packet, ADDR_TYPE_ETH, UAP_CLASS_UCP, UDAP_TYPE_UCP};
use udap::tlv;

/// Builds a reply header: addresses swapped, our MAC as source, the
/// request's sequence echoed, request bit cleared.
fn build_header(request: &Packet, cfg: &DeviceConfig, method: u16) -> Packet {
    Packet {
        dst_broadcast: 0,
        dst_type: ADDR_TYPE_ETH,
        dst_address: request.src_address,
        src_broadcast: 0,
        src_type: ADDR_TYPE_ETH,
        src_address: cfg.mac,
        sequence: request.sequence,
        udap_type: UDAP_TYPE_UCP,
        ucp_flags: 0x00,
        uap_class: UAP_CLASS_UCP,
        ucp_method: method,
    }
}

/// Builds a discovery response: header plus TLVs in go-udap's order —
/// state, device_id, hardware_rev, firmware_rev, device_type, device_name.
#[must_use]
pub fn discovery_response(request: &Packet, cfg: &DeviceConfig) -> Vec<u8> {
    let header = build_header(request, cfg, request.ucp_method);
    let mut out = header.to_bytes().to_vec();
    tlv::encode_into(0x0c, cfg.state.as_bytes(), &mut out);
    tlv::encode_into(0x0b, cfg.device_id.as_bytes(), &mut out);
    tlv::encode_into(0x0a, cfg.hardware.as_bytes(), &mut out);
    tlv::encode_into(0x09, cfg.firmware.as_bytes(), &mut out);
    tlv::encode_into(0x03, cfg.model.as_bytes(), &mut out);
    tlv::encode_into(0x02, cfg.name.as_bytes(), &mut out);
    out
}
```

`crates/mocksbr/src/network.rs`:

```rust
//! A network of virtual devices that a `MockTransport` can drive.

use crate::device::DeviceConfig;
use crate::responses;
use udap::protocol::{method, Packet};
use udap::Mac;

pub struct Network {
    devices: Vec<DeviceConfig>,
}

impl Network {
    #[must_use]
    pub fn new(devices: Vec<DeviceConfig>) -> Self {
        Network { devices }
    }

    /// Builds `n` devices with sequential MACs from `00:04:20:00:00:01`.
    ///
    /// # Panics
    /// If `n` exceeds 255.
    #[must_use]
    pub fn with_auto_devices(n: usize) -> Self {
        assert!(n <= 255, "with_auto_devices supports at most 255 devices");
        let devices = (1..=n)
            .map(|i| {
                #[allow(clippy::cast_possible_truncation, reason = "n is asserted <= 255")]
                let last = i as u8;
                let mut cfg = DeviceConfig::default_with_mac(Mac::from_bytes([
                    0x00, 0x04, 0x20, 0x00, 0x00, last,
                ]));
                cfg.name = format!("Mock SBR {i}");
                cfg
            })
            .collect();
        Network { devices }
    }

    /// Handles one request, returning every reply it provokes.
    ///
    /// M2 answers advanced discovery only; other methods produce no
    /// reply, which is also how a real device behaves when it does not
    /// recognise a request.
    #[must_use]
    pub fn receive(&self, packet: &[u8]) -> Vec<(Vec<u8>, String)> {
        let Ok((request, _payload)) = Packet::from_bytes(packet) else {
            return Vec::new();
        };
        if request.ucp_method != method::ADV_DISC {
            return Vec::new();
        }
        self.devices
            .iter()
            .map(|cfg| {
                (
                    responses::discovery_response(&request, cfg),
                    cfg.mac.to_string(),
                )
            })
            .collect()
    }
}
```

`crates/mocksbr/src/transport.rs`:

```rust
//! `MockTransport` — a `udap::Transport` backed by an in-process `Network`.

use crate::network::Network;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use udap::transport::{Transport, TransportError};

pub struct MockTransport {
    network: Arc<Network>,
    pending: Mutex<std::collections::VecDeque<(Vec<u8>, String)>>,
    notify: Notify,
}

impl MockTransport {
    #[must_use]
    pub fn new(network: Arc<Network>) -> Self {
        MockTransport {
            network,
            pending: Mutex::new(std::collections::VecDeque::new()),
            notify: Notify::new(),
        }
    }

    /// Pops one queued reply, if any.
    ///
    /// The lock is taken and released inside this function so it is
    /// never held across an `.await` — which `await_holding_lock` denies.
    fn pop(&self) -> Option<(Vec<u8>, String)> {
        let mut queue = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        queue.pop_front()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError> {
        let replies = self.network.receive(packet);
        {
            let mut queue = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            queue.extend(replies);
        }
        self.notify.notify_waiters();
        Ok(())
    }

    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
        loop {
            if cancel.is_cancelled() {
                return Err(TransportError::Cancelled);
            }
            if let Some(reply) = self.pop() {
                return Ok(reply);
            }
            tokio::select! {
                () = cancel.cancelled() => return Err(TransportError::Cancelled),
                () = self.notify.notified() => {}
            }
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        Ok(())
    }
}
```

`crates/mocksbr/src/lib.rs`:

```rust
//! In-process mock Squeezebox Receiver, for testing `udap` without hardware.

pub mod device;
pub mod network;
pub mod responses;
pub mod transport;

pub use device::DeviceConfig;
pub use network::Network;
pub use transport::MockTransport;
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
mise exec -- cargo nextest run -p mocksbr
mise exec -- cargo clippy --all-targets
```

Expected: 4 tests pass, clippy clean.

Note the `unwrap_or_else(|e| e.into_inner())` on the mutex: `unwrap_used` is denied, and a poisoned lock here means a test panicked while holding it — recovering the data is the right response, not a second panic.

- [ ] **Step 6: Commit**

```bash
git add crates/udap/src/transport crates/udap/src/lib.rs crates/mocksbr/
git commit -S -m "feat: add Transport trait and a discovery-answering mock device

The Transport trait is the seam that lets one Client run against a real
socket or an in-process mock. MockTransport answers advanced discovery
(0x0009) for every configured device; other methods stay silent, which
is what a real device does with a request it does not recognise."
```

---

### Task 8: Device and discovery client

Port of `udap/discovery.go` and the discovery half of `udap/client.go`.

**Files:**
- Create: `crates/udap/src/device.rs`, `crates/udap/src/client.rs`
- Modify: `crates/udap/src/lib.rs`

**Interfaces:**
- Consumes: `Transport`, `Packet`, `Mac`, `tlv::decode` (Tasks 2–4, 7)
- Produces:
  - `pub struct Device { pub mac: Mac, pub ip: String, pub name: String, pub model: String, pub firmware: String, pub hardware_rev: String, pub uuid: String, pub state: String }`
  - `pub struct Client` with `Client::new(transport: Box<dyn Transport>) -> Client`
  - `Client::set_retries(&mut self, n: usize)`
  - `Client::discover(&mut self, cancel: &CancellationToken) -> Result<(), ClientError>`
  - `Client::devices(&self) -> Vec<&Device>` — sorted by MAC
  - `Client::close(&self) -> Result<(), ClientError>`

**Behaviour notes:**
- Discovery loops on `recv` until the cancellation token fires; a cancelled recv ends discovery **successfully** (that's the timeout path), matching go-udap's `errors.Is(err, context.DeadlineExceeded) -> return nil`.
- Replies whose `src_type` is not `ADDR_TYPE_ETH` are **skipped with a warning**, not turned into pseudo-MAC devices.
- An empty `device_name` TLV falls back to `"Squeezebox Device"`.
- The sequence counter starts at 0 and pre-increments, so the first packet has sequence 1.

- [ ] **Step 1: Write the failing tests**

`crates/udap/tests/discovery.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::Client;

/// Runs discovery with a short deadline, the way the CLI does.
async fn discover_with_deadline(client: &mut Client, timeout: Duration) {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        token.cancel();
    });
    client.discover(&cancel).await.expect("discovery must not error on timeout");
}

#[tokio::test]
async fn finds_every_mock_device() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(3));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50)).await;

    let macs: Vec<String> = client.devices().iter().map(|d| d.mac.to_string()).collect();
    assert_eq!(macs, ["00:04:20:00:00:01", "00:04:20:00:00:02", "00:04:20:00:00:03"]);
}

#[tokio::test]
async fn populates_device_metadata_from_tlvs() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50)).await;

    let devices = client.devices();
    let d = devices.first().expect("one device");
    assert_eq!(d.name, "Mock SBR 1");
    assert_eq!(d.firmware, "77");
    assert_eq!(d.hardware_rev, "0005");
    assert_eq!(d.state, "wait_slimserver");
    // device_id "07" maps to the product name, not the raw device_type.
    assert_eq!(d.model, "Squeezebox Receiver");
}

#[tokio::test]
async fn empty_network_discovers_nothing_without_erroring() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(0));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50)).await;

    assert!(client.devices().is_empty());
}

#[tokio::test]
async fn devices_are_returned_sorted_by_mac() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(5));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50)).await;

    let macs: Vec<String> = client.devices().iter().map(|d| d.mac.to_string()).collect();
    let mut sorted = macs.clone();
    sorted.sort();
    assert_eq!(macs, sorted);
}

#[tokio::test]
async fn rediscovery_does_not_duplicate_devices() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(2));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50)).await;
    discover_with_deadline(&mut client, Duration::from_millis(50)).await;

    assert_eq!(client.devices().len(), 2);
}
```

Add to `crates/udap/Cargo.toml` under `[dev-dependencies]`:

```toml
mocksbr = { path = "../mocksbr" }
```

(Cargo permits this cycle — `mocksbr` depends on `udap`, and `udap`'s *dev*-dependency on `mocksbr` is legal. Go would reject it as an import cycle.)

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run -p udap discovery
```

Expected: compilation failure — `Client` does not exist.

- [ ] **Step 3: Implement `Device`**

`crates/udap/src/device.rs`:

```rust
//! Discovered device metadata.

use crate::Mac;

/// A device found by discovery. Fields come from the response TLVs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    pub mac: Mac,
    /// Source address the reply arrived from.
    pub ip: String,
    /// TLV 0x02 device_name.
    pub name: String,
    /// Derived from TLV 0x03 device_type and TLV 0x0b device_id.
    pub model: String,
    /// TLV 0x09 firmware_rev.
    pub firmware: String,
    /// TLV 0x0a hardware_rev.
    pub hardware_rev: String,
    /// TLV 0x0d uuid, hex-encoded.
    pub uuid: String,
    /// TLV 0x0c device_status.
    pub state: String,
}

/// Maps a device_id (TLV 0x0b, a 2-character ASCII hex string) to its
/// product name. Source: squeezeplay device tables. Only "07"
/// (Receiver) has been verified against real hardware.
const PRODUCT_BY_ID: [(&str, &str); 10] = [
    ("02", "Squeezebox 2"),
    ("03", "Squeezebox 3"),
    ("04", "Transporter"),
    ("05", "SoftSqueeze"),
    ("06", "Squeezebox Boom"),
    ("07", "Squeezebox Receiver"),
    ("08", "Squeezebox Touch"),
    ("09", "Squeezebox Radio"),
    ("0a", "Squeezebox Controller"),
    ("0b", "Squeezeslave"),
];

/// Renders a friendly model string, falling back gracefully.
#[must_use]
pub fn combine_model(device_type: &str, device_id: &str) -> String {
    if let Some((_, product)) = PRODUCT_BY_ID.iter().find(|(id, _)| *id == device_id) {
        return (*product).to_owned();
    }
    match (device_type.is_empty(), device_id.is_empty()) {
        (false, false) => format!("{device_type} (id={device_id})"),
        (false, true) => device_type.to_owned(),
        _ => String::new(),
    }
}
```

- [ ] **Step 4: Implement `Client`**

`crates/udap/src/client.rs`:

```rust
//! The UDAP client: owns a transport and the map of discovered devices.

use crate::device::{combine_model, Device};
use crate::protocol::{method, Packet, ADDR_TYPE_ETH, FLAG_REQUEST, UAP_CLASS_UCP, UDAP_TYPE_UCP};
use crate::transport::{Transport, TransportError};
use crate::{tlv, Mac};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("send discovery: {0}")]
    Send(#[source] TransportError),
    #[error("recv during discovery: {0}")]
    Recv(#[source] TransportError),
}

/// Discovery-response TLV codes, per Net::UDAP `Constant.pm`.
mod tlv_code {
    pub const DEVICE_NAME: u8 = 0x02;
    pub const DEVICE_TYPE: u8 = 0x03;
    pub const FIRMWARE_REV: u8 = 0x09;
    pub const HARDWARE_REV: u8 = 0x0a;
    pub const DEVICE_ID: u8 = 0x0b;
    pub const DEVICE_STATUS: u8 = 0x0c;
    pub const UUID: u8 = 0x0d;
}

pub struct Client {
    transport: Box<dyn Transport>,
    /// Keyed by `Mac` directly. Go keys by the string form and its
    /// `recordDevice` comment explains the compromise; `Mac` is
    /// `Copy + Eq + Hash`, so here it costs nothing.
    devices: BTreeMap<Mac, Device>,
    sequence: u16,
    retries: usize,
}

impl Client {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Client {
            transport,
            devices: BTreeMap::new(),
            sequence: 0,
            retries: 0,
        }
    }

    /// Sets the number of re-transmissions beyond the initial send.
    /// `n` of 2 means three total sends.
    pub fn set_retries(&mut self, n: usize) {
        self.retries = n;
    }

    /// Every discovered device, ordered by MAC.
    #[must_use]
    pub fn devices(&self) -> Vec<&Device> {
        self.devices.values().collect()
    }

    /// Releases the transport.
    ///
    /// # Errors
    /// Propagates the transport's close error.
    pub async fn close(&self) -> Result<(), TransportError> {
        self.transport.close().await
    }

    /// Builds a header with the next sequence number.
    fn next_packet(&mut self, dst: Mac, ucp_method: u16, broadcast: bool) -> Packet {
        self.sequence = self.sequence.wrapping_add(1);
        Packet {
            dst_broadcast: u8::from(broadcast),
            dst_type: ADDR_TYPE_ETH,
            dst_address: dst,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::ZERO,
            sequence: self.sequence,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: FLAG_REQUEST,
            uap_class: UAP_CLASS_UCP,
            ucp_method,
        }
    }

    /// Sends `packet`, retransmitting `self.retries` more times.
    ///
    /// UDP send is fire-and-forget: succeeds if any attempt succeeded,
    /// returns the first error only if every attempt failed. No delay
    /// between sends, matching squeezeplay's triple-send.
    async fn send_retried(&self, packet: &[u8]) -> Result<(), TransportError> {
        let mut first_err = None;
        let mut successes = 0usize;
        for _ in 0..=self.retries {
            match self.transport.send(packet).await {
                Ok(()) => successes += 1,
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        match (successes, first_err) {
            (0, Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Broadcasts an advanced-discovery request and collects replies
    /// until `cancel` fires.
    ///
    /// A cancelled receive ends discovery **successfully** — that is the
    /// normal timeout path, matching go-udap.
    ///
    /// # Errors
    /// [`ClientError::Send`] if every send attempt failed, or
    /// [`ClientError::Recv`] on a non-cancellation transport error.
    pub async fn discover(&mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        info!(method = "0x0009", "starting UDAP discovery");
        let packet = self.next_packet(Mac::ZERO, method::ADV_DISC, true).to_bytes();
        self.send_retried(&packet).await.map_err(ClientError::Send)?;

        loop {
            match self.transport.recv(cancel).await {
                Ok((reply, src)) => self.handle_discovery_reply(&reply, &src),
                Err(TransportError::Cancelled) => return Ok(()),
                Err(e) => return Err(ClientError::Recv(e)),
            }
        }
    }

    fn handle_discovery_reply(&mut self, bytes: &[u8], src: &str) {
        let (packet, payload) = match Packet::from_bytes(bytes) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!(src, error = %e, "failed to parse discovery reply");
                return;
            }
        };
        // Real devices reply with Ethernet addressing. AddrTypeUDP is a
        // wire-spec constant no observed device uses, and a pseudo-MAC
        // would not be usable by any downstream operation.
        if packet.src_type != ADDR_TYPE_ETH {
            warn!(
                src,
                src_type = format!("0x{:02x}", packet.src_type),
                "ignoring discovery reply with non-Ethernet source type"
            );
            return;
        }
        let device = parse_discovery_response(payload, src, &packet);
        info!(mac = %device.mac, name = %device.name, ip = %device.ip, "found device");
        self.devices.insert(device.mac, device);
    }
}

/// Builds a `Device` from a discovery response payload.
fn parse_discovery_response(payload: &[u8], src: &str, packet: &Packet) -> Device {
    let mut device = Device {
        mac: packet.src_address,
        ip: src.to_owned(),
        ..Device::default()
    };
    let mut device_type = String::new();
    let mut device_id = String::new();

    for entry in tlv::decode(payload) {
        let text = String::from_utf8_lossy(entry.value).into_owned();
        match entry.tag {
            tlv_code::DEVICE_NAME => device.name = text,
            tlv_code::DEVICE_TYPE => device_type = text,
            tlv_code::FIRMWARE_REV => device.firmware = text,
            tlv_code::DEVICE_ID => device_id = text,
            tlv_code::DEVICE_STATUS => device.state = text,
            tlv_code::HARDWARE_REV => device.hardware_rev = text,
            tlv_code::UUID => device.uuid = hex_encode(entry.value),
            tag => debug!(tag = format!("0x{tag:02x}"), len = entry.value.len(), "unknown discovery TLV"),
        }
    }

    device.model = combine_model(&device_type, &device_id);
    if device.name.is_empty() {
        device.name = "Squeezebox Device".to_owned();
    }
    device
}

fn hex_encode(value: &[u8]) -> String {
    let mut s = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write;
        let _ = write!(s, "{byte:02x}");
    }
    s
}
```

- [ ] **Step 5: Wire in and run**

Add to `crates/udap/src/lib.rs`:

```rust
pub mod client;
pub mod device;

pub use client::{Client, ClientError};
pub use device::Device;
```

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets
```

Expected: 5 new tests pass, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add crates/udap/src/client.rs crates/udap/src/device.rs crates/udap/src/lib.rs crates/udap/tests/ crates/udap/Cargo.toml
git commit -S -m "feat(udap): add Client and advanced discovery

Ports udap/discovery.go and the discovery half of udap/client.go.

Devices are keyed by Mac rather than its string form: Mac is Copy + Eq
+ Hash, so the compromise recordDevice apologises for is unnecessary
here. A cancelled receive ends discovery successfully, which is the
timeout path."
```

---

### Task 9: `udapcfg discover`

The CLI, and the end of the walking skeleton.

**Files:**
- Create: `crates/udap-cli/src/cli.rs`, `src/cmd/mod.rs`, `src/cmd/discover.rs`
- Modify: `crates/udap-cli/src/main.rs`
- Create: `crates/udap-cli/tests/e2e_discover.rs`

**Interfaces:**
- Consumes: `Client`, `Device`, `Transport` (Tasks 7–8)
- Produces:
  - `pub struct Cli` (clap `Parser`) with `--timeout`, `--verbose/-v`, `--retries`, and the `discover` subcommand
  - `pub async fn run(cli: Cli, make_client: ClientFactory, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), CliError>`
  - `pub type ClientFactory = Box<dyn Fn() -> Result<Client, anyhow::Error>>`
  - `pub struct CliError { pub code: i32, pub source: anyhow::Error }`

**Output contract** (from `cli/discover.go`, minus `--info` which is a later milestone):
- One MAC per line on **stdout**, sorted ascending.
- If nothing found: `no devices found within <timeout>` on **stderr**, exit **0**.
- Discovery failure: exit **2**.

**Dependency injection note:** go-udap swaps a package-level `newClient` variable in tests. Rust makes mutable globals painful, so `run` takes a factory instead. This removes the global *and* the "e2e tests must not be parallel" constraint the Go harness carries.

- [ ] **Step 1: Write the failing e2e test**

`crates/udap-cli/tests/e2e_discover.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;
use udap_cli::{run, Cli, Command};

/// Runs the CLI against an in-process mock, returning (stdout, stderr, exit code).
async fn run_cli(device_count: usize, timeout: Duration) -> (String, String, i32) {
    let network = Arc::new(mocksbr::Network::with_auto_devices(device_count));
    let factory = Box::new(move || {
        Ok(udap::Client::new(Box::new(mocksbr::MockTransport::new(
            Arc::clone(&network),
        ))))
    });
    let cli = Cli {
        timeout,
        verbose: false,
        retries: 0,
        command: Command::Discover,
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match run(cli, factory, &mut out, &mut err).await {
        Ok(()) => 0,
        Err(e) => {
            use std::io::Write;
            let _ = writeln!(&mut err, "error: {}", e.source);
            e.code
        }
    };
    (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap(), code)
}

#[tokio::test]
async fn prints_one_mac_per_line_sorted() {
    let (stdout, _, code) = run_cli(3, Duration::from_millis(50)).await;
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "00:04:20:00:00:01\n00:04:20:00:00:02\n00:04:20:00:00:03\n"
    );
}

#[tokio::test]
async fn reports_no_devices_on_stderr_and_exits_zero() {
    let (stdout, stderr, code) = run_cli(0, Duration::from_millis(50)).await;
    assert_eq!(code, 0, "finding nothing is not an error");
    assert!(stdout.is_empty(), "stdout must stay clean");
    assert_eq!(stderr, "no devices found within 50ms\n");
}

#[tokio::test]
async fn results_go_to_stdout_not_stderr() {
    let (stdout, stderr, _) = run_cli(1, Duration::from_millis(50)).await;
    assert!(stdout.contains("00:04:20:00:00:01"));
    assert!(stderr.is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run -p udap-cli
```

Expected: compilation failure — `udap_cli` has no library target yet.

- [ ] **Step 3: Add a library target to the CLI crate**

Add to `crates/udap-cli/Cargo.toml`, above `[[bin]]`:

```toml
[lib]
name = "udap_cli"
path = "src/lib.rs"
```

- [ ] **Step 4: Write the CLI definitions**

`crates/udap-cli/src/cli.rs`:

```rust
//! Command-line interface definitions.

use clap::{Parser, Subcommand};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "udapcfg",
    version,
    about = "Squeezebox UDAP configuration tool",
    long_about = "udapcfg discovers and configures Squeezebox devices over UDAP\n\
                  (Universal Device Access Protocol) on UDP port 17784.\n\n\
                  It is single-shot: every invocation runs one subcommand to\n\
                  completion and exits.\n\n\
                  UDAP only talks to devices in setup mode (the front light\n\
                  flashes red). Brand-new devices arrive in setup mode; existing\n\
                  devices can be put back into it by holding the front button\n\
                  for 3-6 seconds."
)]
pub struct Cli {
    /// Operation timeout, e.g. 2s, 30s, 2m
    #[arg(long, global = true, value_name = "DURATION",
          default_value = "2s", value_parser = humantime_parse)]
    pub timeout: Duration,

    /// Debug logging to stderr
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// Re-transmit each UDAP send N additional times
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    pub retries: usize,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Discover devices on the network
    #[command(long_about = "Broadcast a UDAP advanced-discover packet on UDP port 17784\n\
                            and print every Squeezebox device that responds within\n\
                            --timeout. MAC addresses are printed one per line.\n\n\
                            Sends always target the limited broadcast address\n\
                            255.255.255.255 so unconfigured devices (which have no\n\
                            DHCP lease and so no notion of a subnet broadcast\n\
                            address) can hear them.")]
    Discover,
}

/// Parses a Go-style duration string ("2s", "500ms", "1m").
///
/// # Errors
/// Returns a message suitable for clap when the input is not a duration.
fn humantime_parse(s: &str) -> Result<Duration, String> {
    parse_duration(s).ok_or_else(|| format!("invalid duration {s:?} (try 2s, 500ms, 1m)"))
}

fn parse_duration(s: &str) -> Option<Duration> {
    let (value, unit) = s.split_at(s.find(|c: char| c.is_alphabetic())?);
    let n: u64 = value.parse().ok()?;
    match unit {
        "ms" => Some(Duration::from_millis(n)),
        "s" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_secs(n * 60)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_duration_forms_go_udap_accepts() {
        assert_eq!(parse_duration("2s"), Some(Duration::from_secs(2)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
    }

    #[test]
    fn rejects_nonsense_durations() {
        assert_eq!(parse_duration("banana"), None);
        assert_eq!(parse_duration("2"), None);
        assert_eq!(parse_duration("2h"), None);
    }
}
```

- [ ] **Step 5: Write the discover command and the runner**

`crates/udap-cli/src/cmd/discover.rs`:

```rust
//! The `discover` subcommand.

use crate::{CliError, ClientFactory};
use std::io::Write;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Discovers devices and prints one MAC per line.
///
/// Finding nothing is not an error: a note goes to stderr and the exit
/// code stays 0, matching go-udap.
///
/// # Errors
/// [`CliError`] with code 2 if the client cannot be built or discovery fails.
pub async fn run(
    make_client: ClientFactory,
    timeout: Duration,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    let mut client = make_client().map_err(|e| CliError { code: 2, source: e })?;

    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        token.cancel();
    });

    client.discover(&cancel).await.map_err(|e| CliError {
        code: 2,
        source: anyhow::Error::new(e).context("discovery failed"),
    })?;

    let devices = client.devices();
    if devices.is_empty() {
        let _ = writeln!(stderr, "no devices found within {}", format_duration(timeout));
        return Ok(());
    }
    for device in devices {
        let _ = writeln!(stdout, "{}", device.mac);
    }
    Ok(())
}

/// Renders a duration the way Go's `time.Duration` prints it, so the
/// "no devices found within 2s" line matches go-udap byte for byte.
fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms % 60_000 == 0 && ms > 0 {
        format!("{}m0s", ms / 60_000)
    } else if ms % 1_000 == 0 {
        format!("{}s", ms / 1_000)
    } else {
        format!("{ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations_like_go() {
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_millis(50)), "50ms");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m0s");
    }
}
```

`crates/udap-cli/src/cmd/mod.rs`:

```rust
pub mod discover;
```

`crates/udap-cli/src/lib.rs`:

```rust
//! The `udapcfg` command-line tool.

pub mod cli;
pub mod cmd;

pub use cli::{Cli, Command};

use std::io::Write;

/// An error carrying the process exit code to use.
///
/// 0 success, 1 usage error, 2 operation failure.
#[derive(Debug)]
pub struct CliError {
    pub code: i32,
    pub source: anyhow::Error,
}

/// Builds a `udap::Client`. Injected so tests can substitute a
/// mock-backed client without a mutable global.
pub type ClientFactory = Box<dyn Fn() -> Result<udap::Client, anyhow::Error>>;

/// Dispatches the parsed command.
///
/// # Errors
/// [`CliError`] carrying the exit code the process should use.
pub async fn run(
    cli: Cli,
    make_client: ClientFactory,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    match cli.command {
        Command::Discover => cmd::discover::run(make_client, cli.timeout, stdout, stderr).await,
    }
}
```

- [ ] **Step 6: Write `main.rs`**

`crates/udap-cli/src/main.rs`:

```rust
//! Entry point for the `udapcfg` binary.

use clap::Parser;
use std::io::Write;
use std::process::ExitCode;
use udap_cli::{run, Cli, ClientFactory};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(if cli.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::WARN
        })
        .init();

    let retries = cli.retries;
    let factory: ClientFactory = Box::new(move || {
        // M3 replaces this with the real UDP transport.
        Err(anyhow::anyhow!(
            "no transport available yet: the UDP transport lands in M3 \
             (retries={retries} will apply then)"
        ))
    });

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    match run(cli, factory, &mut stdout, &mut stderr).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(&mut stderr, "error: {}", e.source);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "exit codes are 0, 1 or 2"
            )]
            ExitCode::from(e.code as u8)
        }
    }
}
```

- [ ] **Step 7: Run the tests to verify they pass**

```bash
mise exec -- cargo nextest run --workspace
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo fmt --all --check
```

Expected: all tests pass, clippy and fmt clean.

- [ ] **Step 8: Verify the binary runs and its help renders**

```bash
mise exec -- cargo run -p udap-cli -- --help
mise exec -- cargo run -p udap-cli -- discover --help
mise exec -- cargo run -p udap-cli -- discover; echo "exit=$?"
```

Expected: help renders with the program name `udapcfg`. `discover` exits 2 with the "no transport available yet" message — correct for M2, since the real UDP transport is M3.

- [ ] **Step 9: Compare help output against go-udap**

```bash
cd ~/code/github.com/yo61/go-udap && go build -o /tmp/go-udap . && cd -
diff <(/tmp/go-udap discover --help) \
     <(mise exec -- cargo run -q -p udap-cli -- discover --help)
```

Expected: differences confined to the program name, clap-versus-cobra section headers, and flags not yet implemented (`--info`, `--bind-interface`, `--all-interfaces`). Record anything else as a defect to fix now — the fidelity contract covers help text modulo program name.

- [ ] **Step 10: Commit**

```bash
git add crates/udap-cli/
git commit -S -m "feat(cli): add the discover subcommand

Completes the M2 walking skeleton: udapcfg discover prints one MAC per
line, sorted, against an in-process mock.

run() takes a client factory rather than swapping a package-level
variable the way the Go e2e harness does. That removes the global and
the constraint that e2e tests cannot run in parallel."
```

- [ ] **Step 11: Open the pull request**

```bash
mise exec -- cargo nextest run --workspace
git push -u origin feat/m0-workspace
gh pr create --title "M0-M2: workspace, protocol core, walking skeleton" \
  --body "Implements M0 through M2 of docs/specs/2026-09-08-rust-port-spec.md.

Protocol core (Mac, TLV, Packet, parameters, GetData decoder) with tests
ported from go-udap, plus the Transport seam, a discovery-answering mock
device, and \`udapcfg discover\` end to end against it.

The real UDP transport is M3; \`discover\` against actual hardware does
not work yet and exits 2 with a message saying so."
```

---

## Verification checklist

Before calling M0–M2 done:

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --all-targets --all-features` clean with zero warnings
- [ ] `cargo nextest run --workspace` all green
- [ ] `cargo deny check` clean
- [ ] `udapcfg discover --help` renders with the correct program name
- [ ] The parameter table diff against `udap/parameters.go` reports `TABLES MATCH`
- [ ] The golden capture test parses `discovery-factory.bin`
- [ ] `Cargo.lock` is committed
- [ ] No `#[allow(...)]` without a `reason = "..."`

## What this plan deliberately leaves out

Each becomes its own plan:

- **M3** — real UDP transport (`socket2` + tokio hand-off), `netdev` interface enumeration, `--bind-interface`, `--all-interfaces` via `MultiTransport`, the Windows `#[cfg]` arm. **Also resolves OQ-2** (whether `SO_BINDTOIFINDEX` needs privileges) and verifies OQ-1's assumption that `netdev` populates flags with default features off.
- **M4** — `get_data`, `set_data`, `reset`, `get_ip`, `get_uuid`; the read-modify-write in `set`; the ADR-3 ownership work.
- **M5** — full `mocksbr`: every handler, the fault-injection knobs, the standalone binary.
- **M6** — remaining subcommands, output formatting, the indicatif progress bar, INI source layering, man pages, completions.
- **M7** — cross-compilation, packaging, SBOMs (OQ-3, OQ-4).
