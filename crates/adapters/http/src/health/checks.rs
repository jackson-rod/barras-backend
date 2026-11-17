//! Readiness shared (BAR-4 spec §6.3): one run of the probes for every request that comes while
//! it goes, on the public port and the internal one, and its reports given again for a second,
//! so that however often the public route is asked, the dependencies see at most one run a
//! second.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use super::probe::{HealthProbe, ProbeReport, check_all};

/// How long the reports of a run are given again once it has ended.
const FRESH_FOR: Duration = Duration::from_secs(1);

/// The probes of an app, and the reports of their last run.
#[derive(Clone)]
pub(crate) struct HealthChecks {
    probes: Arc<[HealthProbe]>,
    last_run: Arc<Mutex<Option<Run>>>,
}

/// What one run of every probe reported, and when it ended.
struct Run {
    ended: Instant,
    reports: Arc<[ProbeReport]>,
}

impl HealthChecks {
    pub(crate) fn new(probes: Vec<HealthProbe>) -> Self {
        Self {
            probes: probes.into(),
            last_run: Arc::new(Mutex::new(None)),
        }
    }

    /// The reports of the last run when it ended less than a second ago, those of a new run
    /// otherwise.
    pub(crate) async fn reports(&self) -> Arc<[ProbeReport]> {
        let mut last_run = Arc::clone(&self.last_run).lock_owned().await;
        if let Some(run) = last_run.as_ref()
            && run.ended.elapsed() < FRESH_FOR
        {
            return Arc::clone(&run.reports);
        }
        let probes = Arc::clone(&self.probes);
        // A task of its own, which keeps the lock until it has recorded its reports: a request
        // that comes meanwhile waits for them rather than starting a run, and the run goes on
        // when its own request goes away, as when the client closes its connection, so that
        // leaving starts runs no faster than waiting does.
        let run = tokio::spawn(async move {
            let reports: Arc<[ProbeReport]> = check_all(&probes).await.into();
            *last_run = Some(Run {
                ended: Instant::now(),
                reports: Arc::clone(&reports),
            });
            reports
        });
        match run.await {
            Ok(reports) => reports,
            // Only a runtime that shuts down stops the task early, since `check_all` answers
            // even for a check that panics; the probes then run here, unrecorded.
            Err(_stopped) => check_all(&self.probes).await.into(),
        }
    }
}
