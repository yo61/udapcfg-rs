//! The `udapcfg` command-line tool.

pub mod cli;
pub mod cmd;

pub use cli::{Cli, Command};

use std::io::Write;

/// An error carrying the process exit code to use.
///
/// 0 success, 1 usage error, 2 operation failure.
#[derive(Debug)]
pub struct CliError {
    pub code: i32,
    pub source: anyhow::Error,
}

/// Builds a `udap::Client`. Injected so tests can substitute a
/// mock-backed client without a mutable global.
pub type ClientFactory = Box<dyn Fn() -> Result<udap::Client, anyhow::Error>>;

/// Dispatches the parsed command.
///
/// # Errors
/// [`CliError`] carrying the exit code the process should use.
pub async fn run(
    cli: Cli,
    make_client: ClientFactory,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    // Validate before dispatch, not in the factory: the factory's error is
    // mapped to exit 2 by every subcommand, and go-udap treats an unusable
    // interface as a usage error (cli/cli.go:124).
    if let Some(name) = cli.bind_interface.as_deref() {
        let ifs = udap::interfaces::enumerate().map_err(|e| CliError {
            code: 2,
            source: anyhow::Error::new(e).context("enumerate interfaces"),
        })?;
        if !ifs.iter().any(|i| i.name == name) {
            return Err(CliError {
                code: 1,
                source: anyhow::anyhow!(
                    "--bind-interface: {name:?} is not usable \
                     (must be up, broadcast-capable, with an IPv4 address)"
                ),
            });
        }
    }

    match cli.command {
        Command::Discover => cmd::discover::run(make_client, cli.timeout, stdout, stderr).await,
    }
}
