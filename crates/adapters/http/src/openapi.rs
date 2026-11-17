//! The OpenAPI document of the public API: the metadata every operation carries (spec §7.2),
//! its canonical JSON, and the check that nothing is missing.

use std::collections::BTreeMap;

use serde_json::Value;
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::{Info, OpenApi, Paths, RefOr, Response};

use crate::error::ApiError;

/// The title of the public document, written by hand, and its version: the workspace version,
/// which this crate inherits as its own (`version.workspace = true`).
const TITLE: &str = "Barras API";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The codes any route can return, whatever it does: listed once, at the top of the document,
/// as `x-common-error-codes`, rather than on every operation. Sorted.
pub const COMMON_ERROR_CODES: &[&str] = &[
    ApiError::INTERNAL_ERROR,
    ApiError::MALFORMED_REQUEST,
    ApiError::METHOD_NOT_ALLOWED,
    ApiError::NOT_ACCEPTABLE,
    ApiError::PAYLOAD_TOO_LARGE,
    ApiError::REQUEST_TIMEOUT,
    ApiError::ROUTE_NOT_FOUND,
    ApiError::UNSUPPORTED_MEDIA_TYPE,
];

/// The keys `ApiRouter::route` writes on every operation, from typed values.
pub(crate) const X_ERROR_CODES: &str = "x-error-codes";
pub(crate) const X_PERMISSION: &str = "x-permission";
pub(crate) const X_PLAN_FEATURE: &str = "x-plan-feature";
pub(crate) const X_COMMON_ERROR_CODES: &str = "x-common-error-codes";

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

/// What `document` lacks of the metadata of spec §7.2, one line per missing item: first what
/// its root lacks of what `document_base` writes; then each item an operation lacks, naming
/// the method and the path; last, each operationId that more than one operation uses. Empty
/// when nothing is missing.
pub fn metadata_problems(document: &OpenApi) -> Vec<String> {
    let mut problems = root_problems(document);
    let mut operation_ids: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (path, method, operation) in operations(document) {
        let mut missing = |item: &str| problems.push(format!("{method} {path}: {item}"));
        match operation.operation_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => {
                let used_by = operation_ids.entry(id).or_default();
                used_by.push(format!("{method} {path}"));
            }
            _ => missing("no operationId"),
        }
        match operation.tags.as_deref().unwrap_or_default() {
            [tag] if !tag.trim().is_empty() => {}
            [] | [_] => missing("no tag"),
            tags => missing(&format!("{} tags, not one", tags.len())),
        }
        if is_blank(operation.summary.as_deref()) {
            missing("no summary");
        }
        let responses = &operation.responses.responses;
        if !responses.keys().any(|status| is_success(status)) {
            missing("no 2xx response");
        }
        for (status, response) in responses {
            if !matches!(response, RefOr::T(response) if !response.description.trim().is_empty()) {
                missing(&format!("response {status} has no description"));
            }
            // A route with nothing to return answers `NoData`, so a 2xx without a body, a 204
            // included, is refused like any other.
            let body = body_component(document, response);
            if is_success(status) && !is_success_envelope(body) {
                missing(&format!("response {status} has no SuccessEnvelope body"));
            }
            if is_error(status) && body != Some(ERROR_ENVELOPE) {
                missing(&format!("response {status} has no ErrorEnvelope body"));
            }
        }
        let extensions = operation.extensions.as_ref();
        let value = |key: &str| extensions.and_then(|extensions| extensions.get(key));
        match value(X_ERROR_CODES).map(error_codes) {
            None => missing(&format!("no {X_ERROR_CODES}")),
            Some(None) => missing(&format!(
                "{X_ERROR_CODES} is not a sorted list of unique codes"
            )),
            Some(Some(codes)) => {
                let common = codes
                    .into_iter()
                    .filter(|code| COMMON_ERROR_CODES.contains(code));
                for code in common {
                    missing(&format!(
                        "{X_ERROR_CODES} repeats {code} of {X_COMMON_ERROR_CODES}"
                    ));
                }
            }
        }
        for key in [X_PERMISSION, X_PLAN_FEATURE] {
            if is_blank(value(key).and_then(Value::as_str)) {
                missing(&format!("no {key}"));
            }
        }
    }
    for (id, used_by) in operation_ids {
        if used_by.len() > 1 {
            let used_by = used_by.join(" and ");
            problems.push(format!("operationId {id} is used by {used_by}"));
        }
    }
    problems
}

