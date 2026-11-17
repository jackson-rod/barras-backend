//! Why the database could not be used, in sentences of a fixed set.

use std::fmt;
use std::io;
use std::time::Duration;

use sqlx::error::DatabaseError;

/// How long getting a connection from a pool may take, opening it included (spec §6.1).
pub(crate) const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);

/// Why the database could not be used: one sentence of a fixed set, with the SQLSTATE when the
/// server gave one. Never sqlx's text: it can carry the server's message, which names the user
/// when a password is refused, and one SCRAM failure prints a character of the password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseUnavailable {
    /// No connection within the pool's limit: the database cannot be reached, or every
    /// connection is busy.
    NoConnectionInTime,
    /// The pool was closed.
    PoolClosed,
    /// The server answered with an error, with its SQLSTATE when it gave one.
    Refused { sqlstate: Option<String> },
    /// The connection could not be opened, or broke, in this way.
    ConnectionFailed(io::ErrorKind),
    /// TLS could not be set up with the server.
    TlsFailed,
    /// The server sent what the driver could not read.
    ProtocolViolated,
    /// Any other failure of the driver.
    DriverFailed,
}

impl From<&sqlx::Error> for DatabaseUnavailable {
    fn from(error: &sqlx::Error) -> Self {
        match error {
            sqlx::Error::PoolTimedOut => Self::NoConnectionInTime,
            sqlx::Error::PoolClosed => Self::PoolClosed,
            sqlx::Error::Database(refusal) => Self::Refused {
                sqlstate: sqlstate_of(refusal.as_ref()),
            },
            sqlx::Error::Io(failure) => Self::ConnectionFailed(failure.kind()),
            sqlx::Error::Tls(_) => Self::TlsFailed,
            sqlx::Error::Protocol(_) => Self::ProtocolViolated,
            _ => Self::DriverFailed,
        }
    }
}

/// The SQLSTATE of the server's `refusal`: five upper-case letters and digits. A code from the
/// server is one; anything else is left out, so that a sentence holds nothing the server wrote.
pub(crate) fn sqlstate_of(refusal: &dyn DatabaseError) -> Option<String> {
    refusal
        .code()
        .filter(|code| {
            code.len() == 5
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
        .map(|code| code.into_owned())
}

impl fmt::Display for DatabaseUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConnectionInTime => {
                let limit = ACQUIRE_TIMEOUT.as_secs();
                write!(f, "no connection within {limit} seconds")
            }
            Self::PoolClosed => f.write_str("the pool is closed"),
            Self::Refused {
                sqlstate: Some(sqlstate),
            } => write!(f, "the server refused it with SQLSTATE {sqlstate}"),
            Self::Refused { sqlstate: None } => f.write_str("the server refused it"),
            // The description of the kind is one of a fixed set, unlike the error's own text,
            // which the operating system writes.
            Self::ConnectionFailed(kind) => write!(f, "the connection failed: {kind}"),
            Self::TlsFailed => f.write_str("TLS could not be set up"),
            Self::ProtocolViolated => f.write_str("the server answered outside the protocol"),
            Self::DriverFailed => f.write_str("the driver failed"),
        }
    }
}

impl std::error::Error for DatabaseUnavailable {}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;

    use sqlx::error::{DatabaseError, ErrorKind};

    use super::*;

    /// What the server's message may hold, and no sentence may show.
    const USER: &str = "secret-user";
    const PASSWORD: &str = "hunter2-secret";

    /// A server error such as a refused password, whose message names the user.
    #[derive(Debug)]
    struct ServerError {
        code: Option<&'static str>,
    }

    impl ServerError {
        fn error(code: Option<&'static str>) -> sqlx::Error {
            sqlx::Error::Database(Box::new(Self { code }))
        }
    }

    impl fmt::Display for ServerError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.message())
        }
    }

    impl StdError for ServerError {}

    impl DatabaseError for ServerError {
        fn message(&self) -> &str {
            "password authentication failed for user \"secret-user\" (hunter2-secret)"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            self.code.map(Cow::Borrowed)
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    /// The driver's own text for errors that are not the server's: it may quote anything.
    fn secret_text() -> String {
        format!("user {USER}, password {PASSWORD}")
    }

    fn every_kind_of_error() -> Vec<sqlx::Error> {
        vec![
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            ServerError::error(Some("28P01")),
            ServerError::error(None),
            ServerError::error(Some("hunter2-secret")),
            sqlx::Error::Io(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                secret_text(),
            )),
            sqlx::Error::Tls(secret_text().into()),
            sqlx::Error::Protocol(secret_text()),
            sqlx::Error::Configuration(secret_text().into()),
            sqlx::Error::InvalidArgument(secret_text()),
            sqlx::Error::WorkerCrashed,
        ]
    }

    #[test]
    fn each_error_of_the_driver_maps_to_its_kind() {
        let kinds: Vec<DatabaseUnavailable> = every_kind_of_error()
            .iter()
            .map(DatabaseUnavailable::from)
            .collect();

        assert_eq!(
            kinds,
            [
                DatabaseUnavailable::NoConnectionInTime,
                DatabaseUnavailable::PoolClosed,
                DatabaseUnavailable::Refused {
                    sqlstate: Some("28P01".to_owned())
                },
                DatabaseUnavailable::Refused { sqlstate: None },
                // Not five upper-case letters and digits, so not a SQLSTATE: left out.
                DatabaseUnavailable::Refused { sqlstate: None },
                DatabaseUnavailable::ConnectionFailed(io::ErrorKind::ConnectionRefused),
                DatabaseUnavailable::TlsFailed,
                DatabaseUnavailable::ProtocolViolated,
                DatabaseUnavailable::DriverFailed,
                DatabaseUnavailable::DriverFailed,
                DatabaseUnavailable::DriverFailed,
            ]
        );
    }

    #[test]
    fn each_kind_is_a_fixed_sentence() {
        for (kind, sentence) in [
            (
                DatabaseUnavailable::NoConnectionInTime,
                "no connection within 2 seconds",
            ),
            (DatabaseUnavailable::PoolClosed, "the pool is closed"),
            (
                DatabaseUnavailable::Refused {
                    sqlstate: Some("28P01".to_owned()),
                },
                "the server refused it with SQLSTATE 28P01",
            ),
            (
                DatabaseUnavailable::Refused { sqlstate: None },
                "the server refused it",
            ),
            (
                DatabaseUnavailable::ConnectionFailed(io::ErrorKind::ConnectionReset),
                "the connection failed: connection reset",
            ),
            (DatabaseUnavailable::TlsFailed, "TLS could not be set up"),
            (
                DatabaseUnavailable::ProtocolViolated,
                "the server answered outside the protocol",
            ),
            (DatabaseUnavailable::DriverFailed, "the driver failed"),
        ] {
            assert_eq!(kind.to_string(), sentence);
        }
    }

    #[test]
    fn no_sentence_shows_what_the_driver_or_the_server_said() {
        for error in every_kind_of_error() {
            let kind = DatabaseUnavailable::from(&error);

            for shown in [kind.to_string(), format!("{kind:?}")] {
                assert!(!shown.contains(USER), "{shown}");
                assert!(!shown.contains(PASSWORD), "{shown}");
            }
        }
    }
}
