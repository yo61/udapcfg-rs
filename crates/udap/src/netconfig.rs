//! The result of a `get_ip` (0x0002) query.
//!
//! Distinct from [`crate::Device`]: `Device` is what discovery passively
//! observed; `NetworkConfig` is what the device reports when asked.

use std::fmt;
use std::net::Ipv4Addr;

/// A device's active network configuration.
///
/// Every field is optional — devices omit TLVs, notably `gateway` on a
/// static address with no gateway configured.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkConfig {
    pub ip: Option<Ipv4Addr>,
    pub subnet_mask: Option<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
}

/// Renders an address, or `-` when it is absent *or* unspecified.
///
/// go-udap's `ipOrDash` collapses both cases, so a device that omits the
/// TLV and one that reports `0.0.0.0` print identically. `Option` alone
/// would not reproduce that.
fn or_dash(addr: Option<Ipv4Addr>) -> String {
    match addr {
        Some(a) if !a.is_unspecified() => a.to_string(),
        _ => "-".to_owned(),
    }
}

impl fmt::Display for NetworkConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IP:      {}\nSubnet:  {}\nGateway: {}",
            or_dash(self.ip),
            or_dash(self.subnet_mask),
            or_dash(self.gateway)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_fields_render_as_a_dash() {
        let nc = NetworkConfig::default();
        assert_eq!(nc.to_string(), "IP:      -\nSubnet:  -\nGateway: -");
    }

    #[test]
    fn an_unspecified_address_also_renders_as_a_dash() {
        // go-udap's ipOrDash treats 0.0.0.0 as absent, not as an address.
        // A device that omits the gateway TLV and one that reports
        // 0.0.0.0 must print identically.
        let nc = NetworkConfig {
            ip: Some(Ipv4Addr::new(192, 168, 1, 50)),
            subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
            gateway: Some(Ipv4Addr::UNSPECIFIED),
        };
        assert_eq!(
            nc.to_string(),
            "IP:      192.168.1.50\nSubnet:  255.255.255.0\nGateway: -"
        );
    }

    #[test]
    fn present_fields_render_as_dotted_quads() {
        let nc = NetworkConfig {
            ip: Some(Ipv4Addr::new(10, 0, 0, 5)),
            subnet_mask: Some(Ipv4Addr::new(255, 0, 0, 0)),
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
        };
        assert_eq!(
            nc.to_string(),
            "IP:      10.0.0.5\nSubnet:  255.0.0.0\nGateway: 10.0.0.1"
        );
    }
}
