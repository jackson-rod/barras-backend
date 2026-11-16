//! The start-up both binaries share: the configuration read from the environment, the
//! logs, the signal that stops them, and the text of a failure that stops them. Only the
//! crates under `apps/` depend on it.

// Taken for its features, not code; named so cargo's unused-dependency lint lets `just lint` pass.
use workspace_hack as _;

mod environment;
mod logging;
mod panic_hook;
// SIGTERM is a Unix signal, and Barras builds and runs only on Unix.
#[cfg(unix)]
mod shutdown;
mod with_sources;

pub use environment::{
    AppEnv, ConfigError, ConfigProblem, Environment, FromEnvValue, LogLevel, ProblemKind,
};
pub use logging::{LoggingError, build_subscriber, init_logging};
pub use panic_hook::install_panic_hook;
#[cfg(unix)]
pub use shutdown::ShutdownSignal;
pub use with_sources::with_sources;
