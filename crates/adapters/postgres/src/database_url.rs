//! The rule a database URL follows: it names every part the driver connects with, so that no
//! part comes from anywhere else.

use std::fmt;
use std::time::Duration;

use log::LevelFilter;
use sqlx::ConnectOptions;
use sqlx::postgres::PgConnectOptions;
use url::{Host, Url};

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
    url: Url,
    options: PgConnectOptions,
}

impl DatabaseUrl {
    /// The URL `text`, or the first problem it has: its scheme is `postgres` or `postgresql`;
    /// it names its host, port, user, password and database; its host is one host, no list, and
    /// no IPv6 address in brackets, which sqlx 0.9 cannot connect to (a name such as `localhost`
    /// reaches `::1`); it has no query string; and the driver can read it.
    pub fn parse(text: &str) -> Result<Self, UrlProblem> {
        let url = Url::parse(text).map_err(|_| UrlProblem::NotAUrl)?;
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return Err(UrlProblem::NotPostgres);
        }
        match url.host() {
            None => return Err(UrlProblem::NoHost),
            Some(Host::Domain("")) => return Err(UrlProblem::NoHost),
            Some(Host::Ipv6(_)) => return Err(UrlProblem::BracketedIpv6),
            // Encoded too: the driver decodes the host when it names a socket's folder.
            Some(Host::Domain(host))
                if host.contains(',') || host.contains("%2C") || host.contains("%2c") =>
            {
                return Err(UrlProblem::CommaInHost);
            }
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
        // An empty one too: no setting of a connection comes from the URL (plan, refinement 10).
        if url.query().is_some() {
            return Err(UrlProblem::QueryString);
        }
        let options = PgConnectOptions::from_url(&url).map_err(|_| UrlProblem::Unreadable)?;
        Ok(Self {
            url,
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

    /// Whether `other` is the same URL, as the `url` crate writes both out.
    pub(crate) fn is_same_as(&self, other: &Self) -> bool {
        self.url == other.url
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
    /// Its host holds a comma, written or percent-encoded, as a list of hosts does: sqlx reads
    /// the list as one name, and never connects.
    CommaInHost,
    /// It names no port.
    NoPort,
    /// It names no user.
    NoUser,
    /// It names no password, or an empty one.
    NoPassword,
    /// It names no database.
    NoDatabase,
    /// It has a query string. Through one, sqlx would take the host, the port, the user, the
    /// password or the database from somewhere the rule does not read, let `options` or
    /// `application_name` contest the settings the binary gives each connection, and log the
    /// value of a key it does not know. BAR-12, which brings TLS, decides what TLS needs here.
    QueryString,
    /// It follows the rule, but the driver cannot read it, as with a password that decodes to
    /// something other than UTF-8.
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
            Self::CommaInHost => "has a comma in its host: it must name one host, never a list",
            Self::NoPort => "names no port",
            Self::NoUser => "names no user",
            Self::NoPassword => "names no password",
            Self::NoDatabase => "names no database",
            Self::QueryString => "has a query string, and must have none",
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
            format!("postgres://barras:{PASSWORD}@localhost:5432/barras"),
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
        ] {
            assert_eq!(problem_of(url), expected, "{url:?}");
        }
    }

    #[test]
    fn a_url_with_a_query_string_is_refused_whatever_it_holds() {
        // The keys that replace one of the URL's own parts; the settings the binary sets itself;
        // one the driver does not know, whose value it would log; and no key at all.
        for query in [
            "host=/tmp",
            "hostaddr=127.0.0.1",
            "port=5432",
            "user=u",
            "password=p",
            "dbname=db",
            "options=-c%20statement_timeout%3D0",
            "application_name=other",
            "sslmode=disable",
            "statement-cache-capacity=0",
            "unknown=x",
            "",
            "&",
            "=x",
        ] {
            let url = format!("postgres://u:p@127.0.0.1:5432/db?{query}");

            assert_eq!(problem_of(&url), UrlProblem::QueryString, "{url}");
        }
    }

    #[test]
    fn a_host_with_a_comma_is_refused_written_or_encoded() {
        // The driver reads a list of hosts as one name, and decodes a socket's folder.
        for url in [
            "postgres://u:p@127.0.0.1,10.0.0.1:5432/db",
            "postgres://u:p@db1.example,db2.example:5432/db",
            "postgres://u:p@db1.example,:5432/db",
            "postgres://u:p@db1.example%2Cdb2.example:5432/db",
            "postgres://u:p@%2Ftmp%2Fa%2cb:5432/db",
        ] {
            assert_eq!(problem_of(url), UrlProblem::CommaInHost, "{url}");
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
            (
                UrlProblem::CommaInHost,
                "the database URL has a comma in its host: it must name one host, never a list",
            ),
            (UrlProblem::NoPort, "the database URL names no port"),
            (UrlProblem::NoUser, "the database URL names no user"),
            (UrlProblem::NoPassword, "the database URL names no password"),
            (UrlProblem::NoDatabase, "the database URL names no database"),
            (
                UrlProblem::QueryString,
                "the database URL has a query string, and must have none",
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
    fn no_problem_shows_the_password_or_any_other_part_of_the_url() {
        for url in [
            format!("mysql://barras:{PASSWORD}@127.0.0.1:5432/barras"),
            format!("postgres://barras:{PASSWORD}@127.0.0.1/barras"),
            format!("postgres://barras:{PASSWORD}@127.0.0.1:5432/barras?password={PASSWORD}"),
            format!("postgres://barras:{PASSWORD}@127.0.0.1:5432/barras?{PASSWORD}=1"),
            format!(
                "postgres://barras:{PASSWORD}@127.0.0.1:5432/barras?application_name={PASSWORD}"
            ),
            format!("postgres://barras:{PASSWORD}@127.0.0.1,{PASSWORD}:5432/barras"),
            format!("postgres://barras:{PASSWORD}%FF@127.0.0.1:5432/barras"),
        ] {
            let problem = problem_of(&url);

            for shown in [problem.to_string(), format!("{problem:?}")] {
                assert!(!shown.contains(PASSWORD), "{shown}");
                assert!(!shown.contains("127.0.0.1"), "{shown}");
                assert!(!shown.contains("barras:"), "{shown}");
            }
        }
    }
}
