//! Wire encoding for the mock's parameter values, and the `set_data`
//! request parser.
//!
//! Deliberately a separate implementation from `udap`'s `format_value`,
//! not a shared one. go-udap duplicates it the same way — but the better
//! reason is independence: if the mock decoded with the client's own
//! decoder, a bug in that decoder would be invisible, because both sides
//! of a round-trip would agree on the wrong answer.

use udap::parameters;

/// Credential fields preceding the item list in a `set_data` request.
const CREDENTIAL_FIELDS: usize = 32;

/// Renders a raw NVRAM value as its display form, dispatching on width.
///
/// Mirrors go-udap's `decodeParamValue`: 1 byte is a decimal number,
/// 2 bytes a big-endian decimal number, 4 bytes a dotted quad, anything
/// else a NUL-terminated string.
///
/// Returns `Vec<u8>`, not `String`: NVRAM strings carry no encoding
/// guarantee (ADR-6), and the mock must not be what mangles them.
pub(crate) fn decode_param_value(value: &[u8]) -> Vec<u8> {
    match value.len() {
        1 => value[0].to_string().into_bytes(),
        2 => u16::from_be_bytes([value[0], value[1]])
            .to_string()
            .into_bytes(),
        4 => format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]).into_bytes(),
        _ => {
            let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
            value[..end].to_vec()
        }
    }
}

/// One offset/length/value triple from a `set_data` request.
pub(crate) struct SetDataItem {
    pub offset: u16,
    pub length: u16,
    pub value: Vec<u8>,
    /// `None` when the offset is not in the parameter table.
    pub name: Option<&'static str>,
}

/// Decodes a `set_data` request body.
///
/// A truncated item ends the walk and keeps what was read, matching
/// go-udap: a malformed tail must not discard a well-formed prefix.
pub(crate) fn parse_set_data_request(payload: &[u8]) -> Vec<SetDataItem> {
    if payload.len() < CREDENTIAL_FIELDS + 2 {
        return Vec::new();
    }
    let mut pos = CREDENTIAL_FIELDS;
    let count = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
    pos += 2;

    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if pos + 4 > payload.len() {
            break;
        }
        let offset = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let length = u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]);
        pos += 4;
        let end = pos + usize::from(length);
        if end > payload.len() {
            break;
        }
        out.push(SetDataItem {
            offset,
            length,
            value: payload[pos..end].to_vec(),
            name: parameters::by_offset(offset).map(|p| p.name),
        });
        pos = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_byte_value_decodes_as_a_decimal_number() {
        assert_eq!(decode_param_value(&[6]), b"6");
        assert_eq!(decode_param_value(&[255]), b"255");
    }

    #[test]
    fn a_two_byte_value_decodes_as_a_big_endian_number() {
        assert_eq!(decode_param_value(&[0x01, 0x2c]), b"300");
    }

    #[test]
    fn a_four_byte_value_decodes_as_a_dotted_quad() {
        assert_eq!(decode_param_value(&[192, 168, 1, 50]), b"192.168.1.50");
    }

    #[test]
    fn a_string_value_stops_at_the_first_nul() {
        // NVRAM string fields are NUL-padded to their full width.
        assert_eq!(decode_param_value(b"hello\0\0\0"), b"hello");
    }

    #[test]
    fn a_string_value_with_no_nul_is_taken_whole() {
        assert_eq!(decode_param_value(b"abcde"), b"abcde");
    }

    #[test]
    fn a_non_utf8_string_value_survives_byte_exact() {
        // ADR-6: 802.11 does not require an SSID to be valid UTF-8, and
        // the mock must not be the thing that mangles it.
        let raw = [0xff, 0xfe, 0x41, 0x00, 0x00];
        assert_eq!(decode_param_value(&raw), vec![0xff, 0xfe, 0x41]);
    }

    #[test]
    fn a_request_decodes_into_its_items() {
        // 32 credential bytes, count=1, then offset/length/value.
        // Offset 4 is lan_ip_mode, a 1-byte parameter.
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&4u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(1);

        let items = parse_set_data_request(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].offset, 4);
        assert_eq!(items[0].length, 1);
        assert_eq!(items[0].value, vec![1]);
        assert_eq!(items[0].name, Some("lan_ip_mode"));
    }

    #[test]
    fn an_unknown_offset_parses_but_has_no_name() {
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.extend_from_slice(&9999u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(7);

        let items = parse_set_data_request(&payload);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, None, "offset 9999 is not in the table");
    }

    #[test]
    fn a_truncated_item_stops_the_walk_without_panicking() {
        // count promises 2, only one complete item follows.
        let mut payload = vec![0u8; 32];
        payload.extend_from_slice(&2u16.to_be_bytes());
        payload.extend_from_slice(&4u16.to_be_bytes());
        payload.extend_from_slice(&1u16.to_be_bytes());
        payload.push(1);
        payload.extend_from_slice(&4u16.to_be_bytes()); // header cut short

        assert_eq!(parse_set_data_request(&payload).len(), 1);
    }

    #[test]
    fn a_payload_too_short_for_the_count_yields_nothing() {
        assert!(parse_set_data_request(&[0u8; 10]).is_empty());
    }
}
