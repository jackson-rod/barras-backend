//! The extractors a handler takes its input with (spec §6.1, §6.2): the value reaches the
//! handler only once it was read whole and passed every rule its type declares. Anything else
//! answers an error envelope whose messages never repeat a value that was sent, which may be
//! a password.
//!
//! A message for a field comes from a fixed set, never from the text of an error: `is
//! required`; `expected <type>`, for the few primitive types serde names in fixed words, and
//! `has the wrong type` for any other; `is not one of the accepted values`; `has a field it
//! does not accept`; `is not valid`. serde quotes the refused value in its messages, and the
//! error of a DTO type's own `Deserialize` or `TryFrom` may say anything, including words that
//! read as serde's. That error must still never quote its input, as the `Display` of a
//! module's error must not, since a log or a later change could repeat it.
//!
//! Every key of `error.fields` is a path of the names the caller wrote, and two rules of the
//! DTOs keep it so:
//!
//! - Every DTO, body and query alike, takes `#[serde(rename_all = "camelCase")]` and never
//!   renames a field one by one. garde names a field by its Rust name, which this module
//!   converts the way serde does, a raw identifier's `r#` dropped; garde 0.23 reads
//!   `#[garde(rename(…))]` but never applies it, so a field renamed by hand would be reported
//!   under its Rust name in camelCase.
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

/// The message of a field that was not sent.
pub(crate) const IS_REQUIRED: &str = "is required";
/// The message of a value of a type serde names other than in fixed words.
pub(crate) const HAS_THE_WRONG_TYPE: &str = "has the wrong type";
/// The message of a value outside an enum; the OpenAPI schema lists the accepted ones.
pub(crate) const IS_NOT_ACCEPTED: &str = "is not one of the accepted values";
/// The message of an object that holds a field it does not take.
pub(crate) const HAS_UNKNOWN_FIELD: &str = "has a field it does not accept";
/// The message of any other value refused.
pub(crate) const IS_NOT_VALID: &str = "is not valid";

/// serde's messages for a value of the wrong type, the refused value quoted before the last
/// `, expected `.
const WRONG_TYPE_FORMS: [&str; 3] = ["invalid type: ", "invalid value: ", "invalid length "];

