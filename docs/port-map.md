# Go → Rust port map

A working reference for translating [`yo61/go-udap`](https://github.com/yo61/go-udap)
into Rust. Read the [spec](specs/2026-09-08-rust-port-spec.md) first for what
we're building and why; this document is the module-by-module detail you keep
open while doing it.

**Source of truth:** go-udap `v2.4.8` (`43864a5`). When the Go and this document
disagree, the Go wins — re-read it and fix this file.

## Crate layout

The Go package graph is a clean DAG — `udap` ← `mocksbr` ← `cli` — so it maps
one-to-one onto a Cargo workspace.

| Go package | Rust crate | Kind |
| --- | --- | --- |
| `udap/` | `udap` | lib |
| `mocksbr/` + `cmd/mocksbr/` | `mocksbr` | lib + bin |
| `cli/` + `main.go` | `udap-cli` | bin (`go-udap`) |
| `cmd/docs/` | `xtask` | bin (dev-only) |

One thing Cargo permits that Go does not: a dev-dependency cycle. `udap`'s tests
may depend on `mocksbr`, which depends on `udap`. In Go that's an import cycle,
which is why every mock-driven test today lives in `mocksbr` or `cli` even when
it is really testing `udap`. Expect to move some tests inward.

## Module mapping

### `udap` crate

| Go file | Rust module | Notes |
| --- | --- | --- |
| `mac.go` | `mac.rs` | Newtype `Mac([u8; 6])`, `FromStr`, `Display`. Nearly mechanical. `Copy + Eq + Hash` makes it a free `HashMap` key — see [Aliasing](#aliasing-and-ownership). |
| `protocol.go` | `protocol.rs`, `tlv.rs` | Split TLV codec out. `binary.Read` reflection → hand-rolled `Packet::{to_bytes, from_bytes}`. See [Packet](#packet-encodedecode). |
| `parameters.go` | `parameters.rs` | `const PARAMETERS: [Parameter; 26]`. Both lazy index maps disappear. |
| `getdata_response.go` | `getdata.rs` | Bounds-safe decode. Keep the `count` allocation clamp. |
| `getip.go`, `getuuid.go` | `ops/getip.rs`, `ops/getuuid.rs` | Small TLV decoders. Mechanical. |
| `netconfig.go` | `netconfig.rs` | `NetworkConfig { ip: Option<Ipv4Addr>, .. }` — `Option` replaces the "zero value means absent" convention. |
| `interfaces.go` | `interfaces.rs` | Needs an interface enumerator; see [Open question 1](specs/2026-09-08-rust-port-spec.md#open-questions). |
| `validation.go` | `validation.rs` | Per-parameter input rules. Mechanical. |
| `loopback.go` | `protocol.rs` | `is_udap_request_packet` — three lines, fold it in. |
| `logger.go` | *(deleted)* | Replaced by `tracing`. See [Logging](#logging). |
| `transport.go` | `transport/udp.rs` | `socket2`. **Gets simpler** — see [Sockets](#sockets). |
| `multi_transport.go` | `transport/multi.rs` | Threads + `mpsc`. The hardest concurrency piece. |
| `socket_unix.go` | `transport/udp.rs` | Absorbed — `set_broadcast` / `set_reuse_address` are plain methods. |
| `socket_darwin.go`, `socket_linux.go` | `transport/udp.rs` | **Both absorbed into one call.** See [Sockets](#sockets). |
| `socket_windows.go` | `transport/udp.rs` | One `#[cfg(windows)]` arm returning `Error::InterfaceBindUnsupported`. |
| `client.go` | `client.rs` | **The ownership fight.** See [Aliasing](#aliasing-and-ownership). |
| `config.go` | `ops/config.rs` | `get`/`set`/`reset`. `context.Context` → `Deadline`. |
| `discovery.go` | `ops/discovery.rs` | Broadcast + collect-until-deadline. |

### `udap-cli` crate

| Go file | Rust module | Notes |
| --- | --- | --- |
| `cli.go` | `main.rs`, `cli.rs` | cobra → clap. Package-level flag globals → a parsed `Cli` struct. |
| `params.go` | `cli.rs` | The three `*WithPlaceholder` types delete entirely — `value_name` is one clap attribute. The runtime-derived 26 flags need clap's *builder* API. |
| `{discover,info,read,get,set,reboot,getip,interfaces}.go` | `cmd/*.rs` | One module per subcommand, as today. |
| `find.go` | `cmd/find.rs` | Discover-then-match-by-MAC helper. |
| `source.go`, `config.go` | `source.rs`, `ini.rs` | INI parse + the file/stdin/flag layering. Mechanical. |
| `output.go` | `output.rs` | Write to `&mut dyn Write`. Good `insta` snapshot targets. |
| `progress.go`, `stderr.go` | `progress.rs` | Thread + `Arc<Mutex<StderrSync>>`. `std::io::IsTerminal` replaces the `Stat()` TTY check — no dependency. |
| `completion.go` | `xtask` | `clap_complete` generates from the same `Command`. |
| `deverr.go` | `error.rs` | `ExitError{Code, Err}` → enum with `exit_code()`. |
| `uuidfallback.go`, `set_interface_default.go` | `cmd/*.rs` | Small behavioural helpers; port with their tests. |

### `mocksbr` crate

| Go file | Rust module | Notes |
| --- | --- | --- |
| `device.go` | `device.rs` | `DeviceConfig` has ~10 test-knob booleans (`DropGetData`, `SuppressDiscoveryUUID`, …). Consider `Vec<Fault>` instead — but that is a design change, so not in the faithful port. |
| `responses.go` | `responses.rs` | Largest single file (384 lines). Response builders per method. |
| `handlers.go` | `handlers.rs` | Method dispatch — becomes a `match` on a `Method` enum. |
| `network.go`, `identity.go` | `network.rs`, `identity.rs` | Mechanical. |
| `transport.go` | `transport.rs` | `MockTransport` — implements the same `Transport` trait. |
| `testhelper/spawn.go` | `testhelper.rs` | Spawns the mock binary for out-of-process tests. |
| `cmd/mocksbr/{main,flags}.go` | `src/bin/mocksbr.rs` | Standalone fake device. |

---

## Idiom translation

### Packet encode/decode

Go declares `Packet` as a struct of fixed-size fields and lets
`binary.Read(r, binary.BigEndian, &packet)` fill it by reflection. It works
because every field is fixed-width with no padding — an invariant the compiler
does not check and a comment has to assert (`UDAPHeaderSize = 27 // sum of
fields, no padding`).

Rust has no reflection-based equivalent worth taking a dependency for. Write it
out:

```rust
pub const HEADER_SIZE: usize = 27;

impl Packet {
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] { /* explicit field writes */ }

    pub fn from_bytes(buf: &[u8]) -> Result<(Packet, &[u8]), ProtocolError> {
        // returns the header and the remaining payload slice
    }
}
```

Roughly 40 lines, faster than the reflective path, and `HEADER_SIZE` becomes
checkable rather than asserted. Do **not** reach for `deku`/`binrw`/`zerocopy`
here — one struct does not justify a proc-macro dependency.

`from_bytes` returning a borrowed payload slice removes the `data []byte` copy
that `ParsePacket` returns today.

### The parameter table

```rust
pub struct Parameter {
    pub name: &'static str,
    pub offset: u16,
    pub length: u16,
    pub placeholder: &'static str,
    pub help: &'static str,
    pub factory_default: &'static str,
}

pub const PARAMETERS: [Parameter; 26] = [ /* ... */ ];
```

Fully const. Both Go lazy-init maps (`parameterIndex`, `configParamByOffset`)
become linear scans — at 26 entries a scan beats a hash, and it removes two
pieces of global mutable state.

`Parameter::encode` switches on `length` (1/2/4/other). Give that a name:

```rust
enum Encoding { U8, U16, Ipv4, Text(u16) }
```

The Go comment about the string-truncation branch being "unreachable from the
CLI but preserved for library callers" then becomes expressible in the type.

### Aliasing and ownership

`client.go` documents the same contract in three separate comments:

> the returned `*Device` aliases the client's internal entry; callers that
> mutate fields on a returned device update the client's view

and `GetAllDeviceConfigWithContext(ctx, device)` writes through that alias while
`devicesMu` may be held elsewhere. Rust will not compile this. That is the
borrow checker reading those three comments and objecting.

Target shape:

- `Client` owns `devices: HashMap<Mac, Device>` (keyed by `Mac`, not by
  `Mac::to_string()` — the Go compromise in `recordDevice` disappears).
- Discovery takes `&mut self` and populates the map.
- Device operations take `&self` for the transport and `&mut Device` for the
  target, with the caller holding the `Device` — obtained by `remove`ing it from
  the map or by cloning.

`Device` is small and cheap to clone; do not reach for `Arc<Mutex<Device>>`
before trying ownership. **Do this module last** — it is the one place where
"port it faithfully" and "make it compile" genuinely pull apart, and it is much
easier once the pure modules have taught you the idioms.

### Concurrency and cancellation

`context.Context` has no std equivalent, and the CLI never actually cancels — it
only ever deadlines. So:

```rust
#[derive(Copy, Clone)]
pub struct Deadline(Instant);
```

threaded where `ctx context.Context` is today. The 200 ms polling read-deadline
loop in `UDPTransport.Recv` (which exists purely to re-check `ctx`) collapses
into a single `socket.set_read_timeout(remaining)`.

`MultiTransport` is the exception — it genuinely needs concurrency. Port it as
one `std::thread` per child feeding a `std::sync::mpsc::Sender`, with
`recv_timeout` on the merge side. `sync.Once` → `std::sync::Once` or `OnceLock`;
`WaitGroup` → collecting `JoinHandle`s. No async runtime; see
[ADR-1](specs/2026-09-08-rust-port-spec.md#adr-1-threads-not-async).

### Sockets

The single biggest simplification in the port.

Go must use `net.ListenConfig{Control: ...}` to reach the fd *before* bind, and
carries a documented landmine:

> this must use `SyscallConn().Control()`, NOT `(*UDPConn).File()`. `File()`
> switches the socket to blocking mode, which on macOS prevents `Close()` from
> interrupting any pending `recvfrom`

With `socket2` you own the fd from creation, so pre-bind versus post-bind is
just statement order, and the `File()` hazard cannot occur:

```rust
let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
sock.set_reuse_address(true)?;
sock.set_reuse_port(true)?;                      // before bind
sock.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;
sock.set_broadcast(true)?;                       // after bind
if let Some(idx) = iface_index {
    sock.bind_device_by_index_v4(Some(idx))?;    // macOS AND Linux
}
```

`bind_device_by_index_v4` is gated on `any(target_os = "macos", "ios", "linux",
"android", "illumos", "solaris", …)` and dispatches internally to `IP_BOUND_IF`
on Apple and `SO_BINDTOIFINDEX` on Linux. **`socket_darwin.go` and
`socket_linux.go` collapse into that one line** — the platform split that
dominates the Go socket layer largely disappears, leaving only a
`#[cfg(windows)]` arm for the unsupported case.

Requires `socket2 = { version = "0.6", features = ["all"] }` — the method is
behind the `all` feature gate.

**Accepted behavioural delta:** Linux moves from `SO_BINDTODEVICE` (by name, old
kernels) to `SO_BINDTOIFINDEX` (by index, **kernel 5.7+**). Recorded in the spec
under [Accepted deltas](specs/2026-09-08-rust-port-spec.md#accepted-behavioural-deltas).

### Errors

| Go | Rust |
| --- | --- |
| `fmt.Errorf("...: %w", err)` in `udap` | `thiserror` enum per module, `#[from]` for wrapping |
| `fmt.Errorf` in `cli` | `anyhow::Result` + `.context(...)` |
| `ExitError{Code, Err}` | enum variant + `fn exit_code(&self) -> i32` |
| `errors.AsType[*ExitError](err)` | `err.downcast_ref::<CliError>()` or a plain `match` |
| `errors.Is(err, context.DeadlineExceeded)` | `matches!(e, Error::Timeout { .. })` |

The exit-code contract (0 success / 1 usage / 2 operation failure) is part of
the [fidelity contract](specs/2026-09-08-rust-port-spec.md#fidelity-contract) —
snapshot-test it.

### Logging

`udap/logger.go` defines a `Logger` interface over an `io.Writer` so the CLI can
route it through `stderrSync`. In Rust that is `tracing` plus a
`tracing_subscriber` `fmt` layer whose `MakeWriter` returns the shared
`Arc<Mutex<StderrSync>>`. The custom logger type deletes; the
progress-bar-versus-log interleaving behaviour is preserved by the same
`StderrSync`.

### CLI flags

The 26 per-parameter `--flags` are derived at runtime from `PARAMETERS`, and
clap's `derive` macro is compile-time. Use `derive` for the fixed global flags
and the *builder* API for the generated ones, joined via `Args::augment_args`:

```rust
let mut cmd = SetArgs::augment_args(Command::new("set"));
for p in &udap::PARAMETERS {
    cmd = cmd.arg(Arg::new(p.name)
        .long(p.flag_name())
        .value_name(p.placeholder)
        .help(p.help));
}
```

Same volume of code as the Go, and it keeps `PARAMETERS` as the single source of
truth. `clap_mangen` and `clap_complete` then generate man pages and
completions off that same `Command`, matching cobra's behaviour today.

### Testing

| Go | Rust |
| --- | --- |
| `t.Run(name, func(t *testing.T))` table tests | `rstest` cases, or a plain loop |
| Golden CLI output compared as strings | `insta` snapshots (strictly better) |
| `prev := newClient; newClient = fake` global seam | Pass a client factory into `run()` — dependency injection |
| `t.Parallel()` opt-in | Parallel by default; env-touching tests need `serial_test` |
| `go test -race` | Ownership rules cover most of it; keep `--cfg` sanitizer runs in CI |

The package-global `newClient` seam in `cli/e2e_harness_test.go` is the one test
pattern that does not translate. Making `run()` take its dependencies removes
both the global and the "e2e tests must not be `t.Parallel`" constraint.

---

## What gets deleted

| Deleted | Why | ~Lines |
| --- | --- | --- |
| `socket_darwin.go` + `socket_linux.go` | One cross-platform `socket2` call | ~120 |
| `params.go`'s three `*WithPlaceholder` types | `value_name` attribute | ~100 |
| `logger.go` | `tracing` | ~135 |
| `parameterIndex`, `configParamByOffset` | Linear scan over a 26-entry const | ~20 |
| `Device` serde derives | JSON is test-only — see [ADR-4](specs/2026-09-08-rust-port-spec.md#adr-4-no-serde) | — |
| `ctx` polling loop in `Recv` | `set_read_timeout` | ~15 |

## What gets harder

| Harder | Why |
| --- | --- |
| `client.go` ownership | Three documented aliasing contracts become compile errors |
| Cross-compilation | `GOOS=windows go build` has no zero-setup Rust equivalent |
| Release packaging | `.goreleaser.yaml` does archives + cask + SBOM + man/completions in one file |
| First build | ~1 s → ~30 s clean |

## Line-count expectation

Go is 6,320 production lines and 6,258 test lines. Expect Rust production to
land in the same range or slightly above: explicit codecs add lines, while
deleted platform files, the logger, and the placeholder types remove them. Do
not treat a smaller number as a win — a faithful port that is much shorter has
probably dropped behaviour.
