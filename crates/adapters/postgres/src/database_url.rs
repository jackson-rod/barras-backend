//! The rule a database URL follows: it names every part the driver connects with, so that no
//! part comes from anywhere else.

use std::fmt;
use std::time::Duration;

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::postgres::PgConnectOptions;
use url::{Host, Url};

/// The query keys through which sqlx lets a URL replace one of its own parts, or name one the
/// URL left out.
const PARTS_IN_QUERY: [&str; 6] = ["host", "hostaddr", "port", "user", "password", "dbname"];

/// A statement that runs longer is logged at `warn`, with its SQL and its duration (spec §6.2).
/// sqlx's own threshold is 1 second.
const SLOW_STATEMENT: Duration = Duration::from_millis(250);

/// A database URL that names its host, port, user, password and database itself.
///
/// sqlx fills whatever a URL leaves out from the `PG*` environment variables and `~/.pgpass`,
/// which would be a second place where the configuration lives (ADR-0016), and its getters show
/// a part filled in that way as if the URL had named it. So the rule reads the URL itself, with
/// the crate sqlx reads it with.
///
/// It holds the password, so it has no `Debug`: the driver's options and the URL both print it.
pub struct DatabaseUrl {
    options: PgConnectOptions,
}

impl DatabaseUrl {
    /// The URL `text`, or the first problem it has: its scheme is `postgres` or `postgresql`;
    /// it names its host, port, user, password and database; its query string names none of
    /// them; its host is no IPv6 address in brackets, which sqlx 0.9 cannot connect to (a name
    /// such as `localhost` reaches `::1`); and the driver can read it.
    pub fn parse(text: &str) -> Result<Self, UrlProblem> {
        let url = Url::parse(text).map_err(|_| UrlProblem::NotAUrl)?;
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(UrlProblem::NotPostgres);
        }
        match url.host() {
            None => return Err(UrlProblem::NoHost),
            Some(Host::Domain("")) => return Err(UrlProblem::NoHost),
            Some(Host::Ipv6(_)) => return Err(UrlProblem::BracketedIpv6),
            Some(_) => {}
        }
        if url.port().is_none() {
            return Err(UrlProblem::NoPort);
        }
        if url.username().is_empty() {
            return Err(UrlProblem::NoUser);
        }
        if url.password().is_none_or(str::is_empty) {
            return Err(UrlProblem::NoPassword);
        }
        if url.path().trim_start_matches('/').is_empty() {
            return Err(UrlProblem::NoDatabase);
        }
        if url
            .query_pairs()
            .any(|(key, _)| PARTS_IN_QUERY.contains(&key.as_ref()))
        {
            return Err(UrlProblem::PartInQuery);
        }
        let options = PgConnectOptions::from_url(&url).map_err(|_| UrlProblem::Unreadable)?;
        Ok(Self {
            // sqlx's event for each statement has no message, which every log line must have,
            // and shows the SQL at `debug`; a slow statement's event has one (plan, refinement 3).
            options: options
                .log_statements(LevelFilter::Off)
                .log_slow_statements(LevelFilter::Warn, SLOW_STATEMENT),
        })
    }

    /// The options every connection of this crate starts from.
    pub(crate) fn connect_options(&self) -> PgConnectOptions {
        self.options.clone()
    }
}

/// Why a text is not a database URL: one sentence of a fixed set, which never quotes the text,
/// since a URL holds its password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlProblem {
    /// It is not a URL at all.
    NotAUrl,
    /// Its scheme is neither `postgres` nor `postgresql`.
    NotPostgres,
    /// It names no host.
    NoHost,
    /// Its host is an IPv6 address in brackets, which sqlx 0.9 cannot connect to.
    BracketedIpv6,
    /// It names no port.
    NoPort,
    /// It names no user.
    NoUser,
    /// It names no password, or an empty one.
    NoPassword,
    /// It names no database.
    NoDatabase,
    /// Its query string names the host, the port, the user, the password or the database.
    PartInQuery,
    /// It follows the rule, but the driver cannot read it, as with a password that decodes to
    /// something other than UTF-8, or an `sslmode` it does not know.
    Unreadable,
}

impl fmt::Display for UrlProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            Self::NotAUrl => "is not a URL",
            Self::NotPostgres => "does not start with postgres:// or postgresql://",
            Self::NoHost => "names no host",
            Self::BracketedIpv6 => {
                "names its host by an IPv6 address in brackets, which the driver cannot connect to"
            }
            Self::NoPort => "names no port",
            Self::NoUser => "names no user",
            Self::NoPassword => "names no password",
            Self::NoDatabase => "names no database",
            Self::PartInQuery => {
                "names its host, port, user, password or database in its query string"
            }
            Self::Unreadable => "cannot be read by the database driver",
        };
        write!(f, "the database URL {what}")
    }
}

