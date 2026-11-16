//! The OpenAPI document of the public API: the metadata every operation carries (spec §7.2),
//! its canonical JSON, and the check that nothing is missing.

use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::{Info, OpenApi, Paths, RefOr, Response};

use crate::error::ApiError;

/// The codes any route can return, whatever it does: listed once, at the top of the document,
/// as `x-common-error-codes`, rather than on every operation. Sorted.
pub const COMMON_ERROR_CODES: &[&str] = &[
    ApiError::INTERNAL_ERROR,
    ApiError::MALFORMED_REQUEST,
    ApiError::METHOD_NOT_ALLOWED,
    ApiError::NOT_ACCEPTABLE,
    ApiError::PAYLOAD_TOO_LARGE,
    ApiError::ROUTE_NOT_FOUND,
    ApiError::UNSUPPORTED_MEDIA_TYPE,
];

/// The keys `ApiRouter::route` writes on every operation, from typed values.
pub(crate) const X_ERROR_CODES: &str = "x-error-codes";
pub(crate) const X_PERMISSION: &str = "x-permission";
pub(crate) const X_PLAN_FEATURE: &str = "x-plan-feature";
const X_COMMON_ERROR_CODES: &str = "x-common-error-codes";

// The components utoipa makes of the two envelopes: `SuccessEnvelope<Goal>` becomes
// `SuccessEnvelope_Goal`, `SuccessEnvelope<Vec<Goal>>` becomes `SuccessEnvelope_Vec_Goal`.
const SCHEMA_REFERENCE: &str = "#/components/schemas/";
const SUCCESS_ENVELOPE_PREFIX: &str = "SuccessEnvelope_";
const ERROR_ENVELOPE: &str = "ErrorEnvelope";

/// The document as it is committed: every object's keys sorted, pretty-printed, one final
/// newline. Two runs give the same bytes.
pub fn canonical_json(document: &OpenApi) -> Result<String, serde_json::Error> {
    // utoipa keeps extensions in a `HashMap`, so its own output changes from run to run. A
    // `Value` keeps its keys sorted, unless another crate switches on serde_json's
    // `preserve_order`; `sort_all_objects` sorts them in that case.
    let mut value = serde_json::to_value(document)?;
    value.sort_all_objects();
    let mut json = serde_json::to_string_pretty(&value)?;
    json.push('\n');
    Ok(json)
}

/// What the operations of `document` lack of the metadata of spec §7.2: one line per missing
/// item, naming the method, the path and the item. Empty when nothing is missing.
pub fn metadata_problems(document: &OpenApi) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, method, operation) in operations(document) {
        let mut missing = |item: &str| problems.push(format!("{method} {path}: {item}"));
        if is_blank(operation.operation_id.as_deref()) {
            missing("no operationId");
        }
        if operation
            .tags
            .iter()
            .flatten()
            .all(|tag| tag.trim().is_empty())
        {
            missing("no tag");
        }
        if is_blank(operation.summary.as_deref()) {
            missing("no summary");
        }
        let responses = &operation.responses.responses;
        let enveloped_success = responses.iter().any(|(status, response)| {
            status.starts_with('2')
                && body_component(document, response)
                    .and_then(|name| name.strip_prefix(SUCCESS_ENVELOPE_PREFIX))
                    .is_some_and(|data| !data.is_empty())
        });
        if !enveloped_success {
            missing("no 2xx response has a SuccessEnvelope body");
        }
        for (status, response) in responses {
            if !matches!(response, RefOr::T(response) if !response.description.trim().is_empty()) {
                missing(&format!("response {status} has no description"));
            }
            let is_error = status.starts_with('4') || status.starts_with('5');
            if is_error && body_component(document, response) != Some(ERROR_ENVELOPE) {
                missing(&format!("response {status} has no ErrorEnvelope body"));
            }
        }
        let extensions = operation.extensions.as_ref();
        for key in [X_ERROR_CODES, X_PERMISSION, X_PLAN_FEATURE] {
            if !extensions.is_some_and(|extensions| extensions.contains_key(key)) {
                missing(&format!("no {key}"));
            }
        }
    }
    problems
}

fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|text| text.trim().is_empty())
}

/// The schema component the JSON body of `response` refers to, when the document has it. A
/// response or a body given by reference is not followed: a check that cannot see the body
/// must not pass it.
fn body_component<'a>(document: &'a OpenApi, response: &'a RefOr<Response>) -> Option<&'a str> {
    let RefOr::T(response) = response else {
        return None;
    };
    let RefOr::T(content) = response.content.get("application/json")? else {
        return None;
    };
    let Some(RefOr::Ref(schema)) = &content.schema else {
        return None;
    };
    let name = schema.ref_location.strip_prefix(SCHEMA_REFERENCE)?;
    let schemas = &document.components.as_ref()?.schemas;
    schemas.contains_key(name).then_some(name)
}

/// The root of the public document. Its `info` is written here, so that nothing comes from
/// a crate's metadata (utoipa would write an empty licence for a crate that has none).
#[allow(dead_code)] // the app builder roots the public document on it; until then only tests call it
pub(crate) fn document_base() -> OpenApi {
    let info = Info::new("Barras API", env!("CARGO_PKG_VERSION"));
    let mut document = OpenApi::new(info, Paths::new());
    let common = ExtensionsBuilder::new().add(X_COMMON_ERROR_CODES, COMMON_ERROR_CODES);
    document.extensions = Some(common.build());
    document
}

/// Every operation of `document`, with its path and its method, in the order of the paths and
/// then of [`operations_mut`]. The method is in upper case, as a request spells it, except
/// that of an additional operation, which is as the document writes it.
pub(crate) fn operations(document: &OpenApi) -> Vec<(&str, &str, &Operation)> {
    let mut found = Vec::new();
    for (path, item) in &document.paths.paths {
        let fixed = [
            ("GET", &item.get),
            ("PUT", &item.put),
            ("POST", &item.post),
            ("DELETE", &item.delete),
            ("OPTIONS", &item.options),
            ("HEAD", &item.head),
            ("PATCH", &item.patch),
            ("TRACE", &item.trace),
            ("QUERY", &item.query),
        ];
        let fixed = fixed
            .into_iter()
            .filter_map(|(method, operation)| Some((method, operation.as_ref()?)));
        let additional = item
            .additional_operations
            .iter()
            .map(|(method, operation)| (method.as_str(), operation));
        found.extend(
            fixed
                .chain(additional)
                .map(|(method, operation)| (path.as_str(), method, operation)),
        );
    }
    found
}

