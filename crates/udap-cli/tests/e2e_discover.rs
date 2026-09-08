use go_duration::GoDuration;
use std::sync::Arc;
use udap_cli::{Cli, Command, run};

/// Runs the CLI against an in-process mock, returning (stdout, stderr, exit code)
/// as raw bytes. Callers (which are `#[tokio::test]`-attributed, so
/// `allow-unwrap-in-tests` applies) decode to `String` themselves.
async fn run_cli(device_count: usize, timeout_ms: i64) -> (Vec<u8>, Vec<u8>, i32) {
    let network = Arc::new(mocksbr::Network::with_auto_devices(device_count));
    let factory = Box::new(move || {
        Ok(udap::Client::new(Box::new(mocksbr::MockTransport::new(
            Arc::clone(&network),
        ))))
    });
    let cli = Cli {
        timeout: GoDuration::from(timeout_ms * 1_000_000),
        verbose: false,
        retries: 0,
        bind_interface: None,
        all_interfaces: false,
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
    let (stdout, _, code) = run_cli(3, 50).await;
    let stdout = String::from_utf8(stdout).unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "00:04:20:00:00:01\n00:04:20:00:00:02\n00:04:20:00:00:03\n"
    );
}

#[tokio::test]
async fn reports_no_devices_on_stderr_and_exits_zero() {
    let (stdout, stderr, code) = run_cli(0, 50).await;
    let stdout = String::from_utf8(stdout).unwrap();
    let stderr = String::from_utf8(stderr).unwrap();
    assert_eq!(code, 0, "finding nothing is not an error");
    assert!(stdout.is_empty(), "stdout must stay clean");
    assert_eq!(stderr, "no devices found within 50ms\n");
}

#[tokio::test]
async fn results_go_to_stdout_not_stderr() {
    let (stdout, stderr, _) = run_cli(1, 50).await;
    let stdout = String::from_utf8(stdout).unwrap();
    let stderr = String::from_utf8(stderr).unwrap();
    assert!(stdout.contains("00:04:20:00:00:01"));
    assert!(stderr.is_empty());
}

use clap::Parser;

#[test]
fn bind_interface_and_all_interfaces_are_mutually_exclusive() {
    let err = Cli::try_parse_from([
        "udapcfg",
        "--bind-interface",
        "en0",
        "--all-interfaces",
        "discover",
    ])
    .expect_err("the two flags must conflict");
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn retries_defaults_to_zero_and_parses() {
    let cli = Cli::try_parse_from(["udapcfg", "discover"]).expect("parse");
    assert_eq!(cli.retries, 0);
    let cli = Cli::try_parse_from(["udapcfg", "--retries", "2", "discover"]).expect("parse");
    assert_eq!(cli.retries, 2);
}

#[test]
fn negative_retries_is_rejected() {
    assert!(Cli::try_parse_from(["udapcfg", "--retries", "-1", "discover"]).is_err());
}

#[tokio::test]
async fn unknown_bind_interface_is_a_usage_error() {
    let factory: udap_cli::ClientFactory =
        Box::new(|| Err(anyhow::anyhow!("factory must not be reached")));
    let cli = Cli {
        timeout: GoDuration::from(50_000_000),
        verbose: false,
        retries: 0,
        bind_interface: Some("definitely-not-an-interface0".to_owned()),
        all_interfaces: false,
        command: Command::Discover,
    };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let e = run(cli, factory, &mut out, &mut err)
        .await
        .expect_err("an unusable interface is an error");

    // go-udap treats this as a usage error, not an operation failure
    // (cli/cli.go:124 returns ExitError{Code: 1}).
    assert_eq!(e.code, 1, "unusable interface must exit 1, not 2");
    assert!(
        e.source.to_string().contains("is not usable"),
        "message must match go-udap: {}",
        e.source
    );
    assert!(out.is_empty(), "stdout stays clean on a usage error");
}
