use anyhow::{Context, Result};
use minijinja::{context, Environment};

use super::context::{FolderContext, PageContext, VaultContext};

/// Template engine wrapping a minijinja::Environment with embedded built-in templates.
pub struct TemplateEngine {
    env: Environment<'static>,
}

impl TemplateEngine {
    /// Create a new TemplateEngine with the four built-in templates embedded via include_str!().
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.add_template("base.html", include_str!("templates/base.html"))
            .expect("built-in base.html template should parse");
        env.add_template("index.html", include_str!("templates/index.html"))
            .expect("built-in index.html template should parse");
        env.add_template("page.html", include_str!("templates/page.html"))
            .expect("built-in page.html template should parse");
        env.add_template("folder.html", include_str!("templates/folder.html"))
            .expect("built-in folder.html template should parse");
        Self { env }
    }

    /// Render the vault index page.
    pub fn render_index(&self, vault_ctx: &VaultContext) -> Result<String> {
        let search_index = build_search_index(vault_ctx);
        let ctx = context! {
            vault => vault_ctx,
            search_index => search_index,
            theme => "emerald",
            active_slug => "",
        };
        let tmpl = self.env.get_template("index.html")
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
            theme => "emerald",
            active_slug => &page_ctx.slug,
        };
        let tmpl = self.env.get_template("page.html")
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
            theme => "emerald",
            active_slug => "",
        };
        let tmpl = self.env.get_template("folder.html")
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

    #[test]
    fn test_render_index() {
        let engine = TemplateEngine::new();
        let vault = sample_vault();
        let html = engine.render_index(&vault).unwrap();
        assert!(html.contains("Vault"));
        assert!(html.contains("Hello"));
    }

    #[test]
    fn test_render_page() {
        let engine = TemplateEngine::new();
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
        let engine = TemplateEngine::new();
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
}
