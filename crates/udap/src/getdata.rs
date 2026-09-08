//! Decoder for the `GetData` (0x0005) response payload.
//!
//! Wire format: `u16 count`, then `count` items of
//! `u16 offset, u16 length, length bytes`. Verified against `Net::UDAP`
//! wire captures.

use crate::error::GetDataError;
use crate::parameters;
use std::collections::BTreeMap;

/// Decodes a `GetData` response payload — everything after the 27-byte header.
///
/// Offsets are mapped back to parameter names via the parameter table.
/// Unrecognised offsets are recorded under a synthetic `offset_<decimal>`
/// key with the raw bytes hex-encoded, matching go-udap.
///
/// # Errors
/// [`GetDataError`] if the payload is malformed.
pub fn parse_response(data: &[u8]) -> Result<BTreeMap<String, String>, GetDataError> {
    if data.len() < 2 {
        return Err(GetDataError::PayloadTooShort { got: data.len() });
    }
    let count = usize::from(u16::from_be_bytes([data[0], data[1]]));
    let mut pos = 2usize;
    let mut out = BTreeMap::new();

    for index in 0..count {
        if pos + 4 > data.len() {
            return Err(GetDataError::TruncatedHeader { index, pos });
        }
        let offset = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let length = u16::from_be_bytes([data[pos + 2], data[pos + 3]]);
        pos += 4;
        if pos + usize::from(length) > data.len() {
            return Err(GetDataError::ItemExceedsPayload {
                index,
                offset,
                length,
                remaining: data.len() - pos,
            });
        }
        let value = &data[pos..pos + usize::from(length)];
        pos += usize::from(length);

        match parameters::by_offset(offset) {
            Some(p) => {
                out.insert(p.name.to_owned(), format_value(value));
            }
            None => {
                out.insert(format!("offset_{offset}"), hex_encode(value));
            }
        }
    }
    Ok(out)
}

/// Renders a raw NVRAM value so it round-trips back through
/// `Parameter::encode`.
fn format_value(value: &[u8]) -> String {
    match value.len() {
        1 => value[0].to_string(),
        2 => u16::from_be_bytes([value[0], value[1]]).to_string(),
        4 => format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]),
        _ => {
            let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
            String::from_utf8_lossy(&value[..end]).into_owned()
        }
    }
}

fn hex_encode(value: &[u8]) -> String {
    let mut s = String::with_capacity(value.len() * 2);
    for byte in value {
        use std::fmt::Write;
        // Writing to a String is infallible.
        let _ = write!(s, "{byte:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a response payload: u16 count, then count x (u16 offset,
    /// u16 length, value bytes).
    fn payload(items: &[(u16, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        #[expect(clippy::cast_possible_truncation, reason = "test data is small")]
        out.extend_from_slice(&(items.len() as u16).to_be_bytes());
        for (offset, value) in items {
            out.extend_from_slice(&offset.to_be_bytes());
            #[expect(clippy::cast_possible_truncation, reason = "test data is small")]
            out.extend_from_slice(&(value.len() as u16).to_be_bytes());
            out.extend_from_slice(value);
        }
        out
    }

    #[test]
    fn decodes_a_one_byte_numeric() {
        let got = parse_response(&payload(&[(4, &[1])])).unwrap();
        assert_eq!(got.get("lan_ip_mode").map(String::as_str), Some("1"));
    }

    #[test]
    fn decodes_a_four_byte_value_as_dotted_quad() {
        let got = parse_response(&payload(&[(5, &[192, 168, 1, 50])])).unwrap();
        assert_eq!(
            got.get("lan_network_address").map(String::as_str),
            Some("192.168.1.50")
        );
    }

    #[test]
    fn decodes_a_string_and_trims_at_the_first_nul() {
        let mut value = b"bedroom".to_vec();
        value.resize(33, 0);
        let got = parse_response(&payload(&[(17, &value)])).unwrap();
        assert_eq!(got.get("hostname").map(String::as_str), Some("bedroom"));
    }

    #[test]
    fn unknown_offsets_become_synthetic_hex_keys() {
        let got = parse_response(&payload(&[(9999, &[0xde, 0xad])])).unwrap();
        assert_eq!(got.get("offset_9999").map(String::as_str), Some("dead"));
    }

    #[test]
    fn decodes_multiple_items() {
        let got = parse_response(&payload(&[(4, &[1]), (5, &[10, 0, 0, 5])])).unwrap();
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn rejects_a_payload_too_short_for_the_count() {
        assert!(matches!(
            parse_response(&[0x00]),
            Err(GetDataError::PayloadTooShort { .. })
        ));
    }

    #[test]
    fn rejects_a_truncated_item_header() {
        // count=1 but only two bytes of the four-byte item header follow
        let data = [0x00, 0x01, 0x00, 0x04];
        assert!(matches!(
            parse_response(&data),
            Err(GetDataError::TruncatedHeader { .. })
        ));
    }

    #[test]
    fn rejects_an_item_longer_than_the_payload() {
        // count=1, offset=4, length=100, but no value bytes follow
        let data = [0x00, 0x01, 0x00, 0x04, 0x00, 0x64];
        assert!(matches!(
            parse_response(&data),
            Err(GetDataError::ItemExceedsPayload { .. })
        ));
    }

    // A crafted count of 65535 with a tiny body must not cause work
    // proportional to the declared count.
    //
    // go-udap needs an explicit clamp for this — `make(map[string]string,
    // min(int(count), (len(data)-2)/4))` at getdata_response.go:42 — because
    // Go pre-sizes the map from the hint. `BTreeMap` takes no capacity hint,
    // so there is no allocation to bound here and nothing to clamp; the
    // protection is a property of the container choice. What is worth
    // asserting is the loop bound, which is ours: parsing must stop at the
    // first item the payload cannot cover rather than iterating `count`
    // times. If this ever moves to `HashMap::with_capacity(count)`, the
    // clamp has to come back and this test needs an allocation assertion.
    #[test]
    fn oversized_count_stops_at_the_payload_bound() {
        let data = [0xff, 0xff, 0x00, 0x04, 0x00, 0x01, 0x07];
        // Only this many item headers fit after the 2-byte count, so the
        // loop must give up here rather than at item 65534.
        let max_items = (data.len() - 2) / 4;
        let bailed_at = match parse_response(&data) {
            Err(GetDataError::TruncatedHeader { index, .. }) => Some(index),
            _ => None,
        };
        assert_eq!(
            bailed_at,
            Some(max_items),
            "must stop at the first item the payload cannot cover"
        );
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // This decoder runs on network input from an untrusted LAN peer.
        #[test]
        fn never_panics_on_arbitrary_input(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
            let _ = parse_response(&data);
        }
    }
}
