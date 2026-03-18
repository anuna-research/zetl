//! Vault context serialiser for hook stdin (CON-016-001).
//!
//! Builds the JSON object written to each hook's stdin, containing:
//! - hook name, vault_root, theme, zetl_version
//! - pages array with name, path, slug, frontmatter, outlinks, backlinks, is_orphan
//! - stats with total_pages, total_links, dead_links, orphans

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::graph::{DeadLink, LinkGraph, Orphan};
use crate::scanner::page_slug_from_path;
use crate::types::{Diagnostic, ParsedFile};
use crate::web::markdown::parse_frontmatter;

/// Base hook context written to stdin (CON-016-001).
#[derive(Debug, Serialize)]
pub struct HookContext {
    /// Hook lifecycle point name (e.g. `"post-build"`).
    pub hook: String,
    /// Absolute path to the vault root.
    pub vault_root: String,
    /// Active theme name (empty string if none).
    pub theme: String,
    /// zetl version string.
    pub zetl_version: String,
    /// All pages in the vault.
    pub pages: Vec<HookPageEntry>,
    /// Aggregate vault statistics.
    pub stats: HookStats,
    /// Vault snapshot history summary (REQ-090).
    /// `null` (JSON) when history is unavailable.
    pub history: serde_json::Value,
    /// Output directory for build hooks (only present for post-build).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_dir: Option<String>,
    /// Number of pages rendered during build (only present for post-build).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages_rendered: Option<usize>,
    /// Server port (only present for pre-serve).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Diagnostics collected during check (only present for post-check).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<HookDiagnostics>,
    /// Saved file info (only present for on-save).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved: Option<HookSaved>,
    /// Authenticated user identity; `null` for unauthenticated CLI operations.
    pub user: Option<HookUser>,
    /// Hook invocation depth for loop prevention (REQ-020-020).
    /// Starts at 0 for the initial event; incremented on each hook invocation.
    pub hook_depth: u32,
    /// Agent task context (only present for on-agent hooks, REQ-020-023).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<HookAgent>,
    /// Access request context (only present for on-access-request hooks, REQ-020-047).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_request: Option<HookAccessRequest>,
    /// ACL violations detected during post-reconciliation (only present for on-acl-violation hooks, REQ-020-043).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acl_violations: Option<HookAclViolations>,
}

/// User identity attached to hook context when an authenticated session exists.
#[derive(Debug, Clone, Serialize)]
pub struct HookUser {
    /// User ID (e.g. `"alice-a1b2c3d4"`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this identity represents an agent token rather than a human.
    pub is_agent: bool,
    /// Roles assigned to this user (e.g. `["admin"]`).
    pub roles: Vec<String>,
    /// Whether this edit originated outside zetl (REQ-020-042).
    /// `true` for git commit authors and filesystem-detected edits.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_external: bool,
}

/// Payload describing the file that was just saved (on-save hooks).
#[derive(Debug, Serialize)]
pub struct HookSaved {
    /// Relative path from vault root (e.g. `"notes/My Page.md"`).
    pub file: String,
    /// Page name (filename stem without extension).
    pub page: String,
    /// Length of the saved content in bytes.
    pub content_length: usize,
    /// Whether this save was detected as an external edit (REQ-020-042).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_external: bool,
}

/// Agent task context for on-agent hooks (REQ-020-023).
#[derive(Debug, Serialize)]
pub struct HookAgent {
    /// Agent task name (the `<name>` from `zetl agent run <name>`).
    pub task: String,
    /// Pages the agent should operate on (empty = vault-wide).
    pub target_pages: Vec<String>,
    /// Token budget for the agent action (0 = unlimited).
    pub budget_tokens: u32,
}

/// Access request context for on-access-request hooks (REQ-020-047).
#[derive(Debug, Serialize)]
pub struct HookAccessRequest {
    /// User ID of the requester.
    pub user_id: String,
    /// Display name of the requester.
    pub user_name: String,
    /// Page slug that was requested.
    pub page: String,
    /// ISO-8601 timestamp of the request.
    pub requested_at: String,
}

