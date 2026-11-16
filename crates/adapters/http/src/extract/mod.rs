//! The extractors a handler takes its input with (spec §6.1, §6.2): the value reaches the
//! handler only once it was read whole and passed every rule its type declares. Anything else
//! answers an error envelope whose messages never repeat a value that was sent, which may be
//! a password.
//!
//! Every key of `error.fields` is a path of the names the caller wrote, and two rules of the
//! DTOs keep it so:
//!
//! - A field's name in a body or a query string is its Rust name in camelCase (spec §4.2),
//!   written with `#[serde(rename_all = "camelCase")]` and never renamed one by one: garde
//!   names a field by its Rust name, which this module converts the way serde does.
//! - A DTO holds no map keyed by what the caller sends, such as a `HashMap<String, _>`: serde
//!   and garde write a map's key in a path as they write a field's name, so a refused value
//!   inside the map would come back keyed by the caller's text. A list of objects, each with
//!   its key as a field, takes its place.

mod validated_json;
mod validated_query;

use garde::Report;
use serde_path_to_error::{Path, Segment};

use crate::error::{ApiError, FieldErrors};

pub use validated_json::ValidatedJson;
#[cfg(any(test, feature = "testing"))]
pub(crate) use validated_json::is_json;
pub use validated_query::ValidatedQuery;

/// The message of a field serde reports missing, by which the test client knows one.
pub(crate) const IS_REQUIRED: &str = "is required";
const IS_NOT_VALID: &str = "is not valid";
const HAS_UNKNOWN_FIELD: &str = "has a field it does not accept";

/// serde's own messages that end with what it expected, the refused value before it:
/// `invalid type: …, expected …` and its kin. Any other text, the error of a type of the
/// DTO's own included, may quote the value anywhere.
const SERDE_EXPECTATIONS: [&str; 4] = [
    "invalid type: ",
    "invalid value: ",
    "invalid length ",
    "unknown variant ",
];

/// 422 with every rule `report` holds broken, each field with all its messages. garde's
/// messages name what a rule expects, never the value it refused.
fn broken_rules(report: &Report) -> ApiError {
    let mut fields = FieldErrors::new();
    for (path, error) in report.iter() {
        let messages = fields.entry(json_path(path)).or_default();
        messages.push(error.message().to_owned());
    }
    ApiError::validation(fields)
}

/// The answer to a value serde refused at `path`, given serde's `message` without any
/// position: 422 at the field, with a message of our own, since serde's quotes the value. A
/// failure that leaves no field to answer for, at the root, such as a key given twice, means
/// the input as a whole cannot be read: 400 with `unreadable`.
fn refused_value(path: &Path, message: &str, unreadable: &'static str) -> ApiError {
    let length = path.iter().len();
    let (field, text) = if let Some(name) = missing_field(message) {
        // serde reports a missing field at its parent's path.
        let field = match written(path, length) {
            Some(parent) => format!("{parent}.{name}"),
            None => name.to_owned(),
        };
        (Some(field), IS_REQUIRED.to_owned())
    } else if message.starts_with("unknown field `") {
        // serde reports an unknown field at a path that ends with its name, which is the
        // caller's text: the answer names the object that refused it.
        let parent = written(path, length.saturating_sub(1));
        (parent, HAS_UNKNOWN_FIELD.to_owned())
    } else {
        (written(path, length), value_message(message))
    };
    match field {
        Some(field) => ApiError::validation(FieldErrors::from([(field, vec![text])])),
        None => ApiError::malformed_request(unreadable),
    }
}

/// The name in serde's ``missing field `name` ``.
fn missing_field(message: &str) -> Option<&str> {
    message.strip_prefix("missing field `")?.strip_suffix('`')
}

