use std::borrow::Cow;
use std::path::{Path, PathBuf};

use include_dir::{include_dir, Dir};
use minijinja::{context, Environment};

use super::context::{FolderContext, PageContext, VaultContext};

// ── Bundled themes ──────────────────────────────────────────────────────────

static BUNDLED_THEMES: Dir = include_dir!("$CARGO_MANIFEST_DIR/themes");

/// Look up a template from the compile-time-embedded themes directory.
///
/// Returns the UTF-8 content of `themes/<theme>/<name>` if it exists,
/// or `None` when the theme or template file is not found.
pub fn bundled_template(theme: &str, name: &str) -> Option<&'static str> {
    BUNDLED_THEMES
        .get_file(format!("{theme}/{name}"))
        .and_then(|f| f.contents_utf8())
}

/// Return the names of all theme directories embedded at compile time.
pub fn bundled_theme_names() -> Vec<&'static str> {
    BUNDLED_THEMES
        .dirs()
        .map(|d| d.path().file_name().and_then(|n| n.to_str()).unwrap_or(""))
        .filter(|n| !n.is_empty())
        .collect()
}

/// Return hook files from a bundled theme's `hooks/` subdirectory.
///
/// Returns `(hook_name, content_bytes)` pairs for each file under
/// `themes/<theme>/hooks/`. Returns an empty vec if the theme has no hooks
/// or is not found in the embedded bundle.
pub fn bundled_theme_hook_files(theme: &str) -> Vec<(String, Vec<u8>)> {
    let all = bundled_theme_files(theme);
    all.into_iter()
        .filter_map(|(path, content)| {
            let path_str = path.to_string_lossy();
            if let Some(name) = path_str.strip_prefix("hooks/") {
                if !name.is_empty() && !name.contains('/') {
                    return Some((name.to_string(), content));
                }
            }
            None
        })
        .collect()
}

/// Return all files in a bundled theme as (relative_path, content_bytes) pairs.
///
/// Paths are relative to the theme root (e.g., `"page.html"`). Returns an
/// empty vec if the named theme is not found in the embedded bundle.
pub fn bundled_theme_files(theme: &str) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let Some(dir) = BUNDLED_THEMES.get_dir(theme) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    collect_dir_files(dir, theme, &mut files);
    files
}

fn collect_dir_files(
    dir: &include_dir::Dir<'_>,
    strip_prefix: &str,
    out: &mut Vec<(std::path::PathBuf, Vec<u8>)>,
) {
    for file in dir.files() {
        let rel = file
            .path()
            .strip_prefix(strip_prefix)
            .unwrap_or(file.path())
            .to_path_buf();
        out.push((rel, file.contents().to_vec()));
    }
    for sub in dir.dirs() {
        collect_dir_files(sub, strip_prefix, out);
    }
}

// ── TemplateError ──────────────────────────────────────────────────────────

/// Structured template error with context extracted from minijinja.
///
/// Carries the template name, line number, error kind, and human-readable
/// message so that callers can produce rich diagnostics (HTML error page in
/// serve mode, formatted stderr in build mode).
#[derive(Debug, Clone)]
pub struct TemplateError {
    pub template_name: Option<String>,
    pub line: Option<usize>,
    pub kind: String,
    pub message: String,
}

impl TemplateError {
    fn from_minijinja(err: minijinja::Error) -> Self {
        Self {
            template_name: err.name().map(|s| s.to_string()),
            line: err.line(),
            kind: format!("{:?}", err.kind()),
            message: err.to_string(),
        }
    }

    fn empty_output(template_name: &str) -> Self {
        Self {
            template_name: Some(template_name.to_string()),
            line: None,
            kind: "EmptyOutput".to_string(),
            message: format!("template '{template_name}' produced empty output"),
        }
    }

    /// Format for build-mode stderr: includes the page/slug being rendered.
    pub fn stderr_line(&self, slug: &str) -> String {
        let loc = match (&self.template_name, self.line) {
            (Some(name), Some(line)) => format!("{name}:{line}"),
            (Some(name), None) => name.clone(),
            _ => "unknown".to_string(),
        };
        format!(
            "error: template {loc}: {msg} (rendering '{slug}')",
            msg = self.message
        )
    }

