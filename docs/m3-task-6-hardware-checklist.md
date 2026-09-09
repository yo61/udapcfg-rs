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
