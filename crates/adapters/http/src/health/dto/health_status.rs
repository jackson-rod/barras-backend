//! The `data` of both public health routes.

use serde::Serialize;
use utoipa::ToSchema;

/// How the service stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HealthStatus {
    // Described on its type: utoipa would wrap an inline schema that has a description of its
    // own in an `allOf` with an object, which no string matches.
    #[schema(inline)]
    status: ServiceStatus,
}

/// `ok`, or `degraded` while a dependency the service can do without is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
enum ServiceStatus {
    Ok,
    Degraded,
}

impl HealthStatus {
    pub(crate) const OK: Self = Self {
        status: ServiceStatus::Ok,
    };
    pub(crate) const DEGRADED: Self = Self {
        status: ServiceStatus::Degraded,
    };
}
