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

/// Builds the client the CLI's flags describe.
///
/// Extracted from `main`'s factory closure so the flag-to-client wiring
/// itself is testable: `main.rs` is a thin, untested entry point, and
/// before this extraction nothing exercised the code path that calls
/// `set_retries` -- a test could delete that call and every test would
/// still pass.
///
/// `port` is injectable rather than hardcoded to `udap::PORT` so tests
/// can bind an ephemeral port (`0`) instead of colliding with anything
/// already bound to the real UDAP port in a shared test environment;
/// production (`main.rs`) always passes `udap::PORT`.
///
/// # Errors
/// Whatever `udap::Client::for_interface` / `udap::Client::with_udp` /
/// `udap::Client::for_all_interfaces` return.
pub fn build_client(
    bind_interface: Option<&str>,
    all_interfaces: bool,
    retries: usize,
    port: u16,
) -> Result<udap::Client, anyhow::Error> {
    let mut client = if let Some(name) = bind_interface {
        udap::Client::for_interface(name, port)?
    } else if all_interfaces {
        udap::Client::for_all_interfaces(port)?
    } else {
        udap::Client::with_udp(port)?
    };
    client.set_retries(retries);
    Ok(client)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves `--retries` reaches the client the factory builds, not just
    /// the send-retry mechanism inside `udap::Client::discover`. Port 0
    /// lets the OS pick an ephemeral port so this doesn't collide with
    /// anything already bound to `udap::PORT` in a shared test
    /// environment. Breaking the wiring inside `build_client` (deleting
    /// its `set_retries` call, or applying the wrong value) makes this
    /// fail, which is the point: nothing else in the suite would notice.
    ///
    /// `#[tokio::test]` rather than `#[test]`: `build_client`'s default
    /// path constructs a real `tokio::net::UdpSocket`, which panics
    /// ("there is no reactor running") without a Tokio runtime context,
    /// even though `build_client` itself is synchronous and this test
    /// never awaits anything.
    #[tokio::test]
    async fn build_client_applies_retries() {
        let client = build_client(None, false, 3, 0).expect("default path must bind");
        assert_eq!(client.retries(), 3);
    }
}
