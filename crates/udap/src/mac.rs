//! The `Mac` value object: a 48-bit IEEE 802 hardware address.
//!
//! Parsing and formatting rules live here so validation happens once at
//! the boundary and the type carries the guarantee downstream.

use std::fmt;
use std::str::FromStr;

/// Length of a MAC address in bytes.
pub const MAC_LEN: usize = 6;

/// Canonical string form is exactly this many characters: `aa:bb:cc:dd:ee:ff`.
const MAC_STR_LEN: usize = 17;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Mac([u8; MAC_LEN]);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MacParseError {
    #[error("invalid MAC address {input:?}: want {MAC_STR_LEN} chars, got {got}")]
    Length { input: String, got: usize },
    #[error("invalid MAC address {input:?}: non-hex digit at {pos}")]
    NonHex { input: String, pos: usize },
    #[error("invalid MAC address {input:?}: missing colon at {pos}")]
    MissingColon { input: String, pos: usize },
}

impl Mac {
    /// The all-zeros MAC, used as the broadcast destination and as the
    /// source placeholder in outgoing packets.
    pub const ZERO: Mac = Mac([0; MAC_LEN]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; MAC_LEN]) -> Self {
        Mac(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; MAC_LEN] {
        &self.0
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; MAC_LEN]
    }
}

/// Converts one ASCII hex digit to its 0-15 value.
///
/// Hand-rolled rather than pulling in a hex crate for a single-byte decode,
/// matching go-udap's `hexNibble`.
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl FromStr for Mac {
    type Err = MacParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != MAC_STR_LEN {
            return Err(MacParseError::Length {
                input: s.to_owned(),
                got: bytes.len(),
            });
        }
        let mut out = [0u8; MAC_LEN];
        for (i, slot) in out.iter_mut().enumerate() {
            let base = i * 3;
            let hi = hex_nibble(bytes[base]).ok_or_else(|| MacParseError::NonHex {
                input: s.to_owned(),
                pos: base,
            })?;
            let lo = hex_nibble(bytes[base + 1]).ok_or_else(|| MacParseError::NonHex {
                input: s.to_owned(),
                pos: base + 1,
            })?;
            if i < MAC_LEN - 1 && bytes[base + 2] != b':' {
                return Err(MacParseError::MissingColon {
                    input: s.to_owned(),
                    pos: base + 2,
                });
            }
            *slot = (hi << 4) | lo;
        }
        Ok(Mac(out))
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ":")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_lowercase() {
        let m: Mac = "00:04:20:16:05:8f".parse().unwrap();
        assert_eq!(m.as_bytes(), &[0x00, 0x04, 0x20, 0x16, 0x05, 0x8f]);
    }

    #[test]
    fn parses_uppercase_and_mixed_case() {
        let upper: Mac = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        let mixed: Mac = "aA:Bb:cC:Dd:eE:Ff".parse().unwrap();
        assert_eq!(upper, mixed);
        assert_eq!(upper.as_bytes(), &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
    }

    #[test]
    fn display_is_canonical_lowercase() {
        let m = Mac::from_bytes([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        assert_eq!(m.to_string(), "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn roundtrips_through_string() {
        let input = "00:04:20:16:05:8f";
        let m: Mac = input.parse().unwrap();
        assert_eq!(m.to_string(), input);
    }

    #[test]
    fn rejects_wrong_length() {
        assert!("00:04:20:16:05".parse::<Mac>().is_err());
        assert!("00:04:20:16:05:8f:aa".parse::<Mac>().is_err());
    }

    // go-udap tightened this: fmt.Sscanf used to accept trailing space.
    #[test]
    fn rejects_leading_and_trailing_whitespace() {
        assert!(" 00:04:20:16:05:8f".parse::<Mac>().is_err());
        assert!("00:04:20:16:05:8f ".parse::<Mac>().is_err());
    }

    #[test]
    fn rejects_wrong_separator() {
        assert!("00-04-20-16-05-8f".parse::<Mac>().is_err());
    }

    #[test]
    fn rejects_non_hex_digits() {
        assert!("zz:04:20:16:05:8f".parse::<Mac>().is_err());
    }

    #[test]
    fn zero_is_detected() {
        assert!(Mac::ZERO.is_zero());
        assert!(!Mac::from_bytes([0, 0, 0, 0, 0, 1]).is_zero());
    }
}

#[cfg(test)]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn display_parse_roundtrip(bytes in proptest::array::uniform6(any::<u8>())) {
            let m = Mac::from_bytes(bytes);
            let parsed: Mac = m.to_string().parse().unwrap();
            prop_assert_eq!(m, parsed);
        }

        #[test]
        fn never_panics_on_arbitrary_input(s in ".*") {
            let _ = s.parse::<Mac>();
        }
    }
}
