//! UDAP packet header and protocol constants.
//!
//! All multi-byte fields are network byte order (big-endian), matching
//! the `Net::UDAP` Perl reference implementation.

use crate::Mac;
use crate::error::ProtocolError;

/// UDAP listens on this UDP port.
pub const PORT: u16 = 17784;

/// Serialized size of the packet header: the sum of its fields, no padding.
pub const HEADER_SIZE: usize = 27;

/// `UDAPType` value identifying a UCP packet.
pub const UDAP_TYPE_UCP: u16 = 0xC001;

/// Ethernet addressing. Real devices always use this.
pub const ADDR_TYPE_ETH: u8 = 0x01;

/// The only UAP class UDAP uses.
pub const UAP_CLASS_UCP: [u8; 4] = [0x00, 0x01, 0x00, 0x01];

/// Byte index of `ucp_flags` within the serialized header — the sum of
/// the field sizes before it: `dst_broadcast` (1) + `dst_type` (1) +
/// `dst_address` (6) + `src_broadcast` (1) + `src_type` (1) +
/// `src_address` (6) + `sequence` (2) + `udap_type` (2) = 20.
pub const UCP_FLAGS_OFFSET: usize = 20;

/// The request bit in `ucp_flags`. We send with it set; devices reply
/// with it clear.
pub const FLAG_REQUEST: u8 = 0x01;

/// UCP method numbers, per `Net::UDAP` `Constant.pm`.
pub mod method {
    pub const DISCOVER: u16 = 0x0001;
    pub const GET_IP: u16 = 0x0002;
    pub const RESET: u16 = 0x0004;
    pub const GET_DATA: u16 = 0x0005;
    pub const SET_DATA: u16 = 0x0006;
    pub const ERROR: u16 = 0x0007;
    pub const CREDENTIALS_ERROR: u16 = 0x0008;
    pub const ADV_DISC: u16 = 0x0009;
    pub const GET_UUID: u16 = 0x000b;
}

/// A UDAP packet header.
///
/// `ucp_method` is deliberately a raw `u16` rather than an enum: the
/// client reports unrecognised methods verbatim ("unexpected response
/// method 0x%04x"), which an enum could not represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub dst_broadcast: u8,
    pub dst_type: u8,
    pub dst_address: Mac,
    pub src_broadcast: u8,
    pub src_type: u8,
    pub src_address: Mac,
    pub sequence: u16,
    pub udap_type: u16,
    pub ucp_flags: u8,
    pub uap_class: [u8; 4],
    pub ucp_method: u16,
}

impl Packet {
    /// Serializes the header. Field order and offsets are the wire contract.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[0] = self.dst_broadcast;
        b[1] = self.dst_type;
        b[2..8].copy_from_slice(self.dst_address.as_bytes());
        b[8] = self.src_broadcast;
        b[9] = self.src_type;
        b[10..16].copy_from_slice(self.src_address.as_bytes());
        b[16..18].copy_from_slice(&self.sequence.to_be_bytes());
        b[18..20].copy_from_slice(&self.udap_type.to_be_bytes());
        b[20] = self.ucp_flags;
        b[21..25].copy_from_slice(&self.uap_class);
        b[25..27].copy_from_slice(&self.ucp_method.to_be_bytes());
        b
    }

    /// Parses a header, returning it alongside the remaining payload.
    ///
    /// Rejects anything shorter than [`HEADER_SIZE`] or whose `UDAPType` is
    /// not [`UDAP_TYPE_UCP`] — such packets are junk on our socket (mDNS
    /// leakage, stray broadcasts), not data to interpret.
    ///
    /// # Errors
    /// [`ProtocolError::TooShort`] or [`ProtocolError::NotUcp`].
    pub fn from_bytes(buf: &[u8]) -> Result<(Packet, &[u8]), ProtocolError> {
        if buf.len() < HEADER_SIZE {
            return Err(ProtocolError::TooShort {
                got: buf.len(),
                min: HEADER_SIZE,
            });
        }
        let mut dst = [0u8; 6];
        dst.copy_from_slice(&buf[2..8]);
        let mut src = [0u8; 6];
        src.copy_from_slice(&buf[10..16]);
        let mut uap_class = [0u8; 4];
        uap_class.copy_from_slice(&buf[21..25]);

        let packet = Packet {
            dst_broadcast: buf[0],
            dst_type: buf[1],
            dst_address: Mac::from_bytes(dst),
            src_broadcast: buf[8],
            src_type: buf[9],
            src_address: Mac::from_bytes(src),
            sequence: u16::from_be_bytes([buf[16], buf[17]]),
            udap_type: u16::from_be_bytes([buf[18], buf[19]]),
            ucp_flags: buf[20],
            uap_class,
            ucp_method: u16::from_be_bytes([buf[25], buf[26]]),
        };
        if packet.udap_type != UDAP_TYPE_UCP {
            return Err(ProtocolError::NotUcp {
                udap_type: packet.udap_type,
            });
        }
        Ok((packet, &buf[HEADER_SIZE..]))
    }
}

