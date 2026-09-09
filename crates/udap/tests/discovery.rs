use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use udap::{Client, ClientError};

/// Runs discovery with a short deadline, the way the CLI does.
///
/// Returns the `discover` result rather than unwrapping it here: this
/// helper is not itself `#[test]`-attributed, so `clippy.toml`'s
/// `allow-expect-in-tests` wouldn't reach an `.expect()` placed in this
/// frame. Callers `.expect()` at their own `#[tokio::test]` call site,
/// where the exemption applies.
async fn discover_with_deadline(client: &mut Client, timeout: Duration) -> Result<(), ClientError> {
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        token.cancel();
    });
    client.discover(&cancel).await
}

#[tokio::test]
async fn finds_every_mock_device() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(3));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");

    let macs: Vec<String> = client.devices().iter().map(|d| d.mac.to_string()).collect();
    assert_eq!(
        macs,
        [
            "00:04:20:00:00:01",
            "00:04:20:00:00:02",
            "00:04:20:00:00:03"
        ]
    );
}

#[tokio::test]
async fn populates_device_metadata_from_tlvs() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");

    let devices = client.devices();
    let d = devices.first().expect("one device");
    assert_eq!(d.name, "Mock SBR 1");
    assert_eq!(d.firmware, "77");
    assert_eq!(d.hardware_rev, "0005");
    assert_eq!(d.state, "wait_slimserver");
    // device_id "07" maps to the product name, not the raw device_type.
    assert_eq!(d.model, "Squeezebox Receiver");
}

#[tokio::test]
async fn empty_network_discovers_nothing_without_erroring() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(0));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");

    assert!(client.devices().is_empty());
}

#[tokio::test]
async fn devices_are_returned_sorted_by_mac() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(5));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");

    let macs: Vec<String> = client.devices().iter().map(|d| d.mac.to_string()).collect();
    let mut sorted = macs.clone();
    sorted.sort();
    assert_eq!(macs, sorted);
}

#[tokio::test]
async fn rediscovery_does_not_duplicate_devices() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(2));
    let mut client = Client::new(Box::new(mocksbr::MockTransport::new(network)));

    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");
    discover_with_deadline(&mut client, Duration::from_millis(50))
        .await
        .expect("discovery must not error on timeout");

    assert_eq!(client.devices().len(), 2);
}

/// A transport that counts sends and never replies.
///
/// The counter is an `Arc<AtomicUsize>` the test also holds, so the
/// transport itself can be a plain `Box<dyn Transport>` — which is what
/// `Client::new` takes. Boxing an `Arc<dyn Transport>` would give
/// `Box<Arc<dyn Transport>>`, an unrelated type (E0308).
struct CountingTransport {
    sends: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl udap::transport::Transport for CountingTransport {
    async fn send(&self, _packet: &[u8]) -> Result<(), udap::transport::TransportError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn recv(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(Vec<u8>, String), udap::transport::TransportError> {
        cancel.cancelled().await;
        Err(udap::transport::TransportError::Cancelled)
    }
    async fn close(&self) -> Result<(), udap::transport::TransportError> {
        Ok(())
    }
}

#[tokio::test]
async fn retries_n_produces_n_plus_one_sends() {
    let sends = Arc::new(AtomicUsize::new(0));
    let mut client = Client::new(Box::new(CountingTransport {
        sends: Arc::clone(&sends),
    }));
    client.set_retries(2);

    let cancel = CancellationToken::new();
    cancel.cancel();
    let _ = client.discover(&cancel).await;

    assert_eq!(
        sends.load(Ordering::SeqCst),
        3,
        "2 retries means 3 total sends"
    );
}
