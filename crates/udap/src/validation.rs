//! Per-parameter input rules for values the *user* supplies.
//!
//! Input is `&str`, not bytes: ADR-6 makes device-supplied values
//! `Vec<u8>`, but these rules run on CLI flags and config-file entries,
//! which are text by construction.

use crate::parameters;
use std::net::Ipv4Addr;

/// Why a value was rejected.
///
/// Every message here is user-visible and copied verbatim from
/// go-udap's `validateParameter` (`udap/validation.go:35-97`).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("expected numeric value (0-255), got {0:?}")]
    NotU8(String),
    #[error("expected numeric value (0-65535), got {0:?}")]
    NotU16(String),
    #[error("expected valid IPv4 address, got {0:?}")]
    NotIpv4(String),
    #[error("value too long (max {max} chars), got {got}")]
    TooLong { max: u16, got: usize },
    #[error("must be 0 (infrastructure) or 1 (ad-hoc)")]
    WirelessMode,
    #[error("must be between 1 and 13")]
    WirelessChannel,
    #[error("must be 5 or 13 for WEP keys")]
    WirelessKeylen,
    #[error("must be 8-63 characters")]
    WpaPskLength,
    #[error("must be 1-32 characters")]
    SsidLength,
}

/// Go's `strconv.ParseUint` semantics: digits only, no sign.
///
/// Rust's `FromStr` accepts a leading `+` for unsigned integers —
/// `"+1".parse::<u8>()` is `Ok(1)` — while Go's `ParseUint` has no sign
/// handling at all and treats it as a syntax error. The difference
/// reaches a fidelity-contract error message, so it is stripped here
/// rather than left to the caller.
fn parses_as_unsigned<T: std::str::FromStr>(value: &str) -> bool {
    !value.starts_with('+') && value.parse::<T>().is_ok()
}

/// Validates a user-supplied value for `name`.
///
/// An unrecognised `name` is accepted, matching go-udap: the CLI can
/// carry parameters this table does not know about.
///
/// Width is checked first, then the parameter-specific rule — the same
/// order as `validateParameter`, so a value failing both reports the
/// width error.
///
/// # Errors
/// [`ValidationError`] describing the first rule the value fails.
pub fn validate_parameter(name: &str, value: &str) -> Result<(), ValidationError> {
    let Some(param) = parameters::by_name(name) else {
        return Ok(());
    };

    match param.length {
        1 => {
            if !parses_as_unsigned::<u8>(value) {
                return Err(ValidationError::NotU8(value.to_owned()));
            }
        }
        2 => {
            if !parses_as_unsigned::<u16>(value) {
                return Err(ValidationError::NotU16(value.to_owned()));
            }
        }
        4 => {
            if value.parse::<Ipv4Addr>().is_err() {
                return Err(ValidationError::NotIpv4(value.to_owned()));
            }
        }
        max => {
            if value.len() > usize::from(max) {
                return Err(ValidationError::TooLong {
                    max,
                    got: value.len(),
                });
            }
        }
    }

    match name {
        "wireless_mode" => {
            if value != "0" && value != "1" {
                return Err(ValidationError::WirelessMode);
            }
        }
        "wireless_channel" => {
            let channel = if parses_as_unsigned::<u32>(value) {
                value.parse::<u32>().ok()
            } else {
                None
            };
            match channel {
                Some(ch) if (1..=13).contains(&ch) => {}
                _ => return Err(ValidationError::WirelessChannel),
            }
        }
        "wireless_keylen" => {
            if value != "5" && value != "13" {
                return Err(ValidationError::WirelessKeylen);
            }
        }
        "wireless_wpa_psk" if !(8..=63).contains(&value.len()) => {
            return Err(ValidationError::WpaPskLength);
        }
        "wireless_SSID" if !(1..=32).contains(&value.len()) => {
            return Err(ValidationError::SsidLength);
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_parameter_is_allowed() {
        // go-udap: "Unknown parameter, but we'll allow it".
        assert!(validate_parameter("not_a_real_param", "anything").is_ok());
    }

    #[test]
    fn a_one_byte_parameter_rejects_non_numeric_input() {
        let err = validate_parameter("lan_ip_mode", "1abc").expect_err("must reject");
        assert_eq!(
            err.to_string(),
            r#"expected numeric value (0-255), got "1abc""#
        );
    }

    #[test]
    fn a_one_byte_parameter_rejects_values_above_255() {
        let err = validate_parameter("lan_ip_mode", "256").expect_err("must reject");
        assert_eq!(
            err.to_string(),
            r#"expected numeric value (0-255), got "256""#
        );
    }

    #[test]
    fn a_numeric_parameter_rejects_a_leading_plus() {
        // Rust's FromStr accepts "+1" for unsigned ints; Go's ParseUint
        // does not. The error text is fidelity-contract, so the two
        // implementations must agree on what is valid.
        let err =
            validate_parameter("lan_ip_mode", "+1").expect_err("Go rejects a signed value here");
        assert_eq!(
            err.to_string(),
            r#"expected numeric value (0-255), got "+1""#
        );
    }

    #[test]
    fn an_ip_parameter_rejects_a_non_address() {
        let err = validate_parameter("lan_gateway", "not-an-ip").expect_err("must reject");
        assert_eq!(
            err.to_string(),
            r#"expected valid IPv4 address, got "not-an-ip""#
        );
    }

    #[test]
    fn an_ip_parameter_accepts_a_dotted_quad() {
        assert!(validate_parameter("lan_gateway", "192.168.1.1").is_ok());
    }

    #[test]
    fn a_string_parameter_rejects_an_overlong_value() {
        let long = "x".repeat(34);
        let err = validate_parameter("squeezecenter_name", &long).expect_err("must reject");
        assert_eq!(err.to_string(), "value too long (max 33 chars), got 34");
    }

    #[test]
    fn wireless_channel_must_be_1_to_13() {
        assert!(validate_parameter("wireless_channel", "6").is_ok());
        let err = validate_parameter("wireless_channel", "14").expect_err("must reject");
        assert_eq!(err.to_string(), "must be between 1 and 13");
    }

    #[test]
    fn wireless_mode_must_be_0_or_1() {
        assert!(validate_parameter("wireless_mode", "0").is_ok());
        let err = validate_parameter("wireless_mode", "2").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 0 (infrastructure) or 1 (ad-hoc)");
    }

    #[test]
    fn wireless_keylen_must_be_5_or_13() {
        let err = validate_parameter("wireless_keylen", "8").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 5 or 13 for WEP keys");
    }

    #[test]
    fn wireless_wpa_psk_must_be_8_to_63_characters() {
        let err = validate_parameter("wireless_wpa_psk", "short").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 8-63 characters");
    }

    #[test]
    fn wireless_ssid_must_be_1_to_32_characters() {
        let err = validate_parameter("wireless_SSID", "").expect_err("must reject");
        assert_eq!(err.to_string(), "must be 1-32 characters");
    }
}
