//! What a panic leaves in the logs.

use std::panic::{self, PanicHookInfo};

/// What the payload of a panic raised with a value that is not text reads as, such as
/// `panic_any(42)`.
const NOT_TEXT: &str = "a payload that is not text";

/// Replaces the default panic hook, whose raw line on standard error sits outside the JSON log,
/// with one that logs every panic as an `error` event: its payload and where it happened, inside
/// whatever spans were current, a request's included. Call it once the logs are up.
pub fn install_panic_hook() {
    panic::set_hook(Box::new(log_panic));
}

fn log_panic(info: &PanicHookInfo<'_>) {
    tracing::error!(
        payload = info.payload_as_str().unwrap_or(NOT_TEXT),
        location = info.location().map(tracing::field::display),
        "a thread panicked"
    );
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::panic::{self, UnwindSafe};
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::Value;
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::util::SubscriberInitExt;

    use super::*;
    use crate::{AppEnv, LogLevel, build_subscriber};

    /// The panic hook belongs to the whole process. When tests share one (`cargo test`; nextest
    /// gives each test its own), the tests that replace it run one at a time.
    static HOOK: Mutex<()> = Mutex::new(());

    /// Keeps what a subscriber writes, for the test that holds a clone.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

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

    /// The JSON lines logged while `work` panics with the hook installed. The logs are the
    /// default of this thread, which is where a hook runs: on the thread that panicked.
    fn logged_while_panicking(work: impl FnOnce() + UnwindSafe) -> Vec<Value> {
        let _alone = HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        let capture = Capture::default();
        let _logs =
            build_subscriber(AppEnv::Production, LogLevel::Info, capture.clone()).set_default();
        let previous = panic::take_hook();
        install_panic_hook();

        let outcome = panic::catch_unwind(work);

        panic::set_hook(previous);
        assert!(outcome.is_err(), "the work did not panic");
        let written = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        written
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_panic_is_an_error_event_with_its_payload_and_its_location() {
        let line = line!() + 1;
        let events = logged_while_panicking(|| panic!("the cache is {} entries short", 3));

        assert_eq!(events.len(), 1, "{events:?}");
        let event = &events[0];
        assert_eq!(event["level"], "ERROR");
        assert_eq!(event["fields"]["message"], "a thread panicked");
        assert_eq!(event["fields"]["payload"], "the cache is 3 entries short");
        let location = event["fields"]["location"].as_str().unwrap();
        let expected = format!("{}:{line}:", file!());
        assert!(location.starts_with(&expected), "{location}");
    }

    #[test]
    fn a_payload_that_is_not_text_is_named_but_not_shown() {
        let events = logged_while_panicking(|| panic::panic_any(42_u32));

        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0]["fields"]["payload"], "a payload that is not text");
    }

    #[test]
    fn a_panic_inside_a_request_carries_the_request_id() {
        // As the HTTP adapter opens it: at `error`, so that it survives every log level.
        let request = || tracing::error_span!("request", request_id = "r-1");

        let events = logged_while_panicking(|| request().in_scope(|| panic!("the handler failed")));

        assert_eq!(events.len(), 1, "{events:?}");
        let spans = &events[0]["spans"];
        assert_eq!(spans[0]["name"], "request", "{spans}");
        assert_eq!(spans[0]["request_id"], "r-1", "{spans}");
    }
}
