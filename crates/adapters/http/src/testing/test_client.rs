//! An in-process client for the public app, and the route guard of spec §9.3.

use std::collections::BTreeSet;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request};
use serde_json::Value;
use tower::ServiceExt;

use crate::app::HttpApp;
use crate::middleware::MatchedRoute;
use crate::openapi::operations;

/// Sends requests to an app in-process, through the whole stack of middleware, and fails the
/// test when a route the OpenAPI document does not have served one: so any test that reaches
/// a route registered around `ApiRouter::route` fails, naming it.
pub struct TestClient {
    router: Router,
    /// Each operation of the document, as its method and its path template.
    documented: BTreeSet<(String, String)>,
}

impl TestClient {
    pub fn new(app: &HttpApp) -> Self {
        let documented = operations(app.openapi())
            .into_iter()
            .map(|(path, method, _)| (method.to_owned(), path.to_owned()))
            .collect();
        Self {
            router: app.router(),
            documented,
        }
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.send(TestRequest::new("GET", path)).await
    }

    pub async fn send(&self, request: TestRequest) -> TestResponse {
        let Ok(response) = self.router.clone().oneshot(request.into_http()).await;
        let (parts, body) = response.into_parts();
        let matched_route = parts.extensions.get::<MatchedRoute>().cloned();
        if let Some(served) = &matched_route {
            self.hold_to_the_document(served);
        }
        let body = to_bytes(body, usize::MAX).await.unwrap();
        TestResponse {
            status: parts.status.as_u16(),
            headers: parts.headers,
            body,
            matched_route,
        }
    }

    fn hold_to_the_document(&self, served: &MatchedRoute) {
        let has = |method: &str| {
            self.documented
                .contains(&(method.to_owned(), served.template.clone()))
        };
        // axum serves `HEAD` with the `GET` handler of a path that has no `HEAD` of its own.
        let documented =
            has(served.method.as_str()) || (served.method == Method::HEAD && has("GET"));
        if !documented {
            panic!(
                "{} {} served the request, and the OpenAPI document has no such operation: \
                 register the route with ApiRouter::route",
                served.method, served.template
            );
        }
    }
}

/// A request to send: a method, a path with its query, headers and a body.
pub struct TestRequest {
    method: Method,
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

impl TestRequest {
    /// An empty request.
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: Method::from_bytes(method.as_bytes()).unwrap(),
            path: path.to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
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

    fn into_http(self) -> Request<Body> {
        let mut request = Request::builder()
            .method(self.method)
            .uri(self.path)
            .body(Body::from(self.body))
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

    /// The route that served the request; `None` when a fallback answered.
    pub fn matched_route(&self) -> Option<&MatchedRoute> {
        self.matched_route.as_ref()
    }
}
