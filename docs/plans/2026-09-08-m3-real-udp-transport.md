# udapcfg-rs M3 Implementation Plan — real UDP transport

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the placeholder client factory with a real UDP socket, so `udapcfg discover` finds actual Squeezebox hardware — plus interface enumeration, `--bind-interface`, `--all-interfaces`, and the `interfaces` subcommand.

**Architecture:** `socket2` builds and configures the socket, then hands it to `tokio::net::UdpSocket`. `netdev` supplies interface enumeration with real `IFF_*` flags. `MultiTransport` composes N `UdpTransport`s with one `tokio::spawn` per child merging into an `mpsc` channel. The `Transport` trait from M2 is the seam — `Client` is unchanged.

**Tech Stack:** Rust 1.98.1, tokio (current_thread), socket2 0.6 (`all`), netdev 0.46 (no default features), clap 4.

**Spec:** [`docs/specs/2026-09-08-rust-port-spec.md`](../specs/2026-09-08-rust-port-spec.md)
**Reference:** [`docs/port-map.md`](../port-map.md)
**Prior plan:** [`2026-09-08-m0-m2-walking-skeleton.md`](2026-09-08-m0-m2-walking-skeleton.md)

## Global Constraints

- **Source of truth is go-udap v2.4.8 (`43864a5`)** at `~/code/github.com/yo61/go-udap`. Where this plan and the Go disagree, the Go wins — read it and fix the plan.
- **Discovery always sends to `255.255.255.255`.** Never a directed subnet broadcast. Unconfigured devices have source IP `0.0.0.0`, no subnet, and only process limited broadcast. go-udap's `docs/superpowers/plans/2026-05-13-getip-hwrev-uuid-iface.md` records the wire-trace spike where directed broadcast meant pre-DHCP devices never replied. **Do not re-derive this on hardware.**
- **`NetInterface.Broadcast` is informational only** — printed by `interfaces`, never used as a destination.
- **The toolchain is mise-managed and NOT on PATH.** Prefix every cargo command: `mise exec -- cargo ...` from the repo root.
- **Zero warnings.** `cargo clippy --all-targets --all-features` and `cargo fmt --all --check` clean. `RUSTFLAGS=-D warnings` is set in `mise.toml`.
- **Zero `#[allow(...)]` in the repo.** Suppress only with `#[expect(lint, reason = "...")]`, and only when a rewrite genuinely cannot satisfy the lint. There are currently four, all `cast_possible_truncation`.
- **No `.unwrap()`/`.expect()` in non-test code.** `clippy.toml` exempts tests, but only in frames carrying `#[test]`/`#[tokio::test]` — a fallible call in a plain helper fn still fires. Move the call, do not suppress.
- **`clippy::panic` is denied everywhere, including tests.** Use `assert!`/`assert_eq!`, never `panic!`.
- **`crates/udap/src/lib.rs` is append-only.** It currently declares `client device error getdata mac parameters protocol tlv transport` and re-exports `Client ClientError Device EncodeError GetDataError ProtocolError Mac MacParseError HEADER_SIZE PORT Packet`. Widening a `pub use` list is fine; losing a name is not.
- **`clippy::print_stdout`/`print_stderr` are denied.** All output through injected writers.
- **TDD.** Failing test first, watch it fail, then implement.
- **Commit per task**, conventional-commit format, `git commit -S`, on a feature branch. Never commit to `main`.
- **Pushing runs the lastlight gate.** Each push needs an independent review recorded for that exact SHA — see [Pushing](#pushing) at the end.

## File Structure

```
crates/udap/
  src/interfaces.rs               NetInterface + enumerate()          [Task 1]
  src/transport/udp.rs            UdpTransport (socket2 -> tokio)     [Task 2]
  src/transport/multi.rs          MultiTransport fan-out              [Task 4]
  src/transport/mod.rs            (modified: add submodules)          [Tasks 2, 4]
  src/client.rs                   (modified: constructors)            [Task 3]
  src/lib.rs                      (modified: append modules)          [Tasks 1, 2, 4]
  Cargo.toml                      (modified: socket2, netdev)         [Task 1]

crates/udap-cli/
  src/cli.rs                      (modified: global flags)            [Task 3]
  src/cmd/interfaces.rs           `interfaces` subcommand             [Task 5]
  src/cmd/mod.rs                  (modified)                          [Task 5]
  src/output.rs                   formatted table                     [Task 5]
  src/lib.rs                      (modified: factory takes flags)     [Task 3]
  src/main.rs                     (modified: real factory)            [Task 3]
```

**Not in this plan:** the remaining five UCP operations (`get_data`, `set_data`, `reset`, `get_ip`, `get_uuid`) — that is M4.

---

### Task 1: Interface enumeration

Port of `udap/interfaces.go`. **Resolves the OQ-1 verification item.**

**Files:**
- Create: `crates/udap/src/interfaces.rs`
- Modify: `crates/udap/Cargo.toml`, `crates/udap/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct NetInterface { pub name: String, pub index: u32, pub addr: Ipv4Addr, pub broadcast: Ipv4Addr }`
  - `pub fn enumerate() -> Result<Vec<NetInterface>, InterfaceError>`
  - `pub enum InterfaceError`

**The filter is the whole point.** go-udap keeps an interface only when **all** hold: `IFF_UP` set, `IFF_BROADCAST` set, `IFF_LOOPBACK` clear, and it has at least one IPv4 address. The `IFF_BROADCAST` test is what excludes WireGuard and Tailscale tunnels — they do not carry that flag. Losing it means discovery fans out across VPN links.

Only the **first** IPv4 address per interface is used, matching the Go's `break`.

- [ ] **Step 1: Add the dependencies**

In `crates/udap/Cargo.toml`, under `[dependencies]`:

```toml
netdev.workspace = true
```

`netdev` is already declared in the workspace root as `{ version = "0.46", default-features = false }`. Defaults would pull `gateway` detection and `apple-system-configuration-extra`, which drags Objective-C bindings onto macOS.

- [ ] **Step 2: Write the failing tests**

`crates/udap/src/interfaces.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn directed_broadcast_is_addr_or_inverted_mask() {
        // 192.168.1.50/24 -> 192.168.1.255
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 1, 50), 24),
            Ipv4Addr::new(192, 168, 1, 255)
        );
        // 10.0.0.5/8 -> 10.255.255.255
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 8),
            Ipv4Addr::new(10, 255, 255, 255)
        );
        // /32 has no host bits, so the broadcast is the address itself
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(172, 16, 0, 1), 32),
            Ipv4Addr::new(172, 16, 0, 1)
        );
        // /0 is the whole space
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(172, 16, 0, 1), 0),
            Ipv4Addr::new(255, 255, 255, 255)
        );
    }

    // enumerate() reads the real machine, so assert invariants rather than
    // a fixed list: any host running this has at least a loopback to exclude.
    #[test]
    fn enumerate_applies_the_filter() {
        let ifs = enumerate().expect("enumeration must not error");
        for ni in &ifs {
            assert!(!ni.name.is_empty(), "interface with empty name");
            assert!(ni.index > 0, "{} has index 0", ni.name);
            assert!(!ni.addr.is_loopback(), "{} is a loopback", ni.name);
            assert!(!ni.addr.is_unspecified(), "{} has 0.0.0.0", ni.name);
        }
    }

    #[test]
    fn enumerate_yields_one_entry_per_interface() {
        let ifs = enumerate().expect("enumeration must not error");
        let mut names: Vec<&str> = ifs.iter().map(|n| n.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "an interface appeared twice");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap interfaces
```

Expected: compilation failure — `enumerate` and `directed_broadcast` are not defined.

- [ ] **Step 4: Implement**

Prepend to `crates/udap/src/interfaces.rs`:

```rust
//! Local network interfaces usable for UDAP broadcast discovery.
//!
//! An anti-corruption layer over `netdev`: the rest of the crate sees
//! `NetInterface` and never the enumeration crate's types.

use std::net::Ipv4Addr;

/// One interface that discovery can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetInterface {
    pub name: String,
    /// OS interface index. Required by `bind_device_by_index_v4`.
    pub index: u32,
    /// First IPv4 address on the interface.
    pub addr: Ipv4Addr,
    /// The subnet's directed-broadcast address.
    ///
    /// Informational only — shown by `udapcfg interfaces`. UDAP sends
    /// always target the limited broadcast 255.255.255.255, because
    /// unconfigured devices have no subnet and would not receive a
    /// directed broadcast.
    pub broadcast: Ipv4Addr,
}

#[derive(Debug, thiserror::Error)]
pub enum InterfaceError {
    #[error("enumerate interfaces: {0}")]
    Enumerate(String),
}

/// Returns the subnet's directed-broadcast address: `addr | !mask`.
fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Ipv4Addr {
    // A /0 mask is 0, and shifting a u32 by 32 is undefined in Rust, so
    // compute the mask via checked_shl and treat the overflow as "no bits".
    let mask: u32 = if prefix_len == 0 {
        0
    } else {
        u32::MAX
            .checked_shl(u32::from(32 - prefix_len))
            .unwrap_or(0)
    };
    Ipv4Addr::from(u32::from(addr) | !mask)
}

/// Every interface usable for UDAP broadcast discovery.
///
/// The filter matches go-udap exactly: up, broadcast-capable, not a
/// loopback, and carrying at least one IPv4 address. Only the first IPv4
/// address per interface is taken.
///
/// The broadcast-capable test is load-bearing: WireGuard and Tailscale
/// interfaces do not set `IFF_BROADCAST`, so this is what keeps discovery
/// off VPN tunnels.
///
/// # Errors
/// [`InterfaceError::Enumerate`] if the platform enumeration fails.
pub fn enumerate() -> Result<Vec<NetInterface>, InterfaceError> {
    let mut out = Vec::new();
    for iface in netdev::get_interfaces() {
        if !iface.is_up() || !iface.is_broadcast() || iface.is_loopback() {
            continue;
        }
        let Some(net) = iface.ipv4.first() else {
            continue;
        };
        out.push(NetInterface {
            name: iface.name.clone(),
            index: iface.index,
            addr: net.addr(),
            broadcast: directed_broadcast(net.addr(), net.prefix_len()),
        });
    }
    Ok(out)
}
```

> **Verify the `netdev` API shape before assuming it.** `Ipv4Net`'s accessors may be `addr()`/`prefix_len()` or fields `addr`/`prefix_len` depending on the version of the `ipnet` type it re-exports. Run `mise exec -- cargo doc -p netdev --no-deps --open`, or read
> `~/.cargo/registry/src/*/netdev-0.46*/src/interface/interface.rs`, and adjust. Report which form you found.

- [ ] **Step 5: Wire in and run**

Add `pub mod interfaces;` and `pub use interfaces::{InterfaceError, NetInterface};` to `crates/udap/src/lib.rs`.

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets --all-features
```

- [ ] **Step 6: Verify against go-udap on the same machine — this closes OQ-1**

```bash
cd ~/code/github.com/yo61/go-udap && go build -o /tmp/go-udap . && cd -
/tmp/go-udap interfaces
mise exec -- cargo test -p udap --lib interfaces -- --nocapture
```

Compare the interface **names and indices** the two produce. They must agree exactly. If Rust lists an interface Go does not (or vice versa), the flag handling differs and that is a defect — investigate before continuing, and record what you found.

**Specifically confirm** that `netdev` populates `flags` with `default-features = false`. If `is_up()`/`is_broadcast()` return `false` for everything, the flags are not being read and the feature set is wrong — say so rather than working around it.

- [ ] **Step 7: Commit**

```bash
git add crates/udap/src/interfaces.rs crates/udap/src/lib.rs crates/udap/Cargo.toml Cargo.lock
git commit -S -m "feat(udap): add interface enumeration

Ports udap/interfaces.go over netdev, taken with default-features off.
The broadcast-capable filter is what keeps discovery off WireGuard and
Tailscale tunnels, which do not set IFF_BROADCAST.

Output verified to match \`go-udap interfaces\` on the same host."
```

---

### Task 2: `UdpTransport`

Port of `udap/transport.go` plus the four `socket_*.go` files. **The two platform files collapse into one call.**

**Files:**
- Create: `crates/udap/src/transport/udp.rs`
- Modify: `crates/udap/src/transport/mod.rs`, `crates/udap/Cargo.toml`

**Interfaces:**
- Consumes: `Transport`, `TransportError` (M2); `NetInterface` (Task 1); `protocol::{PORT, is_request_packet}` (M2)
- Produces:
  - `pub struct UdpTransport`
  - `UdpTransport::bind(port: u16) -> Result<UdpTransport, TransportError>`
  - `UdpTransport::bind_on_interface(iface: &NetInterface, port: u16) -> Result<UdpTransport, TransportError>`
  - `UdpTransport::local_addr(&self) -> Result<SocketAddr, TransportError>` (test helper)
  - `impl Transport for UdpTransport`
  - `TransportError::InterfaceBindUnsupported` (new variant)

**Construction order is the contract.** `SO_REUSEADDR`/`SO_REUSEPORT` must be set **before** `bind`; `SO_BROADCAST` after. `set_nonblocking(true)` must precede `from_std` or the runtime stalls on first read — this is the direct analogue of go-udap's documented `SyscallConn().Control()`-not-`File()` hazard.

- [ ] **Step 1: Add the dependency**

In `crates/udap/Cargo.toml` under `[dependencies]`:

```toml
socket2.workspace = true
```

- [ ] **Step 2: Write the failing tests**

`crates/udap/src/transport/udp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    // Port 0 lets the OS pick, so tests never collide with each other or
    // with anything holding 17784.
    #[tokio::test]
    async fn binds_and_reports_its_address() {
        let t = UdpTransport::bind(0).expect("bind on an ephemeral port");
        let addr = t.local_addr().expect("local_addr");
        assert!(addr.port() > 0);
        assert!(addr.ip().is_unspecified(), "must bind 0.0.0.0 to hear limited broadcast");
    }

    #[tokio::test]
    async fn two_sockets_can_share_a_port() {
        // SO_REUSEPORT must be set pre-bind, or the second bind fails.
        // This is what NewClientForAllInterfaces relies on.
        let a = UdpTransport::bind(0).expect("first bind");
        let port = a.local_addr().expect("local_addr").port();
        let b = UdpTransport::bind(port);
        assert!(b.is_ok(), "second bind on the same port must succeed");
    }

    #[tokio::test]
    async fn recv_returns_cancelled_when_the_token_fires() {
        let t = UdpTransport::bind(0).expect("bind");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = t.recv(&cancel).await.expect_err("must not block");
        assert!(matches!(err, TransportError::Cancelled));
    }

    // We broadcast with the request bit set and the kernel loops it back.
    // recv must drop it rather than surface our own packet as a reply.
    #[tokio::test]
    async fn recv_skips_our_own_looped_back_request() {
        use crate::protocol::{Packet, ADDR_TYPE_ETH, FLAG_REQUEST, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
        use crate::Mac;

        let t = UdpTransport::bind(0).expect("bind");
        let port = t.local_addr().expect("local_addr").port();

        let request = Packet {
            dst_broadcast: 1,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::ZERO,
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: FLAG_REQUEST,
            uap_class: UAP_CLASS_UCP,
            ucp_method: method::ADV_DISC,
        }
        .to_bytes();

        // Send it to ourselves on loopback, then confirm recv ignores it.
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.expect("sender");
        sender
            .send_to(&request, ("127.0.0.1", port))
            .await
            .expect("send");

        let cancel = CancellationToken::new();
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            token.cancel();
        });

        let err = t.recv(&cancel).await.expect_err("the request must be skipped");
        assert!(matches!(err, TransportError::Cancelled));
    }

    #[tokio::test]
    async fn recv_returns_a_reply_packet() {
        use crate::protocol::{Packet, ADDR_TYPE_ETH, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
        use crate::Mac;

        let t = UdpTransport::bind(0).expect("bind");
        let port = t.local_addr().expect("local_addr").port();

        // Same packet with the request bit CLEAR: a device reply.
        let reply = Packet {
            dst_broadcast: 0,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::from_bytes([0x00, 0x04, 0x20, 0x00, 0x00, 0x01]),
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0x00,
            uap_class: UAP_CLASS_UCP,
            ucp_method: method::ADV_DISC,
        }
        .to_bytes();

        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0").await.expect("sender");
        sender.send_to(&reply, ("127.0.0.1", port)).await.expect("send");

        let cancel = CancellationToken::new();
        let (got, src) = t.recv(&cancel).await.expect("a reply must come through");
        assert_eq!(got, reply.to_vec());
        assert_eq!(src, "127.0.0.1");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
mise exec -- cargo nextest run -p udap udp
```

Expected: compilation failure — `UdpTransport` is not defined.

- [ ] **Step 4: Add the new error variant**

In `crates/udap/src/transport/mod.rs`, extend `TransportError`:

```rust
    #[error("{flag} is not supported on this platform")]
    InterfaceBindUnsupported { flag: &'static str },
```

- [ ] **Step 5: Implement**

`crates/udap/src/transport/udp.rs`:

```rust
//! `Transport` over a real UDP socket.
//!
//! `socket2` builds and configures the socket; `tokio` runs it. The split
//! exists because `tokio::net::UdpSocket` cannot set the options UDAP
//! needs, and `socket2::Socket` is blocking.

use crate::interfaces::NetInterface;
use crate::protocol::{self, PORT};
use crate::transport::{Transport, TransportError};
use async_trait::async_trait;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tracing::debug;

/// Maximum UDAP datagram we will read. go-udap uses 2048.
const RECV_BUF: usize = 2048;

pub struct UdpTransport {
    sock: UdpSocket,
}

impl UdpTransport {
    /// Binds `0.0.0.0:port` with broadcast and address reuse enabled.
    ///
    /// Port 0 lets the OS choose — used by tests. Production uses
    /// [`crate::protocol::PORT`].
    ///
    /// The local bind stays `0.0.0.0` so limited-broadcast replies arrive.
    ///
    /// # Errors
    /// [`TransportError::Io`] if any socket call fails.
    pub fn bind(port: u16) -> Result<Self, TransportError> {
        Self::build(port, None)
    }

    /// As [`bind`](Self::bind), but constrains outbound packets to `iface`.
    ///
    /// The local bind remains `0.0.0.0`; only egress is pinned, via
    /// `IP_BOUND_IF` on Apple platforms and `SO_BINDTOIFINDEX` on Linux.
    /// The destination is still the limited broadcast — see the module
    /// docs on why a directed broadcast does not reach unconfigured
    /// devices.
    ///
    /// # Errors
    /// [`TransportError::InterfaceBindUnsupported`] on platforms without
    /// the option, or [`TransportError::Io`].
    pub fn bind_on_interface(iface: &NetInterface, port: u16) -> Result<Self, TransportError> {
        Self::build(port, Some(iface))
    }

    fn build(port: u16, iface: Option<&NetInterface>) -> Result<Self, TransportError> {
        let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;

        // Pre-bind. SO_REUSEPORT is what lets --all-interfaces stand up one
        // socket per interface on the same 0.0.0.0:PORT.
        sock.set_reuse_address(true)?;
        #[cfg(unix)]
        sock.set_reuse_port(true)?;

        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port));
        sock.bind(&addr.into())?;

        // Post-bind.
        sock.set_broadcast(true)?;

        if let Some(iface) = iface {
            bind_to_interface(&sock, iface)?;
        }

        // REQUIRED before from_std: a blocking socket compiles and then
        // stalls the runtime on the first read.
        sock.set_nonblocking(true)?;
        let sock = UdpSocket::from_std(sock.into())?;
        debug!(local = ?sock.local_addr().ok(), "UDP transport bound");
        Ok(UdpTransport { sock })
    }

    /// The bound address. Test helper.
    ///
    /// # Errors
    /// [`TransportError::Io`] if the socket has no local address.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.sock.local_addr()?)
    }
}

/// Constrains egress to one interface.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
fn bind_to_interface(sock: &Socket, iface: &NetInterface) -> Result<(), TransportError> {
    let index = std::num::NonZeroU32::new(iface.index)
        .ok_or_else(|| std::io::Error::other(format!("interface {} has index 0", iface.name)))?;
    sock.bind_device_by_index_v4(Some(index))?;
    debug!(interface = %iface.name, index = iface.index, "egress pinned to interface");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android")))]
fn bind_to_interface(_sock: &Socket, _iface: &NetInterface) -> Result<(), TransportError> {
    Err(TransportError::InterfaceBindUnsupported {
        flag: "--bind-interface",
    })
}

#[async_trait]
impl Transport for UdpTransport {
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError> {
        let dst = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, PORT));
        self.sock.send_to(packet, dst).await?;
        Ok(())
    }

    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
        let mut buf = vec![0u8; RECV_BUF];
        loop {
            let (n, src) = tokio::select! {
                () = cancel.cancelled() => return Err(TransportError::Cancelled),
                r = self.sock.recv_from(&mut buf) => r?,
            };
            // Skip the broadcast the kernel looped back to us: we send with
            // the request bit set, devices reply with it clear.
            if protocol::is_request_packet(&buf[..n]) {
                debug!(%src, "skipping our own looped-back request");
                continue;
            }
            return Ok((buf[..n].to_vec(), src.ip().to_string()));
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        // The socket closes when this transport drops; tokio has no
        // explicit close. Matches UDPTransport.Close's contract.
        Ok(())
    }
}
```

Add to `crates/udap/src/transport/mod.rs`:

```rust
pub mod udp;
pub use udp::UdpTransport;
```

- [ ] **Step 6: Run the tests**

```bash
mise exec -- cargo nextest run -p udap
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo fmt --all --check
```

Expected: all pass. If `two_sockets_can_share_a_port` fails, `SO_REUSEPORT` is not landing pre-bind — check the call order, do not weaken the test.

- [ ] **Step 7: Commit**

```bash
git add crates/udap/src/transport/ crates/udap/Cargo.toml Cargo.lock
git commit -S -m "feat(udap): add the real UDP transport

Ports udap/transport.go and the socket_*.go family. socket2 configures
the socket and tokio runs it; set_nonblocking before from_std is the
analogue of go-udap's File()-vs-SyscallConn hazard.

socket_darwin.go and socket_linux.go collapse into one
bind_device_by_index_v4 call, which dispatches to IP_BOUND_IF on Apple
and SO_BINDTOIFINDEX on Linux."
```

---

### Task 3: Client constructors and the global flags

Wires the transport into `Client` and the CLI. **Closes issue #2 (`--retries` unwired).**

**Files:**
- Modify: `crates/udap/src/client.rs`, `crates/udap-cli/src/cli.rs`, `crates/udap-cli/src/lib.rs`, `crates/udap-cli/src/main.rs`, `crates/udap-cli/tests/e2e_discover.rs` (its `Cli` struct literal gains two fields — see Step 4)

**Interfaces:**
- Consumes: `UdpTransport` (Task 2), `interfaces::enumerate` (Task 1), `Client::set_retries` (M2)
- Produces:
  - `Client::with_udp(port: u16) -> Result<Client, ClientError>`
  - `Client::for_interface(name: &str, port: u16) -> Result<Client, ClientError>`
  - `ClientError::{Interface, NoSuchInterface, Bind}` variants
  - `Cli` gains `bind_interface: Option<String>` and `all_interfaces: bool`

**Behaviour:**
- `--bind-interface` and `--all-interfaces` are **mutually exclusive**; combining them exits **1** (usage error), not 2.
- An unknown interface name exits **1** with go-udap's wording: `--bind-interface: "NAME" is not usable (must be up, broadcast-capable, with an IPv4 address)`.
- Validation happens **before** the subcommand runs, matching go-udap's `PersistentPreRunE`.
- `set_retries` is called by the factory. `--retries N` means N *re-transmissions* beyond the initial send.

- [ ] **Step 1: Write the failing tests**

Append to `crates/udap-cli/tests/e2e_discover.rs`:

```rust
use clap::Parser;

#[test]
fn bind_interface_and_all_interfaces_are_mutually_exclusive() {
    let err = Cli::try_parse_from(["udapcfg", "--bind-interface", "en0", "--all-interfaces", "discover"])
        .expect_err("the two flags must conflict");
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn retries_defaults_to_zero_and_parses() {
    let cli = Cli::try_parse_from(["udapcfg", "discover"]).expect("parse");
    assert_eq!(cli.retries, 0);
    let cli = Cli::try_parse_from(["udapcfg", "--retries", "2", "discover"]).expect("parse");
    assert_eq!(cli.retries, 2);
}

#[test]
fn negative_retries_is_rejected() {
    assert!(Cli::try_parse_from(["udapcfg", "--retries", "-1", "discover"]).is_err());
}
```

And in `crates/udap/tests/discovery.rs`, a test proving retries reach the wire:

```rust
// Add to the file's existing imports. `use std::sync::Arc;` is already
// line 1 of discovery.rs -- re-importing it is E0252, and `-D warnings`
// makes that fatal. Only the atomics are new, and they must be at module
// scope because CountingTransport below is a module-level item.
use std::sync::atomic::{AtomicUsize, Ordering};

/// A transport that counts sends and never replies.
///
/// The counter is an `Arc<AtomicUsize>` the test also holds, so the
/// transport itself can be a plain `Box<dyn Transport>` — which is what
/// `Client::new` takes. Boxing an `Arc<dyn Transport>` would give
/// `Box<Arc<dyn Transport>>`, an unrelated type (E0308).
struct CountingTransport {
    sends: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl udap::transport::Transport for CountingTransport {
    async fn send(&self, _packet: &[u8]) -> Result<(), udap::transport::TransportError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn recv(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(Vec<u8>, String), udap::transport::TransportError> {
        cancel.cancelled().await;
        Err(udap::transport::TransportError::Cancelled)
    }
    async fn close(&self) -> Result<(), udap::transport::TransportError> {
        Ok(())
    }
}

#[tokio::test]
async fn retries_n_produces_n_plus_one_sends() {
    let sends = Arc::new(AtomicUsize::new(0));
    let mut client = Client::new(Box::new(CountingTransport {
        sends: Arc::clone(&sends),
    }));
    client.set_retries(2);

    let cancel = CancellationToken::new();
    cancel.cancel();
    let _ = client.discover(&cancel).await;

    assert_eq!(sends.load(Ordering::SeqCst), 3, "2 retries means 3 total sends");
}
```

Note `Client::new` takes `Box<dyn Transport>`, so the transport is boxed
directly and the shared state is the `Arc<AtomicUsize>` counter — not the
transport. `add_retries` is applied via `set_retries` after construction,
matching how the CLI factory will do it in Step 5.

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run --workspace
```

Expected: compilation failure — the flags and constructors do not exist.

- [ ] **Step 3: Add the client constructors**

In `crates/udap/src/client.rs`, extend `ClientError`:

```rust
    #[error("enumerate interfaces: {0}")]
    Interface(#[from] crate::interfaces::InterfaceError),
    #[error("--bind-interface: {name:?} is not usable (must be up, broadcast-capable, with an IPv4 address)")]
    NoSuchInterface { name: String },
    #[error("bind: {0}")]
    Bind(#[from] crate::transport::TransportError),
```

and add:

```rust
impl Client {
    /// A client on a real UDP socket bound to `0.0.0.0:port`.
    ///
    /// # Errors
    /// [`ClientError::Bind`] if the socket cannot be created.
    pub fn with_udp(port: u16) -> Result<Self, ClientError> {
        let transport = crate::transport::UdpTransport::bind(port)?;
        Ok(Self::new(Box::new(transport)))
    }

    /// A client whose egress is pinned to the named interface.
    ///
    /// # Errors
    /// [`ClientError::NoSuchInterface`] if no usable interface has that
    /// name, [`ClientError::Interface`] if enumeration fails, or
    /// [`ClientError::Bind`].
    pub fn for_interface(name: &str, port: u16) -> Result<Self, ClientError> {
        let ifaces = crate::interfaces::enumerate()?;
        let iface = ifaces
            .into_iter()
            .find(|i| i.name == name)
            .ok_or_else(|| ClientError::NoSuchInterface { name: name.to_owned() })?;
        let transport = crate::transport::UdpTransport::bind_on_interface(&iface, port)?;
        Ok(Self::new(Box::new(transport)))
    }
}
```

- [ ] **Step 4: Add the global flags — and fix the struct literal this breaks**

`Cli` does not derive `Default`, and `crates/udap-cli/tests/e2e_discover.rs:15` constructs it with an exhaustive struct literal:

```rust
    let cli = Cli { timeout: ..., verbose: false, retries: 0, command: Command::Discover };
```

Adding two fields makes that a compile error (E0063, missing fields) in a file this task otherwise only appends to. **Update that literal in the same step**, adding `bind_interface: None, all_interfaces: false`, or the whole suite fails to build before any new test runs.

In `crates/udap-cli/src/cli.rs`, add to `Cli`:

```rust
    /// Bind discovery to one network interface
    #[arg(long, global = true, value_name = "NAME", conflicts_with = "all_interfaces")]
    pub bind_interface: Option<String>,

    /// Broadcast on every usable interface (fan-out)
    #[arg(long, global = true)]
    pub all_interfaces: bool,
```

- [ ] **Step 5: Build the real factory**

Replace the placeholder in `crates/udap-cli/src/main.rs`:

```rust
    let retries = cli.retries;
    let bind_interface = cli.bind_interface.clone();
    let all_interfaces = cli.all_interfaces;
    let factory: ClientFactory = Box::new(move || {
        let mut client = if let Some(name) = bind_interface.as_deref() {
            udap::Client::for_interface(name, udap::PORT)?
        } else if all_interfaces {
            udap::Client::for_all_interfaces(udap::PORT)?
        } else {
            udap::Client::with_udp(udap::PORT)?
        };
        client.set_retries(retries);
        Ok(client)
    });
```

> `for_all_interfaces` arrives in Task 4. Until then, have the `all_interfaces` arm return
> `Err(anyhow::anyhow!("--all-interfaces lands with MultiTransport in the next task"))`
> so this task compiles and its tests pass standalone. Replace it in Task 4 and delete this note.

- [ ] **Step 6a: Write the failing test for interface validation**

This behaviour goes through `run()` so it can be tested with captured writers, like every other command. Append to `crates/udap-cli/tests/e2e_discover.rs`:

```rust
#[tokio::test]
async fn unknown_bind_interface_is_a_usage_error() {
    let factory: udap_cli::ClientFactory =
        Box::new(|| Err(anyhow::anyhow!("factory must not be reached")));
    let cli = Cli {
        timeout: GoDuration::from(50_000_000),
        verbose: false,
        retries: 0,
        bind_interface: Some("definitely-not-an-interface0".to_owned()),
        all_interfaces: false,
        command: Command::Discover,
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let e = run(cli, factory, &mut out, &mut err)
        .await
        .expect_err("an unusable interface is an error");

    // go-udap treats this as a usage error, not an operation failure
    // (cli/cli.go:124 returns ExitError{Code: 1}).
    assert_eq!(e.code, 1, "unusable interface must exit 1, not 2");
    assert!(
        e.source.to_string().contains("is not usable"),
        "message must match go-udap: {}",
        e.source
    );
    assert!(out.is_empty(), "stdout stays clean on a usage error");
}
```

- [ ] **Step 6b: Validate inside `run()`, before dispatch**

`Client::for_interface` returns `ClientError::NoSuchInterface`, but that error travels through the factory into `cmd/discover.rs:22`, which hard-codes `CliError { code: 2 }`. Left alone, a bad `--bind-interface` exits **2**, not 1.

go-udap validates in `PersistentPreRunE`, *before* any subcommand runs (`cli/cli.go:123-125`). Do the same, in `crates/udap-cli/src/lib.rs::run`, ahead of the `match cli.command`:

```rust
    // Validate before dispatch, not in the factory: the factory's error is
    // mapped to exit 2 by every subcommand, and go-udap treats an unusable
    // interface as a usage error (cli/cli.go:124).
    if let Some(name) = cli.bind_interface.as_deref() {
        let ifs = udap::interfaces::enumerate().map_err(|e| CliError {
            code: 2,
            source: anyhow::Error::new(e).context("enumerate interfaces"),
        })?;
        if !ifs.iter().any(|i| i.name == name) {
            return Err(CliError {
                code: 1,
                source: anyhow::anyhow!(
                    "--bind-interface: {name:?} is not usable \
                     (must be up, broadcast-capable, with an IPv4 address)"
                ),
            });
        }
    }
```

Note the two codes: an unusable *name* is a usage error (1); a *failure to enumerate* is an operation failure (2). go-udap makes the same split.

Keeping this in `run()` rather than `main()` is deliberate — it is the injected-writer seam the rest of the CLI uses, so the exit code and the message are both testable. A version in `main()` calling `std::io::stderr()` directly would be neither.

- [ ] **Step 6c: Map clap's parse failures to go-udap's codes**

`clap`'s `conflicts_with` produces exit code 2 by default, but go-udap exits **1** on usage errors. In `main.rs`, map clap's parse failure explicitly:

```rust
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // clap writes its own message; go-udap uses exit 1 for usage errors.
            let _ = e.print();
            return ExitCode::from(if e.use_stderr() { 1 } else { 0 });
        }
    };
```

`e.use_stderr()` is false for `--help`/`--version`, which must exit **0**.

- [ ] **Step 7: Run, then verify exit codes by hand**

```bash
mise exec -- cargo nextest run --workspace
mise exec -- cargo clippy --all-targets --all-features
mise exec -- cargo run -q -p udap-cli -- --help >/dev/null; echo "help exit=$?"          # expect 0
mise exec -- cargo run -q -p udap-cli -- --bind-interface en0 --all-interfaces discover; echo "conflict exit=$?"   # expect 1
mise exec -- cargo run -q -p udap-cli -- --bind-interface nope0 discover; echo "unknown iface exit=$?"             # expect 1
```

Compare each against `/tmp/go-udap` with the same arguments.

- [ ] **Step 8: Commit**

```bash
git add crates/udap/src/client.rs crates/udap-cli/src/
git commit -S -m "feat: wire the UDP transport into the CLI

Adds Client::with_udp and Client::for_interface, the --bind-interface
and --all-interfaces global flags, and the retry count the factory now
applies.

Closes #2: --retries parsed but never reached set_retries, so the flag
did nothing. A counting transport test pins N retries to N+1 sends."
```

---

### Task 4: `MultiTransport` and `--all-interfaces`

Port of `udap/multi_transport.go`.

**Files:**
- Create: `crates/udap/src/transport/multi.rs`
- Modify: `crates/udap/src/transport/mod.rs`, `crates/udap/src/client.rs`, `crates/udap-cli/src/main.rs`

**Interfaces:**
- Consumes: `Transport`, `UdpTransport`, `interfaces::enumerate`
- Produces:
  - `pub struct MultiTransport`
  - `MultiTransport::new(children: Vec<Box<dyn Transport>>) -> MultiTransport`
  - `impl Transport for MultiTransport`
  - `Client::for_all_interfaces(port: u16) -> Result<Client, ClientError>`

**Behaviour, from the Go:**
- `send` fans out to every child. Succeeds if **any** child succeeded; returns an aggregated error only if **all** failed. Per-child failures are logged at warn.
- `recv` merges children through one spawned task each into an `mpsc` channel — the same structure that fixed M2's lost-wakeup race.
- Children that fail to bind are **skipped with a warning**; if none bind, error.
- An empty interface list errors with `no usable interfaces found`.

- [ ] **Step 1: Write the failing tests**

`crates/udap/src/transport/multi.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Chatty {
        sends: Arc<AtomicUsize>,
        reply: Vec<u8>,
        fail_send: bool,
    }

    #[async_trait]
    impl Transport for Chatty {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.fail_send {
                return Err(TransportError::Io(std::io::Error::other("nope")));
            }
            Ok(())
        }
        async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
            if self.reply.is_empty() {
                cancel.cancelled().await;
                return Err(TransportError::Cancelled);
            }
            Ok((self.reply.clone(), "10.0.0.1".to_owned()))
        }
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    fn child(sends: &Arc<AtomicUsize>, reply: &[u8], fail_send: bool) -> Box<dyn Transport> {
        Box::new(Chatty {
            sends: Arc::clone(sends),
            reply: reply.to_vec(),
            fail_send,
        })
    }

    #[tokio::test]
    async fn send_fans_out_to_every_child() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![
            child(&sends, b"", false),
            child(&sends, b"", false),
            child(&sends, b"", false),
        ]);
        m.send(b"x").await.expect("send");
        assert_eq!(sends.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn send_succeeds_when_any_child_succeeds() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", false)]);
        assert!(m.send(b"x").await.is_ok(), "one success is enough");
    }

    #[tokio::test]
    async fn send_fails_only_when_every_child_fails() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", true)]);
        assert!(m.send(b"x").await.is_err(), "all failed, so send must fail");
    }

    #[tokio::test]
    async fn recv_merges_replies_from_children() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"reply-a", false)]);
        let cancel = CancellationToken::new();
        let (pkt, src) = m.recv(&cancel).await.expect("a merged reply");
        assert_eq!(pkt, b"reply-a");
        assert_eq!(src, "10.0.0.1");
    }

    #[tokio::test]
    async fn recv_returns_cancelled_when_the_token_fires() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", false)]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = m.recv(&cancel).await.expect_err("must not block");
        assert!(matches!(err, TransportError::Cancelled));
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
mise exec -- cargo nextest run -p udap multi
```

- [ ] **Step 3: Implement**

Prepend the module docs and the parts both designs share:

```rust
//! Composes several transports: send fans out, recv merges.
//!
//! Used by `--all-interfaces`, one `UdpTransport` per usable interface.
//! `Client` cannot tell the difference — both satisfy `Transport`.

