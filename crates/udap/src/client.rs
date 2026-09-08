//! The UDAP client: owns a transport and the map of discovered devices.

use crate::device::{Device, combine_model};
use crate::protocol::{
    ADDR_TYPE_ETH, FLAG_REQUEST, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, is_request_packet, method,
};
use crate::transport::{Transport, TransportError};
use crate::{Mac, tlv};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("send discovery: {0}")]
    Send(#[source] TransportError),
    #[error("recv during discovery: {0}")]
    Recv(#[source] TransportError),
    #[error("enumerate interfaces: {0}")]
    Interface(#[from] crate::interfaces::InterfaceError),
    #[error(
        "--bind-interface: {name:?} is not usable (must be up, broadcast-capable, with an IPv4 address)"
    )]
    NoSuchInterface { name: String },
    #[error("no usable interfaces found")]
    NoUsableInterfaces,
    #[error("failed to bind on any usable interface")]
    NoInterfaceBound,
    #[error("bind: {0}")]
    Bind(#[from] crate::transport::TransportError),
}

/// Discovery-response TLV codes, per `Net::UDAP` `Constant.pm`.
mod tlv_code {
    pub const DEVICE_NAME: u8 = 0x02;
    pub const DEVICE_TYPE: u8 = 0x03;
    pub const FIRMWARE_REV: u8 = 0x09;
    pub const HARDWARE_REV: u8 = 0x0a;
    pub const DEVICE_ID: u8 = 0x0b;
    pub const DEVICE_STATUS: u8 = 0x0c;
    pub const UUID: u8 = 0x0d;
}

pub struct Client {
    transport: Box<dyn Transport>,
    /// Keyed by `Mac` directly. Go keys by the string form and its
    /// `recordDevice` comment explains the compromise; `Mac` is
    /// `Copy + Eq + Hash`, so here it costs nothing.
    devices: BTreeMap<Mac, Device>,
    sequence: u16,
    retries: usize,
}

impl Client {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Client {
            transport,
            devices: BTreeMap::new(),
            sequence: 0,
            retries: 0,
        }
    }

    /// Sets the number of re-transmissions beyond the initial send.
    /// `n` of 2 means three total sends.
    pub fn set_retries(&mut self, n: usize) {
        self.retries = n;
    }

    /// The configured retry count: re-transmissions beyond each initial
    /// send. Exposed so callers (and tests) can confirm `set_retries` was
    /// actually applied, rather than inferring it from send counts.
    #[must_use]
    pub fn retries(&self) -> usize {
        self.retries
    }

    /// Every discovered device, ordered by MAC.
    #[must_use]
    pub fn devices(&self) -> Vec<&Device> {
        self.devices.values().collect()
    }

    /// Releases the transport.
    ///
    /// # Errors
    /// Propagates the transport's close error.
    pub async fn close(&self) -> Result<(), TransportError> {
        self.transport.close().await
    }

    /// Builds a header with the next sequence number.
    fn next_packet(&mut self, dst: Mac, ucp_method: u16, broadcast: bool) -> Packet {
        self.sequence = self.sequence.wrapping_add(1);
        Packet {
            dst_broadcast: u8::from(broadcast),
            dst_type: ADDR_TYPE_ETH,
            dst_address: dst,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::ZERO,
            sequence: self.sequence,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: FLAG_REQUEST,
            uap_class: UAP_CLASS_UCP,
            ucp_method,
        }
    }

    /// Sends `packet`, retransmitting `self.retries` more times.
    ///
    /// UDP send is fire-and-forget: succeeds if any attempt succeeded,
    /// returns the first error only if every attempt failed. No delay
    /// between sends, matching squeezeplay's triple-send.
    async fn send_retried(&self, packet: &[u8]) -> Result<(), TransportError> {
        let mut first_err = None;
        let mut successes = 0usize;
        for _ in 0..=self.retries {
            match self.transport.send(packet).await {
                Ok(()) => successes += 1,
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        match (successes, first_err) {
            (0, Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Broadcasts an advanced-discovery request and collects replies
    /// until `cancel` fires.
    ///
    /// A cancelled receive ends discovery **successfully** — that is the
    /// normal timeout path, matching go-udap.
    ///
    /// # Errors
    /// [`ClientError::Send`] if every send attempt failed, or
    /// [`ClientError::Recv`] on a non-cancellation transport error.
    pub async fn discover(&mut self, cancel: &CancellationToken) -> Result<(), ClientError> {
        info!(method = "0x0009", "starting UDAP discovery");
        let packet = self
            .next_packet(Mac::ZERO, method::ADV_DISC, true)
            .to_bytes();
        self.send_retried(&packet)
            .await
            .map_err(ClientError::Send)?;

        loop {
            match self.transport.recv(cancel).await {
                Ok((reply, src)) => self.handle_discovery_reply(&reply, &src),
                Err(TransportError::Cancelled) => return Ok(()),
                Err(e) => return Err(ClientError::Recv(e)),
            }
        }
    }

    fn handle_discovery_reply(&mut self, bytes: &[u8], src: &str) {
        // We broadcast with the request bit set and the kernel loops that
        // packet straight back to us on a real socket. go-udap drops it in
        // the capture path (udap/transport.go:105) and M3's UDP transport
        // will do the same, but the guard belongs here too: the invariant
        // is that the client records devices rather than its own echoes,
        // and it should not depend on which transport sits underneath.
        //
        // Without it the echo parses cleanly — Ethernet source type, empty
        // payload — and lands as a device with MAC 00:00:00:00:00:00 named
        // "Squeezebox Device".
        if is_request_packet(bytes) {
            debug!(src, "ignoring our own looped-back request");
            return;
        }
        let (packet, payload) = match Packet::from_bytes(bytes) {
            Ok(parsed) => parsed,
            Err(e) => {
                warn!(src, error = %e, "failed to parse discovery reply");
                return;
            }
        };
        // Real devices reply with Ethernet addressing. AddrTypeUDP is a
        // wire-spec constant no observed device uses, and a pseudo-MAC
        // would not be usable by any downstream operation.
        if packet.src_type != ADDR_TYPE_ETH {
            warn!(
                src,
                src_type = format!("0x{:02x}", packet.src_type),
                "ignoring discovery reply with non-Ethernet source type"
            );
            return;
        }
        let device = parse_discovery_response(payload, src, &packet);
        info!(mac = %device.mac, name = %device.name, ip = %device.ip, "found device");
        self.devices.insert(device.mac, device);
    }
}

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
        let iface = ifaces.into_iter().find(|i| i.name == name).ok_or_else(|| {
            ClientError::NoSuchInterface {
                name: name.to_owned(),
            }
        })?;
        let transport = crate::transport::UdpTransport::bind_on_interface(&iface, port)?;
        Ok(Self::new(Box::new(transport)))
    }

    /// A client fanning out across every usable interface.
    ///
    /// Interfaces that fail to bind are skipped with a warning.
    ///
    /// # Errors
    /// [`ClientError::NoUsableInterfaces`] if enumeration finds none, or
    /// [`ClientError::NoInterfaceBound`] if some exist but none bind —
    /// matching go-udap's two distinct messages (`client.go:437,451`).
    pub fn for_all_interfaces(port: u16) -> Result<Self, ClientError> {
        let ifaces = crate::interfaces::enumerate()?;
        if ifaces.is_empty() {
            return Err(ClientError::NoUsableInterfaces);
        }
        let mut children: Vec<Box<dyn Transport>> = Vec::new();
        for iface in &ifaces {
            match crate::transport::UdpTransport::bind_on_interface(iface, port) {
                Ok(t) => children.push(Box::new(t)),
                Err(e) => {
                    warn!(interface = %iface.name, error = %e, "skipping interface (bind failed)");
                }
            }
        }
        if children.is_empty() {
            return Err(ClientError::NoInterfaceBound);
        }
        Ok(Self::new(Box::new(crate::transport::MultiTransport::new(
            children,
        ))))
    }
}

/// Builds a `Device` from a discovery response payload.
fn parse_discovery_response(payload: &[u8], src: &str, packet: &Packet) -> Device {
    let mut device = Device {
        mac: packet.src_address,
        ip: src.to_owned(),
        ..Device::default()
    };
    let mut device_type = String::new();
    let mut device_id = String::new();

    for entry in tlv::decode(payload) {
        let text = String::from_utf8_lossy(entry.value).into_owned();
        match entry.tag {
            tlv_code::DEVICE_NAME => device.name = text,
            tlv_code::DEVICE_TYPE => device_type = text,
            tlv_code::FIRMWARE_REV => device.firmware = text,
            tlv_code::DEVICE_ID => device_id = text,
            tlv_code::DEVICE_STATUS => device.state = text,
            tlv_code::HARDWARE_REV => device.hardware_rev = text,
            tlv_code::UUID => device.uuid = hex_encode(entry.value),
            tag => debug!(
                tag = format!("0x{tag:02x}"),
                len = entry.value.len(),
                "unknown discovery TLV"
            ),
        }
    }

    device.model = combine_model(&device_type, &device_id);
    if device.name.is_empty() {
        "Squeezebox Device".clone_into(&mut device.name);
    }
    device
}

fn hex_encode(value: &[u8]) -> String {
    let mut s = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write;
        let _ = write!(s, "{byte:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Hands back exactly one packet — a request-flagged broadcast, which
    /// is what a real socket loops back to the sender — then reports
    /// cancellation so `discover` returns the way a timeout would.
    ///
    /// `mocksbr` cannot stand in here: its replies clear the request bit
    /// (`responses.rs:20`), which is precisely the case being excluded.
    #[derive(Default)]
    struct LoopsBackOwnRequest {
        yielded: AtomicBool,
    }

    #[async_trait::async_trait]
    impl Transport for LoopsBackOwnRequest {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            Ok(())
        }

        async fn recv(
            &self,
            _cancel: &CancellationToken,
        ) -> Result<(Vec<u8>, String), TransportError> {
            if self.yielded.swap(true, Ordering::SeqCst) {
                return Err(TransportError::Cancelled);
            }
            // Byte-for-byte what `next_packet` builds for discovery.
            let echo = Packet {
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
            };
            Ok((echo.to_bytes().to_vec(), "192.0.2.1".to_owned()))
        }

        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn ignores_our_own_looped_back_broadcast() {
        let mut client = Client::new(Box::new(LoopsBackOwnRequest::default()));

        client
            .discover(&CancellationToken::new())
            .await
            .expect("a cancelled receive ends discovery cleanly");

        assert!(
            client.devices().is_empty(),
            "our own broadcast was recorded as a device"
        );
    }
}
