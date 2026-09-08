//! Command-line interface definitions.

use clap::{Parser, Subcommand};
use go_duration::GoDuration;
use std::str::FromStr;

#[derive(Debug, Parser)]
#[command(
    name = "udapcfg",
    version,
    about = "Squeezebox UDAP configuration tool",
    long_about = "udapcfg discovers and configures Squeezebox devices over UDAP\n\
                  (Universal Device Access Protocol) on UDP port 17784.\n\n\
                  It is single-shot: every invocation runs one subcommand to\n\
                  completion and exits.\n\n\
                  UDAP only talks to devices in setup mode (the front light\n\
                  flashes red). Brand-new devices arrive in setup mode; existing\n\
                  devices can be put back into it by holding the front button\n\
                  for 3-6 seconds."
)]
pub struct Cli {
    /// Operation timeout, e.g. 2s, 30s, 2m
    #[arg(long, global = true, value_name = "DURATION",
          default_value = "2s", value_parser = parse_timeout)]
    pub timeout: GoDuration,

    /// Debug logging to stderr
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// Re-transmit each UDAP send N additional times
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    pub retries: usize,

    /// Bind discovery to one network interface
    #[arg(
        long,
        global = true,
        value_name = "NAME",
        conflicts_with = "all_interfaces"
    )]
    pub bind_interface: Option<String>,

    /// Broadcast on every usable interface (fan-out)
    #[arg(long, global = true)]
    pub all_interfaces: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Discover devices on the network
    #[command(
        long_about = "Broadcast a UDAP advanced-discover packet on UDP port 17784\n\
                            and print every Squeezebox device that responds within\n\
                            --timeout. MAC addresses are printed one per line.\n\n\
                            Sends always target the limited broadcast address\n\
                            255.255.255.255 so unconfigured devices (which have no\n\
                            DHCP lease and so no notion of a subnet broadcast\n\
                            address) can hear them."
    )]
    Discover,

    /// List usable network interfaces
    #[command(
        long_about = "List local network interfaces that can be used for UDAP discovery.\n\n\
                            An interface is usable if it is up, has broadcast capability,\n\
                            and has an IPv4 address. This list helps when choosing a\n\
                            specific interface for `--bind-interface`."
    )]
    Interfaces,
}

/// Parses `--timeout` with Go's `time.ParseDuration` grammar.
///
/// go-udap's flag is a `time.Duration` whose `Set` calls `time.ParseDuration`
/// and validates nothing further (`cli/params.go:76`), so every form that
/// accepts there must accept here: hours, fractions, compound values and
/// negatives. Negatives are deliberately not rejected — Go treats an
/// already-expired deadline as an immediate timeout, not a usage error.
///
/// # Errors
/// Returns a message suitable for clap when the input is not a duration.
fn parse_timeout(s: &str) -> Result<GoDuration, String> {
    GoDuration::from_str(s).map_err(|e| format!("invalid duration {s:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `None` means the input was rejected, which shows up in a failing
    /// `assert_eq!` as clearly as a panic would and keeps `clippy::panic`
    /// satisfied outside a `#[test]` frame.
    fn nanos(s: &str) -> Option<i64> {
        parse_timeout(s).ok().map(|d| d.nanoseconds())
    }

    #[test]
    fn parses_the_simple_forms() {
        assert_eq!(nanos("2s"), Some(2_000_000_000));
        assert_eq!(nanos("500ms"), Some(500_000_000));
        assert_eq!(nanos("2m"), Some(120_000_000_000));
    }

    // Every one of these is valid input to time.ParseDuration, so go-udap
    // accepts it. An earlier hand-rolled parser took only an integer plus
    // ms/s/m and rejected all of them with exit code 1.
    #[test]
    fn parses_the_forms_go_udap_accepts_beyond_the_simple_ones() {
        assert_eq!(nanos("1h"), Some(3_600_000_000_000));
        assert_eq!(nanos("1.5s"), Some(1_500_000_000));
        assert_eq!(nanos("1m30s"), Some(90_000_000_000));
        assert_eq!(nanos("2h45m"), Some(9_900_000_000_000));
        assert_eq!(nanos("300us"), Some(300_000));
        assert_eq!(nanos("100ns"), Some(100));
    }

    // time.ParseDuration accepts a negative and go-udap does not reject it,
    // so neither do we; the deadline is simply already expired.
    #[test]
    fn accepts_negative_durations_because_go_does() {
        assert_eq!(nanos("-5s"), Some(-5_000_000_000));
    }

    // The overflow the hand-rolled `n * 60` had: huge minute counts wrapped
    // silently in release, turning a long timeout into a tiny one.
    #[test]
    fn rejects_values_too_large_to_represent() {
        assert!(parse_timeout("400000000000000000m").is_err());
    }

    #[test]
    fn rejects_nonsense_durations() {
        assert!(parse_timeout("banana").is_err());
        assert!(parse_timeout("2").is_err());
        assert!(parse_timeout("").is_err());
        assert!(parse_timeout("2x3s").is_err());
    }
}