use crate::transport::{Transport, TransportError};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::warn;
```

**Then choose how `recv` merges its children, and add only that design's state.** The children sit behind `&self`, so they cannot be moved into `tokio::spawn`ed tasks; that constraint is what forces the decision.

| | Option 1 — spawn per child | Option 2 — `select_all` |
| --- | --- | --- |
| Fields | `Vec<Arc<dyn Transport>>`, `mpsc` tx/rx, a `Once`, a stop token | `Vec<Box<dyn Transport>>` only |
| `recv` | drain the merged channel | `select_all` over one recv future per child |
| Extra dep | none | `futures` |
| Notes | structurally matches `multi_transport.go` | smaller; no lost-wakeup surface at all |

Whichever you take, add **only** that option's fields. This repo denies warnings, so an unused `mpsc` channel or an unused `Once` left over from the other design is a build failure, not a lint nit — and `use tokio::sync::{mpsc, Mutex};` belongs in the file only if you took Option 1.

The tests above constrain behaviour, not structure: both designs must pass them unchanged. **Say in your report which you took and why.** If you take Option 1, note that `Vec<Arc<dyn Transport>>` changes `new`'s signature and the tests' `child()` helper accordingly.

Then `send`, `recv` and `close` per the behaviour list above, and:

```rust
impl Client {
    /// A client fanning out across every usable interface.
    ///
    /// Interfaces that fail to bind are skipped with a warning.
    ///
    /// # Errors
    /// [`ClientError::NoUsableInterfaces`] if enumeration finds none or
    /// none bind successfully.
    pub fn for_all_interfaces(port: u16) -> Result<Self, ClientError> {
        let ifaces = crate::interfaces::enumerate()?;
        if ifaces.is_empty() {
            return Err(ClientError::NoUsableInterfaces);
        }
        let mut children: Vec<Box<dyn Transport>> = Vec::new();
        for iface in &ifaces {
            match crate::transport::UdpTransport::bind_on_interface(iface, port) {
                Ok(t) => children.push(Box::new(t)),
                Err(e) => warn!(interface = %iface.name, error = %e, "skipping interface (bind failed)"),
            }
        }
        if children.is_empty() {
            return Err(ClientError::NoUsableInterfaces);
        }
        Ok(Self::new(Box::new(crate::transport::MultiTransport::new(children))))
    }
}
```

Add `ClientError::NoUsableInterfaces` with message `no usable interfaces found`.

- [ ] **Step 4: Replace the Task 3 placeholder**

In `main.rs`, swap the `all_interfaces` arm's temporary error for `udap::Client::for_all_interfaces(udap::PORT)?` and delete the note comment.

- [ ] **Step 5: Run and commit**

```bash
mise exec -- cargo nextest run --workspace
mise exec -- cargo clippy --all-targets --all-features
git add crates/udap/src/transport/ crates/udap/src/client.rs crates/udap-cli/src/main.rs
git commit -S -m "feat(udap): add MultiTransport and --all-interfaces