impl std::error::Error for UrlProblem {}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the tests keep secret; no problem may show it.
    const PASSWORD: &str = "hunter2-secret";

    fn problem_of(url: &str) -> UrlProblem {
        match DatabaseUrl::parse(url) {
            Ok(_) => panic!("{url} was accepted"),
            Err(problem) => problem,
        }
    }

    #[test]
    fn a_url_that_names_every_part_is_accepted() {
        for url in [
            format!("postgres://barras:{PASSWORD}@127.0.0.1:5433/barras"),
            format!("postgresql://barras:{PASSWORD}@127.0.0.1:5433/barras"),
            format!("postgres://barras:{PASSWORD}@localhost:5432/barras?sslmode=disable"),
            format!("postgres://barras:{PASSWORD}@db.example.com:5432/barras"),
            // A socket folder as the host, percent-encoded as the driver reads it.
            format!("postgres://barras:{PASSWORD}@%2Ftmp%2Fsockets:5432/barras"),
            // Percent-encoded user and password.
            "postgres://bar%40ras:hunter%202@127.0.0.1:5432/barras".to_owned(),
        ] {
            assert!(DatabaseUrl::parse(&url).is_ok(), "{url}");
        }
    }

    #[test]
    fn a_url_that_leaves_a_part_out_is_refused_naming_the_part() {
        for (url, expected) in [
            ("not a url", UrlProblem::NotAUrl),
            ("", UrlProblem::NotAUrl),
            ("mysql://u:p@127.0.0.1:5432/db", UrlProblem::NotPostgres),
            ("http://u:p@127.0.0.1:5432/db", UrlProblem::NotPostgres),
            ("postgres:///db", UrlProblem::NoHost),
            ("postgres:db", UrlProblem::NoHost),
            ("postgres://u:p@[::1]:5432/db", UrlProblem::BracketedIpv6),
            ("postgres://u:p@127.0.0.1/db", UrlProblem::NoPort),
            ("postgres://127.0.0.1:5432/db", UrlProblem::NoUser),
            ("postgres://:p@127.0.0.1:5432/db", UrlProblem::NoUser),
            ("postgres://u@127.0.0.1:5432/db", UrlProblem::NoPassword),
            ("postgres://u:@127.0.0.1:5432/db", UrlProblem::NoPassword),
            ("postgres://u:p@127.0.0.1:5432", UrlProblem::NoDatabase),
            ("postgres://u:p@127.0.0.1:5432/", UrlProblem::NoDatabase),
            // Valid by the rule, but not to the driver.
            (
                "postgres://u:p%FF@127.0.0.1:5432/db",
                UrlProblem::Unreadable,
            ),
            (
                "postgres://u:p@127.0.0.1:5432/db?sslmode=sometimes",
                UrlProblem::Unreadable,
            ),
        ] {
            assert_eq!(problem_of(url), expected, "{url:?}");
        }
    }

    #[test]
    fn a_query_string_that_names_a_part_is_refused_whatever_the_part() {
        // Each one would replace what the URL's own part says, or say it where the rule does
        // not read it.
        for key in ["host", "hostaddr", "port", "user", "password", "dbname"] {
            let url = format!("postgres://u:p@127.0.0.1:5432/db?sslmode=disable&{key}=x");

            assert_eq!(problem_of(&url), UrlProblem::PartInQuery, "{url}");
        }
    }

    #[test]
    fn each_problem_is_a_fixed_sentence() {
        for (problem, sentence) in [
            (UrlProblem::NotAUrl, "the database URL is not a URL"),
            (
                UrlProblem::NotPostgres,
                "the database URL does not start with postgres:// or postgresql://",
            ),
            (UrlProblem::NoHost, "the database URL names no host"),
            (
                UrlProblem::BracketedIpv6,
                "the database URL names its host by an IPv6 address in brackets, which the \
                 driver cannot connect to",
            ),
            (UrlProblem::NoPort, "the database URL names no port"),
            (UrlProblem::NoUser, "the database URL names no user"),
            (UrlProblem::NoPassword, "the database URL names no password"),
            (UrlProblem::NoDatabase, "the database URL names no database"),
            (
                UrlProblem::PartInQuery,
                "the database URL names its host, port, user, password or database in its \
                 query string",
            ),
            (
                UrlProblem::Unreadable,
                "the database URL cannot be read by the database driver",
            ),
        ] {
            assert_eq!(problem.to_string(), sentence);
        }
    }

    #[test]
    fn no_problem_shows_the_password() {
        for url in [
            format!("mysql://barras:{PASSWORD}@127.0.0.1:5432/barras"),
            format!("postgres://barras:{PASSWORD}@127.0.0.1/barras"),
            format!("postgres://barras:{PASSWORD}@127.0.0.1:5432/barras?password={PASSWORD}"),
            format!("postgres://barras:{PASSWORD}%FF@127.0.0.1:5432/barras"),
        ] {
            let problem = problem_of(&url);

            assert!(!problem.to_string().contains(PASSWORD), "{problem}");
            assert!(!format!("{problem:?}").contains(PASSWORD), "{problem:?}");
        }
    }
}
