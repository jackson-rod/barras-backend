//! The lines a binary logs, read as its tests read them: one JSON object per line in every
//! environment but `local`, readable lines in `local`.

use std::net::SocketAddr;

use serde_json::Value;

use super::text;

/// Each line of `log`, parsed as JSON; the test fails on a line that is not.
pub fn json_lines(log: &[u8]) -> Vec<Value> {
    text(log)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line:?}")))
        .collect()
}

/// Whether `line`, JSON or readable, is the one a binary logs once its listeners are bound.
pub fn is_listening(line: &str) -> bool {
    match serde_json::from_str::<Value>(line) {
        Ok(json) => json["fields"]["message"] == "listening",
        Err(_) => line.contains(": listening "),
    }
}

/// The address the `listening` line gives for `field`, whether the line is JSON or readable;
/// the test fails when it gives none.
pub fn address_in(line: &str, field: &str) -> SocketAddr {
    let address = match serde_json::from_str::<Value>(line) {
        Ok(json) => json["fields"][field].as_str().map(str::to_owned),
        Err(_) => {
            let prefix = format!("{field}=");
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(&prefix))
                .map(str::to_owned)
        }
    };
    address
        .and_then(|address| address.parse().ok())
        .unwrap_or_else(|| panic!("no {field} in {line:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `listening` line of `barras-api`, as `APP_ENV=staging` logs it.
    const JSON_LINE: &str = r#"{"timestamp":"2026-11-16T10:00:00.000000Z","level":"INFO","target":"barras_api","fields":{"message":"listening","public_addr":"127.0.0.1:41000","internal_addr":"127.0.0.1:41001"}}"#;

    /// The same line, as `APP_ENV=local` logs it.
    const READABLE_LINE: &str = "2026-11-16T10:00:00.000000Z  INFO barras_api: listening \
                                 public_addr=127.0.0.1:41000 internal_addr=127.0.0.1:41001";

    #[test]
    fn each_line_of_a_log_is_parsed() {
        let log = format!("{JSON_LINE}\n{{\"fields\":{{\"message\":\"stopped\"}}}}\n");

        let lines = json_lines(log.as_bytes());

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1]["fields"]["message"], "stopped");
    }

    #[test]
    #[should_panic(expected = "\"listening on 8080\"")]
    fn a_line_that_is_not_json_fails_the_test() {
        json_lines(format!("{JSON_LINE}\nlistening on 8080\n").as_bytes());
    }

    #[test]
    fn the_listening_line_is_told_apart_whether_json_or_readable() {
        assert!(is_listening(JSON_LINE));
        assert!(is_listening(READABLE_LINE));
        for other in [
            r#"{"fields":{"message":"stopped"}}"#,
            "2026-11-16T10:00:00.000000Z  INFO barras_api: stopped",
        ] {
            assert!(!is_listening(other), "{other}");
        }
    }

    #[test]
    fn each_address_is_read_whether_the_line_is_json_or_readable() {
        for line in [JSON_LINE, READABLE_LINE] {
            assert_eq!(
                address_in(line, "public_addr"),
                "127.0.0.1:41000".parse().unwrap()
            );
            assert_eq!(
                address_in(line, "internal_addr"),
                "127.0.0.1:41001".parse().unwrap()
            );
        }
    }

    #[test]
    #[should_panic(expected = "no metrics_addr in")]
    fn a_line_without_the_address_fails_the_test() {
        address_in(JSON_LINE, "metrics_addr");
    }
}
