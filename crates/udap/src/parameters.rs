//! The canonical table of UDAP NVRAM parameters.
//!
//! Single source of truth: the CLI flag table, `read` coverage, and the
//! offset reverse-lookup are all derived from `PARAMETERS`. To add a
//! parameter, add one row here.

use crate::error::EncodeError;
use std::net::Ipv4Addr;

/// One NVRAM-resident parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    /// Canonical wire name, used in protocol messages and INI files.
    pub name: &'static str,
    /// NVRAM byte offset.
    pub offset: u16,
    /// NVRAM field width in bytes. Drives the encoding.
    pub length: u16,
    /// Value form shown after the flag in `--help` (e.g. "IP", "0|1").
    pub placeholder: &'static str,
    /// End-user help text.
    pub help: &'static str,
    /// Value the device reports after a hardware reset. Captured from a
    /// real Squeezebox Receiver; used by `read` to filter uninteresting
    /// values.
    pub factory_default: &'static str,
}

/// Ordered list of every known parameter. Order is intentional and stable.
pub const PARAMETERS: [Parameter; 26] = [
    Parameter {
        name: "lan_ip_mode",
        offset: 4,
        length: 1,
        placeholder: "0|1",
        help: "0=static, 1=DHCP",
        factory_default: "1",
    },
    Parameter {
        name: "lan_network_address",
        offset: 5,
        length: 4,
        placeholder: "IP",
        help: "Static IPv4 address (e.g. 192.168.1.50)",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "lan_subnet_mask",
        offset: 9,
        length: 4,
        placeholder: "MASK",
        help: "Subnet mask (e.g. 255.255.255.0)",
        factory_default: "255.255.255.0",
    },
    Parameter {
        name: "lan_gateway",
        offset: 13,
        length: 4,
        placeholder: "IP",
        help: "Default gateway IPv4 address",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "hostname",
        offset: 17,
        length: 33,
        placeholder: "NAME",
        help: "Device hostname (max 33 chars)",
        factory_default: "",
    },
    Parameter {
        name: "bridging",
        offset: 50,
        length: 1,
        placeholder: "0|1",
        help: "0=disabled, 1=enabled",
        factory_default: "0",
    },
    Parameter {
        name: "interface",
        offset: 52,
        length: 1,
        placeholder: "0|1",
        help: "0=wireless, 1=wired (Ethernet)",
        factory_default: "128",
    },
    Parameter {
        name: "primary_dns",
        offset: 59,
        length: 4,
        placeholder: "IP",
        help: "Primary DNS server IPv4 address",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "secondary_dns",
        offset: 67,
        length: 4,
        placeholder: "IP",
        help: "Secondary DNS server IPv4 address",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "server_address",
        offset: 71,
        length: 4,
        placeholder: "IP",
        help: "Logitech Media Server IPv4 address",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "lms_address",
        offset: 79,
        length: 4,
        placeholder: "IP",
        help: "Alternative LMS server IPv4 address",
        factory_default: "0.0.0.0",
    },
    Parameter {
        name: "squeezecenter_name",
        offset: 83,
        length: 33,
        placeholder: "NAME",
        help: "Squeezecenter / LMS server name (max 33 chars)",
        factory_default: "",
    },
    Parameter {
        name: "wireless_mode",
        offset: 173,
        length: 1,
        placeholder: "0|1",
        help: "0=infrastructure, 1=ad-hoc",
        factory_default: "0",
    },
    Parameter {
        name: "wireless_SSID",
        offset: 183,
        length: 33,
        placeholder: "SSID",
        help: "Wireless SSID (1-32 chars)",
        factory_default: "",
    },
    Parameter {
        name: "wireless_channel",
        offset: 216,
        length: 1,
        placeholder: "N",
        help: "Wireless channel (1-13)",
        factory_default: "6",
    },
    Parameter {
        name: "wireless_region_id",
        offset: 218,
        length: 1,
        placeholder: "ID",
        help: "Wireless region identifier (4=US, 6=CA, 7=AU, 13=FR, 14=EU, 16=JP, 21=TW, 23=CH)",
        factory_default: "4",
    },
    Parameter {
        name: "wireless_keylen",
        offset: 220,
        length: 1,
        placeholder: "5|13",
        help: "WEP key length",
        factory_default: "0",
    },
    Parameter {
        name: "wireless_wep_key",
        offset: 222,
        length: 13,
        placeholder: "HEX",
        help: "Primary WEP key",
        factory_default: "",
    },
    Parameter {
        name: "wireless_wep_key_1",
        offset: 235,
        length: 13,
        placeholder: "HEX",
        help: "WEP key slot 1",
        factory_default: "",
    },
    Parameter {
        name: "wireless_wep_key_2",
        offset: 248,
        length: 13,
        placeholder: "HEX",
        help: "WEP key slot 2",
        factory_default: "",
    },
    Parameter {
        name: "wireless_wep_key_3",
        offset: 261,
        length: 13,
        placeholder: "HEX",
        help: "WEP key slot 3",
        factory_default: "",
    },
    Parameter {
        name: "wireless_wep_on",
        offset: 274,
        length: 1,
        placeholder: "0|1",
        help: "0=disabled, 1=enabled",
        factory_default: "0",
    },
    Parameter {
        name: "wireless_wpa_cipher",
        offset: 275,
        length: 1,
        placeholder: "1|2|3",
        help: "1=TKIP, 2=AES (CCMP), 3=TKIP+AES",
        factory_default: "3",
    },
    Parameter {
        name: "wireless_wpa_mode",
        offset: 276,
        length: 1,
        placeholder: "1|2",
        help: "1=WPA, 2=WPA2",
        factory_default: "1",
    },
    Parameter {
        name: "wireless_wpa_on",
        offset: 277,
        length: 1,
        placeholder: "0|1",
        help: "0=disabled, 1=enabled",
        factory_default: "0",
    },
    Parameter {
        name: "wireless_wpa_psk",
        offset: 278,
        length: 64,
        placeholder: "PSK",
        help: "WPA pre-shared key (8-63 chars)",
        factory_default: "",
    },
];

