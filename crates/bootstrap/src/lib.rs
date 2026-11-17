//! The start-up both binaries share: the configuration read from the environment, the
//! logs, the runtime they serve on, the signal that stops them, the text of a failure that
//! stops them, and how they say on standard error what no log can take. Only the crates under
//! `apps/` depend on it.

// Taken for its features, not code; named so that cargo never reports it as an unused dependency.
use workspace_hack as _;

mod environment;
mod logging;
mod panic_hook;
mod report;
mod runtime;
mod secret;
// SIGTERM is a Unix signal, and Barras builds and runs only on Unix.
#[cfg(unix)]
mod shutdown;
mod with_sources;

pub use environment::{
    AppEnv, ConfigError, ConfigProblem, Environment, FromEnvValue, LogLevel, ProblemKind,
};
pub use logging::{LoggingError, build_subscriber, init_logging};
pub use panic_hook::install_panic_hook;
pub use report::{report_failure, report_usage};
pub use runtime::{RuntimeError, build_runtime};
pub use secret::Secret;
#[cfg(unix)]
pub use shutdown::ShutdownSignal;
pub use with_sources::with_sources;
