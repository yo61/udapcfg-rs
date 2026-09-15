# Handover

Current state of `udapcfg-rs`, for whoever picks it up next.

**Last updated:** 2026-09-15, with `main` at `9752a25`
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
| `main` | `9752a25` — M5-B2 merged |
| Tests | 241 passing |
| Clippy | silent |
| `cargo fmt --all --check` | clean |
| `cargo deny check` | advisories, bans, licences, sources all ok |
| Open PRs | none, once [#33](https://github.com/yo61/udapcfg-rs/pull/33) — this document — lands |
| Open issues | [#29](https://github.com/yo61/udapcfg-rs/issues/29) |

## Start here: what just landed

**M5-B2, the mocksbr `Slow` knob, merged as `9752a25`
([PR #34](https://github.com/yo61/udapcfg-rs/pull/34)).** It was built
task-by-task from
[`docs/plans/2026-09-14-m5b2-slow-knob.md`](plans/2026-09-14-m5b2-slow-knob.md),
argued from [`docs/specs/2026-09-14-m5b2-slow-design.md`](specs/2026-09-14-m5b2-slow-design.md).

The four branch commits were squashed into `9752a25`, so they exist only in the
PR now. What each contributed, since the split is the useful part:

| Commit | What |
| --- | --- |
| `fbf955b` | `ScheduledReply` + `DeviceConfig.slow`; `Network::receive` reports a delay, nothing waits yet |
| `5136489` | `MockTransport::schedule` — the sync/spawn split; mocksbr's first `[dev-dependencies]` |
| `3d0e3bc` | T1/T2/T4 — client round trip, deadline, error path |
| `4374335` | T3/T6 — `slow` interacting with `unreachable`, and per-device fan-out timing |

231 → 241 tests. The gate is clean on all four counts in the State table.

Six new timing tests carry roughly 660 ms of virtual delay between them and
cost 0.00 s of wall clock; both suites still report `finished in 0.00s`. If
that ever changes, a test has picked up a real clock — most likely
`std::time::Instant` where it wants `tokio::time::Instant`, or a missing
`start_paused`.

That holds on CI hardware too, not just locally. Every timing test lands in
0.02–0.04 s on the slowest target (aarch64 Windows), and that residue is
nextest's per-process overhead rather than any of the nominal delay:
`a_deadline_shorter_than_the_delay_times_out` nominally waits 200 ms and ran in
0.039 s. All six targets reported `241 tests run: 241 passed, 0 skipped`, read
from the runner output rather than inferred from a green tick — the distinction
[`quality/criteria.md`](../quality/criteria.md) insists on, since
`--no-tests=pass` makes an empty run look like success.

### What the mutation checks established

Every mutation in the spec's table, each run **per test with `--exact`** and
100 runs, never suite-level:

| Mutation | Caught 100/100 by | Correctly stays green for |
| --- | --- | --- |
| `schedule` always synchronous | T1, T2, T4, T6, and both transport tests | T5, T3 |
| `schedule` always spawns | T5 | the slow-path tests |
| error reply loses its delay | T4 | T1 |
| delay halved at both sites | T1, T4 | T2 |
| one shared delay per batch | T6 | every single-device timing test |
| `unreachable` filter deleted | T3 and both pre-existing unreachable tests | — |

The right-hand column is the part that matters. It shows each test is the
*only* one that can distinguish its mutation, which is exactly what the spec
claimed when it argued T4 and T6 into existence. A mutation table where two
rows name the same test means one of those tests is redundant.

### The review that unblocked the push

`lastlight-review-run.sh` reviewed `4374335` against `e7f1bd3` on 2026-09-15:
**APPROVE, zero findings**, sonnet, sandboxed with containment verified and
probes enabled. Recorded against lastlight-core 0.29.0 and pr-review skill
7.4.0.

It was a real pass, not a rubber stamp — it traced every consumer of the changed
`Network::receive`/`ScheduledReply` contract across all three crates looking for
stale tuple destructuring, and checked the new tests' `timeout`/`sleep` ordering
assumptions against tokio's poll semantics.

Worth knowing for next time: the reviewer's `Write` tool was denied three times
and it fell back to a Bash heredoc. That did not affect the verdict, but it is
noted in `.lastlight/pr-review/reviewer.log` and the permission settings may be
worth a look.

### Two findings from executing the plan

Both are now in [Gotchas](#gotchas-that-cost-real-time); flagged here because
they are corrections to a plan that had already been reviewed and merged.

1. **The plan's test code did not pass the project's own clippy gate.**
   `clippy::doc_markdown` rejected `TestSlowDeviceReplyDelayedByConfiguredDuration`,
   `TestSlowDeviceTimesOutWhenDeadlineShorter` and `CancellationToken` appearing
   unbackticked in `///` doc comments. Prose in `//` body comments is fine,
   which is why the file's existing Go test-name citations never tripped it.
2. **Task 4 step 4's verification was weaker than it read.** As written it ran
   `cargo test --workspace` 100 times and counted non-zero exits. Cargo stops at
   the first failing test *target*, so `scheduling.rs` failing meant
   `ops_faults` never ran at all — the 100/100 proved something failed, not that
   the four named tests did. Re-run per test with `--exact`.

## Milestones

| | Milestone | State |
| --- | --- | --- |
| M0–M2 | walking skeleton | done |
| M3 | real UDP transport | done except [step 7](#still-open) |
| M4 | the remaining UCP operations | done |
| M5-A | stateful mocksbr | done |
| M5-B1 | synchronous fault-injection knobs | done ([#28](https://github.com/yo61/udapcfg-rs/pull/28)) |
| M5-B2 | the `Slow` knob | done ([#34](https://github.com/yo61/udapcfg-rs/pull/34)) |
| M5-C | standalone mocksbr binary | next — needs its own design pass |
| M6 | `RebootDelay`, `DropGetData`, `SuppressDiscoveryUUID` | deferred |

M5-C is the natural next milestone. `ScheduledReply` is what its UDP server
will need, so M5-B2 unblocks it without doing any of it. Design pass first —
spec, then plan, then implement, same as M5-B1 and M5-B2.

M6's three knobs have **zero uses** in go-udap's own mocksbr tests — their doc
comments say they exist for the CLI's tests — so they are low value until the
CLI suite needs them. Do not port them just for completeness.

## Still open

- **[#29](https://github.com/yo61/udapcfg-rs/issues/29) — `MockTransport::InjectReply` unported.**
  Replacing `crates/udap/src/session.rs:136`'s source-mismatch guard with
  `if false` leaves the workspace green, so that branch is entirely untested. It
  is live in production: discovery populates `Device.ip`, and every directed
  reply afterwards goes through it. `InjectReply`
  (`go-udap mocksbr/transport.go:70`) is the missing tool, because `Network`
  always reports the device's own MAC as the source. Labelled `ready-for-agent`.
  Note the test count in that issue predates M5-B2; it is 241 now.
- **`MockTransport` after `close` diverges from go-udap, and it is not in the
  accepted-deltas table.** Verified against `43864a5`, not inferred. Go's
  `Close` (`mocksbr/transport.go:116`) sets `closed` *and* nils `pending`, its
  `enqueue` drops anything arriving afterwards, and its `Recv` tests `t.closed`
  **before** looking at the queue — so after `Close` it returns
  `context.Canceled` every time. The Rust `close` only cancels the `closed`
  token: queued replies stay in the channel, and `recv`'s unbiased
  `tokio::select!` then picks at random between `TransportError::Cancelled` and
  delivering one. Go's ordered check is exactly the `biased;` the Rust lacks.

  Pre-existing — the token and the `select!` both predate M5-B2, so this is not
  a [#34](https://github.com/yo61/udapcfg-rs/pull/34) regression, and the
  independent review did not raise it because it sits outside that diff. But
  `slow` adds a second way for a reply to be in flight at `close` time, which is
  the case Go's `enqueue` guard exists to handle. Decide it before M5-C, whose
  UDP server will have real sockets to close: either add `biased;` and drain the
  channel, or write it into the accepted-deltas table. Per the spec's own
  header, anything not in that table is a bug.

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
mise exec -- cargo deny check
```

`mise.toml` pins the toolchain exactly (Rust 1.98.1) and sets
`RUSTFLAGS = "-D warnings"`, so **every warning is a hard error** — including
unused imports, which is what makes some mutations fail to compile rather than
fail their tests. There is deliberately no `rust-toolchain.toml`; rustup would
shadow mise's pin and give two sources of truth.

`clippy.toml` exempts `unwrap`/`expect` inside tests. Everywhere else they are
denied, along with `panic`, `print_stdout` and `allow_attributes` (use
`#[expect(..., reason = "...")]`, which self-removes when it stops applying).

### Git

- **Never commit on `main`.** Branch first. Never push to `main`/`master`.
- **`git checkout -b <name> origin/main` sets the new branch's upstream to
  `origin/main`**, so a bare `git push` would target main. Run
  `git branch --unset-upstream` straight after.
- Conventional Commits, imperative mood, ≤72-char subject.
- Every push needs a **local Last Light review recorded for that exact SHA**.
  Write `.lastlight/pr-review/findings.json`, then
  `~/.claude/hooks/lastlight-review-record.sh <sha>` (not on `PATH`). Any new
  commit invalidates it, so batch fixes into one push. The gate itself is
  `~/.claude/hooks/lastlight-review-gate.sh`, a `PreToolUse` hook.

### Testing

Read [`quality/criteria.md`](../quality/criteria.md) before calling anything done.
The rules that bite most often:

- **Break the branch and watch the test fail** before claiming it covers
  anything.
- **A scheduling-dependent test must catch its mutant across repeated runs, not
  once.** One observed failure cannot distinguish a reliable test from a coin
  flip. This is not theoretical — see below.
- **Assert non-events with virtual time** (`#[tokio::test(start_paused = true)]`
  plus a timeout), never a pre-cancelled token or a real sleep.

A reusable mutation harness is worth keeping to hand. The shape that works:
build the one test binary with `--no-run --message-format=json`, pull the
`executable` path out of the `compiler-artifact` line whose `target.name`
matches, then loop `"$BIN" --exact <test-name>` and count non-zero exits.
Per test, not per suite, for the reason in the gotchas below.

## Gotchas that cost real time

These are all learned the hard way. None is discoverable from the code.

**Mutation edits silently not applying.** rustfmt reflows code, so a pattern you
copied from memory may not match. A "survived" result in 0.00s almost always
means the edit never landed. Always confirm with `rg` before believing a
mutation result. The same reflow can happen to a test *after* you have
mutation-checked it — `cargo fmt` rewrote the fan-out test's `let` bindings at
the end of M5-B2, so its mutation check was re-run against the formatted
version.

**A mutant that fails to compile scores a fake 100/100.** Any harness counting
non-zero exits cannot tell a build failure from a caught mutation. Under
`RUSTFLAGS=-D warnings` this is easy to trigger: deleting the spawn branch from
`MockTransport::schedule` leaves `use std::time::Duration` unused, which is a
hard error. Always build the mutant once and confirm it compiles before
trusting a loop.

**`cargo test` stops at the first failing test target.** A suite-level mutation
run tells you *a* target failed, and every target after it never ran. During
M5-B2 the mocksbr suite failing meant `ops_faults` was never executed, so a
100/100 suite-level score said nothing about the four tests it was supposed to
be checking. Use `--no-fail-fast` to see the full failure list, and `--exact`
per test to get a number you can believe.

**Suite-level mutation results are confounded even when everything runs.**
Measuring a mutant against the whole suite tells you *something* failed, not
that *your* test did. A sibling test that fails deterministically will mask a
racy one completely.

**`clippy::doc_markdown` applies to `///` but not `//`.** Bare identifiers in a
doc comment — Go test names, type names like `CancellationToken` — are errors
under `-D warnings`. The same text in a body comment is fine. This is why
plan documents that quote Go test names in doc comments do not compile as
written.

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
| `decisions/` | decision log, one file per decision |

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
