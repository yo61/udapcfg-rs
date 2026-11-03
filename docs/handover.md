# Handover

How `udapcfg-rs` works and where it is heading, for whoever picks it up next.

**This is a living document**, so update it in place. It records what stays
true across sessions: the contract, the milestones, open items with no other
home, and the gotchas. It deliberately leaves out anything a command can
answer, because a copy of live state is wrong by the next merge.

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

## Current state

Ask the tools rather than this file:

```bash
git log --oneline -10                      # what landed recently
gh pr list                                 # open PRs, Dependabot's included
gh issue list                              # open issues
gh pr checks <n>                           # why a PR is or is not mergeable
```

If work is paused waiting on something, such as a PR blocked behind another, say
so on the waiting PR or issue, not here. That is where the next person will
look, and it closes when the work does.

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

Each milestone since M5-B1 went spec, then plan, then implementation: designs in
`docs/specs/`, task-by-task plans in `docs/plans/`. M5-C should do the same.
`ScheduledReply`, from M5-B2, is what its UDP server will need. Settle
[#46](https://github.com/yo61/udapcfg-rs/issues/46), `MockTransport`'s
behaviour after `close`, before that design pass: the server will have real
sockets to close.

M6's three knobs have **zero uses** in go-udap's own mocksbr tests — their doc
comments say they exist for the CLI's tests — so they are low value until the
CLI suite needs them. Do not port them just for completeness.

## Still open

Tracked issues are in `gh issue list`. These are the items with no issue:

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

`mise.toml` pins the toolchain exactly and sets `RUSTFLAGS = "-D warnings"`, so
**every warning is a hard error** — including unused imports, which is what
makes some mutations fail to compile rather than fail their tests. There is
deliberately no `rust-toolchain.toml`; rustup would shadow mise's pin and give
two sources of truth.

`clippy.toml` exempts `unwrap`/`expect` inside tests. Everywhere else they are
denied, along with `panic`, `print_stdout` and `allow_attributes` (use
`#[expect(..., reason = "...")]`, which self-removes when it stops applying).

Workflows and hooks are linted by `jactionlint` (config in
`.github/jactionlint.yaml`), both in `prek` and in CI.

### Git

- **Never commit on `main`.** Branch first. Never push to `main`/`master`.
- **`git checkout -b <name> origin/main` sets the new branch's upstream to
  `origin/main`**, so a bare `git push` would target main. Run
  `git branch --unset-upstream` straight after.
- Conventional Commits, imperative mood, ≤72-char subject.
- `main` accepts squash merges only, and requires signed commits.
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
- **Read test counts from the runner output**, not from a green tick.
  `--no-tests=pass` makes an empty run look like success.

The timing tests use virtual time, so both suites report `finished in 0.00s`.
If that ever changes, a test has picked up a real clock — most likely
`std::time::Instant` where it wants `tokio::time::Instant`, or a missing
`start_paused`.

A reusable mutation harness is worth keeping to hand. The shape that works:
build the one test binary with `--no-run --message-format=json`, pull the
`executable` path out of the `compiler-artifact` line whose `target.name`
matches, then loop `"$BIN" --exact <test-name>` and count non-zero exits.
Per test, not per suite, for the reason in the gotchas below. A good mutation
table shows each test is the *only* one that catches its mutant; two rows
naming the same test means one of those tests is redundant.

## Gotchas that cost real time

These are all learned the hard way. None is discoverable from the code.

**Mutation edits silently not applying.** rustfmt reflows code, so a pattern you
copied from memory may not match. A "survived" result in 0.00s almost always
means the edit never landed. Always confirm with `rg` before believing a
mutation result. The same reflow can happen to a test *after* you have
mutation-checked it, so re-run the check after `cargo fmt`.

**A mutant that fails to compile scores a fake 100/100.** Any harness counting
non-zero exits cannot tell a build failure from a caught mutation. Under
`RUSTFLAGS=-D warnings` this is easy to trigger: deleting the spawn branch from
`MockTransport::schedule` leaves `use std::time::Duration` unused, which is a
hard error. Always build the mutant once and confirm it compiles before
trusting a loop.

**`cargo test` stops at the first failing test target.** A suite-level mutation
run tells you *a* target failed, and every target after it never ran. Use
`--no-fail-fast` to see the full failure list, and `--exact` per test to get a
number you can believe.

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
scored 210/300 against its mutant before this was found. Until
[#46](https://github.com/yo61/udapcfg-rs/issues/46) is resolved, this holds.

**A new RUSTSEC advisory fails `check` on every branch at once**, Dependabot's
included, because `cargo deny check` reads the advisory database live. No PR
can merge until a separate PR bumps the affected crate in `Cargo.lock`; land
that first, then rebase the rest.

**A Dependabot PR rebased by anyone but Dependabot can become unmergeable.**
#43's rebased head came out unsigned, with Robin as committer, and `main`
requires signatures, so GitHub reported it `BLOCKED` with every check green.
Local commits here are signed, so the likely route is GitHub's rebase-style
"Update branch", which does not sign. Leave the rebase to Dependabot
(`@dependabot rebase`, or `@dependabot recreate` once someone else has pushed
to the branch).

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
