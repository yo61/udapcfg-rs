//! Error types for the `udap` crate.

/// Errors from decoding a UDAP packet header.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("packet too short: {got} bytes (minimum {min})")]
    TooShort { got: usize, min: usize },
    #[error("not a UDAP/UCP packet: UDAPType=0x{udap_type:04x}")]
    NotUcp { udap_type: u16 },
}

/// Errors from encoding a parameter value to its NVRAM wire form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("{value:?} is not a valid u8")]
    NotU8 { value: String },
    #[error("{value:?} is not a valid u16")]
    NotU16 { value: String },
    #[error("cannot parse {value:?} as an IPv4 address")]
    NotIpv4 { value: String },
}

/// Errors from decoding a `GetData` response payload.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GetDataError {
    #[error("getdata response: payload too short ({got} bytes)")]
    PayloadTooShort { got: usize },
    #[error("getdata response: truncated header for item {index} at offset {pos}")]
    TruncatedHeader { index: usize, pos: usize },
    #[error(
        "getdata response: item {index} (NVRAM offset {offset}, length {length}) \
         exceeds payload ({remaining} bytes left)"
    )]
    ItemExceedsPayload {
        index: usize,
        offset: u16,
        length: u16,
        remaining: usize,
    },
}
