//! Error types for the `udap` crate.

/// Errors from decoding a UDAP packet header.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("packet too short: {got} bytes (minimum {min})")]
    TooShort { got: usize, min: usize },
    #[error("not a UDAP/UCP packet: UDAPType=0x{udap_type:04x}")]
    NotUcp { udap_type: u16 },
}
