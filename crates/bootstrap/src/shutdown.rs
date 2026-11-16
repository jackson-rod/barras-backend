//! The signal that stops a binary.

use std::io;

use tokio::runtime::Handle;
use tokio::signal::unix::{Signal, SignalKind, signal};

/// SIGTERM and Ctrl-C (SIGINT), listened for from the moment [`install`](Self::install)
/// returns, so that a signal sent before [`recv`](Self::recv) is awaited is not lost.
#[derive(Debug)]
pub struct ShutdownSignal {
    terminate: Signal,
    interrupt: Signal,
}

impl ShutdownSignal {
    /// Installs both handlers. Call it before anything is served, so that a failure is a
    /// start-up error. Outside a tokio runtime it is an error too.
    ///
    /// # Panics
    ///
    /// tokio panics when the runtime was built without its I/O driver (`enable_io` or
    /// `enable_all`).
    pub fn install() -> io::Result<Self> {
        // Outside a runtime `signal` would panic.
        Handle::try_current().map_err(io::Error::other)?;
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            // SIGINT is what `tokio::signal::ctrl_c` waits for on Unix, but that function
            // installs its handler only when first polled, with the servers already
            // running, so its failure would not be a start-up error.
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Resolves at the next SIGTERM or Ctrl-C. A signal that arrived since `install`, or
    /// since the last call returned, ends it at once.
    ///
    /// Tokio never removes its handlers, so after the first signal the process keeps
    /// catching both: a second one, during the drain, is ignored until `recv` is called
    /// again.
    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.terminate.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, poll_fn};
    use std::pin::{Pin, pin};
    use std::process::Command;
    use std::sync::{Mutex, PoisonError};
    use std::task::Poll;
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    /// A signal reaches the whole process. When tests share one (`cargo test`; nextest
    /// gives each test its own), the tests that send signals run one at a time, so that
    /// none of them sees another's.
    static SIGNALS: Mutex<()> = Mutex::new(());

    /// Runs `test` inside a tokio runtime, alone among the tests that send signals.
    fn run_alone(test: impl Future<Output = ()>) {
        let _alone = SIGNALS.lock().unwrap_or_else(PoisonError::into_inner);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(test);
    }

    /// Sends `signal` to this test's own process with the `kill` program. The handlers
    /// are installed by then, so the process lives on.
    fn send_to_self(signal: &str) {
        let status = Command::new("kill")
            .args(["-s", signal, &std::process::id().to_string()])
            .status()
            .unwrap();
        assert!(status.success(), "kill -s {signal}: {status}");
    }

    /// Fails unless `future` completes within five seconds.
    async fn completes(future: impl Future<Output = ()>) {
        let outcome = timeout(Duration::from_secs(5), future).await;
        assert!(outcome.is_ok(), "still waiting after five seconds");
    }

    /// Whether `future` is still waiting when polled once, from the test's own task.
    async fn is_pending<F: Future + Unpin>(future: &mut F) -> bool {
        poll_fn(|context| Poll::Ready(Pin::new(&mut *future).poll(context).is_pending())).await
    }

    #[test]
    fn install_outside_a_runtime_is_an_error_not_a_panic() {
        assert!(ShutdownSignal::install().is_err());
    }

    #[test]
    fn sigterm_ends_a_waiting_recv() {
        run_alone(async {
            let mut shutdown = ShutdownSignal::install().unwrap();
            let mut recv = pin!(shutdown.recv());
            assert!(is_pending(&mut recv).await);

            send_to_self("TERM");

            completes(recv).await;
        });
    }

    #[test]
    fn sigint_ends_a_waiting_recv() {
        run_alone(async {
            let mut shutdown = ShutdownSignal::install().unwrap();
            let mut recv = pin!(shutdown.recv());
            assert!(is_pending(&mut recv).await);

            send_to_self("INT");

            completes(recv).await;
        });
    }

    #[test]
    fn a_signal_sent_before_recv_is_kept() {
        run_alone(async {
            let mut shutdown = ShutdownSignal::install().unwrap();
            let mut delivered = signal(SignalKind::terminate()).unwrap();

            send_to_self("TERM");
            // Once this listener has the signal, tokio has handed it to every listener.
            completes(async {
                delivered.recv().await;
            })
            .await;

            completes(shutdown.recv()).await;
        });
    }

    #[test]
    fn recv_returns_again_at_the_next_signal() {
        run_alone(async {
            let mut shutdown = ShutdownSignal::install().unwrap();

            send_to_self("TERM");
            completes(shutdown.recv()).await;

            send_to_self("TERM");
            completes(shutdown.recv()).await;
        });
    }
}