/// A single ACL violation detected during post-reconciliation (REQ-020-043).
#[derive(Debug, Clone, Serialize)]
pub struct HookAclViolationEntry {
    /// Page slug that was edited in violation of policy.
    pub page: String,
    /// User ID that made the edit (empty string if unknown/unattributable).
    pub user_id: String,
    /// The action that was denied (`"edit"`).
    pub action: String,
    /// Human-readable reason from the ACL decision.
    pub reason: String,
}

/// ACL violation context for on-acl-violation hooks (REQ-020-043).
#[derive(Debug, Clone, Serialize)]
pub struct HookAclViolations {
    /// The violations detected in this reconciliation pass.
    pub violations: Vec<HookAclViolationEntry>,
}

/// Diagnostics payload for post-check hooks.
#[derive(Debug, Serialize)]
pub struct HookDiagnostics {
    /// Links pointing to non-existent pages.
    pub dead_links: Vec<DeadLink>,
    /// Pages with zero incoming links.
    pub orphans: Vec<Orphan>,
    /// Markdown syntax errors.
    pub syntax_errors: Vec<Diagnostic>,
}

/// A single page in the hook context.
#[derive(Debug, Serialize)]
pub struct HookPageEntry {
    /// Page name (filename without extension).
    pub name: String,
    /// Relative path from vault root (e.g. `"folder/PageName.md"`).
    pub path: String,
    /// URL slug form (e.g. `"folder/pagename"`).
    pub slug: String,
    /// Parsed YAML frontmatter as a JSON object.
    pub frontmatter: serde_json::Value,
    /// Names of pages this page links to.
    pub outlinks: Vec<String>,
    /// Names of pages that link to this page.
    pub backlinks: Vec<String>,
    /// Whether this page has zero incoming links.
    pub is_orphan: bool,
}

/// Aggregate statistics for the hook context.
#[derive(Debug, Serialize)]
pub struct HookStats {
    pub total_pages: usize,
    pub total_links: usize,
    pub dead_links: usize,
    pub orphans: usize,
}

impl HookUser {
    /// Create a `HookUser` for an external git commit author (REQ-020-042).
    pub fn external_git(name: &str, email: &str) -> Self {
        HookUser {
            id: format!("external:{email}"),
            name: name.to_string(),
            is_agent: false,
            roles: vec![],
            is_external: true,
        }
    }

    /// Create a `HookUser` for an external filesystem edit (REQ-020-042).
    pub fn external_filesystem() -> Self {
        HookUser {
            id: "external:filesystem".to_string(),
            name: "(external)".to_string(),
            is_agent: false,
            roles: vec![],
            is_external: true,
        }
    }

    /// Create a `HookUser` from a `UserProfile`.
    pub fn from_profile(profile: &crate::user::UserProfile, is_agent: bool) -> Self {
        let role = crate::user::Role::for_profile(profile);
        HookUser {
            id: profile.id.clone(),
            name: profile.name.clone(),
            is_agent,
            roles: vec![role.to_string()],
            is_external: false,
        }
    }
}

