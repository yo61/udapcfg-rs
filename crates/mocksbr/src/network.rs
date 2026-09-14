//! A network of virtual devices that a `MockTransport` can drive.

use crate::device::DeviceConfig;
use crate::responses;
use crate::state::DeviceState;
use crate::wire;
use std::collections::BTreeMap;
use std::sync::Mutex;
use udap::Mac;
use udap::protocol::{Packet, method};

pub struct Network {
    devices: Vec<DeviceConfig>,
    /// Per-device parameter state, indexed in step with `devices`.
    ///
    /// A `std::sync::Mutex`, not tokio's: `receive` is synchronous and
    /// holds the lock only while building one reply, so nothing awaits
    /// across it and `clippy::await_holding_lock` does not fire. tokio's
    /// would force `receive` async and ripple into `MockTransport`.
    state: Mutex<Vec<DeviceState>>,
}

impl Network {
    #[must_use]
    pub fn new(devices: Vec<DeviceConfig>) -> Self {
        let state = devices.iter().map(|_| DeviceState::factory()).collect();
        Network {
            devices,
            state: Mutex::new(state),
        }
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
            .collect::<Vec<DeviceConfig>>();
        let state = devices.iter().map(|_| DeviceState::factory()).collect();
        Network {
            devices,
            state: Mutex::new(state),
        }
    }

    /// Handles one request, returning every reply it provokes.
    ///
    /// Answers advanced discovery, `get_ip`, `get_uuid`, `get_data` and
    /// `reset`. Anything else produces no reply, which is also how a real
    /// device behaves when it does not recognise a request.
    #[must_use]
    pub fn receive(&self, packet: &[u8]) -> Vec<(Vec<u8>, String)> {
        let Ok((request, payload)) = Packet::from_bytes(packet) else {
            return Vec::new();
        };
        // Discovery is a broadcast: every device answers. The directed
        // operations answer only if the request names them, matching a
        // real device ignoring traffic addressed elsewhere.
        self.devices
            .iter()
            .enumerate()
            .filter(|(_, cfg)| {
                request.ucp_method == method::ADV_DISC || request.dst_address == cfg.mac
            })
            .filter_map(|(index, cfg)| {
                // Fault injection short-circuits the operation's own
                // reply, but not discovery: a device that cannot answer
                // get_data is still discoverable.
                if let Some(message) = &cfg.error_reply
                    && request.ucp_method != method::ADV_DISC
                {
                    return Some((
                        responses::error_response(&request, cfg, message),
                        cfg.mac.to_string(),
                    ));
                }
                let reply = match request.ucp_method {
                    method::ADV_DISC => responses::discovery_response(&request, cfg),
                    method::GET_IP => responses::get_ip_response(&request, cfg),
                    method::GET_UUID => responses::get_uuid_response(&request, cfg),
                    method::GET_DATA => {
                        let state = self.state.lock().ok()?;
                        responses::get_data_response(&request, cfg, state.get(index)?, payload)
                    }
                    method::SET_DATA => {
                        let items = wire::parse_set_data_request(payload);
                        let mut updates = BTreeMap::new();
                        for item in &items {
                            if let Some(name) = item.name {
                                updates
                                    .insert(name.to_owned(), wire::decode_param_value(&item.value));
                            }
                        }
                        {
                            let mut state = self.state.lock().ok()?;
                            let device = state.get_mut(index)?;
                            device.apply_set(updates);
                            // go-udap saves on every set, so a later
                            // reset observes the most recent values.
                            device.apply_save();
                        }
                        // The ack counts items *parsed*, including any
                        // whose offset the table does not know.
                        let accepted = u16::try_from(items.len()).unwrap_or(u16::MAX);
                        responses::set_data_response(&request, cfg, accepted)
                    }
                    method::RESET => responses::reset_response(&request, cfg),
                    _ => return None,
                };
                Some((reply, cfg.mac.to_string()))
            })
            .collect()
    }
}
