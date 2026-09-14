//! Reply builders. Layout and TLV order match go-udap's mocksbr so the
//! committed wire captures stay valid.

use crate::device::DeviceConfig;
use crate::state::DeviceState;
use udap::protocol::{ADDR_TYPE_ETH, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP};
use udap::tlv;

/// Builds a reply header: addresses swapped, our MAC as source, the
/// request's sequence echoed, request bit cleared.
fn build_header(request: &Packet, cfg: &DeviceConfig, method: u16) -> Packet {
    Packet {
        dst_broadcast: 0,
        dst_type: ADDR_TYPE_ETH,
        dst_address: request.src_address,
        src_broadcast: 0,
        src_type: ADDR_TYPE_ETH,
        src_address: cfg.mac,
        sequence: request.sequence,
        udap_type: UDAP_TYPE_UCP,
        ucp_flags: 0x00,
        uap_class: UAP_CLASS_UCP,
        ucp_method: method,
    }
}

/// Builds a discovery response: header plus TLVs in go-udap's order —
/// state, `device_id`, `hardware_rev`, `firmware_rev`, `device_type`, `device_name`.
#[must_use]
pub fn discovery_response(request: &Packet, cfg: &DeviceConfig) -> Vec<u8> {
    let header = build_header(request, cfg, request.ucp_method);
    let mut out = header.to_bytes().to_vec();
    tlv::encode_into(0x0c, cfg.state.as_bytes(), &mut out);
    tlv::encode_into(0x0b, cfg.device_id.as_bytes(), &mut out);
    tlv::encode_into(0x0a, cfg.hardware.as_bytes(), &mut out);
    tlv::encode_into(0x09, cfg.firmware.as_bytes(), &mut out);
    tlv::encode_into(0x03, cfg.model.as_bytes(), &mut out);
    tlv::encode_into(0x02, cfg.name.as_bytes(), &mut out);
    out
}

/// Builds a `get_ip` response: header plus the three address TLVs in
/// go-udap's order — ip, subnet mask, gateway.
#[must_use]
pub fn get_ip_response(request: &Packet, cfg: &DeviceConfig) -> Vec<u8> {
    let header = build_header(request, cfg, request.ucp_method);
    let mut out = header.to_bytes().to_vec();
    tlv::encode_into(0x05, &cfg.ip.octets(), &mut out);
    tlv::encode_into(0x06, &cfg.subnet_mask.octets(), &mut out);
    tlv::encode_into(0x07, &cfg.gateway.octets(), &mut out);
    out
}

/// Builds a `get_uuid` response: header plus the 16-byte UUID TLV.
#[must_use]
pub fn get_uuid_response(request: &Packet, cfg: &DeviceConfig) -> Vec<u8> {
    let header = build_header(request, cfg, request.ucp_method);
    let mut out = header.to_bytes().to_vec();
    tlv::encode_into(0x0d, &cfg.uuid, &mut out);
    out
}

/// Credential fields preceding the item list in a `get_data` request.
const CREDENTIAL_FIELDS: usize = 32;

/// Builds a `get_data` response for the offsets the request asked for.
///
/// Each requested offset is answered from the device's working memory,
/// encoded to its wire width — so a read reflects whatever was last
/// written rather than a fixed factory value. Offsets absent from the
/// table are skipped, as a real device would skip what it does not have.
#[must_use]
pub fn get_data_response(
    request: &Packet,
    cfg: &DeviceConfig,
    state: &DeviceState,
    payload: &[u8],
) -> Vec<u8> {
    let header = build_header(request, cfg, request.ucp_method);
    let mut out = header.to_bytes().to_vec();

    let mut items: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut pos = CREDENTIAL_FIELDS;
    if payload.len() >= pos + 2 {
        let count = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        pos += 2;
        for _ in 0..count {
            if pos + 4 > payload.len() {
                break;
            }
            let offset = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
            pos += 4; // offset and the requested length
            if let Some(param) = udap::parameters::by_offset(offset)
                && let Ok(encoded) = param.encode(state.get(param.name).unwrap_or(b""))
            {
                items.push((offset, encoded));
            }
        }
    }

    let count = u16::try_from(items.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&count.to_be_bytes());
    for (offset, value) in items {
        let length = u16::try_from(value.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&value);
    }
    out
}

/// Builds a `reset` acknowledgement: header only, no payload.
#[must_use]
pub fn reset_response(request: &Packet, cfg: &DeviceConfig) -> Vec<u8> {
    build_header(request, cfg, request.ucp_method)
        .to_bytes()
        .to_vec()
}

/// Builds an error reply (UCP 0x0007).
///
/// An empty `message` yields a reply with no TLVs at all — the path
/// every operation handles separately from one carrying an explanation.
#[must_use]
pub fn error_response(request: &Packet, cfg: &DeviceConfig, message: &str) -> Vec<u8> {
    let header = build_header(request, cfg, udap::protocol::method::ERROR);
    let mut out = header.to_bytes().to_vec();
    if !message.is_empty() {
        tlv::encode_into(0x03, message.as_bytes(), &mut out);
    }
    out
}

/// Builds a `set_data` acknowledgement: header plus a 2-byte count of
/// the parameters accepted.
///
/// go-udap accepts 0x0006, 0x0005 or 0x0002 as an acknowledgement; real
/// devices have been observed answering with the method they were sent.
#[must_use]
pub fn set_data_response(request: &Packet, cfg: &DeviceConfig, accepted: u16) -> Vec<u8> {
    let mut out = build_header(request, cfg, request.ucp_method)
        .to_bytes()
        .to_vec();
    out.extend_from_slice(&accepted.to_be_bytes());
    out
}
