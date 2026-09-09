//! Formatted output for the CLI.

use std::io::Write;
use udap::NetInterface;

/// Writes the `interfaces` table. Matches go-udap's column widths exactly.
pub fn format_interfaces_table(w: &mut dyn Write, ifs: &[NetInterface]) {
    if ifs.is_empty() {
        return;
    }
    let _ = writeln!(w, "NAME            INDEX  ADDRESS            BROADCAST");
    for ni in ifs {
        let _ = writeln!(
            w,
            "{:<15} {:<5}  {:<18} {}",
            ni.name, ni.index, ni.addr, ni.broadcast
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn sample() -> Vec<NetInterface> {
        vec![NetInterface {
            name: "en0".to_owned(),
            index: 4,
            addr: Ipv4Addr::new(192, 168, 1, 50),
            broadcast: Ipv4Addr::new(192, 168, 1, 255),
        }]
    }

    #[test]
    fn header_matches_go_udap_byte_for_byte() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &sample());
        let s = String::from_utf8(out).expect("utf8");
        let header = s.lines().next().expect("a header line");
        assert_eq!(
            header,
            "NAME            INDEX  ADDRESS            BROADCAST"
        );
    }

    #[test]
    fn row_column_widths_match_go_udap() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &sample());
        let s = String::from_utf8(out).expect("utf8");
        let row = s.lines().nth(1).expect("a data row");
        // %-15s + space + %-5d + two spaces + %-18s + %s
        assert_eq!(
            row,
            "en0             4      192.168.1.50       192.168.1.255"
        );
    }

    #[test]
    fn empty_input_writes_nothing() {
        let mut out = Vec::new();
        format_interfaces_table(&mut out, &[]);
        assert!(out.is_empty(), "no header for an empty list");
    }
}
