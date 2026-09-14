# M5-B2: the mocksbr `Slow` knob — design

**Status:** approved 2026-09-14, awaiting implementation plan
**Source of truth:** go-udap v2.4.8 (`43864a5`)
**Predecessor:** M5-B1 ([#28](https://github.com/yo61/udapcfg-rs/pull/28))

## Goal

Port `DeviceConfig.Slow`, the last of mocksbr's fault-injection knobs and the
only one that needs a clock: a device that replies correctly but late. It is
what makes the client's deadline handling testable without hardware.

## Why it was deferred out of M5-B1

M5-B1 covered the knobs whose effect is visible in a single synchronous call to
`Network::receive`. `Slow` is not one of them — it changes *when* a reply
arrives, and `receive` has no way to express that today. Expressing it means a
new return type and a scheduling step in the transport, enough surface to
deserve its own pass.

## Background: how go-udap does it

Three pieces, all at `43864a5`:

`mocksbr/handlers.go:15` defines the carrier:

```go
type ScheduledReply struct {
	Bytes []byte
	Delay time.Duration
}
```

`ReceiveScheduled` (`handlers.go:39`) is the primitive that returns them.
`Receive` (`handlers.go:28`) is a thin wrapper that strips the delays off,
documented as *"ignoring any per-device Slow delay"*.

`mocksbr/transport.go:55` does the waiting:

```go
func (t *MockTransport) scheduleReply(reply ScheduledReply) {
	src := mockSourceMAC(reply.Bytes)
	if reply.Delay <= 0 {
		t.enqueue(pendingReply{bytes: reply.Bytes, src: src})
		return
	}
	time.AfterFunc(reply.Delay, func() {
		t.enqueue(pendingReply{bytes: reply.Bytes, src: src})
	})
}
```

Note the asymmetry: zero-delay enqueues **synchronously**, non-zero goes through
a timer. That is load-bearing, not incidental — see [Why zero stays
synchronous](#why-zero-stays-synchronous).

## Architecture

One new config field, one new type, one new transport helper.

```rust
// crates/mocksbr/src/device.rs
pub struct DeviceConfig {
    // ...
    /// Fault injection: how long this device takes to answer.
    ///
    /// Applies to every reply it produces, error replies included.
    /// `Duration::ZERO` (the default) replies immediately.
    pub slow: Duration,
}

// crates/mocksbr/src/network.rs
/// A reply plus how long the responding device would take to send it.
///
/// `Network` is synchronous and never waits: it *describes* the wire
/// timeline and leaves the waiting to whoever delivers the bytes.
pub struct ScheduledReply {
    pub bytes: Vec<u8>,
    pub src: String,
    pub delay: Duration,
}

impl Network {
    pub fn receive(&self, packet: &[u8]) -> Vec<ScheduledReply>;
}
```

The division of responsibility is the point: `Network` stays a pure function of
(packet, config, state) with no runtime dependency, so every dispatch decision
stays unit-testable without a reactor. All timing lives in `MockTransport`,
already the owner of the channel and the clock.

### Why one method, not two

go-udap exposes both `Receive` and `ReceiveScheduled`. This port collapses them.

`Receive` is documented as silently ignoring `Slow`. That is the same shape as
two bugs M5-B1 shipped and had to fix — `Op::Save` and `Op::Discover` both named
a knob that quietly did nothing — so a second entry point that quietly drops a
knob is a trap worth not building. Nothing in the workspace is published
(`Cargo.toml:13`, `publish = false`), so matching Go's *method names* buys
nothing a reader of both codebases cannot get from the port map, while the wire
behaviour — the actual fidelity contract — is identical either way.

Blast radius is three call sites: `crates/mocksbr/src/transport.rs:46` and two
in `crates/udap/tests/ops_faults.rs`.

## Where the delay attaches

Every reply a device produces carries that device's `slow`. go-udap sets it on
the discovery fan-out (`handlers.go:111`) and on both the error and success
paths of `dispatchUnicast` (`:142`, `:148`).

| Path | Delayed? | Why |
| --- | --- | --- |
| discovery reply | yes | `handlers.go:111` |
| `get` / `set` / `reset` / `get_ip` / `get_uuid` reply | yes | `handlers.go:148` |
| `fail_on` error reply | yes | `handlers.go:142` — a device that refuses slowly is still slow |
| `malformed` reply | yes | still a reply |
| `unreachable` | n/a | no reply exists to delay |
| `drop_get_ip` / `drop_get_uuid` | n/a | no reply exists to delay |
| `fail_on` containing `Op::Discover` | n/a | device is skipped from the fan-out |

## Data flow

```
MockTransport::send
  └─ Network::receive(packet) -> Vec<ScheduledReply>
       └─ for each reply: MockTransport::schedule(reply)
            ├─ delay == ZERO -> sender.send(...)                    (synchronous)
            └─ delay >  ZERO -> tokio::spawn(sleep(delay) -> send)  (deferred)
                                          |
                                    mpsc::unbounded
                                          |
                                   MockTransport::recv
```

### Why zero stays synchronous

Spawning unconditionally would let `send` return before an instant reply was
queued. Under `#[tokio::test(start_paused = true)]` the runtime auto-advances
the clock whenever it has no work left, so a not-yet-polled spawned task could
lose the race to a timer and reorder replies — silently, and for every existing
test, not just the new ones.

Measured on this workspace with a throwaway probe:

```
PROBE first=immediate @0ns; second=slow @750ms
wall-clock for the 750ms-delay test: 0s
```

A synchronously-enqueued reply is observable at exactly `0ns` of virtual time,
and a spawned 750 ms sleeper at exactly `750ms`, the whole test finishing in
0.00 s. Both halves of the split behave as required.

## Error handling

No new error variants, and no new failure modes.

The spawned task takes a clone of the existing `mpsc::UnboundedSender` and uses
the same `let _ = sender.send(...)` as the synchronous path. An unbounded send
fails only once the receiver is dropped, impossible while the transport lives;
after the transport is dropped, discarding a late reply is the correct outcome
rather than an error.

A spawned task outliving its test is harmless: dropping the runtime drops the
task.

## Testing

### Strategy: virtual time

All timing tests use `#[tokio::test(start_paused = true)]` with
`tokio::time::timeout`, not real sleeps.

go-udap cannot do this, and its tests show the cost
(`failure_injection_test.go:96-101`): an 80 ms delay asserted as
`elapsed >= 80ms && elapsed <= 280ms`, a 200 ms skew tolerance to absorb
scheduler noise. Virtual time replaces that bracket with an exact equality, runs
in 0.00 s instead of ~120 ms, and cannot flake under CI load.

This is also what `quality/criteria.md` now requires, as of
[#30](https://github.com/yo61/udapcfg-rs/pull/30): *"To assert that something
does not happen, prefer virtual time … over a pre-cancelled token or a real
sleep."*

### The tests

Ports of go-udap's two, plus four it does not have. All in
`crates/udap/tests/ops_faults.rs` except T5, covering mocksbr's own scheduling
and so belonging in `crates/mocksbr/tests/`.

| # | Test | Asserts | go-udap counterpart |
| --- | --- | --- | --- |
| T1 | reply is delayed by exactly the configured duration | elapsed `==` `slow` | `TestSlowDeviceReplyDelayedByConfiguredDuration` |
| T2 | a deadline shorter than `slow` times out | `timeout(...)` returns `Err` | `TestSlowDeviceTimesOutWhenDeadlineShorter` |
| T3 | `slow` + `unreachable` stays silent | no reply ever arrives | none |
| T4 | `slow` delays a `fail_on` error reply too | error arrives at `slow`, not at 0 | none (`handlers.go:142` untested in Go) |
| T5 | a zero-`slow` reply is queued before `send` returns | `timeout(ZERO, recv)` succeeds | none |
| T6 | discovery fan-out is ordered by delay, not config order | each reply arrives at *its own* device's delay | none |

T1 asserts equality rather than a lower bound: under virtual time there is no
skew to tolerate, and an inequality would also pass for a delay that was too
long. It reads the clock with `tokio::time::Instant`, which is the virtual one —
`std::time::Instant` would report real elapsed time and defeat the whole
strategy.

T5 deliberately does **not** assert `elapsed == ZERO`. That assertion cannot
tell the two implementations apart: a spawned task with a zero sleep is still
polled before the runtime auto-advances, so the reply lands at `0ns` either way.
What genuinely differs is whether the reply is already queued when `send`
returns, so T5 asserts exactly that, via a zero-duration timeout around `recv`.
Stated as behaviour: a device with no configured delay has its reply available
the moment `send` completes.

That this discriminates was measured, not assumed — `tokio::time::timeout` polls
the inner future before consulting its deadline, so a zero-duration timeout is a
"is it ready right now?" probe rather than an immediate failure:

```
PROBE sync-path  -> ok=true      // synchronously queued: visible
PROBE spawn-path -> ok=false     // spawned with sleep(ZERO): not yet
```

T6 has no Go counterpart and is kept deliberately. Per-reply delays mean a fast
device overtakes a slow one in the same fan-out, matching real hardware. It
asserts each reply's own arrival time rather than merely their order — order
alone would still pass if every reply in the batch were given the same delay,
which is the mutation it exists to catch.

T3 is the interaction test the knob table implies: `unreachable` wins, and
`slow` must not resurrect a reply that should never exist.

T2 expresses its deadline as `tokio::time::timeout` around the operation,
per ADR-2 — `CancellationToken` carries cancellation, `timeout` carries the
deadline, together standing in for Go's `context.Context`.

### Mutation checks

Each verified over repeated runs, per the criterion above: one observed failure
cannot distinguish a reliable test from a coin flip.

| Mutation | Must fail | Why that test and not another |
| --- | --- | --- |
| `schedule` always enqueues synchronously | T1, T2, T4, T6 | the delay disappears entirely |
| `schedule` always spawns | T5 | only T5 observes the moment `send` returns |
| drop `slow` from the error-reply path | T4 | T1 uses a success reply and stays green |
| give every reply in a batch the same delay | T6 | single-reply tests cannot see a per-batch error |

The third and fourth rows are why T4 and T6 exist at all: each is the only test
that can distinguish its mutation. A mutation table where two rows name the same
test is a signal that one of the tests is redundant, and one where a row names
no test is a coverage gap stated out loud.

## Build changes

`crates/mocksbr/Cargo.toml` gains its first `[dev-dependencies]`:

```toml
[dev-dependencies]
tokio = { workspace = true, features = ["rt", "macros", "time", "test-util"] }
```

`test-util` is what provides `start_paused`. `crates/udap` already declares it;
mocksbr declares no dev-dependencies at all today, so T5 cannot be written
without this.

## Documentation

Two entries, serving different readers.

**`docs/specs/2026-09-08-rust-port-spec.md` → Accepted behavioural deltas.**
That table's header is *"Anything not listed here is a bug"*, so a knowing
divergence belongs in it regardless of user visibility — it already carries
`devices` keyed by `Mac` (ADR-5) with impact *"None observable"*.

> | `Network::receive` returns `ScheduledReply` rather than go-udap's
> `Receive`/`ReceiveScheduled` pair | None observable. Wire behaviour is
> identical; the split is dropped so no caller can silently bypass `Slow`, the
> same trap that left `Op::Save` and `Op::Discover` inert in #28 |

**`docs/port-map.md` → `### mocksbr crate`.** The structural Go→Rust mapping,
where a reader comparing the two codebases will look for it.

## Non-goals

- **`RebootDelay`** — the post-reset reboot window. Needs the same clock but is
  a separate behaviour (`handlers.go:106`, `d.rebooting()`), and go-udap's
  mocksbr tests never exercise it. M6.
- **`DropGetData`, `SuppressDiscoveryUUID`** — zero uses in go-udap's mocksbr
  tests. M6.
- **`MockTransport::InjectReply`** — tracked as
  [#29](https://github.com/yo61/udapcfg-rs/issues/29). Touches the same file but
  answers a different question: source validation, not timing.
- **The standalone mocksbr binary** — M5-C. `ScheduledReply` is what its UDP
  server will need, so this design unblocks it without doing it.

## Success criteria

1. `DeviceConfig.slow` delays every reply the device produces, error replies
   included, and delays nothing when it is `ZERO`.
2. go-udap's two `Slow` tests have Rust equivalents asserting exact timings
   rather than bracketed ones.
3. All six tests run under virtual time; the suite's wall-clock time does not
   measurably increase.
4. Every mutation in the table above is caught across repeated runs.
5. `cargo clippy --all-targets --all-features` and `cargo fmt --all --check`
   stay clean; the workspace suite stays green.
