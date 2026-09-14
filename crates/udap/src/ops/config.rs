//! Configuration operations: `get_data` (0x0005) and `reset` (0x0004).
//!
//! go-udap's error handling differs per operation — `get` does not
//! extract the error-message TLV that `get_ip` does, and `reset` words
//! its rejection differently again. Those inconsistencies are carried
//! forward rather than unified: the text is user-visible.

use crate::device::Device;
use crate::ops::{OpError, TLV_ERROR_MESSAGE};
use crate::parameters;
use crate::protocol::method;
use crate::session::Session;
use crate::transport::TransportError;
use crate::{getdata, tlv};
use std::collections::BTreeMap;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Username and password fields, both all-zero, that precede the item
/// list in a `get_data`/`set_data` request.
const CREDENTIAL_FIELDS: usize = 32;

/// Builds a `get_data` request body for `params`.
///
/// Unknown names are skipped with a warning, matching go-udap — the CLI
/// can carry a parameter this table does not know. Items are sorted by
/// offset, which is part of the wire contract.
fn get_data_payload(params: &[&str]) -> Vec<u8> {
    let mut items = Vec::with_capacity(params.len());
    for name in params {
        if let Some(p) = parameters::by_name(name) {
            items.push(p);
        } else {
            tracing::warn!(param = name, "unknown parameter skipped");
        }
    }
    items.sort_by_key(|p| p.offset);

    let mut out = vec![0u8; CREDENTIAL_FIELDS];
    let count = u16::try_from(items.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&count.to_be_bytes());
    for item in items {
        out.extend_from_slice(&item.offset.to_be_bytes());
        out.extend_from_slice(&item.length.to_be_bytes());
    }
    out
}

/// Reads the named parameters from a device.
///
/// # Errors
/// [`OpError::ZeroMac`] if the device has no MAC, transport errors, or —
/// note the narrower switch than [`crate::ops::getip::get_ip`] — a bare
/// [`OpError::DeviceNoMessage`] on an error reply. go-udap does not
/// decode the error TLV here, and does not handle a credentials error at
/// all.
pub async fn get(
    session: &Session,
    cancel: &CancellationToken,
    device: &Device,
    params: &[&str],
) -> Result<BTreeMap<String, Vec<u8>>, OpError> {
    if device.mac.is_zero() {
        return Err(OpError::ZeroMac {
            operation: "GetData",
        });
    }
    let mut packet = session
        .header(device.mac, method::GET_DATA, false)
        .to_bytes()
        .to_vec();
    packet.extend_from_slice(&get_data_payload(params));
    session.send_retried(&packet).await.map_err(OpError::Send)?;
    info!(mac = %device.mac, param_count = params.len(), "sent GetData request");

    let (reply, payload) = session.wait_for_reply(cancel, device).await?;
    let mac = device.mac.to_string();
    match reply.ucp_method {
        method::GET_DATA => {
            getdata::parse_response(&payload).map_err(|source| OpError::Decode { mac, source })
        }
        method::ERROR => Err(OpError::DeviceNoMessage { mac }),
        other => Err(OpError::UnexpectedMethod { mac, method: other }),
    }
}

/// Reads every known parameter into `device.parameters`.
///
/// Returns `()`: the mutation *is* the result, matching go-udap, whose
/// caller (`cli/read.go`) reads `device.Parameters` back afterwards.
///
/// # Errors
/// Whatever [`get`] returns.
pub async fn get_all(
    session: &Session,
    cancel: &CancellationToken,
    device: &mut Device,
) -> Result<(), OpError> {
    let names: Vec<&str> = parameters::names().collect();
    let config = get(session, cancel, device, &names).await?;

    // Clear synthetic offset_NNN keys before merging. parse_response
    // emits them for NVRAM offsets the parameter table does not know, and
    // without this a consumer calling get_all repeatedly across firmware
    // variations would accumulate them without bound.
    device.parameters.retain(|k, _| !k.starts_with("offset_"));
    device.parameters.extend(config);

    info!(
        mac = %device.mac,
        param_count = device.parameters.len(),
        "read parameters from device"
    );
    Ok(())
}

