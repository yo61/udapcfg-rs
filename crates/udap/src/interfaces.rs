//! Local network interfaces usable for UDAP broadcast discovery.
//!
//! An anti-corruption layer over `netdev`: the rest of the crate sees
//! `NetInterface` and never the enumeration crate's types.

use std::net::Ipv4Addr;

/// One interface that discovery can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetInterface {
    pub name: String,
    /// OS interface index. Required by `bind_device_by_index_v4`.
    pub index: u32,
    /// First IPv4 address on the interface.
    pub addr: Ipv4Addr,
    /// The subnet's directed-broadcast address.
    ///
    /// Informational only — shown by `udapcfg interfaces`. UDAP sends
    /// always target the limited broadcast 255.255.255.255, because
    /// unconfigured devices have no subnet and would not receive a
    /// directed broadcast.
    pub broadcast: Ipv4Addr,
}

#[derive(Debug, thiserror::Error)]
pub enum InterfaceError {
    #[error("enumerate interfaces: {0}")]
    Enumerate(String),
}

/// Why an interface fails the filter [`enumerate`] applies, phrased for
/// direct interpolation into a "not usable" message.
///
/// The single source of truth for that phrase. Two call sites build a
/// full message around it with different lead-ins, matching go-udap's
/// own two independent templates: `ClientError::NoSuchInterface`'s
/// `Display` (library-facing, "interface %q ..." per
/// `udap/client.go:425`) and `udap-cli`'s `--bind-interface` pre-dispatch
/// check (CLI-facing, "--bind-interface: %q ..." per `cli/cli.go:124`).
/// Neither can drift from the other on the substance of *why*, because
/// both format this same constant.
pub const NOT_USABLE_REASON: &str =
    "is not usable (must be up, broadcast-capable, with an IPv4 address)";

/// Returns the subnet's directed-broadcast address: `addr | !mask`.
fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Ipv4Addr {
    // A /0 mask is 0, and shifting a u32 by 32 is undefined in Rust, so
    // compute the mask via checked_shl and treat the overflow as "no bits".
    let mask: u32 = if prefix_len == 0 {
        0
    } else {
        u32::MAX
            .checked_shl(u32::from(32 - prefix_len))
            .unwrap_or(0)
    };
    Ipv4Addr::from(u32::from(addr) | !mask)
}

/// Every interface usable for UDAP broadcast discovery.
///
/// The filter matches go-udap exactly: up, broadcast-capable, not a
/// loopback, and carrying at least one IPv4 address. Only the first IPv4
/// address per interface is taken.
///
/// The broadcast-capable test is load-bearing: `WireGuard` and Tailscale
/// interfaces do not set `IFF_BROADCAST`, so this is what keeps discovery
/// off VPN tunnels.
///
/// # Errors
/// [`InterfaceError::Enumerate`] if the platform enumeration fails.
pub fn enumerate() -> Result<Vec<NetInterface>, InterfaceError> {
    let mut out = Vec::new();
    for iface in netdev::get_interfaces() {
        if !iface.is_up() || !iface.is_broadcast() || iface.is_loopback() {
            continue;
        }
        let Some(net) = iface.ipv4.first() else {
            continue;
        };
        out.push(NetInterface {
            name: iface.name.clone(),
            index: iface.index,
            addr: net.addr(),
            broadcast: directed_broadcast(net.addr(), net.prefix_len()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn directed_broadcast_is_addr_or_inverted_mask() {
        // 192.168.1.50/24 -> 192.168.1.255
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 1, 50), 24),
            Ipv4Addr::new(192, 168, 1, 255)
        );
        // 10.0.0.5/8 -> 10.255.255.255
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(10, 0, 0, 5), 8),
            Ipv4Addr::new(10, 255, 255, 255)
        );
        // /32 has no host bits, so the broadcast is the address itself
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(172, 16, 0, 1), 32),
            Ipv4Addr::new(172, 16, 0, 1)
        );
        // /0 is the whole space
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(172, 16, 0, 1), 0),
            Ipv4Addr::BROADCAST
        );
    }

    // enumerate() reads the real machine, so assert invariants rather than
    // a fixed list: any host running this has at least a loopback to exclude.
    #[test]
    fn enumerate_applies_the_filter() {
        let ifs = enumerate().expect("enumeration must not error");
        for ni in &ifs {
            assert!(!ni.name.is_empty(), "interface with empty name");
            assert!(ni.index > 0, "{} has index 0", ni.name);
            assert!(!ni.addr.is_loopback(), "{} is a loopback", ni.name);
            assert!(!ni.addr.is_unspecified(), "{} has 0.0.0.0", ni.name);
        }
    }

    #[test]
    fn enumerate_yields_one_entry_per_interface() {
        let ifs = enumerate().expect("enumeration must not error");
        let mut names: Vec<&str> = ifs.iter().map(|n| n.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "an interface appeared twice");
    }
}
