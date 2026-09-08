//! Type-Length-Value codec used by UDAP discovery and error payloads.
//!
//! Wire form per entry: one tag byte, one length byte, then that many
//! value bytes. Length is `u8`, so a value is at most 255 bytes.

/// One decoded TLV entry. Borrows its value from the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
}

/// Decodes a TLV sequence.
///
/// A truncated trailing entry is dropped and the well-formed prefix is
/// returned — matching go-udap's `DecodeTLV`, which breaks rather than
/// erroring. Callers that need strictness must check the returned count.
#[must_use]
pub fn decode(data: &[u8]) -> Vec<Tlv<'_>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 2 <= data.len() {
        let tag = data[pos];
        let len = usize::from(data[pos + 1]);
        pos += 2;
        if pos + len > data.len() {
            break;
        }
        out.push(Tlv {
            tag,
            value: &data[pos..pos + len],
        });
        pos += len;
    }
    out
}

/// Appends one TLV entry to `out`.
///
/// Values longer than 255 bytes are truncated, because the length field
/// is a single byte. UDAP's own TLVs never approach that.
pub fn encode_into(tag: u8, value: &[u8], out: &mut Vec<u8>) {
    let len = value.len().min(255);
    out.push(tag);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "len is clamped to 255 on the line above"
    )]
    out.push(len as u8);
    out.extend_from_slice(&value[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_single_entry() {
        let data = [0x02, 0x03, b'a', b'b', b'c'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].tag, 0x02);
        assert_eq!(tlvs[0].value, b"abc");
    }

    #[test]
    fn decodes_multiple_entries_in_order() {
        let data = [0x02, 0x01, b'x', 0x09, 0x02, b'7', b'7'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 2);
        assert_eq!(tlvs[0].tag, 0x02);
        assert_eq!(tlvs[0].value, b"x");
        assert_eq!(tlvs[1].tag, 0x09);
        assert_eq!(tlvs[1].value, b"77");
    }

    #[test]
    fn decodes_zero_length_value() {
        let data = [0x0c, 0x00];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].value, b"");
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(decode(&[]).is_empty());
    }

    // Matches Go's DecodeTLV: a truncated tail is dropped, the good
    // prefix is kept, and no error is raised.
    #[test]
    fn truncated_value_keeps_the_good_prefix() {
        let data = [0x02, 0x01, b'x', 0x09, 0x05, b'a', b'b'];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
        assert_eq!(tlvs[0].tag, 0x02);
    }

    #[test]
    fn dangling_tag_byte_is_dropped() {
        let data = [0x02, 0x01, b'x', 0x09];
        let tlvs = decode(&data);
        assert_eq!(tlvs.len(), 1);
    }

    #[test]
    fn encode_then_decode_roundtrips() {
        let mut buf = Vec::new();
        encode_into(0x0c, b"connected", &mut buf);
        encode_into(0x09, b"77", &mut buf);
        let tlvs = decode(&buf);
        assert_eq!(tlvs.len(), 2);
        assert_eq!(tlvs[0].value, b"connected");
        assert_eq!(tlvs[1].value, b"77");
    }

    // Matches mocksbr's writeTLV: the length field is one byte, so
    // over-long values are truncated rather than corrupting the stream.
    #[test]
    fn encode_truncates_values_over_255_bytes() {
        let long = vec![b'z'; 300];
        let mut buf = Vec::new();
        encode_into(0x02, &long, &mut buf);
        assert_eq!(buf[1], 255);
        assert_eq!(buf.len(), 2 + 255);
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // The decoder runs on network input. It must never panic.
        #[test]
        fn decode_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode(&data);
        }

        #[test]
        fn decode_never_reads_past_the_buffer(data in proptest::collection::vec(any::<u8>(), 0..512)) {
            let total: usize = decode(&data).iter().map(|t| 2 + t.value.len()).sum();
            prop_assert!(total <= data.len());
        }
    }
}
