//! The logs of a binary: readable lines on a developer machine, one JSON object per line
//! everywhere else.

use std::error::Error;
use std::fmt;

use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};

use crate::environment::{AppEnv, LogLevel};

/// Starts the logs of the process on standard output, in the format of `env`, from `level`
/// up. A process has one global subscriber, so a second call is an error.
pub fn init_logging(env: AppEnv, level: LogLevel) -> Result<(), LoggingError> {
    build_subscriber(env, level, std::io::stdout)
        .try_init()
        .map_err(|source| LoggingError { source })
}

/// The subscriber [`init_logging`] installs, writing to `writer`. An event below `level` is
/// not written.
///
/// `Local` gives readable lines without colours. Any other environment gives one JSON
/// object per line, with exactly these keys: `timestamp` (RFC 3339, UTC), `level`,
/// `target`, `fields` (the event's fields, `message` included) and, inside a span, `spans`
/// (the name and fields of each enclosing span, the outermost first). Whatever an event or
/// a span records stays inside `fields` or `spans`, so it cannot shadow those keys.
pub fn build_subscriber<W>(env: AppEnv, level: LogLevel, writer: W) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let format = match env {
        AppEnv::Local => tracing_subscriber::fmt::layer()
            // tracing-subscriber's `ansi` feature is off, but feature unification (through
            // cargo-hakari, say) could switch it on, and escape codes garble a log file.
            .with_ansi(false)
            .with_writer(writer)
            .boxed(),
        AppEnv::Staging | AppEnv::Production => tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(false)
            .with_span_list(true)
            .with_writer(writer)
            .boxed(),
    };
    tracing_subscriber::registry()
        .with(format)
        .with(level_filter(level))
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Trace => LevelFilter::TRACE,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}

/// The logs could not start: the process already has a global subscriber.
#[derive(Debug)]
pub struct LoggingError {
    source: TryInitError,
}

impl fmt::Display for LoggingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the logs could not start: the process already has a global subscriber")
    }
}

