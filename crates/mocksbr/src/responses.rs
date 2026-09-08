//! Reply builders. Layout and TLV order match go-udap's mocksbr so the
//! committed wire captures stay valid.

use crate::device::DeviceConfig;
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
