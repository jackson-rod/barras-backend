//! A JSON body, read whole and checked against its type's rules before the handler sees it.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use garde::Validate;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Value;
use serde_path_to_error::{Path, Segment};

use super::{broken_rules, refused_value};
use crate::error::ApiError;

const NOT_JSON: &str = "Request body is not valid JSON";
const NOT_AN_OBJECT: &str = "Request body is not a JSON object";
const UNREADABLE: &str = "Request body could not be read";
const UNFIT: &str = "Request body cannot be read as the object this route takes";

/// The most arrays and objects serde_json reads one inside another; it refuses one more.
const DEEPEST: usize = 127;

/// The request body, a JSON object, read into `T` and checked against every rule `T` declares.
/// A handler takes its body with it, never with axum's `Json`, whose rejections are not
/// envelopes.
///
/// A body not sent as JSON (`application/json` or `application/<x>+json`, parameters aside)
/// answers 415 `UNSUPPORTED_MEDIA_TYPE`; one over the size limit, 413 `PAYLOAD_TOO_LARGE`; one
/// that is not UTF-8 (RFC 8259 §8.1), not JSON or not a JSON object, 400 `MALFORMED_REQUEST`;
/// and one whose fields are missing, have the wrong type or break a rule, 422
/// `VALIDATION_FAILED` with each such field. An array of the wrong length for the tuple it
/// fills is a field of the wrong type too.
///
/// The typed read checks what it reads: a member the DTO does not have is skipped once its
/// grammar is checked, so a lone surrogate escape (`"\ud800"`), nesting past serde_json's limit
/// or a number past `f64` (`1e400`) in it is accepted. In a field the DTO reads, such a string
/// or number answers 422 at that field, and such nesting 400.
///
/// A field an object of the DTO does not accept (`#[serde(deny_unknown_fields)]`) is reported
/// at that object, never under its own name, which is the caller's text; at the root, where
/// no field is left to report it at, the body answers 400 `MALFORMED_REQUEST`. Inside an
/// internally tagged enum (`#[serde(tag = …)]`), serde reads the object whole before it knows
/// the variant, so the path stops at the enum and the field is reported one level up: at the
/// object that holds the enum, or with 400 when that object is the body itself.
#[derive(Debug)]
pub struct ValidatedJson<T>(pub T);

impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned + Validate,
    T::Context: Default,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        if !is_json(request.headers()) {
            return Err(ApiError::unsupported_media_type());
        }
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                // The rejection type is non-exhaustive: only its status tells the size limit
                // from a body that broke off.
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::payload_too_large()
                } else {
                    ApiError::malformed_request(UNREADABLE)
                }
            })?;
        // The typed read skips the bytes of a member the DTO does not have unchecked.
        let text =
            std::str::from_utf8(&bytes).map_err(|_| ApiError::malformed_request(NOT_JSON))?;
        // A derived struct also reads a JSON array, by position.
        if first_token(text) != Some(b'{') {
            return Err(ApiError::malformed_request(NOT_AN_OBJECT));
        }
        let mut deserializer = serde_json::Deserializer::from_str(text);
        let value: T = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            // serde stops at the first value it refuses, which may come before the place where
            // the body stops being JSON; a body that is not JSON is malformed, whatever it holds.
            // In a body that is JSON, the fault is a value's, even one serde_json words as
            // syntax: the elements of an array past those a tuple takes.
            if is_json_text(text) {
                let message = without_position(error.inner());
                refused_value(error.path(), &message, UNFIT, |path, name| {
                    bears_out(text, path, name)
                })
            } else {
                ApiError::malformed_request(NOT_JSON)
            }
        })?;
        deserializer
            .end()
            .map_err(|_| ApiError::malformed_request(NOT_JSON))?;
        value.validate().map_err(|report| broken_rules(&report))?;
        Ok(Self(value))
    }
}

