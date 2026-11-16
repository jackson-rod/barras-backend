//! The extractors a handler takes its input with (spec §6.1, §6.2): the value reaches the
//! handler only once it was read whole and passed every rule its type declares. Anything else
//! answers an error envelope whose messages never repeat a value that was sent, which may be
//! a password.

mod validated_json;
mod validated_query;

use garde::Report;
use serde_path_to_error::Path;

use crate::error::{ApiError, FieldErrors};

pub use validated_json::ValidatedJson;
pub use validated_query::ValidatedQuery;

const IS_REQUIRED: &str = "is required";
const IS_NOT_VALID: &str = "is not valid";

/// 422 with every rule `report` holds broken, each field with all its messages. garde's
/// messages name what a rule expects, never the value it refused.
fn broken_rules(report: &Report) -> ApiError {
    let mut fields = FieldErrors::new();
    for (path, error) in report.iter() {
        let messages = fields.entry(path.to_string()).or_default();
        messages.push(error.message().to_owned());
    }
    ApiError::validation(fields)
}

/// The answer to a value serde refused at `path`, given serde's `message` without any
/// position: 422 with the field and a message of our own, since serde's quotes the value. A
/// failure at the root that names no field, such as a key given twice, leaves no field to
/// answer for: the input as a whole cannot be read, and the answer is 400 with `unreadable`.
fn refused_value(path: &Path, message: &str, unreadable: &'static str) -> ApiError {
    let at_root = path.iter().next().is_none();
    let (field, text) = if let Some(name) = missing_field(message) {
        // serde reports a missing field at its parent's path.
        let field = if at_root {
            name.to_owned()
        } else {
            format!("{path}.{name}")
        };
        (field, IS_REQUIRED.to_owned())
    } else if at_root {
        return ApiError::malformed_request(unreadable);
    } else {
        // serde writes the refused value first and what it expected after the last
        // `, expected `, so a value that holds those words itself cannot reach the answer.
        let text = match message.rsplit_once(", expected ") {
            Some((_, expected)) => format!("expected {expected}"),
            None => IS_NOT_VALID.to_owned(),
        };
        (path.to_string(), text)
    };
    ApiError::validation(FieldErrors::from([(field, vec![text])]))
}

/// The name in serde's ``missing field `name` ``.
fn missing_field(message: &str) -> Option<&str> {
    message.strip_prefix("missing field `")?.strip_suffix('`')
}
