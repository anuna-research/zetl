//! Pure idempotency-diff for the sender per REQ-3906 + REQ-3907.
//!
//! Given the current set of outbound mentions (computed from the rendered
//! build) and the persisted SentRecord log, decide which `(source,
//! target)` pairs to actually POST. The zero-POST property — "rebuild
//! with no changes sends zero outbound POSTs" — is verifiable here as a
//! pure-data property test.

use std::collections::HashMap;

use crate::webmention::types::{OutboundMention, SentRecord};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdempotencyDiff {
    /// Pairs the sender SHOULD POST: either new (never sent) or
    /// content-hash changed since the last send.
    pub to_send: Vec<OutboundMention>,
    /// Pairs that were previously sent but are absent from `current` —
    /// the sender SHOULD re-POST per REQ-3907 so the receiver re-fetches
    /// and tombstones the link.
    pub to_resend_for_removal: Vec<OutboundMention>,
}

/// Compute the build's outbound diff. Inputs are pure data; output is
/// deterministic up to insertion order (preserved from `current` for
/// `to_send`, and from `previous_log` for `to_resend_for_removal`).
///
/// Settle semantics: only the *latest* record for each `(source,
/// target)` key matters. A "send → remove → restore → remove" chain
/// emits a fresh removal on the second deletion (the prior removal is
/// shadowed by the subsequent restore). Walking `previous_log` in
/// order and overwriting the per-key entry gives this last-wins
/// behaviour.
pub fn idempotency_diff(
    current: &[OutboundMention],
    previous_log: &[SentRecord],
) -> IdempotencyDiff {
    // Latest record per (source, target). Append-only log → last write
    // wins.
    let mut latest_per_key: HashMap<(&str, &str), &SentRecord> = HashMap::new();
    for rec in previous_log {
        latest_per_key.insert((rec.source_page_url.as_str(), rec.target_url.as_str()), rec);
    }

    let mut current_keys: HashMap<(&str, &str), &OutboundMention> = HashMap::new();
    for m in current {
        current_keys.insert((m.source_page_url.as_str(), m.target_url.as_str()), m);
    }

    let mut to_send = Vec::new();
    for m in current {
        let key = (m.source_page_url.as_str(), m.target_url.as_str());
        match latest_per_key.get(&key) {
            None => to_send.push(m.clone()),
            Some(prev) if prev.removal => {
                // Link was previously removed; it's back — send.
                to_send.push(m.clone());
            }
            Some(prev) if prev.content_hash != m.content_hash => to_send.push(m.clone()),
            _ => {}
        }
    }

    // Removal candidates: the LATEST record for the key is a non-removal
    // send AND the key is absent from `current`. We iterate `previous_log`
    // in order so output ordering stays stable for tests, but the only
    // log entry that produces a removal for each key is the one that
    // happens to be that key's latest record.
    let mut to_resend_for_removal = Vec::new();
    let mut emitted: std::collections::HashSet<(&str, &str)> = std::collections::HashSet::new();
    for rec in previous_log {
        let key = (rec.source_page_url.as_str(), rec.target_url.as_str());
        if emitted.contains(&key) {
            continue;
        }
        let Some(latest) = latest_per_key.get(&key) else {
            continue;
        };
        if std::ptr::eq(*latest, rec) && !latest.removal && !current_keys.contains_key(&key) {
            to_resend_for_removal.push(OutboundMention {
                source_page_url: rec.source_page_url.clone(),
                target_url: rec.target_url.clone(),
                content_hash: rec.content_hash.clone(),
            });
            emitted.insert(key);
        }
    }

    IdempotencyDiff {
        to_send,
        to_resend_for_removal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(src: &str, tgt: &str, h: &str) -> OutboundMention {
        OutboundMention {
            source_page_url: src.into(),
            target_url: tgt.into(),
            content_hash: h.into(),
        }
    }

    fn rec(src: &str, tgt: &str, h: &str, removal: bool) -> SentRecord {
        SentRecord {
            source_page_url: src.into(),
            target_url: tgt.into(),
            content_hash: h.into(),
            sent_at: 0,
            response_status: 201,
            removal,
        }
    }

    #[test]
    fn fresh_build_sends_everything() {
        let current = vec![
            out("https://me.example/a", "https://x.example/", "h1"),
            out("https://me.example/b", "https://y.example/", "h2"),
        ];
        let diff = idempotency_diff(&current, &[]);
        assert_eq!(diff.to_send.len(), 2);
        assert!(diff.to_resend_for_removal.is_empty());
    }

    #[test]
    fn unchanged_rebuild_is_zero_post() {
        let current = vec![out("https://me.example/a", "https://x.example/", "h1")];
        let log = vec![rec(
            "https://me.example/a",
            "https://x.example/",
            "h1",
            false,
        )];
        let diff = idempotency_diff(&current, &log);
        assert!(diff.to_send.is_empty());
        assert!(diff.to_resend_for_removal.is_empty());
    }

    #[test]
    fn content_hash_change_triggers_resend() {
        let current = vec![out("https://me.example/a", "https://x.example/", "h2")];
        let log = vec![rec(
            "https://me.example/a",
            "https://x.example/",
            "h1",
            false,
        )];
        let diff = idempotency_diff(&current, &log);
        assert_eq!(diff.to_send.len(), 1);
        assert_eq!(diff.to_send[0].content_hash, "h2");
    }

    #[test]
    fn removed_link_emits_removal_post() {
        let current: Vec<OutboundMention> = Vec::new();
        let log = vec![rec(
            "https://me.example/a",
            "https://x.example/",
            "h1",
            false,
        )];
        let diff = idempotency_diff(&current, &log);
        assert!(diff.to_send.is_empty());
        assert_eq!(diff.to_resend_for_removal.len(), 1);
    }

    #[test]
    fn removal_already_settled_does_not_re_emit() {
        let current: Vec<OutboundMention> = Vec::new();
        let log = vec![
            rec("https://me.example/a", "https://x.example/", "h1", false),
            rec("https://me.example/a", "https://x.example/", "h1", true),
        ];
        let diff = idempotency_diff(&current, &log);
        assert!(diff.to_send.is_empty());
        assert!(diff.to_resend_for_removal.is_empty());
    }

    #[test]
    fn restored_link_after_removal_is_sent_again() {
        let current = vec![out("https://me.example/a", "https://x.example/", "h1")];
        let log = vec![
            rec("https://me.example/a", "https://x.example/", "h1", false),
            rec("https://me.example/a", "https://x.example/", "h1", true),
        ];
        let diff = idempotency_diff(&current, &log);
        assert_eq!(diff.to_send.len(), 1);
        assert!(diff.to_resend_for_removal.is_empty());
    }

    #[test]
    fn duplicate_log_entries_yield_single_removal() {
        let current: Vec<OutboundMention> = Vec::new();
        let log = vec![
            rec("https://me.example/a", "https://x.example/", "h1", false),
            rec("https://me.example/a", "https://x.example/", "h2", false),
        ];
        let diff = idempotency_diff(&current, &log);
        assert_eq!(diff.to_resend_for_removal.len(), 1);
    }

    #[test]
    fn second_deletion_after_restore_emits_another_removal() {
        // Send (h1) → Remove → Restore (h2) send → Remove again.
        // The second deletion MUST emit a new removal POST so the
        // remote receiver tombstones the restored mention.
        let current: Vec<OutboundMention> = Vec::new();
        let log = vec![
            rec("https://me.example/a", "https://x.example/", "h1", false), // send
            rec("https://me.example/a", "https://x.example/", "h1", true),  // first removal
            rec("https://me.example/a", "https://x.example/", "h2", false), // restore + send
        ];
        let diff = idempotency_diff(&current, &log);
        assert!(diff.to_send.is_empty());
        assert_eq!(
            diff.to_resend_for_removal.len(),
            1,
            "second deletion after restore must emit another removal"
        );
        assert_eq!(diff.to_resend_for_removal[0].content_hash, "h2");
    }
}
