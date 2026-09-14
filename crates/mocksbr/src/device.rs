//! One virtual Squeezebox Receiver.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use udap::Mac;

/// Per-device configuration, including the fault-injection knobs.
#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub mac: Mac,
    /// Reported as TLV 0x02 (`device_name`).
    pub name: String,
    /// Reported as TLV 0x03 (`device_type`).
    pub model: String,
    /// Reported as TLV 0x0b (`device_id`). "07" is a Receiver.
    pub device_id: String,
    /// Reported as TLV 0x09 (`firmware_rev`).
    pub firmware: String,
    /// Reported as TLV 0x0a (`hardware_rev`).
    pub hardware: String,
    /// Reported as TLV 0x0c (`device_status`).
    pub state: String,
    /// Reported by `get_ip` as TLV 0x05.
    pub ip: Ipv4Addr,
    /// Reported by `get_ip` as TLV 0x06.
    pub subnet_mask: Ipv4Addr,
    /// Reported by `get_ip` as TLV 0x07.
    pub gateway: Ipv4Addr,
    /// Reported by `get_uuid` as TLV 0x0d. Sixteen bytes.
    pub uuid: [u8; 16],
    /// Fault injection: operations the device rejects with UCP 0x0007.
    ///
    /// Two aliasing rules, both go-udap's (`mocksbr/device.go:217`):
    /// naming either [`Op::Set`] or [`Op::Save`] rejects the other, as
    /// they are one wire method; and [`Op::Discover`] makes the device
    /// skip discovery *silently* rather than answering with an error,
    /// because a broadcast has no requester to reply to.
    pub fail_on: Vec<Op>,
    /// The rejection message.
    ///
    /// `None` uses go-udap's `mocksbr: configured to fail <op>`.
    /// `Some("")` sends an error reply carrying **no** TLV at all, which
    /// is a distinct client path (`OpError::DeviceNoMessage`) that
    /// go-udap has no way to produce and does not test.
    pub fail_message: Option<String>,
    /// Fault injection: the device answers nothing at all, including
    /// discovery. Models a device that is off the network, as distinct
    /// from `fail_on`, which models one that refuses a request.
    pub unreachable: bool,
    /// Fault injection: `get_ip` requests get no reply.
    pub drop_get_ip: bool,
    /// Fault injection: `get_uuid` requests get no reply.
    pub drop_get_uuid: bool,
    /// Fault injection: a deliberately broken `get_data` reply.
    pub malformed: Malformed,
    /// NVRAM values overriding the factory defaults.
    ///
    /// Seeds **both** tiers, so a device starts as though it had been
    /// configured and saved — a reset reloads these, not the factory
    /// table.
    pub nvram: BTreeMap<String, Vec<u8>>,
}

impl DeviceConfig {
    /// Whether `op` is configured to fail.
    ///
    /// Ports go-udap's `failsOn` (`mocksbr/device.go:217`), including
    /// its two-way `Set`/`Save` alias: they are one wire method, so
    /// naming either rejects both. `from_method` never yields `Save`,
    /// so only the `Set`-requested direction is reachable over the
    /// wire; the other is kept because the Go has it.
    pub(crate) fn fails_on(&self, op: Op) -> bool {
        for configured in &self.fail_on {
            if *configured == op {
                return true;
            }
            if (op == Op::Set && *configured == Op::Save)
                || (op == Op::Save && *configured == Op::Set)
            {
                return true;
            }
        }
        false
    }

    /// Builds a device with go-udap's mocksbr defaults.
    #[must_use]
    pub fn default_with_mac(mac: Mac) -> Self {
        DeviceConfig {
            mac,
            name: "Mock SBR".to_owned(),
            model: "squeezebox".to_owned(),
            device_id: "07".to_owned(),
            firmware: "77".to_owned(),
            hardware: "0005".to_owned(),
            state: "wait_slimserver".to_owned(),
            // A device in setup mode has no lease, so every address is
            // unspecified — which `NetworkConfig` renders as "-".
            ip: Ipv4Addr::UNSPECIFIED,
            subnet_mask: Ipv4Addr::UNSPECIFIED,
            gateway: Ipv4Addr::UNSPECIFIED,
            // Fixed rather than random, so tests are reproducible.
            uuid: [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
                0x32, 0x10,
            ],
            fail_on: Vec::new(),
            fail_message: None,
            unreachable: false,
            drop_get_ip: false,
            drop_get_uuid: false,
            malformed: Malformed::None,
            nvram: BTreeMap::new(),
        }
    }
}

/// A UDAP operation, for the failure-injection knobs.
///
/// `Set` and `Save` are the same wire method (0x0006) — a real device
/// does both on one request — so `from_method` reports `Set`, and
/// [`DeviceConfig::fails_on`] treats the two as aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Discover,
    Get,
    Set,
    Save,
    Reset,
    GetIp,
    GetUuid,
}

impl Op {
    /// The operation a UCP method denotes, if this mock models it.
    pub(crate) fn from_method(method: u16) -> Option<Op> {
        use udap::protocol::method;
        match method {
            method::ADV_DISC => Some(Op::Discover),
            method::GET_DATA => Some(Op::Get),
            method::SET_DATA => Some(Op::Set),
            method::RESET => Some(Op::Reset),
            method::GET_IP => Some(Op::GetIp),
            method::GET_UUID => Some(Op::GetUuid),
            _ => None,
        }
    }

    /// The name go-udap uses in its failure message.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Op::Discover => "discover",
            Op::Get => "get",
            Op::Set => "set",
            Op::Save => "save",
            Op::Reset => "reset",
            Op::GetIp => "getip",
            Op::GetUuid => "getuuid",
        }
    }
}

/// A deliberately broken reply shape, for exercising the client's
/// decode error paths.
///
/// Applies to `get_data` only, matching go-udap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Malformed {
    /// Well-formed replies.
    #[default]
    None,
    /// Declare 65535 items and write no bodies, so the client's
    /// per-item bounds check fires on the first one.
    OversizedCount,
    /// Declare one item of length 1000 and write nothing, so the
    /// client's "item exceeds payload" check fires.
    LengthExceedsPayload,
    /// Reply with an unrecognised UCP method.
    UnknownMethod,
}
