//! One virtual Squeezebox Receiver.

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
        }
    }
}
