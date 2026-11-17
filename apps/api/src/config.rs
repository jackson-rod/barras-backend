//! The configuration of `barras-api`, read from the environment once, at start (spec §8.1).

use std::net::SocketAddr;
use std::num::NonZeroU32;

use barras_adapter_postgres::DatabaseUrl;
use barras_bootstrap::{AppEnv, ConfigError, Environment, FromEnvValue, LogLevel, Secret};

const APP_ENV: &str = "APP_ENV";
const LOG_LEVEL: &str = "LOG_LEVEL";
const API_BIND_ADDR: &str = "API_BIND_ADDR";
const API_INTERNAL_BIND_ADDR: &str = "API_INTERNAL_BIND_ADDR";
const API_DATABASE_URL: &str = "API_DATABASE_URL";
const API_DATABASE_READ_URL: &str = "API_DATABASE_READ_URL";
const API_DATABASE_POOL_SIZE: &str = "API_DATABASE_POOL_SIZE";

/// Every setting of the binary. Each one is required: a default in code would be a second
/// place where the configuration lives. Its `Debug` shows neither database URL, since each
/// holds a password.
#[derive(Debug)]
pub struct ApiConfig {
    /// Decides the format of the logs.
    pub env: AppEnv,
    pub log_level: LogLevel,
    /// The public API, under `/api/v1`.
    pub bind_addr: SocketAddr,
    /// The detail of the health probes, and the metrics.
    pub internal_bind_addr: SocketAddr,
    /// The primary database: the writes, and the reads that follow a write.
    pub database_url: Secret<DatabaseUrlValue>,
    /// The read database: a replica when there is one, the primary's URL when there is not, and
    /// then one pool serves both.
    pub database_read_url: Secret<DatabaseUrlValue>,
    /// The most connections each pool opens.
    pub database_pool_size: NonZeroU32,
}

impl ApiConfig {
    /// Every variable the configuration reads, in the order it reads them.
    pub const VARIABLES: &[&str] = &[
        APP_ENV,
        LOG_LEVEL,
        API_BIND_ADDR,
        API_INTERNAL_BIND_ADDR,
        API_DATABASE_URL,
        API_DATABASE_READ_URL,
        API_DATABASE_POOL_SIZE,
    ];

    /// The configuration `env` holds, or every variable that is missing or not valid, at once.
    pub fn from_env(mut env: Environment) -> Result<Self, ConfigError> {
        let app_env = env.required(APP_ENV);
        let log_level = env.required(LOG_LEVEL);
        let bind_addr = env.required(API_BIND_ADDR);
        let internal_bind_addr = env.required(API_INTERNAL_BIND_ADDR);
        let database_url = env.required(API_DATABASE_URL);
        let database_read_url = env.required(API_DATABASE_READ_URL);
        let database_pool_size = env.required(API_DATABASE_POOL_SIZE);
        let config = (|| {
            Some(Self {
                env: app_env?,
                log_level: log_level?,
                bind_addr: bind_addr?,
                internal_bind_addr: internal_bind_addr?,
                database_url: database_url?,
                database_read_url: database_read_url?,
                database_pool_size: database_pool_size?,
            })
        })();
        env.finish(config)
    }
}

/// A database URL read from a variable, as the postgres adapter's rule accepts it (spec §5): it
/// names its user, password, host, port and database, so that no part of it comes from anywhere
/// else. The rule is the adapter's and the trait the start-up crate's, so a type of the binary
/// joins the two.
///
/// It keeps the adapter's URL, never the text it was read from, and has no `Debug`: both hold the
/// password.
pub struct DatabaseUrlValue(DatabaseUrl);

impl DatabaseUrlValue {
    /// The URL, for the adapter.
    pub fn get(&self) -> &DatabaseUrl {
        &self.0
    }
}

impl FromEnvValue for DatabaseUrlValue {
    const EXPECTED: &'static str = "a postgres:// URL that names its user, password, host, port \
                                    and database, and no query string";

    fn from_env_value(raw: &str) -> Option<Self> {
        // The trait answers only whether the text is a value, so the adapter's reason goes with
        // the text: the problem line names the variable and what a value must look like.
        DatabaseUrl::parse(raw).ok().map(Self)
    }
}

#[cfg(test)]
mod tests {
    use barras_bootstrap::ProblemKind;

    use super::*;

    /// What the tests keep secret; no output may show it.
    const PASSWORD: &str = "hunter2-secret";

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

    fn database_url(host: &str) -> String {
        format!("postgres://barras:{PASSWORD}@{host}/barras")
    }

