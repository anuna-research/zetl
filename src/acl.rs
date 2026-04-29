//! ACL evaluation pipeline for collaborative vaults (CON-020-005).
//!
//! Implements the six-step process:
//! 1. Load SPL from built-in defaults + `access.spl` + page-level SPL
//! 2. Inject runtime facts (authenticated, requesting, now)
//! 3. If agent: inject `is-agent` fact
//! 4. Load user roles from profile → inject as facts
//! 5. Combine into single theory, ground, reason
//! 6. Check conclusion for `(can-<action> "<user_id>" "<page_slug>")`
//!
//! ## Deontic Overlay (REQ-020-012)
//!
//! After the base ACL check, the pipeline scans for modal conclusions:
//! - `[F](edit/read user page)` at +d/+D → overrides base Allowed → Denied (403)
//! - `[P](edit/read user page)` at +d/+D → explicit permission (redundant with base)
//! - `[O](action user page)` at +d/+D → informational obligation (does not affect access)

use anyhow::{Context, Result};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::reason::build_theory;
use crate::reason::types::{ConclusionType, ProofSource};
use crate::types::SplBlock;
use crate::user;

/// Escape a string for safe injection into SPL literals.
///
/// Prevents SPL injection by escaping backslashes and double quotes,
/// and stripping characters that could break Datalog syntax (parens, newlines).
fn escape_spl(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '(' | ')' | '\n' | '\r' => {} // strip syntax-breaking chars
            _ => out.push(c),
        }
    }
    out
}

/// Visibility mode controlling how denied pages appear to users (REQ-020-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisibilityMode {
    /// Denied pages visible (grayed) in sidebar; 403 on direct access; grayed-out wikilinks.
    Transparent,
    /// Denied pages hidden from sidebar/search; 403 on direct access; lock icon on wikilinks.
    #[default]
    Mixed,
    /// Denied pages fully hidden; 404 on direct access; dead-link wikilinks.
    Hidden,
}

/// Per-page visibility override for a specific user (REQ-020-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageVisibilityOverride {
    /// `(hidden-from ?user ?page)` — force hide regardless of mode.
    ForceHidden,
    /// `(visible-title ?user ?page)` — force show title with lock even in hidden mode.
    ForceVisible,
    /// No override — use vault's visibility-mode setting.
    None,
}

/// Regex matching `(given (owner ...))` facts — these must only be injected
/// from profile.json, never from user-editable SPL (REQ-020-058).
static OWNER_FACT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*\(given\s+\(owner\s[^)]*\)\s*\)").unwrap());

/// Regex matching global identity predicates that must only be injected at runtime,
/// never from page-level SPL (REQ-020-059). Covers: admin, role, scope,
/// visibility-mode, is-agent.  Owner is already handled by `OWNER_FACT_RE`.
static GLOBAL_PREDICATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*\(given\s+\((admin|role|scope|visibility-mode|is-agent)\s[^)]*\)\s*\)")
        .unwrap()
});

/// Regex matching `can-read` / `can-edit` with a quoted page slug:
///   `can-read "user" "page-slug"` or `can-edit "user" "page-slug"`
/// Captures the page slug in group 1.
static ACCESS_CONCLUSION_PAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"can-(?:read|edit)\s+"[^"]*"\s+"([^"]*)""#).unwrap());

/// Regex matching `(given (during <name> <start-ms> <end-ms>))` temporal interval
/// declarations (REQ-020-011).
static INTERVAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(given\s+\(during\s+\((\S+)\)\s+(\d+)\s+(\d+)\s*\)\s*\)").unwrap()
});

/// A named temporal interval parsed from SPL `(given (during ...))` facts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TemporalInterval {
    name: String,
    start_ms: i64,
    end_ms: i64,
}

/// Allen interval algebra relation between a point (`now`) and an interval (REQ-020-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemporalRelation {
    /// now < start
    Before,
    /// now == start
    Meets,
    /// start < now < end
    Within,
    /// now == end
    Finishes,
    /// now > end
    After,
}

/// Parse all `(given (during <name> <start> <end>))` declarations from SPL content.
fn parse_temporal_intervals(spl_content: &str) -> Vec<TemporalInterval> {
    INTERVAL_RE
        .captures_iter(spl_content)
        .filter_map(|cap| {
            let name = cap[1].to_string();
            let start_ms: i64 = cap[2].parse().ok()?;
            let end_ms: i64 = cap[3].parse().ok()?;
            Some(TemporalInterval {
                name,
                start_ms,
                end_ms,
            })
        })
        .collect()
}

/// Compute the Allen relation between a point (`now`) and an interval.
fn temporal_relation(now: i64, interval: &TemporalInterval) -> TemporalRelation {
    if now < interval.start_ms {
        TemporalRelation::Before
    } else if now == interval.start_ms {
        TemporalRelation::Meets
    } else if now < interval.end_ms {
        TemporalRelation::Within
    } else if now == interval.end_ms {
        TemporalRelation::Finishes
    } else {
        TemporalRelation::After
    }
}

/// Inject grounded temporal relation facts for all declared intervals (REQ-020-011).
///
/// For each `(given (during <name> <start> <end>))` found in the SPL blocks,
/// computes the Allen relation between `now` and the interval and injects
/// grounded facts:
/// - `(given (now-before <name>))` if now < start
/// - `(given (now-meets <name>))` if now == start
/// - `(given (now-within <name>))` if start < now < end  (also injects `active`)
/// - `(given (now-finishes <name>))` if now == end  (also injects `active`)
/// - `(given (now-after <name>))` if now > end
/// - `(given (active <name>))` if within or meets or finishes (now is in [start, end])
fn ground_temporal_facts(spl_blocks: &[SplBlock], now_epoch_ms: i64) -> String {
    let mut intervals = Vec::new();
    for block in spl_blocks {
        intervals.extend(parse_temporal_intervals(&block.content));
    }

    let mut facts = String::new();
    for interval in &intervals {
        let relation = temporal_relation(now_epoch_ms, interval);
        let name = &interval.name;

        match relation {
            TemporalRelation::Before => {
                facts.push_str(&format!("(given (now-before {name}))\n"));
            }
            TemporalRelation::Meets => {
                facts.push_str(&format!("(given (now-meets {name}))\n"));
                facts.push_str(&format!("(given (active {name}))\n"));
                facts.push_str(&format!("(given (now-within {name}))\n"));
            }
            TemporalRelation::Within => {
                facts.push_str(&format!("(given (now-within {name}))\n"));
                facts.push_str(&format!("(given (active {name}))\n"));
            }
            TemporalRelation::Finishes => {
                facts.push_str(&format!("(given (now-finishes {name}))\n"));
                facts.push_str(&format!("(given (active {name}))\n"));
                facts.push_str(&format!("(given (now-within {name}))\n"));
            }
            TemporalRelation::After => {
                facts.push_str(&format!("(given (now-after {name}))\n"));
            }
        }
    }

    facts
}

/// Deontic modality from SPL modal operators (REQ-020-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeonticModality {
    /// `[P]` — Permission: action is explicitly permitted.
    Permission,
    /// `[F]` — Forbidden: action is explicitly prohibited.
    Forbidden,
    /// `[O]` — Obligation: user is obligated to perform action (informational).
    Obligation,
}

impl std::fmt::Display for DeonticModality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeonticModality::Permission => write!(f, "[P]"),
            DeonticModality::Forbidden => write!(f, "[F]"),
            DeonticModality::Obligation => write!(f, "[O]"),
        }
    }
}

/// A single deontic conclusion found in the theory (REQ-020-012).
#[derive(Debug, Clone)]
pub struct DeonticConclusion {
    /// The modality: Permission, Forbidden, or Obligation.
    pub modality: DeonticModality,
    /// The predicate name (e.g. "edit", "read", "review").
    pub predicate: String,
    /// The conclusion tag (+D, +d, -D, -d).
    pub tag: ConclusionTag,
    /// Rules that contributed to this conclusion.
    pub rule_trace: Vec<RuleRef>,
}

