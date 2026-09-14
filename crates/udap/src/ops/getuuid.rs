//! `get_uuid` (UCP 0x000b): a device's 16-byte UUID.

use crate::device::Device;
use crate::hex;
use crate::ops::{OpError, reply_error};
use crate::protocol::method;
use crate::session::Session;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// `get_uuid` reply TLV code, per `Net::UDAP` `Constant.pm`. The same
/// code discovery uses for the UUID.
const TLV_UUID: u8 = 0x0d;
const UUID_LEN: usize = 16;

/// Decodes a `get_uuid` reply to lowercase hex.
///
/// # Errors
/// [`OpError::MissingUuid`] if no 16-byte UUID TLV is present — matching
/// go-udap, which rejects a wrong-length UUID rather than padding it.
fn parse_response(data: &[u8]) -> Result<String, OpError> {
    for entry in crate::tlv::decode(data) {
        if entry.tag == TLV_UUID && entry.value.len() == UUID_LEN {
            return Ok(hex::encode(entry.value));
        }
    }
    Err(OpError::MissingUuid)
}

/// Queries a device's UUID, hex-encoded.
///
/// # Errors
/// [`OpError::Send`] or [`OpError::Recv`] on transport failure,
/// [`OpError::MissingUuid`] if the reply carries no UUID TLV, or one of
/// the device-reply variants if the device answers with anything but
/// `get_uuid`.
pub async fn get_uuid(
    session: &Session,
    cancel: &CancellationToken,
    device: &Device,
) -> Result<String, OpError> {
    let packet = session
        .header(device.mac, method::GET_UUID, false)
        .to_bytes();
    session.send_retried(&packet).await.map_err(OpError::Send)?;
    info!(mac = %device.mac, "sent GetUUID request");

    let (reply, payload) = session.wait_for_reply(cancel, device).await?;
    if reply.ucp_method == method::GET_UUID {
        parse_response(&payload)
    } else {
        Err(reply_error(device, &reply, &payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_16_byte_uuid_as_lowercase_hex() {
        let mut data = vec![TLV_UUID, 16];
        data.extend_from_slice(&[
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, //
            0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ]);
        let uuid = parse_response(&data).expect("a 16-byte UUID TLV decodes");
        assert_eq!(uuid, "0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn a_wrong_length_uuid_is_not_accepted() {
        let data = [TLV_UUID, 8, 1, 2, 3, 4, 5, 6, 7, 8];
        assert!(matches!(parse_response(&data), Err(OpError::MissingUuid)));
    }

    #[test]
    fn a_missing_uuid_tlv_is_an_error() {
        let data = [0x05, 4, 10, 0, 0, 1];
        assert!(matches!(parse_response(&data), Err(OpError::MissingUuid)));
    }
}