/// Resets a device to factory defaults.
///
/// A cancelled wait is **success**: the device may have reset before it
/// could acknowledge, and go-udap treats that as done rather than as a
/// failure.
///
/// # Errors
/// [`OpError::ZeroMac`] if the device has no MAC,
/// [`OpError::ResetRejected`] or [`OpError::ResetRejectedNoMessage`] if
/// the device refuses, [`OpError::UnexpectedMethod`] otherwise.
pub async fn reset(
    session: &Session,
    cancel: &CancellationToken,
    device: &Device,
) -> Result<(), OpError> {
    // Before the cancellation-means-success branch below, not after: an
    // unaddressed reset would otherwise wait, observe cancellation, and
    // be reported as a successful reset of a device that was never asked.
    if device.mac.is_zero() {
        return Err(OpError::ZeroMac { operation: "Reset" });
    }
    let packet = session.header(device.mac, method::RESET, false).to_bytes();
    session.send_retried(&packet).await.map_err(OpError::Send)?;
    info!(mac = %device.mac, "sent Reset");

    let (reply, payload) = match session.wait_for_reply(cancel, device).await {
        Ok(got) => got,
        // The reset landed and the device rebooted before replying. UDP
        // has no teardown, so a real transport error is still an error;
        // only cancellation means "probably fine".
        Err(OpError::Recv(TransportError::Cancelled)) => {
            info!("no reset acknowledgment; device may have reset immediately");
            return Ok(());
        }
        Err(e) => return Err(e),
    };

    let mac = device.mac.to_string();
    match reply.ucp_method {
        method::RESET => {
            info!("device acknowledged reset");
            Ok(())
        }
        method::ERROR => {
            for entry in tlv::decode(&payload) {
                if entry.tag == TLV_ERROR_MESSAGE {
                    return Err(OpError::ResetRejected {
                        mac,
                        message: String::from_utf8_lossy(entry.value).into_owned(),
                    });
                }
            }
            Err(OpError::ResetRejectedNoMessage { mac })
        }
        other => Err(OpError::UnexpectedMethod { mac, method: other }),
    }
}

