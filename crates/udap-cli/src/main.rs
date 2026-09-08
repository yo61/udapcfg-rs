//! Entry point for the `udapcfg` binary.

use clap::Parser;
use std::io::Write;
use std::process::ExitCode;
use udap_cli::{Cli, ClientFactory, run};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
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
    let factory: ClientFactory = Box::new(move || {
        // M3 replaces this with the real UDP transport.
        Err(anyhow::anyhow!(
            "no transport available yet: the UDP transport lands in M3 \
             (retries={retries} will apply then)"
        ))
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