    /// A whole configuration, every value valid, with `changed` over it.
    fn environment(changed: &[(&'static str, &str)]) -> Environment {
        let primary = database_url("127.0.0.1:5433");
        let replica = database_url("replica.example:5432");
        let valid = [
            ("APP_ENV", "staging"),
            ("LOG_LEVEL", "warn"),
            ("API_BIND_ADDR", "127.0.0.1:8080"),
            ("API_INTERNAL_BIND_ADDR", "[::1]:9080"),
            ("API_DATABASE_URL", primary.as_str()),
            ("API_DATABASE_READ_URL", replica.as_str()),
            ("API_DATABASE_POOL_SIZE", "12"),
        ];
        let pairs: Vec<(&str, String)> = valid
            .iter()
            .chain(changed)
            .map(|&(name, value)| (name, value.to_owned()))
            .collect();
        Environment::from_pairs(pairs)
    }

    #[test]
    fn the_seven_variables_are_read_into_their_types() {
        let config = ApiConfig::from_env(environment(&[])).unwrap();

        assert_eq!(config.env, AppEnv::Staging);
        assert_eq!(config.log_level, LogLevel::Warn);
        assert_eq!(config.bind_addr, address("127.0.0.1:8080"));
        assert_eq!(config.internal_bind_addr, address("[::1]:9080"));
        assert_eq!(config.database_pool_size.get(), 12);
    }

    #[test]
    fn the_configuration_shows_neither_database_url() {
        let config = ApiConfig::from_env(environment(&[])).unwrap();

        let shown = format!("{config:?}");

        assert!(shown.contains("database_url: [redacted]"), "{shown}");
        assert!(shown.contains("database_read_url: [redacted]"), "{shown}");
        assert!(!shown.contains(PASSWORD), "{shown}");
        assert!(!shown.contains("127.0.0.1:5433"), "{shown}");
    }

    #[test]
    fn an_empty_environment_names_every_variable_in_the_order_read() {
        let env = Environment::from_pairs(std::iter::empty::<(&str, &str)>());

        let error = ApiConfig::from_env(env).unwrap_err();

        let every_variable: Vec<_> = ApiConfig::VARIABLES
            .iter()
            .map(|&name| (name, ProblemKind::Missing))
            .collect();
        assert_eq!(summary(&error), every_variable);
        assert_eq!(
            ApiConfig::VARIABLES,
            [
                "APP_ENV",
                "LOG_LEVEL",
                "API_BIND_ADDR",
                "API_INTERNAL_BIND_ADDR",
                "API_DATABASE_URL",
                "API_DATABASE_READ_URL",
                "API_DATABASE_POOL_SIZE",
            ]
        );
    }

    #[test]
    fn every_value_that_is_not_valid_is_named_and_none_is_repeated() {
        let no_port = database_url("127.0.0.1");
        let env = environment(&[
            ("APP_ENV", "local"),
            ("LOG_LEVEL", PASSWORD),
            ("API_INTERNAL_BIND_ADDR", PASSWORD),
            ("API_DATABASE_READ_URL", &no_port),
            ("API_DATABASE_POOL_SIZE", "0"),
        ]);

        let error = ApiConfig::from_env(env).unwrap_err();

        assert_eq!(
            summary(&error),
            [
                ("LOG_LEVEL", ProblemKind::Invalid),
                ("API_INTERNAL_BIND_ADDR", ProblemKind::Invalid),
                ("API_DATABASE_READ_URL", ProblemKind::Invalid),
                ("API_DATABASE_POOL_SIZE", ProblemKind::Invalid),
            ]
        );
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains("hunter2"), "{shown}");
        }
    }

    #[test]
    fn a_database_url_the_adapter_refuses_is_named_with_what_is_expected_and_never_shown() {
        // The adapter's rule refuses each one for a reason of its own; the line is the same.
        for refused in [
            database_url("127.0.0.1"),
            format!("postgres://barras@127.0.0.1:5433/barras?password={PASSWORD}"),
            format!("{}?sslmode=disable", database_url("127.0.0.1:5433")),
            database_url("127.0.0.1:5433").replace("postgres://", "mysql://"),
            database_url("[::1]:5433"),
            format!("not a URL {PASSWORD}"),
        ] {
            let error =
                ApiConfig::from_env(environment(&[("API_DATABASE_URL", &refused)])).unwrap_err();

            assert_eq!(
                error.to_string(),
                "API_DATABASE_URL is not valid: expected a postgres:// URL that names its user, \
                 password, host, port and database, and no query string",
                "{refused}"
            );
            assert!(!format!("{error:?}").contains("hunter2"), "{refused}");
        }
    }

    #[test]
    fn a_database_url_is_any_url_the_adapter_accepts() {
        for accepted in [
            database_url("127.0.0.1:5433"),
            database_url("localhost:5432").replace("postgres://", "postgresql://"),
            database_url("db.example.com:5432"),
            database_url("%2Fvar%2Frun%2Fpostgresql:5432"),
        ] {
            assert!(
                DatabaseUrlValue::from_env_value(&accepted).is_some(),
                "{accepted}"
            );
        }
    }

    #[test]
    fn a_pool_size_is_a_whole_number_from_one() {
        for (size, read) in [("1", Some(1)), ("10", Some(10)), ("0", None), ("-1", None)] {
            let config = ApiConfig::from_env(environment(&[("API_DATABASE_POOL_SIZE", size)]));

            let size_read = config.ok().map(|config| config.database_pool_size.get());

            assert_eq!(size_read, read, "{size}");
        }
    }
}
