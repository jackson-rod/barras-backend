//! The configuration of `barras-ingest`, read from the environment once, at start (spec §8.1).

use std::net::SocketAddr;

use barras_bootstrap::{AppEnv, ConfigError, Environment, LogLevel};

const APP_ENV: &str = "APP_ENV";
const LOG_LEVEL: &str = "LOG_LEVEL";
const INGEST_INTERNAL_BIND_ADDR: &str = "INGEST_INTERNAL_BIND_ADDR";

/// Every setting of the binary. Each one is required: a default in code would be a second
/// place where the configuration lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestConfig {
    /// Decides the format of the logs.
    pub env: AppEnv,
    pub log_level: LogLevel,
    /// The detail of the health probes, and the metrics.
    pub internal_bind_addr: SocketAddr,
}

impl IngestConfig {
    /// Every variable the configuration reads, in the order it reads them.
    pub const VARIABLES: &[&str] = &[APP_ENV, LOG_LEVEL, INGEST_INTERNAL_BIND_ADDR];

    /// The configuration `env` holds, or every variable that is missing or not valid, at once.
    pub fn from_env(mut env: Environment) -> Result<Self, ConfigError> {
        let app_env = env.required(APP_ENV);
        let log_level = env.required(LOG_LEVEL);
        let internal_bind_addr = env.required(INGEST_INTERNAL_BIND_ADDR);
        let config = (|| {
            Some(Self {
                env: app_env?,
                log_level: log_level?,
                internal_bind_addr: internal_bind_addr?,
            })
        })();
        env.finish(config)
    }
}

#[cfg(test)]
mod tests {
    use barras_bootstrap::ProblemKind;

    use super::*;

    fn summary(error: &ConfigError) -> Vec<(&'static str, ProblemKind)> {
        error
            .problems()
            .iter()
            .map(|problem| (problem.name(), problem.kind()))
            .collect()
    }

    fn address(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_three_variables_are_read_into_their_types() {
        let env = Environment::from_pairs([
            ("APP_ENV", "production"),
            ("LOG_LEVEL", "debug"),
            ("INGEST_INTERNAL_BIND_ADDR", "[::1]:9081"),
        ]);

        let config = IngestConfig::from_env(env).unwrap();

        assert_eq!(
            config,
            IngestConfig {
                env: AppEnv::Production,
                log_level: LogLevel::Debug,
                internal_bind_addr: address("[::1]:9081"),
            }
        );
    }

    #[test]
    fn an_empty_environment_names_every_variable_in_the_order_read() {
        let env = Environment::from_pairs(std::iter::empty::<(&str, &str)>());

        let error = IngestConfig::from_env(env).unwrap_err();

        let every_variable: Vec<_> = IngestConfig::VARIABLES
            .iter()
            .map(|&name| (name, ProblemKind::Missing))
            .collect();
        assert_eq!(summary(&error), every_variable);
        assert_eq!(
            IngestConfig::VARIABLES,
            ["APP_ENV", "LOG_LEVEL", "INGEST_INTERNAL_BIND_ADDR"]
        );
    }

    #[test]
    fn the_variables_of_barras_api_are_neither_needed_nor_read() {
        let env = Environment::from_pairs([
            ("APP_ENV", "local"),
            ("LOG_LEVEL", "info"),
            ("INGEST_INTERNAL_BIND_ADDR", "127.0.0.1:9081"),
            // The API's internal port: a binary that read it by mistake would listen there.
            ("API_INTERNAL_BIND_ADDR", "127.0.0.1:9080"),
        ]);

        let config = IngestConfig::from_env(env).unwrap();

        assert_eq!(config.internal_bind_addr, address("127.0.0.1:9081"));
    }

    #[test]
    fn every_value_that_is_not_valid_is_named_and_none_is_repeated() {
        let env = Environment::from_pairs([
            ("APP_ENV", "hunter2-secret"),
            ("LOG_LEVEL", "info"),
            ("INGEST_INTERNAL_BIND_ADDR", "hunter2-secret"),
        ]);

        let error = IngestConfig::from_env(env).unwrap_err();

        assert_eq!(
            summary(&error),
            [
                ("APP_ENV", ProblemKind::Invalid),
                ("INGEST_INTERNAL_BIND_ADDR", ProblemKind::Invalid)
            ]
        );
        assert!(!error.to_string().contains("hunter2"), "{error}");
    }
}
