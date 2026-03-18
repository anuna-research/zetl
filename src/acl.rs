//! ACL evaluation pipeline for collaborative vaults (CON-020-005).
//!
//! Implements the six-step process:
//! 1. Load SPL from built-in defaults + `access.spl` + page-level SPL
//! 2. Inject runtime facts (authenticated, requesting, now)
//! 3. If agent: inject `is-agent` fact
//! 4. Load user roles from profile → inject as facts
//! 5. Combine into single theory, ground, reason
//! 6. Check conclusion for `(can-<action> "<user_id>" "<page_slug>")`

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::reason::build_theory;
use crate::reason::types::{ConclusionType, ProofSource};
use crate::types::SplBlock;
use crate::user;

/// An access control action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Edit,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Read => write!(f, "read"),
            Action::Edit => write!(f, "edit"),
        }
    }
}

/// Input to the ACL evaluation pipeline (CON-020-005).
#[derive(Debug, Clone)]
pub struct AclQuery {
    pub user_id: String,
    pub page_slug: String,
    pub action: Action,
    pub is_agent: bool,
    pub now_epoch_ms: i64,
}

/// The tag on a defeasible conclusion that determined the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConclusionTag {
    /// Definitely provable (+D) — strict proof.
    DefinitelyProvable,
    /// Defeasibly provable (+d) — defeasible proof, not defeated.
    DefeasiblyProvable,
    /// Definitely not provable (-D).
    DefinitelyNotProvable,
    /// Defeasibly not provable (-d).
    DefeasiblyNotProvable,
}

/// A reference to a rule that contributed to the decision.
#[derive(Debug, Clone)]
pub struct RuleRef {
    pub label: Option<String>,
    pub source_file: PathBuf,
    pub source_line: u32,
    pub contribution: String,
}

impl From<&ProofSource> for RuleRef {
    fn from(ps: &ProofSource) -> Self {
        RuleRef {
            label: ps.rule_label.clone(),
            source_file: ps.path.clone(),
            source_line: ps.line,
            contribution: ps.contribution.clone(),
        }
    }
}

/// Output of the ACL evaluation pipeline (CON-020-005).
#[derive(Debug, Clone)]
pub enum AclDecision {
    Allowed {
        tag: ConclusionTag,
        rule_trace: Vec<RuleRef>,
    },
    Denied {
        tag: ConclusionTag,
        rule_trace: Vec<RuleRef>,
    },
}

impl AclDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, AclDecision::Allowed { .. })
    }
}

/// Generate the built-in default policy grounded for a specific query (REQ-020-010).
///
/// Spindle-core enforces Datalog safety (head vars must appear in body),
/// so we ground the universal rules with the concrete user_id and page_slug
/// from the query being evaluated.
fn built_in_defaults(user_id: &str, page_slug: &str) -> String {
    format!(
        r#"; Built-in ACL defaults (REQ-020-010), grounded for query
; Default: authenticated users can read all pages
(normally r-default-read
  (authenticated "{user_id}")
  (can-read "{user_id}" "{page_slug}"))

; Owners can do everything — strict rules, cannot be defeated
(always s-owner-read (owner "{user_id}") (can-read "{user_id}" "{page_slug}"))
(always s-owner-edit (owner "{user_id}") (can-edit "{user_id}" "{page_slug}"))
(always s-owner-invite (owner "{user_id}") (can-invite "{user_id}"))

; Admins can edit all pages including access.spl
(normally r-admin-edit
  (admin "{user_id}")
  (can-edit "{user_id}" "{page_slug}"))

; Admins can invite
(normally r-admin-invite
  (admin "{user_id}")
  (can-invite "{user_id}"))

; Agents cannot edit access.spl — defeater argues for negation
(except d-agent-no-acl
  (is-agent "{user_id}")
  (not (can-edit "{user_id}" "access")))
(prefer d-agent-no-acl r-admin-edit)

; Scoped readers can read pages matching their scope
(normally r-scoped-read
  (and (role "{user_id}" reader) (in-scope "{page_slug}" "{user_id}"))
  (can-read "{user_id}" "{page_slug}"))

; Scoped editors can read and edit pages matching their scope
(normally r-scoped-edit
  (and (role "{user_id}" editor) (in-scope "{page_slug}" "{user_id}"))
  (can-edit "{user_id}" "{page_slug}"))

(normally r-scoped-edit-read
  (and (role "{user_id}" editor) (in-scope "{page_slug}" "{user_id}"))
  (can-read "{user_id}" "{page_slug}"))

; Default visibility mode
(given (visibility-mode mixed))
"#
    )
}