/// Reports whether `buf` is a UDAP packet with the request bit set.
///
/// The capture path uses this to skip our own kernel-looped broadcast:
/// we send with the request bit set, devices reply with it clear.
/// Returns `false` for buffers too short to contain the flags byte.
#[must_use]
pub fn is_request_packet(buf: &[u8]) -> bool {
    match buf.get(UCP_FLAGS_OFFSET) {
        Some(flags) => flags & FLAG_REQUEST != 0,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mac;

    fn sample() -> Packet {
        Packet {
            dst_broadcast: 1,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::from_bytes([0x00, 0x04, 0x20, 0x16, 0x05, 0x8f]),
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0x01,
            uap_class: UAP_CLASS_UCP,
            ucp_method: method::ADV_DISC,
        }
    }

    #[test]
    fn header_is_27_bytes() {
        assert_eq!(HEADER_SIZE, 27);
        assert_eq!(sample().to_bytes().len(), 27);
    }

    #[test]
    fn field_offsets_match_the_wire_layout() {
        let b = sample().to_bytes();
        assert_eq!(b[0], 1, "dst_broadcast");
        assert_eq!(b[1], ADDR_TYPE_ETH, "dst_type");
        assert_eq!(&b[2..8], &[0u8; 6], "dst_address");
        assert_eq!(b[8], 0, "src_broadcast");
        assert_eq!(b[9], ADDR_TYPE_ETH, "src_type");
        assert_eq!(
            &b[10..16],
            &[0x00, 0x04, 0x20, 0x16, 0x05, 0x8f],
            "src_address"
        );
        assert_eq!(&b[16..18], &[0x00, 0x01], "sequence, big-endian");
        assert_eq!(&b[18..20], &[0xC0, 0x01], "udap_type, big-endian");
        assert_eq!(b[20], 0x01, "ucp_flags");
        assert_eq!(&b[21..25], &[0x00, 0x01, 0x00, 0x01], "uap_class");
        assert_eq!(&b[25..27], &[0x00, 0x09], "ucp_method, big-endian");
    }

    #[test]
    fn ucp_flags_offset_constant_matches_the_layout() {
        let b = sample().to_bytes();
        assert_eq!(b[UCP_FLAGS_OFFSET], 0x01);
    }

    #[test]
    fn roundtrips_through_bytes() {
        let original = sample();
        let bytes = original.to_bytes();
        let (parsed, payload) = Packet::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, original);
        assert!(payload.is_empty());
    }

    #[test]
    fn returns_the_payload_after_the_header() {
        let mut bytes = sample().to_bytes().to_vec();
        bytes.extend_from_slice(&[0x02, 0x03, b'a', b'b', b'c']);
        let (_, payload) = Packet::from_bytes(&bytes).unwrap();
        assert_eq!(payload, &[0x02, 0x03, b'a', b'b', b'c']);
    }

    #[test]
    fn rejects_short_packets() {
        let bytes = [0u8; 26];
        let err = Packet::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ProtocolError::TooShort { got: 26, min: 27 }));
    }

    #[test]
    fn rejects_non_ucp_packets() {
        let mut bytes = sample().to_bytes();
        bytes[18] = 0xAA;
        bytes[19] = 0xBB;
        let err = Packet::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, ProtocolError::NotUcp { udap_type: 0xAABB }));
    }

    // We broadcast with the request bit set; the kernel loops our own
    // packet back to us. The capture path uses this to skip it.
    #[test]
    fn identifies_our_own_looped_back_request() {
        let bytes = sample().to_bytes();
        assert!(is_request_packet(&bytes));
    }

    #[test]
    fn device_replies_are_not_requests() {
        let mut p = sample();
        p.ucp_flags = 0x00;
        assert!(!is_request_packet(&p.to_bytes()));
    }

    #[test]
    fn short_buffers_are_not_requests() {
        assert!(!is_request_packet(&[0u8; 10]));
        assert!(!is_request_packet(&[]));
    }
}
