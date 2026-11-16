//! The runtime a binary serves on.

use std::error::Error;
use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::thread;

use tokio::runtime::{Builder, Runtime};

/// tokio's multi-thread runtime, its I/O and its timers on, with one worker per processor: the
/// count tokio picks by default, given explicitly so that tokio never reads
/// `TOKIO_WORKER_THREADS` (ADR-0029). A binary's configuration is its own typed one, and tokio
/// panics on a value of that variable it cannot read, printing the value.
pub fn build_runtime() -> Result<Runtime, RuntimeError> {
    Builder::new_multi_thread()
        .worker_threads(worker_threads())
        .enable_all()
        .build()
        .map_err(|source| RuntimeError { source })
}

fn worker_threads() -> usize {
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

/// The runtime could not start. The message names the step; the cause is the source.
#[derive(Debug)]
pub struct RuntimeError {
    source: io::Error,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the tokio runtime could not start")
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::with_sources;

    #[test]
    fn the_runtime_has_one_worker_per_processor() {
        let processors = thread::available_parallelism().unwrap().get();

        let runtime = build_runtime().unwrap();

        assert_eq!(runtime.metrics().num_workers(), processors);
    }

    #[test]
    fn the_runtime_runs_tasks_and_timers() {
        let runtime = build_runtime().unwrap();

        let answer = runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            tokio::spawn(async { 42 }).await.unwrap()
        });

        assert_eq!(answer, 42);
    }

    #[test]
    fn a_runtime_that_cannot_start_names_the_step_and_keeps_the_cause() {
        let error = RuntimeError {
            source: io::Error::other("no threads left"),
        };

        assert_eq!(error.to_string(), "the tokio runtime could not start");
        assert!(error.source().unwrap().is::<io::Error>());
        assert_eq!(
            with_sources(&error),
            "the tokio runtime could not start: no threads left"
        );
    }
}