/// What serde expected, from one of its own messages; for any other, `is not valid`. serde
/// writes the refused value before the last `, expected `, so a value that holds those words
/// itself cannot reach the answer. The one such message that names no expectation, for an
/// enum without variants, ends with the value.
fn value_message(message: &str) -> String {
    let from_serde = SERDE_EXPECTATIONS
        .iter()
        .any(|form| message.starts_with(form))
        && !message.ends_with(", there are no variants");
    match message.rsplit_once(", expected ") {
        Some((_, expected)) if from_serde => format!("expected {expected}"),
        _ => IS_NOT_VALID.to_owned(),
    }
}

/// The first `count` segments of `path`, written as serde_path_to_error writes a path
/// (`address.zip`, `tags[1]`), or `None` for no segment: the root.
fn written(path: &Path, count: usize) -> Option<String> {
    let mut written = String::new();
    for segment in path.iter().take(count) {
        if !written.is_empty() && !matches!(segment, Segment::Seq { .. }) {
            written.push('.');
        }
        written.push_str(&segment.to_string());
    }
    (!written.is_empty()).then_some(written)
}

/// `path` with each field named as the body names it: garde writes `display_name[1]` where
/// serde, under `rename_all = "camelCase"`, writes `displayName[1]`.
fn json_path(path: &garde::Path) -> String {
    let path = path.to_string();
    let segments: Vec<String> = path
        .split('.')
        .map(|segment| {
            let (name, indices) = segment.split_at(segment.find('[').unwrap_or(segment.len()));
            camel_case(name) + indices
        })
        .collect();
    segments.join(".")
}

/// `name` in camelCase, as serde's `rename_all = "camelCase"` writes a field: each letter
/// after an underscore in upper case, the underscores dropped, the first letter in lower case.
fn camel_case(name: &str) -> String {
    let mut pascal = String::with_capacity(name.len());
    let mut capitalize = true;
    for character in name.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            pascal.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            pascal.push(character);
        }
    }
    if let Some(first) = pascal.get_mut(..1) {
        first.make_ascii_lowercase();
    }
    pascal
}

#[cfg(test)]
mod tests {
    use garde::error::NoKey;

    use super::*;

    #[test]
    fn a_garde_path_is_written_with_the_json_name_of_each_field() {
        for (path, written) in [
            (garde::Path::new("display_name"), "displayName"),
            (garde::Path::new("email"), "email"),
            (garde::Path::new("type_"), "type"),
            (
                garde::Path::new("favorite_tags").join(1_usize),
                "favoriteTags[1]",
            ),
            (
                garde::Path::new("grid").join(0_usize).join(2_usize),
                "grid[0][2]",
            ),
            // A list validated at the root.
            (
                garde::Path::new(0_usize).join("unit_price"),
                "[0].unitPrice",
            ),
            (
                garde::Path::new("strict_named")
                    .join(NoKey::default())
                    .join("max_level"),
                "strictNamed.maxLevel",
            ),
            (
                garde::Path::new("line_items")
                    .join(3_usize)
                    .join("unit_price"),
                "lineItems[3].unitPrice",
            ),
        ] {
            assert_eq!(json_path(&path), written);
        }
    }

    #[test]
    fn only_serdes_own_words_say_what_it_expected() {
        for (message, answered) in [
            ("invalid type: string \"x\", expected u32", "expected u32"),
            ("invalid value: integer `-1`, expected u32", "expected u32"),
            (
                "invalid length 1, expected a tuple of size 2",
                "expected a tuple of size 2",
            ),
            (
                "unknown variant `x`, expected `a` or `b`",
                "expected `a` or `b`",
            ),
            (
                "unknown variant `x`, expected `y`, expected `a` or `b`",
                "expected `a` or `b`",
            ),
            (
                "unknown variant `x, expected hunter2`, there are no variants",
                IS_NOT_VALID,
            ),
            (
                "not a nickname, expected hunter2 to be lower-case",
                IS_NOT_VALID,
            ),
            ("invalid digit found in string", IS_NOT_VALID),
            ("trailing characters", IS_NOT_VALID),
        ] {
            assert_eq!(value_message(message), answered, "{message}");
        }
    }
}
