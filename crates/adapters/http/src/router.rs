//! The door to the public API: a route is a documented handler registered together with what
//! it needs from the caller and the error codes of its own it can return.

use barras_kernel::Feature;
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa_axum::router::{OpenApiRouter, UtoipaMethodRouter};

use crate::openapi::{X_ERROR_CODES, X_PERMISSION, X_PLAN_FEATURE, operations_mut};

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
/// from the same values. There is no way to add a bare axum route.
pub struct ApiRouter<S = ()>(pub(crate) OpenApiRouter<S>);

impl<S: Clone + Send + Sync + 'static> ApiRouter<S> {
    /// A router with no route, whose document holds nothing else: the title, the version and
    /// the codes common to every route are written once, at the root of the public document.
    pub fn new() -> Self {
        // `OpenApiRouter::new()` would stamp utoipa-axum's own title, version and licence.
        Self(OpenApiRouter::default())
    }

    /// Adds the handlers of `route`, made with `routes!` from handlers of one path: utoipa-axum
    /// serves each handler of the call on every path the call names.
    ///
    /// Every operation of the route gets `x-permission` and `x-plan-feature` from `access`,
    /// and `x-error-codes`: `error_codes`, the codes of its own the route can return, sorted,
    /// each once. These replace any key of the same name the handler declared itself.
    pub fn route(
        self,
        route: UtoipaMethodRouter<S>,
        access: Access,
        error_codes: &'static [&'static str],
    ) -> Self {
        let (schemas, mut paths, method_router) = route;
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
        Self(self.0.routes((schemas, paths, method_router)))
    }

    /// Adds every route of `other`, with its documentation. A method and path both routers
    /// serve makes axum panic, as it does for any two routes that overlap.
    pub fn merge(self, other: Self) -> Self {
        Self(self.0.merge(other.0))
    }

    /// Gives the routes their state.
    pub fn with_state<S2>(self, state: S) -> ApiRouter<S2> {
        ApiRouter(self.0.with_state(state))
    }
}

impl<S: Clone + Send + Sync + 'static> Default for ApiRouter<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::State;
    use axum::routing::MethodRouter;
    use serde::Serialize;
    use serde_json::{Value, json};
    use utoipa::ToSchema;
    use utoipa::openapi::path::{HttpMethod, Operation, PathItem, Paths};
    use utoipa::openapi::{Info, OpenApi};
    use utoipa_axum::routes;

    use super::*;
    use crate::envelope::{ApiResult, ApiSuccess, ErrorEnvelope, SuccessEnvelope};

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
        router.0.into_openapi()
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
}