/// Deontic overlay: optional layer of modal conclusions on top of base ACL (REQ-020-012).
///
/// Vaults that don't use deontic modalities will have an empty overlay.
#[derive(Debug, Clone, Default)]
pub struct DeonticOverlay {
    /// `[F]` conclusions — forbidden actions. If provable, overrides base Allowed → Denied.
    pub forbidden: Vec<DeonticConclusion>,
    /// `[P]` conclusions — explicitly permitted actions (redundant with base, but explicit).
    pub permitted: Vec<DeonticConclusion>,
    /// `[O]` conclusions — obligations (informational only, does not affect access).
    pub obligations: Vec<DeonticConclusion>,
}

impl DeonticOverlay {
    /// Returns true if this overlay has any deontic conclusions.
    pub fn is_empty(&self) -> bool {
        self.forbidden.is_empty() && self.permitted.is_empty() && self.obligations.is_empty()
    }

    /// Returns true if a matching `[F]` conclusion is provable (overrides base access).
    pub fn has_active_forbidden(&self) -> bool {
        self.forbidden.iter().any(|c| {
            matches!(
                c.tag,
                ConclusionTag::DefinitelyProvable | ConclusionTag::DefeasiblyProvable
            )
        })
    }
}

/// An access control action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    let user_id = escape_spl(user_id);
    let page_slug = escape_spl(page_slug);
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

