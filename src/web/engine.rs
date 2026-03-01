use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use minijinja::{context, Environment};

use super::context::{FolderContext, PageContext, VaultContext};

/// Template engine wrapping a minijinja::Environment with two-tier template resolution.
///
/// Templates resolve in order:
/// 1. `.zetl/themes/<theme>/<name>` on disk (skipped when theme is "default")
/// 2. Built-in default templates embedded via `include_str!()`
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

/// Build a minijinja Environment with the two-tier template loader.
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
        // Tier 2: fall back to built-in defaults
        Ok(match name {
            "base.html" => Some(include_str!("templates/base.html").to_string()),
            "index.html" => Some(include_str!("templates/index.html").to_string()),
            "page.html" => Some(include_str!("templates/page.html").to_string()),
            "folder.html" => Some(include_str!("templates/folder.html").to_string()),
            _ => None,
        })
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
    pub fn new(vault_root: &Path, theme: &str, reload: bool) -> Self {
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
    pub fn render_index(&self, vault_ctx: &VaultContext) -> Result<String> {
        let search_index = build_search_index(vault_ctx);
        let ctx = context! {
            vault => vault_ctx,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
        };
        let env = self.env();
        let tmpl = env.get_template("index.html")
            .context("failed to load index.html template")?;
        tmpl.render(ctx)
            .context("failed to render index.html template")
    }

    /// Render a single page.
    pub fn render_page(
        &self,
        vault_ctx: &VaultContext,
        page_ctx: &PageContext,
        mode: &str,
    ) -> Result<String> {
        let search_index = build_search_index(vault_ctx);
        let ctx = context! {
            vault => vault_ctx,
            page => page_ctx,
            mode => mode,
            search_index => search_index,
            theme => &self.theme,
            active_slug => &page_ctx.slug,
        };
        let env = self.env();
        let tmpl = env.get_template("page.html")
            .context("failed to load page.html template")?;
        tmpl.render(ctx)
            .context(format!("failed to render page.html for '{}'", page_ctx.title))
    }

    /// Render a folder index page.
    pub fn render_folder(
        &self,
        vault_ctx: &VaultContext,
        folder_ctx: &FolderContext,
    ) -> Result<String> {
        let search_index = build_search_index(vault_ctx);
        let ctx = context! {
            vault => vault_ctx,
            folder => folder_ctx,
            search_index => search_index,
            theme => &self.theme,
            active_slug => "",
        };
        let env = self.env();
        let tmpl = env.get_template("folder.html")
            .context("failed to load folder.html template")?;
        tmpl.render(ctx)
            .context(format!("failed to render folder.html for '{}'", folder_ctx.name))
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
            }],
            stats: StatsContext {
                total_pages: 1,
                total_links: 1,
                dead_links: 0,
                orphans: 0,
            },
        }
    }

    fn default_engine() -> TemplateEngine {
        TemplateEngine::new(Path::new("."), "default", false)
    }

    #[test]
    fn test_render_index() {
        let engine = default_engine();
        let vault = sample_vault();
        let html = engine.render_index(&vault).unwrap();
        assert!(html.contains("Vault"));
        assert!(html.contains("Hello"));
    }

    #[test]
    fn test_render_page() {
        let engine = default_engine();
        let vault = sample_vault();
        let page = PageContext {
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
        };
        let html = engine.render_page(&vault, &page, "static").unwrap();
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
        let html = engine.render_folder(&vault, &folder).unwrap();
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
        let engine = TemplateEngine::new(Path::new("."), "fountain", false);
        let vault = sample_vault();
        let html = engine.render_index(&vault).unwrap();
        assert!(html.contains(r#"data-theme="fountain""#));
    }

    #[test]
    fn test_default_theme_skips_disk() {
        // "default" theme should work without any .zetl/themes directory
        let engine = TemplateEngine::new(Path::new("/nonexistent"), "default", false);
        let vault = sample_vault();
        let html = engine.render_index(&vault).unwrap();
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

        let engine = TemplateEngine::new(tmp.path(), "custom", false);
        let vault = sample_vault();
        let page = PageContext {
            title: "Test".to_string(),
            slug: "test".to_string(),
            content_html: "<p>hi</p>".to_string(),
            content_raw: "hi".to_string(),
            frontmatter: serde_json::json!({}),
            backlinks: vec![],
            outlinks: vec![],
            breadcrumbs: vec![],
            transclusion_cards: String::new(),
            is_new: false,
            raw_escaped: None,
        };
        let html = engine.render_page(&vault, &page, "static").unwrap();
        // Custom template wraps content in <div class="custom">
        assert!(html.contains(r#"<div class="custom">"#));
        // base.html is still the built-in (cross-tier inheritance)
        assert!(html.contains("CUSTOM: Test"));
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

        let engine = TemplateEngine::new(tmp.path(), "live", true);
        let vault = sample_vault();

        let html1 = engine.render_index(&vault).unwrap();
        assert!(html1.contains("VERSION1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}VERSION2{% endblock %}"#,
        )
        .unwrap();

        // Reload mode should pick up the change
        let html2 = engine.render_index(&vault).unwrap();
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

        let engine = TemplateEngine::new(tmp.path(), "cached", false);
        let vault = sample_vault();

        let html1 = engine.render_index(&vault).unwrap();
        assert!(html1.contains("CACHED_V1"));

        // Update template on disk
        std::fs::write(
            theme_dir.join("index.html"),
            r#"{% extends "base.html" %}{% block title %}V2{% endblock %}{% block content %}CACHED_V2{% endblock %}"#,
        )
        .unwrap();

        // Cached mode should still return the old version
        let html2 = engine.render_index(&vault).unwrap();
        assert!(html2.contains("CACHED_V1"));
    }
}
