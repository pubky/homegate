//! Prometheus metrics recorder shared by every verification route.
//!
//! Routes and the shared signup issuer *record* into a cloned [`Metrics`];
//! `infrastructure::http::metrics_server` *serves* it. Each instance owns its
//! own registry, so tests can create one per test without clashing.
//!
//! Failures are classified once per route from its error enum (see each
//! route's `error.rs`) and recorded by the HTTP handler. Positive events
//! (started, issued, bypassed, sats received, homeserver latency) are
//! recorded where they happen.

use std::sync::Arc;
use std::time::Instant;

use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGaugeVec, Opts, Registry,
    TextEncoder, core::Collector,
};

/// The verification routes, used as the `type` label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignupType {
    Sms,
    Lightning,
    Ip,
    Google,
}

impl SignupType {
    pub const ALL: [SignupType; 4] = [
        SignupType::Sms,
        SignupType::Lightning,
        SignupType::Ip,
        SignupType::Google,
    ];

    /// Routes with a distinct "started" step before a code can be issued.
    pub const WITH_START_EVENT: [SignupType; 2] = [SignupType::Sms, SignupType::Lightning];

    pub fn as_str(self) -> &'static str {
        match self {
            SignupType::Sms => "sms",
            SignupType::Lightning => "lightning",
            SignupType::Ip => "ip",
            SignupType::Google => "google",
        }
    }
}

/// Upstream services homegate depends on, used as the `dependency` label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dependency {
    Homeserver,
    Prelude,
    Phoenixd,
    GoogleJwks,
}

impl Dependency {
    pub const ALL: [Dependency; 4] = [
        Dependency::Homeserver,
        Dependency::Prelude,
        Dependency::Phoenixd,
        Dependency::GoogleJwks,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Dependency::Homeserver => "homeserver",
            Dependency::Prelude => "prelude",
            Dependency::Phoenixd => "phoenixd",
            Dependency::GoogleJwks => "google_jwks",
        }
    }
}

/// Why a verification request was turned down, used as the `reason` label.
///
/// A closed set keeps label cardinality bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectionReason {
    WeeklyLimitExceeded,
    AnnualLimitExceeded,
    InvalidGoogleIdToken,
    InvalidPhoneNumber,
    IpAddressRequired,
    Blocked,
    PreludeRateLimited,
    NoActiveVerification,
    MaxValidationAttemptsExceeded,
    InvalidCode,
}

impl RejectionReason {
    /// The reasons a route can emit, used to pre-create its series.
    pub fn for_signup_type(signup_type: SignupType) -> &'static [RejectionReason] {
        match signup_type {
            SignupType::Sms => &[
                RejectionReason::WeeklyLimitExceeded,
                RejectionReason::AnnualLimitExceeded,
                RejectionReason::InvalidPhoneNumber,
                RejectionReason::Blocked,
                RejectionReason::PreludeRateLimited,
                RejectionReason::NoActiveVerification,
                RejectionReason::MaxValidationAttemptsExceeded,
                RejectionReason::InvalidCode,
            ],
            SignupType::Lightning => &[],
            SignupType::Ip => &[
                RejectionReason::WeeklyLimitExceeded,
                RejectionReason::AnnualLimitExceeded,
                RejectionReason::IpAddressRequired,
            ],
            SignupType::Google => &[
                RejectionReason::WeeklyLimitExceeded,
                RejectionReason::AnnualLimitExceeded,
                RejectionReason::InvalidGoogleIdToken,
            ],
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RejectionReason::WeeklyLimitExceeded => "weekly_limit_exceeded",
            RejectionReason::AnnualLimitExceeded => "annual_limit_exceeded",
            RejectionReason::InvalidGoogleIdToken => "invalid_google_id_token",
            RejectionReason::InvalidPhoneNumber => "invalid_phone_number",
            RejectionReason::IpAddressRequired => "ip_address_required",
            RejectionReason::Blocked => "blocked",
            RejectionReason::PreludeRateLimited => "prelude_rate_limited",
            RejectionReason::NoActiveVerification => "no_active_verification",
            RejectionReason::MaxValidationAttemptsExceeded => "max_validation_attempts_exceeded",
            RejectionReason::InvalidCode => "invalid_code",
        }
    }
}

/// How a route-level error is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// The request was refused for a reason attributable to the client.
    Rejected(RejectionReason),
    /// An upstream service failed.
    DependencyError(Dependency),
    /// Internal failure (e.g. database); not counted by these metrics.
    Internal,
}

