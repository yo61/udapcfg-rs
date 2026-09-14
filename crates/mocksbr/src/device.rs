//! One virtual Squeezebox Receiver.

use std::net::Ipv4Addr;
use udap::Mac;

/// Per-device configuration. This is the M2 subset; fault-injection
/// knobs arrive with the full mocksbr port.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub mac: Mac,
    /// Reported as TLV 0x02 (`device_name`).
    pub name: String,
    /// Reported as TLV 0x03 (`device_type`).
    pub model: String,
    /// Reported as TLV 0x0b (`device_id`). "07" is a Receiver.
    pub device_id: String,
    /// Reported as TLV 0x09 (`firmware_rev`).
    pub firmware: String,
    /// Reported as TLV 0x0a (`hardware_rev`).
    pub hardware: String,
    /// Reported as TLV 0x0c (`device_status`).
    pub state: String,
    /// Reported by `get_ip` as TLV 0x05.
    pub ip: Ipv4Addr,
    /// Reported by `get_ip` as TLV 0x06.
    pub subnet_mask: Ipv4Addr,
    /// Reported by `get_ip` as TLV 0x07.
    pub gateway: Ipv4Addr,
    /// Reported by `get_uuid` as TLV 0x0d. Sixteen bytes.
    pub uuid: [u8; 16],
    /// Fault injection: when set, every directed request is answered
    /// with UCP 0x0007 instead of the operation's own reply.
    ///
    /// `Some(text)` carries an error-message TLV; `Some("")` answers with
    /// no TLV at all, which is a distinct path in every operation that
    /// handles an error reply.
    pub error_reply: Option<String>,
}

impl DeviceConfig {
    /// Builds a device with go-udap's mocksbr defaults.
    #[must_use]
    pub fn default_with_mac(mac: Mac) -> Self {
        DeviceConfig {
            mac,
            name: "Mock SBR".to_owned(),
            model: "squeezebox".to_owned(),
            device_id: "07".to_owned(),
            firmware: "77".to_owned(),
            hardware: "0005".to_owned(),
            state: "wait_slimserver".to_owned(),
            // A device in setup mode has no lease, so every address is
            // unspecified — which `NetworkConfig` renders as "-".
            ip: Ipv4Addr::UNSPECIFIED,
            subnet_mask: Ipv4Addr::UNSPECIFIED,
            gateway: Ipv4Addr::UNSPECIFIED,
            // Fixed rather than random, so tests are reproducible.
            uuid: [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
                0x32, 0x10,
            ],
            error_reply: None,
        }
    }
}
