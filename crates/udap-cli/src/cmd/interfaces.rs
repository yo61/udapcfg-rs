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
pub fn run(stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), CliError> {
    let ifs = udap::interfaces::enumerate().map_err(|e| CliError {
        code: 2,
        source: anyhow::Error::new(e),
    })?;
    if ifs.is_empty() {
        let _ = writeln!(stderr, "no usable interfaces found");
        return Ok(());
    }
    output::format_interfaces_table(stdout, &ifs);
    Ok(())
}
