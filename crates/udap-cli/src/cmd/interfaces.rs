//! The `interfaces` subcommand.

use crate::{CliError, output};
use std::io::Write;

/// Lists interfaces usable for discovery.
///
/// Finding none is not an error — a note goes to stderr and the exit
/// code stays 0, matching `discover`.
///
/// # Errors
/// [`CliError`] with code 2 if enumeration fails.
pub fn run(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    enumerate: impl Fn() -> Result<Vec<udap::NetInterface>, udap::InterfaceError>,
) -> Result<(), CliError> {
    let ifs = enumerate().map_err(|e| CliError {
        code: 2,
        source: anyhow::Error::new(e).context("enumerate interfaces"),
    })?;
    if ifs.is_empty() {
        let _ = writeln!(stderr, "no usable interfaces found");
        return Ok(());
    }
    output::format_interfaces_table(stdout, &ifs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn populated_path_writes_table_to_stdout() {
        let sample = vec![udap::NetInterface {
            name: "en0".to_owned(),
            index: 4,
            addr: Ipv4Addr::new(192, 168, 1, 50),
            broadcast: Ipv4Addr::new(192, 168, 1, 255),
        }];
        let enumerate = || Ok(sample.clone());

        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run(&mut out, &mut err, enumerate);

        assert!(result.is_ok(), "populated path must succeed");
        let stdout = String::from_utf8(out).expect("utf8");
        let stderr = String::from_utf8(err).expect("utf8");
        assert!(stdout.contains("NAME"), "header must be in stdout");
        assert!(stdout.contains("en0"), "interface data must be in stdout");
        assert!(stderr.is_empty(), "stderr must be empty on success");
    }

    #[test]
    fn empty_path_writes_message_to_stderr() {
        let enumerate = || Ok(vec![]);

        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run(&mut out, &mut err, enumerate);

        assert!(result.is_ok(), "empty result is not an error");
        let stdout = String::from_utf8(out).expect("utf8");
        let stderr = String::from_utf8(err).expect("utf8");
        assert!(
            stdout.is_empty(),
            "stdout must stay empty when no interfaces found"
        );
        assert_eq!(
            stderr, "no usable interfaces found\n",
            "message must match go-udap"
        );
    }
}
