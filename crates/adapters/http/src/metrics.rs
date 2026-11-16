//! The metrics of spec §6.6, in a registry of their own that the composition root creates
//! and hands over: nothing is global.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use axum::http::StatusCode;
use prometheus::{
    HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};

use crate::app::ServiceInfo;

/// The edges of the duration histogram, in seconds. The budgets of ADR-0014 (80, 150 and
/// 200 ms) are edges, so the share of requests over a budget reads straight off a bucket.
const DURATION_BUCKETS: [f64; 12] = [
    0.005, 0.01, 0.025, 0.05, 0.08, 0.1, 0.15, 0.2, 0.5, 1.0, 2.5, 5.0,
];

/// The metrics of one binary. Its clones record into the same registry; two values made by
/// [`Metrics::new`] share nothing, so neither sees what the other counted.
#[derive(Debug, Clone)]
pub struct Metrics {
    service: ServiceInfo,
    registry: Registry,
    requests: IntCounterVec,
    durations: HistogramVec,
}

impl Metrics {
    /// A registry with `http_requests_total`, `http_request_duration_seconds` and
    /// `barras_build_info`, which is 1 under the name and the version of `service`.
    pub fn new(service: ServiceInfo) -> Result<Self, MetricsError> {
        let requests = IntCounterVec::new(
            Opts::new(
                "http_requests_total",
                "HTTP requests answered, by method, route template and status.",
            ),
            &["method", "path", "status"],
        )
        .map_err(MetricsError::register)?;
        let durations = HistogramVec::new(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "Time from an HTTP request to the head of its answer, in seconds, by method and \
                 route template.",
            )
            .buckets(DURATION_BUCKETS.to_vec()),
            &["method", "path"],
        )
        .map_err(MetricsError::register)?;
        let build = IntGauge::with_opts(
            Opts::new(
                "barras_build_info",
                "The service and the version of the running binary; always 1.",
            )
            .const_label("service", service.name)
            .const_label("version", service.version),
        )
        .map_err(MetricsError::register)?;
        build.set(1);
        let registry = Registry::new();
        registry
            .register(Box::new(requests.clone()))
            .map_err(MetricsError::register)?;
        registry
            .register(Box::new(durations.clone()))
            .map_err(MetricsError::register)?;
        registry
            .register(Box::new(build))
            .map_err(MetricsError::register)?;
        Ok(Self {
            service,
            registry,
            requests,
            durations,
        })
    }

    /// The binary the metrics were made for, as `barras_build_info` names it: the apps built
    /// with these metrics name themselves the same way on their internal port.
    pub fn service(&self) -> ServiceInfo {
        self.service
    }

    /// Every metric, in the Prometheus text format (`prometheus::TEXT_FORMAT`).
    pub fn render(&self) -> Result<String, MetricsError> {
        TextEncoder::new()
            .encode_to_string(&self.registry.gather())
            .map_err(MetricsError::render)
    }

    /// Counts and times one answered request. A failure is logged and dropped: the metrics
    /// never cost a caller the answer.
    pub(crate) fn record_request(
        &self,
        method: &str,
        path: &str,
        status: StatusCode,
        elapsed: Duration,
    ) {
        let counted = self
            .requests
            .get_metric_with_label_values(&[method, path, status.as_str()])
            .map(|counter| counter.inc());
        let timed = self
            .durations
            .get_metric_with_label_values(&[method, path])
            .map(|histogram| histogram.observe(elapsed.as_secs_f64()));
        if let Err(error) = counted.and(timed) {
            // The error says how many labels were given, never their values.
            tracing::warn!(error = %error, "a request could not be recorded in the metrics");
        }
    }

    /// Metrics whose two vectors expect a label the recording never gives, so that every
    /// recording fails: what the HTTP layer must survive.
    #[cfg(test)]
    pub(crate) fn unrecordable(service: ServiceInfo) -> Result<Self, MetricsError> {
        let mut metrics = Self::new(service)?;
        metrics.requests = IntCounterVec::new(
            Opts::new("unrecordable_total", "Never recorded."),
            &["unknown"],
        )
        .map_err(MetricsError::register)?;
        metrics.durations = HistogramVec::new(
            HistogramOpts::new("unrecordable_seconds", "Never recorded."),
            &["unknown"],
        )
        .map_err(MetricsError::register)?;
        Ok(metrics)
    }

    /// Metrics whose registry also holds a family with a sample and no name, which the text
    /// format refuses, so that every rendering fails: what the internal port must survive.
    #[cfg(test)]
    pub(crate) fn unrenderable(service: ServiceInfo) -> Result<Self, MetricsError> {
        use std::collections::HashMap;

        use prometheus::core::{Collector, Desc};
        use prometheus::proto::{Metric, MetricFamily};

        struct Nameless(Desc);

        impl Collector for Nameless {
            fn desc(&self) -> Vec<&Desc> {
                vec![&self.0]
            }

            fn collect(&self) -> Vec<MetricFamily> {
                let mut family = MetricFamily::default();
                family.mut_metric().push(Metric::default());
                vec![family]
            }
        }

        let metrics = Self::new(service)?;
        let desc = Desc::new(
            "nameless".to_owned(),
            "Never rendered.".to_owned(),
            Vec::new(),
            HashMap::new(),
        )
        .map_err(MetricsError::register)?;
        metrics
            .registry
            .register(Box::new(Nameless(desc)))
            .map_err(MetricsError::register)?;
        Ok(metrics)
    }
}

