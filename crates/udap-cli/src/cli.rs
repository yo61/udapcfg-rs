//! Command-line interface definitions.

use clap::{Parser, Subcommand};
use std::time::Duration;

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
          default_value = "2s", value_parser = humantime_parse)]
    pub timeout: Duration,

    /// Debug logging to stderr
    #[arg(long, short, global = true)]
    pub verbose: bool,

    /// Re-transmit each UDAP send N additional times
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    pub retries: usize,

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
}

/// Parses a Go-style duration string ("2s", "500ms", "1m").
///
/// # Errors
/// Returns a message suitable for clap when the input is not a duration.
fn humantime_parse(s: &str) -> Result<Duration, String> {
    parse_duration(s).ok_or_else(|| format!("invalid duration {s:?} (try 2s, 500ms, 1m)"))
}

fn parse_duration(s: &str) -> Option<Duration> {
    let (value, unit) = s.split_at(s.find(|c: char| c.is_alphabetic())?);
    let n: u64 = value.parse().ok()?;
    match unit {
        "ms" => Some(Duration::from_millis(n)),
        "s" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_secs(n * 60)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_duration_forms_go_udap_accepts() {
        assert_eq!(parse_duration("2s"), Some(Duration::from_secs(2)));
        assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(parse_duration("2m"), Some(Duration::from_secs(120)));
    }

    #[test]
    fn rejects_nonsense_durations() {
        assert_eq!(parse_duration("banana"), None);
        assert_eq!(parse_duration("2"), None);
        assert_eq!(parse_duration("2h"), None);
    }
}
