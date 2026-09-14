//! The UCP operations, as free functions over a [`crate::Session`] and a
//! caller-held [`crate::Device`].
//!
//! Free functions rather than `Client` methods because an operation needs
//! the transport and *one* device, never the registry — and `&Client`
//! plus `&mut Client.devices[..]` is two overlapping borrows. `Client`
//! wraps each of these so call sites still read like go-udap's.

pub mod config;
pub mod getip;
pub mod getuuid;

use crate::device::Device;
use crate::protocol::Packet;
use crate::tlv;
use crate::transport::TransportError;

/// Why an operation failed.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("send request: {0}")]
    Send(#[source] TransportError),
    #[error("recv reply: {0}")]
    Recv(#[source] TransportError),
    #[error("build packet: {0}")]
    Encode(#[from] crate::error::EncodeError),
    /// A `get_data` reply that would not decode.
    ///
    /// Names the device, matching go-udap's
    /// `decode GetData response from %s: %w`.
    #[error("decode GetData response from {mac}: {source}")]
    Decode {
        mac: String,
        #[source]
        source: crate::error::GetDataError,
    },
    /// A device that refused a reset, with its explanation.
    ///
    /// Distinct wording from [`OpError::Device`]: go-udap says "rejected
    /// reset" here and "error" elsewhere. Carried forward rather than
    /// unified.
    #[error("device {mac} rejected reset: {message}")]
    ResetRejected { mac: String, message: String },
    /// A device that refused a reset without saying why.
    #[error("device {mac} rejected reset")]
    ResetRejectedNoMessage { mac: String },
    /// A request that cannot be built.
    ///
    /// go-udap: `cannot build GetData packet: device has zero MAC address`.
    #[error("cannot build {operation} packet: device has zero MAC address")]
    ZeroMac { operation: &'static str },
    /// The device answered with UCP method 0x0007 and an error TLV.
    ///
    /// Text is fidelity-contract: go-udap formats this as
    /// `device %s error: %s` (`udap/config.go`).
    #[error("device {mac} error: {message}")]
    Device { mac: String, message: String },
    /// The device answered 0x0007 with no error-message TLV.
    #[error("device {mac} returned error response")]
    DeviceNoMessage { mac: String },
    /// The device answered 0x0008.
    #[error("device {mac} rejected credentials")]
    CredentialsRejected { mac: String },
    /// The device answered with a method the operation does not expect.
    #[error("device {mac}: unexpected response method 0x{method:04x}")]
    UnexpectedMethod { mac: String, method: u16 },
    #[error("get_uuid response missing UUID TLV")]
    MissingUuid,
}

/// Error-message TLV code, per `Net::UDAP` `Constant.pm`.
pub(crate) const TLV_ERROR_MESSAGE: u8 = 0x03;

/// Maps a non-success reply method onto the matching [`OpError`].
///
/// Shared by every operation: go-udap repeats the same four-arm switch
/// in `config.go`, `getip.go` and `getuuid.go`, and all three produce
/// identical text. `expected` is the method that would have meant
/// success, so anything else is unexpected.
pub(crate) fn reply_error(device: &Device, packet: &Packet, payload: &[u8]) -> OpError {
    let mac = device.mac.to_string();
    match packet.ucp_method {
        crate::protocol::method::ERROR => {
            for entry in tlv::decode(payload) {
                if entry.tag == TLV_ERROR_MESSAGE {
                    return OpError::Device {
                        mac,
                        // Device bytes carry no encoding guarantee, but
                        // this one is destined for a message: go-udap
                        // does `string(tlv.Value)`, which is equally
                        // lossy on invalid UTF-8.
                        message: String::from_utf8_lossy(entry.value).into_owned(),
                    };
                }
            }
            OpError::DeviceNoMessage { mac }
        }
        crate::protocol::method::CREDENTIALS_ERROR => OpError::CredentialsRejected { mac },
        method => OpError::UnexpectedMethod { mac, method },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mac;
    use crate::protocol::{ADDR_TYPE_ETH, UAP_CLASS_UCP, UDAP_TYPE_UCP, method};

    const MAC: [u8; 6] = [0x00, 0x04, 0x20, 0x16, 0x17, 0x18];

    /// The wire code for an error message, written out rather than read
    /// from `TLV_ERROR_MESSAGE`. Building the payload from the same
    /// constant the code reads makes the test move with the bug: changing
    /// the constant would keep every assertion green. `Net::UDAP`
    /// `Constant.pm` fixes this at 0x03.
    const WIRE_ERROR_MESSAGE_TAG: u8 = 0x03;

    fn device() -> Device {
        Device {
            mac: Mac::from_bytes(MAC),
            ..Device::default()
        }
    }

    fn reply(ucp_method: u16) -> Packet {
        Packet {
            dst_broadcast: 0,
            dst_type: ADDR_TYPE_ETH,
            dst_address: Mac::ZERO,
            src_broadcast: 0,
            src_type: ADDR_TYPE_ETH,
            src_address: Mac::from_bytes(MAC),
            sequence: 1,
            udap_type: UDAP_TYPE_UCP,
            ucp_flags: 0,
            uap_class: UAP_CLASS_UCP,
            ucp_method,
        }
    }

    #[test]
    fn an_error_reply_carrying_a_message_reports_it() {
        let mut payload = Vec::new();
        tlv::encode_into(WIRE_ERROR_MESSAGE_TAG, b"no such parameter", &mut payload);
        let err = reply_error(&device(), &reply(method::ERROR), &payload);
        assert_eq!(
            err.to_string(),
            "device 00:04:20:16:17:18 error: no such parameter"
        );
    }

    #[test]
    fn an_error_reply_without_a_message_still_reports_the_device() {
        let err = reply_error(&device(), &reply(method::ERROR), &[]);
        assert_eq!(
            err.to_string(),
            "device 00:04:20:16:17:18 returned error response"
        );
    }

    #[test]
    fn an_error_reply_whose_tlvs_are_all_other_tags_has_no_message() {
        // Only tag 0x03 is the error message; anything else is not.
        let mut payload = Vec::new();
        tlv::encode_into(0x09, b"firmware", &mut payload);
        let err = reply_error(&device(), &reply(method::ERROR), &payload);
        assert_eq!(
            err.to_string(),
            "device 00:04:20:16:17:18 returned error response"
        );
    }

    #[test]
    fn a_credentials_error_is_distinct_from_a_plain_error() {
        let err = reply_error(&device(), &reply(method::CREDENTIALS_ERROR), &[]);
        assert_eq!(
            err.to_string(),
            "device 00:04:20:16:17:18 rejected credentials"
        );
    }

    #[test]
    fn any_other_method_reports_the_method_it_saw() {
        let err = reply_error(&device(), &reply(method::DISCOVER), &[]);
        assert_eq!(
            err.to_string(),
            "device 00:04:20:16:17:18: unexpected response method 0x0001"
        );
    }

    #[test]
    fn a_non_utf8_error_message_does_not_lose_the_error() {
        // Device bytes carry no encoding guarantee. go-udap does
        // string(tlv.Value), which is equally lossy; the point is that
        // the error survives rather than being dropped.
        let mut payload = Vec::new();
        tlv::encode_into(WIRE_ERROR_MESSAGE_TAG, &[0xff, 0xfe], &mut payload);
        let err = reply_error(&device(), &reply(method::ERROR), &payload);
        assert!(
            err.to_string()
                .starts_with("device 00:04:20:16:17:18 error: "),
            "got {err}"
        );
    }
}
