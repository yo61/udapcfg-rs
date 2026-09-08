//! The network abstraction beneath `Client`.
//!
//! Addressing is encoded in the packets themselves, not at this layer:
//! `send` broadcasts, and the destination MAC lives inside the packet.

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("operation cancelled")]
    Cancelled,
    #[error("transport I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Send and receive raw UDAP packets.
///
/// Implemented by the real UDP transport and by `mocksbr::MockTransport`
/// for hermetic in-process tests.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Dispatches a packet. The destination is encoded in the packet.
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError>;

    /// Waits for the next packet, or until `cancel` fires.
    ///
    /// Returns the raw bytes and an informational source identifier —
    /// an IP for the UDP transport, a MAC for the mock. The source is
    /// for logging and reply validation; routing uses packet contents.
    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError>;

    /// Releases transport resources.
    async fn close(&self) -> Result<(), TransportError>;
}