/// Scan theory conclusions for deontic modal literals matching the query (REQ-020-012).
///
/// Looks for `[P]`, `[F]`, `[O]` prefixed conclusions that match the queried
/// action/user/page combination and collects them into a `DeonticOverlay`.
fn scan_deontic_conclusions(
    conclusions: &[crate::reason::types::ProvenancedConclusion],
    user_id: &str,
    page_slug: &str,
    action: Action,
) -> DeonticOverlay {
    let mut overlay = DeonticOverlay::default();

    let action_str = match action {
        Action::Read => "read",
        Action::Edit => "edit",
    };

    for conclusion in conclusions {
        let lit = &conclusion.literal;

        // Match deontic conclusions: [P]pred(user, page), [F]pred(user, page), [O]pred(user, page)
        let (modality, rest) = if let Some(rest) = lit.strip_prefix("[P]") {
            (DeonticModality::Permission, rest)
        } else if let Some(rest) = lit.strip_prefix("[F]") {
            (DeonticModality::Forbidden, rest)
        } else if let Some(rest) = lit.strip_prefix("[O]") {
            (DeonticModality::Obligation, rest)
        } else {
            continue;
        };

        // Extract predicate and check if it matches our query args.
        // Format: "pred(arg1, arg2)" or just "pred"
        let (predicate, matches_query) = if let Some(paren_pos) = rest.find('(') {
            let pred = &rest[..paren_pos];
            let args_str = &rest[paren_pos + 1..rest.len().saturating_sub(1)]; // strip parens
            let args: Vec<&str> = args_str.split(", ").collect();
            let matches = args.len() == 2 && args[0] == user_id && args[1] == page_slug;
            (pred.to_string(), matches)
        } else {
            (rest.to_string(), false)
        };

        if !matches_query {
            continue;
        }

        let tag = match conclusion.conclusion_type {
            ConclusionType::DefinitelyProvable => ConclusionTag::DefinitelyProvable,
            ConclusionType::DefeasiblyProvable => ConclusionTag::DefeasiblyProvable,
            ConclusionType::DefinitelyNotProvable => ConclusionTag::DefinitelyNotProvable,
            ConclusionType::DefeasiblyNotProvable => ConclusionTag::DefeasiblyNotProvable,
        };

        let rule_trace: Vec<RuleRef> = conclusion.proof_sources.iter().map(RuleRef::from).collect();

        let dc = DeonticConclusion {
            modality,
            predicate: predicate.clone(),
            tag,
            rule_trace,
        };

        match modality {
            DeonticModality::Permission => {
                // [P] for the matching action
                if predicate == action_str {
                    overlay.permitted.push(dc);
                }
            }
            DeonticModality::Forbidden => {
                // [F] for the matching action
                if predicate == action_str {
                    overlay.forbidden.push(dc);
                }
            }
            DeonticModality::Obligation => {
                // [O] for any predicate (informational)
                overlay.obligations.push(dc);
            }
        }
    }

    overlay
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

    // 1b. Vault policy: access.spl (strip owner facts — REQ-020-058)
    if let Some(mut access_block) = load_access_spl(vault_root)? {
        strip_owner_facts(&mut access_block);
        spl_blocks.push(access_block);
    }

    // 1c. Page-level overrides (highest priority, sandboxed — REQ-020-058/059)
    for block in page_spl_blocks {
        let mut sanitized = block.clone();
        strip_owner_facts(&mut sanitized);
        sandbox_page_spl(&mut sanitized);
        if !sanitized.content.trim().is_empty() {
            spl_blocks.push(sanitized);
        }
    }

    // ── Step 2: Inject runtime facts ─────────────────────────────────────

    let mut runtime_facts = String::new();
    let safe_user = escape_spl(&query.user_id);
    let safe_page = escape_spl(&query.page_slug);

    // Core runtime facts
    runtime_facts.push_str(&format!("(given (authenticated \"{safe_user}\"))\n"));
    runtime_facts.push_str(&format!(
        "(given (requesting \"{}\" \"{}\" \"{}\"))\n",
        safe_user, safe_page, query.action
    ));
    runtime_facts.push_str(&format!("(given (now {}))\n", query.now_epoch_ms));

    // ── Step 3: Agent flag ───────────────────────────────────────────────

    if query.is_agent {
        runtime_facts.push_str(&format!("(given (is-agent \"{safe_user}\"))\n"));
    }

    // ── Step 4: User roles from profile ──────────────────────────────────

    if let Some(profile) = user::load_profile(vault_root, &query.user_id)? {
        if profile.owner {
            runtime_facts.push_str(&format!("(given (owner \"{safe_user}\"))\n"));
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }

        let role = user::Role::for_profile_with_vault(&profile, vault_root);
        runtime_facts.push_str(&format!("(given (role \"{safe_user}\" {role}))\n"));

        if role == user::Role::Admin {
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }
    }

    // ── Step 4b: Ground in-scope facts ───────────────────────────────────
    // The built-in defaults use `(in-scope "<page>" "<user_id>")` as a proxy.
    // We inject this fact when the queried page matches any of the user's scopes.

    let scopes = extract_user_scopes_from_access_spl(vault_root, &query.user_id);
    let page_in_scope = scopes.iter().any(|s| {
        let glob = match build_scope_glob(s) {
            Some(g) => g,
            None => return false,
        };
        glob.is_match(&query.page_slug)
    });

    if page_in_scope {
        runtime_facts.push_str(&format!(
            "(given (in-scope \"{safe_page}\" \"{safe_user}\"))\n"
        ));
    }

    // Also inject in-scope facts for all page slugs (used by page-level SPL overrides
    // that may reference other pages' scope status).
    for page in all_page_slugs {
        if page == &query.page_slug {
            continue; // already handled above
        }
        let in_scope = scopes
            .iter()
            .any(|s| build_scope_glob(s).is_some_and(|g| g.is_match(page.as_str())));
        if in_scope {
            runtime_facts.push_str(&format!(
                "(given (in-scope \"{}\" \"{}\"))\n",
                escape_spl(page),
                safe_user
            ));
        }
    }

    // ── Step 4c: Ground temporal interval facts (REQ-020-011) ─────────────
    // Parse (given (during <name> <start> <end>)) from all SPL blocks so far,
    // evaluate Allen relations against `now`, and inject grounded facts.
    let temporal_facts = ground_temporal_facts(&spl_blocks, query.now_epoch_ms);
    if !temporal_facts.is_empty() {
        runtime_facts.push_str(&temporal_facts);
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
    let mut base_decision = None;
    for conclusion in &result.conclusions {
        let lit = &conclusion.literal;

        if !lit.contains(target_predicate) {
            continue;
        }

        let matches = literal_matches(lit, target_predicate, &query.user_id, &query.page_slug);
        if !matches {
            continue;
        }

        let rule_trace: Vec<RuleRef> = conclusion.proof_sources.iter().map(RuleRef::from).collect();

        let (tag, allowed) = match conclusion.conclusion_type {
            ConclusionType::DefinitelyProvable => (ConclusionTag::DefinitelyProvable, true),
            ConclusionType::DefeasiblyProvable => (ConclusionTag::DefeasiblyProvable, true),
            ConclusionType::DefinitelyNotProvable => (ConclusionTag::DefinitelyNotProvable, false),
            ConclusionType::DefeasiblyNotProvable => (ConclusionTag::DefeasiblyNotProvable, false),
        };

        base_decision = Some(if allowed {
            AclDecision::Allowed { tag, rule_trace }
        } else {
            AclDecision::Denied { tag, rule_trace }
        });
        break;
    }

    let decision = base_decision.unwrap_or(AclDecision::Denied {
        tag: ConclusionTag::DefeasiblyNotProvable,
        rule_trace: vec![],
    });

    // ── Step 7: Deontic overlay (REQ-020-012) ────────────────────────────
    // Scan for [F], [P], [O] modal conclusions. If [F] is provable and
    // the base decision was Allowed, override to Denied.
    let overlay = scan_deontic_conclusions(
        &result.conclusions,
        &query.user_id,
        &query.page_slug,
        query.action,
    );

    if decision.is_allowed() && overlay.has_active_forbidden() {
        // [F] override: forbidden conclusion overrides base permission
        if let Some(forbidden) = overlay.forbidden.iter().find(|c| {
            matches!(
                c.tag,
                ConclusionTag::DefinitelyProvable | ConclusionTag::DefeasiblyProvable
            )
        }) {
            return Ok(AclDecision::Denied {
                tag: forbidden.tag,
                rule_trace: forbidden.rule_trace.clone(),
            });
        }
    }

    Ok(decision)
}

/// Strip `(given (owner ...))` facts from SPL content (REQ-020-058).
///
/// Owner status must only be injected from profile.json at runtime.
/// Any `(given (owner ...))` found in user-editable SPL (access.spl or
/// page-level blocks) is removed and a warning is emitted.
fn strip_owner_facts(block: &mut SplBlock) {
    let mut stripped = Vec::new();
    let mut new_lines = Vec::new();

    for (i, line) in block.content.lines().enumerate() {
        if OWNER_FACT_RE.is_match(line) {
            let source_line = block.start_line + i as u32;
            stripped.push((source_line, line.trim().to_string()));
        } else {
            new_lines.push(line);
        }
    }

    if !stripped.is_empty() {
        for (line_no, fact) in &stripped {
            eprintln!(
                "warning: stripped owner fact from {} line {}: {} (owner is set via profile.json only)",
                block.source_file.display(),
                line_no,
                fact,
            );
        }
        block.content = new_lines.join("\n");
        // Preserve trailing newline if original had one
        if !block.content.is_empty() && !block.content.ends_with('\n') {
            block.content.push('\n');
        }
    }
}

/// Sandbox page-level SPL: strip global predicates and cross-page access conclusions (REQ-020-059).
///
/// Page-level SPL may only:
/// - Declare facts/rules that reference its own page slug in `can-read`/`can-edit` conclusions
/// - Use defeaters and superiority relations within its own page
///
/// Any global identity predicates (`admin`, `role`, `scope`, `visibility-mode`, `is-agent`)
/// are stripped with a warning. If any `can-read`/`can-edit` reference targets a page
/// other than `source_page`, the entire block is rejected (emptied) with a warning.
///
/// Owner facts are handled separately by [`strip_owner_facts`].
pub fn sandbox_page_spl(block: &mut SplBlock) {
    // Phase 1: Strip global identity predicates (line-by-line)
    let mut global_stripped = Vec::new();
    let mut surviving_lines = Vec::new();

    for (i, line) in block.content.lines().enumerate() {
        if GLOBAL_PREDICATE_RE.is_match(line) {
            let source_line = block.start_line + i as u32;
            global_stripped.push((source_line, line.trim().to_string()));
        } else {
            surviving_lines.push(line);
        }
    }

    if !global_stripped.is_empty() {
        for (line_no, fact) in &global_stripped {
            eprintln!(
                "warning: stripped global predicate from {} (page \"{}\") line {}: {} \
                 (global predicates are set via profile/runtime only)",
                block.source_file.display(),
                block.source_page,
                line_no,
                fact,
            );
        }
        block.content = surviving_lines.join("\n");
        if !block.content.is_empty() && !block.content.ends_with('\n') {
            block.content.push('\n');
        }
    }

    // Phase 2: Reject cross-page access conclusions
    let source_page = &block.source_page;
    let mut cross_page_refs: Vec<String> = Vec::new();

    for cap in ACCESS_CONCLUSION_PAGE_RE.captures_iter(&block.content) {
        let referenced_page = &cap[1];
        if referenced_page != source_page {
            cross_page_refs.push(referenced_page.to_string());
        }
    }

    if !cross_page_refs.is_empty() {
        let unique_pages: Vec<String> = {
            let mut v = cross_page_refs;
            v.sort();
            v.dedup();
            v
        };
        eprintln!(
            "warning: rejected page SPL block from {} (page \"{}\"): \
             cross-page access conclusions targeting {:?} — \
             page-level SPL may only affect its own page",
            block.source_file.display(),
            block.source_page,
            unique_pages,
        );
        block.content = String::new();
    }
}

/// Evaluate an ACL query and return the decision, deontic overlay, and full `TheoryResult`.
///
/// This is the same as [`evaluate`] but also returns the reasoner output and
/// deontic overlay, enabling proof-trace / why-not explanation in the
/// `/api/acl/explain` endpoint (REQ-020-014) and deontic modality display (REQ-020-012).
pub fn evaluate_with_theory(
    vault_root: &Path,
    query: &AclQuery,
    page_spl_blocks: &[SplBlock],
    all_page_slugs: &[String],
) -> Result<(
    AclDecision,
    DeonticOverlay,
    crate::reason::types::TheoryResult,
)> {
    let mut spl_blocks: Vec<SplBlock> = Vec::new();

    // ── Step 1: Load SPL sources ─────────────────────────────────────────
    let defaults = built_in_defaults(&query.user_id, &query.page_slug);
    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<built-in>"),
        source_page: String::from("<acl-defaults>"),
        start_line: 1,
        end_line: defaults.lines().count() as u32,
        content: defaults,
    });

    if let Some(mut access_block) = load_access_spl(vault_root)? {
        strip_owner_facts(&mut access_block);
        spl_blocks.push(access_block);
    }

    for block in page_spl_blocks {
        let mut sanitized = block.clone();
        strip_owner_facts(&mut sanitized);
        sandbox_page_spl(&mut sanitized);
        if !sanitized.content.trim().is_empty() {
            spl_blocks.push(sanitized);
        }
    }

    // ── Step 2–4: Runtime facts (same as evaluate) ───────────────────────
    let mut runtime_facts = String::new();
    let safe_user = escape_spl(&query.user_id);
    let safe_page = escape_spl(&query.page_slug);
    runtime_facts.push_str(&format!("(given (authenticated \"{safe_user}\"))\n"));
    runtime_facts.push_str(&format!(
        "(given (requesting \"{}\" \"{}\" \"{}\"))\n",
        safe_user, safe_page, query.action
    ));
    runtime_facts.push_str(&format!("(given (now {}))\n", query.now_epoch_ms));

    if query.is_agent {
        runtime_facts.push_str(&format!("(given (is-agent \"{safe_user}\"))\n"));
    }

    if let Some(profile) = user::load_profile(vault_root, &query.user_id)? {
        if profile.owner {
            runtime_facts.push_str(&format!("(given (owner \"{safe_user}\"))\n"));
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }
        let role = user::Role::for_profile_with_vault(&profile, vault_root);
        runtime_facts.push_str(&format!("(given (role \"{safe_user}\" {role}))\n"));
        if role == user::Role::Admin {
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }
    }

    let scopes = extract_user_scopes_from_access_spl(vault_root, &query.user_id);
    let page_in_scope = scopes.iter().any(|s| {
        let glob = match build_scope_glob(s) {
            Some(g) => g,
            None => return false,
        };
        glob.is_match(&query.page_slug)
    });
    if page_in_scope {
        runtime_facts.push_str(&format!(
            "(given (in-scope \"{safe_page}\" \"{safe_user}\"))\n"
        ));
    }
    for page in all_page_slugs {
        if page == &query.page_slug {
            continue;
        }
        let in_scope = scopes
            .iter()
            .any(|s| build_scope_glob(s).is_some_and(|g| g.is_match(page.as_str())));
        if in_scope {
            runtime_facts.push_str(&format!(
                "(given (in-scope \"{}\" \"{}\"))\n",
                escape_spl(page),
                safe_user
            ));
        }
    }

    let temporal_facts = ground_temporal_facts(&spl_blocks, query.now_epoch_ms);
    if !temporal_facts.is_empty() {
        runtime_facts.push_str(&temporal_facts);
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

    let mut base_decision = None;
    for conclusion in &result.conclusions {
        let lit = &conclusion.literal;
        if !lit.contains(target_predicate) {
            continue;
        }
        let matches = literal_matches(lit, target_predicate, &query.user_id, &query.page_slug);
        if !matches {
            continue;
        }

        let rule_trace: Vec<RuleRef> = conclusion.proof_sources.iter().map(RuleRef::from).collect();

        let (tag, allowed) = match conclusion.conclusion_type {
            ConclusionType::DefinitelyProvable => (ConclusionTag::DefinitelyProvable, true),
            ConclusionType::DefeasiblyProvable => (ConclusionTag::DefeasiblyProvable, true),
            ConclusionType::DefinitelyNotProvable => (ConclusionTag::DefinitelyNotProvable, false),
            ConclusionType::DefeasiblyNotProvable => (ConclusionTag::DefeasiblyNotProvable, false),
        };

        base_decision = Some(if allowed {
            AclDecision::Allowed { tag, rule_trace }
        } else {
            AclDecision::Denied { tag, rule_trace }
        });
        break;
    }

    let decision = base_decision.unwrap_or(AclDecision::Denied {
        tag: ConclusionTag::DefeasiblyNotProvable,
        rule_trace: vec![],
    });

    // ── Step 7: Deontic overlay (REQ-020-012) ────────────────────────────
    let overlay = scan_deontic_conclusions(
        &result.conclusions,
        &query.user_id,
        &query.page_slug,
        query.action,
    );

    let final_decision = if decision.is_allowed() && overlay.has_active_forbidden() {
        if let Some(forbidden) = overlay.forbidden.iter().find(|c| {
            matches!(
                c.tag,
                ConclusionTag::DefinitelyProvable | ConclusionTag::DefeasiblyProvable
            )
        }) {
            AclDecision::Denied {
                tag: forbidden.tag,
                rule_trace: forbidden.rule_trace.clone(),
            }
        } else {
            decision
        }
    } else {
        decision
    };

    Ok((final_decision, overlay, result))
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
/// Extract scope patterns from access.spl that belong to a specific user.
///
/// Looks for `(given (scope "<user>" "<pattern>"))` facts and returns only
/// the patterns assigned to the given `user_id`.
fn extract_user_scopes_from_access_spl(vault_root: &Path, user_id: &str) -> Vec<String> {
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
                    let scope_user = &after_first[..end];
                    // Only include scopes belonging to this user
                    if scope_user != user_id {
                        continue;
                    }
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
///
/// Returns `None` (and logs a warning) if the glob pattern is invalid,
/// rather than falling back to a permissive `**` match.
fn build_scope_glob(scope: &str) -> Option<globset::GlobMatcher> {
    match globset::Glob::new(scope) {
        Ok(g) => Some(g.compile_matcher()),
        Err(e) => {
            eprintln!("warning: invalid scope glob pattern {scope:?}: {e} — skipping");
            None
        }
    }
}

/// Query the vault's visibility mode from SPL theory (REQ-020-030).
///
/// Loads built-in defaults + access.spl and checks which `(visibility-mode X)` fact
/// is concluded. Returns `Mixed` if no explicit mode is found.
pub fn query_visibility_mode(vault_root: &Path) -> VisibilityMode {
    let mut spl_blocks: Vec<SplBlock> = Vec::new();

    // Built-in defaults include `(given (visibility-mode mixed))`
    let defaults = r#"(given (visibility-mode mixed))"#.to_string();
    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<built-in>"),
        source_page: String::from("<visibility-defaults>"),
        start_line: 1,
        end_line: 1,
        content: defaults,
    });

    // access.spl may override with `(given (visibility-mode hidden))` etc.
    if let Ok(Some(access_block)) = load_access_spl(vault_root) {
        spl_blocks.push(access_block);
    }

    // Check the last `(given (visibility-mode ...))` fact — last wins.
    let mut mode = VisibilityMode::Mixed;
    for block in &spl_blocks {
        for line in block.content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("(given (visibility-mode ") {
                let val = rest.trim_end_matches("))").trim();
                match val {
                    "transparent" => mode = VisibilityMode::Transparent,
                    "hidden" => mode = VisibilityMode::Hidden,
                    "mixed" => mode = VisibilityMode::Mixed,
                    _ => {}
                }
            }
        }
    }

    mode
}

