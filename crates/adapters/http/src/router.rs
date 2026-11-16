//! The door to the public API: a route is a documented handler registered together with what
//! it needs from the caller and the error codes of its own it can return.

use std::collections::{BTreeMap, BTreeSet};

use barras_kernel::Feature;
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::Paths;
use utoipa::openapi::schema::Schema;
use utoipa::openapi::{OpenApi, RefOr};
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouter};

use crate::openapi::{
    X_ERROR_CODES, X_PERMISSION, X_PLAN_FEATURE, operations, operations_mut, operations_of,
};

/// What a route needs from the caller: a permission and a plan feature, each optional. The
/// document states them from this value, and the guard that will enforce them reads the same
/// value, so what a route documents and what it enforces cannot differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    permission: Option<&'static str>,
    feature: Option<Feature>,
}

impl Access {
    /// Anyone may call the route: it needs no permission and no plan feature.
    pub const fn public() -> Self {
        Self {
            permission: None,
            feature: None,
        }
    }

    /// `x-permission`: the permission, or `public`.
    fn permission_code(self) -> &'static str {
        self.permission.unwrap_or("public")
    }

    /// `x-plan-feature`: the code of the feature, or `none`.
    fn feature_code(self) -> &'static str {
        self.feature.map_or("none", Feature::code)
    }
}

/// The only way to add a route to the public API. A route is made of handlers that carry
/// `#[utoipa::path]`, gathered by `routes!`, so the router and the OpenAPI document are built
/// from the same values.
///
/// A route can still be built by hand, as the tuple `routes!` makes. [`ApiRouter::route`]
/// refuses one the document cannot describe, and `HttpAppBuilder::build` then refuses the app;
/// a method the tuple serves and its document lacks is left to the test client's route guard.
///
/// Both fields are private, not `pub(crate)`: every handler lives in this crate, so with fields
/// its modules could set, any of them could build an `ApiRouter` around an axum router of its
/// own, whose routes the app would serve with no document, no error codes and no problem
/// recorded. No test here can show that such code fails to compile, hence this note.
pub struct ApiRouter<S = ()> {
    router: OpenApiRouter<S>,
    /// One line for each route refused and each schema name given two shapes.
    problems: Vec<String>,
}

/// The schema components of a document, by name.
type Schemas = BTreeMap<String, RefOr<Schema>>;

impl<S: Clone + Send + Sync + 'static> ApiRouter<S> {
    /// A router with no route, whose document holds nothing else: the title, the version and
    /// the codes common to every route are written once, at the root of the public document.
    pub fn new() -> Self {
        Self {
            // `OpenApiRouter::new()` would stamp utoipa-axum's own title, version and licence.
            router: OpenApiRouter::default(),
            problems: Vec::new(),
        }
    }

    /// Adds the handlers of `route`, made with `routes!` from handlers of one path.
    ///
    /// Every operation of the route gets `x-permission` and `x-plan-feature` from `access`,
    /// and `x-error-codes`: `error_codes`, the codes of its own the route can return, sorted,
    /// each once. These replace any key of the same name the handler declared itself.
    ///
    /// A route the document cannot describe is not added; a problem is recorded instead. That
    /// is a route that names other than exactly one path (utoipa-axum serves each handler of a
    /// `routes!` call on every path the call names), whose path documents no operation, that
    /// brings a schema under a name already registered with another shape (one of the two
    /// would document the routes of the other), or that brings a method its path already has
    /// here (axum would panic).
    pub fn route(
        mut self,
        route: UtoipaMethodRouter<S>,
        access: Access,
        error_codes: &'static [&'static str],
    ) -> Self {
        let (schemas, mut paths, method_router) = route;
        let problems = route_problems(&paths, &schemas, self.router.get_openapi());
        if !problems.is_empty() {
            self.problems.extend(problems);
            return self;
        }
        let mut codes = error_codes.to_vec();
        codes.sort_unstable();
        codes.dedup();
        for operation in paths.paths.values_mut().flat_map(operations_mut) {
            let extensions = ExtensionsBuilder::new()
                .add(X_PERMISSION, access.permission_code())
                .add(X_PLAN_FEATURE, access.feature_code())
                .add(X_ERROR_CODES, codes.clone())
                .build();
            match operation.extensions.as_mut() {
                Some(existing) => existing.merge(extensions),
                None => operation.extensions = Some(extensions),
            }
        }
        self.router = self.router.routes((schemas, paths, method_router));
        self
    }

    /// Adds every route of `other`, with its documentation and its problems. A schema name the
    /// two hold with different shapes is a problem. So is each method and path both document:
    /// axum would panic, so `other` is then not merged at all. Two templates that differ only
    /// in the names of their parameters still make axum panic.
    pub fn merge(mut self, other: Self) -> Self {
        let (ours, theirs) = (self.router.get_openapi(), other.router.get_openapi());
        for (name, schema) in schemas_of(theirs).into_iter().flatten() {
            if schemas_of(ours)
                .and_then(|ours| ours.get(name))
                .is_some_and(|ours| ours != schema)
            {
                let problem = format!("the merged routers hold two schemas named {name}");
                self.problems.push(problem);
            }
        }
        let in_both: Vec<String> = documented(ours)
            .intersection(&documented(theirs))
            .map(|(path, method)| format!("the merged routers both have {method} {path}"))
            .collect();
        let mergeable = in_both.is_empty();
        self.problems.extend(in_both);
        self.problems.extend(other.problems);
        if mergeable {
            self.router = self.router.merge(other.router);
        }
        self
    }

    /// Gives the routes their state.
    pub fn with_state<S2>(self, state: S) -> ApiRouter<S2> {
        ApiRouter {
            router: self.router.with_state(state),
            problems: self.problems,
        }
    }

    /// The routes with their document, or every problem recorded when the document cannot
    /// describe one of them: then nothing is served.
    pub(crate) fn into_router(self) -> Result<OpenApiRouter<S>, Vec<String>> {
        if self.problems.is_empty() {
            Ok(self.router)
        } else {
            Err(self.problems)
        }
    }
}

