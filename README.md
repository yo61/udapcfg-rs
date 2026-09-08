# udapcfg-rs

A Rust port of [go-udap](https://github.com/yo61/go-udap) — a command-line tool
for discovering and configuring Squeezebox devices over UDAP (Universal Device
Access Protocol) on UDP port 17784.

## Status

**Specification only. No code yet.**

This is a learning project. go-udap remains the maintained tool; udapcfg-rs is a
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
crates/udap-cli/   produces the `udapcfg` binary
xtask/             man pages, completions, release helpers
```

## Prerequisites

The toolchain is pinned in `mise.toml`. With [mise](https://mise.jdx.dev)
installed:

```sh
mise install
```

That provides Rust 1.98.1 (with rustfmt and clippy) plus `cargo-deny`,
`cargo-audit`, `cargo-nextest`, and `cargo-mutants`, all pinned exactly.

## Licence

MIT — see [LICENSE](LICENSE).
