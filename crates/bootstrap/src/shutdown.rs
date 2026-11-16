//! The signal that stops a binary.

use std::io;

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
    /// start-up error.
    ///
    /// # Panics
    ///
    /// tokio panics when this runs outside a tokio runtime, or in one built without its
    /// I/O driver (`enable_io` or `enable_all`). The apps call it inside their async `run`.
    pub fn install() -> io::Result<Self> {
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            // SIGINT is what `tokio::signal::ctrl_c` waits for on Unix, but that function
            // installs its handler only when first polled, with the servers already
            // running, so its failure would not be a start-up error.
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Resolves at the first SIGTERM or Ctrl-C.
    pub async fn recv(mut self) {
        tokio::select! {
            _ = self.terminate.recv() => {}
            _ = self.interrupt.recv() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::task::{Context, Waker};

    use super::*;

    // No test sends a signal: it would reach the whole test process. The process tests of
    // the binaries send real ones.

    #[tokio::test]
    async fn both_handlers_install_inside_a_runtime() {
        assert!(ShutdownSignal::install().is_ok());
    }

    #[tokio::test]
    async fn recv_waits_while_no_signal_has_arrived() {
        let shutdown = ShutdownSignal::install().unwrap();
        let mut recv = pin!(shutdown.recv());
        let mut context = Context::from_waker(Waker::noop());
        assert!(recv.as_mut().poll(&mut context).is_pending());
    }
}
