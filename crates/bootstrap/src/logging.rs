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
    subscriber(env, level, std::io::stdout)
        .try_init()
        .map_err(|source| LoggingError { source })
}

/// The subscriber [`init_logging`] installs, writing to `writer`. `Local` gives readable
/// lines without colours. Any other environment gives one JSON object per line, with
/// `timestamp` (RFC 3339, UTC), `level`, `target`, `message` and the event's fields at the
/// top level, and the fields of the enclosing spans under `spans`. An event below `level`
/// is not written.
pub fn subscriber<W>(env: AppEnv, level: LogLevel, writer: W) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let format = match env {
        AppEnv::Local => tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(writer)
            .boxed(),
        AppEnv::Staging | AppEnv::Production => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(true)
            .with_target(true)
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

    use serde_json::Value;
    use tracing::{debug, error, info, info_span, trace, warn};

    use super::*;

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
        let _default = subscriber(env, level, capture.clone()).set_default();
        emit();
        capture.lines()
    }

    fn json(line: &str) -> Value {
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
    fn a_deployed_environment_writes_json_with_the_four_fields_at_the_top_level() {
        for env in [AppEnv::Staging, AppEnv::Production] {
            let lines = lines_logged(env, LogLevel::Info, || {
                info!(status = 200, "request finished");
            });

            assert_eq!(lines.len(), 1, "{env:?}: {lines:?}");
            let line = json(&lines[0]);
            let timestamp = line["timestamp"].as_str().unwrap();
            assert!(is_rfc3339_utc(timestamp), "{timestamp}");
            assert_eq!(line["level"], "INFO");
            assert_eq!(line["target"], "barras_bootstrap::logging::tests");
            assert_eq!(line["message"], "request finished");
            assert_eq!(line["status"], 200);
        }
    }

    #[test]
    fn a_json_line_carries_the_fields_of_its_spans_under_spans() {
        let lines = lines_logged(AppEnv::Staging, LogLevel::Info, || {
            info_span!("request", request_id = "req-7").in_scope(|| info!("handled"));
        });

        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = json(&lines[0]);
        assert_eq!(line["spans"][0]["name"], "request");
        assert_eq!(line["spans"][0]["request_id"], "req-7");
        assert!(line.get("request_id").is_none(), "{line}");
    }

    #[test]
    fn every_json_line_is_one_whole_object() {
        let lines = lines_logged(AppEnv::Production, LogLevel::Info, || {
            info!("first");
            warn!(detail = "a \"quoted\"\nsecond line", "second");
            error!("third");
        });

        assert_eq!(lines.len(), 3, "{lines:?}");
        for line in &lines {
            assert!(json(line).is_object(), "{line}");
        }
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
            let levels: Vec<Value> = lines
                .iter()
                .map(|line| json(line)["level"].clone())
                .collect();
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
