//! The guard against a route without OpenAPI metadata (spec §9.3): the app refuses a route
//! the document cannot describe, and a request that a route the document lacks serves fails
//! the test that sent it, naming the route.

use axum::routing::get;
use utoipa::openapi::path::{HttpMethod, Operation, PathItem, Paths};
use utoipa_axum::router::UtoipaMethodRouter;

use super::support::{SERVICE, client, fixture_routes};
use crate::testing::{TestClient, TestRequest};
use crate::{Access, ApiResult, ApiRouter, ApiSuccess, HttpApp, NoData};

async fn hidden() -> ApiResult<NoData> {
    Ok(ApiSuccess::ok("Hidden", NoData))
}

/// The fixture app, plus `GET /api/v1/hidden`, which the document lacks.
fn client_with_a_hidden_route() -> TestClient {
    let app = HttpApp::builder(SERVICE)
        .routes(fixture_routes())
        .undocumented_route("/hidden", get(hidden))
        .build()
        .unwrap();
    TestClient::new(&app)
}

#[test]
fn an_app_with_a_route_the_document_cannot_describe_is_not_built() {
    let mut paths = Paths::new();
    paths
        .paths
        .insert("/hidden".to_owned(), PathItem::default());
    let operationless: UtoipaMethodRouter = (Vec::new(), paths, get(hidden));
    let pathless: UtoipaMethodRouter = (Vec::new(), Paths::new(), get(hidden));
    let routes = ApiRouter::new().route(operationless, Access::public(), &[]);
    let more = ApiRouter::new().route(pathless, Access::public(), &[]);

    let built = HttpApp::builder(SERVICE)
        .routes(fixture_routes())
        .routes(routes)
        .routes(more)
        .build();

    let Err(error) = built else {
        panic!("an app with a route the document cannot describe was built");
    };
    assert_eq!(
        error.problems(),
        [
            "the route at /hidden documents no operation",
            "a route names no path"
        ]
    );
    assert_eq!(
        error.to_string(),
        "the OpenAPI document cannot describe every route:\n\
         the route at /hidden documents no operation\n\
         a route names no path"
    );
}

#[tokio::test]
#[should_panic(expected = "GET /api/v1/hidden")]
async fn a_route_the_document_lacks_fails_the_test_that_reaches_it() {
    client_with_a_hidden_route().get("/api/v1/hidden").await;
}

#[tokio::test]
#[should_panic(expected = "HEAD /api/v1/hidden")]
async fn head_on_a_route_the_document_lacks_fails_too() {
    let request = TestRequest::new("HEAD", "/api/v1/hidden");

    client_with_a_hidden_route().send(request).await;
}

#[tokio::test]
#[should_panic(expected = "GET /api/v1/posted")]
async fn a_method_the_document_lacks_on_a_documented_path_fails() {
    // A route built by hand may document one method and serve another.
    let item = PathItem::new(HttpMethod::Post, Operation::new());
    let mut paths = Paths::new();
    paths.paths.insert("/posted".to_owned(), item);
    let route: UtoipaMethodRouter = (Vec::new(), paths, get(hidden));
    let routes = ApiRouter::new().route(route, Access::public(), &[]);
    let app = HttpApp::builder(SERVICE).routes(routes).build().unwrap();
    let client = TestClient::new(&app);

    client.get("/api/v1/posted").await;
}

#[tokio::test]
async fn a_documented_route_passes() {
    let response = client().get("/api/v1/small").await;

    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn head_on_a_documented_get_passes() {
    let response = client()
        .send(TestRequest::new("HEAD", "/api/v1/small"))
        .await;

    assert_eq!(response.status, 200);
    assert!(response.bytes().is_empty());
}

#[tokio::test]
async fn the_fallback_passes() {
    let response = client_with_a_hidden_route().get("/api/v1/nowhere").await;

    assert_eq!(response.status, 404);
}

#[tokio::test]
async fn a_wrong_method_passes_even_on_a_route_the_document_lacks() {
    let client = client_with_a_hidden_route();

    for path in ["/api/v1/small", "/api/v1/hidden"] {
        let response = client.send(TestRequest::new("POST", path)).await;

        assert_eq!(response.status, 405, "{path}");
    }
}
