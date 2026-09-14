# M3 Task 6 — real-hardware verification

Everything in M3 except this has landed. Tasks 1–5 are review-clean, 108 tests pass,
and all three client paths work against real sockets on the dev host. This document is
the part that needs equipment.

**You need:** a Squeezebox in setup mode (front light flashing red — hold the front
button 3–6 seconds), on the same L2 segment. Steps 5–7 additionally need a Linux box
with an unprivileged user; steps 8–9 need a host with **two or more** usable interfaces.

**Setup:**

```sh
cd ~/code/github.com/yo61/udapcfg-rs
git switch feat/m3-udp-transport
mise exec -- cargo build -p udap-cli          # → target/debug/udapcfg
cd ~/code/github.com/yo61/go-udap && go build -o /tmp/go-udap . && cd -
```

---

## 1. Discovery against real hardware

```sh
/tmp/go-udap discover
./target/debug/udapcfg discover
```

The MAC lists must match. This is the milestone's headline claim — the first time
`udapcfg` talks to a device rather than a mock.

## 2. The loopback filter on a real socket

```sh
./target/debug/udapcfg -v discover
```

Confirm the debug log shows our own broadcast being skipped, and that **no device with
MAC `00:00:00:00:00:00`** appears. That phantom is what happens if `is_request_packet`
is not consulted, and until now the filter has only been exercised against a mock.

## 3. `--bind-interface`, both ways

```sh
./target/debug/udapcfg --bind-interface <name-that-reaches-the-device> discover
./target/debug/udapcfg --bind-interface <name-that-does-not> discover
```

The first finds the device; the second finds nothing and **exits 0** — finding nothing
is not an error.

## 4. `--all-interfaces`

```sh
./target/debug/udapcfg --all-interfaces discover
```

The device must appear **exactly once**, not once per interface. If it duplicates, the
`BTreeMap<Mac, Device>` dedup is not working.

---

## 5. OQ-2 — does `SO_BINDTOIFINDEX` need privileges? (Linux)

**As an unprivileged user:**

```sh
mise exec -- cargo run -q -p udap-cli -- --bind-interface <name> discover; echo "exit=$?"
```

go-udap's error text claims `SO_BINDTODEVICE` "may require CAP_NET_RAW".
`SO_BINDTOIFINDEX` is a different option and may not. **Record which it is.**

- If it needs privileges, the error message must say so — that is fidelity-contract text.
- If it does not, the Rust port is *less* restrictive than the Go. Add that to the
  spec's accepted-deltas table.

## 6. Confirm the Linux kernel floor

```sh
uname -r
```

`SO_BINDTOIFINDEX` needs **kernel 5.7+**; go-udap's `SO_BINDTODEVICE` does not. The
spec already names this as an accepted narrowing — record the kernel you tested on. If
you have anything older to hand, test there too.

## 7. The Windows arm

Neither the Linux nor the Windows `#[cfg]` arm has been *runtime*-exercised — only
`aarch64-apple-darwin` is installed here, so even a cross `cargo check` cannot run.
Step 5 covers Linux. Windows should at minimum surface
`binding egress to a specific network interface is not supported on this platform`
rather than silently succeeding.

---

## 8. `--all-interfaces` with more than one real NIC

**Added by the final review, and it is the biggest untested gap.** This host has one
usable interface, so genuine fan-out has only ever run against mock children. Never
verified against two live sockets:

- the `select_all` merge across real `UdpTransport`s
- per-child retirement when a real socket errors
- that `SO_REUSEPORT` genuinely permits N sockets on `0.0.0.0:17784` (proven for 2 on
  ephemeral ports, never on the real port with real interfaces)

A host with Wi-Fi plus Ethernet, or a USB NIC, is enough.

## 9. A flaky interface mid-discovery

