//! Parses real captured device responses, so a refactor that changes
//! decoding is caught against hardware-observed bytes rather than
//! against our own encoder.

use udap::protocol::{Packet, UDAP_TYPE_UCP, method};

const DISCOVERY_FACTORY: &[u8] = include_bytes!("fixtures/discovery-factory.bin");

#[test]
fn parses_a_real_discovery_response() {
    let (packet, payload) = Packet::from_bytes(DISCOVERY_FACTORY).expect("fixture must parse");
    assert_eq!(packet.udap_type, UDAP_TYPE_UCP);
    assert_eq!(packet.ucp_method, method::ADV_DISC);
    assert_eq!(
        packet.ucp_flags & 0x01,
        0,
        "a device reply has the request bit clear"
    );
    assert!(!payload.is_empty(), "discovery responses carry TLVs");
}

#[test]
fn decodes_the_discovery_tlvs() {
    let (_, payload) = Packet::from_bytes(DISCOVERY_FACTORY).expect("fixture must parse");
    let tlvs = udap::tlv::decode(payload);
    assert!(!tlvs.is_empty());
    // 0x09 is firmware_rev; every real device reports one.
    assert!(
        tlvs.iter().any(|t| t.tag == 0x09),
        "expected a firmware_rev TLV"
    );
}
