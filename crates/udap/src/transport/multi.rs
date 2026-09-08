//! Composes several transports: send fans out, recv merges.
//!
//! Used by `--all-interfaces`, one `UdpTransport` per usable interface.
//! `Client` cannot tell the difference — both satisfy `Transport`.
//!
//! `recv` merges children with [`select_all`]: each call builds one fresh
//! `recv` future per child and returns whichever settles first. Nothing is
//! pre-fetched or buffered between calls, which rules out the M2 mock
//! transport's lost-wakeup bug by construction rather than by careful
//! bookkeeping. That bug was a queue checked for a pending item, and only
//! *then* had the receiving task's waker registered — a `send` landing in
//! that gap between the check and the registration was missed forever.
//! The failure mode requires exactly that shape: a persistent queue with a
//! check-then-register step done by hand. `select_all` has neither. There
//! is no queue for a reply to land in unobserved: every child's `recv` is
//! polled by a task that is, at that instant, also polling `select_all`
//! itself, so "checked" and "registered as waiting" are the same poll —
//! there is no gap between them for a reply to land in.
//!
//! The alternative (spawning one long-lived pump task per child, feeding
//! a shared `mpsc` channel) was considered and rejected: the children
//! live behind `&self`, so they cannot be moved into `tokio::spawn`ed
//! tasks without also wrapping them in `Arc` and adding a `Once` to
//! start the pumps lazily and a token to stop them on `close` — real
//! state whose lifecycle has to be gotten right. `select_all` needs none
//! of it: an empty `Vec<Box<dyn Transport>>` behind a shared reference is
//! enough.

use crate::transport::{Transport, TransportError};
use async_trait::async_trait;
use futures_util::future::select_all;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Fans a single logical transport out across several children.
///
/// Used by `--all-interfaces`: one [`UdpTransport`](super::UdpTransport)
/// per usable network interface, composed here so `Client` sees one
/// `Transport` regardless of how many sockets back it.
pub struct MultiTransport {
    children: Vec<Box<dyn Transport>>,
}

impl MultiTransport {
    /// Composes `children` into one fan-out transport.
    ///
    /// An empty `children` is accepted (so this constructor never fails),
    /// but `send` then has nothing to fan out to and `recv` returns an
    /// error immediately rather than hanging forever. Callers that build
    /// children from interface enumeration — [`Client::for_all_interfaces`](
    /// crate::Client::for_all_interfaces) — reject an empty list themselves
    /// so the error they surface can say "no usable interfaces".
    #[must_use]
    pub fn new(children: Vec<Box<dyn Transport>>) -> Self {
        MultiTransport { children }
    }
}

#[async_trait]
impl Transport for MultiTransport {
    /// Sends `packet` on every child. Succeeds if any child succeeded;
    /// fails only if every child did. Per-child failures are logged at
    /// warn, matching `multi_transport.go`'s `Send`.
    async fn send(&self, packet: &[u8]) -> Result<(), TransportError> {
        let mut failures = Vec::new();
        let mut successes = 0usize;
        for (index, child) in self.children.iter().enumerate() {
            match child.send(packet).await {
                Ok(()) => successes += 1,
                Err(e) => {
                    warn!(child = index, error = %e, "MultiTransport: child send failed");
                    failures.push(e.to_string());
                }
            }
        }
        if successes == 0 {
            return Err(TransportError::Io(std::io::Error::other(format!(
                "all children failed: {failures:?}"
            ))));
        }
        Ok(())
    }

    /// Returns the next packet from whichever child produces one first,
    /// or the cancellation error if `cancel` fires first — see the
    /// module docs for why this cannot lose or duplicate a reply.
    async fn recv(&self, cancel: &CancellationToken) -> Result<(Vec<u8>, String), TransportError> {
        if self.children.is_empty() {
            return Err(TransportError::Io(std::io::Error::other(
                "MultiTransport has no children",
            )));
        }
        let (result, _index, _still_pending) =
            select_all(self.children.iter().map(|child| child.recv(cancel))).await;
        result
    }

    /// Closes every child. Returns the first error, if any, matching
    /// `multi_transport.go`'s `Close`.
    async fn close(&self) -> Result<(), TransportError> {
        let mut first_err = None;
        for child in &self.children {
            if let Err(e) = child.close().await
                && first_err.is_none()
            {
                first_err = Some(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Chatty {
        sends: Arc<AtomicUsize>,
        reply: Vec<u8>,
        fail_send: bool,
    }

    #[async_trait]
    impl Transport for Chatty {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if self.fail_send {
                return Err(TransportError::Io(std::io::Error::other("nope")));
            }
            Ok(())
        }
        async fn recv(
            &self,
            cancel: &CancellationToken,
        ) -> Result<(Vec<u8>, String), TransportError> {
            if self.reply.is_empty() {
                cancel.cancelled().await;
                return Err(TransportError::Cancelled);
            }
            Ok((self.reply.clone(), "10.0.0.1".to_owned()))
        }
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    fn child(sends: &Arc<AtomicUsize>, reply: &[u8], fail_send: bool) -> Box<dyn Transport> {
        Box::new(Chatty {
            sends: Arc::clone(sends),
            reply: reply.to_vec(),
            fail_send,
        })
    }

    #[tokio::test]
    async fn send_fans_out_to_every_child() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![
            child(&sends, b"", false),
            child(&sends, b"", false),
            child(&sends, b"", false),
        ]);
        m.send(b"x").await.expect("send");
        assert_eq!(sends.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn send_succeeds_when_any_child_succeeds() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", false)]);
        assert!(m.send(b"x").await.is_ok(), "one success is enough");
    }

    #[tokio::test]
    async fn send_fails_only_when_every_child_fails() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", true), child(&sends, b"", true)]);
        assert!(m.send(b"x").await.is_err(), "all failed, so send must fail");
    }

    #[tokio::test]
    async fn recv_merges_replies_from_children() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"reply-a", false)]);
        let cancel = CancellationToken::new();
        let (pkt, src) = m.recv(&cancel).await.expect("a merged reply");
        assert_eq!(pkt, b"reply-a");
        assert_eq!(src, "10.0.0.1");
    }

    #[tokio::test]
    async fn recv_returns_cancelled_when_the_token_fires() {
        let sends = Arc::new(AtomicUsize::new(0));
        let m = MultiTransport::new(vec![child(&sends, b"", false)]);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = m.recv(&cancel).await.expect_err("must not block");
        assert!(matches!(err, TransportError::Cancelled));
    }
}