If you can arrange it — bring a VPN tunnel down during a `--all-interfaces` run.
Discovery on the healthy interfaces must continue. This is the behaviour the Task 4 fix
added (per-child retirement matching go-udap's `pumpChild`), and it is the substantive
reason `MultiTransport` exists rather than picking one interface and hoping.

---

## 10. Update the spec

Amend `docs/specs/2026-09-08-rust-port-spec.md`:

- Mark **OQ-1** and **OQ-2** resolved with what you measured.
- Add any behavioural delta found to the accepted-deltas table.
- If M3 showed a milestone description wrong — as M2 did for ADR-3's ownership
  prediction — fix it.

```sh
git add docs/specs/
git commit -S -m "docs(spec): resolve OQ-1 and OQ-2 against real hardware"
```

---

## Known deltas already recorded — do not re-file

1. **Linux uses `SO_BINDTOIFINDEX`**, not go-udap's `SO_BINDTODEVICE`. Kernel 5.7+.
2. **The Windows-unsupported message deliberately omits the flag name.** go-udap's
   `socket_windows.go:39` names `--bind-interface` in the transport layer, contradicting
   its own `client.go:425`. Copying that would have re-introduced a layering defect the
   final review flagged. Deliberate.
3. **`format_value` dispatches on wire length, not table width** — faithful to
   `formatGetDataValue`; a carried-forward wart.
4. **Non-UTF-8 values are lossy** — [issue #3](https://github.com/yo61/udapcfg-rs/issues/3),
   unresolved, and cheapest to decide before M4 builds `read`/`get`/`set` on `String`.

## Carried findings, none blocking

- `enumerate()`'s tests cannot catch a regression removing the `IFF_BROADCAST` check —
  a VPN interface still has a name, index and non-loopback IPv4. Worth extracting a pure
  `fn is_usable(up, broadcast, loopback, has_ipv4)` and table-testing it (~10 lines).
- `MultiTransport`'s retirement flags use `SeqCst` where `Relaxed` would be sound.
- The `prefix_len == 0` branch in `directed_broadcast` is redundant.
- An unexplained nextest `leaky` flag, seen twice on unrelated code, never reproduced
  (11 runs in the final review). The final review falsified the socket-leak theory:
  the M0–M2 sighting was on a synchronous decoder with no sockets or tasks.

---

## Results — 2026-09-13

Run against a real Squeezebox in setup mode, MAC `00:04:20:16:17:18`
(`00:04:20` is the Slim Devices OUI; UDAP reports `ip=0.0.0.0`, confirming
setup mode). Dev host: macOS 25.6.0, `aarch64-apple-darwin`. Reference:
go-udap built from `7cce675`.

### Provenance of the reference binary

The spec pins the source of truth to go-udap `v2.4.8` (`43864a5`), but the
comparisons below used `7cce675` on the dev host and the `v2.4.9` release on
Linux. That is not a gap: `git diff --name-only v2.4.8..7cce675 -- '*.go' go.mod
go.sum` is **empty**, and so is `v2.4.8..v2.4.9`. Everything between those points
is CI, goreleaser, the docs site and decision records. The three binaries are
behaviourally identical, so every "matches" below holds for the pinned reference.

Check this again if go-udap ever ships a release that does touch `udap/` or
`cli/` — at that point the comparison would need re-running or the pin moving.

**This host no longer has one usable interface.** It has two — `en0`
(192.168.1.243) and `en8` (192.168.20.169) — which is what made step 8
possible. The preamble above, written when it had one, is stale.

| Step | Result |
|------|--------|
| 1. Discovery vs go-udap | **Pass.** Identical MAC, exit 0, 6/6 runs. |
| 2. Loopback filter on a real socket | **Pass.** `skipping our own looped-back request src=192.168.20.169:17784`; no `00:00:00:00:00:00` phantom. |
| 3. `--bind-interface` both ways | **Pass** (on Linux). Positive and negative both verified on `nas1`; untestable on the dev host — see below. |
| 4. `--all-interfaces` exactly once | **Pass.** 6/6, one MAC per run. |
| 5. OQ-2, `SO_BINDTOIFINDEX` privileges | **Pass. RESOLVED: no privileges needed.** See below. |
| 6. Linux kernel floor | **Pass.** Verified on 6.18.42, well above the 5.7 floor. Nothing older to hand. |
| 7. Windows arm | **Open.** The CI matrix compiles and lints it (116 tests pass on both Windows targets); nothing invokes it. |
| 8. Two real NICs | **Pass.** See below. |
| 9. Flaky interface mid-discovery | **Pass**, with a caveat — see below. |

### Step 3's negative case does not exist on *this* topology

Tested instead on `nas1`, which does have two separate segments — see the Linux
results below. On the dev host:

Both `en0` and `en8` find the device, because a setup-mode Squeezebox has no
IP address and so answers any L2 broadcast reaching its NIC regardless of
subnet. `en0` and `en8` are two paths onto the same physical segment. There is
no interface here that legitimately fails to reach it, so "finds nothing, exits
0" could not be tested deliberately — though it *was* observed incidentally
whenever `en8` dropped a packet: `no devices found within 2s` on stderr, exit 0.

### Step 8 passed, and the dedup is doing real work

`-v --all-interfaces` on the dev host (2 NICs) shows **four** `found device`
events for one device, collapsed to a single line of output. go-udap shows four
too. On `nas1` (10 NICs, only one of which reaches the device) both show
**two**, and both still print one MAC. The dedup is doing real work in each
case, and the two implementations agree exactly.

### The two platform arms filter ingress differently

Worth knowing before anyone runs `--all-interfaces` on a many-NIC host, and not
something any test had surfaced:

| Host | NICs bound | Own-broadcast skips |
|------|-----------|---------------------|
| macOS, `IP_BOUND_IF` | 2 | 4 |
| Linux, `SO_BINDTOIFINDEX` | 10 | 10 |

On macOS each socket sees **every** sibling's broadcast, so loopback skips grow
with the square of the interface count. On Linux each socket sees only its own,
so they grow linearly. The options are not equivalent: `IP_BOUND_IF` pins egress
only and leaves ingress unfiltered, while `SO_BINDTOIFINDEX` is a device binding
that filters ingress as well.

This is faithful — go-udap selects the same option per platform — and it costs
nothing at these sizes. Recorded because the quadratic arm is the macOS one, and
a macOS host with many interfaces would pay for it.

### OQ-1's "Verify at M3" rider is resolved

The spec asks to confirm `netdev` still populates `flags` with default features
off. It does. A probe against the same `netdev` 0.46 configuration the crate
uses reports `utun16` (Tailscale) as **up, non-loopback, carrying IPv4
100.122.155.106, and `is_broadcast() == false`** — excluded by the broadcast
test alone. Flags are discriminating, not defaulted.

This also makes the carried finding below concrete rather than theoretical: on
*this* host, deleting the `IFF_BROADCAST` check would make `utun16` usable and
point discovery into a Tailscale tunnel.

### Error paths match byte-for-byte

Both tools, for an unknown interface and for `lo0`:

```
error: --bind-interface: "nosuch0" is not usable (must be up, broadcast-capable, with an IPv4 address)
```

exit 1 in each case.

### The Linux host these results used

`nas1` (TrueNAS Scale, Linux 6.18.42, glibc 2.41, x86_64) is multi-homed onto
both segments — `bond0` 192.168.1.10 and `vlan20` 192.168.20.10 — and has an
unprivileged account, which is what let it answer OQ-2 *and* redo steps 1–4 and
8 on Linux. Those results are in the section below.
It also has ten broadcast-capable IPv4 interfaces, which stress
`--all-interfaces` far harder than two.

One trap: `/tmp` is `noexec` there, as are the boot-pool `/home` and `/mnt`
datasets. But `$HOME` is **not** under those — it is `/mnt/space/home/robin`,
its own ZFS dataset mounted without `noexec` — so `~/bin` runs fine and is
where the binaries live. `/var/tmp` also works. Check with
`findmnt -no OPTIONS --target <path>` rather than reasoning from the parent
directory's mount, which is what makes this a trap.

`~/bin` is not on the default PATH there (`/usr/local/bin:/usr/bin:/bin:/usr/games`),
so invoke by full path or add it.

Use a static musl binary from the CI build matrix rather than an ad-hoc local
cross-build, so the artifact comes from a recorded toolchain.


---

## Linux results — 2026-09-13, `nas1`

TrueNAS Scale, Linux 6.18.42, glibc 2.41, x86_64, unprivileged user `robin`,
10 usable interfaces. Binaries: `udapcfg` from the CI musl build matrix,
`go-udap` v2.4.9 from the GitHub release — both official builds, no ad-hoc
cross-compilation.

Discovery matches: both print `00:04:20:16:17:18`, exit 0.

### OQ-2 resolved — `SO_BINDTOIFINDEX` needs no privileges

As unprivileged `robin` on kernel 6.18.42:

```
./udapcfg --bind-interface bond0 discover   ->  00:04:20:16:17:18   exit 0
```

The socket bound and discovery succeeded. No `EPERM`, no `CAP_NET_RAW`.
go-udap, which uses `SO_BINDTODEVICE`, also succeeded unprivileged on the same
kernel — so on 6.18.42 neither option is restricted, and the Rust port is not
more restrictive than the Go.

**Consequence for the fidelity contract:** the error text does *not* need to
mention privileges. Recorded in the spec's accepted-deltas table by this change:
the privilege warning in go-udap's message is not reproducible on a current
kernel, for either implementation.

Not established: whether an older kernel restricts `SO_BINDTODEVICE` where
`SO_BINDTOIFINDEX` would not. Only 6.18.42 was available.

### Step 3's negative case, finally testable

`nas1` is multi-homed onto two genuinely separate segments, which the dev host
was not:

```
--bind-interface bond0    (192.168.1.10)   ->  00:04:20:16:17:18   exit 0
--bind-interface vlan20   (192.168.20.10)  ->  no devices found within 2s, exit 0
```

go-udap gives identical output for both. Finding nothing is not an error —
verified, not assumed.

### Steps 4 and 8 at ten interfaces

`--all-interfaces` binds 10 transports and prints the device exactly once,
3/3 runs, matching go-udap. The `BTreeMap<Mac, Device>` dedup holds at five
times the interface count the dev host could offer.

### A fidelity bug found: non-deterministic `interfaces` order

`udapcfg interfaces` prints its rows in a different order on every run on Linux;
go-udap is stable at ascending index. Ten interfaces made it obvious where two
never could. Cause is `netdev`'s Linux netlink backend collecting through a
`HashMap` and losing the kernel's ordering.

Filed as [issue #13](https://github.com/yo61/udapcfg-rs/issues/13) and fixed by
this change: `enumerate()` now sorts by stem and trailing unit number, so `en2`
precedes `en10`, VLAN sub-interfaces sort by id, and opaque hex ids such as
docker's compare in hex order. The divergence is recorded in the spec's
accepted-deltas table.

Note that this cannot be fixed by matching go-udap, because go-udap does not
sort at all — it prints OS order, and no single rule reproduces that on both
platforms: it is ascending index on Linux but creation order on macOS, where
`en0` (index 15) precedes `en8` (index 13). Any fix is therefore a deliberate
divergence and needs its own accepted-deltas row.

### Step 9 — a link dropped mid-discovery

Run on 2026-09-14 against the dev host. `--all-interfaces -v` with a 60-second
timeout; `en8` was brought down six seconds in, with its flags sampled every
second to prove when it happened:

```
18:48:02  flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST>
18:48:08  flags=8822<BROADCAST,SMART,SIMPLEX,MULTICAST>      <- down
```

Both transports had bound and pinned before the drop (`en0` index 15, `en8`
index 13). Discovery ran the full 60 seconds, ended on its own timeout, exited
**0**, and reported `00:04:20:16:17:18` exactly once.

**The caveat, which matters for what this does and does not prove.** Nothing
was retired, because nothing errored. A `SO_REUSEPORT` socket bound to
`0.0.0.0` does not fail when the interface its egress is pinned to goes down —
it simply stops receiving. There is no error for `MultiTransport` to react to.

So the *outcome* the checklist asks for is verified on real hardware: discovery
on the healthy interface continued and the run completed cleanly. The
**per-child retirement mechanism itself is still exercised only by mocks**,
because this failure mode never triggers it. Retirement needs a `recv` that
returns an error, not one that goes quiet.

Expect the same on Linux: `SO_BINDTOIFINDEX` filters ingress, so a downed
interface there would likewise fall silent rather than error. Whatever does
trigger retirement on a real socket, it is not an interface going down.

A first run of this test was discarded: `en8` never actually went down, and the
30-second window elapsed with the interface still `UP` throughout. Sampling the
flags is what caught it — without that the run looks identical to a pass.

### How lossy is `en8`, and do retries help?

`en8` loses packets often enough to matter, so it doubles as a test of
`--retries`. 450 discoveries, 150 per arm, interleaved run-by-run so that
drift in the network hits every arm equally:

| `--retries` | packets sent | found the device |
|---|---|---|
| 0 | 1 | 139/150 — **92.7%** |
| 2 | 3 | 149/150 — **99.3%** |
| 5 | 6 | 150/150 — **100%** |

**The loss is independent per packet, not bursty.** Single-packet failure is
11/150 ≈ 7.3%; if each packet fails independently, three should fail
0.073³ ≈ 0.04% of the time, or about 0.06 runs in 150. One was observed. Six
packets should essentially never fail, and none did.

That settles a design question worth not re-opening. `send_retried` fires every
copy back-to-back with no inter-send delay, copying squeezeplay's triple-send,
and the fidelity contract pins it that way. It is tempting to assume spreading
the retransmits across the listen window would be more robust — but that only
helps against *bursty* loss, and this is not bursty. Packets microseconds apart
already fail independently, so spacing them would buy nothing. The inherited
design is sound.

Do not conclude anything about retries from a small sample. An earlier reading
of 11/12 runs suggested retries did not help; twelve runs cannot distinguish
92.7% from 99.3%, where the expected difference is under one run.

### Remaining

- **Step 7 (Windows runtime).** Still open. The CI matrix compiles and lints the
  Windows arm on both `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`,
  and the suite reports 116 tests run / 116 passed there. But no test calls
  `bind_on_interface`, so the unsupported-interface path itself is still never
  invoked, and nobody has observed Windows surfacing that message at runtime.
- **Step 9 (flaky interface mid-discovery).** Passed, but see the caveat: it
  verifies the outcome, not the retirement path.
