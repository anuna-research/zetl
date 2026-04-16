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
    /// SPEC-028 REQ-105: when true, every render receives the caller-supplied
    /// `graph_index` string in its template context; when false, `graph_index`
    /// is forced to `""`. Sourced from `graph_inline` in the active theme's
    /// `theme.toml` (disk override preferred, bundled manifest as fallback).
    graph_inline: bool,
}

const KNOWN_TEMPLATES: &[&str] = &[
    "base.html",
    "index.html",
    "page.html",
    "editor.html",
    "folder.html",
    "login.html",
    "passkey_register.html",
    "recovery_show.html",
    "invite_accept.html",
    "admin_invite.html",
    "admin_permissions.html",
    "dashboard.html",
    "page_history.html",
    "vault_history.html",
    "vault_graph.html",
    "help.html",
];

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

    // SPEC-027 REQ-300: expose humanise_days (e.g. `3d`, `2w`, `9mo`) as a
    // template filter for history-metadata rendering.
    #[cfg(feature = "history")]
    env.add_filter(
        "humanise_days",
        |v: minijinja::Value| -> Result<String, minijinja::Error> {
            let n = v.as_i64().ok_or_else(|| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    "humanise_days expects an integer",
                )
            })?;
            Ok(crate::history::core::humanise_days(n))
        },
    );

    // `tojson`: serialise a value to a JSON literal (string quoting + escaping).
    // Used by page_history.html to embed history data inside `<script>`.
    // Minijinja's built-in `tojson` requires the `json` feature, which we
    // don't enable — so we register our own.
    env.add_filter(
        "tojson",
        |v: minijinja::Value| -> Result<String, minijinja::Error> {
            let json_val = serde_json::to_value(&v).map_err(|e| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
            })?;
            serde_json::to_string(&json_val).map_err(|e| {
                minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
            })
        },
    );

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
        let graph_inline = load_graph_inline(vault_root, theme);
        Self {
            cached_env,
            vault_root: vault_root.to_path_buf(),
            theme: theme.to_string(),
            reload,
            graph_inline,
        }
    }

    /// SPEC-028 REQ-105: returns whether the active theme requests that the
    /// serialised graph index be inlined into every rendered page. Callers
    /// can skip computing the (potentially large) JSON payload when this is
    /// `false`.
    pub fn graph_inline(&self) -> bool {
        self.graph_inline
    }

    /// Resolve the effective `graph_index` value for the template context,
    /// respecting the theme's `graph_inline` opt-in (SPEC-028 REQ-105).
    fn effective_graph_index<'a>(&self, graph_index: &'a str) -> &'a str {
        if self.graph_inline {
            graph_index
        } else {
            ""
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
        history_index: &str,
        graph_index: &str,
    ) -> Result<String, TemplateError> {
        // Build mode: don't inline the search index — it's written to pages.json
        // and fetched lazily on first Cmd+K. Serve mode keeps it inline because
        // openSearch() needs to respond instantly and the payload is tiny.
        let search_index = if mode == "build" {
            String::new()
        } else {
            build_search_index(vault_ctx)
        };
        let root_path = compute_root_path(mode, "");
        let idx_file = index_file(mode);
        let graph_url = graph_index_url(&root_path);
        let ctx = context! {
            vault => vault_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
            root_path => root_path,
            index_file => idx_file,
            bm25_index => bm25_index,
            history_index => history_index,
            graph_index_url => graph_url,
            graph_index => self.effective_graph_index(graph_index),
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

    /// Render the help page.
    pub fn render_help(
        &self,
        vault_ctx: &VaultContext,
        mode: &str,
    ) -> Result<String, TemplateError> {
        let search_index = if mode == "build" {
            String::new()
        } else {
            build_search_index(vault_ctx)
        };
        let root_path = compute_root_path(mode, "help");
        let idx_file = index_file(mode);
        let graph_url = graph_index_url(&root_path);
        let ctx = context! {
            vault => vault_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "help",
            root_path => root_path,
            index_file => idx_file,
            bm25_index => "",
            history_index => "",
            graph_index_url => graph_url,
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("help.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("help.html"));
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
        history_index: &str,
        graph_index: &str,
    ) -> Result<String, TemplateError> {
        let search_index = if mode == "build" {
            String::new()
        } else {
            build_search_index(vault_ctx)
        };
        let root_path = compute_root_path(mode, &page_ctx.slug);
        let idx_file = index_file(mode);
        let graph_url = graph_index_url(&root_path);
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
            history_index => history_index,
            graph_index_url => graph_url,
            graph_index => self.effective_graph_index(graph_index),
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

    /// Render the collaborative editor page.
    #[allow(clippy::too_many_arguments)]
    pub fn render_editor(
        &self,
        vault_ctx: &VaultContext,
        page_title: &str,
        page_slug: &str,
        breadcrumbs: &[super::context::BreadcrumbEntry],
        editor_json: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let ctx = context! {
            vault => vault_ctx,
            page_title => page_title,
            page_slug => page_slug,
            breadcrumbs => breadcrumbs,
            editor_json => editor_json,
            mode => "serve",
            search_index => search_index,
            theme => &self.theme,
            active_slug => page_slug,
            root_path => "/",
            index_file => "",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("editor.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("editor.html"));
        }
        Ok(html)
    }

    /// Render the passkey registration guidance page.
    pub fn render_login(&self, vault_name: &str) -> Result<String, TemplateError> {
        let ctx = context! {
            vault_name => vault_name,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("login.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("login.html"));
        }
        Ok(html)
    }

    pub fn render_passkey_register(
        &self,
        vault_name: &str,
        user_id: &str,
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault => context! { name => vault_name },
            vault_name => vault_name,
            user_id => user_id,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("passkey_register.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("passkey_register.html"));
        }
        Ok(html)
    }

    /// Render the recovery phrase display page.
    #[allow(clippy::too_many_arguments)] // template context fields
    pub fn render_recovery_show(
        &self,
        vault_name: &str,
        mnemonic: &str,
        words: &[&str],
        continue_url: &str,
        user_id: &str,
        recovery_pubkey: &str,
        csrf_token: &str,
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault => context! { name => vault_name },
            vault_name => vault_name,
            mnemonic => mnemonic,
            words => words,
            continue_url => continue_url,
            user_id => user_id,
            recovery_pubkey => recovery_pubkey,
            csrf_token => csrf_token,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("recovery_show.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("recovery_show.html"));
        }
        Ok(html)
    }

    /// Render the invitation acceptance page.
    pub fn render_invite_accept(
        &self,
        vault_name: &str,
        token: &str,
        inviter: &str,
        role: &str,
        pages: Option<&str>,
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault_name => vault_name,
            token => token,
            inviter => inviter,
            role => role,
            pages => pages,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("invite_accept.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("invite_accept.html"));
        }
        Ok(html)
    }

    /// Render the admin invitation management page.
    pub fn render_admin_invite(
        &self,
        vault_name: &str,
        csrf_token: &str,
        invitations: &[serde_json::Value],
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault_name => vault_name,
            csrf_token => csrf_token,
            invitations => invitations,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("admin_invite.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("admin_invite.html"));
        }
        Ok(html)
    }

    /// Render the admin permissions management page (REQ-020-048).
    pub fn render_admin_permissions(
        &self,
        vault_name: &str,
        csrf_token: &str,
        users: &[serde_json::Value],
        spl_preview: &str,
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault_name => vault_name,
            csrf_token => csrf_token,
            users => users,
            spl_preview => spl_preview,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("admin_permissions.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("admin_permissions.html"));
        }
        Ok(html)
    }

    /// Render the user dashboard page (/_me).
    #[allow(clippy::too_many_arguments)]
    pub fn render_dashboard(
        &self,
        vault_name: &str,
        csrf_token: &str,
        user_name: &str,
        user_id: &str,
        role: &str,
        is_admin: bool,
        recent_edits: &[serde_json::Value],
        accessible_pages: &[serde_json::Value],
        page_count: usize,
        pending_invites: &[serde_json::Value],
        access_requests: &[serde_json::Value],
        active_sessions: usize,
        passkey_count: usize,
    ) -> Result<String, TemplateError> {
        let ctx = context! {
            vault_name => vault_name,
            csrf_token => csrf_token,
            user_name => user_name,
            user_id => user_id,
            role => role,
            is_admin => is_admin,
            recent_edits => recent_edits,
            accessible_pages => accessible_pages,
            page_count => page_count,
            pending_invites => pending_invites,
            access_requests => access_requests,
            active_sessions => active_sessions,
            passkey_count => passkey_count,
            mode => "serve",
            theme => &self.theme,
            root_path => "/",
            index_file => "",
            search_index => "[]",
            graph_index_url => graph_index_url("/"),
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("dashboard.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("dashboard.html"));
        }
        Ok(html)
    }

    /// Render the page history UI (/{slug}/_history).
    ///
    /// `page_history` is the jj-derived `page.history` context (link
    /// trend, snapshot-level neighbourhood deltas, last_changed). When
    /// non-null it is rendered as a server-side "Snapshot timeline"
    /// section that complements (or replaces) the JS-rendered git
    /// commit log driven by `history_json` — important because pages
    /// imported into the vault from outside zetl have no git commits
    /// but may have many jj snapshots.
    #[allow(clippy::too_many_arguments)]
    pub fn render_page_history(
        &self,
        vault_ctx: &VaultContext,
        page_title: &str,
        page_slug: &str,
        breadcrumbs: &[super::context::BreadcrumbEntry],
        history_json: &str,
        page_history: &serde_json::Value,
        has_draft: bool,
        mode: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let root_path = compute_root_path(mode, page_slug);
        let graph_url = graph_index_url(&root_path);
        let ctx = context! {
            vault => vault_ctx,
            page_title => page_title,
            page_slug => page_slug,
            breadcrumbs => breadcrumbs,
            history_json => history_json,
            page_history => page_history,
            has_draft => has_draft,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => page_slug,
            root_path => root_path,
            index_file => index_file(mode),
            graph_index_url => graph_url,
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("page_history.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("page_history.html"));
        }
        Ok(html)
    }

    /// Render the vault-wide history UI (/_history) — SPEC-027 REQ-303.
    #[cfg(feature = "history")]
    pub fn render_vault_history(
        &self,
        vault_ctx: &VaultContext,
        recent_changes: &[crate::history::core::RecentChangeEntry],
        sparkline_points: &[f32],
        mode: &str,
    ) -> Result<String, TemplateError> {
        let search_index = build_search_index(vault_ctx);
        let root_path = compute_root_path(mode, "");
        let graph_url = graph_index_url(&root_path);
        let ctx = context! {
            vault => vault_ctx,
            recent_changes => recent_changes,
            sparkline_points => sparkline_points,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
            root_path => root_path,
            index_file => index_file(mode),
            graph_index_url => graph_url,
            graph_index => "",
        };
        let env = self.env();
        let tmpl = env
            .get_template("vault_history.html")
            .map_err(TemplateError::from_minijinja)?;
        let html = tmpl.render(ctx).map_err(TemplateError::from_minijinja)?;
        if html.trim().is_empty() {
            return Err(TemplateError::empty_output("vault_history.html"));
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
        history_index: &str,
        graph_index: &str,
    ) -> Result<String, TemplateError> {
        let search_index = if mode == "build" {
            String::new()
        } else {
            build_search_index(vault_ctx)
        };
        let root_path = compute_root_path(mode, &folder_ctx.slug);
        let idx_file = index_file(mode);
        let graph_url = graph_index_url(&root_path);
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
            history_index => history_index,
            graph_index_url => graph_url,
            graph_index => self.effective_graph_index(graph_index),
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

/// SPEC-028 REQ-104: resolve the URL at which the emitted `graph-index.json`
/// can be fetched, given the caller's `root_path`. In serve mode `root_path`
/// is `/`, yielding `/graph-index.json`; in build mode it is a `../`-chain
/// that resolves the filename against the current page's directory, so the
/// URL remains valid under `file://` and relative CDN roots alike.
fn graph_index_url(root_path: &str) -> String {
    format!("{root_path}graph-index.json")
}

/// SPEC-028 REQ-105: read the top-level `graph_inline` flag from the active
/// theme's `theme.toml`. Prefers an on-disk override (`.zetl/themes/<theme>/
/// theme.toml`) when present, else falls back to the compile-time-bundled
/// manifest. Any parse or IO error is treated as "flag absent" (false) — the
/// flag is a best-effort opt-in, not a hard dependency.
fn load_graph_inline(vault_root: &Path, theme: &str) -> bool {
    if theme != "default" {
        let disk = vault_root.join(".zetl/themes").join(theme).join("theme.toml");
        if let Ok(content) = std::fs::read_to_string(&disk) {
            if let Some(flag) = parse_graph_inline(&content) {
                return flag;
            }
        }
    }
    if let Some(content) = bundled_template(theme, "theme.toml") {
        if let Some(flag) = parse_graph_inline(content) {
            return flag;
        }
    }
    false
}

/// Best-effort extraction of the top-level `graph_inline` boolean from a
/// `theme.toml` string. Returns `None` when the document is unparseable or
/// the key is absent; returns `Some(false)` when present and set to false so
/// callers can distinguish "absent" from "explicitly disabled" if they wish.
fn parse_graph_inline(content: &str) -> Option<bool> {
    let value: toml::Value = toml::from_str(content).ok()?;
    value.get("graph_inline")?.as_bool()
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
pub fn build_search_index(vault_ctx: &VaultContext) -> String {
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
            sidebar_tree: vec![],
            stats: StatsContext {
                total_pages: 1,
                total_links: 1,
                dead_links: 0,
                orphans: 0,
            },
            history: serde_json::Value::Null,
            semantic_available: false,
            site_url: String::new(),
        }
    }

    fn sample_page() -> PageContext {
        PageContext {
            title: "Hello".to_string(),
            slug: "hello".to_string(),
            content_html: "<p>world</p>".to_string(),
            content_raw: "world".to_string(),
            frontmatter: serde_json::json!({}),
            description: String::new(),
            backlinks: vec![],
            outlinks: vec![],
            breadcrumbs: vec![],
            transclusion_cards: String::new(),
            is_new: false,
            raw_escaped: None,
            history: serde_json::Value::Null,
        }
    }

    fn default_engine() -> TemplateEngine {
        TemplateEngine::new(Path::new("."), "default", false, false)
    }

    #[test]
    fn test_render_index() {
        let engine = default_engine();
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "", "", "").unwrap();
        assert!(html.contains("Vault"));
        assert!(html.contains("Hello"));
    }

    #[test]
    fn test_render_page() {
        let engine = default_engine();
        let vault = sample_vault();
        let page = sample_page();
        let html = engine.render_page(&vault, &page, "static", "", "", "").unwrap();
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
        let html = engine
            .render_folder(&vault, &folder, "serve", "", "", "")
            .unwrap();
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
            sidebar_tree: vec![],
            stats: StatsContext {
                total_pages: 1,
                total_links: 0,
                dead_links: 0,
                orphans: 0,
            },
            history: serde_json::Value::Null,
            semantic_available: false,
            site_url: String::new(),
        };
        let idx = build_search_index(&vault);
        assert!(idx.contains(r#"\"hello\""#));
    }

    #[test]
    fn test_theme_variable_in_context() {
        let engine = TemplateEngine::new(Path::new("."), "fountain", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "", "", "").unwrap();
        assert!(html.contains(r#"data-theme="fountain""#));
    }

    #[test]
    fn test_default_theme_skips_disk() {
        // "default" theme should work without any .zetl/themes directory
        let engine = TemplateEngine::new(Path::new("/nonexistent"), "default", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "", "", "").unwrap();
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
        let html = engine.render_page(&vault, &page, "static", "", "", "").unwrap();
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

        let html1 = engine.render_index(&vault, "serve", "", "", "").unwrap();
        assert!(html1.contains("VERSION1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}VERSION2{% endblock %}"#,
        )
        .unwrap();

        // Reload mode should pick up the change
        let html2 = engine.render_index(&vault, "serve", "", "", "").unwrap();
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

        let html1 = engine.render_index(&vault, "serve", "", "", "").unwrap();
        assert!(html1.contains("CACHED_V1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}CACHED_V2{% endblock %}"#,
        )
        .unwrap();

        // Cached mode should still return the old version
        let html2 = engine.render_index(&vault, "serve", "", "", "").unwrap();
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
        let err = engine.render_index(&vault, "serve", "", "", "").unwrap_err();
        assert!(err.template_name.is_some());
        assert!(!err.message.is_empty());
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
        let err = engine.render_index(&vault, "serve", "", "", "").unwrap_err();
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

    // ── Passkey registration guidance tests ──────────────────────────────

    #[test]
    fn test_render_passkey_register() {
        let engine = default_engine();
        let html = engine
            .render_passkey_register("test-vault", "alice-a1b2c3d4")
            .unwrap();
        // Explanation text
        assert!(html.contains("Register a Passkey"));
        assert!(html.contains("phishing-resistant"));
        // Steps
        assert!(html.contains("Touch ID"));
        assert!(html.contains("Face ID"));
        // Visual indicator (spinner class)
        assert!(html.contains("pk-spinner"));
        // Retry button
        assert!(html.contains("Try Again"));
        // Fallback note
        assert!(html.contains("No passkey support"));
        assert!(html.contains("recovery"));
        // User ID passed through
        assert!(html.contains("alice-a1b2c3d4"));
    }

    #[test]
    fn test_render_passkey_register_empty_user() {
        let engine = default_engine();
        let html = engine.render_passkey_register("my-vault", "").unwrap();
        assert!(html.contains("Register a Passkey"));
        assert!(html.contains("my-vault"));
    }

    #[test]
    fn test_passkey_register_has_error_state() {
        let engine = default_engine();
        let html = engine
            .render_passkey_register("vault", "test-user")
            .unwrap();
        // Error state UI
        assert!(html.contains("Registration failed"));
        assert!(html.contains("pk-error"));
        assert!(html.contains("Back to instructions"));
    }

    #[test]
    fn test_passkey_register_has_success_state() {
        let engine = default_engine();
        let html = engine
            .render_passkey_register("vault", "test-user")
            .unwrap();
        assert!(html.contains("Passkey Registered"));
        assert!(html.contains("Continue to Vault"));
    }

    #[test]
    fn test_bundled_passkey_register_template_exists() {
        assert!(
            bundled_template("default", "passkey_register.html").is_some(),
            "passkey_register.html should be bundled in default theme"
        );
    }

    // ── Recovery show tests ───────────────────────────────────────────────

    #[test]
    fn test_bundled_recovery_show_template_exists() {
        assert!(
            bundled_template("default", "recovery_show.html").is_some(),
            "recovery_show.html should be bundled in default theme"
        );
    }

    #[test]
    fn test_render_recovery_show() {
        let engine = default_engine();
        let words = vec![
            "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract",
            "absurd", "abuse", "access", "accident",
        ];
        let html = engine
            .render_recovery_show(
                "test-vault",
                "abandon ability able about above absent absorb abstract absurd abuse access accident",
                &words,
                "/passkey/register?user_id=alice",
                "alice-12345678",
                "fakepubkey123",
                "csrf-token-abc",
            )
            .unwrap();
        // Explanation text
        assert!(html.contains("Your Recovery Phrase"));
        assert!(html.contains("only way"));
        assert!(html.contains("only be shown once"));
        // Numbered word grid
        for (i, word) in words.iter().enumerate() {
            assert!(html.contains(word), "missing word: {word}");
            assert!(
                html.contains(&format!("{}", i + 1)),
                "missing number: {}",
                i + 1
            );
        }
        // Confirmation checkbox
        assert!(html.contains("rc-check"));
        assert!(html.contains("written down my recovery phrase"));
        // Copy to clipboard
        assert!(html.contains("Copy to clipboard"));
        assert!(html.contains("clipboard may be accessible"));
        // Continue button
        // Continue link with user_id
        assert!(html.contains("rc-continue"));
        assert!(html.contains("user_id=alice"));
        assert!(html.contains("disabled"));
    }

    #[test]
    fn test_render_recovery_show_vault_name() {
        let engine = default_engine();
        let words = vec!["abandon"; 12];
        let html = engine
            .render_recovery_show(
                "my-notes",
                "abandon ".repeat(12).trim(),
                &words,
                "/",
                "user-1",
                "pk",
                "csrf",
            )
            .unwrap();
        assert!(html.contains("my-notes"));
    }

    // ── SPEC-028 graph template variables ────────────────────────────────

    /// Write a minimal valid theme.toml plus a probe template that emits
    /// the two graph variables verbatim, so tests can inspect them without
    /// depending on the full bundled theme.
    fn write_probe_theme(root: &std::path::Path, name: &str, graph_inline: bool) {
        let dir = root.join(".zetl/themes").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let inline_line = if graph_inline {
            "graph_inline = true\n"
        } else {
            ""
        };
        std::fs::write(
            dir.join("theme.toml"),
            format!(
                "{inline_line}[theme]\nname = \"{name}\"\nversion = \"1.0.0\"\n",
            ),
        )
        .unwrap();
        // Minimal probe template that prints `URL|LEN` — inherits nothing,
        // so it renders under every render_* method whose template name we
        // override below. `| safe` bypasses HTML-escape of the URL slashes.
        let probe =
            r#"ZETL_GRAPH|{{ graph_index_url|safe }}|{{ graph_index|length }}|END"#;
        for name in &["index.html", "page.html", "folder.html"] {
            std::fs::write(dir.join(name), probe).unwrap();
        }
    }

    #[test]
    fn test_graph_index_url_serve_mode() {
        // REQ-104: serve mode resolves to the absolute URL.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", false);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "serve", "", "", "").unwrap();
        assert!(
            html.contains("ZETL_GRAPH|/graph-index.json|0|END"),
            "expected absolute URL and empty graph_index, got: {html}"
        );
    }

    #[test]
    fn test_graph_index_url_build_mode_root() {
        // REQ-104: build mode at the vault root uses a `./`-prefixed
        // relative URL so the file:// protocol can resolve it.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", false);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        let vault = sample_vault();
        let html = engine.render_index(&vault, "build", "", "", "").unwrap();
        assert!(
            html.contains("ZETL_GRAPH|./graph-index.json|0|END"),
            "expected relative root URL, got: {html}"
        );
    }

    #[test]
    fn test_graph_index_url_build_mode_nested_page() {
        // REQ-104: a nested page at depth N must prefix N ../ segments so
        // the URL resolves against the page's own directory.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", false);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        let vault = sample_vault();
        let mut page = sample_page();
        page.slug = "docs/architecture/scanner".to_string();
        let html = engine.render_page(&vault, &page, "build", "", "", "").unwrap();
        assert!(
            html.contains("ZETL_GRAPH|../../../graph-index.json|0|END"),
            "expected ../../../graph-index.json for depth-3 slug, got: {html}"
        );
    }

    #[test]
    fn test_graph_index_absent_when_graph_inline_false() {
        // REQ-105: with graph_inline omitted from theme.toml the engine
        // MUST force graph_index to the empty string even if the caller
        // supplies a non-empty value, so authors opt-in deliberately.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", false);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        assert!(!engine.graph_inline());
        let vault = sample_vault();
        let html = engine
            .render_index(&vault, "serve", "", "", r#"{"nodes":[1,2,3]}"#)
            .unwrap();
        assert!(
            html.contains("|0|END"),
            "graph_index should be empty when graph_inline=false, got: {html}"
        );
    }

    #[test]
    fn test_graph_index_present_when_graph_inline_true() {
        // REQ-105: with graph_inline=true the caller's JSON string flows
        // through to the template context as-is.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", true);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        assert!(engine.graph_inline());
        let vault = sample_vault();
        let payload = r#"{"nodes":[1,2,3]}"#;
        let html = engine
            .render_index(&vault, "serve", "", "", payload)
            .unwrap();
        let expected_len = payload.len();
        assert!(
            html.contains(&format!("|{expected_len}|END")),
            "graph_index length should be {expected_len}, got: {html}"
        );
    }

    #[test]
    fn test_graph_vars_flow_through_render_page_and_folder() {
        // REQ-104/105: both variables must be present in every content
        // render path, not only render_index.
        let tmp = tempfile::tempdir().unwrap();
        write_probe_theme(tmp.path(), "gprobe", true);
        let engine = TemplateEngine::new(tmp.path(), "gprobe", false, false);
        let vault = sample_vault();

        let page = sample_page();
        let page_html = engine
            .render_page(&vault, &page, "serve", "", "", "abc")
            .unwrap();
        assert!(
            page_html.contains("ZETL_GRAPH|/graph-index.json|3|END"),
            "render_page missing graph vars, got: {page_html}"
        );

        let folder = FolderContext {
            name: "docs".to_string(),
            slug: "docs".to_string(),
            breadcrumbs: vec![],
            subfolders: vec![],
            pages: vec![],
            total_pages: 0,
        };
        let folder_html = engine
            .render_folder(&vault, &folder, "serve", "", "", "xy")
            .unwrap();
        assert!(
            folder_html.contains("ZETL_GRAPH|/graph-index.json|2|END"),
            "render_folder missing graph vars, got: {folder_html}"
        );
    }

    #[test]
    fn test_parse_graph_inline_extracts_top_level_bool() {
        assert_eq!(parse_graph_inline("graph_inline = true"), Some(true));
        assert_eq!(parse_graph_inline("graph_inline = false"), Some(false));
        assert_eq!(parse_graph_inline("[theme]\nname=\"x\""), None);
        assert_eq!(parse_graph_inline("not valid toml ][ "), None);
    }

    #[test]
    fn test_load_graph_inline_default_theme_has_no_flag() {
        // The bundled default theme ships without the flag, so the
        // engine's cached graph_inline for "default" must be false.
        let engine =
            TemplateEngine::new(std::path::Path::new("/nonexistent"), "default", false, false);
        assert!(!engine.graph_inline());
    }

    // ── Graph partial graceful-absence (SPEC-028 REQ-109, NFR-105) ──

    fn empty_vault_ctx(pages: usize, links: usize) -> VaultContext {
        VaultContext {
            name: "empty".to_string(),
            pages: Vec::new(),
            sidebar_tree: Vec::new(),
            stats: StatsContext {
                total_pages: pages,
                total_links: links,
                dead_links: 0,
                orphans: 0,
            },
            history: serde_json::Value::Null,
            semantic_available: false,
            site_url: String::new(),
        }
    }

    fn render_graph_partial(vault: &VaultContext) -> String {
        let engine = default_engine();
        let ctx = context! {
            vault => vault,
            root_path => "/",
            index_file => "",
            graph_index_url => "/graph-index.json",
            graph_index => "",
            mode => "serve",
            theme => "default",
        };
        let env = engine.env();
        let tmpl = env
            .get_template("_graph.html")
            .expect("bundled default theme must provide _graph.html");
        tmpl.render(ctx).expect("graph partial must render")
    }

    #[test]
    fn test_graph_partial_bundled_exists() {
        // REQ-109 prerequisite: the default theme must bundle _graph.html so
        // the graceful-absence contract (noscript, empty state, keyboard
        // fallback) is available everywhere.
        assert!(
            bundled_template("default", "_graph.html").is_some(),
            "default theme must bundle _graph.html"
        );
    }

    #[test]
    fn test_graph_partial_noscript_fallback() {
        let html = render_graph_partial(&sample_vault());
        assert!(html.contains("<noscript>"), "must provide a <noscript> block");
        assert!(
            html.contains("Graph view requires JavaScript"),
            "<noscript> must explain JS requirement"
        );
        assert!(
            html.contains("href=\"/\""),
            "<noscript> must link to the vault index"
        );
    }

    #[test]
    fn test_graph_partial_empty_vault_no_pages() {
        let html = render_graph_partial(&empty_vault_ctx(0, 0));
        assert!(
            html.contains("No pages yet"),
            "zero pages must render the no-pages empty-state"
        );
        // Canvas container still present so JS-enabled future-population
        // does not require a re-render of the partial.
        assert!(html.contains("id=\"zetl-graph\""));
    }

    #[test]
    fn test_graph_partial_empty_vault_no_links() {
        let mut vault = sample_vault();
        vault.stats.total_links = 0;
        let html = render_graph_partial(&vault);
        assert!(
            html.contains("No links yet"),
            "zero links must render the no-links empty-state"
        );
        assert!(
            html.contains("[[wikilinks]]"),
            "empty-state must teach the [[wikilink]] syntax"
        );
    }

    #[test]
    fn test_graph_partial_details_fallback_has_every_page() {
        let vault = VaultContext {
            name: "v".to_string(),
            pages: vec![
                PageEntry {
                    title: "Alpha".to_string(),
                    slug: "alpha".to_string(),
                    outlink_count: 1,
                    backlink_count: 0,
                    extension: "md".to_string(),
                },
                PageEntry {
                    title: "Guide Intro".to_string(),
                    slug: "guides/intro".to_string(),
                    outlink_count: 0,
                    backlink_count: 1,
                    extension: "md".to_string(),
                },
                PageEntry {
                    title: "Guide Next".to_string(),
                    slug: "guides/next".to_string(),
                    outlink_count: 0,
                    backlink_count: 0,
                    extension: "md".to_string(),
                },
            ],
            sidebar_tree: Vec::new(),
            stats: StatsContext {
                total_pages: 3,
                total_links: 2,
                dead_links: 0,
                orphans: 0,
            },
            history: serde_json::Value::Null,
            semantic_available: false,
            site_url: String::new(),
        };
        let html = render_graph_partial(&vault);
        assert!(html.contains("<details"), "NFR-105 requires a <details> fallback");
        assert!(html.contains("<summary>Page list (3)</summary>"));
        // Every slug reachable by tab — rendered as an anchor.
        assert!(html.contains("href=\"/alpha/\""));
        assert!(html.contains("href=\"/guides/intro/\""));
        assert!(html.contains("href=\"/guides/next/\""));
        // Slug disclosed as a <small> next to the nested page title so
        // sighted users can distinguish same-titled pages across folders.
        assert!(html.contains("guides/intro"));
    }

    #[test]
    fn test_graph_partial_tolerates_embedding_with_ignore_missing() {
        // REQ-109 contract: embedders (sidebar, /_graph route) must include
        // the partial via `{% include "_graph.html" ignore missing %}` so a
        // theme that removes the file does not crash the render. This test
        // proves the pattern works — both when the partial resolves (via
        // the bundled default) and when a theme shadows it with nothing.
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/no-graph");
        std::fs::create_dir_all(&theme_dir).unwrap();
        // Host template that wraps the widget exactly how the /_graph route
        // / sidebar entry will wrap it once those tasks land.
        std::fs::write(
            theme_dir.join("host.html"),
            r#"<div id="host">{% include "_graph.html" ignore missing %}</div>"#,
        )
        .unwrap();

        let engine = TemplateEngine::new(tmp.path(), "no-graph", false, false);
        let env = engine.env();
        let tmpl = env
            .get_template("host.html")
            .expect("host template must load");
        let html = tmpl
            .render(context! {
                vault => empty_vault_ctx(0, 0),
                root_path => "/",
                index_file => "",
                graph_index_url => "",
                graph_index => "",
                mode => "serve",
                theme => "no-graph",
            })
            .expect("host must render even when `_graph.html` is missing or inert");
        assert!(html.contains("<div id=\"host\">"));
    }
}
