//! Discovered device metadata.

use crate::Mac;

/// A device found by discovery. Fields come from the response TLVs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    pub mac: Mac,
    /// Source address the reply arrived from.
    pub ip: String,
    /// TLV 0x02 `device_name`.
    pub name: String,
    /// Derived from TLV 0x03 `device_type` and TLV 0x0b `device_id`.
    pub model: String,
    /// TLV 0x09 `firmware_rev`.
    pub firmware: String,
    /// TLV 0x0a `hardware_rev`.
    pub hardware_rev: String,
    /// TLV 0x0d uuid, hex-encoded.
    pub uuid: String,
    /// TLV 0x0c `device_status`.
    pub state: String,
}

/// Maps a `device_id` (TLV 0x0b, a 2-character ASCII hex string) to its
/// product name. Source: squeezeplay device tables. Only "07"
/// (Receiver) has been verified against real hardware.
const PRODUCT_BY_ID: [(&str, &str); 10] = [
    ("02", "Squeezebox 2"),
    ("03", "Squeezebox 3"),
    ("04", "Transporter"),
    ("05", "SoftSqueeze"),
    ("06", "Squeezebox Boom"),
    ("07", "Squeezebox Receiver"),
    ("08", "Squeezebox Touch"),
    ("09", "Squeezebox Radio"),
    ("0a", "Squeezebox Controller"),
    ("0b", "Squeezeslave"),
];

/// Renders a friendly model string, falling back gracefully.
#[must_use]
pub fn combine_model(device_type: &str, device_id: &str) -> String {
    if let Some((_, product)) = PRODUCT_BY_ID.iter().find(|(id, _)| *id == device_id) {
        return (*product).to_owned();
    }
    match (device_type.is_empty(), device_id.is_empty()) {
        (false, false) => format!("{device_type} (id={device_id})"),
        (false, true) => device_type.to_owned(),
        _ => String::new(),
    }
}