/// Builds a `set_data` request body for `params`.
///
/// Layout mirrors `get_data`'s, with a value after each offset/length
/// pair. `Parameter::encode` owns the wire encoding and guarantees
/// exactly `length` bytes on success, so there is no separate padding
/// step. Unknown names are skipped with a warning; items are sorted by
/// offset, which the fidelity contract names explicitly.
///
/// # Errors
/// [`OpError::Encode`] if a value will not fit its parameter, naming the
/// parameter as go-udap does (`param %q: %w`).
fn set_data_payload(params: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, OpError> {
    let mut items = Vec::with_capacity(params.len());
    for (name, value) in params {
        if let Some(p) = parameters::by_name(name) {
            items.push((p, value));
        } else {
            tracing::warn!(param = %name, "unknown parameter skipped");
        }
    }
    items.sort_by_key(|(p, _)| p.offset);

    let mut out = vec![0u8; CREDENTIAL_FIELDS];
    let count = u16::try_from(items.len()).unwrap_or(u16::MAX);
    out.extend_from_slice(&count.to_be_bytes());
    for (param, value) in items {
        let encoded = param.encode(value)?;
        out.extend_from_slice(&param.offset.to_be_bytes());
        out.extend_from_slice(&param.length.to_be_bytes());
        out.extend_from_slice(&encoded);
    }
    Ok(out)
}

/// Writes `config` to a device, preserving everything else.
///
/// Read-modify-write. Omitting a parameter from a `set_data` request
/// writes zeros over the neighbouring NVRAM region, so the device's
/// current values are read first and `config` overlaid on top. If
/// `device.parameters` is already populated — as `cli/set.go` arranges —
/// that prelude read is skipped, turning two round trips into one.
///
/// A failed prelude read aborts the whole operation. go-udap's earlier
/// warn-and-continue path produced exactly the partial write the read
/// exists to prevent.
///
/// # Errors
/// [`OpError::ZeroMac`], [`OpError::Encode`] if a value will not fit, or
/// the device-reply variants. Note this switch is the four-arm one, like
/// [`crate::ops::getip::get_ip`] and unlike [`get`] directly above.
pub async fn set(
    session: &Session,
    cancel: &CancellationToken,
    device: &mut Device,
    config: &BTreeMap<String, Vec<u8>>,
) -> Result<(), OpError> {
    if device.mac.is_zero() {
        return Err(OpError::ZeroMac {
            operation: "SetData",
        });
    }
    if device.parameters.is_empty() {
        info!("device parameters not loaded, reading current configuration");
        get_all(session, cancel, device).await?;
    }

    let mut merged = device.parameters.clone();
    merged.extend(config.iter().map(|(k, v)| (k.clone(), v.clone())));

    let mut packet = session
        .header(device.mac, method::SET_DATA, false)
        .to_bytes()
        .to_vec();
    packet.extend_from_slice(&set_data_payload(&merged)?);
    session.send_retried(&packet).await.map_err(OpError::Send)?;
    info!(mac = %device.mac, total_params = merged.len(), "sent SetData request");

    let (reply, payload) = session.wait_for_reply(cancel, device).await?;
    match reply.ucp_method {
        // go-udap accepts three acknowledgement methods here, not one.
        method::SET_DATA | method::GET_DATA | method::GET_IP => {
            // The commit barrier. Merging before the acknowledgement left
            // device.parameters advertising values that were never
            // persisted whenever the round trip failed.
            //
            // Carried-forward wart: this merges the caller's whole map,
            // including names `set_data_payload` skipped because the
            // parameter table does not know them. Those were never sent,
            // so the cache ends up showing a value the device never
            // received — the same shape of staleness the barrier exists
            // to prevent. go-udap does the same (`maps.Copy(device
            // .Parameters, config)` against a filtered packet), so it is
            // reproduced rather than fixed. See the spec's known warts.
            device
                .parameters
                .extend(config.iter().map(|(k, v)| (k.clone(), v.clone())));
            info!(
                method = format!("0x{:04x}", reply.ucp_method),
                "device acknowledged configuration change"
            );
            Ok(())
        }
        _ => Err(crate::ops::reply_error(device, &reply, &payload)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads back the (offset, length) pairs a request body carries.
    fn items_of(payload: &[u8]) -> Vec<(u16, u16)> {
        let mut pos = CREDENTIAL_FIELDS;
        let count = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        pos += 2;
        let mut out = Vec::new();
        for _ in 0..count {
            out.push((
                u16::from_be_bytes([payload[pos], payload[pos + 1]]),
                u16::from_be_bytes([payload[pos + 2], payload[pos + 3]]),
            ));
            pos += 4;
        }
        out
    }

    #[test]
    fn the_request_opens_with_zeroed_credential_fields() {
        let payload = get_data_payload(&["hostname"]);
        assert_eq!(
            &payload[..CREDENTIAL_FIELDS],
            &[0u8; CREDENTIAL_FIELDS],
            "16 bytes of username then 16 of password, all zero"
        );
    }

    #[test]
    fn items_are_sorted_by_offset() {
        // Part of the fidelity contract: "the sort-by-offset in
        // get_data/set_data". Requested in a deliberately unsorted order.
        let payload = get_data_payload(&["wireless_SSID", "lan_ip_mode", "hostname"]);
        let offsets: Vec<u16> = items_of(&payload).into_iter().map(|(o, _)| o).collect();
        let mut sorted = offsets.clone();
        sorted.sort_unstable();
        assert_eq!(offsets, sorted, "items must be ordered by NVRAM offset");
        assert_eq!(offsets.len(), 3);
    }

    #[test]
    fn each_item_carries_its_parameters_width() {
        let payload = get_data_payload(&["lan_ip_mode", "lan_gateway"]);
        for (offset, length) in items_of(&payload) {
            let param = parameters::by_offset(offset).expect("offset came from the table");
            assert_eq!(length, param.length, "{} width", param.name);
        }
    }

    #[test]
    fn an_unknown_name_is_skipped_rather_than_encoded() {
        let payload = get_data_payload(&["not_a_real_parameter", "hostname"]);
        assert_eq!(items_of(&payload).len(), 1);
    }

    #[test]
    fn no_parameters_yields_a_zero_count() {
        let payload = get_data_payload(&[]);
        assert!(items_of(&payload).is_empty());
        assert_eq!(payload.len(), CREDENTIAL_FIELDS + 2, "just the count");
    }
}