/// Legacy and third-party names that refer to an existing parameter.
/// These get no `read` slot and no CLI flag; they resolve on lookup only.
const ALIASES: [(&str, &str); 2] = [
    ("slimserver_address", "server_address"),
    ("squeezecenter_address", "server_address"),
];

impl Parameter {
    /// The CLI flag form: lowercased, underscores to hyphens.
    #[must_use]
    pub fn flag_name(&self) -> String {
        self.name.to_lowercase().replace('_', "-")
    }

    /// Encodes `value` to exactly `self.length` bytes.
    ///
    /// Width drives the encoding: 1 is `u8`, 2 is big-endian `u16`, 4 is
    /// IPv4, anything else is zero-padded UTF-8 (truncated if too long).
    ///
    /// # Errors
    /// [`EncodeError`] if the value does not parse for this width.
    pub fn encode(&self, value: &str) -> Result<Vec<u8>, EncodeError> {
        match self.length {
            1 => {
                let n: u8 = value.parse().map_err(|_| EncodeError::NotU8 {
                    value: value.to_owned(),
                })?;
                Ok(vec![n])
            }
            2 => {
                let n: u16 = value.parse().map_err(|_| EncodeError::NotU16 {
                    value: value.to_owned(),
                })?;
                Ok(n.to_be_bytes().to_vec())
            }
            4 => {
                let ip: Ipv4Addr = value.parse().map_err(|_| EncodeError::NotIpv4 {
                    value: value.to_owned(),
                })?;
                Ok(ip.octets().to_vec())
            }
            width => {
                let mut out = vec![0u8; usize::from(width)];
                let src = value.as_bytes();
                let take = src.len().min(out.len());
                out[..take].copy_from_slice(&src[..take]);
                Ok(out)
            }
        }
    }
}

