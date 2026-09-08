//! A network of virtual devices that a `MockTransport` can drive.

use crate::device::DeviceConfig;
use crate::responses;
use udap::Mac;
use udap::protocol::{Packet, method};

pub struct Network {
    devices: Vec<DeviceConfig>,
}

impl Network {
    #[must_use]
    pub fn new(devices: Vec<DeviceConfig>) -> Self {
        Network { devices }
    }

    /// Builds `n` devices with sequential MACs from `00:04:20:00:00:01`.
    ///
    /// # Panics
    /// If `n` exceeds 255.
    #[must_use]
    pub fn with_auto_devices(n: usize) -> Self {
        assert!(n <= 255, "with_auto_devices supports at most 255 devices");
        let devices = (1..=n)
            .map(|i| {
                #[expect(clippy::cast_possible_truncation, reason = "n is asserted <= 255")]
                let last = i as u8;
                let mut cfg = DeviceConfig::default_with_mac(Mac::from_bytes([
                    0x00, 0x04, 0x20, 0x00, 0x00, last,
                ]));
                cfg.name = format!("Mock SBR {i}");
                cfg
            })
            .collect();
        Network { devices }
    }

    /// Handles one request, returning every reply it provokes.
    ///
    /// M2 answers advanced discovery only; other methods produce no
    /// reply, which is also how a real device behaves when it does not
    /// recognise a request.
    #[must_use]
    pub fn receive(&self, packet: &[u8]) -> Vec<(Vec<u8>, String)> {
        let Ok((request, _payload)) = Packet::from_bytes(packet) else {
            return Vec::new();
        };
        if request.ucp_method != method::ADV_DISC {
            return Vec::new();
        }
        self.devices
            .iter()
            .map(|cfg| {
                (
                    responses::discovery_response(&request, cfg),
                    cfg.mac.to_string(),
                )
            })
            .collect()
    }
}
