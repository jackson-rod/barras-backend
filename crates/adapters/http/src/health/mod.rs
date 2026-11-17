//! Health (spec §5): the probes of the dependencies, the two public routes, and the readiness
//! that both they and the internal port answer from.

mod checks;
mod dto;
mod probe;
mod routes;

pub(crate) use checks::HealthChecks;
pub use probe::{Criticality, HealthProbe};
pub(crate) use probe::{ProbeReport, Readiness};
pub(crate) use routes::health_routes;
