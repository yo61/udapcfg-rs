//! A network of virtual devices that a `MockTransport` can drive.

use crate::device::{DeviceConfig, Op};
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
        let state = devices
            .iter()
            .map(|cfg| DeviceState::factory_with(&cfg.nvram))
            .collect();
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
        Network::new(devices)
    }

    /// Handles one request, returning every reply it provokes.
    ///
    /// Answers advanced discovery, `get_ip`, `get_uuid`, `get_data`,
    /// `set_data` and `reset`. Anything else produces no reply, which is
    /// also how a real device behaves when it does not recognise a
    /// request.
    ///
    /// `set_data` and `reset` mutate the addressed device's state, so a
    /// later `get_data` reflects them.
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
            // Off the network entirely: the device answers nothing,
            // discovery included.
            .filter(|(_, cfg)| !cfg.unreachable)
            // Failing discovery skips the device silently rather than
            // answering with an error, because a broadcast has no
            // requester to reply to. go-udap/mocksbr/handlers.go:103.
            .filter(|(_, cfg)| {
                request.ucp_method != method::ADV_DISC || !cfg.fails_on(Op::Discover)
            })
            .filter(|(_, cfg)| {
                request.ucp_method == method::ADV_DISC || request.dst_address == cfg.mac
            })
            .filter_map(|(index, cfg)| {
                // Failure injection short-circuits the operation's own
                // reply, but not discovery: a device that cannot answer
                // get_data is still discoverable. A device failing
                // discovery itself was already skipped above.
                //
                // The message names the *requested* operation, so a
                // device failing only get_ip never claims to have
                // failed a reset.
                let op = Op::from_method(request.ucp_method);
                if let Some(op) = op
                    && cfg.fails_on(op)
                {
                    let message = cfg
                        .fail_message
                        .clone()
                        .unwrap_or_else(|| format!("mocksbr: configured to fail {}", op.as_str()));
                    return Some((
                        responses::error_response(&request, cfg, &message),
                        cfg.mac.to_string(),
                    ));
                }
                let reply = match request.ucp_method {
                    method::ADV_DISC => responses::discovery_response(&request, cfg),
                    // Above the reply arms. A reorder is a compile
                    // error, not a silent bug: rustc reports the
                    // guarded arm as an unreachable pattern, which
                    // RUSTFLAGS=-D warnings promotes to an error.
                    method::GET_IP if cfg.drop_get_ip => return None,
                    method::GET_UUID if cfg.drop_get_uuid => return None,
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
                    method::RESET => {
                        // Reload working memory from NVRAM. go-udap
                        // serves the ack first and then enters the
                        // reboot window; the window itself is plan B.
                        {
                            let mut state = self.state.lock().ok()?;
                            state.get_mut(index)?.apply_reset();
                        }
                        responses::reset_response(&request, cfg)
                    }
                    _ => return None,
                };
                Some((reply, cfg.mac.to_string()))
            })
            .collect()
    }
}
