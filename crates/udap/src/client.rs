//! The UDAP client: owns a transport and the map of discovered devices.

use crate::device::{Device, combine_model};
use crate::protocol::{ADDR_TYPE_ETH, FLAG_REQUEST, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
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
