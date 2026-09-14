//! `get_ip` (UCP 0x0002): a device's active network configuration.

use crate::device::Device;
use crate::netconfig::NetworkConfig;
use crate::ops::{OpError, reply_error};
use crate::protocol::method;
use crate::session::Session;
use std::net::Ipv4Addr;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// `get_ip` reply TLV codes, per `Net::UDAP` `Constant.pm`.
const TLV_IP_ADDR: u8 = 0x05;
const TLV_SUBNET_MASK: u8 = 0x06;
const TLV_GATEWAY_ADDR: u8 = 0x07;

/// Decodes a `get_ip` reply.
///
/// Unrecognised tags are skipped, and anything whose value is not exactly
/// four bytes is ignored — go-udap only assigns when `length == 4`. Never
/// fails: go-udap returns a nil error here, and an empty config is a
/// legitimate answer from a device with no address yet.
///
/// Uses [`crate::tlv::decode`] rather than a hand-rolled walk. go-udap
/// re-implements the walk per file, but that is internal structure, not
/// observable behaviour, and `tlv::decode` is already proptest-fuzzed for
/// bounds and panic-safety.
fn parse_response(data: &[u8]) -> NetworkConfig {
    let mut config = NetworkConfig::default();
    for entry in crate::tlv::decode(data) {
        let Ok(octets) = <[u8; 4]>::try_from(entry.value) else {
            continue;
        };
        let addr = Ipv4Addr::from(octets);
        match entry.tag {
            TLV_IP_ADDR => config.ip = Some(addr),
            TLV_SUBNET_MASK => config.subnet_mask = Some(addr),
            TLV_GATEWAY_ADDR => config.gateway = Some(addr),
            _ => {}
        }
    }
    config
}

/// Queries a device's active network configuration.
///
/// # Errors
/// [`OpError::Send`] or [`OpError::Recv`] on transport failure, or one of
/// the device-reply variants if the device answers with anything but
/// `get_ip`.
pub async fn get_ip(
    session: &Session,
    cancel: &CancellationToken,
    device: &Device,
) -> Result<NetworkConfig, OpError> {
    let packet = session.header(device.mac, method::GET_IP, false).to_bytes();
    session.send_retried(&packet).await.map_err(OpError::Send)?;
    info!(mac = %device.mac, "sent GetIP request");

    let (reply, payload) = session.wait_for_reply(cancel, device).await?;
    if reply.ucp_method == method::GET_IP {
        Ok(parse_response(&payload))
    } else {
        Err(reply_error(device, &reply, &payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_three_addresses() {
        let data = [
            0x05, 4, 192, 168, 1, 50, //
            0x06, 4, 255, 255, 255, 0, //
            0x07, 4, 192, 168, 1, 1,
        ];
        let nc = parse_response(&data);
        assert_eq!(nc.ip, Some(Ipv4Addr::new(192, 168, 1, 50)));
        assert_eq!(nc.subnet_mask, Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(nc.gateway, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn an_omitted_gateway_stays_none() {
        let data = [0x05, 4, 10, 0, 0, 5, 0x06, 4, 255, 0, 0, 0];
        let nc = parse_response(&data);
        assert_eq!(nc.gateway, None, "a missing TLV must not invent an address");
        assert_eq!(nc.ip, Some(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn a_wrong_length_value_is_ignored() {
        // go-udap only assigns when length == 4.
        let data = [0x05, 3, 10, 0, 0];
        assert_eq!(parse_response(&data).ip, None);
    }

    #[test]
    fn a_truncated_tlv_stops_decoding_without_panicking() {
        // Length claims 4 bytes but only 2 follow.
        let data = [0x05, 4, 10, 0];
        assert_eq!(parse_response(&data), NetworkConfig::default());
    }

    #[test]
    fn empty_input_yields_an_empty_config() {
        assert_eq!(parse_response(&[]), NetworkConfig::default());
    }
}