const SIGNUP_CODES_ISSUED: &str = "homegate_signup_codes_issued_total";
const VERIFICATION_REJECTED: &str = "homegate_verification_rejected_total";
const DEPENDENCY_ERRORS: &str = "homegate_dependency_errors_total";
const VERIFICATION_STARTED: &str = "homegate_verification_started_total";
const LIMIT_BYPASSED: &str = "homegate_limit_bypassed_total";
const LN_SATS_RECEIVED: &str = "homegate_ln_sats_received_total";
const HOMESERVER_REQUEST_DURATION: &str = "homegate_homeserver_request_duration_seconds";
const BUILD_INFO: &str = "homegate_build_info";
const ROUTE_ENABLED: &str = "homegate_route_enabled";

#[derive(Clone, Debug)]
pub struct Metrics {
    registry: Arc<Registry>,
    signup_codes_issued: IntCounterVec,
    verification_rejected: IntCounterVec,
    dependency_errors: IntCounterVec,
    verification_started: IntCounterVec,
    limit_bypassed: IntCounterVec,
    ln_sats_received: IntCounter,
    homeserver_request_duration: Histogram,
    build_info: IntGaugeVec,
    route_enabled: IntGaugeVec,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let counter_vec = |name, help, labels: &[&str]| {
            register(
                &registry,
                IntCounterVec::new(Opts::new(name, help), labels).expect("valid definition"),
            )
        };
        let gauge_vec = |name, help, labels: &[&str]| {
            register(
                &registry,
                IntGaugeVec::new(Opts::new(name, help), labels).expect("valid definition"),
            )
        };

        let metrics = Self {
            signup_codes_issued: counter_vec(
                SIGNUP_CODES_ISSUED,
                "Signup codes issued, per verification type.",
                &["type"],
            ),
            verification_rejected: counter_vec(
                VERIFICATION_REJECTED,
                "Verification requests refused, per verification type and reason.",
                &["type", "reason"],
            ),
            dependency_errors: counter_vec(
                DEPENDENCY_ERRORS,
                "Failed calls to upstream services, per dependency.",
                &["dependency"],
            ),
            verification_started: counter_vec(
                VERIFICATION_STARTED,
                "Multi-step verifications started (SMS code sent, Lightning invoice created), per verification type.",
                &["type"],
            ),
            limit_bypassed: counter_vec(
                LIMIT_BYPASSED,
                "Verifications where rate limits were skipped because the identity is whitelisted, per verification type.",
                &["type"],
            ),
            ln_sats_received: register(
                &registry,
                IntCounter::new(
                    LN_SATS_RECEIVED,
                    "Satoshis received for finalised Lightning verifications.",
                )
                .expect("valid definition"),
            ),
            homeserver_request_duration: register(
                &registry,
                Histogram::with_opts(
                    HistogramOpts::new(
                        HOMESERVER_REQUEST_DURATION,
                        "Duration of signup token requests to the homeserver admin API in seconds.",
                    )
                    .buckets(vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
                )
                .expect("valid definition"),
            ),
            build_info: gauge_vec(BUILD_INFO, "Build information; always 1.", &["version"]),
            route_enabled: gauge_vec(
                ROUTE_ENABLED,
                "Whether a verification route is enabled in config (1) or not (0).",
                &["type"],
            ),
            registry: Arc::new(registry),
        };

        metrics.pre_create_series();
        metrics
    }

    /// Create every series with a known label set so each appears at 0 from
    /// the first scrape. Otherwise Prometheus misses the first increment.
    fn pre_create_series(&self) {
        self.build_info
            .with_label_values(&[env!("CARGO_PKG_VERSION")])
            .set(1);
        for signup_type in SignupType::ALL {
            let t = signup_type.as_str();
            self.signup_codes_issued.with_label_values(&[t]);
            self.limit_bypassed.with_label_values(&[t]);
            self.route_enabled.with_label_values(&[t]).set(0);
            for reason in RejectionReason::for_signup_type(signup_type) {
                self.verification_rejected
                    .with_label_values(&[t, reason.as_str()]);
            }
        }
        for signup_type in SignupType::WITH_START_EVENT {
            self.verification_started
                .with_label_values(&[signup_type.as_str()]);
        }
        for dependency in Dependency::ALL {
            self.dependency_errors
                .with_label_values(&[dependency.as_str()]);
        }
    }

    pub fn record_signup_code_issued(&self, signup_type: SignupType) {
        self.signup_codes_issued
            .with_label_values(&[signup_type.as_str()])
            .inc();
    }

    pub fn record_rejection(&self, signup_type: SignupType, reason: RejectionReason) {
        self.verification_rejected
            .with_label_values(&[signup_type.as_str(), reason.as_str()])
            .inc();
    }

    pub fn record_dependency_error(&self, dependency: Dependency) {
        self.dependency_errors
            .with_label_values(&[dependency.as_str()])
            .inc();
    }

    /// Count a route-level failure according to its [`FailureKind`].
    pub fn record_failure(&self, signup_type: SignupType, kind: FailureKind) {
        match kind {
            FailureKind::Rejected(reason) => self.record_rejection(signup_type, reason),
            FailureKind::DependencyError(dependency) => self.record_dependency_error(dependency),
            FailureKind::Internal => {}
        }
    }

