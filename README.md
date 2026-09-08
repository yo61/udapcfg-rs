# udap-rs

A Rust port of [go-udap](https://github.com/yo61/go-udap) — a command-line tool
for discovering and configuring Squeezebox devices over UDAP (Universal Device
Access Protocol) on UDP port 17784.

## Status

**Specification only. No code yet.**

This is a learning project. go-udap remains the maintained tool; udap-rs is a
faithful port of it — same CLI surface, same wire bytes, same exit codes —
undertaken to learn Rust on a codebase whose behaviour is already pinned down by
a large test suite.

## Documents

- [Port specification](docs/specs/2026-09-08-rust-port-spec.md) — goal, scope,
  architecture decisions, milestones, open questions
- [Port map](docs/port-map.md) — module-by-module Go → Rust translation reference

## Planned layout

```
crates/udap/       protocol, transport, client
crates/mocksbr/    fake Squeezebox Receiver for testing
crates/udap-cli/   the `go-udap` binary
xtask/             man pages, completions, release helpers
```

## Prerequisites

No Rust toolchain is installed yet. Install via [rustup](https://rustup.rs)
before starting M1.

## Licence

MIT — see [LICENSE](LICENSE).
