//! Entry point for the `udapcfg` binary.

use clap::Parser;
use std::io::Write;
use std::process::ExitCode;
use udap_cli::{Cli, ClientFactory, run};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // clap's own parse-failure exit codes already match go-udap: 2 for a
    // usage error (cobra/pflag's own parse errors are never wrapped in
    // go-udap's ExitError, so they fall through to ExitCode's default of
    // 2 -- see cli.go's ExitCode/PersistentPreRunE), 0 for --help/--version.
    // No custom mapping needed: `Cli::parse()` calls `clap::Error::exit()`
    // internally, which already reproduces that split.
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(if cli.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::WARN
        })
        .init();

    let retries = cli.retries;
    let bind_interface = cli.bind_interface.clone();
    let all_interfaces = cli.all_interfaces;
    let factory: ClientFactory = Box::new(move || {
        let mut client = if let Some(name) = bind_interface.as_deref() {
            udap::Client::for_interface(name, udap::PORT)?
        } else if all_interfaces {
            // TODO(M3 task 4): replace with Client::for_all_interfaces once
            // MultiTransport lands; this arm exists only so this task
            // compiles and its own tests pass standalone.
            return Err(anyhow::anyhow!(
                "--all-interfaces lands with MultiTransport in the next task"
            ));
        } else {
            udap::Client::with_udp(udap::PORT)?
        };
        client.set_retries(retries);
        Ok(client)
    });

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    match run(cli, factory, &mut stdout, &mut stderr).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(&mut stderr, "error: {}", e.source);
            // CliError::code is documented as 0 (success, never reached here),
            // 1 (usage error) or 2 (operation failure); anything else is
            // programmer error in a future command, so fail loudly rather
            // than silently truncating or wrapping a signed cast.
            match e.code {
                1 => ExitCode::from(1u8),
                _ => ExitCode::from(2u8),
            }
        }
    }
}