/// Check per-page visibility overrides for a specific user (REQ-020-030).
///
/// Evaluates `(hidden-from ?user ?page)` and `(visible-title ?user ?page)` predicates
/// by running the ACL theory. Returns `ForceHidden`, `ForceVisible`, or `None`.
pub fn query_page_visibility_override(
    vault_root: &Path,
    user_id: &str,
    page_slug: &str,
    page_spl_blocks: &[SplBlock],
    all_page_slugs: &[String],
) -> PageVisibilityOverride {
    // Build a minimal ACL query to evaluate the theory
    let query = AclQuery {
        user_id: user_id.to_string(),
        page_slug: page_slug.to_string(),
        action: Action::Read,
        is_agent: false,
        now_epoch_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
    };

    let result = match evaluate_with_theory(vault_root, &query, page_spl_blocks, all_page_slugs) {
        Ok((_decision, _overlay, theory)) => theory,
        Err(_) => return PageVisibilityOverride::None,
    };

    // Check for hidden-from conclusion
    let hidden_pred = format!("hidden-from({user_id}, {page_slug})");
    let visible_pred = format!("visible-title({user_id}, {page_slug})");

    for conclusion in &result.conclusions {
        let lit = &conclusion.literal;
        let is_positive = matches!(
            conclusion.conclusion_type,
            ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
        );

        if is_positive && lit == &hidden_pred {
            return PageVisibilityOverride::ForceHidden;
        }
        if is_positive && lit == &visible_pred {
            return PageVisibilityOverride::ForceVisible;
        }
    }

    PageVisibilityOverride::None
}

/// Determine effective visibility for a denied page.
///
/// Combines the vault-level visibility mode with any per-page override.
/// Returns the effective mode to use for rendering.
pub fn effective_visibility(
    mode: VisibilityMode,
    page_override: PageVisibilityOverride,
) -> VisibilityMode {
    match page_override {
        PageVisibilityOverride::ForceHidden => VisibilityMode::Hidden,
        PageVisibilityOverride::ForceVisible => VisibilityMode::Transparent,
        PageVisibilityOverride::None => mode,
    }
}