/// Evaluate an ACL query against the vault's policy.
///
/// Implements the 6-step pipeline from CON-020-005:
/// 1. Load SPL sources (defaults + access.spl + page SPL)
/// 2. Inject runtime facts
/// 3. If agent, inject is-agent
/// 4. Inject user roles from profile
/// 5. Combine, ground, reason
/// 6. Check conclusion
pub fn evaluate(
    vault_root: &Path,
    query: &AclQuery,
    page_spl_blocks: &[SplBlock],
    all_page_slugs: &[String],
) -> Result<AclDecision> {
    let mut spl_blocks: Vec<SplBlock> = Vec::new();

    // ── Step 1: Load SPL sources ─────────────────────────────────────────

    // 1a. Built-in defaults grounded for this query (lowest priority)
    let defaults = built_in_defaults(&query.user_id, &query.page_slug);
    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<built-in>"),
        source_page: String::from("<acl-defaults>"),
        start_line: 1,
        end_line: defaults.lines().count() as u32,
        content: defaults,
    });

    // 1b. Vault policy: access.spl
    if let Some(access_block) = load_access_spl(vault_root)? {
        spl_blocks.push(access_block);
    }

    // 1c. Page-level overrides (highest priority)
    spl_blocks.extend_from_slice(page_spl_blocks);

    // ── Step 2: Inject runtime facts ─────────────────────────────────────

    let mut runtime_facts = String::new();

    // Core runtime facts
    runtime_facts.push_str(&format!(
        "(given (authenticated \"{}\"))\n",
        query.user_id
    ));
    runtime_facts.push_str(&format!(
        "(given (requesting \"{}\" \"{}\" \"{}\"))\n",
        query.user_id, query.page_slug, query.action
    ));
    runtime_facts.push_str(&format!("(given (now {}))\n", query.now_epoch_ms));

    // ── Step 3: Agent flag ───────────────────────────────────────────────

    if query.is_agent {
        runtime_facts.push_str(&format!(
            "(given (is-agent \"{}\"))\n",
            query.user_id
        ));
    }

    // ── Step 4: User roles from profile ──────────────────────────────────

    if let Some(profile) = user::load_profile(vault_root, &query.user_id)? {
        if profile.owner {
            runtime_facts.push_str(&format!(
                "(given (owner \"{}\"))\n",
                query.user_id
            ));
            runtime_facts.push_str(&format!(
                "(given (admin \"{}\"))\n",
                query.user_id
            ));
        }

        let role = user::Role::for_profile(&profile);
        runtime_facts.push_str(&format!(
            "(given (role \"{}\" {}))\n",
            query.user_id, role
        ));

        if role == user::Role::Admin {
            runtime_facts.push_str(&format!(
                "(given (admin \"{}\"))\n",
                query.user_id
            ));
        }
    }

    // ── Step 4b: Ground in-scope facts ───────────────────────────────────
    // The built-in defaults use `(in-scope "<page>" "<user_id>")` as a proxy.
    // We inject this fact when the queried page matches any of the user's scopes.

    let scopes = extract_scopes_from_access_spl(vault_root);
    let page_in_scope = scopes.iter().any(|s| {
        let glob = build_scope_glob(s);
        glob.is_match(&query.page_slug)
    });

    if page_in_scope {
        runtime_facts.push_str(&format!(
            "(given (in-scope \"{}\" \"{}\"))\n",
            query.page_slug, query.user_id
        ));
    }

    // Also inject in-scope facts for all page slugs (used by page-level SPL overrides
    // that may reference other pages' scope status).
    for page in all_page_slugs {
        if page == &query.page_slug {
            continue; // already handled above
        }
        let in_scope = scopes.iter().any(|s| {
            let glob = build_scope_glob(s);
            glob.is_match(page.as_str())
        });
        if in_scope {
            runtime_facts.push_str(&format!(
                "(given (in-scope \"{}\" \"{}\"))\n",
                page, query.user_id
            ));
        }
    }

    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<runtime>"),
        source_page: String::from("<acl-runtime>"),
        start_line: 1,
        end_line: runtime_facts.lines().count() as u32,
        content: runtime_facts,
    });

    // ── Step 5: Combine, ground, reason ──────────────────────────────────

    let result = build_theory(&spl_blocks)
        .context("ACL pipeline: failed to build and reason over access theory")?;

    // ── Step 6: Check conclusion ─────────────────────────────────────────

    let target_predicate = match query.action {
        Action::Read => "can-read",
        Action::Edit => "can-edit",
    };

    // Look for a conclusion matching (can-<action> "<user_id>" "<page_slug>")
    for conclusion in &result.conclusions {
        let lit = &conclusion.literal;

        if !lit.contains(target_predicate) {
            continue;
        }

        let matches = literal_matches(lit, target_predicate, &query.user_id, &query.page_slug);
        if !matches {
            continue;
        }

        let rule_trace: Vec<RuleRef> =
            conclusion.proof_sources.iter().map(RuleRef::from).collect();

        let (tag, allowed) = match conclusion.conclusion_type {
            ConclusionType::DefinitelyProvable => (ConclusionTag::DefinitelyProvable, true),
            ConclusionType::DefeasiblyProvable => (ConclusionTag::DefeasiblyProvable, true),
            ConclusionType::DefinitelyNotProvable => {
                (ConclusionTag::DefinitelyNotProvable, false)
            }
            ConclusionType::DefeasiblyNotProvable => {
                (ConclusionTag::DefeasiblyNotProvable, false)
            }
        };

        if allowed {
            return Ok(AclDecision::Allowed { tag, rule_trace });
        } else {
            return Ok(AclDecision::Denied { tag, rule_trace });
        }
    }

    // No conclusion found → access denied (closed-world assumption)
    Ok(AclDecision::Denied {
        tag: ConclusionTag::DefeasiblyNotProvable,
        rule_trace: vec![],
    })
}