/// The metrics could not be registered or written out: a fault of this code, never of a
/// request. The message names the step; the cause is the [`source`](Error::source).
#[derive(Debug)]
pub struct MetricsError {
    step: &'static str,
    source: prometheus::Error,
}

impl MetricsError {
    fn register(source: prometheus::Error) -> Self {
        Self {
            step: "the metrics could not be registered",
            source,
        }
    }

    fn render(source: prometheus::Error) -> Self {
        Self {
            step: "the metrics could not be written as text",
            source,
        }
    }
}

impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.step)
    }
}

impl Error for MetricsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: ServiceInfo = ServiceInfo {
        name: "barras-probe",
        version: "4.5.6",
    };

    /// The samples of `text` named exactly `name`, without the `# HELP` and `# TYPE` lines.
    fn samples<'text>(text: &'text str, name: &str) -> Vec<&'text str> {
        text.lines()
            .filter(|line| line.split(['{', ' ']).next() == Some(name))
            .collect()
    }

    #[test]
    fn a_new_registry_holds_only_the_build_info() {
        let metrics = Metrics::new(SERVICE).unwrap();

        let text = metrics.render().unwrap();

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].starts_with("# HELP barras_build_info "), "{text}");
        assert_eq!(lines[1], "# TYPE barras_build_info gauge");
        assert_eq!(
            lines[2],
            r#"barras_build_info{service="barras-probe",version="4.5.6"} 1"#
        );
    }

    #[test]
    fn the_metrics_know_the_service_they_were_made_for() {
        let metrics = Metrics::new(SERVICE).unwrap();

        assert_eq!(metrics.service(), SERVICE);
        assert_eq!(metrics.clone().service(), SERVICE);
    }

    #[test]
    fn a_request_is_counted_under_its_labels_and_timed_under_its_route() {
        let metrics = Metrics::new(SERVICE).unwrap();
        let route = "/api/v1/goals/{id}";

        metrics.record_request("GET", route, StatusCode::OK, Duration::from_millis(30));
        metrics.record_request("GET", route, StatusCode::OK, Duration::from_millis(90));
        metrics.record_request(
            "GET",
            route,
            StatusCode::NOT_FOUND,
            Duration::from_millis(3),
        );

        let text = metrics.render().unwrap();
        assert_eq!(
            samples(&text, "http_requests_total"),
            [
                r#"http_requests_total{method="GET",path="/api/v1/goals/{id}",status="200"} 2"#,
                r#"http_requests_total{method="GET",path="/api/v1/goals/{id}",status="404"} 1"#,
            ]
        );
        let buckets = samples(&text, "http_request_duration_seconds_bucket");
        for expected in [
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/goals/{id}",le="0.005"} 1"#,
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/goals/{id}",le="0.025"} 1"#,
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/goals/{id}",le="0.05"} 2"#,
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/goals/{id}",le="0.08"} 2"#,
            r#"http_request_duration_seconds_bucket{method="GET",path="/api/v1/goals/{id}",le="0.1"} 3"#,
        ] {
            assert!(buckets.contains(&expected), "{expected}\n{text}");
        }
        assert_eq!(
            samples(&text, "http_request_duration_seconds_count"),
            [r#"http_request_duration_seconds_count{method="GET",path="/api/v1/goals/{id}"} 3"#]
        );
    }

    #[test]
    fn the_duration_is_said_to_run_to_the_head_of_the_answer() {
        let metrics = Metrics::new(SERVICE).unwrap();
        metrics.record_request("GET", "/api/v1/goals", StatusCode::OK, Duration::ZERO);

        let text = metrics.render().unwrap();

        let help = "# HELP http_request_duration_seconds Time from an HTTP request to the head \
                    of its answer, in seconds, by method and route template.";
        assert!(text.lines().any(|line| line == help), "{text}");
    }

    #[test]
    fn an_error_names_its_step_and_leaves_the_cause_to_its_source() {
        for error in [
            MetricsError::register(prometheus::Error::AlreadyReg),
            MetricsError::render(prometheus::Error::AlreadyReg),
        ] {
            let cause = error.source().unwrap();

            assert!(cause.downcast_ref::<prometheus::Error>().is_some());
            assert!(!error.to_string().is_empty());
            // A chain printed whole would otherwise say the cause twice.
            assert!(!error.to_string().contains(&cause.to_string()), "{error}");
        }
    }
}
