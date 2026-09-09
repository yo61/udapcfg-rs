//! The `interfaces` subcommand.

use crate::output;
use std::io::Write;

/// Lists interfaces usable for discovery.
///
/// Finding none is not an error — a note goes to stderr and the exit
/// code stays 0, matching `discover`. Enumeration itself is infallible
/// (see [`udap::interfaces::enumerate`]), so this cannot fail either.
pub fn run(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    enumerate: impl Fn() -> Vec<udap::NetInterface>,
) {
    let ifs = enumerate();
    if ifs.is_empty() {
        let _ = writeln!(stderr, "no usable interfaces found");
        return;
    }
    output::format_interfaces_table(stdout, &ifs);
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
        let enumerate = || sample.clone();

        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, enumerate);

        let stdout = String::from_utf8(out).expect("utf8");
        let stderr = String::from_utf8(err).expect("utf8");
        assert!(stdout.contains("NAME"), "header must be in stdout");
        assert!(stdout.contains("en0"), "interface data must be in stdout");
        assert!(stderr.is_empty(), "stderr must be empty on success");
    }

    #[test]
    fn empty_path_writes_message_to_stderr() {
        let enumerate = || vec![];

        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&mut out, &mut err, enumerate);

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
