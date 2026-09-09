//! Lowercase hex encoding for wire values.
//!
//! Deliberately not the `hex` crate: this is one loop with no edge cases,
//! and its output is verified byte-identical to Go's `%x` on a byte slice
//! (which zero-pads each byte to two digits). Decoding lives in
//! [`crate::mac`] as `hex_nibble`, hand-rolled for the same reason.

use std::fmt::Write;

/// Encodes `value` as lowercase hex, two characters per byte.
///
/// Matches Go's `fmt.Sprintf("%x", value)` and `hex.EncodeToString`.
pub(crate) fn encode(value: &[u8]) -> String {
    let mut s = String::with_capacity(value.len() * 2);
    for byte in value {
        // Writing to a String is infallible.
        let _ = write!(s, "{byte:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn zero_pads_bytes_below_0x10() {
        // The property Go's %x has and a naive {:x} loop does not.
        assert_eq!(encode(&[0x00, 0x0f, 0x01]), "000f01");
    }

    #[test]
    fn encodes_lowercase() {
        assert_eq!(encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(encode(&[]), "");
    }
}
