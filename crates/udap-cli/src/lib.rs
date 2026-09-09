//! The `udapcfg` command-line tool.

pub mod cli;
pub mod cmd;
pub mod output;

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
///
/// Takes the `--bind-interface` target pre-resolved (see
/// [`build_client`]'s doc comment for why) rather than a bare name, so
/// `enumerate()` runs exactly once per invocation regardless of how many
/// times the factory itself gets called.
pub type ClientFactory =
    Box<dyn Fn(Option<&udap::NetInterface>) -> Result<udap::Client, anyhow::Error>>;

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
/// `resolved_interface` is the already-enumerated match for
/// `--bind-interface NAME`, computed once by [`run`]'s pre-dispatch
/// check. This function does not call `udap::interfaces::enumerate`
/// itself: `Client::for_interface` would (a second sweep on top of
/// `run`'s), and the two sweeps are a TOCTOU window -- an interface that
/// vanishes between them would surface as an operation failure (exit 2)
/// via the factory instead of the usage error (exit 1) `run` already
/// decided on the first sweep.
///
/// # Errors
/// Whatever `udap::Client::for_resolved_interface` /
/// `udap::Client::with_udp` / `udap::Client::for_all_interfaces` return.
pub fn build_client(
    resolved_interface: Option<&udap::NetInterface>,
    all_interfaces: bool,
    retries: usize,
    port: u16,
) -> Result<udap::Client, anyhow::Error> {
    let mut client = if let Some(iface) = resolved_interface {
        udap::Client::for_resolved_interface(iface, port)?
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
    // interface as a usage error (cli/cli.go:124). This is also the only
    // place `--bind-interface` enumerates: the resolved `NetInterface`
    // (not just its name) travels into the factory via `build_client`,
    // so `Client::for_interface`'s own enumerate never runs for the CLI
    // path -- one sweep per invocation, not two, and no TOCTOU window
    // between them.
    let resolved_interface = match cli.bind_interface.as_deref() {
        Some(name) => {
            let ifs = udap::interfaces::enumerate();
            let Some(iface) = ifs.into_iter().find(|i| i.name == name) else {
                return Err(CliError {
                    code: 1,
                    source: anyhow::anyhow!(
                        "--bind-interface: {name:?} {}",
                        udap::interfaces::NOT_USABLE_REASON
                    ),
                });
            };
            Some(iface)
        }
        None => None,
    };

    match cli.command {
        Command::Discover => {
            cmd::discover::run(
                make_client,
                resolved_interface.as_ref(),
                cli.timeout,
                stdout,
                stderr,
            )
            .await
        }
        Command::Interfaces => {
            cmd::interfaces::run(stdout, stderr, udap::interfaces::enumerate);
            Ok(())
        }
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