Ports udap/multi_transport.go. Send fans out and succeeds if any child
succeeded; recv merges children. Interfaces that fail to bind are
skipped with a warning rather than failing the run."
```

---

### Task 5: The `interfaces` subcommand

Port of `cli/interfaces.go` and `formatInterfacesTable`.

**Files:**
- Create: `crates/udap-cli/src/cmd/interfaces.rs`, `crates/udap-cli/src/output.rs`
- Modify: `crates/udap-cli/src/cmd/mod.rs`, `crates/udap-cli/src/cli.rs`, `crates/udap-cli/src/lib.rs`

**Interfaces:**
- Consumes: `udap::interfaces::enumerate`, `NetInterface` (Task 1)
- Produces: `Command::Interfaces` variant; `output::format_interfaces_table(w, &[NetInterface])`

**Output contract — copy the widths exactly.** go-udap's `cli/output.go:91-94`:

```go
fmt.Fprintln(w, "NAME            INDEX  ADDRESS            BROADCAST")
fmt.Fprintf(w, "%-15s %-5d  %-18s %s\n", ni.Name, ni.Index, ni.Addr, ni.Broadcast)
```

In Rust: `{:<15} {:<5}  {:<18} {}`. Note the **two** spaces after the index field and **one** after the others — that is not a typo in the Go, it is the column alignment.

Empty result: `no usable interfaces found` on **stderr**, exit **0**.

- [ ] **Step 1: Write the failing tests**

`crates/udap-cli/src/output.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use udap::NetInterface;

    fn sample() -> Vec<NetInterface> {
        vec![NetInterface {
            name: "en0".to_owned(),
            index: 4,
            addr: Ipv4Addr::new(192, 168, 1, 50),
            broadcast: Ipv4Addr::new(192, 168, 1, 255),
        }]
    }

    #[test]
    fn header_matches_go_udap_byte_for_byte() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &sample());
        let s = String::from_utf8(out).expect("utf8");
        let header = s.lines().next().expect("a header line");
        assert_eq!(header, "NAME            INDEX  ADDRESS            BROADCAST");
    }

    #[test]
    fn row_column_widths_match_go_udap() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &sample());
        let s = String::from_utf8(out).expect("utf8");
        let row = s.lines().nth(1).expect("a data row");
        // %-15s + space + %-5d + two spaces + %-18s + %s
        assert_eq!(row, "en0             4      192.168.1.50       192.168.1.255");
    }

    #[test]
    fn empty_input_writes_nothing() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &[]);
        assert!(out.is_empty(), "no header for an empty list");
    }
}
```

> **Derive the expected row from the Go, do not trust mine.** Before implementing, run:
> `/tmp/go-udap interfaces | cat -A | head -3` (or `sed -n l` on macOS, where `cat -A` is unavailable)
> and compare the exact spacing against the literals above. If they differ, the Go wins — fix the test and say so in your report. This is the same discipline that caught `format_duration` in M2.

- [ ] **Step 2: Run to verify failure, then implement**

```rust
//! Formatted output for the CLI.

