# udapcfg-rs — port specification

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

1. `udapcfg` produces byte-identical UDAP packets to go-udap for every operation,
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
| Binary name | **`udapcfg`**, not `go-udap` — the two must be installable side by side. See the exception below |
| Subcommands | `discover`, `info`, `read`, `get`, `set`, `reboot`, `getip`, `interfaces` |
| Global flags | `--timeout`, `--retries`, `--verbose/-v`, `--version`, `--help/-h`, `--bind-interface`, `--all-interfaces`; accepted before *or* after the subcommand |
| Exit codes | 0 success, 1 usage error, 2 operation failure |
| Streams | Results on stdout; logs, warnings, progress bar on stderr |
| Output text | Byte-identical, including the `-` placeholder for absent network values and the fixed-column `interfaces` table |
| Retries | `--retries N` = N *re-transmissions* beyond the initial send, no inter-send delay |
| Broadcast target | Always `255.255.255.255`, never a directed subnet broadcast |

**Program-name exception.** Because the binary is `udapcfg`, "identical output"
means *identical modulo the program name*. Affected surfaces:

- `--help` and all subcommand help (usage lines, "Usage: udapcfg …")
- usage-error messages on stderr
- `--version`, which prints `udapcfg X.Y.Z` rather than `go-udap X.Y.Z`
- man page filenames (`udapcfg.1`, `udapcfg-discover.1`, …) and their `.TH` header
- completion script names and their internal function prefixes

Everywhere else — device output, parameter dumps, the `interfaces` table, error
text about devices — the program name does not appear and byte-identity holds
without qualification. When porting go-udap's golden output, substitute the
program name and nothing else; any *other* diff is a bug.

The always-broadcast row below is load-bearing. go-udap's
`docs/superpowers/plans/2026-05-13-getip-hwrev-uuid-iface.md` records a spike
where binding to the interface IP and sending to a directed broadcast meant
pre-DHCP devices never replied. **Do not re-derive this on hardware.** Copy the
behaviour.

## Architecture decisions

### ADR-1: async on tokio

Use `tokio` as the async runtime. `tokio::net::UdpSocket` for transport,
`tokio::select!` for the `MultiTransport` merge, `tokio::sync::mpsc` for
channels, `tokio::time::timeout` for deadlines.

*Rationale:* learning async Rust is an explicit goal of the project, and this is
a well-suited codebase for it — the concurrency is real but small, so the async
surface stays comprehensible. It also happens to be the more faithful
translation: goroutines-plus-channels map onto tasks-plus-channels far more
directly than onto OS threads, and `MultiTransport`'s per-child pump goroutine
becomes a spawned task almost line-for-line.

*Cost accepted:* tokio is a large dependency for a single-shot CLI, and startup
carries runtime-initialisation overhead that a threaded build would not. Use
`#[tokio::main(flavor = "current_thread")]` — this workload has no CPU
parallelism to exploit, and the single-threaded scheduler avoids spawning a
worker pool for a process that sends a handful of UDP packets and exits.

*Rejected:* `std::thread` + `mpsc`. Fewer dependencies and a smaller binary, but
it teaches the thing Robin already knows from Go rather than the thing this
project exists to learn.

### ADR-2: `CancellationToken` + `timeout`, mirroring `Context`

Choosing tokio (ADR-1) makes this *more* faithful than the alternative would
have been. `context.Context` carries both a deadline and a cancellation signal;
`tokio_util::sync::CancellationToken` paired with `tokio::time::timeout` is a
direct analogue of both halves, so operation signatures keep their shape:

| Go | Rust |
| --- | --- |
| `f(ctx context.Context, ...)` | `async fn f(&self, cancel: &CancellationToken, ...)` |
| `ctx, cancel := context.WithTimeout(parent, d)` | `timeout(d, fut).await` |
| `<-ctx.Done()` | `cancel.cancelled().await` |
| `errors.Is(err, context.DeadlineExceeded)` | `Err(Elapsed)` from `timeout` |

This also deletes the 200 ms polling loop in `UDPTransport.Recv`, whose only
purpose is re-checking `ctx` against a blocking socket — `select!` on a real
async socket needs no polling.

