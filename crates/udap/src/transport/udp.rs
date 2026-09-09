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
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
fn bind_to_interface(sock: &Socket, iface: &NetInterface) -> Result<(), TransportError> {
    let index = std::num::NonZeroU32::new(iface.index)
        .ok_or_else(|| std::io::Error::other(format!("interface {} has index 0", iface.name)))?;
    sock.bind_device_by_index_v4(Some(index))?;
    debug!(interface = %iface.name, index = iface.index, "egress pinned to interface");
    Ok(())
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
)))]
fn bind_to_interface(_sock: &Socket, _iface: &NetInterface) -> Result<(), TransportError> {
    Err(TransportError::InterfaceBindUnsupported)
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
        assert!(
            addr.ip().is_unspecified(),
            "must bind 0.0.0.0 to hear limited broadcast"
        );
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
        use crate::Mac;
        use crate::protocol::{
            ADDR_TYPE_ETH, FLAG_REQUEST, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method,
        };

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
        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender");
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

        let err = t
            .recv(&cancel)
            .await
            .expect_err("the request must be skipped");
        assert!(matches!(err, TransportError::Cancelled));
    }

    #[tokio::test]
    async fn recv_returns_a_reply_packet() {
        use crate::Mac;
        use crate::protocol::{ADDR_TYPE_ETH, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};

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

        let sender = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("sender");
        sender
            .send_to(&reply, ("127.0.0.1", port))
            .await
            .expect("send");

        let cancel = CancellationToken::new();
        let (got, src) = t.recv(&cancel).await.expect("a reply must come through");
        assert_eq!(got, reply.to_vec());
        assert_eq!(src, "127.0.0.1");
    }
}