    /// Build a self-contained HTML error page for serve mode.
    pub fn to_error_html(&self) -> String {
        let esc = |s: &str| -> String {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };

        let template_info = match (&self.template_name, self.line) {
            (Some(name), Some(line)) => format!(
                r#"<span class="label">Template:</span> <span class="value">{}</span> <span class="label">Line:</span> <span class="value">{}</span>"#,
                esc(name),
                line
            ),
            (Some(name), None) => format!(
                r#"<span class="label">Template:</span> <span class="value">{}</span>"#,
                esc(name)
            ),
            _ => String::new(),
        };

        format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Template Error — zetl</title>
<style>
  body {{ font-family: ui-monospace, "Cascadia Code", "Source Code Pro", Menlo, monospace; background: #1a1b26; color: #a9b1d6; margin: 0; padding: 2rem; }}
  .error-box {{ max-width: 720px; margin: 3rem auto; background: #24283b; border-left: 4px solid #f7768e; border-radius: 6px; padding: 1.5rem 2rem; }}
  h1 {{ color: #f7768e; font-size: 1.3rem; margin: 0 0 1rem; font-weight: 600; }}
  .meta {{ margin-bottom: 1rem; font-size: 0.85rem; }}
  .label {{ color: #565f89; }}
  .value {{ color: #7aa2f7; background: #1a1b26; padding: 2px 6px; border-radius: 3px; }}
  pre {{ background: #1a1b26; color: #c0caf5; padding: 1rem; border-radius: 4px; overflow-x: auto; white-space: pre-wrap; word-wrap: break-word; font-size: 0.85rem; line-height: 1.6; margin: 0; }}
</style>
</head>
<body>
<div class="error-box">
  <h1>Template Error</h1>
  <div class="meta">{template_info}</div>
  <pre>{message}</pre>
</div>
</body>
</html>"##,
            template_info = template_info,
            message = esc(&self.message),
        )
    }
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(name) = &self.template_name {
            write!(f, "{name}")?;
            if let Some(line) = self.line {
                write!(f, ":{line}")?;
            }
            write!(f, ": ")?;
        }
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TemplateError {}

// ── TemplateEngine ─────────────────────────────────────────────────────────

/// Template engine wrapping a minijinja::Environment with three-tier template resolution.
///
/// Templates resolve in order:
/// 1. `.zetl/themes/<theme>/<name>` on disk (skipped when theme is "default")
/// 2. Bundled theme matching the active theme name (compile-time embed)
/// 3. Bundled `default` theme as final fallback (compile-time embed)
///
/// When `reload` is true (serve mode), a fresh Environment is built for each render
/// call so that on-disk template edits take effect immediately. When false (build mode),
/// templates are cached in the Environment for the lifetime of the engine.
pub struct TemplateEngine {
    cached_env: Environment<'static>,
    vault_root: PathBuf,
    theme: String,
    reload: bool,
}

const KNOWN_TEMPLATES: &[&str] = &["base.html", "index.html", "page.html", "folder.html"];

/// Build a minijinja Environment with the three-tier template loader.
fn build_env(vault_root: &Path, theme: &str) -> Environment<'static> {
    let mut env = Environment::new();
    let vr = vault_root.to_path_buf();
    let t = theme.to_string();
    env.set_loader(move |name: &str| {
        // Tier 1: check active theme directory on disk (skip for "default")
        if t != "default" {
            let theme_path = vr.join(".zetl/themes").join(&t).join(name);
            if let Ok(content) = std::fs::read_to_string(&theme_path) {
                return Ok(Some(content));
            }
        }
        // Tier 2: check bundled theme for the active theme name
        if let Some(content) = bundled_template(&t, name) {
            return Ok(Some(content.to_string()));
        }
        // Tier 3: fall back to built-in default theme embedded at compile time
        Ok(bundled_template("default", name).map(|s| s.to_string()))
    });
    env
}

impl TemplateEngine {
    /// Create a new TemplateEngine with two-tier template resolution.
    ///
    /// - `vault_root`: path to the vault directory (for locating `.zetl/themes/`)
    /// - `theme`: active theme name ("default" skips disk lookup entirely)
    /// - `reload`: when true (serve mode), rebuild the environment on every render;
    ///   when false (build mode), cache templates for the engine's lifetime
    /// - `verbose`: when true, log which templates loaded from theme dir vs built-in
    pub fn new(vault_root: &Path, theme: &str, reload: bool, verbose: bool) -> Self {
        if verbose {
            for name in KNOWN_TEMPLATES {
                if theme != "default" {
                    let theme_path = vault_root.join(".zetl/themes").join(theme).join(name);
                    if theme_path.exists() {
                        eprintln!("  theme: {name} <- .zetl/themes/{theme}/{name} (disk)");
                        continue;
                    }
                }
                if bundled_template(theme, name).is_some() {
                    eprintln!("  theme: {name} <- bundled:{theme}/{name} (bundled)");
                } else {
                    eprintln!("  theme: {name} <- bundled:default/{name} (fallback)");
                }
            }
        }

        let cached_env = build_env(vault_root, theme);
        Self {
            cached_env,
            vault_root: vault_root.to_path_buf(),
            theme: theme.to_string(),
            reload,
        }
    }

    /// Get the environment to use for rendering. In reload mode, builds a fresh
    /// environment each time; otherwise returns a reference to the cached one.
    fn env(&self) -> Cow<'_, Environment<'static>> {
        if self.reload {
            Cow::Owned(build_env(&self.vault_root, &self.theme))
        } else {
            Cow::Borrowed(&self.cached_env)
        }
    }

    /// Render the vault index page.
    pub fn render_index(
        &self,
        vault_ctx: &VaultContext,
        mode: &str,
        bm25_index: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let root_path = compute_root_path(mode, "");
        let idx_file = index_file(mode);
        let ctx = context! {
            vault => vault_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
            root_path => root_path,
            index_file => idx_file,
            bm25_index => bm25_index,
        };
        let env = self.env();
        let tmpl = env
            .get_template("index.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("index.html"));
        }
        Ok(html)
    }

    /// Render a single page.
    pub fn render_page(
        &self,
        vault_ctx: &VaultContext,
        page_ctx: &PageContext,
        mode: &str,
        bm25_index: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let root_path = compute_root_path(mode, &page_ctx.slug);
        let idx_file = index_file(mode);
        let ctx = context! {
            vault => vault_ctx,
            page => page_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => &page_ctx.slug,
            root_path => root_path,
            index_file => idx_file,
            bm25_index => bm25_index,
        };
        let env = self.env();
        let tmpl = env
            .get_template("page.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("page.html"));
        }
        Ok(html)
    }

    /// Render a folder index page.
    pub fn render_folder(
        &self,
        vault_ctx: &VaultContext,
        folder_ctx: &FolderContext,
        mode: &str,
        bm25_index: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let root_path = compute_root_path(mode, &folder_ctx.slug);
        let idx_file = index_file(mode);
        let ctx = context! {
            vault => vault_ctx,
            folder => folder_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
            root_path => root_path,
            index_file => idx_file,
            bm25_index => bm25_index,
        };
        let env = self.env();
        let tmpl = env
            .get_template("folder.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("folder.html"));
        }
        Ok(html)
    }
}

/// Compute a relative root path for use in template links.
///
/// In serve mode, returns `"/"` (absolute). In build mode, returns a relative
/// path based on slug depth: `"./"` for the root index, `"../" * N` for nested
/// pages/folders so that `href="{{ root_path }}{{ slug }}/"` resolves correctly
/// even when opened via `file://`.
fn compute_root_path(mode: &str, slug: &str) -> String {
    if mode == "serve" {
        "/".to_string()
    } else if slug.is_empty() {
        "./".to_string()
    } else {
        let depth = slug.split('/').count();
        "../".repeat(depth)
    }
}

/// Returns `"index.html"` in build mode (needed for `file://` protocol which
/// doesn't auto-resolve directory indexes) and `""` in serve mode.
fn index_file(mode: &str) -> &'static str {
    if mode == "serve" {
        ""
    } else {
        "index.html"
    }
}

/// Build a JSON search index string from vault pages for the Cmd+K search modal.
fn build_search_index(vault_ctx: &VaultContext) -> String {
    let entries: Vec<String> = vault_ctx
        .pages
        .iter()
        .map(|p| {
            format!(
                r#"{{"n":"{}","s":"{}"}}"#,
                json_escape(&p.title),
                json_escape(&p.slug),
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

/// Minimal JSON string escaping for search index values.
fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::context::{PageEntry, StatsContext};
    use std::path::Path;

    fn sample_vault() -> VaultContext {
        VaultContext {
            name: "test-vault".to_string(),
            pages: vec![PageEntry {
                title: "Hello".to_string(),
                slug: "hello".to_string(),
                outlink_count: 1,
                backlink_count: 0,
                extension: "md".to_string(),
            }],
            stats: StatsContext {
                total_pages: 1,
                total_links: 1,
                dead_links: 0,
                orphans: 0,
            },
        }
    }

    fn sample_page() -> PageContext {
        PageContext {
            title: "Hello".to_string(),
            slug: "hello".to_string(),
            content_html: "<p>world</p>".to_string(),
            content_raw: "world".to_string(),
            frontmatter: serde_json::json!({}),
            backlinks: vec![],
            outlinks: vec![],
            breadcrumbs: vec![],
            transclusion_cards: String::new(),
            is_new: false,
            raw_escaped: None,
        }
    }

    fn default_engine() -> TemplateEngine {
        TemplateEngine::new(Path::new("."), "default", false, false)
    }

    #[test]
    fn test_render_index() {
        let engine = default_engine();
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html.contains("Vault"));
        assert!(html.contains("Hello"));
    }

    #[test]
    fn test_render_page() {
        let engine = default_engine();
        let vault = sample_vault();
        let page = sample_page();
        let html = engine.render_page(&vault, &page, "static", "").unwrap();
        assert!(html.contains("Hello"));
        assert!(html.contains("<p>world</p>"));
    }

    #[test]
    fn test_render_folder() {
        let engine = default_engine();
        let vault = sample_vault();
        let folder = FolderContext {
            name: "docs".to_string(),
            slug: "docs".to_string(),
            breadcrumbs: vec![],
            subfolders: vec![],
            pages: vec![],
            total_pages: 0,
        };
        let html = engine.render_folder(&vault, &folder, "serve", "").unwrap();
        assert!(html.contains("docs"));
        assert!(html.contains("0 pages in this folder"));
    }

    #[test]
    fn test_search_index_escaping() {
        let vault = VaultContext {
            name: "vault".to_string(),
            pages: vec![PageEntry {
                title: r#"He said "hello""#.to_string(),
                slug: "test".to_string(),
                outlink_count: 0,
                backlink_count: 0,
                extension: "md".to_string(),
            }],
            stats: StatsContext {
                total_pages: 1,
                total_links: 0,
                dead_links: 0,
                orphans: 0,
            },
        };
        let idx = build_search_index(&vault);
        assert!(idx.contains(r#"\"hello\""#));
    }

    #[test]
    fn test_theme_variable_in_context() {
        let engine = TemplateEngine::new(Path::new("."), "fountain", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html.contains(r#"data-theme="fountain""#));
    }

    #[test]
    fn test_default_theme_skips_disk() {
        // "default" theme should work without any .zetl/themes directory
        let engine = TemplateEngine::new(Path::new("/nonexistent"), "default", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html.contains("Vault"));
    }

    #[test]
    fn test_theme_disk_override() {
        // Create a temp dir with a custom theme that overrides page.html
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/custom");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(
            theme_dir.join("page.html"),
            r#"{% extends "base.html" %}{% block title %}CUSTOM: {{ page.title }}{% endblock %}{% block content %}<div class="custom">{{ page.content_html }}</div>{% endblock %}"#,
        )
        .unwrap();

        let engine = TemplateEngine::new(tmp.path(), "custom", false, false);
        let vault = sample_vault();
        let page = sample_page();
        let html = engine.render_page(&vault, &page, "static", "").unwrap();
        // Custom template wraps content in <div class="custom">
        assert!(html.contains(r#"<div class="custom">"#));
        // base.html is still the built-in (cross-tier inheritance)
        assert!(html.contains("CUSTOM: Hello"));
    }

    #[test]
    fn test_reload_mode_picks_up_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/live");
        std::fs::create_dir_all(&theme_dir).unwrap();

        // Start with custom index template
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V1{% endblock %}{% block content %}VERSION1{% endblock %}"#,
        )
        .unwrap();

        let engine = TemplateEngine::new(tmp.path(), "live", true, false);
        let vault = sample_vault();

        let html1 = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html1.contains("VERSION1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}VERSION2{% endblock %}"#,
        )
        .unwrap();

        // Reload mode should pick up the change
        let html2 = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html2.contains("VERSION2"));
    }

    #[test]
    fn test_cached_mode_does_not_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/cached");
        std::fs::create_dir_all(&theme_dir).unwrap();

        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V1{% endblock %}{% block content %}CACHED_V1{% endblock %}"#,
        )
        .unwrap();

        let engine = TemplateEngine::new(tmp.path(), "cached", false, false);
        let vault = sample_vault();

        let html1 = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html1.contains("CACHED_V1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}CACHED_V2{% endblock %}"#,
        )
        .unwrap();

        // Cached mode should still return the old version
        let html2 = engine.render_index(&vault, "serve", "").unwrap();
        assert!(html2.contains("CACHED_V1"));
    }

    // ── TemplateError tests ────────────────────────────────────────────────

    #[test]
    fn test_syntax_error_returns_template_error() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/broken");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(
            theme_dir.join("index.html"),
            "{% extends 'base.html' %}{% block content %}{{ unclosed }",
        )
        .unwrap();

        let engine = TemplateEngine::new(tmp.path(), "broken", false, false);
        let vault = sample_vault();
        let err = engine.render_index(&vault, "serve", "").unwrap_err();
        assert!(err.template_name.is_some());
        assert!(err.message.len() > 0);
    }

    #[test]
    fn test_error_html_contains_details() {
        let err = TemplateError {
            template_name: Some("page.html".to_string()),
            line: Some(42),
            kind: "SyntaxError".to_string(),
            message: "unexpected end of template".to_string(),
        };
        let html = err.to_error_html();
        assert!(html.contains("page.html"));
        assert!(html.contains("42"));
        assert!(html.contains("unexpected end of template"));
        assert!(html.contains("Template Error"));
    }

    #[test]
    fn test_error_html_escapes_html() {
        let err = TemplateError {
            template_name: Some("<script>".to_string()),
            line: None,
            kind: "SyntaxError".to_string(),
            message: "bad <tag> & stuff".to_string(),
        };
        let html = err.to_error_html();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("bad &lt;tag&gt; &amp; stuff"));
    }

    #[test]
    fn test_stderr_line_format() {
        let err = TemplateError {
            template_name: Some("page.html".to_string()),
            line: Some(10),
            kind: "SyntaxError".to_string(),
            message: "syntax error".to_string(),
        };
        let line = err.stderr_line("my-page");
        assert!(line.contains("page.html:10"));
        assert!(line.contains("my-page"));
        assert!(line.starts_with("error:"));
    }

    #[test]
    fn test_stderr_line_without_line_number() {
        let err = TemplateError {
            template_name: Some("index.html".to_string()),
            line: None,
            kind: "EmptyOutput".to_string(),
            message: "template 'index.html' produced empty output".to_string(),
        };
        let line = err.stderr_line("index");
        // Without a line number, the location should just be "index.html" (no ":N" suffix)
        assert!(line.contains("template index.html:"));
        assert!(!line.contains("index.html:1"));
        assert!(line.contains("'index'"));
    }

    #[test]
    fn test_empty_output_caught() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/empty");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("index.html"), "   ").unwrap();

        let engine = TemplateEngine::new(tmp.path(), "empty", false, false);
        let vault = sample_vault();
        let err = engine.render_index(&vault, "serve", "").unwrap_err();
        assert_eq!(err.kind, "EmptyOutput");
        assert!(err.message.contains("empty output"));
    }

    // ── bundled_template / bundled_theme_names tests ───────────────────────

    #[test]
    fn test_bundled_template_default_exists() {
        for name in KNOWN_TEMPLATES {
            assert!(
                bundled_template("default", name).is_some(),
                "missing bundled default template: {name}"
            );
        }
    }

    #[test]
    fn test_bundled_template_unknown_returns_none() {
        assert!(bundled_template("default", "nonexistent.html").is_none());
        assert!(bundled_template("nosuchtheme", "page.html").is_none());
    }

    #[test]
    fn test_bundled_theme_names_contains_default() {
        let names = bundled_theme_names();
        assert!(
            names.contains(&"default"),
            "expected 'default' in bundled_theme_names(), got: {names:?}"
        );
    }

    #[test]
    fn test_display_with_name_and_line() {
        let err = TemplateError {
            template_name: Some("page.html".to_string()),
            line: Some(5),
            kind: "SyntaxError".to_string(),
            message: "unexpected token".to_string(),
        };
        let s = format!("{err}");
        assert_eq!(s, "page.html:5: unexpected token");
    }

    #[test]
    fn test_display_without_name() {
        let err = TemplateError {
            template_name: None,
            line: None,
            kind: "Unknown".to_string(),
            message: "something failed".to_string(),
        };
        let s = format!("{err}");
        assert_eq!(s, "something failed");
    }

    // ── bundled_theme_hook_files tests ───────────────────────────────────

    #[test]
    fn test_bundled_theme_hook_files_unknown_theme() {
        let hooks = bundled_theme_hook_files("nosuchtheme");
        assert!(hooks.is_empty());
    }

    #[test]
    fn test_bundled_theme_hook_files_no_hooks_subdir() {
        // The default bundled theme has no hooks/ subdirectory,
        // so this should return an empty vec.
        let hooks = bundled_theme_hook_files("default");
        assert!(hooks.is_empty());
    }
}
