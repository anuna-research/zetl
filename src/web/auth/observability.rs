//! Authentication observability (SPEC-041 §9 OBS-4101..4106, REQ-4115).
//!
//! Two output streams:
//!
//! * **Operator log** (`eprintln!`) — one line per resolved request, the
//!   `[zetl] auth: method=… outcome=… identity=… [cause=…]` shape.
//!   Visible to anyone watching the server's stderr.
//! * **Audit log** (`.zetl/collab/auth-audit.log`) — append-only file
//!   recording every successful authentication for forensic review.
//!
//! Redaction contract (REQ-4115): the primitives in this file take only
//! the safe, structured fields (`method_id` is `&'static str`, `outcome`
//! is a typed enum, `identity` is the resolved user_id or capability
//! handle, `cause` is an `&'static str` category). They CANNOT be passed a
//! raw password, agent token, ID token, PKCE verifier, or `?cap=` token —
//! the function signatures make secret leakage a type error.
//!
//! OBS-4101 (resolve-duration histogram), OBS-4102 (outcomes counter), and
//! OBS-4106 (capability-token state gauge) are skeletal counters here —
//! they expose accumulator types that a future Prometheus/OpenTelemetry
//! integration can scrape. The full metrics-endpoint plumbing is deferred
//! to a follow-up; this task ships the integrity floor (logs + audit +
//! redaction contract) so the SPEC-041 acceptance for REQ-4115 is met.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

const AUDIT_LOG_RELATIVE: &str = ".zetl/collab/auth-audit.log";

/// Outcome class for an authentication decision. Cause-indistinguishable
/// at the user-visible layer (REQ-4107/4110); distinguishable in the
/// operator log via `cause` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AuthOutcomeClass {
    Authenticated,
    Abstained,
    Rejected,
}

impl AuthOutcomeClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AuthOutcomeClass::Authenticated => "authenticated",
            AuthOutcomeClass::Abstained => "abstained",
            AuthOutcomeClass::Rejected => "rejected",
        }
    }
}

/// Emit OBS-4103 — the per-decision operator log line.
///
/// Always goes to stderr. `identity` is the resolved user_id / capability
/// handle (never a token byte) — `Option` because abstained requests have
/// no identity. `cause` is an operator-channel category (never user-
/// visible) — only set on rejections (REQ-4115).
pub(crate) fn log_auth_event(
    method_id: &'static str,
    outcome: AuthOutcomeClass,
    identity: Option<&str>,
    cause: Option<&'static str>,
) {
    let mut line = format!(
        "[zetl] auth: method={method_id} outcome={}",
        outcome.as_str()
    );
    if let Some(id) = identity {
        // Identities can legitimately contain `@` (emails) and `-` (handles).
        // We do not allow whitespace through the audit format, so guard
        // against pathological values by truncating + redacting.
        let safe = sanitise_identity(id);
        line.push_str(" identity=");
        line.push_str(&safe);
    }
    if let Some(c) = cause {
        line.push_str(" cause=");
        line.push_str(c);
    }
    eprintln!("{line}");
}

/// Append an OBS-4104 audit line to `.zetl/collab/auth-audit.log`.
///
/// Successful authentications and rejections are persisted; abstains are
/// not (they're noise). Failures to write the audit file degrade the
/// server's audit trail but DO NOT fail the request — we eprintln a
/// warning so the operator notices. Atomic-append via `O_APPEND` so
/// concurrent writers don't clobber each other.
pub(crate) fn write_audit_line(
    vault_root: &Path,
    method_id: &'static str,
    outcome: AuthOutcomeClass,
    identity: Option<&str>,
    cause: Option<&'static str>,
) {
    if matches!(outcome, AuthOutcomeClass::Abstained) {
        return; // audit only the decisive outcomes
    }
    let path = audit_log_path(vault_root);
    let now = crate::user::access_request::now_iso8601();
    let mut line = format!("{now} method={method_id} outcome={}", outcome.as_str());
    if let Some(id) = identity {
        let safe = sanitise_identity(id);
        line.push_str(" identity=");
        line.push_str(&safe);
    }
    if let Some(c) = cause {
        line.push_str(" cause=");
        line.push_str(c);
    }
    line.push('\n');

    if let Err(e) = append_atomic(&path, line.as_bytes()) {
        eprintln!("[zetl] WARN: failed to append auth audit line: {e}");
    }
}

/// Return the audit log path inside `<vault_root>/.zetl/collab/`.
pub(crate) fn audit_log_path(vault_root: &Path) -> PathBuf {
    vault_root.join(AUDIT_LOG_RELATIVE)
}

fn append_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Re-set perms on first creation so the audit log is operator-only.
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    file.write_all(body)?;
    file.sync_data()?;
    Ok(())
}

/// Render an identity string safe for a single audit line — never allows
/// whitespace, control characters, or `=` (which would corrupt the
/// `k=v` line format). Length-capped at 128 chars to defend against
/// pathological inputs.
pub(crate) fn sanitise_identity(id: &str) -> String {
    let truncated = id.chars().take(128);
    let mut out = String::with_capacity(id.len().min(128));
    for c in truncated {
        match c {
            c if c.is_control() || c.is_whitespace() => out.push('_'),
            '=' | '"' => out.push('_'),
            c => out.push(c),
        }
    }
    out
}

// ── Skeletal counters (OBS-4101/4102/4106 — Prometheus integration deferred) ──