/// Build the base hook context JSON (CON-016-001).
///
/// Reads frontmatter from disk for each page. If a file cannot be read,
/// frontmatter falls back to an empty object.
pub fn build_hook_context(
    hook_name: &str,
    vault_root: &Path,
    theme: &str,
    zetl_version: &str,
    files: &[ParsedFile],
    graph: &LinkGraph,
) -> HookContext {
    let graph_stats = graph.stats(0);

    // Pre-compute orphan set for O(1) lookup.
    let orphan_set: HashSet<String> = graph.orphans().into_iter().map(|o| o.page).collect();

    let pages: Vec<HookPageEntry> = files
        .iter()
        .map(|file| {
            let slug = page_slug_from_path(&file.path);

            // Read file content for frontmatter parsing.
            let frontmatter = std::fs::read_to_string(vault_root.join(&file.path))
                .map(|content| parse_frontmatter(&content))
                .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

            let outlinks: Vec<String> = graph
                .forward_links(&file.page_name)
                .into_iter()
                .map(|fl| fl.target)
                .collect();

            let backlinks: Vec<String> = graph
                .backlinks(&file.page_name)
                .into_iter()
                .map(|bl| bl.source)
                .collect();

            let is_orphan = orphan_set.contains(&file.page_name);

            HookPageEntry {
                name: file.page_name.clone(),
                path: file.path.to_string_lossy().into_owned(),
                slug,
                frontmatter,
                outlinks,
                backlinks,
                is_orphan,
            }
        })
        .collect();

    let stats = HookStats {
        total_pages: graph_stats.pages,
        total_links: graph_stats.links,
        dead_links: graph_stats.dead_links,
        orphans: graph_stats.orphans,
    };

    #[cfg(feature = "history")]
    let history = crate::history::build_hook_history_context(vault_root);
    #[cfg(not(feature = "history"))]
    let history = serde_json::Value::Null;

    HookContext {
        hook: hook_name.to_string(),
        vault_root: vault_root.to_string_lossy().into_owned(),
        theme: theme.to_string(),
        zetl_version: zetl_version.to_string(),
        pages,
        stats,
        history,
        out_dir: None,
        pages_rendered: None,
        port: None,
        diagnostics: None,
        saved: None,
        user: None,
        hook_depth: 0,
        agent: None,
        access_request: None,
        acl_violations: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ParsedFile;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use tempfile::TempDir;

    /// Helper: create a minimal ParsedFile.
    fn make_parsed_file(name: &str, rel_path: &str) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(rel_path),
            page_name: name.to_string(),
            links: vec![],
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    #[test]
    fn empty_vault_produces_valid_context() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context(
            "post-build",
            tmp.path(),
            "fountain",
            "0.1.0",
            &files,
            &graph,
        );

        assert_eq!(ctx.hook, "post-build");
        assert_eq!(ctx.theme, "fountain");
        assert_eq!(ctx.zetl_version, "0.1.0");
        assert!(ctx.pages.is_empty());
        assert_eq!(ctx.stats.total_pages, 0);
        assert_eq!(ctx.stats.total_links, 0);
        assert_eq!(ctx.stats.dead_links, 0);
        assert_eq!(ctx.stats.orphans, 0);

        // Must serialise to valid JSON.
        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.is_object());
        assert!(val["pages"].is_array());
    }

    #[test]
    fn single_page_no_links() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Hello.md"),
            "---\ntitle: Hello World\ntags:\n  - test\n---\nSome content.\n",
        )
        .unwrap();

        let files = vec![make_parsed_file("Hello", "Hello.md")];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-index", tmp.path(), "", "0.2.0", &files, &graph);

        assert_eq!(ctx.pages.len(), 1);
        let page = &ctx.pages[0];
        assert_eq!(page.name, "Hello");
        assert_eq!(page.path, "Hello.md");
        assert_eq!(page.slug, "hello");
        assert!(page.outlinks.is_empty());
        assert!(page.backlinks.is_empty());
        assert!(page.is_orphan); // no incoming links

        // Frontmatter should have title and tags.
        assert_eq!(page.frontmatter["title"], "Hello World");
        assert!(page.frontmatter["tags"].is_array());

        assert_eq!(ctx.stats.total_pages, 1);
        assert_eq!(ctx.stats.orphans, 1);
    }

    #[test]
    fn pages_with_links() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Alpha.md"), "Link to [[Beta]]\n").unwrap();
        std::fs::write(tmp.path().join("Beta.md"), "No links here.\n").unwrap();

        let mut alpha = make_parsed_file("Alpha", "Alpha.md");
        alpha.links.push(crate::types::WikiLink {
            target_page: "Beta".to_string(),
            raw_target: "Beta".to_string(),
            heading: None,
            block_ref: None,
            alias: None,
            is_embed: false,
            line: 1,
            column: 9,
        });
        let beta = make_parsed_file("Beta", "Beta.md");

        let mut resolved: HashMap<String, String> = HashMap::new();
        resolved.insert("beta".to_string(), "Beta".to_string());

        let files = vec![alpha, beta];
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "minimal", "0.1.0", &files, &graph);

        assert_eq!(ctx.pages.len(), 2);

        let alpha_page = ctx.pages.iter().find(|p| p.name == "Alpha").unwrap();
        assert_eq!(alpha_page.outlinks, vec!["Beta"]);
        assert!(alpha_page.backlinks.is_empty());
        // Alpha has no incoming links → orphan.
        assert!(alpha_page.is_orphan);

        let beta_page = ctx.pages.iter().find(|p| p.name == "Beta").unwrap();
        assert!(beta_page.outlinks.is_empty());
        assert_eq!(beta_page.backlinks, vec!["Alpha"]);
        // Beta has an incoming link from Alpha → not orphan.
        assert!(!beta_page.is_orphan);

        assert_eq!(ctx.stats.total_pages, 2);
        assert_eq!(ctx.stats.total_links, 1);
        assert_eq!(ctx.stats.dead_links, 0);
        assert_eq!(ctx.stats.orphans, 1); // Alpha is orphan
    }

    #[test]
    fn dead_link_counted_in_stats() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Page.md"), "Link to [[Missing]]\n").unwrap();

        let mut page = make_parsed_file("Page", "Page.md");
        page.links.push(crate::types::WikiLink {
            target_page: "Missing".to_string(),
            raw_target: "Missing".to_string(),
            heading: None,
            block_ref: None,
            alias: None,
            is_embed: false,
            line: 1,
            column: 9,
        });

        let files = vec![page];
        // No resolved mapping for "Missing" → dead link.
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-check", tmp.path(), "", "0.1.0", &files, &graph);

        assert_eq!(ctx.stats.dead_links, 1);
        assert_eq!(ctx.stats.total_links, 1);
    }

    #[test]
    fn frontmatter_fallback_on_missing_file() {
        let tmp = TempDir::new().unwrap();
        // Don't create the file on disk — simulates a deleted file.
        let files = vec![make_parsed_file("Gone", "Gone.md")];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);

        let page = &ctx.pages[0];
        assert!(page.frontmatter.is_object());
        assert!(page.frontmatter.as_object().unwrap().is_empty());
    }

    #[test]
    fn context_serialises_to_valid_json() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Note.md"), "---\ntitle: Test\n---\nBody\n").unwrap();

        let files = vec![make_parsed_file("Note", "Note.md")];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("on-save", tmp.path(), "dark", "1.0.0", &files, &graph);
        let json = serde_json::to_string_pretty(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify all required top-level keys from CON-016-001.
        assert!(val["hook"].is_string());
        assert!(val["vault_root"].is_string());
        assert!(val["theme"].is_string());
        assert!(val["zetl_version"].is_string());
        assert!(val["pages"].is_array());
        assert!(val["stats"].is_object());
        // history is always present: null when history feature is absent.
        assert!(val.get("history").is_some());

        // Verify page fields.
        let page = &val["pages"][0];
        assert!(page["name"].is_string());
        assert!(page["path"].is_string());
        assert!(page["slug"].is_string());
        assert!(page["frontmatter"].is_object());
        assert!(page["outlinks"].is_array());
        assert!(page["backlinks"].is_array());
        assert!(page["is_orphan"].is_boolean());

        // Verify stats fields.
        let stats = &val["stats"];
        assert!(stats["total_pages"].is_number());
        assert!(stats["total_links"].is_number());
        assert!(stats["dead_links"].is_number());
        assert!(stats["orphans"].is_number());
    }

    #[test]
    fn vault_root_is_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("pre-build", tmp.path(), "", "0.1.0", &files, &graph);

        // vault_root in context should match the provided path.
        assert_eq!(ctx.vault_root, tmp.path().to_string_lossy());
    }

    #[test]
    fn subdirectory_page_path_and_slug() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("notes");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("My Note.md"), "content\n").unwrap();

        let files = vec![make_parsed_file("My Note", "notes/My Note.md")];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);

        let page = &ctx.pages[0];
        assert_eq!(page.path, "notes/My Note.md");
        assert_eq!(page.slug, "notes/my-note");
    }

    #[test]
    fn diagnostics_absent_by_default() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);
        assert!(ctx.diagnostics.is_none());

        // Ensure "diagnostics" key is absent from JSON when None.
        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("diagnostics").is_none());
    }

    #[test]
    fn diagnostics_serialises_for_post_check() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("Page.md"), "Link to [[Missing]]\n").unwrap();

        let mut page = make_parsed_file("Page", "Page.md");
        page.links.push(crate::types::WikiLink {
            target_page: "Missing".to_string(),
            raw_target: "Missing".to_string(),
            heading: None,
            block_ref: None,
            alias: None,
            is_embed: false,
            line: 1,
            column: 9,
        });
        page.diagnostics.push(crate::types::Diagnostic {
            level: crate::types::DiagnosticLevel::Error,
            message: "bad syntax".to_string(),
            file: PathBuf::from("Page.md"),
            line: 3,
            column: 1,
        });

        let files = vec![page];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context("post-check", tmp.path(), "", "0.1.0", &files, &graph);

        // Attach diagnostics like cmd_check does.
        let dead_links = graph.dead_links();
        let orphans = graph.orphans();
        let syntax_errors: Vec<crate::types::Diagnostic> =
            files.iter().flat_map(|f| f.diagnostics.clone()).collect();

        ctx.diagnostics = Some(HookDiagnostics {
            dead_links,
            orphans,
            syntax_errors,
        });

        let json = serde_json::to_string_pretty(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        let diag = &val["diagnostics"];
        assert!(diag.is_object());

        // dead_links array with one entry.
        let dl = &diag["dead_links"];
        assert!(dl.is_array());
        assert_eq!(dl.as_array().unwrap().len(), 1);
        assert_eq!(dl[0]["source"], "Page");
        assert_eq!(dl[0]["target"], "Missing");

        // orphans array (Page has no incoming links).
        let orph = &diag["orphans"];
        assert!(orph.is_array());
        assert_eq!(orph.as_array().unwrap().len(), 1);
        assert_eq!(orph[0]["page"], "Page");

        // syntax_errors array with one entry.
        let se = &diag["syntax_errors"];
        assert!(se.is_array());
        assert_eq!(se.as_array().unwrap().len(), 1);
        assert_eq!(se[0]["message"], "bad syntax");
    }

    #[test]
    fn user_null_by_default() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);
        assert!(ctx.user.is_none());

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val["user"].is_null());
    }

    #[test]
    fn user_serialises_when_set() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context("on-save", tmp.path(), "", "0.1.0", &files, &graph);
        ctx.user = Some(HookUser {
            id: "alice-a1b2c3d4".to_string(),
            name: "Alice".to_string(),
            is_agent: false,
            roles: vec!["admin".to_string()],
            is_external: false,
        });

        let json = serde_json::to_string_pretty(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        let user = &val["user"];
        assert!(user.is_object());
        assert_eq!(user["id"], "alice-a1b2c3d4");
        assert_eq!(user["name"], "Alice");
        assert_eq!(user["is_agent"], false);
        assert!(user["roles"].is_array());
        assert_eq!(user["roles"][0], "admin");
    }

    #[test]
    fn hook_depth_defaults_to_zero() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("on-save", tmp.path(), "", "0.1.0", &files, &graph);
        assert_eq!(ctx.hook_depth, 0);

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["hook_depth"], 0);
    }

    #[test]
    fn hook_depth_serialises_when_set() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context("on-save", tmp.path(), "", "0.1.0", &files, &graph);
        ctx.hook_depth = 3;

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["hook_depth"], 3);
    }

    #[test]
    fn hook_user_from_profile_owner() {
        let profile = crate::user::UserProfile {
            id: "alice-a1b2c3d4".to_string(),
            name: "Alice".to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: None,
            owner: true,
            credentials: vec![],
            recovery_pubkey: "dGVzdA".to_string(),
            agent_token_generation: 0,
        };

        let hook_user = HookUser::from_profile(&profile, false);
        assert_eq!(hook_user.id, "alice-a1b2c3d4");
        assert_eq!(hook_user.name, "Alice");
        assert!(!hook_user.is_agent);
        assert_eq!(hook_user.roles, vec!["admin"]);
    }

    #[test]
    fn hook_user_from_profile_editor() {
        let profile = crate::user::UserProfile {
            id: "bob-12345678".to_string(),
            name: "Bob".to_string(),
            created_at: "2026-03-18T10:00:00Z".to_string(),
            invited_by: Some("alice-a1b2c3d4".to_string()),
            owner: false,
            credentials: vec![],
            recovery_pubkey: "dGVzdA".to_string(),
            agent_token_generation: 0,
        };

        let hook_user = HookUser::from_profile(&profile, true);
        assert_eq!(hook_user.id, "bob-12345678");
        assert_eq!(hook_user.name, "Bob");
        assert!(hook_user.is_agent);
        assert_eq!(hook_user.roles, vec!["reader"]);
    }

    #[test]
    fn agent_absent_by_default() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);
        assert!(ctx.agent.is_none());

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("agent").is_none());
    }

    #[test]
    fn agent_serialises_when_set() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context("on-agent", tmp.path(), "", "0.1.0", &files, &graph);
        ctx.agent = Some(HookAgent {
            task: "link-checker".to_string(),
            target_pages: vec!["Note A".to_string(), "Note B".to_string()],
            budget_tokens: 4000,
        });

        let json = serde_json::to_string_pretty(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        let agent = &val["agent"];
        assert!(agent.is_object());
        assert_eq!(agent["task"], "link-checker");
        assert_eq!(agent["target_pages"].as_array().unwrap().len(), 2);
        assert_eq!(agent["target_pages"][0], "Note A");
        assert_eq!(agent["target_pages"][1], "Note B");
        assert_eq!(agent["budget_tokens"], 4000);
    }

    #[test]
    fn agent_empty_target_pages() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context("on-agent", tmp.path(), "", "0.1.0", &files, &graph);
        ctx.agent = Some(HookAgent {
            task: "summariser".to_string(),
            target_pages: vec![],
            budget_tokens: 0,
        });

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        let agent = &val["agent"];
        assert_eq!(agent["task"], "summariser");
        assert!(agent["target_pages"].as_array().unwrap().is_empty());
        assert_eq!(agent["budget_tokens"], 0);
    }

    #[test]
    fn acl_violations_absent_by_default() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let ctx = build_hook_context("post-build", tmp.path(), "", "0.1.0", &files, &graph);
        assert!(ctx.acl_violations.is_none());

        let json = serde_json::to_string(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("acl_violations").is_none());
    }

    #[test]
    fn acl_violations_serialises_when_set() {
        let tmp = TempDir::new().unwrap();
        let files: Vec<ParsedFile> = vec![];
        let resolved: HashMap<String, String> = HashMap::new();
        let graph = LinkGraph::build(&files, &resolved);

        let mut ctx = build_hook_context(
            "on-acl-violation",
            tmp.path(),
            "",
            "0.1.0",
            &files,
            &graph,
        );
        ctx.acl_violations = Some(HookAclViolations {
            violations: vec![
                HookAclViolationEntry {
                    page: "secret".to_string(),
                    user_id: "bob-12345678".to_string(),
                    action: "edit".to_string(),
                    reason: "policy denied".to_string(),
                },
            ],
        });

        let json = serde_json::to_string_pretty(&ctx).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();

        let violations = &val["acl_violations"];
        assert!(violations.is_object());

        let entries = violations["violations"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["page"], "secret");
        assert_eq!(entries[0]["user_id"], "bob-12345678");
        assert_eq!(entries[0]["action"], "edit");
        assert_eq!(entries[0]["reason"], "policy denied");
    }

    #[test]
    fn hook_user_external_git() {
        let user = HookUser::external_git("Bot", "bot@ci.example.com");
        assert_eq!(user.id, "external:bot@ci.example.com");
        assert_eq!(user.name, "Bot");
        assert!(!user.is_agent);
        assert!(user.roles.is_empty());
        assert!(user.is_external);

        let json = serde_json::to_string(&user).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["is_external"], true);
    }

    #[test]
    fn hook_user_external_filesystem() {
        let user = HookUser::external_filesystem();
        assert_eq!(user.id, "external:filesystem");
        assert_eq!(user.name, "(external)");
        assert!(user.is_external);

        let json = serde_json::to_string(&user).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["is_external"], true);
    }

    #[test]
    fn is_external_skipped_when_false() {
        let user = HookUser {
            id: "alice".to_string(),
            name: "Alice".to_string(),
            is_agent: false,
            roles: vec![],
            is_external: false,
        };
        let json = serde_json::to_string(&user).unwrap();
        assert!(!json.contains("is_external"));
    }

    #[test]
    fn hook_saved_is_external_serialisation() {
        let saved = HookSaved {
            file: "notes/page.md".to_string(),
            page: "page".to_string(),
            content_length: 42,
            is_external: true,
        };
        let json = serde_json::to_string(&saved).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["is_external"], true);

        // When false, is_external should be absent.
        let saved_internal = HookSaved {
            file: "notes/page.md".to_string(),
            page: "page".to_string(),
            content_length: 42,
            is_external: false,
        };
        let json_internal = serde_json::to_string(&saved_internal).unwrap();
        assert!(!json_internal.contains("is_external"));
    }
}
