//! What a binary says on standard error: whatever no log can take, because the logs have not
//! started, because they are what failed, or because the binary runs a mode that has none.

use std::fmt::Display;
use std::io::{self, Write};
use std::process::ExitCode;

/// The exit status of a command line the binary does not understand, as with most tools.
const USAGE_ERROR: u8 = 2;

/// Writes `message` to standard error, and returns the exit status of a failure.
pub fn report_failure(message: &dyn Display) -> ExitCode {
    write_line(io::stderr().lock(), message);
    ExitCode::FAILURE
}

/// Writes `usage` to standard error, and returns the exit status of a command line the binary
/// does not understand.
pub fn report_usage(usage: &dyn Display) -> ExitCode {
    write_line(io::stderr().lock(), usage);
    ExitCode::from(USAGE_ERROR)
}

/// `message` and a newline. `eprintln!` would panic on a standard error that was closed; then
/// nowhere is left to say anything, and the exit status still tells the failure, so the write's
/// own failure is dropped.
fn write_line(mut writer: impl Write, message: &dyn Display) {
    let _ = writeln!(writer, "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standard error that was closed: every write fails.
    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn a_message_is_written_on_a_line_of_its_own() {
        let mut written = Vec::new();

        write_line(
            &mut written,
            &"APP_ENV is missing: expected one of local, staging, production",
        );

        assert_eq!(
            String::from_utf8(written).unwrap(),
            "APP_ENV is missing: expected one of local, staging, production\n"
        );
    }

    #[test]
    fn a_message_that_cannot_be_written_is_dropped_rather_than_panicking() {
        write_line(Closed, &"the logs could not start");
    }

    #[test]
    fn a_failure_exits_with_status_1_and_a_command_line_not_understood_with_status_2() {
        assert_eq!(
            report_failure(&"the logs could not start"),
            ExitCode::FAILURE
        );
        assert_eq!(report_usage(&"usage: barras-probe"), ExitCode::from(2));
    }
}
