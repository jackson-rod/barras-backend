//! Reading an env file such as `.env.example`, the way the binaries' tests check it.

use std::fs;
use std::path::Path;

/// What a value may not hold: a shell, or a dotenv loader, would read it otherwise than as
/// written (a quote, an expansion, an escape, a command, a comment).
const READ_OTHERWISE: &[char] = &['"', '\'', '$', '\\', '`', '#'];

/// The variables of the env file at `path`, in the order written: one `NAME=value` per line,
/// the value taken as written. Blank lines and lines that start with `#` are skipped. Any other
/// line fails the test, naming the file and the line, so the file keeps the one shape that every
/// reader of it reads the same way.
pub fn env_file(path: impl AsRef<Path>) -> Vec<(String, String)> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    variables(&text, &path.display().to_string())
}

/// The variables of `text`, the content of the env file `origin`.
fn variables(text: &str, origin: &str) -> Vec<(String, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(index, line)| {
            variable(line).unwrap_or_else(|| {
                panic!(
                    "{origin}:{}: not a NAME=value line whose value every reader takes as written",
                    index + 1
                )
            })
        })
        .collect()
}

/// The name and the value of `line` when it is `NAME=value`: a name of upper-case letters,
/// digits and underscores that does not start with a digit, and a value of visible ASCII
/// characters, none of them read otherwise.
fn variable(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once('=')?;
    let name_is_valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_uppercase() || first == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    let value_is_plain = value
        .chars()
        .all(|c| c.is_ascii_graphic() && !READ_OTHERWISE.contains(&c));
    (name_is_valid && value_is_plain).then(|| (name.to_owned(), value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|&(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    #[test]
    fn the_variables_come_in_order_without_comments_or_blank_lines() {
        let text = "# Where the binary runs.\nAPP_ENV=local\n\n  \n# The public API.\n\
                    API_BIND_ADDR=127.0.0.1:8080\nIPV6_ADDR=[::1]:9080\nEMPTY=\n";

        let variables = variables(text, "example.env");

        assert_eq!(
            variables,
            owned(&[
                ("APP_ENV", "local"),
                ("API_BIND_ADDR", "127.0.0.1:8080"),
                ("IPV6_ADDR", "[::1]:9080"),
                ("EMPTY", ""),
            ])
        );
    }

    #[test]
    fn a_line_that_every_reader_takes_as_written_is_a_variable() {
        assert_eq!(
            variable("LOG_LEVEL=info"),
            Some(("LOG_LEVEL".to_owned(), "info".to_owned()))
        );
        assert_eq!(
            variable("_PRIVATE_2=a=b"),
            Some(("_PRIVATE_2".to_owned(), "a=b".to_owned()))
        );
    }

    #[test]
    fn a_line_that_a_shell_or_a_dotenv_loader_would_read_otherwise_is_not() {
        for line in [
            "NO_EQUALS_SIGN",
            "=value",
            "lower_case=value",
            "2_STARTS_WITH_A_DIGIT=value",
            " LEADING_SPACE=value",
            "export LOG_LEVEL=info",
            "SPACE_BEFORE =value",
            "TWO_WORDS=two words",
            "TRAILING_SPACE=value ",
            "DOUBLE_QUOTED=\"value\"",
            "SINGLE_QUOTED='value'",
            "EXPANDED=$HOME",
            "ESCAPED=back\\slash",
            "COMMAND=`date`",
            "INLINE_COMMENT=value#comment",
            "TAB=a\tb",
        ] {
            assert_eq!(variable(line), None, "{line:?}");
        }
    }

    #[test]
    #[should_panic(expected = "example.env:3: ")]
    fn a_line_that_is_not_a_variable_fails_the_test_naming_the_file_and_the_line() {
        variables(
            "# A comment.\nAPP_ENV=local\nexport LOG_LEVEL=info\n",
            "example.env",
        );
    }

    #[test]
    fn env_file_reads_the_variables_of_the_file_at_its_path() {
        let path = std::env::temp_dir().join(format!("barras-testkit-{}.env", std::process::id()));
        fs::write(&path, "# Where the binary runs.\nAPP_ENV=local\n").unwrap();

        let variables = env_file(&path);

        fs::remove_file(&path).unwrap();
        assert_eq!(variables, owned(&[("APP_ENV", "local")]));
    }

    #[test]
    #[should_panic(expected = "cannot read")]
    fn a_file_that_cannot_be_read_fails_the_test() {
        env_file("/nonexistent/barras-testkit.env");
    }
}