/// What serde says it expected of a primitive type: the only text of its messages an answer
/// repeats, matched whole, so that nothing the caller wrote can reach the answer.
const PRIMITIVES: [&str; 17] = [
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "f32",
    "f64",
    "a string",
    "a boolean",
    "a sequence",
    "a map",
    "a unit value",
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
/// position: 422 at the field, with a message from the fixed set. A failure that leaves no
/// field to answer for, at the root, such as a key given twice, means the input as a whole
/// cannot be read: 400 with `unreadable`.
///
/// `lacks(path, name)` says whether what was sent holds, at `path`, an object without the key
/// `name`.
fn refused_value(
    path: &Path,
    message: &str,
    unreadable: &'static str,
    lacks: impl FnOnce(&Path, &str) -> bool,
) -> ApiError {
    let length = path.iter().len();
    let (field, text) = match missing_field(message) {
        // serde reports a missing field at its parent's path. A type of the DTO's own can write
        // the same words, so they are believed only when what was sent bears them out, and
        // the name, which then becomes a key, is one a field could have.
        Some(name) if is_identifier(name) && lacks(path, name) => {
            let field = match written(path, length) {
                Some(parent) => format!("{parent}.{name}"),
                None => name.to_owned(),
            };
            (Some(field), IS_REQUIRED.to_owned())
        }
        _ if message.starts_with("unknown field `") => {
            // serde reports an unknown field at a path that ends with its name, which is the
            // caller's text: the answer names the object that refused it.
            let parent = written(path, length.saturating_sub(1));
            (parent, HAS_UNKNOWN_FIELD.to_owned())
        }
        _ => (written(path, length), value_message(message)),
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

/// A name a field of a DTO can have: an ASCII letter or `_`, then ASCII letters, digits or
/// `_`.
fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// The message for a value serde refused, from the fixed set. serde writes what it expected
/// after the last `, expected `; the answer repeats it only when it is one of `PRIMITIVES`
/// whole, and then in the words of that list.
fn value_message(message: &str) -> String {
    if message.starts_with("unknown variant ") {
        return IS_NOT_ACCEPTED.to_owned();
    }
    if !WRONG_TYPE_FORMS
        .iter()
        .any(|form| message.starts_with(form))
    {
        return IS_NOT_VALID.to_owned();
    }
    let expected = message
        .rsplit_once(", expected ")
        .map(|(_, expected)| expected);
    match PRIMITIVES
        .iter()
        .find(|primitive| Some(**primitive) == expected)
    {
        Some(primitive) => format!("expected {primitive}"),
        None => HAS_THE_WRONG_TYPE.to_owned(),
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

/// `path` with each field named as the body names it: garde writes `display_name[1]` and
/// `r#type` where serde, under `rename_all = "camelCase"`, writes `displayName[1]` and `type`.
fn json_path(path: &garde::Path) -> String {
    let path = path.to_string();
    let segments: Vec<String> = path
        .split('.')
        .map(|segment| {
            let (name, indices) = segment.split_at(segment.find('[').unwrap_or(segment.len()));
            let name = name.strip_prefix("r#").unwrap_or(name);
            camel_case(name) + indices
        })
        .collect();
    segments.join(".")
}

/// `name` in camelCase, as serde's `rename_all = "camelCase"` writes a field: each letter
/// after an underscore in upper case, the underscores dropped, the first letter in lower case.
pub(crate) fn camel_case(name: &str) -> String {
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
            // serde names a raw identifier without its `r#`.
            (garde::Path::new("r#type"), "type"),
            (
                garde::Path::new("r#type").join("valid_from"),
                "type.validFrom",
            ),
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
    fn a_refused_value_is_answered_from_a_fixed_set_of_messages() {
        for (message, answered) in [
            ("invalid type: string \"x\", expected u32", "expected u32"),
            ("invalid value: integer `-1`, expected u32", "expected u32"),
            (
                "invalid type: integer `7`, expected a string",
                "expected a string",
            ),
            (
                "invalid type: string \"x\", expected a sequence",
                "expected a sequence",
            ),
            (
                "invalid type: integer `1`, expected a unit value",
                "expected a unit value",
            ),
            ("invalid type: string \"x\", expected i128", "expected i128"),
            // Named after the code, not the contract.
            (
                "invalid type: string \"x\", expected struct Address",
                HAS_THE_WRONG_TYPE,
            ),
            (
                "invalid length 1, expected a tuple of size 2",
                HAS_THE_WRONG_TYPE,
            ),
            // serde's words written by the caller, ahead of the error of a type of its own.
            (
                "invalid type: x, expected hunter2 is not a valid code",
                HAS_THE_WRONG_TYPE,
            ),
            (
                "invalid type: x, expected u8 is not a valid code",
                HAS_THE_WRONG_TYPE,
            ),
            ("unknown variant `x`, expected `a` or `b`", IS_NOT_ACCEPTED),
            (
                "unknown variant `x, expected hunter2`, there are no variants",
                IS_NOT_ACCEPTED,
            ),
            (
                "not a nickname, expected hunter2 to be lower-case",
                IS_NOT_VALID,
            ),
            ("missing field `hunter2`", IS_NOT_VALID),
            ("invalid digit found in string", IS_NOT_VALID),
            ("trailing characters", IS_NOT_VALID),
        ] {
            assert_eq!(value_message(message), answered, "{message}");
        }
    }

    #[test]
    fn only_a_plain_identifier_can_name_a_missing_field() {
        for (name, plain) in [
            ("email", true),
            ("displayName", true),
            ("_x1", true),
            ("", false),
            ("1x", false),
            ("a b", false),
            ("a.b", false),
            ("hunter2-secret", false),
            ("é", false),
        ] {
            assert_eq!(is_identifier(name), plain, "{name:?}");
        }
    }
}