/// Check if a rendered literal string matches the target predicate with the given args.
///
/// Spindle renders predicate conclusions as `"functor(arg1, arg2)"` in Display form.
fn literal_matches(rendered: &str, predicate: &str, user_id: &str, page_slug: &str) -> bool {
    // Negated conclusions (starting with ~) are not positive matches
    if rendered.starts_with('~') {
        return false;
    }

    let expected = format!("{predicate}({user_id}, {page_slug})");
    rendered == expected
}

fn literal_matches_1arg(rendered: &str, predicate: &str, arg: &str) -> bool {
    if rendered.starts_with('~') {
        return false;
    }
    let expected = format!("{predicate}({arg})");
    rendered == expected
}

fn literal_matches_2arg(rendered: &str, predicate: &str, arg1: &str, arg2: &str) -> bool {
    if rendered.starts_with('~') {
        return false;
    }
    let expected = format!("{predicate}({arg1}, {arg2})");
    rendered == expected
}

// ── Asset ACL helpers (SPEC-035) ────────────────────────────────────────────

/// Built-in ACL defaults for `can-upload` predicate (REQ-3504).
fn built_in_upload_defaults(user_id: &str) -> String {
    let user_id = escape_spl(user_id);
    format!(
        r#"; Built-in upload ACL defaults (REQ-3504)
(normally r-editor-upload
  (and (role "{user_id}" editor) (scope "{user_id}" "**"))
  (can-upload "{user_id}"))

(always s-owner-upload
  (owner "{user_id}")
  (can-upload "{user_id}"))

(normally r-admin-upload
  (admin "{user_id}")
  (can-upload "{user_id}"))

(except d-agent-no-upload
  (is-agent "{user_id}")
  (not (can-upload "{user_id}")))
(prefer d-agent-no-upload r-admin-upload)
"#
    )
}

/// Built-in ACL defaults for `can-read-assets` predicate (REQ-3516).
fn built_in_read_assets_defaults(user_id: &str, visibility_mode: VisibilityMode) -> String {
    let user_id = escape_spl(user_id);
    let mode_str = match visibility_mode {
        VisibilityMode::Transparent => "transparent",
        VisibilityMode::Mixed => "mixed",
        VisibilityMode::Hidden => "hidden",
    };
    format!(
        r#"; Built-in read-assets ACL defaults (REQ-3516)
(given (visibility-mode {mode_str}))

(normally r-default-read-assets
  (authenticated "{user_id}")
  (can-read-assets "{user_id}" "*"))

(normally r-public-read-assets
  (and (visibility-mode transparent) (not (authenticated "{user_id}")))
  (can-read-assets "anonymous" "*"))
"#
    )
}

/// Evaluate a single-argument or two-argument asset predicate.
fn evaluate_asset_predicate(
    vault_root: &Path,
    user_id: &str,
    predicate: &str,
    built_in_defaults: &str,
    is_authenticated: bool,
    is_agent: bool,
) -> Result<bool> {
    let mut spl_blocks: Vec<SplBlock> = Vec::new();

    // 1a. Built-in defaults
    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<built-in>"),
        source_page: String::from("<acl-defaults>"),
        start_line: 1,
        end_line: built_in_defaults.lines().count() as u32,
        content: built_in_defaults.to_string(),
    });

    // 1b. Vault policy: access.spl
    if let Some(mut access_block) = load_access_spl(vault_root)? {
        strip_owner_facts(&mut access_block);
        spl_blocks.push(access_block);
    }

    // 2. Runtime facts
    let mut runtime_facts = String::new();
    let safe_user = escape_spl(user_id);
    if is_authenticated {
        runtime_facts.push_str(&format!("(given (authenticated \"{safe_user}\"))\n"));
    }
    runtime_facts.push_str(&format!(
        "(given (requesting \"{safe_user}\" \"*\" \"read\"))\n"
    ));
    if is_agent {
        runtime_facts.push_str(&format!("(given (is-agent \"{safe_user}\"))\n"));
    }

    // 3. User roles from profile
    if let Some(profile) = user::load_profile(vault_root, user_id)? {
        if profile.owner {
            runtime_facts.push_str(&format!("(given (owner \"{safe_user}\"))\n"));
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }
        let role = user::Role::for_profile_with_vault(&profile, vault_root);
        runtime_facts.push_str(&format!("(given (role \"{safe_user}\" {role}))\n"));
        if role == user::Role::Admin {
            runtime_facts.push_str(&format!("(given (admin \"{safe_user}\"))\n"));
        }

        // Inject scopes
        let scopes = extract_user_scopes_from_access_spl(vault_root, user_id);
        for scope in scopes {
            runtime_facts.push_str(&format!("(given (scope \"{safe_user}\" \"{scope}\"))\n"));
        }
    }

    spl_blocks.push(SplBlock {
        source_file: PathBuf::from("<runtime>"),
        source_page: String::from("<acl-runtime>"),
        start_line: 1,
        end_line: runtime_facts.lines().count() as u32,
        content: runtime_facts,
    });

    // 4. Build theory and reason
    let result =
        build_theory(&spl_blocks).context("ACL pipeline: failed to build asset access theory")?;

    // 5. Check conclusion
    for conclusion in &result.conclusions {
        let lit = &conclusion.literal;
        if !lit.contains(predicate) {
            continue;
        }
        let matches = if predicate == "can-upload" {
            literal_matches_1arg(lit, predicate, user_id)
        } else {
            literal_matches_2arg(lit, predicate, user_id, "*")
        };
        if !matches {
            continue;
        }
        let is_positive = matches!(
            conclusion.conclusion_type,
            ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
        );
        return Ok(is_positive);
    }

    Ok(false)
}

/// Check whether `user_id` has the `can-upload` permission (REQ-3504).
pub fn check_can_upload(vault_root: &Path, user_id: &str, is_agent: bool) -> Result<bool> {
    let defaults = built_in_upload_defaults(user_id);
    evaluate_asset_predicate(vault_root, user_id, "can-upload", &defaults, true, is_agent)
}