*Note:* an earlier draft of this spec proposed replacing `Context` with a plain
`Deadline(Instant)` and dropping cancellation, on the grounds that the CLI never
cancels early. That simplification is no longer needed, and cancellation now
comes essentially free.

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
udapcfg-rs/
├── Cargo.toml              # [workspace]
├── mise.toml               # pinned toolchain
├── crates/
│   ├── udap/               # protocol, transport, client — no I/O beyond UDP
│   ├── mocksbr/            # fake Squeezebox Receiver (lib + bin)
│   └── udap-cli/           # produces the `udapcfg` binary
└── xtask/                  # man pages, completions, release helpers
```

The CLI crate is `udap-cli` but its binary is `udapcfg`. The suffix-free binary
name is deliberate: `go-udap` encoded its implementation language into what
users type, and `-rs` would repeat that mistake. The marker belongs on the
repo, where it disambiguates in a listing, not on the command:

```toml
[[bin]]
name = "udapcfg"
path = "src/main.rs"
```

## Dependencies

Each entry needs a justification; go-udap ships with two direct dependencies and
we should not casually exceed that.

| Crate | Version | Replaces | Justification |
| --- | --- | --- | --- |
| `tokio` (rt, net, time, sync, macros) | 1.53 | goroutines, channels, `context` | Async runtime (ADR-1). `current_thread` flavour — no worker pool |
| `tokio-util` | 0.7 | `context.Context` cancellation | `CancellationToken` (ADR-2). No features needed — `tokio_util::sync` is not feature-gated |
| `async-trait` | 0.1 | Go interface methods | `Transport` needs `dyn` dispatch, and AFIT traits are still not `dyn`-compatible in Rust 1.98. Drop it if that lands |
| `clap` (derive, wrap_help) | 4.6 | cobra + pflag | Subcommands, help, and the derive/builder mix the generated flags need |
| `indicatif` | 0.18 | `cli/progress.go`, `cli/stderr.go` | Progress bar. Replaces the ticker, the erase-line dance, and the TTY check — see [Progress bar](#progress-bar) |
| `netdev` (no default features) | 0.46 | `net.Interfaces()` | Interface enumeration with real `IFF_*` flags — see [OQ-1](#open-questions) |
| `socket2` (all) | 0.6 | `syscall.Setsockopt*`, `net.ListenConfig` | `SO_BROADCAST`, `SO_REUSEPORT`, interface binding. `all` feature gates `bind_device_by_index_v4`. See [socket construction](#socket-construction) |
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

**Explicitly not taken:** `serde` (ADR-4), `deku`/`binrw` (one 27-byte struct
does not justify a proc macro), `hex` (the Go hand-rolls nibble decoding for the
same reason), `tracing-indicatif` (the one integration point — clearing the bar
around a log write — is a single `suspend()` call; a crate for that is not
earned).

### Progress bar

`indicatif` subsumes most of what `cli/progress.go` and `cli/stderr.go` build by
hand:

| go-udap hand-rolls | indicatif |
| --- | --- |
| ticker goroutine | `enable_steady_tick(Duration)` |
| `stderrSync`'s erase-then-write + `barActive` flag | `ProgressBar::suspend(\|\| ...)` |
| `Stat() & ModeCharDevice` TTY check | automatic — `ProgressDrawTarget::term` tests `!term.is_term()` |
| `\033[2K\r` erase-line escape | handled internally |
| *(not handled)* | `TERM=dumb`, which the same check covers |

So `stderr.go` largely disappears: instead of a mutex-wrapped writer tracking bar
state, the `tracing` writer wraps its emit in `pb.suspend(...)`.

**What indicatif does not give us** is go-udap's 500 ms start delay, which keeps
fast operations from flashing a bar on and off. Implement it by constructing the
bar with `ProgressDrawTarget::hidden()` and swapping to
`ProgressDrawTarget::stderr()` once the delay elapses. Port
`cli/progress_test.go`'s coverage of that behaviour.

### Socket construction

`socket2` and `tokio` have to be joined explicitly — this is the one seam where
ADR-1 costs something. `tokio::net::UdpSocket` cannot set the options we need,
and `socket2::Socket` is blocking. Build with one, hand off to the other:

```rust
let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
sock.set_reuse_address(true)?;
sock.set_reuse_port(true)?;                        // must precede bind
sock.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;
sock.set_broadcast(true)?;
if let Some(idx) = iface_index {
    sock.bind_device_by_index_v4(Some(idx))?;      // IP_BOUND_IF / SO_BINDTOIFINDEX
}
sock.set_nonblocking(true)?;                       // REQUIRED before from_std
let sock = tokio::net::UdpSocket::from_std(sock.into())?;
```

`set_nonblocking(true)` is load-bearing: `from_std` on a blocking socket
compiles and then stalls the runtime at the first read. This is the direct
analogue of go-udap's documented `SyscallConn().Control()`-not-`File()` hazard —
same class of mistake, different mechanism, equally silent. Assert it in a test.

## Milestones

**Walking-skeleton shape.** M2 builds the thinnest possible end-to-end slice —
one operation, all the way through — so there is a running binary early and every
later milestone thickens something that already works. Each milestone ends with
its Go tests ported and passing.

`mocksbr` cannot come first: it imports 16 `udap` symbols (`ParsePacket`,
`Packet`, `TLVData`, `Parameters`, every method constant) because it is the
responder side of the same protocol. Building it before the protocol core would
mean building the protocol core anyway, with less test coverage while doing it.
Hence the protocol module leads, but only just.

**M0 — foundations**
`mise.toml`, Cargo workspace, the three crate skeletons, `[lints.clippy]`,
CI running `fmt` + `clippy -D warnings` + `nextest`. *Done when:* an empty
workspace builds clean and CI is green.

**M1 — protocol core** (`mac`, `protocol`, `tlv`, `parameters`, `getdata`)
No I/O, no async. Where the Rust idioms get learned before anything harder.
The Go test files port almost line-for-line. *Done when:* `cargo test -p udap`
passes, and encoding a known packet produces bytes identical to the committed
captures in `mocksbr/testdata/captures/`.

**M2 — walking skeleton: `discover` end to end**
The thinnest vertical slice that runs. Discovery request encode; a `mocksbr`
that answers `adv_disc` (0x0009) and nothing else; the in-process
`MockTransport`; a `Transport` trait; a clap `discover` subcommand printing
MACs. Introduces async, since the transport trait is async from the start.
*Done when:* `udapcfg discover` against an in-process mock prints the same MAC
list `go-udap discover` does. **This is the first milestone with a working
binary.**

**M3 — real transport** (`transport/udp`, `interfaces`, Windows `#[cfg]` arm)
Swap the mock for a real socket: `socket2` construction, the tokio hand-off,
`--bind-interface`, `--all-interfaces` via `MultiTransport`. *Done when:*
`udapcfg discover` finds a real device on real hardware, on macOS and Linux.

**M4 — remaining operations** (`client`, `ops/*`, `validation`, `netconfig`)
`get_data`, `set_data`, `reset`, `get_ip`, `get_uuid`, plus the read-modify-write
in `set`. Contains the ADR-3 ownership work. *Done when:* all six UCP operations
round-trip against `mocksbr`.

**M5 — mocksbr complete**
All handlers, the fault-injection knobs, the standalone binary. Unblocks the
full e2e suite. *Done when:* go-udap's `mocksbr` integration tests pass in Rust.

**M6 — CLI complete**
Remaining subcommands, output formatting, the progress bar and stderr sync, INI
source layering, man pages, completions. *Done when:* every go-udap e2e scenario
passes with identical stdout/stderr/exit code, modulo program name.

**M7 — release plumbing**
Cross-compilation, packaging, SBOMs. See [Open questions](#open-questions).

**Stopping points.** M2 is the first — a running binary that does one real
thing. M4 is the second and more meaningful: at that point `udap` is a complete,
standalone Rust UDAP library, whatever happens to the CLI.

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

The toolchain is managed by [mise](https://mise.jdx.dev) and pinned in
`mise.toml`. Versions are exact, never ranges — refresh deliberately with
`mise outdated` / `mise upgrade` and commit the result.

| Tool | Pinned | Purpose |
| --- | --- | --- |
| `rust` | 1.98.1 | Toolchain, with `rustfmt` + `clippy` components |
| `cargo-deny` | 0.20.2 | Advisories, licences, bans |
| `cargo-audit` | 0.22.2 | RustSec advisory scan |
| `cargo-nextest` | 0.9.143 | Test runner |
| `cargo-mutants` | 27.1.0 | Mutation testing, from M1 |

No `rust-toolchain.toml`: rustup would read it and shadow mise's pin, giving two
sources of truth for the same thing. `mise.toml` also sets
`RUSTFLAGS = "-D warnings"` so the zero-warnings policy holds without repeating
the flag at every call site.

Gate, per the project standard:

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run
cargo deny check          # advisories, licences, bans
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

**OQ-1 — interface enumeration. RESOLVED: `netdev`.**
`EnumerateInterfaces` needs five things per interface — name, index, first IPv4
address, netmask, and three flags (Up, Broadcast, Loopback). The flags are the
discriminator: the Broadcast test is what keeps discovery from fanning out
across WireGuard and Tailscale tunnels, which do not carry `IFF_BROADCAST`.

| Crate | index | netmask | Up | Broadcast | Loopback |
| --- | --- | --- | --- | --- | --- |
| **`netdev` 0.46.2** | `u32` | prefix len | `is_up()` → `IFF_UP` | `is_broadcast()` → `IFF_BROADCAST` | `is_loopback()` → `IFF_LOOPBACK` |
| `if-addrs` 0.15.0 | `Option<u32>` | yes | `oper_status` (RFC 2863, not `IFF_UP`) | not exposed | from the *IP*, not the flag |

`netdev` is the only candidate exposing all three real `IFF_*` bits, which is
exactly what `iface.Flags & net.FlagBroadcast` tests. `if-addrs` would force the
broadcast filter to be inferred from whether a broadcast address happened to get
populated — an undocumented internal detail to hang the VPN filter on. `netdev`
is also the most actively maintained candidate (released 2026-09-04).

Take it as `netdev = { version = "0.46", default-features = false }`. The
default features pull in `gateway` detection and
`apple-system-configuration-extra`, which drags Objective-C bindings
(`objc2-system-configuration`) onto macOS. Disabled, the tree is `mac-addr` +
`ipnet` + `libc`, plus netlink crates on Linux only.

*Verify at M3:* that `flags` is still populated with default features off.
Flags come from `getifaddrs`, so it should be, but confirm rather than assume.

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
