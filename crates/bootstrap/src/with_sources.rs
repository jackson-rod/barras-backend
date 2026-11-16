//! An error written out whole, causes included, on one line.

use std::error::Error;

/// The error's text, then that of each of its sources in turn, joined by `": "`: at most
/// sixteen, since an error whose sources led back to itself would never end.
///
/// What a binary reports when it stops on a failure: its errors name the step that failed and
/// leave the cause to their source, so the text alone would not say why.
pub fn with_sources(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&error| error.source())
        .take(16)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::io;

    use super::*;

    /// An error that names a step of the work and keeps what made it fail as its source.
    #[derive(Debug)]
    struct Step {
        what: &'static str,
        cause: Option<Box<dyn Error>>,
    }

    impl fmt::Display for Step {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.what)
        }
    }

    impl Error for Step {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.cause.as_deref()
        }
    }

    /// An error that is its own source.
    #[derive(Debug)]
    struct Loop;

    impl fmt::Display for Loop {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("round")
        }
    }

    impl Error for Loop {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self)
        }
    }

    #[test]
    fn an_error_without_a_source_is_its_own_text() {
        let error = Step {
            what: "the metrics could not be registered",
            cause: None,
        };

        assert_eq!(with_sources(&error), "the metrics could not be registered");
    }

    #[test]
    fn an_error_is_followed_by_each_of_its_sources_in_turn() {
        let bind = Step {
            what: "the internal listener could not bind 127.0.0.1:9081",
            cause: Some(Box::new(io::Error::from(io::ErrorKind::AddrInUse))),
        };
        let error = Step {
            what: "serving stopped",
            cause: Some(Box::new(bind)),
        };

        assert_eq!(
            with_sources(&error),
            "serving stopped: the internal listener could not bind 127.0.0.1:9081: address in use"
        );
    }

    #[test]
    fn sources_that_lead_back_to_the_error_end_after_sixteen_texts() {
        assert_eq!(with_sources(&Loop), vec!["round"; 16].join(": "));
    }
}