/// Check whether the user (or anonymous) can read assets (REQ-3516).
pub fn check_can_read_assets(
    vault_root: &Path,
    user_id: Option<&str>,
    visibility_mode: VisibilityMode,
    is_agent: bool,
) -> Result<bool> {
    let (effective_user, is_authenticated) = match user_id {
        Some(uid) => (uid, true),
        None => ("anonymous", false),
    };
    let defaults = built_in_read_assets_defaults(effective_user, visibility_mode);
    evaluate_asset_predicate(
        vault_root,
        effective_user,
        "can-read-assets",
        &defaults,
        is_authenticated,
        is_agent,
    )
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
        assert!(
            decision.is_allowed(),
            "owner should be able to read any page"
        );

        let edit_q = AclQuery {
            action: Action::Edit,
            ..read_q
        };
        let decision = evaluate(&vault, &edit_q, &[], &["secret/internal".to_string()]).unwrap();
        assert!(
            decision.is_allowed(),
            "owner should be able to edit any page"
        );
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
            "(given (role \"{bob_id}\" editor))\n(given (scope \"{bob_id}\" \"projects/*\"))\n"
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
        let access_spl =
            format!("(given (role \"{bob_id}\" editor))\n(given (scope \"{bob_id}\" \"**\"))\n");
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // Page-level override: defeater argues for negation to block reads
        let page_override = format!(
            "(except d-secret-block\n  (authenticated \"{bob_id}\")\n  (not (can-read \"{bob_id}\" \"secret\")))\n\
             (prefer d-secret-block r-default-read)\n\
             (prefer d-secret-block r-scoped-edit-read)\n"
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
        let access_spl =
            format!("(given (admin \"{agent_id}\"))\n(given (role \"{agent_id}\" admin))\n");
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

    // ── Owner hardening tests (REQ-020-058) ─────────────────────────────

    #[test]
    fn strip_owner_facts_removes_owner_from_content() {
        let mut block = SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<access-policy>".to_string(),
            start_line: 1,
            end_line: 3,
            content: "(given (role \"bob\" editor))\n(given (owner \"bob\"))\n(given (scope \"bob\" \"*\"))\n".to_string(),
        };

        strip_owner_facts(&mut block);

        assert!(
            !block.content.contains("owner"),
            "owner fact should be stripped"
        );
        assert!(
            block.content.contains("role"),
            "non-owner facts should remain"
        );
        assert!(
            block.content.contains("scope"),
            "non-owner facts should remain"
        );
    }

    #[test]
    fn strip_owner_facts_handles_various_formats() {
        let mut block = SplBlock {
            source_file: PathBuf::from("page.md"),
            source_page: "page".to_string(),
            start_line: 10,
            end_line: 14,
            content: "  (given (owner \"alice\"))\n(given (owner \"bob-123\"))\n(given (admin \"carol\"))\n".to_string(),
        };

        strip_owner_facts(&mut block);

        assert!(
            !block.content.contains("owner"),
            "all owner facts should be stripped"
        );
        assert!(block.content.contains("admin"), "admin facts should remain");
    }

    #[test]
    fn strip_owner_facts_noop_when_no_owner() {
        let original = "(given (role \"bob\" editor))\n(given (scope \"bob\" \"*\"))\n";
        let mut block = SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<access-policy>".to_string(),
            start_line: 1,
            end_line: 2,
            content: original.to_string(),
        };

        strip_owner_facts(&mut block);

        assert_eq!(block.content, original, "content should be unchanged");
    }

    #[test]
    fn owner_fact_in_access_spl_is_stripped_during_evaluate() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Malicious access.spl tries to grant Bob owner status
        let access_spl =
            format!("(given (owner \"{bob_id}\"))\n(given (role \"{bob_id}\" reader))\n",);
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // Bob should NOT be able to edit (owner fact was stripped)
        let q = AclQuery {
            user_id: bob_id,
            page_slug: "secret".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[], &["secret".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "injected owner fact in access.spl should be stripped — Bob must not get owner privileges"
        );
    }

    #[test]
    fn owner_fact_in_page_spl_is_stripped_during_evaluate() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Malicious page SPL tries to grant Bob owner status
        let page_block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 5,
            end_line: 6,
            content: format!("(given (owner \"{bob_id}\"))\n"),
        };

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "evil".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[page_block], &["evil".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "injected owner fact in page SPL should be stripped — Bob must not get owner privileges"
        );
    }

    // ── Temporal interval parsing tests ──────────────────────────────────

    #[test]
    fn parse_temporal_intervals_extracts_declarations() {
        let spl = r#"
(given (during (conference-access) 1742428800000 1742688000000))
(given (during (sprint-review) 1000 2000))
(given (role "bob" editor))
"#;
        let intervals = parse_temporal_intervals(spl);
        assert_eq!(intervals.len(), 2);
        assert_eq!(intervals[0].name, "conference-access");
        assert_eq!(intervals[0].start_ms, 1742428800000);
        assert_eq!(intervals[0].end_ms, 1742688000000);
        assert_eq!(intervals[1].name, "sprint-review");
        assert_eq!(intervals[1].start_ms, 1000);
        assert_eq!(intervals[1].end_ms, 2000);
    }

    #[test]
    fn parse_temporal_intervals_empty_when_none() {
        let spl = "(given (role \"bob\" editor))\n";
        let intervals = parse_temporal_intervals(spl);
        assert!(intervals.is_empty());
    }

    #[test]
    fn temporal_relation_before() {
        let interval = TemporalInterval {
            name: "test".into(),
            start_ms: 1000,
            end_ms: 2000,
        };
        assert_eq!(temporal_relation(500, &interval), TemporalRelation::Before);
    }

    #[test]
    fn temporal_relation_meets() {
        let interval = TemporalInterval {
            name: "test".into(),
            start_ms: 1000,
            end_ms: 2000,
        };
        assert_eq!(temporal_relation(1000, &interval), TemporalRelation::Meets);
    }

    #[test]
    fn temporal_relation_within() {
        let interval = TemporalInterval {
            name: "test".into(),
            start_ms: 1000,
            end_ms: 2000,
        };
        assert_eq!(temporal_relation(1500, &interval), TemporalRelation::Within);
    }

    #[test]
    fn temporal_relation_finishes() {
        let interval = TemporalInterval {
            name: "test".into(),
            start_ms: 1000,
            end_ms: 2000,
        };
        assert_eq!(
            temporal_relation(2000, &interval),
            TemporalRelation::Finishes
        );
    }

    #[test]
    fn temporal_relation_after() {
        let interval = TemporalInterval {
            name: "test".into(),
            start_ms: 1000,
            end_ms: 2000,
        };
        assert_eq!(temporal_relation(2500, &interval), TemporalRelation::After);
    }

    #[test]
    fn ground_temporal_facts_injects_within_when_active() {
        let blocks = vec![SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<test>".into(),
            start_line: 1,
            end_line: 1,
            content: "(given (during (conf) 1000 2000))\n".into(),
        }];
        let facts = ground_temporal_facts(&blocks, 1500);
        assert!(facts.contains("(given (now-within conf))"));
        assert!(facts.contains("(given (active conf))"));
        assert!(!facts.contains("now-before"));
        assert!(!facts.contains("now-after"));
    }

    #[test]
    fn ground_temporal_facts_injects_before_when_early() {
        let blocks = vec![SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<test>".into(),
            start_line: 1,
            end_line: 1,
            content: "(given (during (conf) 1000 2000))\n".into(),
        }];
        let facts = ground_temporal_facts(&blocks, 500);
        assert!(facts.contains("(given (now-before conf))"));
        assert!(!facts.contains("active"));
        assert!(!facts.contains("now-within"));
    }

    #[test]
    fn ground_temporal_facts_injects_after_when_expired() {
        let blocks = vec![SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<test>".into(),
            start_line: 1,
            end_line: 1,
            content: "(given (during (conf) 1000 2000))\n".into(),
        }];
        let facts = ground_temporal_facts(&blocks, 3000);
        assert!(facts.contains("(given (now-after conf))"));
        assert!(!facts.contains("active"));
        assert!(!facts.contains("now-within"));
    }

    #[test]
    fn ground_temporal_facts_meets_boundary() {
        let blocks = vec![SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<test>".into(),
            start_line: 1,
            end_line: 1,
            content: "(given (during (conf) 1000 2000))\n".into(),
        }];
        // At start boundary: meets + active + within
        let facts = ground_temporal_facts(&blocks, 1000);
        assert!(facts.contains("(given (now-meets conf))"));
        assert!(facts.contains("(given (active conf))"));
        assert!(facts.contains("(given (now-within conf))"));
    }

    #[test]
    fn ground_temporal_facts_finishes_boundary() {
        let blocks = vec![SplBlock {
            source_file: PathBuf::from("access.spl"),
            source_page: "<test>".into(),
            start_line: 1,
            end_line: 1,
            content: "(given (during (conf) 1000 2000))\n".into(),
        }];
        // At end boundary: finishes + active + within
        let facts = ground_temporal_facts(&blocks, 2000);
        assert!(facts.contains("(given (now-finishes conf))"));
        assert!(facts.contains("(given (active conf))"));
        assert!(facts.contains("(given (now-within conf))"));
    }

    // ── Temporal ACL integration tests (TEST-020-011) ────────────────────

    #[test]
    fn temporal_access_allowed_during_interval() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Grant Bob reader role + temporal read access during interval [1000, 2000]
        let access_spl = format!(
            r#"(given (role "{bob_id}" reader))
(given (during (temp-grant) 1000 2000))
(normally r-temp-read
  (and (role "{bob_id}" reader) (active temp-grant))
  (can-edit "{bob_id}" "notes"))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // At time 1500 (within interval) → allowed
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "notes".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: 1500,
        };
        let decision = evaluate(&vault, &q, &[], &["notes".to_string()]).unwrap();
        assert!(
            decision.is_allowed(),
            "temporal access should be allowed during the interval"
        );
    }

    #[test]
    fn temporal_access_denied_after_interval() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Grant Bob reader role + temporal edit access during interval [1000, 2000]
        let access_spl = format!(
            r#"(given (role "{bob_id}" reader))
(given (during (temp-grant) 1000 2000))
(normally r-temp-edit
  (and (role "{bob_id}" reader) (active temp-grant))
  (can-edit "{bob_id}" "notes"))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // At time 2001 (after interval) → denied
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "notes".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: 2001,
        };
        let decision = evaluate(&vault, &q, &[], &["notes".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "temporal access should be denied after the interval expires"
        );
    }

    #[test]
    fn temporal_access_denied_before_interval() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        let access_spl = format!(
            r#"(given (role "{bob_id}" reader))
(given (during (temp-grant) 1000 2000))
(normally r-temp-edit
  (and (role "{bob_id}" reader) (active temp-grant))
  (can-edit "{bob_id}" "notes"))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // At time 500 (before interval) → denied
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "notes".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: 500,
        };
        let decision = evaluate(&vault, &q, &[], &["notes".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "temporal access should be denied before the interval starts"
        );
    }

    #[test]
    fn temporal_within_fact_usable_in_rules() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Use (now-within ...) directly in rule body
        let access_spl = format!(
            r#"(given (role "{bob_id}" reader))
(given (during (review-window) 5000 10000))
(normally r-review-edit
  (and (role "{bob_id}" reader) (now-within review-window))
  (can-edit "{bob_id}" "review"))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // Within interval → allowed
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "review".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: 7500,
        };
        let decision = evaluate(&vault, &q, &[], &["review".to_string()]).unwrap();
        assert!(
            decision.is_allowed(),
            "(now-within ...) fact should enable temporal rule"
        );

        // After interval → denied
        let q2 = AclQuery {
            now_epoch_ms: 10001,
            ..q
        };
        let decision2 = evaluate(&vault, &q2, &[], &["review".to_string()]).unwrap();
        assert!(
            !decision2.is_allowed(),
            "(now-within ...) should not be present after interval"
        );
    }

    // ── Visibility mode tests (TEST-020-030) ─────────────────────────

    #[test]
    fn default_visibility_mode_is_mixed() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let mode = query_visibility_mode(&vault);
        assert_eq!(mode, VisibilityMode::Mixed);
    }

    #[test]
    fn visibility_mode_override_to_hidden() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        std::fs::write(
            vault.join(".zetl/collab/access.spl"),
            "(given (visibility-mode hidden))\n",
        )
        .unwrap();
        let mode = query_visibility_mode(&vault);
        assert_eq!(mode, VisibilityMode::Hidden);
    }

    #[test]
    fn visibility_mode_override_to_transparent() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        std::fs::write(
            vault.join(".zetl/collab/access.spl"),
            "(given (visibility-mode transparent))\n",
        )
        .unwrap();
        let mode = query_visibility_mode(&vault);
        assert_eq!(mode, VisibilityMode::Transparent);
    }

    #[test]
    fn effective_visibility_force_hidden_overrides_transparent() {
        let result = effective_visibility(
            VisibilityMode::Transparent,
            PageVisibilityOverride::ForceHidden,
        );
        assert_eq!(result, VisibilityMode::Hidden);
    }

    #[test]
    fn effective_visibility_force_visible_overrides_hidden() {
        let result =
            effective_visibility(VisibilityMode::Hidden, PageVisibilityOverride::ForceVisible);
        assert_eq!(result, VisibilityMode::Transparent);
    }

    #[test]
    fn effective_visibility_no_override_uses_mode() {
        assert_eq!(
            effective_visibility(VisibilityMode::Mixed, PageVisibilityOverride::None),
            VisibilityMode::Mixed
        );
        assert_eq!(
            effective_visibility(VisibilityMode::Hidden, PageVisibilityOverride::None),
            VisibilityMode::Hidden
        );
    }

    // ── Deontic modality tests (TEST-020-012) ────────────────────────────

    #[test]
    fn deontic_forbidden_overrides_base_allowed() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Grant Bob editor on all pages, but forbid editing "Audit Log"
        // SPL syntax: (forbidden (pred args...)) maps to [F] modality
        let access_spl = format!(
            r#"(given (role "{bob_id}" editor))
(given (scope "{bob_id}" "**"))
(always s-no-audit-edit
  (authenticated "{bob_id}")
  (forbidden (edit "{bob_id}" "Audit Log")))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        // Bob can edit regular pages
        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "notes".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let decision = evaluate(
            &vault,
            &q,
            &[],
            &["notes".to_string(), "Audit Log".to_string()],
        )
        .unwrap();
        assert!(
            decision.is_allowed(),
            "editor should be able to edit regular pages"
        );

        // Bob is forbidden from editing Audit Log (deontic override)
        let q2 = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "Audit Log".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let decision2 = evaluate(
            &vault,
            &q2,
            &[],
            &["notes".to_string(), "Audit Log".to_string()],
        )
        .unwrap();
        assert!(
            !decision2.is_allowed(),
            "[F] should override base can-edit to deny access"
        );
    }

    #[test]
    fn deontic_forbidden_with_theory_returns_overlay() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        let access_spl = format!(
            r#"(given (role "{bob_id}" editor))
(given (scope "{bob_id}" "**"))
(always s-no-audit
  (authenticated "{bob_id}")
  (forbidden (edit "{bob_id}" "Audit Log")))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "Audit Log".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let (decision, overlay, _theory) =
            evaluate_with_theory(&vault, &q, &[], &["Audit Log".to_string()]).unwrap();

        assert!(!decision.is_allowed(), "should be denied by [F] override");
        assert!(
            !overlay.forbidden.is_empty(),
            "overlay should contain [F] conclusion"
        );
        assert_eq!(overlay.forbidden[0].modality, DeonticModality::Forbidden);
        assert_eq!(overlay.forbidden[0].predicate, "edit");
        assert!(overlay.has_active_forbidden());
    }

    #[test]
    fn deontic_permission_explicit_but_redundant() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Grant Bob editor + explicit [P] permission
        // SPL syntax: (may (pred args...)) maps to [P] modality
        let access_spl = format!(
            r#"(given (role "{bob_id}" editor))
(given (scope "{bob_id}" "**"))
(normally r-explicit-perm
  (authenticated "{bob_id}")
  (may (edit "{bob_id}" "docs")))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "docs".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let (decision, overlay, _theory) =
            evaluate_with_theory(&vault, &q, &[], &["docs".to_string()]).unwrap();

        assert!(decision.is_allowed(), "[P] should not prevent access");
        assert!(
            !overlay.permitted.is_empty(),
            "overlay should contain [P] conclusion"
        );
        assert_eq!(overlay.permitted[0].modality, DeonticModality::Permission);
    }

    #[test]
    fn deontic_obligation_informational_only() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Bob is obligated to review a page, but can still read it
        // SPL syntax: (must (pred args...)) maps to [O] modality
        let access_spl = format!(
            r#"(given (role "{bob_id}" editor))
(given (scope "{bob_id}" "**"))
(normally r-review-oblig
  (authenticated "{bob_id}")
  (must (review "{bob_id}" "flagged-page")))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "flagged-page".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let (decision, overlay, _theory) =
            evaluate_with_theory(&vault, &q, &[], &["flagged-page".to_string()]).unwrap();

        assert!(decision.is_allowed(), "[O] should not block read access");
        assert!(
            !overlay.obligations.is_empty(),
            "overlay should contain [O] conclusion"
        );
        assert_eq!(overlay.obligations[0].modality, DeonticModality::Obligation);
        assert_eq!(overlay.obligations[0].predicate, "review");
    }

    #[test]
    fn deontic_overlay_empty_when_no_modalities() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");

        let q = AclQuery {
            user_id: owner_id.clone(),
            page_slug: "plain-page".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let (decision, overlay, _theory) =
            evaluate_with_theory(&vault, &q, &[], &["plain-page".to_string()]).unwrap();

        assert!(decision.is_allowed());
        assert!(
            overlay.is_empty(),
            "overlay should be empty when no deontic modalities used"
        );
    }

    #[test]
    fn deontic_forbidden_read_blocks_read() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Forbid Bob from reading a classified page
        // SPL syntax: (forbidden (pred args...)) maps to [F] modality
        let access_spl = format!(
            r#"(always s-no-classified-read
  (authenticated "{bob_id}")
  (forbidden (read "{bob_id}" "classified")))
