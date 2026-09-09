//! Discovered device metadata.

use crate::Mac;

/// A device found by discovery. Fields come from the response TLVs.
///
/// `name`, `firmware`, `hardware_rev`, and `state` are `Vec<u8>`, not
/// `String`: Go's `string` is an arbitrary byte container, and these
/// values are copied straight from device-supplied TLVs with no encoding
/// guarantee (802.11 SSIDs, for instance, are not required to be valid
/// UTF-8). Rendering lossily is a display-boundary concern, not a
/// storage one — see [`crate::hex::encode`] callers and any future CLI
/// formatter for where that conversion belongs.
///
/// `ip`, `model`, and `uuid` stay `String` deliberately: `ip` is
/// formatted by us from a `SocketAddr`, `uuid` is hex-encoded from TLV
/// 0x0d before storage (so already ASCII), and `model` is synthesized
/// text from [`combine_model`] (either a fixed product-table name or a
/// formatted fallback) — never raw device bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    pub mac: Mac,
    /// Source address the reply arrived from.
    pub ip: String,
    /// TLV 0x02 `device_name`.
    pub name: Vec<u8>,
    /// Derived from TLV 0x03 `device_type` and TLV 0x0b `device_id`.
    pub model: String,
    /// TLV 0x09 `firmware_rev`.
    pub firmware: Vec<u8>,
    /// TLV 0x0a `hardware_rev`.
    pub hardware_rev: Vec<u8>,
    /// TLV 0x0d uuid, hex-encoded.
    pub uuid: String,
    /// TLV 0x0c `device_status`.
    pub state: Vec<u8>,
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
///
/// `device_type` and `device_id` are raw TLV bytes (not guaranteed
/// UTF-8), but the result is always our own synthesized text: a known
/// product name from the fixed table above, or a formatted fallback.
/// Lossy conversion here only affects a lookup key and a display
/// fallback — it never round-trips back to the device, so it does not
/// carry the fidelity risk `Device::name` etc. do.
#[must_use]
pub fn combine_model(device_type: &[u8], device_id: &[u8]) -> String {
    let device_id = String::from_utf8_lossy(device_id);
    if let Some((_, product)) = PRODUCT_BY_ID.iter().find(|(id, _)| *id == device_id) {
        return (*product).to_owned();
    }
    let device_type = String::from_utf8_lossy(device_type);
    match (device_type.is_empty(), device_id.is_empty()) {
        (false, false) => format!("{device_type} (id={device_id})"),
        (false, true) => device_type.into_owned(),
        _ => String::new(),
    }
}
