//! Local network interfaces usable for UDAP broadcast discovery.
//!
//! An anti-corruption layer over `netdev`: the rest of the crate sees
//! `NetInterface` and never the enumeration crate's types.

use std::cmp::Ordering;
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
///
/// Delegates to `ipnet`, whose `Ipv4Net::broadcast` is `addr | hostmask` —
/// the same arithmetic this used to hand-roll, with the `/0` case (where
/// shifting a `u32` by 32 is undefined) handled inside the crate. `ipnet`
/// was already in the tree transitively via `netdev`.
///
/// Kept as a named function so the tests below pin the behaviour against
/// go-udap's `computeDirectedBroadcast` rather than against `ipnet`.
fn directed_broadcast(addr: Ipv4Addr, prefix_len: u8) -> Ipv4Addr {
    // prefix_len comes from netdev, which cannot report > 32 for IPv4;
    // Ipv4Net::new rejects anything larger, and we fall back to the
    // address itself (a /32 has no host bits) rather than panicking.
    ipnet::Ipv4Net::new(addr, prefix_len).map_or(addr, |net| net.broadcast())
}

/// Shortest token treated as an opaque identifier rather than a counter.
///
/// Docker names its bridges `br-` plus a 12-character hex id. No
/// conventional interface name is eight or more characters of pure hex, so
/// this floor keeps short all-hex names such as `ae2` on the counter path.
const HEX_IDENTIFIER_MIN_LEN: usize = 8;

/// A digit run with its leading zeros removed, keeping at least one digit.
fn strip_leading_zeros(run: &[u8]) -> &[u8] {
    let mut start = 0;
    while start + 1 < run.len() && run[start] == b'0' {
        start += 1;
    }
    &run[start..]
}

/// True when a token is an opaque hex identifier rather than a counter:
/// long enough, every byte a hex digit, and at least one of them a letter.
///
/// The letter requirement keeps all-digit tokens numeric, since digits are
/// valid hex too.
fn is_hex_identifier(token: &[u8]) -> bool {
    if token.len() < HEX_IDENTIFIER_MIN_LEN {
        return false;
    }
    let mut has_letter = false;
    for &byte in token {
        if !byte.is_ascii_hexdigit() {
            return false;
        }
        if byte.is_ascii_alphabetic() {
            has_letter = true;
        }
    }
    has_letter
}

/// Compares one alphanumeric token, reading digit runs as numbers.
///
/// Runs are compared without parsing: once leading zeros are gone the
/// longer run is the larger number, and equal-length runs compare
/// bytewise. That is what an integer comparison would give but cannot
/// overflow, which matters because a docker bridge id is far wider than
/// `u64`.
fn token_cmp(left: &[u8], right: &[u8]) -> Ordering {
    let mut left_at = 0;
    let mut right_at = 0;

    while left_at < left.len() && right_at < right.len() {
        if left[left_at].is_ascii_digit() && right[right_at].is_ascii_digit() {
            let left_start = left_at;
            while left_at < left.len() && left[left_at].is_ascii_digit() {
                left_at += 1;
            }
            let right_start = right_at;
            while right_at < right.len() && right[right_at].is_ascii_digit() {
                right_at += 1;
            }

            let left_run = &left[left_start..left_at];
            let right_run = &right[right_start..right_at];
            let by_value = strip_leading_zeros(left_run)
                .len()
                .cmp(&strip_leading_zeros(right_run).len())
                .then_with(|| strip_leading_zeros(left_run).cmp(strip_leading_zeros(right_run)));
            if by_value != Ordering::Equal {
                return by_value;
            }
        } else {
            let by_byte = left[left_at].cmp(&right[right_at]);
            if by_byte != Ordering::Equal {
                return by_byte;
            }
            left_at += 1;
            right_at += 1;
        }
    }

    (left.len() - left_at).cmp(&(right.len() - right_at))
}