use std::io::Write;
use udap::NetInterface;

/// Writes the `interfaces` table. Matches go-udap's column widths exactly.
pub fn format_interfaces_table(w: &mut dyn Write, ifs: &[NetInterface]) {
    if ifs.is_empty() {
        return;
    }
    let _ = writeln!(w, "NAME            INDEX  ADDRESS            BROADCAST");
    for ni in ifs {
        let _ = writeln!(
            w,
            "{:<15} {:<5}  {:<18} {}",
            ni.name, ni.index, ni.addr, ni.broadcast
        );
    }
}
```

- [ ] **Step 3: Add the subcommand**

`crates/udap-cli/src/cmd/interfaces.rs`:

```rust
//! The `interfaces` subcommand.

use crate::{output, CliError};
use std::io::Write;

/// Lists interfaces usable for discovery.
///
/// Finding none is not an error — a note goes to stderr and the exit
/// code stays 0, matching `discover`.
///
/// # Errors
/// [`CliError`] with code 2 if enumeration fails.
pub fn run(stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), CliError> {
    let ifs = udap::interfaces::enumerate().map_err(|e| CliError {
        code: 2,
        source: anyhow::Error::new(e),
    })?;
    if ifs.is_empty() {
        let _ = writeln!(stderr, "no usable interfaces found");
        return Ok(());
    }
    output::format_interfaces_table(stdout, &ifs);
    Ok(())
}
```

Add `Interfaces` to `Command` with go-udap's help text (`cli/interfaces.go`), and dispatch it in `lib.rs::run`.

- [ ] **Step 4: Verify against go-udap byte-for-byte**

```bash
diff <(/tmp/go-udap interfaces) <(mise exec -- cargo run -q -p udap-cli -- interfaces) \
  && echo "IDENTICAL"
