//! Boundary types shared across SPEC-039 pure core and shell.
//!
//! Every type is `Debug + Clone + serde::{Serialize, Deserialize}` so
//! the JSONL storage format is the canonical wire shape. Timestamps are
//! Unix epoch seconds (`u64`) — same convention as
//! [`crate::feed::fetch::FetchState`] — so the leaf module pulls in no
//! `chrono` dependency. URLs use the `url` crate already on the default
//! dependency surface.

use serde::{Deserialize, Serialize};

/// Unix epoch seconds (UTC). Resolution we care about is "last-seen
/// dates"; sub-second precision is unnecessary noise in a peer-replicable
/// JSONL log.
pub type EpochSecs = u64;

/// A POST /webmention request after parsing + structural validation but
/// before source-fetch / verification. The receive handler constructs
/// this; the pipeline consumes it.
///
/// `rationale` and `source_title` are populated by the receive pipeline
/// when the mention reaches the queue (post-verify, post-moderate). They
/// are `Option<...>` with serde default so older `queue.jsonl` rows
/// written before this field existed still round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IncomingMention {
    pub source: String,
    pub target: String,
    pub received_at: EpochSecs,
    /// Rule that caused this to be queued (e.g. `default-queue`).
    /// Surfaced by `zetl webmention list` so moderators can answer
    /// "why is this pending?" without re-running the rule engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// HTML <title> of the source page, extracted at verify time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
}

/// Post-verification, pre-moderation. The pure verifier produces this
/// indirectly: the shell builds it after `core::verify::verify_link_present`
/// returns `Found`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedMention {
    pub source: String,
    pub target: String,
    pub verified_at: EpochSecs,
    /// Hex-encoded blake3 of the source HTML at verify time. Used by the
    /// re-verification path (REQ-3911) and for observability.
    pub source_html_hash: String,
}

/// Moderation gate output. Carries a rationale tag naming the rule that
/// fired so `zetl webmention list --json` can surface "why" without
/// re-running the rule engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationDecision {
    pub kind: ModerationKind,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationKind {
    Accept,
    Queue,
    Deny,
}

/// An accepted external backlink, persisted to
/// `.zetl/webmentions/received.jsonl`. Carries enough provenance for
/// `zetl webmention list` and the backlink-panel renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEdge {
    pub source: String,
    pub target: String,
    pub accepted_at: EpochSecs,
    pub last_seen: EpochSecs,
    /// Best-effort, set when the source page provided one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    /// Moderation rule that fired (`already-linked` / `allowlist` /
    /// `default-accept` / manual `moderator-accept`). Optional for
    /// backward-compat with older logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    /// `true` when the edge has been removed (the source page no longer
    /// links to the target — REQ-3911). Tombstones live in the same file
    /// as live edges so the read path can fold them in a single pass.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tombstoned: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One outbound POST candidate emitted by the build/serve sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundMention {
    pub source_page_url: String,
    pub target_url: String,
    /// blake3 hex of the surrounding paragraph context — flips when the
    /// link's surrounding content changes meaningfully so the receiver
    /// re-fetches and updates its display (REQ-3906 changed-content
    /// signal).
    pub content_hash: String,
}

/// One row of `.zetl/webmentions/sent.jsonl`. Append-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentRecord {
    pub source_page_url: String,
    pub target_url: String,
    pub content_hash: String,
    pub sent_at: EpochSecs,
    pub response_status: u16,
    /// `true` when the SentRecord represents a post-removal re-POST per
    /// REQ-3907 (we POSTed even though the link is gone, so the receiver
    /// re-fetches and tombstones).
    #[serde(default, skip_serializing_if = "is_false")]
    pub removal: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum WebmentionError {
    #[error("invalid form input: {0}")]
    BadInput(String),
    #[error("verification failed: source does not link to target")]
    NotVerified,
    #[error("source fetch failed: {0}")]
    SourceFetch(#[from] crate::feed::fetch::FetchError),
    #[error("io: {0}")]
    Io(String),
    #[error("config: {0}")]
    Config(String),
}

impl From<std::io::Error> for WebmentionError {
    fn from(err: std::io::Error) -> Self {
        WebmentionError::Io(err.to_string())
    }
}

/// Current epoch-seconds. Convenience wrapper so call-sites don't have
/// to re-derive `SystemTime::now().duration_since(UNIX_EPOCH)` every
/// time.
pub fn now_epoch() -> EpochSecs {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moderation_decision_roundtrips() {
        let d = ModerationDecision {
            kind: ModerationKind::Accept,
            rationale: "already-linked".to_string(),
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: ModerationDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
        assert!(json.contains("\"accept\""));
    }

    #[test]
    fn external_edge_omits_optional_fields() {
        let edge = ExternalEdge {
            source: "https://a.example/post".into(),
            target: "https://b.example/page".into(),
            accepted_at: 1_700_000_000,
            last_seen: 1_700_000_000,
            source_title: None,
            rationale: None,
            tombstoned: false,
        };
        let json = serde_json::to_string(&edge).unwrap();
        assert!(!json.contains("source_title"));
        assert!(!json.contains("tombstoned"));
    }

    #[test]
    fn sent_record_omits_removal_when_false() {
        let r = SentRecord {
            source_page_url: "https://me.example/p".into(),
            target_url: "https://t.example/q".into(),
            content_hash: "deadbeef".into(),
            sent_at: 1,
            response_status: 201,
            removal: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("removal"));
    }

    #[test]
    fn now_epoch_is_recent() {
        let n = now_epoch();
        // Sanity check: this test was authored 2026-05; epoch must be
        // greater than 2024-01-01 (1_704_067_200 secs).
        assert!(n > 1_704_067_200_u64);
    }
}