/// Orders interface names the way someone reading a list expects.
///
/// The name is walked as alternating alphanumeric tokens and separators.
/// Each pair of tokens is compared one of two ways:
///
/// - **Counters** read their digit runs as numbers, wherever those runs
///   appear in the token. That covers `en2` before `en10` and, because the
///   run need not be at the end, systemd's predictable names: `enp2s0`
///   before `enp10s0`. Splitting only on a *trailing* digit run would send
///   every `enpXsY` name down the opaque path and reintroduce the bug.
/// - **Hex identifiers** (`br-a27c6b90d180`) compare as written, so they
///   come out in hex order. Reading their digit runs as numbers scrambles
///   them: `7` is less than `41` numerically, but `7a91…` is greater than
///   `41b8…` as hex.
///
/// Separators are compared bytewise, which sorts VLAN sub-interfaces by id
/// — `eth0.99` before `eth0.100` — since the segments either side are
/// compared in turn.
///
/// The final `then_with` guarantees a total order. Two distinct names must
/// never compare equal: `sort_by` is stable, so a tie would preserve
/// `netdev`'s arbitrary input order, which is the non-determinism this
/// exists to remove.
fn iface_cmp(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let mut left_at = 0;
    let mut right_at = 0;

    while left_at < left_bytes.len() && right_at < right_bytes.len() {
        if left_bytes[left_at].is_ascii_alphanumeric()
            && right_bytes[right_at].is_ascii_alphanumeric()
        {
            let left_start = left_at;
            while left_at < left_bytes.len() && left_bytes[left_at].is_ascii_alphanumeric() {
                left_at += 1;
            }
            let right_start = right_at;
            while right_at < right_bytes.len() && right_bytes[right_at].is_ascii_alphanumeric() {
                right_at += 1;
            }

            let left_token = &left_bytes[left_start..left_at];
            let right_token = &right_bytes[right_start..right_at];

            let ordering = if is_hex_identifier(left_token) && is_hex_identifier(right_token) {
                left_token.cmp(right_token)
            } else {
                token_cmp(left_token, right_token)
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        } else {
            let by_byte = left_bytes[left_at].cmp(&right_bytes[right_at]);
            if by_byte != Ordering::Equal {
                return by_byte;
            }
            left_at += 1;
            right_at += 1;
        }
    }

    (left_bytes.len() - left_at)
        .cmp(&(right_bytes.len() - right_at))
        .then_with(|| left.cmp(right))
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
/// Infallible: `netdev::get_interfaces()` returns a bare `Vec`, not a
/// `Result` — the underlying platform enumeration has no reported failure
/// mode to propagate. An interface list that comes back empty or partial
/// (e.g. a permissions problem) is indistinguishable from "no usable
/// interfaces" and is handled the same way by every caller.
///
/// **Ordered by name, naturally** — `en2` before `en10`. This is a
/// deliberate divergence from go-udap, which prints whatever order the OS
/// hands back, and it is recorded in the spec's accepted-deltas table.
/// `netdev` guarantees no order at all: its Linux backend collects through
/// a `HashMap` and Rust re-seeds `RandomState` per process, so before this
/// sort `udapcfg interfaces` printed a different order on every run
/// ([issue #13](https://github.com/yo61/udapcfg-rs/issues/13)). Sorting
/// here rather than at the point of display keeps one order across the
/// table, `--bind-interface` validation, and `--all-interfaces` fan-out.
#[must_use]
pub fn enumerate() -> Vec<NetInterface> {
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
    out.sort_by(|a, b| iface_cmp(&a.name, &b.name));
    out
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
        let ifs = enumerate();
        for ni in &ifs {
            assert!(!ni.name.is_empty(), "interface with empty name");
            assert!(ni.index > 0, "{} has index 0", ni.name);
            assert!(!ni.addr.is_loopback(), "{} is a loopback", ni.name);
            assert!(!ni.addr.is_unspecified(), "{} has 0.0.0.0", ni.name);
        }
    }

    #[test]
    fn enumerate_yields_one_entry_per_interface() {
        let ifs = enumerate();
        let mut names: Vec<&str> = ifs.iter().map(|n| n.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "an interface appeared twice");
    }

    #[test]
    fn iface_cmp_orders_digit_runs_numerically() {
        // The whole point: plain string ordering puts en10 before en2.
        assert_eq!(iface_cmp("en2", "en10"), std::cmp::Ordering::Less);
        assert_eq!(iface_cmp("en10", "en2"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn iface_cmp_orders_single_digits() {
        assert_eq!(iface_cmp("en0", "en8"), std::cmp::Ordering::Less);
        assert_eq!(iface_cmp("en8", "en0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn iface_cmp_is_equal_for_identical_names() {
        assert_eq!(iface_cmp("bond0", "bond0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn iface_cmp_falls_back_to_text_without_digits() {
        assert_eq!(iface_cmp("bond0", "vlan20"), std::cmp::Ordering::Less);
        assert_eq!(iface_cmp("docker0", "bond0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn iface_cmp_does_not_wrap_on_digit_runs_too_large_for_u64() {
        // Too long to be a unit number, so the name stays opaque. The point
        // is that it neither wraps onto a shared key nor ties.
        let long = "br-99999999999999999999999";
        let longer = "br-100000000000000000000000";
        let forward = iface_cmp(long, longer);
        assert_ne!(forward, Ordering::Equal);
        assert_eq!(forward.reverse(), iface_cmp(longer, long));
    }

    #[test]
    fn iface_cmp_is_total_for_names_carrying_the_same_unit() {
        // en7 and en007 carry the same unit but are different interfaces.
        // Which leads is arbitrary; returning Equal is not acceptable,
        // because sort_by is stable and would then keep netdev's arbitrary
        // input order for the pair.
        let forward = iface_cmp("en7", "en007");
        assert_ne!(forward, Ordering::Equal, "distinct names must not tie");
        assert_eq!(forward.reverse(), iface_cmp("en007", "en7"));
    }

    #[test]
    fn vlan_subinterfaces_sort_by_vlan_id() {
        // eth0.100 is a VLAN sub-interface, so 99 precedes 100 rather than
        // "100" preceding "99" as text would give.
        assert_eq!(iface_cmp("eth0.99", "eth0.100"), Ordering::Less);
        assert_eq!(iface_cmp("bond0.20", "bond0.100"), Ordering::Less);
        assert_eq!(iface_cmp("eth0", "eth0.2"), Ordering::Less);
    }

    #[test]
    fn iface_cmp_orders_a_prefix_before_its_extension() {
        assert_eq!(iface_cmp("en", "en0"), std::cmp::Ordering::Less);
        assert_eq!(iface_cmp("br-", "br-0"), std::cmp::Ordering::Less);
    }

    #[test]
    fn enumerate_returns_names_in_sorted_order() {
        let ifs = enumerate();
        let names: Vec<&str> = ifs.iter().map(|n| n.name.as_str()).collect();
        for pair in names.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert_eq!(
                iface_cmp(a, b),
                std::cmp::Ordering::Less,
                "{a} must sort before {b}"
            );
        }
    }

    #[test]
    fn hex_identifiers_compare_in_hex_order() {
        // Docker bridge suffixes are opaque 12-digit hex ids, not counters.
        // Reading embedded digit runs as numbers scrambles them: "7" < "41"
        // numerically, but 0x7a91... > 0x41b8... as hex.
        assert_eq!(
            iface_cmp("br-41b85cbb03f0", "br-7a918edb41cb"),
            Ordering::Less
        );
        assert_eq!(
            iface_cmp("br-7a918edb41cb", "br-41b85cbb03f0"),
            Ordering::Greater
        );
    }

    #[test]
    fn hex_identifiers_sort_digits_before_letters() {
        assert_eq!(
            iface_cmp("br-5bad6fd80036", "br-a27c6b90d180"),
            Ordering::Less
        );
        assert_eq!(
            iface_cmp("br-a27c6b90d180", "br-c110052b0ec9"),
            Ordering::Less
        );
    }

    #[test]
    fn a_token_with_a_non_hex_letter_stays_numeric() {
        // "en10" is a counter, not a hex blob: 'n' is not a hex digit.
        assert_eq!(iface_cmp("en2", "en10"), Ordering::Less);
        assert_eq!(iface_cmp("vlan2", "vlan10"), Ordering::Less);
        assert_eq!(iface_cmp("docker2", "docker10"), Ordering::Less);
    }

    #[test]
    fn an_all_digit_token_stays_numeric() {
        // Digits are all valid hex, but with no letter this is a counter.
        assert_eq!(iface_cmp("vlan20", "vlan100"), Ordering::Less);
        assert_eq!(iface_cmp("eth9", "eth10"), Ordering::Less);
    }

    #[test]
    fn a_short_all_hex_token_stays_a_counter() {
        // Every character of "ae2"/"ae10" is a hex digit, but three
        // characters is a counter, not an identifier.
        assert_eq!(iface_cmp("ae2", "ae10"), Ordering::Less);
        assert_eq!(iface_cmp("e2", "e10"), Ordering::Less);
    }

    #[test]
    fn systemd_predictable_names_sort_numerically() {
        // enpXsY is the Linux default for physical NICs. nas1's own bond
        // slaves are enp7s0 and enp8s0.
        assert_eq!(iface_cmp("enp2s0", "enp10s0"), Ordering::Less);
        assert_eq!(iface_cmp("enp3s0f0", "enp3s0f1"), Ordering::Less);
        assert_eq!(iface_cmp("enp7s0", "enp8s0"), Ordering::Less);
        assert_eq!(iface_cmp("ens5", "ens21"), Ordering::Less);
    }
}