"#
        );
        std::fs::write(vault.join(".zetl/collab/access.spl"), &access_spl).unwrap();

        let q = AclQuery {
            user_id: bob_id.clone(),
            page_slug: "classified".to_string(),
            action: Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };
        let decision = evaluate(&vault, &q, &[], &["classified".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "[F](read ...) should override base default-read to deny"
        );
    }

    #[test]
    fn deontic_modality_display() {
        assert_eq!(DeonticModality::Permission.to_string(), "[P]");
        assert_eq!(DeonticModality::Forbidden.to_string(), "[F]");
        assert_eq!(DeonticModality::Obligation.to_string(), "[O]");
    }

    #[test]
    fn deontic_overlay_has_active_forbidden_false_when_not_provable() {
        let overlay = DeonticOverlay {
            forbidden: vec![DeonticConclusion {
                modality: DeonticModality::Forbidden,
                predicate: "edit".to_string(),
                tag: ConclusionTag::DefeasiblyNotProvable,
                rule_trace: vec![],
            }],
            permitted: vec![],
            obligations: vec![],
        };
        assert!(
            !overlay.has_active_forbidden(),
            "not-provable [F] should not count as active"
        );
    }

    // ── Page-level SPL sandbox tests (REQ-020-059) ────────────────────────

    #[test]
    fn sandbox_strips_admin_fact() {
        let mut block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 1,
            end_line: 2,
            content: "(given (admin \"bob\"))\n(given (authenticated \"bob\"))\n".to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            !block.content.contains("admin"),
            "admin fact should be stripped"
        );
        assert!(
            block.content.contains("authenticated"),
            "non-global facts should remain"
        );
    }

    #[test]
    fn sandbox_strips_role_fact() {
        let mut block = SplBlock {
            source_file: PathBuf::from("page.md"),
            source_page: "page".to_string(),
            start_line: 1,
            end_line: 1,
            content: "(given (role \"bob\" admin))\n".to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            !block.content.contains("role"),
            "role fact should be stripped"
        );
    }

    #[test]
    fn sandbox_strips_scope_fact() {
        let mut block = SplBlock {
            source_file: PathBuf::from("page.md"),
            source_page: "page".to_string(),
            start_line: 1,
            end_line: 1,
            content: "(given (scope \"bob\" \"**\"))\n".to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            !block.content.contains("scope"),
            "scope fact should be stripped"
        );
    }

    #[test]
    fn sandbox_strips_visibility_mode_fact() {
        let mut block = SplBlock {
            source_file: PathBuf::from("page.md"),
            source_page: "page".to_string(),
            start_line: 1,
            end_line: 1,
            content: "(given (visibility-mode transparent))\n".to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            !block.content.contains("visibility-mode"),
            "visibility-mode fact should be stripped"
        );
    }

    #[test]
    fn sandbox_strips_is_agent_fact() {
        let mut block = SplBlock {
            source_file: PathBuf::from("page.md"),
            source_page: "page".to_string(),
            start_line: 1,
            end_line: 1,
            content: "(given (is-agent \"bot-123\"))\n".to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            !block.content.contains("is-agent"),
            "is-agent fact should be stripped"
        );
    }

    #[test]
    fn sandbox_strips_all_global_predicates_at_once() {
        let mut block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 1,
            end_line: 6,
            content: "(given (admin \"bob\"))\n\
                      (given (role \"bob\" admin))\n\
                      (given (scope \"bob\" \"**\"))\n\
                      (given (visibility-mode hidden))\n\
                      (given (is-agent \"bob\"))\n\
                      (given (authenticated \"bob\"))\n"
                .to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(!block.content.contains("admin"));
        assert!(!block.content.contains("role"));
        assert!(!block.content.contains("scope"));
        assert!(!block.content.contains("visibility-mode"));
        assert!(!block.content.contains("is-agent"));
        assert!(
            block.content.contains("authenticated"),
            "non-global facts survive"
        );
    }

    #[test]
    fn sandbox_rejects_cross_page_access_conclusion() {
        let mut block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 1,
            end_line: 3,
            content:
                "(normally r-steal\n  (authenticated \"bob\")\n  (can-edit \"bob\" \"secret\"))\n"
                    .to_string(),
        };
        sandbox_page_spl(&mut block);
        assert!(
            block.content.is_empty(),
            "block with cross-page conclusion should be fully rejected"
        );
    }

    #[test]
    fn sandbox_allows_own_page_access_conclusion() {
        let content =
            "(normally r-local\n  (authenticated \"bob\")\n  (can-read \"bob\" \"my-page\"))\n"
                .to_string();
        let mut block = SplBlock {
            source_file: PathBuf::from("my-page.md"),
            source_page: "my-page".to_string(),
            start_line: 1,
            end_line: 3,
            content: content.clone(),
        };
        sandbox_page_spl(&mut block);
        assert_eq!(
            block.content, content,
            "own-page conclusion should survive sandbox"
        );
    }

    #[test]
    fn sandbox_noop_when_clean() {
        let content =
            "(except d-block\n  (authenticated \"bob\")\n  (not (can-read \"bob\" \"notes\")))\n\
                       (prefer d-block r-default-read)\n"
                .to_string();
        let mut block = SplBlock {
            source_file: PathBuf::from("notes.md"),
            source_page: "notes".to_string(),
            start_line: 5,
            end_line: 8,
            content: content.clone(),
        };
        sandbox_page_spl(&mut block);
        assert_eq!(block.content, content, "clean block should be unchanged");
    }

    #[test]
    fn sandbox_global_predicate_in_page_spl_denied_during_evaluate() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Malicious page SPL tries to grant Bob admin status
        let page_block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 1,
            end_line: 2,
            content: format!("(given (admin \"{bob_id}\"))\n(given (role \"{bob_id}\" admin))\n"),
        };

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "evil".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(&vault, &q, &[page_block], &["evil".to_string()]).unwrap();
        assert!(
            !decision.is_allowed(),
            "admin/role facts in page SPL should be stripped — Bob must not get admin privileges"
        );
    }

    #[test]
    fn sandbox_cross_page_conclusion_denied_during_evaluate() {
        let tmp = TempDir::new().unwrap();
        let vault = setup_vault(&tmp);
        let owner_id = create_owner(&vault, "Alice");
        let bob_id = create_user(&vault, "Bob", &owner_id);

        // Malicious page SPL tries to grant Bob edit on "secret" from page "evil"
        let page_block = SplBlock {
            source_file: PathBuf::from("evil.md"),
            source_page: "evil".to_string(),
            start_line: 1,
            end_line: 3,
            content: format!(
                "(normally r-steal\n  (authenticated \"{bob_id}\")\n  (can-edit \"{bob_id}\" \"secret\"))\n"
            ),
        };

        let q = AclQuery {
            user_id: bob_id,
            page_slug: "secret".to_string(),
            action: Action::Edit,
            is_agent: false,
            now_epoch_ms: now_ms(),
        };

        let decision = evaluate(
            &vault,
            &q,
            &[page_block],
            &["evil".to_string(), "secret".to_string()],
        )
        .unwrap();
        assert!(
            !decision.is_allowed(),
            "cross-page conclusion should be rejected — evil.md cannot grant access to secret"
        );
    }
}