/// What the root of `document` lacks of what `document_base` writes there: a document nested
/// on another root would pass every other check.
fn root_problems(document: &OpenApi) -> Vec<String> {
    let mut problems = Vec::new();
    if document.info.title != TITLE {
        problems.push(format!("info.title is not {TITLE}"));
    }
    if document.info.version != VERSION {
        problems.push(format!("info.version is not {VERSION}"));
    }
    let common = document
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(X_COMMON_ERROR_CODES));
    if common != Some(&Value::from(COMMON_ERROR_CODES)) {
        let problem = format!("{X_COMMON_ERROR_CODES} is not the list of COMMON_ERROR_CODES");
        problems.push(problem);
    }
    problems
}

fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|text| text.trim().is_empty())
}

/// A 2xx status, or the range `2XX`.
fn is_success(status: &str) -> bool {
    status.starts_with('2')
}

/// A 4xx or 5xx status, a range of them, or `default`, which stands for every other status.
fn is_error(status: &str) -> bool {
    status.starts_with('4') || status.starts_with('5') || status == "default"
}

/// `SuccessEnvelope_` followed by the name of the data: the prefix alone wraps nothing.
fn is_success_envelope(body: Option<&str>) -> bool {
    body.and_then(|name| name.strip_prefix(SUCCESS_ENVELOPE_PREFIX))
        .is_some_and(|data| !data.is_empty())
}

