//! An in-process client for the public app, the route guard of spec §9.3, a guard on the error
//! answers of a route, and a guard on the keys of a validation failure.

use std::collections::{BTreeMap, BTreeSet};

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::response::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use serde_json::Value;
use tower::ServiceExt;
use utoipa::openapi::OpenApi;
use utoipa::openapi::extensions::Extensions;

use crate::app::HttpApp;
use crate::error::ApiError;
use crate::extract::{camel_case, is_json};
use crate::internal::InternalApp;
use crate::middleware::MatchedRoute;
use crate::openapi::{X_COMMON_ERROR_CODES, X_ERROR_CODES, operations};

/// The statuses of the codes of `x-common-error-codes`, which any route can answer, so that no
/// operation documents them: 400, 404, 405, 406, 413, 415 and 500. Sorted.
const COMMON_STATUSES: [u16; 7] = [400, 404, 405, 406, 413, 415, 500];

/// Sends requests to an app in-process, through the whole stack of middleware, and fails the
/// test when a route the OpenAPI document does not have served one: so any test that reaches
/// a route registered around `ApiRouter::route` fails, naming it.
///
/// It also fails the test when a route the document has answers an error envelope whose
/// `error.code` neither the operation's `x-error-codes` nor the document's
/// `x-common-error-codes` lists, or whose status the operation documents neither as itself,
/// nor as its range such as `4XX`, nor with `default`, unless a common code has that status:
/// the document would otherwise hide from the app an answer it must handle.
///
/// And it fails the test when a validation failure has a key in `error.fields` with a name
/// that holds `_`, a Rust name where every JSON name is camelCase (spec §4.2), or, answered to
/// a JSON body, a key that names neither a member of that body, nor a parameter of the query
/// string, nor a field missing from an object of the body: the app finds the field to flag by
/// that key, so a key spelled otherwise flags nothing.
pub struct TestClient {
    router: Router,
    /// What the OpenAPI document says of the app; `None` for the internal app, whose routes
    /// the document does not have, by design.
    document: Option<Documented>,
}

impl TestClient {
    pub fn new(app: &HttpApp) -> Self {
        Self {
            router: app.router(),
            document: Some(Documented::of(app.openapi())),
        }
    }

    /// A client of the internal app, with no route guard: its routes are not in the OpenAPI
    /// document, by design.
    pub fn internal(app: &InternalApp) -> Self {
        Self {
            router: app.router(),
            document: None,
        }
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.send(TestRequest::new("GET", path)).await
    }

    pub async fn send(&self, request: TestRequest) -> TestResponse {
        let sent = request.json_body();
        let parameters = request.query_parameters();
        let Ok(response) = self.router.clone().oneshot(request.into_http()).await;
        let (parts, body) = response.into_parts();
        let body = to_bytes(body, usize::MAX).await.unwrap();
        let response = TestResponse::new(parts, body);
        if let (Some(served), Some(document)) = (response.matched_route(), &self.document) {
            let operation = document.operation(served);
            document.hold_the_error(served, operation, &response);
        }
        hold_the_keys(&response, sent.as_ref(), &parameters);
        response
    }
}

/// What the OpenAPI document of an app says of the routes that can serve a request.
struct Documented {
    /// Each operation, by its method and its path template.
    operations: BTreeMap<(String, String), DocumentedErrors>,
    /// `x-common-error-codes`: the codes any route can answer.
    common_codes: BTreeSet<String>,
}

/// What an operation documents of its error answers.
struct DocumentedErrors {
    /// `x-error-codes`: the codes of its own.
    codes: BTreeSet<String>,
    /// The keys of its responses: statuses, ranges such as `4XX`, and `default`.
    statuses: BTreeSet<String>,
}

impl Documented {
    fn of(document: &OpenApi) -> Self {
        let operations = operations(document)
            .into_iter()
            .map(|(path, method, operation)| {
                let errors = DocumentedErrors {
                    codes: codes_at(operation.extensions.as_ref(), X_ERROR_CODES),
                    statuses: operation.responses.responses.keys().cloned().collect(),
                };
                ((method.to_owned(), path.to_owned()), errors)
            })
            .collect();
        Self {
            operations,
            common_codes: codes_at(document.extensions.as_ref(), X_COMMON_ERROR_CODES),
        }
    }

