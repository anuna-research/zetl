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
}