/// The codes of `value` when it is a list of codes, each once and in order; `None` otherwise.
fn error_codes(value: &Value) -> Option<Vec<&str>> {
    let codes: Vec<&str> = value
        .as_array()?
        .iter()
        .map(|code| code.as_str().filter(|code| !code.trim().is_empty()))
        .collect::<Option<_>>()?;
    codes
        .is_sorted_by(|earlier, later| earlier < later)
        .then_some(codes)
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

/// The root of the public document. Its `info` is set here, the title by hand and the version
/// the workspace's, so that nothing else comes from a crate's metadata (utoipa would write an
/// empty licence for a crate that has none).
pub(crate) fn document_base() -> OpenApi {
    let info = Info::new(TITLE, VERSION);
    let mut document = OpenApi::new(info, Paths::new());
    let common = ExtensionsBuilder::new().add(X_COMMON_ERROR_CODES, COMMON_ERROR_CODES);
    document.extensions = Some(common.build());
    document
}

/// Every operation of `document`, with its path and its method, in the order of the paths and
/// then of [`operations_of`].
pub(crate) fn operations(document: &OpenApi) -> Vec<(&str, &str, &Operation)> {
    document
        .paths
        .paths
        .iter()
        .flat_map(|(path, item)| {
            operations_of(item).map(move |(method, operation)| (path.as_str(), method, operation))
        })
        .collect()
}

/// Every operation of a path item, with its method, in the order of [`operations_mut`]. The
/// method is in upper case, as a request spells it, except that of an additional operation,
/// which is as the document writes it.
pub(crate) fn operations_of(item: &PathItem) -> impl Iterator<Item = (&str, &Operation)> {
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
    let additional = item
        .additional_operations
        .iter()
        .map(|(method, operation)| (method.as_str(), operation));
    fixed
        .into_iter()
        .filter_map(|(method, operation)| Some((method, operation.as_ref()?)))
        .chain(additional)
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
    use std::time::Duration;

    use serde::Serialize;
    use serde_json::{Value, json};
    use utoipa::ToSchema;
    use utoipa::openapi::extensions::{Extensions, ExtensionsBuilder};
    use utoipa::openapi::path::{HttpMethod, Paths};
    use utoipa::openapi::schema::{ObjectBuilder, Type};
    use utoipa::openapi::{Content, Info, Ref, RefOr, Response, ResponseBuilder};
    use utoipa_axum::router::OpenApiRouter;
    use utoipa_axum::routes;

    use super::*;
    use crate::envelope::{ApiResult, ApiSuccess, ErrorEnvelope, NoData, SuccessEnvelope};
    use crate::router::{Access, ApiRouter};

    /// Two handlers of the same name, in two modules: utoipa names each operation after its
    /// function, so both get the operationId `read`.
    mod first {
        use super::*;

        /// Read the first.
        #[utoipa::path(
            get,
            path = "/first",
            tag = "fixtures",
            responses((status = 200, description = "The first", body = SuccessEnvelope<Fixture>)),
        )]
        pub(super) async fn read() -> ApiResult<Fixture> {
            Ok(ApiSuccess::ok("First read", Fixture { name: "first" }))
        }
    }

    mod second {
        use super::*;

        /// Read the second.
        #[utoipa::path(
            get,
            path = "/second",
            tag = "fixtures",
            responses((status = 200, description = "The second", body = SuccessEnvelope<Fixture>)),
        )]
        pub(super) async fn read() -> ApiResult<Fixture> {
            Ok(ApiSuccess::ok("Second read", Fixture { name: "second" }))
        }
    }

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
            .nest("/api/v1", routes.into_router().unwrap())
            .split_for_parts()
            .1
    }

    /// A described response whose JSON body is `content`.
    fn content_of(content: RefOr<Content>) -> RefOr<Response> {
        let mut response = ResponseBuilder::new()
            .description("Changed by the test")
            .build();
        response
            .content
            .insert("application/json".to_owned(), content);
        RefOr::T(response)
    }

    /// A described response whose JSON body is a schema written in place.
    fn inline_body() -> RefOr<Response> {
        let schema = ObjectBuilder::new().schema_type(Type::Object);
        content_of(RefOr::T(Content::new(Some(schema))))
    }

    /// A described response whose JSON body is given by reference.
    fn body_by_reference() -> RefOr<Response> {
        content_of(RefOr::Ref(Ref::new("#/components/schemas/Fixture")))
    }

    /// The extensions of the fixture operation, to change one.
    fn fixture_extensions(document: &mut OpenApi) -> &mut Extensions {
        fixture_operation(document).extensions.as_mut().unwrap()
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
    fn the_common_codes_are_the_eight_of_the_spec_sorted() {
        assert_eq!(
            COMMON_ERROR_CODES,
            [
                "INTERNAL_ERROR",
                "MALFORMED_REQUEST",
                "METHOD_NOT_ALLOWED",
                "NOT_ACCEPTABLE",
                "PAYLOAD_TOO_LARGE",
                "REQUEST_TIMEOUT",
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
            ApiError::request_timeout(Duration::from_secs(5)).code(),
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
    fn more_than_one_tag_is_one_line() {
        for tags in [
            vec!["fixtures".to_owned(), "health".to_owned()],
            vec!["fixtures".to_owned(), " ".to_owned()],
        ] {
            let label = format!("{tags:?}");
            let mut document = complete();
            fixture_operation(&mut document).tags = Some(tags);

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: 2 tags, not one"],
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
    fn no_2xx_response_is_one_line() {
        // A success envelope counts only on a 2xx.
        for other in [None, Some(("302", body_of("SuccessEnvelope_Fixture")))] {
            let mut document = complete();
            let responses = &mut fixture_operation(&mut document).responses.responses;
            responses.remove("200");
            let label = other.as_ref().map(|(status, _)| *status);
            if let Some((status, response)) = other {
                responses.insert(status.to_owned(), response);
            }

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: no 2xx response"],
                "{label:?}"
            );
        }
    }

    #[test]
    fn a_2xx_without_a_success_envelope_is_one_line() {
        // `Missing` names a component the document does not have.
        let cases = [
            ("an error envelope", body_of("ErrorEnvelope")),
            ("bare data", body_of("Fixture")),
            ("a component it lacks", body_of("SuccessEnvelope_Missing")),
            ("only the prefix", body_of("SuccessEnvelope_")),
            ("a body by reference", body_by_reference()),
            ("a body in place", inline_body()),
            (
                "no body",
                RefOr::T(ResponseBuilder::new().description("Empty").build()),
            ),
        ];
        for (label, response) in cases {
            let mut document = complete();
            let components = document.components.as_mut().unwrap();
            let schema = components.schemas["SuccessEnvelope_Fixture"].clone();
            components
                .schemas
                .insert("SuccessEnvelope_".to_owned(), schema);
            let responses = &mut fixture_operation(&mut document).responses.responses;
            responses.insert("200".to_owned(), response);

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: response 200 has no SuccessEnvelope body"],
                "{label}"
            );
        }
    }

    #[test]
    fn every_2xx_needs_a_success_envelope_and_default_an_error_envelope() {
        let mut document = complete();
        let responses = &mut fixture_operation(&mut document).responses.responses;
        responses.insert("201".to_owned(), inline_body());
        let bodiless = ResponseBuilder::new().description("Nothing to return");
        responses.insert("204".to_owned(), RefOr::T(bodiless.build()));
        responses.insert("default".to_owned(), inline_body());

        assert_eq!(
            metadata_problems(&document),
            [
                "GET /api/v1/fixtures: response 201 has no SuccessEnvelope body",
                "GET /api/v1/fixtures: response 204 has no SuccessEnvelope body",
                "GET /api/v1/fixtures: response default has no ErrorEnvelope body",
            ]
        );
    }

    #[test]
    fn a_default_response_without_the_error_envelope_is_one_line() {
        for (label, response) in [
            ("a success envelope", body_of("SuccessEnvelope_Fixture")),
            ("a body by reference", body_by_reference()),
            ("a body in place", inline_body()),
            (
                "no body",
                RefOr::T(ResponseBuilder::new().description("Empty").build()),
            ),
        ] {
            let mut document = complete();
            let responses = &mut fixture_operation(&mut document).responses.responses;
            responses.insert("default".to_owned(), response);

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: response default has no ErrorEnvelope body"],
                "{label}"
            );
        }
    }

    #[test]
    fn an_operation_id_used_twice_is_one_line_naming_every_operation() {
        let routes = ApiRouter::new()
            .route(routes!(first::read), Access::public(), &[])
            .route(routes!(second::read), Access::public(), &[]);

        assert_eq!(
            metadata_problems(&document_of(routes)),
            ["operationId read is used by GET /api/v1/first and GET /api/v1/second"]
        );
    }

    #[test]
    fn a_permission_or_a_plan_feature_that_is_blank_or_not_text_is_missing() {
        for (key, value) in [
            ("x-permission", json!("")),
            ("x-permission", json!(" ")),
            ("x-permission", json!(5)),
            ("x-plan-feature", json!(" ")),
            ("x-plan-feature", Value::Null),
        ] {
            let mut document = complete();
            fixture_extensions(&mut document).insert(key.to_owned(), value.clone());

            assert_eq!(
                metadata_problems(&document),
                [format!("GET /api/v1/fixtures: no {key}")],
                "{value}"
            );
        }
    }

    #[test]
    fn the_error_codes_must_be_a_sorted_list_of_unique_codes() {
        for codes in [
            json!({ "not": "an array" }),
            json!("FIXTURE_GONE"),
            json!(["FIXTURE_LOCKED", "FIXTURE_GONE"]),
            json!(["FIXTURE_GONE", "FIXTURE_GONE"]),
            json!(["FIXTURE_GONE", 1]),
            json!([" "]),
        ] {
            let mut document = complete();
            fixture_extensions(&mut document).insert("x-error-codes".to_owned(), codes.clone());

            assert_eq!(
                metadata_problems(&document),
                ["GET /api/v1/fixtures: x-error-codes is not a sorted list of unique codes"],
                "{codes}"
            );
        }
    }

    #[test]
    fn the_codes_of_a_route_do_not_repeat_a_common_code() {
        let routes = ApiRouter::new().route(
            routes!(read_fixture),
            Access::public(),
            &[
                ApiError::INTERNAL_ERROR,
                "FIXTURE_GONE",
                ApiError::MALFORMED_REQUEST,
            ],
        );

        assert_eq!(
            metadata_problems(&document_of(routes)),
            [
                "GET /api/v1/fixtures: x-error-codes repeats INTERNAL_ERROR of x-common-error-codes",
                "GET /api/v1/fixtures: x-error-codes repeats MALFORMED_REQUEST of x-common-error-codes",
            ]
        );
    }

    #[test]
    fn a_document_not_rooted_on_the_base_is_reported() {
        let routes: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &["FIXTURE_GONE"]);

        let document = OpenApiRouter::default()
            .nest("/api/v1", routes.into_router().unwrap())
            .split_for_parts()
            .1;

        assert_eq!(
            metadata_problems(&document),
            [
                "info.title is not Barras API".to_owned(),
                format!("info.version is not {}", env!("CARGO_PKG_VERSION")),
                "x-common-error-codes is not the list of COMMON_ERROR_CODES".to_owned(),
            ]
        );
    }

    #[test]
    fn each_item_of_the_root_is_one_line() {
        let mut retitled = complete();
        retitled.info.title = "Other API".to_owned();
        let mut reversioned = complete();
        reversioned.info.version = "0.0.0".to_owned();
        let mut short = complete();
        let common = short.extensions.as_mut().unwrap();
        common.insert(
            "x-common-error-codes".to_owned(),
            json!(&COMMON_ERROR_CODES[1..]),
        );
        let mut bare = complete();
        bare.extensions = None;

        for (document, problem) in [
            (retitled, "info.title is not Barras API".to_owned()),
            (
                reversioned,
                format!("info.version is not {}", env!("CARGO_PKG_VERSION")),
            ),
            (
                short,
                "x-common-error-codes is not the list of COMMON_ERROR_CODES".to_owned(),
            ),
            (
                bare,
                "x-common-error-codes is not the list of COMMON_ERROR_CODES".to_owned(),
            ),
        ] {
            assert_eq!(metadata_problems(&document), [problem]);
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
                "POST /api/v1/careless: response 200 has no description",
                "POST /api/v1/careless: response 200 has no SuccessEnvelope body",
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