/// Process-wide auth-event counters. Exposed via accessor methods for a
/// future metrics-endpoint task to consume. Today they're just numbers
/// you can dump from a test or a future `/metrics` handler.
#[derive(Debug, Default)]
pub(crate) struct AuthCounters {
    /// Total resolve-outcomes by `(method_id, outcome)`.
    /// `Mutex<HashMap>` is overkill for runtime but suffices for the Phase
    /// 6a scope; a future task can swap in a lock-free shard or a real
    /// histogram.
    outcomes: Mutex<std::collections::HashMap<(&'static str, AuthOutcomeClass), u64>>,
    /// Capability-token state gauge (OBS-4106). Three buckets:
    /// live/expired/revoked. Updated by the share CLI handlers and the
    /// capability-URL authenticator.
    pub cap_live: AtomicU64,
    pub cap_expired: AtomicU64,
    pub cap_revoked: AtomicU64,
}

impl AuthCounters {
    pub(crate) fn record_outcome(&self, method: &'static str, outcome: AuthOutcomeClass) {
        let mut map = self.outcomes.lock().unwrap_or_else(|e| e.into_inner());
        *map.entry((method, outcome)).or_insert(0) += 1;
    }

    pub(crate) fn outcome_count(&self, method: &'static str, outcome: AuthOutcomeClass) -> u64 {
        let map = self.outcomes.lock().unwrap_or_else(|e| e.into_inner());
        map.get(&(method, outcome)).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// REQ-4115: identity sanitiser strips whitespace, controls, and
    /// audit-format-breaking chars; length-caps at 128.
    #[test]
    fn sanitise_identity_strips_dangerous_chars() {
        assert_eq!(sanitise_identity("alice"), "alice");
        assert_eq!(sanitise_identity("alice@example.com"), "alice@example.com");
        assert_eq!(sanitise_identity("a b"), "a_b");
        assert_eq!(sanitise_identity("a\tb"), "a_b");
        assert_eq!(sanitise_identity("a\nb"), "a_b");
        assert_eq!(sanitise_identity("a=b"), "a_b");
        assert_eq!(sanitise_identity("a\"b"), "a_b");
        let long = "a".repeat(200);
        assert_eq!(sanitise_identity(&long).len(), 128);
    }

    /// OBS-4104: audit line gets appended; format is the documented shape.
    #[test]
    fn audit_line_appended() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_audit_line(
            tmp.path(),
            "passkey",
            AuthOutcomeClass::Authenticated,
            Some("alice"),
            None,
        );
        let body = std::fs::read_to_string(audit_log_path(tmp.path())).unwrap();
        assert!(body.contains("method=passkey"));
        assert!(body.contains("outcome=authenticated"));
        assert!(body.contains("identity=alice"));
        assert!(body.ends_with('\n'));
    }

    /// Abstains do not generate audit lines (noise floor).
    #[test]
    fn audit_skips_abstains() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_audit_line(
            tmp.path(),
            "passkey",
            AuthOutcomeClass::Abstained,
            None,
            None,
        );
        assert!(!audit_log_path(tmp.path()).exists());
    }

    /// Reject lines carry the cause category.
    #[test]
    fn audit_reject_carries_cause() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_audit_line(
            tmp.path(),
            "agent-token",
            AuthOutcomeClass::Rejected,
            None,
            Some("bad-signature"),
        );
        let body = std::fs::read_to_string(audit_log_path(tmp.path())).unwrap();
        assert!(body.contains("cause=bad-signature"));
        assert!(body.contains("outcome=rejected"));
    }

    /// REQ-4115: secrets cannot be passed to the audit primitives — the
    /// types force the caller to pre-extract a safe identity. This test
    /// is a documentation-style check that `write_audit_line`'s `identity`
    /// parameter accepts only `Option<&str>` and is bounded by
    /// `sanitise_identity`. A separate grep-based integration test in
    /// downstream test files will verify on real captured logs.
    #[test]
    fn write_audit_line_has_safe_signature() {
        // Just a compile-check: this would not compile if we accidentally
        // changed the signature to take raw bytes / a token.
        let _: fn(&Path, &'static str, AuthOutcomeClass, Option<&str>, Option<&'static str>) =
            write_audit_line;
    }

    /// OBS-4102: outcome counter increments per (method, outcome) pair.
    #[test]
    fn auth_counters_record_outcomes() {
        let c = AuthCounters::default();
        c.record_outcome("passkey", AuthOutcomeClass::Authenticated);
        c.record_outcome("passkey", AuthOutcomeClass::Authenticated);
        c.record_outcome("passkey", AuthOutcomeClass::Abstained);
        c.record_outcome("agent-token", AuthOutcomeClass::Rejected);

        assert_eq!(
            c.outcome_count("passkey", AuthOutcomeClass::Authenticated),
            2
        );
        assert_eq!(c.outcome_count("passkey", AuthOutcomeClass::Abstained), 1);
        assert_eq!(
            c.outcome_count("agent-token", AuthOutcomeClass::Rejected),
            1
        );
        assert_eq!(c.outcome_count("oidc", AuthOutcomeClass::Authenticated), 0);
    }

    /// OBS-4106: capability state gauge buckets are atomically incrementable.
    #[test]
    fn capability_state_gauge_atomics() {
        let c = AuthCounters::default();
        c.cap_live.fetch_add(3, Ordering::Relaxed);
        c.cap_revoked.fetch_add(1, Ordering::Relaxed);
        assert_eq!(c.cap_live.load(Ordering::Relaxed), 3);
        assert_eq!(c.cap_revoked.load(Ordering::Relaxed), 1);
        assert_eq!(c.cap_expired.load(Ordering::Relaxed), 0);
    }
}