/// Looks up a parameter by canonical name, resolving aliases.
///
/// Linear scan: the table has 26 entries, so this beats hashing and
/// avoids the lazily-initialised global maps the Go version needs.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static Parameter> {
    if let Some(p) = PARAMETERS.iter().find(|p| p.name == name) {
        return Some(p);
    }
    let canonical = ALIASES.iter().find(|(alias, _)| *alias == name)?.1;
    PARAMETERS.iter().find(|p| p.name == canonical)
}

/// Looks up a parameter by its NVRAM offset.
#[must_use]
pub fn by_offset(offset: u16) -> Option<&'static Parameter> {
    PARAMETERS.iter().find(|p| p.offset == offset)
}

/// Every canonical parameter name, in table order.
pub fn names() -> impl Iterator<Item = &'static str> {
    PARAMETERS.iter().map(|p| p.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_26_entries() {
        assert_eq!(PARAMETERS.len(), 26);
    }

    #[test]
    fn every_entry_is_self_consistent() {
        for p in &PARAMETERS {
            assert!(!p.name.is_empty(), "{p:?} has an empty name");
            assert!(p.length > 0, "{} has zero length", p.name);
            assert!(
                p.length <= 256,
                "{} length {} exceeds max",
                p.name,
                p.length
            );
        }
    }

    #[test]
    fn offsets_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in &PARAMETERS {
            assert!(
                seen.insert(p.offset),
                "duplicate offset {} ({})",
                p.offset,
                p.name
            );
        }
    }

    #[test]
    fn lookup_by_name_works() {
        let p = by_name("wireless_SSID").expect("known parameter");
        assert_eq!(p.offset, 183);
        assert_eq!(p.length, 33);
    }

    #[test]
    fn unknown_names_return_none() {
        assert!(by_name("no_such_parameter").is_none());
    }

    #[test]
    fn aliases_resolve_to_the_canonical_entry() {
        let canonical = by_name("server_address").expect("known");
        assert_eq!(
            by_name("squeezecenter_address").map(|p| p.offset),
            Some(canonical.offset)
        );
        assert_eq!(
            by_name("slimserver_address").map(|p| p.offset),
            Some(canonical.offset)
        );
    }

    #[test]
    fn lookup_by_offset_works() {
        assert_eq!(by_offset(4).map(|p| p.name), Some("lan_ip_mode"));
        assert!(by_offset(9999).is_none());
    }

    #[test]
    fn flag_name_lowercases_and_hyphenates() {
        assert_eq!(
            by_name("wireless_SSID").unwrap().flag_name(),
            "wireless-ssid"
        );
        assert_eq!(by_name("lan_ip_mode").unwrap().flag_name(), "lan-ip-mode");
    }

    #[test]
    fn encodes_one_byte_values() {
        let p = by_name("lan_ip_mode").unwrap();
        assert_eq!(p.encode("1").unwrap(), vec![1]);
        assert!(p.encode("256").is_err());
        assert!(p.encode("nope").is_err());
    }

    #[test]
    fn encodes_ipv4_values_as_four_bytes() {
        let p = by_name("lan_network_address").unwrap();
        assert_eq!(p.encode("192.168.1.50").unwrap(), vec![192, 168, 1, 50]);
        assert!(p.encode("not-an-ip").is_err());
        assert!(p.encode("::1").is_err(), "IPv6 must be rejected");
    }

    #[test]
    fn encodes_strings_zero_padded_to_length() {
        let p = by_name("hostname").unwrap();
        let out = p.encode("bedroom").unwrap();
        assert_eq!(out.len(), 33);
        assert_eq!(&out[..7], b"bedroom");
        assert!(
            out[7..].iter().all(|&b| b == 0),
            "remainder must be zero-padded"
        );
    }

    #[test]
    fn encode_always_returns_exactly_length_bytes() {
        for p in &PARAMETERS {
            let sample = match p.length {
                1 | 2 => "1",
                4 => "192.168.1.1",
                _ => "x",
            };
            let out = p.encode(sample).unwrap();
            assert_eq!(
                out.len(),
                usize::from(p.length),
                "{} encoded to the wrong width",
                p.name
            );
        }
    }
}
