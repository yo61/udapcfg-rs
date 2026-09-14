//! The wire half of a client: a transport, a retry count, and the
//! sequence counter.
//!
//! Split out of [`crate::Client`] because an operation needs the
//! transport and one device, never the device registry. go-udap keeps
//! both halves in `Client` and relies on pointer aliasing to reconcile
//! them; Rust cannot express that, and it turns out not to need to —
//! nothing re-reads the registry after an operation.

use crate::Mac;
use crate::device::Device;
use crate::ops::OpError;
use crate::protocol::{ADDR_TYPE_ETH, FLAG_REQUEST, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP};
use crate::transport::{Transport, TransportError};
use std::sync::atomic::{AtomicU16, Ordering};
use tracing::{debug, warn};

pub struct Session {
    transport: Box<dyn Transport>,
    retries: usize,
    /// Atomic because every operation holds only `&Session`: `Client`
    /// passes `&self.session` while the caller holds `&mut Device`.
    sequence: AtomicU16,
}

impl Session {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Session {
            transport,
            retries: 0,
            sequence: AtomicU16::new(0),
        }
    }

    /// Sets the number of re-transmissions beyond the initial send.
    pub fn set_retries(&mut self, n: usize) {
        self.retries = n;
    }

    #[must_use]
    pub fn retries(&self) -> usize {
        self.retries
    }

    /// Releases the transport.
    ///
    /// # Errors
    /// Propagates the transport's close error.
    pub async fn close(&self) -> Result<(), TransportError> {
        self.transport.close().await
    }

    /// Waits for the next packet from anyone. Discovery's receive path.
    pub(crate) async fn recv(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(Vec<u8>, String), TransportError> {
        self.transport.recv(cancel).await
    }

    /// Builds a header with the next sequence number.
    pub(crate) fn header(&self, dst: Mac, ucp_method: u16, broadcast: bool) -> Packet {
        // fetch_add returns the value from *before* the addition, while
        // the Go increments then reads — so the first packet must still
        // carry sequence 1. The golden captures were taken at sequence=1;
        // an off-by-one here shows up as a one-byte fixture diff.
        let sequence = self
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        Packet {
            dst_broadcast: u8::from(broadcast),
            dst_type: ADDR_TYPE_ETH,
            dst_address: dst,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::ZERO,
            sequence,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: FLAG_REQUEST,
            uap_class: UAP_CLASS_UCP,
            ucp_method,
        }
    }

    /// Sends `packet`, retransmitting `retries` more times.
    ///
    /// UDP send is fire-and-forget: succeeds if any attempt succeeded,
    /// returns the first error only if every attempt failed.
    ///
    /// # Errors
    /// The first transport error, if every attempt failed.
    pub(crate) async fn send_retried(&self, packet: &[u8]) -> Result<(), TransportError> {
        let mut first_err = None;
        let mut successes = 0usize;
        for _ in 0..=self.retries {
            match self.transport.send(packet).await {
                Ok(()) => successes += 1,
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        match (successes, first_err) {
            (0, Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    /// Waits for a reply from `device`, discarding anything else.
    ///
    /// # Errors
    /// [`OpError::Recv`] on a transport error, including cancellation.
    pub(crate) async fn wait_for_reply(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
        device: &Device,
    ) -> Result<(Packet, Vec<u8>), OpError> {
        loop {
            let (buf, src) = self.transport.recv(cancel).await.map_err(OpError::Recv)?;
            let Ok((packet, payload)) = Packet::from_bytes(&buf) else {
                debug!(src, "ignoring unparseable reply");
                continue;
            };
            if packet.src_address != device.mac {
                debug!(
                    from = %packet.src_address,
                    want = %device.mac,
                    "ignoring reply from a different device"
                );
                continue;
            }
            if !device.ip.is_empty() && src != device.ip {
                warn!(
                    mac = %packet.src_address,
                    src,
                    expected = %device.ip,
                    "ignoring reply with mismatched source"
                );
                continue;
            }
            return Ok((packet, payload.to_vec()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    /// Hands out a scripted sequence of packets, one per `recv`.
    struct ScriptedTransport {
        replies: Mutex<Vec<Vec<u8>>>,
    }

    #[async_trait]
    impl Transport for ScriptedTransport {
        async fn send(&self, _packet: &[u8]) -> Result<(), TransportError> {
            Ok(())
        }
        async fn recv(
            &self,
            _cancel: &CancellationToken,
        ) -> Result<(Vec<u8>, String), TransportError> {
            let mut replies = self
                .replies
                .lock()
                .map_err(|_| TransportError::Io(std::io::Error::other("poisoned")))?;
            if replies.is_empty() {
                return Err(TransportError::Cancelled);
            }
            Ok((replies.remove(0), "mock".to_owned()))
        }
        async fn close(&self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    fn reply_from(mac: Mac) -> Vec<u8> {
        Packet {
            dst_broadcast: 0,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: mac,
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0, // reply: request bit clear
            uap_class: UAP_CLASS_UCP,
            ucp_method: crate::protocol::method::GET_DATA,
        }
        .to_bytes()
        .to_vec()
    }

    #[tokio::test]
    async fn wait_for_reply_skips_other_devices() {
        let wanted = Mac::from_bytes([0x00, 0x04, 0x20, 0x11, 0x11, 0x11]);
        let other = Mac::from_bytes([0x00, 0x04, 0x20, 0x22, 0x22, 0x22]);

        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![reply_from(other), reply_from(wanted)]),
        }));
        let device = Device {
            mac: wanted,
            ..Device::default()
        };

        let (packet, _payload) = session
            .wait_for_reply(&CancellationToken::new(), &device)
            .await
            .expect("the second reply matches");
        assert_eq!(packet.src_address, wanted, "must skip the other device");
    }

    #[tokio::test]
    async fn wait_for_reply_reports_a_transport_error() {
        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![]),
        }));
        let device = Device::default();
        let err = session
            .wait_for_reply(&CancellationToken::new(), &device)
            .await
            .expect_err("an exhausted transport must not hang");
        assert!(matches!(err, OpError::Recv(_)));
    }

    #[tokio::test]
    async fn the_first_packet_carries_sequence_one() {
        // client.rs incremented then read, so the first packet was 1.
        // AtomicU16::fetch_add returns the pre-increment value, so this
        // pins that the +1 compensation is present.
        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![]),
        }));
        assert_eq!(session.header(Mac::ZERO, 0x0009, true).sequence, 1);
        assert_eq!(session.header(Mac::ZERO, 0x0009, true).sequence, 2);
    }

    #[tokio::test]
    async fn the_broadcast_flag_reaches_the_header() {
        // discover() passes true; dropping the flag would silently unset
        // the broadcast bit on the discovery packet.
        let session = Session::new(Box::new(ScriptedTransport {
            replies: Mutex::new(vec![]),
        }));
        assert_eq!(session.header(Mac::ZERO, 0x0009, true).dst_broadcast, 1);
        assert_eq!(session.header(Mac::ZERO, 0x0009, false).dst_broadcast, 0);
    }
}
