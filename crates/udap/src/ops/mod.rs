//! The UCP operations, as free functions over a [`crate::Session`] and a
//! caller-held [`crate::Device`].
//!
//! Free functions rather than `Client` methods because an operation needs
//! the transport and *one* device, never the registry — and `&Client`
//! plus `&mut Client.devices[..]` is two overlapping borrows. `Client`
//! wraps each of these so call sites still read like go-udap's.

use crate::transport::TransportError;

/// Why an operation failed.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("send request: {0}")]
    Send(#[source] TransportError),
    #[error("recv reply: {0}")]
    Recv(#[source] TransportError),
    #[error("build packet: {0}")]
    Encode(#[from] crate::error::EncodeError),
    #[error("decode response: {0}")]
    Decode(#[from] crate::error::GetDataError),
    /// The device answered with UCP method 0x0007 and an error TLV.
    ///
    /// Text is fidelity-contract: go-udap formats this as
    /// `device %s error: %s` (`udap/config.go`).
    #[error("device {mac} error: {message}")]
    Device { mac: String, message: String },
    #[error("get_uuid response missing UUID TLV")]
    MissingUuid,
}