    pub fn record_verification_started(&self, signup_type: SignupType) {
        self.verification_started
            .with_label_values(&[signup_type.as_str()])
            .inc();
    }

    pub fn record_limit_bypassed(&self, signup_type: SignupType) {
        self.limit_bypassed
            .with_label_values(&[signup_type.as_str()])
            .inc();
    }

    pub fn record_ln_sats_received(&self, sats: u64) {
        self.ln_sats_received.inc_by(sats);
    }

    /// Run a homeserver admin API call and record how long it took.
    pub async fn observe_homeserver_request<T>(&self, call: impl Future<Output = T>) -> T {
        let start = Instant::now();
        let output = call.await;
        self.homeserver_request_duration
            .observe(start.elapsed().as_secs_f64());
        output
    }

    pub fn set_route_enabled(&self, signup_type: SignupType, enabled: bool) {
        self.route_enabled
            .with_label_values(&[signup_type.as_str()])
            .set(enabled as i64);
    }

    // Current values, for asserting on wiring from route tests.

    #[cfg(test)]
    pub fn signup_codes_issued(&self, signup_type: SignupType) -> u64 {
        self.signup_codes_issued
            .with_label_values(&[signup_type.as_str()])
            .get()
    }

    #[cfg(test)]
    pub fn verification_rejected(&self, signup_type: SignupType, reason: RejectionReason) -> u64 {
        self.verification_rejected
            .with_label_values(&[signup_type.as_str(), reason.as_str()])
            .get()
    }

    #[cfg(test)]
    pub fn dependency_errors(&self, dependency: Dependency) -> u64 {
        self.dependency_errors
            .with_label_values(&[dependency.as_str()])
            .get()
    }

    #[cfg(test)]
    pub fn verification_started(&self, signup_type: SignupType) -> u64 {
        self.verification_started
            .with_label_values(&[signup_type.as_str()])
            .get()
    }

    #[cfg(test)]
    pub fn limit_bypassed(&self, signup_type: SignupType) -> u64 {
        self.limit_bypassed
            .with_label_values(&[signup_type.as_str()])
            .get()
    }

    #[cfg(test)]
    pub fn ln_sats_received(&self) -> u64 {
        self.ln_sats_received.get()
    }

    #[cfg(test)]
    pub fn route_enabled(&self, signup_type: SignupType) -> i64 {
        self.route_enabled
            .with_label_values(&[signup_type.as_str()])
            .get()
    }

    /// Render all metrics in the Prometheus text exposition format.
    pub fn render(&self) -> Result<String, prometheus::Error> {
        let mut buffer = Vec::new();
        TextEncoder::new().encode(&self.registry.gather(), &mut buffer)?;
        String::from_utf8(buffer).map_err(|e| prometheus::Error::Msg(e.to_string()))
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

fn register<C: Collector + Clone + 'static>(registry: &Registry, collector: C) -> C {
    registry
        .register(Box::new(collector.clone()))
        .expect("fresh registry has no duplicates");
    collector
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample(rendered: &str, series: &str) -> Option<String> {
        rendered
            .lines()
            .find(|line| line.starts_with(series) && line[series.len()..].starts_with(' '))
            .map(|line| line[series.len() + 1..].to_string())
    }

    #[test]
    fn fresh_recorder_exposes_every_known_series_at_zero() {
        let rendered = Metrics::new().render().unwrap();

        for signup_type in SignupType::ALL {
            let t = signup_type.as_str();
            assert_eq!(
                sample(&rendered, &format!("{SIGNUP_CODES_ISSUED}{{type=\"{t}\"}}")).as_deref(),
                Some("0")
            );
            assert_eq!(
                sample(&rendered, &format!("{LIMIT_BYPASSED}{{type=\"{t}\"}}")).as_deref(),
                Some("0")
            );
            assert_eq!(
                sample(&rendered, &format!("{ROUTE_ENABLED}{{type=\"{t}\"}}")).as_deref(),
                Some("0")
            );
        }
        for dependency in Dependency::ALL {
            let d = dependency.as_str();
            assert_eq!(
                sample(
                    &rendered,
                    &format!("{DEPENDENCY_ERRORS}{{dependency=\"{d}\"}}")
                )
                .as_deref(),
                Some("0")
            );
        }
        // Started series exist only for routes with a start step.
        for signup_type in SignupType::WITH_START_EVENT {
            let t = signup_type.as_str();
            assert_eq!(
                sample(
                    &rendered,
                    &format!("{VERIFICATION_STARTED}{{type=\"{t}\"}}")
                )
                .as_deref(),
                Some("0")
            );
        }
        for t in ["ip", "google"] {
            assert_eq!(
                sample(
                    &rendered,
                    &format!("{VERIFICATION_STARTED}{{type=\"{t}\"}}")
                ),
                None
            );
        }
        // Rejection series exist for every (type, reason) pair a route can emit,
        // and only those.
        for signup_type in SignupType::ALL {
            let t = signup_type.as_str();
            for reason in RejectionReason::for_signup_type(signup_type) {
                let r = reason.as_str();
                assert_eq!(
                    sample(
                        &rendered,
                        &format!("{VERIFICATION_REJECTED}{{reason=\"{r}\",type=\"{t}\"}}")
                    )
                    .as_deref(),
                    Some("0"),
                    "missing series for {t}/{r}"
                );
            }
            let series_for_type = rendered
                .lines()
                .filter(|line| {
                    line.starts_with(VERIFICATION_REJECTED)
                        && line.contains(&format!("type=\"{t}\""))
                })
                .count();
            assert_eq!(
                series_for_type,
                RejectionReason::for_signup_type(signup_type).len()
            );
        }
        assert_eq!(sample(&rendered, LN_SATS_RECEIVED).as_deref(), Some("0"));
        assert_eq!(
            sample(&rendered, &format!("{HOMESERVER_REQUEST_DURATION}_count")).as_deref(),
            Some("0")
        );
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            sample(&rendered, &format!("{BUILD_INFO}{{version=\"{version}\"}}")).as_deref(),
            Some("1")
        );
        assert!(rendered.contains(&format!("# TYPE {SIGNUP_CODES_ISSUED} counter\n")));
        assert!(rendered.contains(&format!("# TYPE {ROUTE_ENABLED} gauge\n")));
        assert!(rendered.contains(&format!("# TYPE {HOMESERVER_REQUEST_DURATION} histogram\n")));
    }