impl<S: Clone + Send + Sync + 'static> Default for ApiRouter<S> {
    fn default() -> Self {
        Self::new()
    }
}

fn schemas_of(document: &OpenApi) -> Option<&Schemas> {
    let components = document.components.as_ref()?;
    Some(&components.schemas)
}

/// Each path and method `document` has an operation for.
fn documented(document: &OpenApi) -> BTreeSet<(&str, &str)> {
    operations(document)
        .into_iter()
        .map(|(path, method, _)| (path, method))
        .collect()
}

/// Why the document cannot describe the route of `paths` and `schemas`, one line each, given
/// the document of the routes already `registered`.
fn route_problems(
    paths: &Paths,
    schemas: &[(String, RefOr<Schema>)],
    registered: &OpenApi,
) -> Vec<String> {
    let named: Vec<&str> = paths.paths.keys().map(String::as_str).collect();
    let mut problems = Vec::new();
    match named.as_slice() {
        [] => problems.push("a route names no path".to_owned()),
        [path]
            if paths
                .paths
                .values()
                .all(|item| operations_of(item).next().is_none()) =>
        {
            problems.push(format!("the route at {path} documents no operation"));
        }
        [_] => {}
        many => problems.push(format!(
            "a route names {} paths, {}: give routes! the handlers of one path",
            many.len(),
            many.join(" and ")
        )),
    }
    let taken = documented(registered);
    for (path, item) in &paths.paths {
        let again = operations_of(item).filter(|(method, _)| taken.contains(&(path, method)));
        problems.extend(again.map(|(method, _)| {
            format!("the route at {path} brings {method}, which another route already has")
        }));
    }
    let mut brought: BTreeMap<&str, &RefOr<Schema>> = BTreeMap::new();
    let mut clashes = BTreeSet::new();
    for (name, schema) in schemas {
        let earlier = schemas_of(registered)
            .and_then(|registered| registered.get(name))
            .or_else(|| brought.get(name.as_str()).copied());
        if earlier.is_some_and(|earlier| earlier != schema) {
            clashes.insert(name.as_str());
        }
        brought.insert(name, schema);
    }
    problems.extend(clashes.into_iter().map(|name| {
        let at = named.join(" and ");
        format!("the route at {at} brings a schema {name} unlike the one of that name")
    }));
    problems
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::State;
    use axum::http::Request;
    use axum::routing::{MethodRouter, get};
    use serde::Serialize;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use utoipa::ToSchema;
    use utoipa::openapi::path::{HttpMethod, Operation, PathItem, Paths};
    use utoipa::openapi::{Info, OpenApi};
    use utoipa_axum::routes;

    use super::*;
    use crate::envelope::{ApiResult, ApiSuccess, ErrorEnvelope, SuccessEnvelope};

    /// Another module's type of the same name as this one's, and a route that answers it.
    mod other {
        use super::*;

        /// What another module calls a fixture.
        #[derive(Serialize, ToSchema)]
        pub(super) struct Fixture {
            size: u8,
        }

        /// Read the other fixture.
        #[utoipa::path(
            get,
            path = "/others",
            tag = "fixtures",
            responses((status = 200, description = "The other", body = SuccessEnvelope<Fixture>)),
        )]
        pub(super) async fn read_other() -> ApiResult<Fixture> {
            Ok(ApiSuccess::ok("Other read", Fixture { size: 1 }))
        }

        /// Replace the fixture with the other module's.
        #[utoipa::path(
            put,
            path = "/fixtures",
            tag = "fixtures",
            responses((status = 200, description = "The other", body = SuccessEnvelope<Fixture>)),
        )]
        pub(super) async fn replace_with_other() -> ApiResult<Fixture> {
            Ok(ApiSuccess::ok("Fixture replaced", Fixture { size: 2 }))
        }
    }

    /// Create a widget.
    #[utoipa::path(
        post,
        path = "/widgets",
        tag = "fixtures",
        responses((status = 200, description = "The widget", body = SuccessEnvelope<Fixture>)),
    )]
    async fn create_widget() -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok("Widget created", Fixture { name: "widget" }))
    }

    /// The status `router` answers `method path` with, when nothing else serves it.
    async fn status_of(router: ApiRouter, method: &str, path: &str) -> u16 {
        let (router, _) = router.router.split_for_parts();
        let request = Request::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        let Ok(response) = router.oneshot(request).await;
        response.status().as_u16()
    }

    /// A route built by hand: `paths` documented, `GET` served.
    fn by_hand(paths: Paths) -> UtoipaMethodRouter {
        (Vec::new(), paths, get(read_fixture))
    }

    fn paths_of(document: &OpenApi) -> Vec<&str> {
        document.paths.paths.keys().map(String::as_str).collect()
    }

    /// What every fixture route answers.
    #[derive(Serialize, ToSchema)]
    struct Fixture {
        name: &'static str,
    }

    /// Read the fixture.
    #[utoipa::path(
        get,
        path = "/fixtures",
        tag = "fixtures",
        responses(
            (status = 200, description = "The fixture", body = SuccessEnvelope<Fixture>),
            (status = 404, description = "No fixture", body = ErrorEnvelope),
        ),
    )]
    async fn read_fixture() -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok("Fixture read", Fixture { name: "read" }))
    }

    /// Replace the fixture.
    #[utoipa::path(
        put,
        path = "/fixtures",
        tag = "fixtures",
        responses((status = 200, description = "The fixture", body = SuccessEnvelope<Fixture>)),
    )]
    async fn replace_fixture() -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok(
            "Fixture replaced",
            Fixture { name: "replaced" },
        ))
    }

    /// Read the owner of the fixture, declaring extensions of its own.
    #[utoipa::path(
        get,
        path = "/fixtures/owner",
        tag = "fixtures",
        extensions(("x-permission" = json!("admin")), ("x-team" = json!("goals"))),
        responses((status = 200, description = "The owner", body = SuccessEnvelope<Fixture>)),
    )]
    async fn read_owner() -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok("Owner read", Fixture { name: "owner" }))
    }

    /// Greet with the state of the router.
    #[utoipa::path(
        get,
        path = "/greeting",
        tag = "fixtures",
        responses((status = 200, description = "The greeting", body = SuccessEnvelope<Fixture>)),
    )]
    async fn greet(State(greeting): State<&'static str>) -> ApiResult<Fixture> {
        Ok(ApiSuccess::ok("Greeted", Fixture { name: greeting }))
    }

    fn document_of<S: Clone + Send + Sync + 'static>(router: ApiRouter<S>) -> OpenApi {
        router.router.into_openapi()
    }

    fn operation<'a>(document: &'a OpenApi, path: &str, method: HttpMethod) -> &'a Operation {
        document
            .paths
            .get_path_operation(path, method)
            .unwrap_or_else(|| panic!("the document has no operation at {path}"))
    }

    /// The three keys `route` writes, `null` where one is missing.
    fn metadata(operation: &Operation) -> Value {
        let extensions = operation.extensions.as_ref();
        let value = |key: &str| extensions.and_then(|extensions| extensions.get(key));
        json!({
            "x-permission": value("x-permission"),
            "x-plan-feature": value("x-plan-feature"),
            "x-error-codes": value("x-error-codes"),
        })
    }

    #[test]
    fn a_public_route_documents_public_none_and_its_own_codes() {
        let router: ApiRouter = ApiRouter::new().route(
            routes!(read_fixture),
            Access::public(),
            &["FIXTURE_GONE", "FIXTURE_LOCKED"],
        );

        let document = document_of(router);

        assert_eq!(
            metadata(operation(&document, "/fixtures", HttpMethod::Get)),
            json!({
                "x-permission": "public",
                "x-plan-feature": "none",
                "x-error-codes": ["FIXTURE_GONE", "FIXTURE_LOCKED"],
            })
        );
    }

    #[test]
    fn the_error_codes_are_written_sorted_and_each_once() {
        let router: ApiRouter = ApiRouter::new().route(
            routes!(read_fixture),
            Access::public(),
            &["FIXTURE_LOCKED", "FIXTURE_GONE", "FIXTURE_LOCKED"],
        );

        let document = document_of(router);

        assert_eq!(
            metadata(operation(&document, "/fixtures", HttpMethod::Get))["x-error-codes"],
            json!(["FIXTURE_GONE", "FIXTURE_LOCKED"])
        );
    }

    #[test]
    fn a_route_with_no_code_of_its_own_still_has_the_key() {
        let router: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);

        let document = document_of(router);

        assert_eq!(
            metadata(operation(&document, "/fixtures", HttpMethod::Get))["x-error-codes"],
            json!([])
        );
    }

    #[test]
    fn a_permission_and_a_plan_feature_are_written_as_their_codes() {
        let access = Access {
            permission: Some("goals.write"),
            feature: Some(Feature::FullHistory),
        };
        let router: ApiRouter = ApiRouter::new().route(routes!(read_fixture), access, &[]);

        let document = document_of(router);

        assert_eq!(
            metadata(operation(&document, "/fixtures", HttpMethod::Get)),
            json!({
                "x-permission": "goals.write",
                "x-plan-feature": "FULL_HISTORY",
                "x-error-codes": [],
            })
        );
    }

    #[test]
    fn every_operation_of_the_route_gets_the_metadata() {
        let router: ApiRouter = ApiRouter::new().route(
            routes!(read_fixture, replace_fixture),
            Access::public(),
            &["FIXTURE_GONE"],
        );

        let document = document_of(router);

        for (name, method) in [("GET", HttpMethod::Get), ("PUT", HttpMethod::Put)] {
            assert_eq!(
                metadata(operation(&document, "/fixtures", method)),
                json!({
                    "x-permission": "public",
                    "x-plan-feature": "none",
                    "x-error-codes": ["FIXTURE_GONE"],
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn operations_routes_never_make_get_the_metadata_too() {
        let mut item = PathItem::default();
        item.query = Some(Operation::new());
        item.additional_operations
            .insert("COPY".to_owned(), Operation::new());
        let mut paths = Paths::new();
        paths.paths.insert("/fixtures/search".to_owned(), item);
        let by_hand: UtoipaMethodRouter = (Vec::new(), paths, MethodRouter::new());

        let document = document_of(ApiRouter::new().route(by_hand, Access::public(), &[]));

        let item = &document.paths.paths["/fixtures/search"];
        let query = item.query.as_ref().unwrap();
        for operation in [query, &item.additional_operations["COPY"]] {
            assert_eq!(
                metadata(operation),
                json!({ "x-permission": "public", "x-plan-feature": "none", "x-error-codes": [] })
            );
        }
    }

    #[test]
    fn the_typed_values_replace_what_the_handler_declared_and_its_other_keys_stay() {
        let router: ApiRouter = ApiRouter::new().route(routes!(read_owner), Access::public(), &[]);

        let document = document_of(router);

        let owner = operation(&document, "/fixtures/owner", HttpMethod::Get);
        assert_eq!(metadata(owner)["x-permission"], "public");
        let extensions = owner.extensions.as_ref().unwrap();
        assert_eq!(extensions.get("x-team"), Some(&json!("goals")));
    }

    #[test]
    fn the_schemas_of_a_route_are_registered_in_the_document() {
        let router: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);

        let document = document_of(router);

        let schemas = &document.components.unwrap().schemas;
        let names: Vec<&str> = schemas.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "ErrorDetail",
                "ErrorEnvelope",
                "Fixture",
                "SuccessEnvelope_Fixture"
            ]
        );
    }

    #[test]
    fn merge_keeps_the_documented_routes_of_both() {
        let fixtures: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &["FIXTURE_GONE"]);
        let owners: ApiRouter = ApiRouter::new().route(routes!(read_owner), Access::public(), &[]);

        let document = document_of(fixtures.merge(owners));

        let paths: Vec<&str> = document.paths.paths.keys().map(String::as_str).collect();
        assert_eq!(paths, ["/fixtures", "/fixtures/owner"]);
        let read = operation(&document, "/fixtures", HttpMethod::Get);
        assert_eq!(metadata(read)["x-error-codes"], json!(["FIXTURE_GONE"]));
        let owner = operation(&document, "/fixtures/owner", HttpMethod::Get);
        assert_eq!(metadata(owner)["x-permission"], "public");
    }

    #[test]
    fn with_state_keeps_the_documented_routes() {
        let router: ApiRouter = ApiRouter::<&'static str>::new()
            .route(routes!(greet), Access::public(), &[])
            .with_state("hello");

        let document = document_of(router);

        let greeting = operation(&document, "/greeting", HttpMethod::Get);
        assert_eq!(metadata(greeting)["x-plan-feature"], "none");
    }

    #[test]
    fn a_new_router_takes_nothing_from_utoipa_axum_metadata() {
        let document = document_of(ApiRouter::<()>::new());

        assert!(
            document.info == Info::default(),
            "{:?}",
            document.info.title
        );
    }

    #[test]
    fn a_default_router_is_a_new_one() {
        let router = ApiRouter::<()>::default();

        assert!(router.problems.is_empty());
        assert!(document_of(router).paths.paths.is_empty());
    }

    fn two_paths() -> ApiRouter {
        ApiRouter::new().route(routes!(read_fixture, create_widget), Access::public(), &[])
    }

    #[tokio::test]
    async fn a_route_naming_two_paths_is_a_problem_and_neither_is_served_or_documented() {
        let router = two_paths();

        assert_eq!(
            router.problems,
            [
                "a route names 2 paths, /fixtures and /widgets: give routes! the handlers of one path"
            ]
        );
        assert!(document_of(two_paths()).paths.paths.is_empty());
        for (method, path) in [
            ("GET", "/fixtures"),
            ("POST", "/widgets"),
            ("GET", "/widgets"),
        ] {
            assert_eq!(
                status_of(two_paths(), method, path).await,
                404,
                "{method} {path}"
            );
        }
    }

    #[test]
    fn the_routes_are_given_out_only_when_no_problem_was_recorded() {
        let fine: ApiRouter = ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);
        let refused = two_paths().route(routes!(read_owner), Access::public(), &[]);

        let Ok(routes) = fine.into_router() else {
            panic!("a router without a problem gave no routes");
        };
        assert_eq!(paths_of(routes.get_openapi()), ["/fixtures"]);
        assert_eq!(
            refused.into_router().err(),
            Some(vec![
                "a route names 2 paths, /fixtures and /widgets: give routes! the handlers of one path"
                    .to_owned()
            ])
        );
    }

    #[test]
    fn a_route_naming_no_path_is_a_problem() {
        let router: ApiRouter =
            ApiRouter::new().route(by_hand(Paths::new()), Access::public(), &[]);

        assert_eq!(router.problems, ["a route names no path"]);
    }

    fn operationless() -> ApiRouter {
        let mut paths = Paths::new();
        paths
            .paths
            .insert("/hidden".to_owned(), PathItem::default());
        ApiRouter::new().route(by_hand(paths), Access::public(), &[])
    }

    #[tokio::test]
    async fn a_path_with_no_operation_is_a_problem_and_is_not_served() {
        let router = operationless();

        assert_eq!(
            router.problems,
            ["the route at /hidden documents no operation"]
        );
        assert!(document_of(operationless()).paths.paths.is_empty());
        assert_eq!(status_of(operationless(), "GET", "/hidden").await, 404);
    }

    #[test]
    fn a_problem_does_not_stop_the_routes_after_it() {
        let router = two_paths().route(routes!(read_owner), Access::public(), &[]);

        assert_eq!(router.problems.len(), 1);
        assert_eq!(paths_of(&document_of(router)), ["/fixtures/owner"]);
    }

    #[test]
    fn merge_carries_the_problems_of_both_routers() {
        let merged = two_paths().merge(operationless());

        assert_eq!(
            merged.problems,
            [
                "a route names 2 paths, /fixtures and /widgets: give routes! the handlers of one path",
                "the route at /hidden documents no operation",
            ]
        );
    }

    #[test]
    fn with_state_keeps_the_problems() {
        let pathless = (Vec::new(), Paths::new(), MethodRouter::new());
        let router: ApiRouter = ApiRouter::<&'static str>::new()
            .route(pathless, Access::public(), &[])
            .with_state("hello");

        assert_eq!(router.problems, ["a route names no path"]);
    }

    #[test]
    fn a_schema_name_taken_by_another_shape_is_a_problem_and_the_route_is_not_added() {
        let router: ApiRouter = ApiRouter::new()
            .route(routes!(read_fixture), Access::public(), &[])
            .route(routes!(other::read_other), Access::public(), &[]);

        assert_eq!(
            router.problems,
            [
                "the route at /others brings a schema Fixture unlike the one of that name",
                "the route at /others brings a schema SuccessEnvelope_Fixture unlike the one of that name",
            ]
        );
        assert_eq!(paths_of(&document_of(router)), ["/fixtures"]);
    }

    #[test]
    fn handlers_of_one_route_that_bring_two_shapes_of_a_name_are_a_problem() {
        let router: ApiRouter = ApiRouter::new().route(
            routes!(read_fixture, other::replace_with_other),
            Access::public(),
            &[],
        );

        assert_eq!(
            router.problems,
            [
                "the route at /fixtures brings a schema Fixture unlike the one of that name",
                "the route at /fixtures brings a schema SuccessEnvelope_Fixture unlike the one of that name",
            ]
        );
        assert!(document_of(router).paths.paths.is_empty());
    }

    #[test]
    fn merge_finds_a_schema_name_with_two_shapes() {
        let fixtures: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);
        let others: ApiRouter =
            ApiRouter::new().route(routes!(other::read_other), Access::public(), &[]);

        let merged = fixtures.merge(others);

        assert_eq!(
            merged.problems,
            [
                "the merged routers hold two schemas named Fixture",
                "the merged routers hold two schemas named SuccessEnvelope_Fixture",
            ]
        );
    }

    /// `GET /fixtures`, then `PUT /fixtures`, then `GET /fixtures` again.
    fn get_put_get() -> ApiRouter {
        ApiRouter::new()
            .route(routes!(read_fixture), Access::public(), &[])
            .route(routes!(replace_fixture), Access::public(), &[])
            .route(routes!(read_fixture), Access::public(), &[])
    }

    #[tokio::test]
    async fn a_method_its_path_already_has_is_a_problem_and_another_method_is_not() {
        let router = get_put_get();

        assert_eq!(
            router.problems,
            ["the route at /fixtures brings GET, which another route already has"]
        );
        for method in ["GET", "PUT"] {
            assert_eq!(status_of(get_put_get(), method, "/fixtures").await, 200);
        }
    }

    #[test]
    fn routers_merged_with_a_method_and_path_in_common_are_a_problem() {
        let fixtures: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);
        let both: ApiRouter = ApiRouter::new().route(
            routes!(read_fixture, replace_fixture),
            Access::public(),
            &[],
        );

        let merged = fixtures.merge(both);

        assert_eq!(
            merged.problems,
            ["the merged routers both have GET /fixtures"]
        );
    }

    #[test]
    fn routes_that_share_a_schema_are_no_problem() {
        let together: ApiRouter = ApiRouter::new()
            .route(routes!(read_fixture), Access::public(), &[])
            .route(routes!(read_owner), Access::public(), &[]);
        let fixtures: ApiRouter =
            ApiRouter::new().route(routes!(read_fixture), Access::public(), &[]);
        let owners: ApiRouter = ApiRouter::new().route(routes!(read_owner), Access::public(), &[]);

        let merged = fixtures.merge(owners);

        assert!(together.problems.is_empty(), "{:?}", together.problems);
        assert!(merged.problems.is_empty(), "{:?}", merged.problems);
    }
}
