use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use udap::Mac;
use udap::protocol::{ADDR_TYPE_ETH, Packet, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};
use udap::transport::Transport;

fn adv_discovery_request() -> Vec<u8> {
    Packet {
        dst_broadcast: 1,
        dst_type: ADDR_TYPE_ETH,
        dst_address: Mac::ZERO,
        src_broadcast: 0,
        src_type: ADDR_TYPE_ETH,
        src_address: Mac::ZERO,
        sequence: 1,
        udap_type: UDAP_TYPE_UCP,
        ucp_flags: 0x01,
        uap_class: UAP_CLASS_UCP,
        ucp_method: method::ADV_DISC,
    }
    .to_bytes()
    .to_vec()
}

#[tokio::test]
async fn mock_answers_advanced_discovery() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    transport.send(&adv_discovery_request()).await.unwrap();
    let (reply, _src) = transport.recv(&cancel).await.unwrap();

    let (packet, payload) = Packet::from_bytes(&reply).unwrap();
    assert_eq!(packet.ucp_method, method::ADV_DISC);
    assert_eq!(packet.ucp_flags, 0x00, "replies clear the request bit");
    assert_eq!(packet.src_address.to_string(), "00:04:20:00:00:01");
    assert_eq!(packet.sequence, 1, "sequence is echoed");
    assert!(!payload.is_empty());
}

#[tokio::test]
async fn every_device_replies() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(3));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    transport.send(&adv_discovery_request()).await.unwrap();
    let mut macs = Vec::new();
    for _ in 0..3 {
        let (reply, _) = transport.recv(&cancel).await.unwrap();
        let (packet, _) = Packet::from_bytes(&reply).unwrap();
        macs.push(packet.src_address.to_string());
    }
    macs.sort();
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
async fn recv_returns_cancelled_when_the_token_fires() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let err = transport.recv(&cancel).await.unwrap_err();
    assert!(matches!(err, udap::transport::TransportError::Cancelled));
}

#[tokio::test]
async fn non_discovery_methods_are_ignored_for_now() {
    let network = Arc::new(mocksbr::Network::with_auto_devices(1));
    let transport = mocksbr::MockTransport::new(network);
    let cancel = CancellationToken::new();

    let mut request = adv_discovery_request();
    request[25..27].copy_from_slice(&method::GET_DATA.to_be_bytes());
    transport.send(&request).await.unwrap();

    cancel.cancel();
    assert!(
        transport.recv(&cancel).await.is_err(),
        "no reply should be queued"
    );
}