    #[tokio::test]
    async fn recorded_events_show_up_in_render() {
        let metrics = Metrics::new();

        metrics.record_signup_code_issued(SignupType::Ip);
        metrics.record_signup_code_issued(SignupType::Ip);
        metrics.record_rejection(SignupType::Google, RejectionReason::InvalidGoogleIdToken);
        metrics.record_failure(
            SignupType::Sms,
            FailureKind::DependencyError(Dependency::Prelude),
        );
        metrics.record_failure(SignupType::Sms, FailureKind::Internal);
        metrics.record_verification_started(SignupType::Lightning);
        metrics.record_limit_bypassed(SignupType::Ip);
        metrics.record_ln_sats_received(1000);
        metrics.record_ln_sats_received(500);
        metrics.set_route_enabled(SignupType::Google, true);
        let value = metrics
            .observe_homeserver_request(async {
                tokio::time::sleep(Duration::from_millis(5)).await;
                42
            })
            .await;
        assert_eq!(value, 42);

        let rendered = metrics.render().unwrap();
        assert_eq!(
            sample(&rendered, &format!("{SIGNUP_CODES_ISSUED}{{type=\"ip\"}}")).as_deref(),
            Some("2")
        );
        assert_eq!(
            sample(
                &rendered,
                &format!(
                    "{VERIFICATION_REJECTED}{{reason=\"invalid_google_id_token\",type=\"google\"}}"
                )
            )
            .as_deref(),
            Some("1")
        );
        assert_eq!(
            sample(
                &rendered,
                &format!("{DEPENDENCY_ERRORS}{{dependency=\"prelude\"}}")
            )
            .as_deref(),
            Some("1")
        );
        assert_eq!(
            sample(
                &rendered,
                &format!("{VERIFICATION_STARTED}{{type=\"lightning\"}}")
            )
            .as_deref(),
            Some("1")
        );
        assert_eq!(
            sample(&rendered, &format!("{LIMIT_BYPASSED}{{type=\"ip\"}}")).as_deref(),
            Some("1")
        );
        assert_eq!(sample(&rendered, LN_SATS_RECEIVED).as_deref(), Some("1500"));
        assert_eq!(
            sample(&rendered, &format!("{ROUTE_ENABLED}{{type=\"google\"}}")).as_deref(),
            Some("1")
        );
        assert_eq!(
            sample(&rendered, &format!("{HOMESERVER_REQUEST_DURATION}_count")).as_deref(),
            Some("1")
        );
        let sum: f64 = sample(&rendered, &format!("{HOMESERVER_REQUEST_DURATION}_sum"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(sum >= 0.005, "sum was {sum}");
    }

    #[test]
    fn clones_share_one_registry() {
        let metrics = Metrics::new();
        let clone = metrics.clone();
        clone.record_signup_code_issued(SignupType::Sms);
        let rendered = metrics.render().unwrap();
        assert_eq!(
            sample(&rendered, &format!("{SIGNUP_CODES_ISSUED}{{type=\"sms\"}}")).as_deref(),
            Some("1")
        );
    }
}
