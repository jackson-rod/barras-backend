//! The start-up both binaries share: the configuration read from the environment, the
//! logs, and the signal that stops them. Only the crates under `apps/` depend on it.

mod environment;
mod logging;
// SIGTERM is a Unix signal, and Barras builds and runs only on Unix.
#[cfg(unix)]
mod shutdown;

pub use environment::{
    AppEnv, ConfigError, ConfigProblem, Environment, FromEnvValue, LogLevel, ProblemKind,
};
pub use logging::{LoggingError, init_logging, subscriber};
#[cfg(unix)]
pub use shutdown::ShutdownSignal;
