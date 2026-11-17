//! A configuration value that holds a credential, kept out of every output.

use std::fmt;

use crate::FromEnvValue;

/// What a secret prints wherever it is formatted.
const REDACTED: &str = "[redacted]";

/// A value that holds a credential, such as a database URL with its password. `Debug` and
/// `Display` print `[redacted]` in its place, so that a configuration that derives `Debug`, a log
/// event or an error message shows nothing of it. It implements no `Serialize` either, so no
/// serializer writes it out.
///
/// It is read from the environment as its value is, and only [`expose`](Self::expose) reaches
/// the value, so every place that uses it is one search away.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// The value, for the code that hands it on, such as to the database driver. Whatever
    /// formats what this returns shows the credential.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Read, and refused, as its value is. A refused one stays out of the problem line, as every
/// value does: [`Environment`](crate::Environment) names the variable, never its text.
impl<T: FromEnvValue> FromEnvValue for Secret<T> {
    const EXPECTED: &'static str = T::EXPECTED;

    fn from_env_value(raw: &str) -> Option<Self> {
        T::from_env_value(raw).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::num::NonZeroU32;

    use super::*;
    use crate::Environment;

    /// What the tests keep secret; no output may show it.
    const PASSWORD: &str = "hunter2-secret";

    fn password() -> Secret<String> {
        Secret(PASSWORD.to_owned())
    }

    /// Tells whether `T` implements `serde::Serialize`, as the compiler sees it: the inherent
    /// constant applies only where its bound holds, and the trait's default stands in elsewhere.
    struct SerializeProbe<T>(PhantomData<T>);

    trait NoSerialize {
        const IMPLEMENTED: bool = false;
    }

    impl<T> NoSerialize for SerializeProbe<T> {}

    impl<T: serde::Serialize> SerializeProbe<T> {
        const IMPLEMENTED: bool = true;
    }

    #[test]
    fn debug_and_display_print_redacted_instead_of_the_value() {
        let secret = password();

        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(format!("{secret:#?}"), "[redacted]");
        assert_eq!(secret.to_string(), "[redacted]");
    }

    #[test]
    fn a_secret_inside_a_derived_debug_is_redacted() {
        #[allow(dead_code)] // only formatted, and a derived Debug counts as no read
        #[derive(Debug)]
        struct Credentials {
            user: String,
            password: Secret<String>,
        }
        let credentials = Credentials {
            user: "barras".to_owned(),
            password: password(),
        };

        assert_eq!(
            format!("{credentials:?}"),
            r#"Credentials { user: "barras", password: [redacted] }"#
        );
        let pretty = format!("{credentials:#?}");
        assert!(!pretty.contains("hunter2"), "{pretty}");
        assert!(pretty.contains("password: [redacted]"), "{pretty}");
    }

    #[test]
    fn expose_reaches_the_value() {
        assert_eq!(password().expose(), PASSWORD);
    }

    #[test]
    fn a_secret_is_cloned_and_compared_by_its_value() {
        // `Eq` has no method to call, so a bound checks it.
        fn clone_and_eq<T: Clone + Eq>() {}
        clone_and_eq::<Secret<String>>();
        let secret = password();

        let copy = secret.clone();

        assert_eq!(copy.expose(), PASSWORD);
        assert_eq!(copy, secret);
        assert_ne!(copy, Secret("another".to_owned()));
    }

    #[test]
    fn a_secret_cannot_be_serialized() {
        // The compiler decides both, so the tests fail to build when either is false. The first
        // shows that the probe sees an implementation where there is one.
        const { assert!(SerializeProbe::<String>::IMPLEMENTED) };
        const { assert!(!SerializeProbe::<Secret<String>>::IMPLEMENTED) };
    }

    #[test]
    fn a_secret_is_read_from_the_environment_as_its_value_is() {
        let url = "postgres://barras:hunter2-secret@127.0.0.1:5433/barras";
        let mut env = Environment::from_pairs([("API_DATABASE_URL", url)]);

        let read = env.required::<Secret<String>>("API_DATABASE_URL");

        assert_eq!(format!("{read:?}"), "Some([redacted])");
        let secret = env.finish(read).unwrap();
        assert_eq!(secret.expose(), url);
    }

    #[test]
    fn a_secret_expects_what_its_value_expects() {
        assert_eq!(Secret::<NonZeroU32>::EXPECTED, NonZeroU32::EXPECTED);
        assert_eq!(Secret::<String>::EXPECTED, String::EXPECTED);
    }

    #[test]
    fn a_secret_its_type_refuses_is_named_but_never_shown() {
        let mut env = Environment::from_pairs([("ACCESS_PIN", PASSWORD)]);
        let pin = env.required::<Secret<NonZeroU32>>("ACCESS_PIN");
        assert_eq!(format!("{pin:?}"), "None");

        let error = env.finish(pin).unwrap_err();

        assert_eq!(
            error.to_string(),
            "ACCESS_PIN is not valid: expected a whole number from 1 to 4294967295"
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
    }

    #[cfg(unix)]
    #[test]
    fn a_secret_that_is_not_unicode_is_named_but_never_shown() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let not_unicode = OsString::from_vec(b"hunter2-secret\xff".to_vec());
        let mut env = Environment::from_pairs([("DATABASE_PASSWORD", not_unicode)]);
        let password = env.required::<Secret<String>>("DATABASE_PASSWORD");

        let error = env.finish(password).unwrap_err();

        assert_eq!(
            error.to_string(),
            "DATABASE_PASSWORD is not valid: expected any text"
        );
        let debug = format!("{error:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
    }
}
