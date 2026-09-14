# Handover

Current state of `udapcfg-rs`, for whoever picks it up next.

**Last updated:** 2026-09-14, at `e7f1bd3`
**This is a living document** — update it in place rather than adding dated
copies. It should always describe *now*.

## What this is

A fidelity port of [go-udap](https://github.com/yo61/go-udap) to Rust. It speaks
UDAP/UCP over UDP to Squeezebox Receiver hardware.

**Fidelity is the overriding contract.** Where this port and the Go disagree,
the Go wins — warts included. Deliberate divergences go in the spec's
[Accepted behavioural deltas](specs/2026-09-08-rust-port-spec.md#accepted-behavioural-deltas)
table, whose header says the quiet part out loud: *anything not listed there is
a bug*.

**Source of truth is v2.4.8, commit `43864a5`** — not go-udap's `main`, and not
whatever its working tree happens to be. Check citations with
`git show 43864a5:path/to/file.go`. A local checkout normally sits at
`~/code/github.com/yo61/go-udap`.

## State

| | |
| --- | --- |
| `main` | `e7f1bd3` |
| Tests | 231 passing |
| Clippy | silent |
| `cargo fmt --all --check` | clean |
| Open PRs | none |
| Open issues | [#29](https://github.com/yo61/udapcfg-rs/issues/29) |

## Milestones

| | Milestone | State |
| --- | --- | --- |
| M0–M2 | walking skeleton | done |
| M3 | real UDP transport | done except [step 7](#still-open) |
| M4 | the remaining UCP operations | done |
| M5-A | stateful mocksbr | done |
| M5-B1 | synchronous fault-injection knobs | done ([#28](https://github.com/yo61/udapcfg-rs/pull/28)) |
| **M5-B2** | **the `Slow` knob** | **spec + plan on main, not started** |
| M5-C | standalone mocksbr binary | needs its own design pass |
| M6 | `RebootDelay`, `DropGetData`, `SuppressDiscoveryUUID` | deferred |

### Start here

[`docs/plans/2026-09-14-m5b2-slow-knob.md`](plans/2026-09-14-m5b2-slow-knob.md),
argued from [`docs/specs/2026-09-14-m5b2-slow-design.md`](specs/2026-09-14-m5b2-slow-design.md).
Read both — the plan cites the spec rather than repeating it.

Four tasks: make the delay expressible, add the sync/spawn split in the
transport, then cover it through the client. Six tests, six mutations at 100
runs each. It is written to be executed task-by-task with a review between,
which is what surfaced both fidelity bugs in M5-B1.

M6's three knobs have **zero uses** in go-udap's own mocksbr tests — their doc
comments say they exist for the CLI's tests — so they are low value until the
CLI suite needs them. Do not port them just for completeness.

## Still open

- **[#29](https://github.com/yo61/udapcfg-rs/issues/29) — `MockTransport::InjectReply` unported.**
  Replacing `crates/udap/src/session.rs:136`'s source-mismatch guard with
  `if false` leaves the workspace 231/231 green, so that branch is entirely
  untested. It is live in production: discovery populates `Device.ip`, and every
  directed reply afterwards goes through it. `InjectReply`
  (`go-udap mocksbr/transport.go:70`) is the missing tool, because `Network`
  always reports the device's own MAC as the source. Labelled `ready-for-agent`.
- **M3 task 6 step 7** — `udapcfg` at runtime on Windows. CI builds both Windows
  targets, but nothing has exercised the binary against a device there. Needs a
  Windows machine on the Receiver's LAN. See
  [`docs/m3-task-6-hardware-checklist.md`](m3-task-6-hardware-checklist.md).
- **OQ-8** — needs a configured device with a non-ASCII SSID. Blocked on
  hardware state, not on code.
- **Upstream, awaiting Robin's decisions:** go-udap #225 (adopt the same
  interface sort order) and #226 (adaptive retry).

## How to work here

### Every command goes through mise

```bash
mise exec -- cargo test --workspace
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo fmt --all --check
```

`mise.toml` pins the toolchain exactly (Rust 1.98.1) and sets
`RUSTFLAGS = "-D warnings"`, so **every warning is a hard error**. There is
deliberately no `rust-toolchain.toml` — rustup would shadow mise's pin and give
two sources of truth.

`clippy.toml` exempts `unwrap`/`expect` inside tests. Everywhere else they are
denied, along with `panic`, `print_stdout` and `allow_attributes` (use
`#[expect(..., reason = "...")]`, which self-removes when it stops applying).

### Git

- **Never commit on `main`.** Branch first. Never push to `main`/`master`.
- Conventional Commits, imperative mood, ≤72-char subject.
- Every push needs a **local Last Light review recorded for that exact SHA**.
  Write `.lastlight/pr-review/findings.json`, then
  `lastlight-review-record.sh <sha>`. Any new commit invalidates it, so batch
  fixes into one push.

### Testing

Read [`quality/criteria.md`](../quality/criteria.md) before calling anything
done. The rules that bite most often:

- **Break the branch and watch the test fail** before claiming it covers
  anything.
- **A scheduling-dependent test must catch its mutant across repeated runs, not
  once.** One observed failure cannot distinguish a reliable test from a coin
  flip. This is not theoretical — see below.
- **Assert non-events with virtual time** (`#[tokio::test(start_paused = true)]`
  plus a timeout), never a pre-cancelled token or a real sleep.

## Gotchas that cost real time

These are all learned the hard way. None is discoverable from the code.

**Mutation edits silently not applying.** rustfmt reflows code, so a pattern you
copied from memory may not match. A "survived" result in 0.00s almost always
means the edit never landed. Always confirm with `rg` before believing a
mutation result.

**Suite-level mutation results are confounded.** Measuring a mutant against the
whole suite tells you *something* failed, not that *your* test did. A sibling
test that fails deterministically will mask a racy one completely. Always run
the single test with `--exact`.

**`MockTransport::recv` uses an unbiased `tokio::select!`.** When both a
cancellation and a queued reply are ready, the winner is random. Any test that
pre-cancels a token to stand in for a timeout is a coin flip. One such test
scored 210/300 against its mutant before this was found.

**A blocked hook kills the entire Bash call.** The Last Light gate is a
`PreToolUse` hook, so combining "record the review" and "push" into one command
means neither runs. Keep them in separate calls.

**The bash-guard hook false-positives on prose.** It rejects the word "which" in
commit messages, PR bodies and heredocs, and any `rg -r`. Use `--body-file`, or
write files with the Write tool instead of a heredoc.

**`rg -r` is `--replace`, not `--recursive`.** `rg -rn "x"` silently rewrites
every match to `n`. ripgrep recurses by default.

## Layout

| Path | What |
| --- | --- |
| `crates/udap` | the protocol library — packets, TLV, ops, transport, session |
| `crates/udap-cli` | the `udapcfg` binary |
| `crates/mocksbr` | in-process mock Receiver; the test double the suite runs against |
| `docs/specs/` | designs, approved before implementation |
| `docs/plans/` | task-by-task implementation plans, argued from a spec |
| `docs/port-map.md` | Go→Rust structural mapping |
| `quality/criteria.md` | the gate to evaluate against before calling work done |

CI builds six targets on native runners: `{x86_64,aarch64}` × Linux musl, macOS
and Windows MSVC. `rustup target add` is used **only** on the musl legs — every
other target builds for its own host, and adding a target on Windows hung for 30
minutes before that was restricted.

## Hardware

A real Squeezebox Receiver, MAC `00:04:20:16:17:18`, is reachable on the LAN for
live testing. `nas1` (TrueNAS Scale, on both `192.168.1.0/24` and
`192.168.20.0/24`) is available for Linux testing, with binaries kept in
`~/bin`; it has 10 interfaces, which is how the non-deterministic interface
ordering bug (#13) was found.

Measured on a lossy link: discovery succeeds 92.7% of the time with no retries,
99.3% with `--retries 2`, and 100% with `--retries 5` (450 runs). Loss is
independent per packet, not bursty.