    /// The operation that documents the route that `served` the request; the test fails when
    /// the document has none.
    fn operation(&self, served: &MatchedRoute) -> &DocumentedErrors {
        let find = |method: &str| {
            let key = (method.to_owned(), served.template.clone());
            self.operations.get(&key)
        };
        find(served.method.as_str())
            // axum serves `HEAD` with the `GET` handler of a path that has no `HEAD` of its own.
            .or_else(|| find("GET").filter(|_| served.method == Method::HEAD))
            .unwrap_or_else(|| {
                panic!(
                    "{} {} served the request, and the OpenAPI document has no such operation: \
                     register the route with ApiRouter::route",
                    served.method, served.template
                )
            })
    }

    /// When `response` is an error envelope, its code is one that `operation` or the document
    /// lists, and its status one that `operation` documents or any route can answer.
    fn hold_the_error(
        &self,
        served: &MatchedRoute,
        operation: &DocumentedErrors,
        response: &TestResponse,
    ) {
        let Some(code) = response.error_code() else {
            return;
        };
        let route = format!("{} {}", served.method, served.template);
        let status = response.status;
        if !operation.codes.contains(&code) && !self.common_codes.contains(&code) {
            panic!(
                "{route} answered {status} {code}, a code that neither its {X_ERROR_CODES} nor \
                 {X_COMMON_ERROR_CODES} lists: give it to ApiRouter::route among the codes of \
                 the route"
            );
        }
        if !documents(&operation.statuses, status) && !COMMON_STATUSES.contains(&status) {
            panic!(
                "{route} answered {status} {code}, a status that its operation does not \
                 document: give the handler's #[utoipa::path] a {status} response with the \
                 ErrorEnvelope body"
            );
        }
    }
}

/// The codes listed at `key` of `extensions`; none when there is no such list.
fn codes_at(extensions: Option<&Extensions>, key: &str) -> BTreeSet<String> {
    extensions
        .and_then(|extensions| extensions.get(key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// Whether `statuses`, the keys of an operation's responses, document `status`: as itself, as
/// its range (`4XX` for 409), or with `default`, which stands for every status the others do not
/// name (OpenAPI 3.1, the Responses Object).
fn documents(statuses: &BTreeSet<String>, status: u16) -> bool {
    let range = format!("{}XX", status / 100);
    [status.to_string(), range, "default".to_owned()]
        .iter()
        .any(|key| statuses.contains(key))
}

/// On a 422 `VALIDATION_FAILED`, no name in a key of `error.fields` may hold `_`. To a JSON
/// body, `sent`, each key must also name a member of it, one of the `parameters` of the query
/// string, or a field missing from an object of `sent`, whatever its message: a rule such as
/// garde's `required` reports a field that was not sent too.
fn hold_the_keys(response: &TestResponse, sent: Option<&Value>, parameters: &BTreeSet<String>) {
    if response.status != 422
        || response.error_code().as_deref() != Some(ApiError::VALIDATION_FAILED)
    {
        return;
    }
    let answer = response.json();
    let keys = answer["error"]["fields"].as_object().into_iter().flatten();
    for (key, _) in keys {
        if let Some(name) = key.split('.').find(|name| name.contains('_')) {
            panic!(
                "error.fields has the key {key}, whose name {name} is a Rust name: name each \
                 field as JSON does, in camelCase"
            );
        }
        let Some(sent) = sent else {
            continue;
        };
        if member(sent, key).is_none() && !parameters.contains(key) && !is_missing_from(sent, key) {
            panic!(
                "error.fields has the key {key}, which names no member of the body that was \
                 sent: key a field by its path in the body, its JSON names in camelCase"
            );
        }
    }
}

/// Whether `key` names a field missing from an object of `body`: a name, never an index, after
/// the path of an object that holds the name neither as it is nor in camelCase. An object that
/// holds it in camelCase makes the key the Rust spelling of a field that was sent.
fn is_missing_from(body: &Value, key: &str) -> bool {
    let (parent, name) = key.rsplit_once('.').unwrap_or(("", key));
    !name.contains('[')
        && member(body, parent)
            .and_then(Value::as_object)
            .is_some_and(|object| {
                !object.contains_key(name) && !object.contains_key(&camel_case(name))
            })
}

/// The member of `body` at `key`, a path such as `address.zip` or `tags[1]`; the empty key is
/// the body itself.
fn member<'body>(body: &'body Value, key: &str) -> Option<&'body Value> {
    let mut value = body;
    for segment in key.split('.') {
        let (name, mut indices) = segment.split_at(segment.find('[').unwrap_or(segment.len()));
        if !name.is_empty() {
            value = value.as_object()?.get(name)?;
        }
        while !indices.is_empty() {
            let (index, rest) = indices.strip_prefix('[')?.split_once(']')?;
            value = value.as_array()?.get(index.parse::<usize>().ok()?)?;
            indices = rest;
        }
    }
    Some(value)
}

/// A request to send: a method, a path with its query, headers and a body.
pub struct TestRequest {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Bytes,
    streamed: bool,
}

impl TestRequest {
    /// An empty request.
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: Method::from_bytes(method.as_bytes()).unwrap(),
            path: path.to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            streamed: false,
        }
    }

    /// Adds a header, after any other of the same name.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let name = HeaderName::from_bytes(name.as_bytes()).unwrap();
        let value = HeaderValue::from_bytes(value.as_bytes()).unwrap();
        self.headers.append(name, value);
        self
    }

    /// `body` as JSON, with its content type.
    pub fn json(mut self, body: &Value) -> Self {
        let json = HeaderValue::from_static("application/json");
        self.headers.insert(CONTENT_TYPE, json);
        self.body(serde_json::to_vec(body).unwrap())
    }

    /// `body` as it is, with no content type.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Sends the body as a stream whose length is not known before it ends, as a client that
    /// streams its body does, rather than as a body of known length.
    pub fn streamed(mut self) -> Self {
        self.streamed = true;
        self
    }

    /// The names of the parameters of the query string.
    fn query_parameters(&self) -> BTreeSet<String> {
        let query = self.path.split_once('?').map_or("", |(_, query)| query);
        form_urlencoded::parse(query.as_bytes())
            .map(|(name, _)| name.into_owned())
            .collect()
    }

    /// The body, when it is sent as JSON and is JSON.
    fn json_body(&self) -> Option<Value> {
        if !is_json(&self.headers) {
            return None;
        }
        serde_json::from_slice(&self.body).ok()
    }

    fn into_http(self) -> Request<Body> {
        let body = Body::from(self.body);
        let body = if self.streamed {
            // A stream tells nothing of its length, so no layer below can know it in advance.
            Body::from_stream(body.into_data_stream())
        } else {
            body
        };
        let mut request = Request::builder()
            .method(self.method)
            .uri(self.path)
            .body(body)
            .unwrap();
        *request.headers_mut() = self.headers;
        request
    }
}

