use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::scanner::page_slug_from_path;
use crate::web::context::{build_folder_context, build_page_context, build_vault_context};
use crate::web::engine::TemplateEngine;
use crate::web::html::{html_escape, urlencoding};
use crate::web::markdown;
use crate::web::VaultData;

/// Generate a complete static HTML site from the vault data.
pub fn build_static(data: &VaultData, vault_root: &Path, out_dir: &str, theme: &str) -> Result<()> {
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out)
        .with_context(|| format!("Cannot create output directory: {out_dir}"))?;

    let engine = TemplateEngine::new(vault_root, theme, false);
    let vault_name = vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    let vault_ctx = build_vault_context(data, &vault_name);

    // ── index.html ──────────────────────────────────────────────────────
    let index_html = engine
        .render_index(&vault_ctx)
        .context("failed to render index page for static build")?;
    std::fs::write(out.join("index.html"), index_html)?;

    // ── per-page HTML ───────────────────────────────────────────────────
    let mut count = 0usize;
    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        let page_dir = out.join(&slug);
        std::fs::create_dir_all(&page_dir)?;

        let full_path = vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path)
            .with_context(|| format!("Cannot read {}", full_path.display()))?;

        let rendered = markdown::render_to_html(&content, &data.page_slug_map);
        let mut page_ctx =
            build_page_context(data, &file.page_name, &slug, &rendered, &content);
        page_ctx.transclusion_cards =
            build_transclusion_cards(data, vault_root, &file.page_name);

        let page_html = engine
            .render_page(&vault_ctx, &page_ctx, "build")
            .with_context(|| format!("failed to render page '{}'", file.page_name))?;
        std::fs::write(page_dir.join("index.html"), page_html)?;
        count += 1;
    }

    // ── folder index pages ─────────────────────────────────────────────
    let mut folders: HashSet<String> = HashSet::new();
    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        let mut pos = 0;
        while let Some(sep) = slug[pos..].find('/') {
            let folder = &slug[..pos + sep];
            folders.insert(folder.to_string());
            pos += sep + 1;
        }
    }

    let mut folder_count = 0usize;
    for folder in &folders {
        let folder_prefix = format!("{}/", folder.to_lowercase());
        let has_pages = data.files.iter().any(|f| {
            let s = page_slug_from_path(&f.path);
            s.to_lowercase().starts_with(&folder_prefix)
        });

        if !has_pages {
            continue;
        }

        let folder_dir = out.join(folder);
        std::fs::create_dir_all(&folder_dir)?;

        let folder_name = folder.rsplit('/').next().unwrap_or(folder);
        let folder_ctx = build_folder_context(data, folder, folder_name);
        let folder_html = engine
            .render_folder(&vault_ctx, &folder_ctx)
            .with_context(|| format!("failed to render folder '{}'", folder))?;
        std::fs::write(folder_dir.join("index.html"), folder_html)?;
        folder_count += 1;
    }

    eprintln!("zetl build  →  {count} pages + {folder_count} folder indexes written to {out_dir}/");
    Ok(())
}

/// Build transclusion card HTML for a page's forward links.
fn build_transclusion_cards(data: &VaultData, vault_root: &Path, page_name: &str) -> String {
    let forward_links = data.graph.forward_links(page_name);
    let mut seen_targets = HashSet::new();
    let mut unique_targets: Vec<String> = Vec::new();
    for link in &forward_links {
        let key = link.target.to_lowercase();
        if seen_targets.insert(key) {
            unique_targets.push(link.target.clone());
        }
    }

    let colors = [
        "#f472b6", "#60a5fa", "#34d399", "#fbbf24", "#a78bfa", "#fb923c", "#2dd4bf", "#f87171",
    ];

    let mut cards = String::new();
    for (i, target) in unique_targets.iter().enumerate() {
        let color = colors[i % colors.len()];
        let target_slug = data.slug_for_page(target);
        let href = urlencoding(&target_slug);

        let preview_html = data
            .files
            .iter()
            .find(|f| f.page_name.eq_ignore_ascii_case(target))
            .and_then(|file| {
                let full_path = vault_root.join(&file.path);
                std::fs::read_to_string(&full_path).ok()
            })
            .map(|content| markdown::render_preview_html(&content, &data.page_slug_map))
            .unwrap_or_else(|| {
                format!(
                    "<p><em>{}</em></p>",
                    html_escape("(page does not exist)")
                )
            });

        cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="/{href}" style="border-left-color: {color};">
  <a href="/{href}" class="tc-title" style="color: {color};">{name}</a>
  <div class="tc-excerpt prose prose-sm max-w-none">{preview}</div>
</div>"#,
            href = href,
            color = color,
            name = html_escape(target),
            preview = preview_html,
        ));
    }
    cards
}
