# udap-rs — port specification

**Status:** draft, awaiting review
**Date:** 2026-09-08
**Source of truth:** [`yo61/go-udap`](https://github.com/yo61/go-udap) `v2.4.8` (`43864a5`)

## Goal

Port go-udap to Rust as a **faithful reimplementation**: same CLI surface, same
output, same wire bytes, same exit codes. The driver is learning Rust on a
codebase whose behaviour is already pinned down by 6,258 lines of tests, so the
work is translation rather than design.

## Non-goals

- **Replacing go-udap.** go-udap stays the maintained tool. This is a parallel
  learning project, not a migration. The global "replace, don't deprecate" rule
  applies to two implementations *within* one project; it does not apply here.
- **Improving the design.** Where the Go has a wart (see
  [Known warts](#known-warts-carried-forward)) we carry it forward and record
  it. Redesign is a separate decision, taken later, with the port finished.
- **New features.** No subcommand, flag, or output format that go-udap does not
  have.
- **Publishing to crates.io.** Out of scope until at least M3 lands.

## Success criteria

1. `udap-rs` produces byte-identical UDAP packets to go-udap for every operation,
   verified against the same wire captures.
2. Every go-udap CLI invocation produces identical stdout, stderr, and exit code.
3. `mocksbr` drives the Rust CLI through the same e2e scenarios the Go suite
   covers today.
4. It talks to real hardware — a Squeezebox Receiver in setup mode — for
   `discover`, `info`, `read`, `get`, `set`, `getip`, `reboot`.
5. Robin can explain, unprompted, why `client.rs` is shaped the way it is.

Criterion 5 is the real one. The others are how we know we did not cheat to get
there.

## Fidelity contract

"Faithful" means these are pinned and any change is a spec amendment:

| Surface | Contract |
| --- | --- |
| Wire bytes | Identical for all six UCP methods, including field order, padding, and the sort-by-offset in `get_data`/`set_data` |
| Subcommands | `discover`, `info`, `read`, `get`, `set`, `reboot`, `getip`, `interfaces` |
| Global flags | `--timeout`, `--retries`, `--verbose/-v`, `--version`, `--help/-h`, `--bind-interface`, `--all-interfaces`; accepted before *or* after the subcommand |
| Exit codes | 0 success, 1 usage error, 2 operation failure |
| Streams | Results on stdout; logs, warnings, progress bar on stderr |
| Output text | Byte-identical, including the `-` placeholder for absent network values and the fixed-column `interfaces` table |
| Retries | `--retries N` = N *re-transmissions* beyond the initial send, no inter-send delay |
| Broadcast target | Always `255.255.255.255`, never a directed subnet broadcast |

That last row is load-bearing. go-udap's
`docs/superpowers/plans/2026-05-13-getip-hwrev-uuid-iface.md` records a spike
where binding to the interface IP and sending to a directed broadcast meant
pre-DHCP devices never replied. **Do not re-derive this on hardware.** Copy the
behaviour.

## Architecture decisions

### ADR-1: threads, not async

`MultiTransport` is the only genuinely concurrent component. Port it as one
`std::thread` per child transport feeding a `std::sync::mpsc` channel.

*Rejected:* tokio. It is a large dependency for a process that performs one
operation and exits, and async Rust is a second learning curve stacked on the
first. Threads are also the more faithful translation of goroutines-plus-channels.

*Revisit if:* a future feature needs concurrent operations against many devices.

### ADR-2: `Deadline`, not `Context`

Replace `ctx context.Context` with `Deadline(Instant)`. The CLI is single-shot
and never cancels early — it only ever times out. This deletes the 200 ms
polling loop in `UDPTransport.Recv`, whose sole purpose is re-checking `ctx`.

*Trade-off:* no early cancellation. Nothing uses it today; Ctrl-C terminates the
process.

### ADR-3: ownership over aliasing

`Client` owns `HashMap<Mac, Device>`. Operations take `&self` plus `&mut Device`
with the caller holding the device. `Device` is cloned rather than shared.

*Rejected:* `Arc<Mutex<Device>>`, which would reproduce the Go aliasing contract
faithfully but teach nothing and hide the problem.

### ADR-4: no serde

`Device`'s `json:` tags and `Mac`'s `MarshalText`/`UnmarshalText` are exercised
**only from tests** — there is no CLI JSON output path and no non-test
`encoding/json` import in go-udap. Taking `serde` + `serde_json` to satisfy
tests alone is not justified.

*Revisit if:* a `--json` output flag is ever added (a feature, hence out of scope).

### ADR-5: `Mac` as the map key

Key `devices` by `Mac`, not by its string form. The Go keys by string and
`recordDevice` explains why: promoting the key type would force every caller to
parse at the boundary. In Rust `Mac` is `Copy + Eq + Hash`, parsing at the CLI
boundary is where it belongs, and the compromise is unnecessary.

*This is the one place we knowingly diverge from the Go's internal structure.*
It changes no observable behaviour.

## Crate layout

```
udap-rs/
├── Cargo.toml              # [workspace]
├── crates/
│   ├── udap/               # protocol, transport, client — no I/O beyond UDP
│   ├── mocksbr/            # fake Squeezebox Receiver (lib + bin)
│   └── udap-cli/           # the `go-udap` binary
└── xtask/                  # man pages, completions, release helpers
```

## Dependencies

Each entry needs a justification; go-udap ships with two direct dependencies and
we should not casually exceed that.

| Crate | Version | Replaces | Justification |
| --- | --- | --- | --- |
| `clap` (derive, wrap_help) | 4.6 | cobra + pflag | Subcommands, help, and the derive/builder mix the generated flags need |
| `socket2` (all) | 0.6 | `syscall.Setsockopt*`, `net.ListenConfig` | `SO_BROADCAST`, `SO_REUSEPORT`, interface binding. `all` feature gates `bind_device_by_index_v4` |
| `thiserror` | 2.0 | `fmt.Errorf` in `udap` | Library error enums |
| `anyhow` | 1.0 | `fmt.Errorf` in `cli` | Application error context |
| `tracing` | 0.1 | `udap/logger.go` | Structured logging |
| `tracing-subscriber` | 0.3 | — | `fmt` layer with a custom `MakeWriter` for stderr sync |
| `clap_mangen` | 0.3 | `cmd/docs` | Man pages from the clap tree (build/xtask only) |
| `clap_complete` | 4.6 | `cli/completion.go` | Shell completions (build/xtask only) |
| `insta` | 1.48 | golden string comparisons | dev-only |
| `serial_test` | 4.0 | Go's per-process test isolation | dev-only; Rust runs tests as threads in one process |

Versions are current stable as of 2026-09-08 (verified against the crates.io
API). Pin exact versions per the project standard.

**Explicitly not taken:** `serde` (ADR-4), `tokio` (ADR-1), `indicatif` (the
progress bar's logger-interleaving behaviour is bespoke; `std::io::IsTerminal`
covers TTY detection), `deku`/`binrw` (one 27-byte struct does not justify a
proc macro), `hex` (the Go hand-rolls nibble decoding for the same reason).

## Milestones

Ordered so the pure-logic modules teach the idioms before the ownership fight.
Each milestone ends with its Go tests ported and passing.

**M1 — pure protocol** (`mac`, `protocol`, `tlv`, `parameters`, `getdata`,
`netconfig`, `validation`)
No I/O. Roughly 40% of `udap` by value. The Go test files port almost
line-for-line. *Done when:* `cargo test -p udap` passes with the ported suite,
and encoding a known packet produces bytes identical to a go-udap capture.

**M2 — transport** (`transport/udp`, `interfaces`, the Windows `#[cfg]` arm)
First I/O, first platform conditionals. *Done when:* a real broadcast reaches a
real device and a reply is received, on macOS and Linux.

**M3 — client and operations** (`client`, `ops/*`, `transport/multi`)
The ownership work and the only concurrency. *Done when:* all six UCP operations
round-trip against `mocksbr`.

**M4 — mocksbr**
Needed in full before the CLI e2e suite can be ported. Both the library and the
standalone binary.

**M5 — CLI**
clap, output formatting, progress bar, INI source layering, man pages,
completions. *Done when:* every go-udap e2e scenario passes with identical
stdout/stderr/exit code.

**M6 — release plumbing**
CI, cross-compilation, packaging. See [Open questions](#open-questions).

M1 alone is a defensible stopping point: `udap` is a standalone library and the
learning-per-hour is highest there.

## Testing strategy

Port the Go tests as the primary source of truth — they encode hardware
behaviour that would otherwise have to be rediscovered.

- **Unit:** alongside each module, mirroring the `*_test.go` files.
- **Wire fidelity:** golden-byte tests against go-udap's committed captures in
  `mocksbr/testdata/captures/`. Copy those fixtures in verbatim.
- **e2e:** `mocksbr` in-process, driving `run()` with captured stdout/stderr, via
  `insta` snapshots.
- **Property:** `proptest` for the TLV codec and `parseGetDataResponse` — both
  are parsers over adversarial input and both have hand-rolled bounds checks
  worth fuzzing. (Adds a dev-dependency; justified by the project standard's
  "property-based testing for parsers".)
- **Mutation:** `cargo-mutants` on `udap` once M1 is stable, to confirm the
  ported tests actually catch failures.

Follow the project standard on environment: tests declare what they need rather
than inheriting it, and anything touching env vars uses `serial_test`.

## Toolchain and CI

**Prerequisite: no Rust toolchain is currently installed on this machine.**
`rustc`, `cargo`, and `rustup` are all absent. Install via `rustup` before M1.
`cargo-deny`, `cargo-audit`, and `cargo-mutants` are also not installed.

Per the project standard:

```
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo test
cargo deny check          # advisories, licenses, bans
```

`Cargo.toml` carries the standard `[lints.clippy]` block (pedantic, panic
prevention, `dbg_macro`/`todo`/`print_stdout` denied). Note `print_stdout = "deny"`
needs a scoped `allow` in `udap-cli`'s output module, since printing to stdout is
that module's job — add it with a justification comment rather than weakening the
lint globally.

## Accepted behavioural deltas

Deviations we are taking knowingly. Anything not listed here is a bug.

| Delta | Impact |
| --- | --- |
| Linux interface binding moves from `SO_BINDTODEVICE` (by name) to `SO_BINDTOIFINDEX` (by index) | Requires **kernel 5.7+**. go-udap works on older kernels. Affects `--bind-interface` / `--all-interfaces` only |
| `devices` keyed by `Mac` rather than its string form (ADR-5) | None observable |
| No early cancellation, only deadlines (ADR-2) | None observable |
| Man pages and completions generated by clap rather than cobra | Wording may differ slightly; snapshot them and review |

## Known warts carried forward

Recorded so we do not "fix" them mid-port and lose fidelity, and so a later
redesign has a list to work from.

1. **`Device.Parameters` mixes known and unknown keys.** Unrecognised NVRAM
   offsets are stored as synthetic `offset_NNN` string keys, which then need
   explicit cleanup in `GetAllDeviceConfigWithContext` to stop the map growing
   without bound. An enum key would make that structurally impossible.
2. **`DeviceConfig` has ~10 boolean test knobs** (`DropGetData`, `DropGetIP`,
   `DropGetUUID`, `SuppressDiscoveryUUID`, `Unreachable`, …). A `Vec<Fault>`
   would model this better.
3. **`SetDeviceConfig` is a full read-modify-write of all 26 parameters** on
   every set. Correct — omitted parameters would zero neighbouring NVRAM — but
   it means a one-parameter change costs two round trips.
4. **`squeezecenter_address` / `slimserver_address` aliases** exist in the
   parameter table with no `read` slot or CLI flag, resolving only on lookup.

## Not ported

Faithfulness is to observable behaviour, not to the function list. These exist
in go-udap and are deliberately left out:

- **`Client.CreateDiscoveryPacket()`** — builds a plain UDAP discovery packet
  (`UCP_METHOD_DISCOVER`, `0x0001`). Verified dead: it is never called outside
  tests, and all real discovery uses `CreateAdvancedDiscoveryPacket` (`0x0009`).
  Porting it would give dead code a second life in a codebase where it looks
  load-bearing. *Worth raising as a deletion against go-udap separately.*

If a later hardware test shows `0x0001` is needed for some firmware revision,
this becomes a spec amendment, not a silent addition.

## Open questions

Numbered so they can be closed individually. None block M1.

**OQ-1 — interface enumeration.** `udap/interfaces.go` uses Go's
`net.Interfaces()` for name, index, IPv4 address, and the Up/Broadcast/!Loopback
flags. Rust's std has no equivalent. Candidates: `if-addrs`, `network-interface`,
`nix` + raw `getifaddrs`. Needs a maintenance and cross-platform check
(macOS/Linux/Windows) before choosing. *Blocks M2.* **Resolve by:** comparing
crate maintenance and testing enumeration output against `go-udap interfaces` on
both platforms.

**OQ-2 — `SO_BINDTOIFINDEX` privileges.** `SO_BINDTODEVICE` needs `CAP_NET_RAW`
on most Linux distributions, and go-udap's error message says so. Whether
`SO_BINDTOIFINDEX` has the same requirement is unverified. If it differs, the
error text in the fidelity contract changes. *Blocks M2.* **Resolve by:** testing
as an unprivileged user on Linux.

**OQ-3 — release and packaging.** `.goreleaser.yaml` produces archives, a
Homebrew cask, SPDX + CycloneDX SBOMs, and bundles man pages and completions.
`cargo-dist` is actively maintained (v0.32.0, May 2026, axodotdev) and is the
closest equivalent, but has not been evaluated against this feature set —
particularly the cask and dual SBOM formats. Alternative: a hand-rolled GitHub
Actions matrix. *Blocks M6 only.* **Resolve by:** prototyping `cargo-dist` at M5.

**OQ-4 — cross-compilation.** Which of `cross` (Docker), `cargo-zigbuild`, or
per-target rustup toolchains to use for Windows and Linux builds from macOS.
This is the largest practical regression versus Go. *Blocks M6 only.*
**Resolve by:** trying `cargo-zigbuild` first — no Docker requirement.

**OQ-5 — Windows `--bind-interface`.** go-udap returns "not yet supported" on
Windows; the equivalent is `IP_UNICAST_IF`. `socket2` may expose this, which
would make Rust's Windows support *better* than the Go's. Faithfulness says keep
the error; opportunism says take the win. *Recommendation:* keep the error for
the port, note it as the first post-port improvement. *Blocks nothing.*

**OQ-6 — `mocksbr` capture fixtures.** go-udap commits six raw per-packet
fixtures under `mocksbr/testdata/captures/`: `discovery-configured.bin`,
`discovery-factory.bin`, `reset-ack.bin`, `savedata-status-ack.bin`,
`setdata-empty-ack.bin`, `setdata-status-ack.bin`. They are raw bytes, so they
copy verbatim; the question is whether the Go loader encodes framing or
offset assumptions that need porting alongside them. *Blocks M4.* **Resolve by:**
reading `mocksbr/fixture_test.go` when M4 starts.

**OQ-7 — repo tooling parity.** Which of go-udap's supporting setup to
replicate: `prek` hooks, commitlint, release-please, Dependabot, the Taskfile.
`release-please` supports Rust. Deferring all of it until M5 is defensible for a
learning project. *Blocks nothing.* **Recommendation:** add `prek` + `cargo fmt`
/ `clippy` hooks at M1; defer the rest.

## Rejected alternatives

**Rewrite from the protocol spec rather than porting.** The Go encodes hardware
behaviour discovered by wire-capture — the limited-broadcast finding, the macOS
`File()` hazard, the factory-default table captured from a real Receiver after
reset. A clean-room rewrite would rediscover those on hardware, slowly.

**Port incrementally with Rust called from Go (cgo/FFI).** Adds a build system
and an FFI boundary to a project whose entire point is learning Rust.

**Start with the CLI to get something runnable sooner.** The CLI is the least
interesting part to write in Rust and depends on everything else. Working
outside-in would mean stubbing the whole library first.