/// What the app answered, its body read whole.
pub struct TestResponse {
    pub status: u16,
    headers: HeaderMap,
    body: Bytes,
    matched_route: Option<MatchedRoute>,
}

impl TestResponse {
    /// The answer whose head is `parts`, its body read whole, with the route that served it
    /// when a route of an app in this process did.
    pub(super) fn new(parts: Parts, body: Bytes) -> Self {
        Self {
            status: parts.status.as_u16(),
            matched_route: parts.extensions.get::<MatchedRoute>().cloned(),
            headers: parts.headers,
            body,
        }
    }

    /// The first value of the header `name`.
    pub fn header(&self, name: &str) -> Option<&str> {
        let value = self.headers.get(name)?;
        Some(value.to_str().unwrap())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// The body as JSON; the test fails when it is not.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            let body = String::from_utf8_lossy(&self.body);
            panic!("the body is not JSON ({error}): {body}")
        })
    }

    /// `error.code` of an error envelope.
    pub fn error_code(&self) -> Option<String> {
        let body: Value = serde_json::from_slice(&self.body).ok()?;
        Some(body.get("error")?.get("code")?.as_str()?.to_owned())
    }

    /// The route that served the request; `None` when a fallback answered, and on an answer
    /// that came over a socket.
    pub fn matched_route(&self) -> Option<&MatchedRoute> {
        self.matched_route.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::COMMON_ERROR_CODES;

    fn keys(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|&key| key.to_owned()).collect()
    }

    #[test]
    fn the_common_statuses_are_those_of_the_common_codes() {
        let common = [
            ApiError::route_not_found(),
            ApiError::method_not_allowed(),
            ApiError::malformed_request("Request body is not valid JSON"),
            ApiError::unsupported_media_type(),
            ApiError::payload_too_large(),
            ApiError::not_acceptable(),
            ApiError::internal("a handler panicked".to_owned()),
        ];
        let mut codes: Vec<&str> = common.iter().map(ApiError::code).collect();
        codes.sort_unstable();
        let mut statuses: Vec<u16> = common.iter().map(|error| error.status().as_u16()).collect();
        statuses.sort_unstable();

        assert_eq!(codes, COMMON_ERROR_CODES);
        assert_eq!(statuses, COMMON_STATUSES);
    }

    #[test]
    fn a_status_is_documented_as_itself_by_its_range_or_by_default() {
        assert!(documents(&keys(&["200", "409"]), 409));
        assert!(documents(&keys(&["200", "4XX"]), 409));
        assert!(documents(&keys(&["200", "default"]), 503));
        assert!(!documents(&keys(&["200", "404", "5XX"]), 409));
    }
}
