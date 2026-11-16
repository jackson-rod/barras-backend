//! A probe of one dependency, and readiness: every probe run at once, each with two seconds to
//! answer, and what their answers say of the service as a whole.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout};

/// How long a probe has to answer before it counts as failed (spec §5).
const PROBE_LIMIT: Duration = Duration::from_secs(2);

/// The error of a probe whose check panicked: its task ended without an answer.
const STOPPED: &str = "stopped without an answer";

/// Whether the service can work while the dependency a probe checks is down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    /// Down, the service cannot take traffic: readiness answers 503.
    Critical,
    /// Down, the service still works: readiness answers `degraded`.
    Optional,
}

/// One run of a check.
type CheckRun = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// A check of one dependency: a name for the internal port, whether the service can do without
/// the dependency, and an async check that answers `Ok` when it is up, or why it is not.
#[derive(Clone)]
pub struct HealthProbe {
    name: &'static str,
    criticality: Criticality,
    check: Arc<dyn Fn() -> CheckRun + Send + Sync>,
}

impl HealthProbe {
    /// `check` runs once per readiness request, at the same time as every other probe, and has
    /// two seconds to answer. The name and the error show on the internal port and in the log,
    /// never on the public port; neither may hold a credential.
    pub fn new<F, Fut>(name: &'static str, criticality: Criticality, check: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        let check = move || -> CheckRun { Box::pin(check()) };
        Self {
            name,
            criticality,
            check: Arc::new(check),
        }
    }

    /// One run of the check, timed, failed when it does not answer within the limit.
    fn timed_check(&self) -> impl Future<Output = (Duration, Result<(), String>)> + Send + 'static {
        let check = (self.check)();
        async move {
            let started = Instant::now();
            let outcome = timeout(PROBE_LIMIT, check).await.unwrap_or_else(|_| {
                let limit = PROBE_LIMIT.as_secs();
                Err(format!("did not answer within {limit} seconds"))
            });
            (started.elapsed(), outcome)
        }
    }
}

/// What one probe answered, and how long it took.
pub(crate) struct ProbeReport {
    pub(crate) name: &'static str,
    pub(crate) critical: bool,
    pub(crate) latency: Duration,
    /// Why the dependency is down; `None` when it is up.
    pub(crate) error: Option<String>,
}

/// Runs every probe at once, each in a task of its own and limited to two seconds, and reports
/// on each in the order given. A probe that does not answer in time, or whose check panics,
/// counts as failed. Dropping the future stops every check still running.
pub(crate) async fn check_all(probes: &[HealthProbe]) -> Vec<ProbeReport> {
    let started = Instant::now();
    let mut running = JoinSet::new();
    for (index, probe) in probes.iter().enumerate() {
        let check = probe.timed_check();
        running.spawn(async move { (index, check.await) });
    }
    let mut answers: Vec<Option<(Duration, Result<(), String>)>> =
        probes.iter().map(|_| None).collect();
    while let Some(joined) = running.join_next().await {
        // A check that panicked leaves its answer empty.
        if let Ok((index, answer)) = joined
            && let Some(slot) = answers.get_mut(index)
        {
            *slot = Some(answer);
        }
    }
    let waited = started.elapsed();
    probes
        .iter()
        .zip(answers)
        .map(|(probe, answer)| {
            // A check that panicked took no longer than the whole wait, the one time known.
            let (latency, outcome) = answer.unwrap_or_else(|| (waited, Err(STOPPED.to_owned())));
            ProbeReport {
                name: probe.name,
                critical: probe.criticality == Criticality::Critical,
                latency,
                error: outcome.err(),
            }
        })
        .collect()
}

/// What the probes say of the service as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Readiness {
    /// Every probe passed, or there is none.
    Ok,
    /// Only probes of optional dependencies failed.
    Degraded,
    /// A probe of a critical dependency failed: the service cannot take traffic.
    Unavailable,
}

impl Readiness {
    pub(crate) fn of(reports: &[ProbeReport]) -> Self {
        let mut readiness = Self::Ok;
        for report in reports.iter().filter(|report| report.error.is_some()) {
            if report.critical {
                return Self::Unavailable;
            }
            readiness = Self::Degraded;
        }
        readiness
    }
}
