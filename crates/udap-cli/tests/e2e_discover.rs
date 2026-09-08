use std::sync::Arc;
use std::time::Duration;
use udap_cli::{Cli, Command, run};

/// Runs the CLI against an in-process mock, returning (stdout, stderr, exit code)
/// as raw bytes. Callers (which are `#[tokio::test]`-attributed, so
/// `allow-unwrap-in-tests` applies) decode to `String` themselves.
async fn run_cli(device_count: usize, timeout: Duration) -> (Vec<u8>, Vec<u8>, i32) {
    let network = Arc::new(mocksbr::Network::with_auto_devices(device_count));
    let factory = Box::new(move || {
        Ok(udap::Client::new(Box::new(mocksbr::MockTransport::new(
            Arc::clone(&network),
        ))))
    });
    let cli = Cli {
        timeout,
        verbose: false,
        retries: 0,
        command: Command::Discover,
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = match run(cli, factory, &mut out, &mut err).await {
        Ok(()) => 0,
        Err(e) => {
            use std::io::Write;
            let _ = writeln!(&mut err, "error: {}", e.source);
            e.code
        }
    };
    (out, err, code)
}

#[tokio::test]
async fn prints_one_mac_per_line_sorted() {
    let (stdout, _, code) = run_cli(3, Duration::from_millis(50)).await;
    let stdout = String::from_utf8(stdout).unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "00:04:20:00:00:01\n00:04:20:00:00:02\n00:04:20:00:00:03\n"
    );
}

#[tokio::test]
async fn reports_no_devices_on_stderr_and_exits_zero() {
    let (stdout, stderr, code) = run_cli(0, Duration::from_millis(50)).await;
    let stdout = String::from_utf8(stdout).unwrap();
    let stderr = String::from_utf8(stderr).unwrap();
    assert_eq!(code, 0, "finding nothing is not an error");
    assert!(stdout.is_empty(), "stdout must stay clean");
    assert_eq!(stderr, "no devices found within 50ms\n");
}

#[tokio::test]
async fn results_go_to_stdout_not_stderr() {
    let (stdout, stderr, _) = run_cli(1, Duration::from_millis(50)).await;
    let stdout = String::from_utf8(stdout).unwrap();
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stdout.contains("00:04:20:00:00:01"));
    assert!(stderr.is_empty());
}