impl Error for LoggingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use serde::Deserialize;
    use serde_json::{Map, Value};
    use tracing::{debug, error, info, info_span, trace, warn};

    use super::*;

    /// One JSON line. A key outside these, or a key written twice, fails the parse, so a
    /// line that parses has exactly this shape.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct JsonLine {
        timestamp: String,
        level: String,
        target: String,
        fields: Map<String, Value>,
        spans: Option<Vec<Map<String, Value>>>,
    }

    /// Keeps what a subscriber writes, for the test that holds a clone.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn lines(&self) -> Vec<String> {
            let written = self.0.lock().unwrap().clone();
            let text = String::from_utf8(written).unwrap();
            text.lines().map(str::to_owned).collect()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Capture {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    /// The lines written while `emit` runs. The subscriber is the default of this thread
    /// only, so tests running side by side do not see each other's events.
    fn lines_logged(env: AppEnv, level: LogLevel, emit: impl FnOnce()) -> Vec<String> {
        let capture = Capture::default();
        let _default = build_subscriber(env, level, capture.clone()).set_default();
        emit();
        capture.lines()
    }

    fn parse(line: &str) -> JsonLine {
        serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}"))
    }

    /// `YYYY-MM-DDTHH:MM:SS`, an optional fraction of a second, then `Z` for UTC.
    fn is_rfc3339_utc(text: &str) -> bool {
        let Some(local) = text.strip_suffix('Z') else {
            return false;
        };
        let (seconds, fraction) = local.split_once('.').unwrap_or((local, "0"));
        let shape = "dddd-dd-ddTdd:dd:dd";
        seconds.len() == shape.len()
            && seconds.bytes().zip(shape.bytes()).all(|(byte, expected)| {
                if expected == b'd' {
                    byte.is_ascii_digit()
                } else {
                    byte == expected
                }
            })
            && !fraction.is_empty()
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
    }

    #[test]
    fn the_line_parser_refuses_a_key_written_twice() {
        let doubled = r#"{"timestamp":"t","level":"INFO","level":"x","target":"t","fields":{}}"#;
        let error = serde_json::from_str::<JsonLine>(doubled).unwrap_err();
        assert!(
            error.to_string().contains("duplicate field `level`"),
            "{error}"
        );
    }

    #[test]
    fn a_deployed_environment_writes_one_json_object_per_event() {
        for env in [AppEnv::Staging, AppEnv::Production] {
            let lines = lines_logged(env, LogLevel::Info, || {
                info!(status = 200, "request finished");
            });

            assert_eq!(lines.len(), 1, "{env:?}: {lines:?}");
            let line = parse(&lines[0]);
            assert!(is_rfc3339_utc(&line.timestamp), "{}", line.timestamp);
            assert_eq!(line.level, "INFO");
            assert_eq!(line.target, "barras_bootstrap::logging::tests");
            assert_eq!(line.fields["message"], "request finished");
            assert_eq!(line.fields["status"], 200);
            assert!(line.spans.is_none(), "{env:?}");
        }
    }

    #[test]
    fn a_json_line_carries_the_fields_of_its_spans_under_spans() {
        let lines = lines_logged(AppEnv::Staging, LogLevel::Info, || {
            info_span!("request", request_id = "req-7").in_scope(|| info!("handled"));
        });

        assert_eq!(lines.len(), 1, "{lines:?}");
        let spans = parse(&lines[0]).spans.unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0]["name"], "request");
        assert_eq!(spans[0]["request_id"], "req-7");
    }

    #[test]
    fn no_field_can_shadow_a_key_of_the_line() {
        let lines = lines_logged(AppEnv::Production, LogLevel::Info, || {
            info_span!("request", level = "span-level").in_scope(|| {
                info!(
                    level = "spoofed-level",
                    timestamp = "spoofed-time",
                    target = "spoofed-target",
                    spans = "spoofed-spans",
                    "probe"
                );
            });
        });

        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = parse(&lines[0]);
        assert!(is_rfc3339_utc(&line.timestamp), "{}", line.timestamp);
        assert_eq!(line.level, "INFO");
        assert_eq!(line.target, "barras_bootstrap::logging::tests");
        assert_eq!(line.fields["level"], "spoofed-level");
        assert_eq!(line.fields["timestamp"], "spoofed-time");
        assert_eq!(line.fields["target"], "spoofed-target");
        assert_eq!(line.fields["spans"], "spoofed-spans");
        assert_eq!(line.fields["message"], "probe");
        assert_eq!(line.spans.unwrap()[0]["level"], "span-level");
    }

    #[test]
    fn every_json_line_is_one_whole_object() {
        let lines = lines_logged(AppEnv::Production, LogLevel::Info, || {
            info!("first");
            warn!(detail = "a \"quoted\"\nsecond line", "second");
            error!("third");
        });

        let messages: Vec<Value> = lines
            .iter()
            .map(|line| parse(line).fields["message"].clone())
            .collect();
        assert_eq!(messages, ["first", "second", "third"]);
    }

    #[test]
    fn local_writes_readable_lines_without_colours() {
        let lines = lines_logged(AppEnv::Local, LogLevel::Info, || {
            info_span!("request", request_id = "req-7").in_scope(|| info!(status = 200, "handled"));
        });

        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(serde_json::from_str::<Value>(line).is_err(), "{line}");
        assert!(!line.contains('\u{1b}'), "{line:?}");
        let timestamp = line.split_whitespace().next().unwrap();
        assert!(is_rfc3339_utc(timestamp), "{line}");
        for part in [
            "INFO",
            "request",
            "req-7",
            "barras_bootstrap::logging::tests",
            "handled",
            "status=200",
        ] {
            assert!(line.contains(part), "{part:?} in {line}");
        }
    }

    #[test]
    fn an_event_below_the_chosen_level_is_not_written() {
        let every_level = || {
            trace!("trace");
            debug!("debug");
            info!("info");
            warn!("warn");
            error!("error");
        };
        for (level, written) in [
            (
                LogLevel::Trace,
                &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"][..],
            ),
            (LogLevel::Debug, &["DEBUG", "INFO", "WARN", "ERROR"]),
            (LogLevel::Info, &["INFO", "WARN", "ERROR"]),
            (LogLevel::Warn, &["WARN", "ERROR"]),
            (LogLevel::Error, &["ERROR"]),
        ] {
            let lines = lines_logged(AppEnv::Staging, level, every_level);
            let levels: Vec<String> = lines.iter().map(|line| parse(line).level).collect();
            assert_eq!(levels, written, "{level:?}");
        }
    }

    #[test]
    fn starting_the_logs_a_second_time_is_an_error_not_a_panic() {
        init_logging(AppEnv::Local, LogLevel::Info).unwrap();

        let error = init_logging(AppEnv::Staging, LogLevel::Warn).unwrap_err();

        assert!(error.source().is_some());
        assert!(!error.to_string().is_empty());
    }
}