/// Every operation of a path item: the eight methods of OpenAPI 3.1, then the `query` and
/// additional operations of 3.2, which `routes!` never makes but a route built by hand could.
pub(crate) fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = &mut Operation> {
    [
        &mut item.get,
        &mut item.put,
        &mut item.post,
        &mut item.delete,
        &mut item.options,
        &mut item.head,
        &mut item.patch,
        &mut item.trace,
        &mut item.query,
    ]
    .into_iter()
    .flatten()
    .chain(item.additional_operations.values_mut())
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;
    use utoipa::ToSchema;
    use utoipa::openapi::extensions::ExtensionsBuilder;
    use utoipa::openapi::path::{HttpMethod, Paths};
    use utoipa::openapi::{Content, Info, Ref, RefOr, Response, ResponseBuilder};
    use utoipa_axum::router::OpenApiRouter;
    use utoipa_axum::routes;

    use super::*;
    use crate::envelope::{ApiResult, ApiSuccess, ErrorEnvelope, NoData, SuccessEnvelope};
    use crate::router::{Access, ApiRouter};

    /// What the fixture route answers.
    #[derive(Serialize, ToSchema)]
    struct Fixture {
        name: &'static str,
    }

    /// Save the settings, with nothing to return.
    #[utoipa::path(
        put,
        path = "/settings",
        tag = "settings",
        responses(
            (status = 200, description = "Saved", body = SuccessEnvelope<NoData>),
            (status = 422, description = "Invalid settings", body = ErrorEnvelope),
        ),
    )]
    async fn save_settings() -> ApiResult<NoData> {
        Ok(ApiSuccess::ok("Settings saved", NoData))
    }

    /// Read the fixture.
    #[utoipa::path(
        get,
        path = "/fixtures",
        tag = "fixtures",
        responses(
            (status = 200, description = "The fixture", body = SuccessEnvelope<Fixture>),
            (status = 404, description = "No fixture", body = ErrorEnvelope),
            (status = 503, description = "The store is down", body = ErrorEnvelope),
        ),
    )]
    async fn read_fixture() -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok("Fixture read", Fixture { name: "read" }))
    }

    #[utoipa::path(
        post,
        path = "/careless",
        responses((status = 200), (status = 409, description = "Taken")),
    )]
    async fn careless() {}

    /// The public document of `routes` as the app builds it: nested under `/api/v1`, on the
    /// base.
    fn document_of(routes: ApiRouter) -> OpenApi {
        OpenApiRouter::with_openapi(document_base())
            .nest("/api/v1", routes.0)
            .split_for_parts()
            .1
    }

    /// A document with every item of spec §7.2, to take one away from.
    fn complete() -> OpenApi {
        let routes =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &["FIXTURE_GONE"]);
        document_of(routes)
    }

    fn fixture_operation(document: &mut OpenApi) -> &mut Operation {
        let item = document.paths.paths.get_mut("/api/v1/fixtures").unwrap();
        item.get.as_mut().unwrap()
    }

    /// A described response whose JSON body refers to the schema component `name`.
    fn body_of(name: &str) -> RefOr<Response> {
        let content = Content::new(Some(Ref::from_schema_name(name)));
        let response = ResponseBuilder::new()
            .description("Changed by the test")
            .content("application/json", content);
        RefOr::T(response.build())
    }

    #[test]
    fn the_common_codes_are_the_seven_of_the_spec_sorted() {
        assert_eq!(
            COMMON_ERROR_CODES,
            [
                "INTERNAL_ERROR",
                "MALFORMED_REQUEST",
                "METHOD_NOT_ALLOWED",
                "NOT_ACCEPTABLE",
                "PAYLOAD_TOO_LARGE",
                "ROUTE_NOT_FOUND",
                "UNSUPPORTED_MEDIA_TYPE",
            ]
        );
        assert!(COMMON_ERROR_CODES.is_sorted());
    }

    #[test]
    fn the_common_codes_are_those_the_adapter_answers_with() {
        let mut answered = [
            ApiError::route_not_found().code(),
            ApiError::method_not_allowed().code(),
            ApiError::malformed_request("Request body is not valid JSON").code(),
            ApiError::unsupported_media_type().code(),
            ApiError::payload_too_large().code(),
            ApiError::not_acceptable().code(),
            ApiError::internal("handler panicked".to_owned()).code(),
        ];
        answered.sort_unstable();

        assert_eq!(COMMON_ERROR_CODES, answered);
    }

    #[test]
    fn the_base_has_the_title_the_version_and_the_common_codes_and_nothing_else() {
        assert_eq!(
            serde_json::to_value(document_base()).unwrap(),
            json!({
                "openapi": "3.1.0",
                "info": { "title": "Barras API", "version": env!("CARGO_PKG_VERSION") },
                "paths": {},
                "x-common-error-codes": COMMON_ERROR_CODES,
            })
        );
    }

    #[test]
    fn canonical_json_sorts_every_key_and_ends_with_one_newline() {
        let mut operation = Operation::new();
        operation.operation_id = Some("read_fixture".to_owned());
        operation.extensions = Some(
            ExtensionsBuilder::new()
                .add("x-plan-feature", "none")
                .add("x-permission", "public")
                .add("x-error-codes", Vec::<&str>::new())
                .build(),
        );
        let mut document = OpenApi::new(Info::new("Barras API", "1.2.3"), Paths::new());
        let item = PathItem::new(HttpMethod::Get, operation);
        document
            .paths
            .paths
            .insert("/api/v1/fixtures".to_owned(), item);

        let json = canonical_json(&document).unwrap();

        let expected = r#"{
  "info": {
    "title": "Barras API",
    "version": "1.2.3"
  },
  "openapi": "3.1.0",
  "paths": {
    "/api/v1/fixtures": {
      "get": {
        "operationId": "read_fixture",
        "responses": {},
        "x-error-codes": [],
        "x-permission": "public",
        "x-plan-feature": "none"
      }
    }
  }
}
"#;
        assert_eq!(json, expected);
    }

    #[test]
    fn documents_built_apart_give_the_same_bytes() {
        // Each build keeps the operation's extensions in a new `HashMap`, with its own seed.
        let mut document = complete();
        let extensions = fixture_operation(&mut document)
            .extensions
            .as_ref()
            .unwrap();
        assert_eq!(extensions.len(), 3);
        let first = canonical_json(&document).unwrap();

        for _ in 0..16 {
            assert_eq!(canonical_json(&complete()).unwrap(), first);
        }
    }

    #[test]
    fn a_complete_document_has_no_problem() {
        assert_eq!(metadata_problems(&complete()), Vec::<String>::new());
    }

    #[test]
    fn a_route_with_nothing_to_return_can_be_complete() {
        let routes = ApiRouter::new().route(
            routes!(save_settings),
            Access::public(),
            &[ApiError::VALIDATION_FAILED],
        );

        assert_eq!(
            metadata_problems(&document_of(routes)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_missing_operation_id_is_one_line() {
        let mut document = complete();
        fixture_operation(&mut document).operation_id = None;

        assert_eq!(
            metadata_problems(&document),
            ["GET /api/v1/fixtures: no operationId"]
        );
    }

    #[test]
    fn a_missing_or_blank_tag_is_one_line() {
        for tags in [None, Some(vec![]), Some(vec![" ".to_owned()])] {
            let label = format!("{tags:?}");
            let mut document = complete();
            fixture_operation(&mut document).tags = tags;

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: no tag"],
                "{label}"
            );
        }
    }

    #[test]
    fn a_missing_or_blank_summary_is_one_line() {
        for summary in [None, Some(" ".to_owned())] {
            let label = format!("{summary:?}");
            let mut document = complete();
            fixture_operation(&mut document).summary = summary;

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: no summary"],
                "{label}"
            );
        }
    }

    #[test]
    fn no_success_with_a_success_envelope_is_one_line() {
        // `Missing` names a component the document does not have.
        for body in [
            None,
            Some("ErrorEnvelope"),
            Some("Fixture"),
            Some("SuccessEnvelope_Missing"),
        ] {
            let mut document = complete();
            let responses = &mut fixture_operation(&mut document).responses.responses;
            responses.remove("200");
            if let Some(name) = body {
                responses.insert("200".to_owned(), body_of(name));
            }

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: no 2xx response has a SuccessEnvelope body"],
                "{body:?}"
            );
        }
    }

    #[test]
    fn an_error_status_without_the_error_envelope_is_one_line() {
        let mut document = complete();
        let responses = &mut fixture_operation(&mut document).responses.responses;
        responses.insert("404".to_owned(), body_of("SuccessEnvelope_Fixture"));
        let bodiless = ResponseBuilder::new().description("The store is down");
        responses.insert("503".to_owned(), RefOr::T(bodiless.build()));

        assert_eq!(
            metadata_problems(&document),
            [
                "GET /api/v1/fixtures: response 404 has no ErrorEnvelope body",
                "GET /api/v1/fixtures: response 503 has no ErrorEnvelope body",
            ]
        );
    }

    #[test]
    fn an_error_envelope_the_document_lacks_is_no_error_envelope() {
        let mut document = complete();
        let components = document.components.as_mut().unwrap();
        components.schemas.remove("ErrorEnvelope");

        assert_eq!(
            metadata_problems(&document),
            [
                "GET /api/v1/fixtures: response 404 has no ErrorEnvelope body",
                "GET /api/v1/fixtures: response 503 has no ErrorEnvelope body",
            ]
        );
    }

    #[test]
    fn a_response_without_a_description_is_one_line() {
        for description in ["", " "] {
            let mut document = complete();
            let responses = &mut fixture_operation(&mut document).responses.responses;
            let Some(RefOr::T(not_found)) = responses.get_mut("404") else {
                panic!("the fixture has a 404");
            };
            not_found.description = description.to_owned();

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: response 404 has no description"],
                "{description:?}"
            );
        }
    }

    #[test]
    fn a_response_given_by_reference_is_not_followed() {
        let mut document = complete();
        let responses = &mut fixture_operation(&mut document).responses.responses;
        responses.insert("404".to_owned(), Ref::from_response_name("NotFound").into());

        assert_eq!(
            metadata_problems(&document),
            [
                "GET /api/v1/fixtures: response 404 has no description",
                "GET /api/v1/fixtures: response 404 has no ErrorEnvelope body",
            ]
        );
    }

    #[test]
    fn each_missing_key_of_the_router_is_one_line() {
        for key in ["x-error-codes", "x-permission", "x-plan-feature"] {
            let mut document = complete();
            let extensions = fixture_operation(&mut document)
                .extensions
                .as_mut()
                .unwrap();
            extensions.remove(key);

            assert_eq!(
                metadata_problems(&document),
                [format!("GET /api/v1/fixtures: no {key}")]
            );
        }

        let mut document = complete();
        fixture_operation(&mut document).extensions = None;
        assert_eq!(
            metadata_problems(&document),
            [
                "GET /api/v1/fixtures: no x-error-codes",
                "GET /api/v1/fixtures: no x-permission",
                "GET /api/v1/fixtures: no x-plan-feature",
            ]
        );
    }

    #[test]
    fn a_careless_handler_is_reported_item_by_item() {
        let routes = ApiRouter::new().route(routes!(careless), Access::public(), &[]);

        assert_eq!(
            metadata_problems(&document_of(routes)),
            [
                "POST /api/v1/careless: no tag",
                "POST /api/v1/careless: no summary",
                "POST /api/v1/careless: no 2xx response has a SuccessEnvelope body",
                "POST /api/v1/careless: response 200 has no description",
                "POST /api/v1/careless: response 409 has no ErrorEnvelope body",
            ]
        );
    }

    #[test]
    fn every_operation_is_listed_with_its_path_and_method() {
        let mut copied = PathItem::new(HttpMethod::Get, Operation::new());
        copied.put = Some(Operation::new());
        let mut searched = PathItem::default();
        searched.query = Some(Operation::new());
        searched
            .additional_operations
            .insert("COPY".to_owned(), Operation::new());
        let mut document = OpenApi::new(Info::new("Barras API", "1.2.3"), Paths::new());
        let paths = &mut document.paths.paths;
        paths.insert(
            "/b".to_owned(),
            PathItem::new(HttpMethod::Post, Operation::new()),
        );
        paths.insert("/a".to_owned(), copied);
        paths.insert("/c".to_owned(), searched);

        let listed: Vec<(&str, &str)> = operations(&document)
            .into_iter()
            .map(|(path, method, _)| (path, method))
            .collect();

        assert_eq!(
            listed,
            [
                ("/a", "GET"),
                ("/a", "PUT"),
                ("/b", "POST"),
                ("/c", "QUERY"),
                ("/c", "COPY"),
            ]
        );
    }
}
