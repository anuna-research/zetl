//! Observability surface per OBS-3801..OBS-3810 + NFR-3807.
//!
//! Metric naming follows the existing zetl convention:
//! `zetl_feed_<noun>_<unit>{label=...}`. Cardinality is bounded by the
//! config (subscription_id, scope_id, cohort_id) and by closed enums
//! (license, decision, action, reason, format). The page_slug is
//! never a label.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

/// Counter metrics enumerated by OBS-3801..OBS-3810.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CounterKind {
    /// `zetl_feed_build_total{result="success|error"}`
    BuildTotal,
    /// `zetl_feed_inbound_license_decision_total{license, decision}`
    InboundLicenseDecision,
    /// `zetl_feed_capability_cohort_emit_total{cohort_id, result}`
    CapabilityCohortEmit,
    /// `zetl_feed_retention_total{subscription_id, action, reason}`
    Retention,
    /// `zetl_feed_tombstone_block_total{subscription_id}`
    TombstoneBlock,
    /// `zetl_feed_inbound_auth_failure_total{feed_slug, code}`
    InboundAuthFailure,
}

impl CounterKind {
    /// Canonical metric name.
    pub fn metric_name(self) -> &'static str {
        match self {
            CounterKind::BuildTotal => "zetl_feed_build_total",
            CounterKind::InboundLicenseDecision => "zetl_feed_inbound_license_decision_total",
            CounterKind::CapabilityCohortEmit => "zetl_feed_capability_cohort_emit_total",
            CounterKind::Retention => "zetl_feed_retention_total",
            CounterKind::TombstoneBlock => "zetl_feed_tombstone_block_total",
            CounterKind::InboundAuthFailure => "zetl_feed_inbound_auth_failure_total",
        }
    }
}

/// Histogram metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistogramKind {
    /// `zetl_feed_build_duration_seconds`
    BuildDuration,
}

impl HistogramKind {
    pub fn metric_name(self) -> &'static str {
        match self {
            HistogramKind::BuildDuration => "zetl_feed_build_duration_seconds",
        }
    }
}

/// In-memory metric sink. Production systems forward this to whatever
/// exposition format they use; tests inspect counts directly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetricSink {
    counters: BTreeMap<MetricKey, u64>,
    histograms: BTreeMap<MetricKey, Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
struct MetricKey {
    name: &'static str,
    labels: BTreeMap<&'static str, String>,
}

