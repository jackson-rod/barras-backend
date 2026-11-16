//! The configuration of a binary, read from environment variables into typed values.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::net::SocketAddr;

/// Where the binary runs. It decides the log format: readable lines on `Local`, JSON
/// everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Local,
    Staging,
    Production,
}

/// The lowest level the logs write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// A type a configuration value is read into.
pub trait FromEnvValue: Sized {
    /// What a value must look like, as the problem line says it.
    const EXPECTED: &'static str;

    /// The value the text stands for, or `None`. [`Environment`] never passes an empty
    /// text, and never trims one.
    fn from_env_value(raw: &str) -> Option<Self>;
}

impl FromEnvValue for AppEnv {
    const EXPECTED: &'static str = "one of local, staging, production";

    fn from_env_value(raw: &str) -> Option<Self> {
        match raw {
            "local" => Some(Self::Local),
            "staging" => Some(Self::Staging),
            "production" => Some(Self::Production),
            _ => None,
        }
    }
}

impl FromEnvValue for LogLevel {
    const EXPECTED: &'static str = "one of trace, debug, info, warn, error";

    fn from_env_value(raw: &str) -> Option<Self> {
        match raw {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

impl FromEnvValue for SocketAddr {
    const EXPECTED: &'static str = "a socket address such as 127.0.0.1:8080";

    fn from_env_value(raw: &str) -> Option<Self> {
        raw.parse().ok()
    }
}

/// Finds the value of a variable by its name.
type Lookup = dyn Fn(&str) -> Option<OsString> + Send + Sync;

/// Reads a configuration from environment variables, recording every problem instead of
/// stopping at the first, so that a binary reports them all at once.
///
/// A value may be a secret, so none ever appears in an error. For the same reason the
/// type has no `Debug`.
pub struct Environment {
    lookup: Box<Lookup>,
    problems: Vec<ConfigProblem>,
}

impl Environment {
    /// The process environment, read one name at a time as [`required`](Self::required)
    /// asks for it, so nothing else in it is copied. The only reader of the process
    /// environment (ADR-0016).
    #[allow(clippy::disallowed_methods)] // ADR-0016: the one reader of the process environment
    pub fn from_process() -> Self {
        Self::with_lookup(|name| std::env::var_os(name))
    }

    /// The given pairs and nothing else: for tests, and for checking `.env.example`. A
    /// name given twice keeps its last value, as in a shell.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<OsString>,
    {
        let values: HashMap<String, OsString> = pairs
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        Self::with_lookup(move |name| values.get(name).cloned())
    }

    fn with_lookup(lookup: impl Fn(&str) -> Option<OsString> + Send + Sync + 'static) -> Self {
        Self {
            lookup: Box::new(lookup),
            problems: Vec::new(),
        }
    }

    /// The value of `name` as a `T`. `None` after recording why there is none: the
    /// variable is missing (not set, or set to the empty string) or not valid (not
    /// Unicode, or not a `T`).
    pub fn required<T: FromEnvValue>(&mut self, name: &'static str) -> Option<T> {
        let kind = match (self.lookup)(name) {
            None => ProblemKind::Missing,
            Some(value) if value.is_empty() => ProblemKind::Missing,
            Some(value) => match value.to_str().and_then(T::from_env_value) {
                Some(parsed) => return Some(parsed),
                None => ProblemKind::Invalid,
            },
        };
        self.problems.push(ConfigProblem {
            name,
            kind,
            expected: T::EXPECTED,
        });
        None
    }

    /// Ends the reading. Returns the configuration the caller assembled from what
    /// [`required`](Self::required) returned when no problem was recorded, and otherwise
    /// every problem, in the order the names were asked for.
    ///
    /// `required` returns `None` only after recording a problem, so `config` is `None`
    /// exactly when there is one. A `None` with no problem recorded is still an error.
    pub fn finish<T>(self, config: Option<T>) -> Result<T, ConfigError> {
        match config {
            Some(config) if self.problems.is_empty() => Ok(config),
            _ => Err(ConfigError {
                problems: self.problems,
            }),
        }
    }
}

/// Every problem found while reading a configuration, in the order the names were asked
/// for. Its `Display` is one line per problem, which is what a binary prints before it
/// stops. No line repeats a value.
#[derive(Debug)]
pub struct ConfigError {
    problems: Vec<ConfigProblem>,
}

impl ConfigError {
    /// Every problem, in the order the names were asked for.
    pub fn problems(&self) -> &[ConfigProblem] {
        &self.problems
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some((first, rest)) = self.problems.split_first() else {
            return f.write_str(
                "the configuration is incomplete, but no variable is missing or not valid",
            );
        };
        write!(f, "{first}")?;
        for problem in rest {
            write!(f, "\n{problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// One variable that could not be read: its name, what went wrong, and what its value
/// must look like. It never holds the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigProblem {
    name: &'static str,
    kind: ProblemKind,
    expected: &'static str,
}

impl ConfigProblem {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn kind(&self) -> ProblemKind {
        self.kind
    }
}

/// `<NAME> is missing: expected <EXPECTED>` or `<NAME> is not valid: expected <EXPECTED>`.
impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            ProblemKind::Missing => "is missing",
            ProblemKind::Invalid => "is not valid",
        };
        write!(f, "{} {what}: expected {}", self.name, self.expected)
    }
}

/// What went wrong with a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemKind {
    /// Not set, or set to the empty string.
    Missing,
    /// Set, but not Unicode or not a value of its type.
    Invalid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProblemKind::{Invalid, Missing};

    /// The name and the kind of every problem, in order.
    fn summary(error: &ConfigError) -> Vec<(&'static str, ProblemKind)> {
        error
            .problems()
            .iter()
            .map(|problem| (problem.name(), problem.kind()))
            .collect()
    }

    #[test]
    fn values_are_read_into_their_types() {
        let mut env = Environment::from_pairs([
            ("APP_ENV", "staging"),
            ("LOG_LEVEL", "debug"),
            ("API_BIND_ADDR", "127.0.0.1:8080"),
        ]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let bind_addr = env.required::<SocketAddr>("API_BIND_ADDR");

        let config = env.finish(app_env.zip(log_level).zip(bind_addr)).unwrap();

        let address: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        assert_eq!(config, ((AppEnv::Staging, LogLevel::Debug), address));
    }

    #[test]
    fn an_app_env_is_one_of_three_lower_case_names() {
        for (raw, app_env) in [
            ("local", AppEnv::Local),
            ("staging", AppEnv::Staging),
            ("production", AppEnv::Production),
        ] {
            assert_eq!(AppEnv::from_env_value(raw), Some(app_env), "{raw:?}");
        }
        for raw in [
            "Local",
            "LOCAL",
            "prod",
            "dev",
            "development",
            "test",
            " local",
            "local ",
            "local\n",
        ] {
            assert_eq!(AppEnv::from_env_value(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn a_log_level_is_one_of_five_lower_case_names() {
        for (raw, level) in [
            ("trace", LogLevel::Trace),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("warn", LogLevel::Warn),
            ("error", LogLevel::Error),
        ] {
            assert_eq!(LogLevel::from_env_value(raw), Some(level), "{raw:?}");
        }
        for raw in [
            "INFO", "Info", "warning", "fatal", "off", "verbose", "3", " info", "info ",
        ] {
            assert_eq!(LogLevel::from_env_value(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn a_socket_address_is_read_by_the_standard_parser() {
        for raw in ["127.0.0.1:8080", "0.0.0.0:0", "[::1]:9080"] {
            let address = SocketAddr::from_env_value(raw);
            assert_eq!(address, Some(raw.parse().unwrap()), "{raw:?}");
        }
        for raw in [
            "localhost:8080",
            "127.0.0.1",
            ":8080",
            "127.0.0.1:65536",
            " 127.0.0.1:8080",
            "127.0.0.1:8080 ",
            "http://127.0.0.1:8080",
        ] {
            assert_eq!(SocketAddr::from_env_value(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn the_expected_values_are_spelled_out() {
        assert_eq!(AppEnv::EXPECTED, "one of local, staging, production");
        assert_eq!(LogLevel::EXPECTED, "one of trace, debug, info, warn, error");
        assert_eq!(
            SocketAddr::EXPECTED,
            "a socket address such as 127.0.0.1:8080"
        );
    }

    #[test]
    fn a_variable_that_is_not_set_is_missing() {
        let mut env = Environment::from_pairs([("APP_ENVIRONMENT", "local")]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        assert_eq!(app_env, None);
        let error = env.finish(app_env).unwrap_err();
        assert_eq!(summary(&error), [("APP_ENV", Missing)]);
    }

    #[test]
    fn an_empty_value_is_missing() {
        let mut env = Environment::from_pairs([("APP_ENV", "")]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let error = env.finish(app_env).unwrap_err();
        assert_eq!(summary(&error), [("APP_ENV", Missing)]);
    }

    #[test]
    fn a_value_its_type_does_not_accept_is_not_valid() {
        let mut env = Environment::from_pairs([("LOG_LEVEL", "loud")]);
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let error = env.finish(log_level).unwrap_err();
        assert_eq!(summary(&error), [("LOG_LEVEL", Invalid)]);
    }

    #[test]
    fn values_are_not_trimmed() {
        let mut env = Environment::from_pairs([("APP_ENV", " local"), ("LOG_LEVEL", " ")]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let error = env.finish(app_env.zip(log_level)).unwrap_err();
        assert_eq!(
            summary(&error),
            [("APP_ENV", Invalid), ("LOG_LEVEL", Invalid)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_value_that_is_not_unicode_is_not_valid() {
        use std::os::unix::ffi::OsStringExt;

        let not_unicode = OsString::from_vec(b"loc\xffal".to_vec());
        let mut env = Environment::from_pairs([("APP_ENV", not_unicode)]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let error = env.finish(app_env).unwrap_err();
        assert_eq!(summary(&error), [("APP_ENV", Invalid)]);
    }

    #[test]
    fn every_problem_is_reported_in_the_order_asked() {
        let mut env = Environment::from_pairs([
            ("API_INTERNAL_BIND_ADDR", ""),
            ("API_BIND_ADDR", "127.0.0.1:8080"),
            ("LOG_LEVEL", "loud"),
        ]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let bind_addr = env.required::<SocketAddr>("API_BIND_ADDR");
        let internal_bind_addr = env.required::<SocketAddr>("API_INTERNAL_BIND_ADDR");
        assert!(bind_addr.is_some());

        let config = app_env
            .zip(log_level)
            .zip(bind_addr)
            .zip(internal_bind_addr);
        let error = env.finish(config).unwrap_err();

        assert_eq!(
            summary(&error),
            [
                ("APP_ENV", Missing),
                ("LOG_LEVEL", Invalid),
                ("API_INTERNAL_BIND_ADDR", Missing),
            ]
        );
    }

    #[test]
    fn each_problem_is_one_line_with_the_name_and_what_is_expected() {
        let mut env = Environment::from_pairs([("LOG_LEVEL", "loud"), ("API_BIND_ADDR", "x")]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let bind_addr = env.required::<SocketAddr>("API_BIND_ADDR");
        let error = env
            .finish(app_env.zip(log_level).zip(bind_addr))
            .unwrap_err();

        let lines = [
            "APP_ENV is missing: expected one of local, staging, production",
            "LOG_LEVEL is not valid: expected one of trace, debug, info, warn, error",
            "API_BIND_ADDR is not valid: expected a socket address such as 127.0.0.1:8080",
        ];
        assert_eq!(error.to_string(), lines.join("\n"));
    }

    #[test]
    fn no_problem_repeats_the_value() {
        let secret = "hunter2-secret";
        let mut env = Environment::from_pairs([
            ("APP_ENV", secret),
            ("LOG_LEVEL", secret),
            ("API_BIND_ADDR", secret),
        ]);
        let app_env = env.required::<AppEnv>("APP_ENV");
        let log_level = env.required::<LogLevel>("LOG_LEVEL");
        let bind_addr = env.required::<SocketAddr>("API_BIND_ADDR");
        let error = env
            .finish(app_env.zip(log_level).zip(bind_addr))
            .unwrap_err();

        assert_eq!(error.problems().len(), 3);
        for text in [error.to_string(), format!("{error:?}")] {
            assert!(!text.contains("hunter2"), "{text}");
        }
    }

    #[test]
    fn a_name_given_twice_keeps_its_last_value() {
        let mut env = Environment::from_pairs([("APP_ENV", "local"), ("APP_ENV", "production")]);
        assert_eq!(env.required::<AppEnv>("APP_ENV"), Some(AppEnv::Production));
    }

    #[test]
    fn the_process_environment_is_read_by_name() {
        let mut env = Environment::from_process();
        // `PATH` is set wherever the tests run, and it is no socket address.
        let path = env.required::<SocketAddr>("PATH");
        let unset = env.required::<SocketAddr>("BARRAS_BOOTSTRAP_TEST_NEVER_SET");
        let error = env.finish(path.zip(unset)).unwrap_err();
        assert_eq!(
            summary(&error),
            [
                ("PATH", Invalid),
                ("BARRAS_BOOTSTRAP_TEST_NEVER_SET", Missing)
            ]
        );
    }

    #[test]
    fn a_configuration_left_incomplete_without_a_problem_is_still_an_error() {
        let env = Environment::from_pairs([("APP_ENV", "local")]);
        let error = env.finish(None::<AppEnv>).unwrap_err();
        assert!(error.problems().is_empty());
        assert!(!error.to_string().is_empty());
    }
}