/// Load `.zetl/collab/access.spl` as a single SPL block.
fn load_access_spl(vault_root: &Path) -> Result<Option<SplBlock>> {
    let path = vault_root.join(".zetl/collab/access.spl");
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    if content.trim().is_empty() {
        return Ok(None);
    }

    let line_count = content.lines().count().max(1) as u32;
    Ok(Some(SplBlock {
        source_file: PathBuf::from(".zetl/collab/access.spl"),
        source_page: String::from("<access-policy>"),
        start_line: 1,
        end_line: line_count,
        content,
    }))
}

/// Extract scope patterns from access.spl.
///
/// Looks for `(given (scope "<user>" "<pattern>"))` facts and returns
/// the unique set of scope patterns.
fn extract_scopes_from_access_spl(vault_root: &Path) -> Vec<String> {
    let path = vault_root.join(".zetl/collab/access.spl");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut scopes = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        // Match pattern: (given (scope "user" "pattern"))
        if let Some(rest) = trimmed.strip_prefix("(given (scope ") {
            if let Some(start) = rest.find('"') {
                let after_first = &rest[start + 1..];
                if let Some(end) = after_first.find('"') {
                    let remaining = &after_first[end + 1..];
                    if let Some(s2) = remaining.find('"') {
                        let after_s2 = &remaining[s2 + 1..];
                        if let Some(e2) = after_s2.find('"') {
                            let scope = &after_s2[..e2];
                            if !scopes.contains(&scope.to_string()) {
                                scopes.push(scope.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    scopes
}

/// Build a globset::GlobMatcher for a scope pattern.
fn build_scope_glob(scope: &str) -> globset::GlobMatcher {
    globset::Glob::new(scope)
        .unwrap_or_else(|_| globset::Glob::new("**").unwrap())
        .compile_matcher()
}

/// Check if a rendered literal string matches the target predicate with the given args.
///
/// Spindle renders predicate conclusions as `"functor(arg1, arg2)"` in Display form.
fn literal_matches(rendered: &str, predicate: &str, user_id: &str, page_slug: &str) -> bool {
    let expected = format!("{predicate}({user_id}, {page_slug})");
    if rendered == expected {
        return true;
    }

    // Also match negated form
    let neg_expected = format!("~{predicate}({user_id}, {page_slug})");
    if rendered == neg_expected {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_vault(tmp: &TempDir) -> PathBuf {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".zetl/collab")).unwrap();
        std::fs::create_dir_all(root.join(".zetl/users")).unwrap();
        root
    }

    fn create_owner(vault_root: &Path, name: &str) -> String {
        let id = user::generate_user_id(name);
        let profile = user::UserProfile {
            id: id.clone(),
            name: name.to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: vec![],
            recovery_pubkey: "dGVzdA".to_string(),
            agent_token_generation: 0,
        };
        user::save_profile(vault_root, &profile).unwrap();
        id
    }

    fn create_user(vault_root: &Path, name: &str, invited_by: &str) -> String {
        let id = user::generate_user_id(name);
        let profile = user::UserProfile {
            id: id.clone(),
            name: name.to_string(),
            created_at: "2026-03-18T11:00:00Z".to_string(),
            invited_by: Some(invited_by.to_string()),
            owner: false,
            credentials: vec![],
            recovery_pubkey: "dGVzdA".to_string(),
            agent_token_generation: 0,
        };
        user::save_profile(vault_root, &profile).unwrap();
        id
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    #[test]
    fn owner_can_read_and_edit_any_page() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");

        let read_q = AclQuery {
            user_id: owner_id.clone(),
            page_slug: "secret/internal".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &read_q, &[], &["secret/internal".to_string()]).unwrap();
        assert!(decision.is_allowed(), "owner should be able to read any page");

        let edit_q = AclQuery {
            action: Action::Edit,
            ..read_q
        };
        let decision = evaluate(&vault, &edit_q, &[], &["secret/internal".to_string()]).unwrap();
        assert!(decision.is_allowed(), "owner should be able to edit any page");
    }

    #[test]
    fn authenticated_user_can_read_by_default() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "public-page".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[], &["public-page".to_string()]).unwrap();
        assert!(
            decision.is_allowed(),
            "authenticated user should be able to read by default"
        );
    }

    #[test]
    fn non_owner_cannot_edit_without_grant() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "any-page".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[], &["any-page".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "non-owner without edit grant should be denied"
        );
    }

    #[test]
    fn scoped_editor_can_edit_matching_pages() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Write access.spl granting Bob editor on projects/*
        let access_spl = format!(
            "(given (role \"{}\" editor))\n(given (scope \"{}\" \"projects/*\"))\n",
            bob_id, bob_id
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let pages = vec![
            "projects/roadmap".to_string(),
            "secret/internal".to_string(),
        ];

        // Bob can edit projects/roadmap
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "projects/roadmap".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let decision = evaluate(&vault, &q, &[], &pages).unwrap();
        assert!(
            decision.is_allowed(),
            "scoped editor should be able to edit matching page"
        );

        // Bob cannot edit secret/internal
        let q2 = AclQuery {
            user_id: bob_id,
            page_slug: "secret/internal".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let decision2 = evaluate(&vault, &q2, &[], &pages).unwrap();
        assert!(
            !decision2.is_allowed(),
            "scoped editor should not edit non-matching page"
        );
    }

    #[test]
    fn page_level_override_defeats_vault_policy() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Vault policy: Bob is an editor with ** scope (can read everything)
        let access_spl = format!(
            "(given (role \"{}\" editor))\n(given (scope \"{}\" \"**\"))\n",
            bob_id, bob_id
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // Page-level override: defeater argues for negation to block reads
        let page_override = format!(
            "(except d-secret-block\n  (authenticated \"{}\")\n  (not (can-read \"{}\" \"secret\")))\n\
             (prefer d-secret-block r-default-read)\n\
             (prefer d-secret-block r-scoped-edit-read)\n",
            bob_id, bob_id
        );
        let page_block = SplBlock {
            source_file: PathBuf::from("secret.md"),
            source_page: "secret".to_string(),
            start_line: 1,
            end_line: 4,
            content: page_override,
        };

        let pages = vec!["secret".to_string()];

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "secret".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[page_block], &pages).unwrap();
        assert!(
            !decision.is_allowed(),
            "page-level defeater should block vault-level read grant"
        );
    }

    #[test]
    fn agent_cannot_edit_access_spl() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let agent_id = create_user(&vault, "Agent", &owner_id);

        // Give agent admin role
        let access_spl = format!(
            "(given (admin \"{}\"))\n(given (role \"{}\" admin))\n",
            agent_id, agent_id
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let q = AclQuery {
            user_id: agent_id,
            page_slug: "access".to_string(),
            action: Action::Edit,
            is_agent: true,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[], &["access".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "agent should not be able to edit access.spl"
        );
    }

    #[test]
    fn action_display() {
        assert_eq!(Action::Read.to_string(), "read");
        assert_eq!(Action::Edit.to_string(), "edit");
    }

    #[test]
    fn literal_matches_positive() {
        assert!(literal_matches(
            "can-read(alice, projects/roadmap)",
            "can-read",
            "alice",
            "projects/roadmap"
        ));
        assert!(!literal_matches(
            "can-read(alice, projects/roadmap)",
            "can-edit",
            "alice",
            "projects/roadmap"
        ));
        assert!(!literal_matches(
            "can-read(alice, other)",
            "can-read",
            "alice",
            "projects/roadmap"
        ));
    }

    #[test]
    fn decision_is_allowed() {
        let allowed = AclDecision::Allowed {
            tag: ConclusionTag::DefeasiblyProvable,
            rule_trace: vec![],
        };
        assert!(allowed.is_allowed());

        let denied = AclDecision::Denied {
            tag: ConclusionTag::DefeasiblyNotProvable,
            rule_trace: vec![],
        };
        assert!(!denied.is_allowed());
    }
}