```

Expected: `IDENTICAL`. Any difference is a defect — the table is part of the fidelity contract.

- [ ] **Step 5: Commit**

```bash
git add crates/udap-cli/src/
git commit -S -m "feat(cli): add the interfaces subcommand

Ports cli/interfaces.go and formatInterfacesTable. Output verified
byte-identical to go-udap interfaces on the same host."
```

---

### Task 6: Real-hardware verification

**No code.** This is the milestone's acceptance test, and it resolves two spec open questions that cannot be settled by reading.

**Requires:** a Squeezebox device in setup mode (front light flashing red; hold the front button 3–6 seconds to enter it), on the same L2 segment.

- [ ] **Step 1: Discover against real hardware, both binaries**

```bash
/tmp/go-udap discover
mise exec -- cargo run -q -p udap-cli -- discover
```

The MAC lists must match. Record both outputs in the report.

- [ ] **Step 2: Confirm the loopback filter works on a real socket**

Run `udapcfg discover` with `--verbose` and confirm the debug log shows our own broadcast being skipped, and that **no device with MAC `00:00:00:00:00:00`** appears. That phantom is what happens if `is_request_packet` is not consulted — the failure this guard exists to prevent.

- [ ] **Step 3: `--bind-interface` on the interface that has the device**

```bash
mise exec -- cargo run -q -p udap-cli -- --bind-interface <name> discover
```

Then on an interface that does **not** reach the device, and confirm it finds nothing rather than erroring.

- [ ] **Step 4: `--all-interfaces`**

```bash
mise exec -- cargo run -q -p udap-cli -- --all-interfaces discover
```

Must find the device exactly once, not once per interface. If duplicates appear, the `BTreeMap<Mac, Device>` dedup is not doing its job — investigate.

- [ ] **Step 5: Resolve OQ-2 — `SO_BINDTOIFINDEX` privileges**

**On Linux, as an unprivileged user:**

```bash
mise exec -- cargo run -q -p udap-cli -- --bind-interface <name> discover; echo "exit=$?"
```

go-udap's error text claims `SO_BINDTODEVICE` "may require CAP_NET_RAW". `SO_BINDTOIFINDEX` is a different option and may not. Record which it is. If it needs privileges, the error message must say so — that is part of the fidelity contract. If it does not, note that the Rust port is *less* restrictive than the Go here and add it to the accepted-deltas table.

- [ ] **Step 6: Confirm the Linux kernel floor**

`SO_BINDTOIFINDEX` needs **kernel 5.7+**. Record the kernel you tested on (`uname -r`). If you have access to anything older, test there too — the spec's accepted-deltas table names this as a known narrowing versus go-udap's `SO_BINDTODEVICE`.

- [ ] **Step 7: Update the spec**

Amend `docs/specs/2026-09-08-rust-port-spec.md`:
- Mark **OQ-1** and **OQ-2** resolved with what you measured.
- Add any behavioural delta you found to the accepted-deltas table.
- If M3 revealed the milestone descriptions are wrong (as M2 did for ADR-3's ownership prediction), fix them.

```bash
git add docs/specs/
git commit -S -m "docs(spec): resolve OQ-1 and OQ-2 against real hardware"
```

---

## Verification checklist

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --all-targets --all-features` clean, zero warnings
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo deny check` clean
- [ ] `udapcfg interfaces` byte-identical to `go-udap interfaces`
- [ ] `udapcfg discover` finds the same devices as `go-udap discover` on real hardware
- [ ] `--bind-interface` and `--all-interfaces` both work against a real device
- [ ] Exit codes match go-udap: 0 success, 1 usage, 2 operation failure
- [ ] No `#[allow(...)]` anywhere; any new `#[expect(...)]` carries a `reason`
- [ ] Issue #2 closed
- [ ] OQ-1 and OQ-2 marked resolved in the spec

## Pushing

Each push runs the lastlight gate, which requires an independent review recorded for that **exact** SHA. Batch the work and push once — every new commit invalidates the record.

```bash
~/code/github.com/yo61/claude-plugin-lastlight-pr-gate/scripts/lastlight-review-run.sh
~/code/github.com/yo61/claude-plugin-lastlight-pr-gate/scripts/lastlight-review-record.sh
git push -u origin <branch>
```

Note the gate's own error message points at a plugin-cache path that does not contain the scripts; use the source-repo path above. Findings must be fixed, or dismissed with a written reason of at least 25 characters in `.lastlight/pr-review/dismissed.json`.

The repo now requires `check`, `lint` and `Conventional Commits` to pass plus one approving review, with **no bypass actor**. Commit messages must be Conventional Commits or the PR cannot merge.

## What M3 leaves out

- **M4** — `get_data`, `set_data`, `reset`, `get_ip`, `get_uuid`; the read-modify-write in `set`; the ADR-3 ownership work, which M2 showed lands here rather than in `client.rs`.
- **Issue #3** (`from_utf8_lossy` at two sites) — decide before M4 builds `read`/`get`/`set` on `String`.
- **Issues #4, #5** — the untested `u16` encode branch, and the unused `insta`/`rstest`.