/// `application/json` or `application/<x>+json`, in any case, its parameters aside.
pub(crate) fn is_json(headers: &HeaderMap) -> bool {
    let Some(Ok(content_type)) = headers.get(CONTENT_TYPE).map(|value| value.to_str()) else {
        return false;
    };
    let essence = content_type.split(';').next().unwrap_or_default().trim();
    let Some((kind, subtype)) = essence.split_once('/') else {
        return false;
    };
    let has_json_suffix = subtype
        .rsplit_once('+')
        .is_some_and(|(name, suffix)| !name.is_empty() && suffix.eq_ignore_ascii_case("json"));
    kind.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json") || has_json_suffix)
}

/// Whether `text` is one JSON value nested no deeper than serde_json reads, decided without
/// keeping any of it: a refused body of 1 MiB costs a pass over it, not a tree of it.
fn is_json_text(text: &str) -> bool {
    serde_json::from_str::<IgnoredAny>(text).is_ok() && !nests_too_deep(text)
}

/// Whether `json`, which is grammatical JSON, nests arrays and objects deeper than `DEEPEST`.
/// It counts without recursion, so no body can exhaust the stack.
fn nests_too_deep(json: &str) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0_usize, false, false);
    for byte in json.bytes() {
        match byte {
            _ if escaped => escaped = false,
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'[' | b'{' if !in_string => {
                depth += 1;
                if depth > DEEPEST {
                    return true;
                }
            }
            b']' | b'}' if !in_string => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Whether the body `text` bears out that the object at `path` lacks the field `name`, which
/// serde reports missing in words a type of the DTO's own can write too. The object must lack
/// it at any depth, and no string the caller sent may be `name`: a type can quote only what it
/// read, and a secret is a value. Only the value at `path` is searched for the key, so that a
/// field other elements of a list have is still reported missing from the one that lacks it.
///
/// This alone builds a tree of the body, a copy that the size limit of 1 MiB bounds. A body
/// that is JSON yet has no tree, for a lone surrogate or a number past `f64` in a member the
/// DTO does not read, bears nothing out.
fn bears_out(text: &str, path: &Path, name: &str) -> bool {
    let Ok(sent) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    value_at(&sent, path).is_some_and(|at| at.is_object() && !mentions(at, name, true))
        && !mentions(&sent, name, false)
}

/// The value `path` leads to in `sent`.
fn value_at<'a>(sent: &'a Value, path: &Path) -> Option<&'a Value> {
    let mut value = sent;
    for segment in path.iter() {
        value = match segment {
            // An enum's content is the member named after its variant.
            Segment::Map { key } | Segment::Enum { variant: key } => value.as_object()?.get(key)?,
            Segment::Seq { index } => value.as_array()?.get(*index)?,
            Segment::Unknown => return None,
        };
    }
    Some(value)
}

/// Whether `name` is, anywhere in `value`, a string or, when `keys`, the key of a member.
fn mentions(value: &Value, name: &str, keys: bool) -> bool {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::String(text) if text == name => return true,
            Value::Array(items) => pending.extend(items),
            Value::Object(members) => {
                if keys && members.contains_key(name) {
                    return true;
                }
                pending.extend(members.values());
            }
            _ => {}
        }
    }
    false
}

/// The first byte of `text` that is not JSON white space.
fn first_token(text: &str) -> Option<u8> {
    text.bytes()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
}

/// serde_json's text for `error` without the position it appends.
fn without_position(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let position = format!(" at line {} column {}", error.line(), error.column());
    text.strip_suffix(&position).unwrap_or(&text).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nesting_refused_is_the_nesting_serde_json_refuses() {
        for depth in [1, 126, 127, 128, 129, 400] {
            let arrays = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            let inner = depth - 1;
            let objects = format!(r#"{}{{"a":1}}{}"#, "[".repeat(inner), "]".repeat(inner));
            for json in [arrays, objects] {
                let refused = serde_json::from_str::<Value>(&json).is_err();

                assert_eq!(nests_too_deep(&json), refused, "{depth}");
            }
        }
    }

    #[test]
    fn brackets_inside_a_string_nest_nothing() {
        let brackets = format!(r#"\"{}\\{}"#, "[".repeat(200), "{".repeat(200));
        let json = format!(r#"{{"a": "{brackets}", "b": "\\\\", "c": []}}"#);

        assert!(serde_json::from_str::<Value>(&json).is_ok(), "{json}");
        assert!(!nests_too_deep(&json));
    }
}
