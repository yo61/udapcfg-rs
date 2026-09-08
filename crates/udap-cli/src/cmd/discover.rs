//! The `discover` subcommand.

use crate::{CliError, ClientFactory};
use std::io::Write;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Discovers devices and prints one MAC per line.
///
/// Finding nothing is not an error: a note goes to stderr and the exit
/// code stays 0, matching go-udap.
///
/// # Errors
/// [`CliError`] with code 2 if the client cannot be built or discovery fails.
pub async fn run(
    make_client: ClientFactory,
    timeout: Duration,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), CliError> {
    let mut client = make_client().map_err(|e| CliError { code: 2, source: e })?;

    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(timeout).await;
        token.cancel();
    });

    client.discover(&cancel).await.map_err(|e| CliError {
        code: 2,
        source: anyhow::Error::new(e).context("discovery failed"),
    })?;

    let devices = client.devices();
    if devices.is_empty() {
        let _ = writeln!(
            stderr,
            "no devices found within {}",
            format_duration(timeout)
        );
        return Ok(());
    }
    for device in devices {
        let _ = writeln!(stdout, "{}", device.mac);
    }
    Ok(())
}

/// Renders a duration the way Go's `time.Duration` prints it, so the
/// "no devices found within 2s" line matches go-udap byte for byte.
fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms > 0 && ms.is_multiple_of(60_000) {
        format!("{}m0s", ms / 60_000)
    } else if ms.is_multiple_of(1_000) {
        format!("{}s", ms / 1_000)
    } else {
        format!("{ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations_like_go() {
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_millis(50)), "50ms");
        assert_eq!(format_duration(Duration::from_secs(120)), "2m0s");
    }
}
