//! Pure moderation gate per REQ-3905 + ADR-3903.
//!
//! Hybrid defeasible-rule policy. Order of evaluation:
//!
//! 1. Source domain in `denylist_domains` → Deny (`"denylist"`).
//! 2. Source domain in `allowlist_domains` → Accept (`"allowlist"`).
//! 3. Source domain in `vault_outbound_domains` (the vault has previously
//!    linked to this domain) → Accept (`"already-linked"`).
//! 4. Otherwise → `default_decision` (`"default-{accept,queue,deny}"`).
//!
//! Domain matching is exact on the registered name, case-insensitive.
//! Subdomains do NOT inherit parent allow/deny entries — the conservative
//! choice for v1. (`evil.example.com` does not match `example.com` in
//! either direction. Operators wanting tree-style scoping enumerate
//! subdomains explicitly.)
//!
//! Pure: deterministic over inputs, no allocations beyond the rationale
//! string.

use std::collections::HashSet;

use url::Url;

use crate::webmention::config::ModerationDefault;
use crate::webmention::types::{ModerationDecision, ModerationKind, VerifiedMention};

#[derive(Debug, Clone)]
pub struct ModerationContext<'a> {
    pub allowlist_domains: &'a HashSet<String>,
    pub denylist_domains: &'a HashSet<String>,
    pub vault_outbound_domains: &'a HashSet<String>,
    pub default_decision: ModerationDefault,
}

/// Evaluate the moderation rules against `mention`. Returns the decision
/// + a rationale tag (the rule name that fired). Pure.
pub fn moderate(mention: &VerifiedMention, ctx: &ModerationContext<'_>) -> ModerationDecision {
    let domain = source_domain(&mention.source).unwrap_or_default();

    if domain.is_empty() {
        return ModerationDecision {
            kind: ModerationKind::Queue,
            rationale: "no-source-host".to_string(),
        };
    }

    if ctx.denylist_domains.contains(&domain) {
        return ModerationDecision {
            kind: ModerationKind::Deny,
            rationale: "denylist".to_string(),
        };
    }
    if ctx.allowlist_domains.contains(&domain) {
        return ModerationDecision {
            kind: ModerationKind::Accept,
            rationale: "allowlist".to_string(),
        };
    }
    if ctx.vault_outbound_domains.contains(&domain) {
        return ModerationDecision {
            kind: ModerationKind::Accept,
            rationale: "already-linked".to_string(),
        };
    }

    let (kind, tag) = match ctx.default_decision {
        ModerationDefault::Accept => (ModerationKind::Accept, "default-accept"),
        ModerationDefault::Queue => (ModerationKind::Queue, "default-queue"),
        ModerationDefault::Deny => (ModerationKind::Deny, "default-deny"),
    };
    ModerationDecision {
        kind,
        rationale: tag.to_string(),
    }
}

/// Lower-cased registered domain of the source URL, suitable for set
/// membership against operator-configured domain lists. Returns `None`
/// when the URL parses but has no host (e.g., `data:` URIs — though those
/// are filtered earlier).
pub fn source_domain(source_url: &str) -> Option<String> {
    let url = Url::parse(source_url).ok()?;
    let host = url.host_str()?;
    Some(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm(src: &str) -> VerifiedMention {
        VerifiedMention {
            source: src.into(),
            target: "https://me.example/".into(),
            verified_at: 1,
            source_html_hash: "h".into(),
        }
    }

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn denylist_takes_precedence_over_allowlist() {
        let allow = set(&["spam.example"]);
        let deny = set(&["spam.example"]);
        let outbound = set(&[]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &deny,
            vault_outbound_domains: &outbound,
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("https://spam.example/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Deny);
        assert_eq!(d.rationale, "denylist");
    }

    #[test]
    fn allowlist_accepts() {
        let allow = set(&["friend.example"]);
        let deny = set(&[]);
        let outbound = set(&[]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &deny,
            vault_outbound_domains: &outbound,
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("https://friend.example/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Accept);
        assert_eq!(d.rationale, "allowlist");
    }

    #[test]
    fn already_linked_accepts() {
        let allow = set(&[]);
        let deny = set(&[]);
        let outbound = set(&["peer.example"]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &deny,
            vault_outbound_domains: &outbound,
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("https://peer.example/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Accept);
        assert_eq!(d.rationale, "already-linked");
    }

    #[test]
    fn default_queue_for_strangers() {
        let allow = set(&[]);
        let deny = set(&[]);
        let outbound = set(&[]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &deny,
            vault_outbound_domains: &outbound,
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("https://stranger.example/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Queue);
        assert_eq!(d.rationale, "default-queue");
    }

    #[test]
    fn default_deny_can_be_configured() {
        let allow = set(&[]);
        let deny = set(&[]);
        let outbound = set(&[]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &deny,
            vault_outbound_domains: &outbound,
            default_decision: ModerationDefault::Deny,
        };
        let d = moderate(&vm("https://stranger.example/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Deny);
        assert_eq!(d.rationale, "default-deny");
    }

    #[test]
    fn host_match_is_case_insensitive() {
        let allow = set(&["friend.example"]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &HashSet::new(),
            vault_outbound_domains: &HashSet::new(),
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("https://FRIEND.EXAMPLE/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Accept);
    }

    #[test]
    fn subdomain_is_not_inherited() {
        let allow = set(&["example.com"]);
        let ctx = ModerationContext {
            allowlist_domains: &allow,
            denylist_domains: &HashSet::new(),
            vault_outbound_domains: &HashSet::new(),
            default_decision: ModerationDefault::Queue,
        };
        // evil.example.com is NOT a member of the allowlist.
        let d = moderate(&vm("https://evil.example.com/p"), &ctx);
        assert_eq!(d.kind, ModerationKind::Queue);
    }

    #[test]
    fn unparseable_source_queues_with_no_host_rationale() {
        let ctx = ModerationContext {
            allowlist_domains: &HashSet::new(),
            denylist_domains: &HashSet::new(),
            vault_outbound_domains: &HashSet::new(),
            default_decision: ModerationDefault::Queue,
        };
        let d = moderate(&vm("not-a-url"), &ctx);
        assert_eq!(d.kind, ModerationKind::Queue);
        assert_eq!(d.rationale, "no-source-host");
    }
}
