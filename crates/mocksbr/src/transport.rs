//! `MockTransport` — a `udap::Transport` backed by an in-process `Network`.

use crate::network::Network;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use udap::transport::{Transport, TransportError};

/// Backed by an `mpsc` channel rather than a queue-plus-`Notify` pair:
/// the channel's own implementation checks for a queued item and
/// registers the receiving task's waker as one atomic step, so there is
/// no gap in which a concurrent `send` can land between "queue looked
/// empty" and "waker registered" and be missed. `Notify::notify_waiters`
/// wakes only tasks already registered at the moment it is called, so a
/// hand-rolled queue-plus-`Notify` version has exactly that gap.
pub struct MockTransport {
    network: Arc<Network>,
    sender: mpsc::UnboundedSender<(Vec<u8>, String)>,
    receiver: Mutex<mpsc::UnboundedReceiver<(Vec<u8>, String)>>,
    /// Cancelled by `close`. A separate token from the one `recv`
    /// callers pass in: that one scopes a single call, this one scopes
    /// the transport's lifetime, so `close` wakes every pending (and
    /// future) `recv` regardless of which caller-supplied token it was
    /// given — matching go-udap's `MockTransport.Close`, which wakes a
    /// blocked `Recv` immediately rather than leaving it to time out.
    closed: CancellationToken,
}

impl MockTransport {
    #[must_use]
    pub fn new(network: Arc<Network>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        MockTransport {
            network,
            sender,
            receiver: Mutex::new(receiver),
            closed: CancellationToken::new(),
        }
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError> {
        for reply in self.network.receive(packet) {
            // The receiver lives in `self.receiver` for as long as `self`
            // does, so it can never be dropped out from under this
            // sender; `send` on an unbounded channel only fails once the
            // receiver is gone, which cannot happen here.
            let _ = self.sender.send(reply);
        }
        Ok(())
    }

    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
        let mut receiver = self.receiver.lock().await;
        tokio::select! {
            () = cancel.cancelled() => Err(TransportError::Cancelled),
            () = self.closed.cancelled() => Err(TransportError::Cancelled),
            reply = receiver.recv() => reply.ok_or(TransportError::Cancelled),
        }
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.closed.cancel();
        Ok(())
    }
}