impl MetricSink {
    pub fn incr(&mut self, kind: CounterKind, labels: &[(&'static str, &str)]) {
        let key = MetricKey {
            name: kind.metric_name(),
            labels: labels.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        };
        *self.counters.entry(key).or_insert(0) += 1;
    }

    pub fn observe_duration(&mut self, kind: HistogramKind, duration: Duration) {
        let key = MetricKey {
            name: kind.metric_name(),
            labels: BTreeMap::new(),
        };
        self.histograms
            .entry(key)
            .or_default()
            .push(duration.as_secs_f64());
    }

    pub fn counter_value(&self, kind: CounterKind, labels: &[(&'static str, &str)]) -> u64 {
        let key = MetricKey {
            name: kind.metric_name(),
            labels: labels.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        };
        *self.counters.get(&key).unwrap_or(&0)
    }

    pub fn histogram_count(&self, kind: HistogramKind) -> usize {
        let key = MetricKey {
            name: kind.metric_name(),
            labels: BTreeMap::new(),
        };
        self.histograms.get(&key).map(|v| v.len()).unwrap_or(0)
    }

    /// Verify cardinality bounds: every counter's label set should
    /// only contain keys from the closed allowlist.
    pub fn verify_label_cardinality(&self, allowed: &[&str]) -> Result<(), Vec<String>> {
        let mut violations = Vec::new();
        for key in self.counters.keys() {
            for label in key.labels.keys() {
                if !allowed.iter().any(|a| a == label) {
                    violations.push(format!("{}::{}", key.name, label));
                }
            }
        }
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

/// Allowed label names. Anything beyond this set fails NFR-3807
/// cardinality checks.
pub const ALLOWED_LABELS: &[&str] = &[
    "result",
    "license",
    "decision",
    "cohort_id",
    "subscription_id",
    "scope_id",
    "action",
    "reason",
    "feed_slug",
    "code",
    "format",
];

/// REQ-3831 enforcement helper: assert a label value is NOT a
/// capability token. Heuristic: hex/base64-shaped strings of ≥32 chars
/// are flagged. Used by [`MetricSink::incr`] callers.
pub fn assert_not_token(value: &str) -> Result<(), &'static str> {
    if value.len() >= 32
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '_')
    {
        return Err("label value looks like a capability token (REQ-3831)");
    }
    Ok(())
}

/// Structured failure-mode log line per NFR-3807. Emitted at warn or
/// error level; the format is JSON so log aggregators can parse it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureLog {
    /// Violated REQ id (e.g. "REQ-3811").
    pub req_id: String,
    /// Page slug or feed URL the failure refers to.
    pub identity: String,
    /// Brief remediation hint surfaced to the operator.
    pub remediation: String,
    /// `warn` or `error`.
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Warn,
    Error,
}

impl FailureLog {
    pub fn render(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }
}

/// OBS-3807 build-time stderr summary line emitted when items were
/// excluded due to license-policy default-deny.
#[derive(Debug, Clone, Default)]
pub struct ExclusionSummary {
    pub denied_unknown_license: usize,
    pub denied_cc_by_nd_full_attempted: usize,
    pub denied_cc_by_nc_commercial: usize,
}

impl fmt::Display for ExclusionSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.denied_unknown_license
            + self.denied_cc_by_nd_full_attempted
            + self.denied_cc_by_nc_commercial;
        write!(
            f,
            "[zetl] feed: {total} item(s) excluded by license policy \
             (unknown={u}, CC-BY-ND-full={nd}, CC-BY-NC-commercial={nc})",
            u = self.denied_unknown_license,
            nd = self.denied_cc_by_nd_full_attempted,
            nc = self.denied_cc_by_nc_commercial,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_increments_per_label_set() {
        let mut sink = MetricSink::default();
        sink.incr(CounterKind::BuildTotal, &[("result", "success")]);
        sink.incr(CounterKind::BuildTotal, &[("result", "success")]);
        sink.incr(CounterKind::BuildTotal, &[("result", "error")]);
        assert_eq!(
            sink.counter_value(CounterKind::BuildTotal, &[("result", "success")]),
            2
        );
        assert_eq!(
            sink.counter_value(CounterKind::BuildTotal, &[("result", "error")]),
            1
        );
    }

    #[test]
    fn histogram_records_durations() {
        let mut sink = MetricSink::default();
        sink.observe_duration(HistogramKind::BuildDuration, Duration::from_millis(123));
        sink.observe_duration(HistogramKind::BuildDuration, Duration::from_millis(456));
        assert_eq!(sink.histogram_count(HistogramKind::BuildDuration), 2);
    }

    #[test]
    fn label_cardinality_check_passes_for_allowed_labels() {
        let mut sink = MetricSink::default();
        sink.incr(
            CounterKind::Retention,
            &[
                ("subscription_id", "x"),
                ("action", "archive"),
                ("reason", "age"),
            ],
        );
        sink.verify_label_cardinality(ALLOWED_LABELS).unwrap();
    }

    #[test]
    fn label_cardinality_check_rejects_unbounded_labels() {
        let mut sink = MetricSink::default();
        sink.counters.insert(
            MetricKey {
                name: "zetl_feed_build_total",
                labels: {
                    let mut m = BTreeMap::new();
                    m.insert("page_slug", "foo".to_string());
                    m
                },
            },
            1,
        );
        let err = sink.verify_label_cardinality(ALLOWED_LABELS).unwrap_err();
        assert!(err.iter().any(|v| v.contains("page_slug")));
    }

    #[test]
    fn assert_not_token_flags_hex_strings() {
        assert!(assert_not_token("alpha").is_ok());
        assert!(assert_not_token("0123456789abcdef0123456789abcdef").is_err());
        assert!(assert_not_token("short").is_ok());
    }

    #[test]
    fn failure_log_serialises_as_json() {
        let log = FailureLog {
            req_id: "REQ-3811".to_string(),
            identity: "https://upstream/feed".to_string(),
            remediation: "configure ssrf allowlist".to_string(),
            level: LogLevel::Warn,
        };
        let s = log.render();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["req_id"], "REQ-3811");
        assert_eq!(v["level"], "warn");
    }

    #[test]
    fn exclusion_summary_renders_total() {
        let s = ExclusionSummary {
            denied_unknown_license: 3,
            denied_cc_by_nd_full_attempted: 1,
            denied_cc_by_nc_commercial: 0,
        };
        assert_eq!(
            format!("{s}"),
            "[zetl] feed: 4 item(s) excluded by license policy (unknown=3, CC-BY-ND-full=1, CC-BY-NC-commercial=0)"
        );
    }
}
