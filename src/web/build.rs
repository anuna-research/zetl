use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::web::context::{build_folder_context, build_page_context, build_vault_context};
use crate::web::engine::{bundled_theme_files, TemplateEngine};
use crate::web::html::{html_escape, urlencoding};
use crate::web::markdown;
use crate::web::VaultData;

/// Strip YAML frontmatter (--- ... ---) from fountain file content.
pub fn strip_fountain_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    let after_first = &trimmed[3..];
    if let Some(end_pos) = after_first.find("\n---") {
        let skip = 3 + end_pos + 4;
        let rest = &trimmed[skip..];
        rest.strip_prefix('\n').unwrap_or(rest).to_string()
    } else {
        content.to_string()
    }
}

/// Tokenize body text and count term occurrences.
///
/// Mirrors Tantivy's default tokenizer: lowercase, split on non-alphanumeric characters.
fn tokenize_and_count(text: &str) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_lowercase();
        *counts.entry(lower).or_insert(0) += 1;
    }
    counts
}

/// Compute a relative root path for build mode based on slug depth.
///
/// For the root index (empty slug), returns `"./"`.
/// For a page at `hello`, returns `"../"`.
/// For a page at `architecture/scanner`, returns `"../../"`.
fn compute_root_path(slug: &str) -> String {
    if slug.is_empty() {
        "./".to_string()
    } else {
        let depth = slug.split('/').count();
        "../".repeat(depth)
    }
}

/// Extract sections from markdown content for search context.
///
/// Splits content by headings, returning each section's heading text,
/// body text (excluding code blocks, frontmatter, HTML comments), and
/// its 1-based line number in the original file.
fn extract_sections(content: &str) -> Vec<serde_json::Value> {
    let mut sections: Vec<serde_json::Value> = Vec::new();
    let mut current_heading = String::new();
    let mut current_text = String::new();
    let mut current_line: usize = 1;
    let mut in_frontmatter = false;
    let mut _frontmatter_started = false;
    let mut in_code_fence = false;

    for (i, line) in content.lines().enumerate() {
        let line_num = i + 1;
        let trimmed = line.trim();

        // Track frontmatter (--- delimited at start of file)
        if line_num == 1 && trimmed == "---" {
            in_frontmatter = true;
            _frontmatter_started = true;
            continue;
        }
        if in_frontmatter {
            if trimmed == "---" {
                in_frontmatter = false;
            }
            continue;
        }

        // Track fenced code blocks
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence {
            continue;
        }

        // Skip HTML comments (single-line)
        if trimmed.starts_with("<!--") && trimmed.ends_with("-->") {
            continue;
        }

        // Detect heading lines
        let heading_match = if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            if level <= 6 {
                let rest = trimmed[level..].trim();
                if !rest.is_empty() {
                    Some(rest.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(heading) = heading_match {
            // Close current section
            let text = current_text.trim().to_string();
            if !text.is_empty() || !current_heading.is_empty() {
                sections.push(serde_json::json!({
                    "h": current_heading,
                    "t": text,
                    "l": current_line,
                }));
            }
            current_heading = heading;
            current_text = String::new();
            current_line = line_num;
        } else if !trimmed.is_empty() {
            if !current_text.is_empty() {
                current_text.push(' ');
            }
            current_text.push_str(trimmed);
        }
    }

    // Close final section
    let text = current_text.trim().to_string();
    if !text.is_empty() || !current_heading.is_empty() {
        sections.push(serde_json::json!({
            "h": current_heading,
            "t": text,
            "l": current_line,
        }));
    }

    sections
}

/// Build the BM25 search index JSON and write it to `{out_dir}/search-index.json`.
///
/// Returns the JSON string so callers can also embed it inline in HTML
/// (needed for `file://` protocol where `fetch` is blocked by CORS).
///
/// Schema: avgDl, docs[{n,s,dl,tf,secs}], df.
/// Each doc includes `secs` — an array of sections with heading (h),
/// body text (t), and line number (l) — for rich search context.
fn write_search_index_json(data: &VaultData, vault_root: &Path, out_dir: &Path) -> Result<String> {
    let mut doc_entries: Vec<serde_json::Value> = Vec::with_capacity(data.files.len());
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut total_dl: usize = 0;

    for file in &data.files {
        let slug = data
            .page_slug_map
            .get(&file.page_name)
            .cloned()
            .unwrap_or_else(|| page_slug_from_path(&file.path));

        let full_path = vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path).unwrap_or_default();

        // Extract body text using the same exclusions as the Tantivy indexer
        // (no frontmatter, fenced code blocks, inline code, HTML comments).
        let body: String = body_text_ranges(&content)
            .iter()
            .map(|&(start, end)| &content[start..end])
            .collect::<Vec<_>>()
            .join(" ");

        let tf = tokenize_and_count(&body);
        let dl: usize = tf.values().sum();
        total_dl += dl;

        for term in tf.keys() {
            *df.entry(term.clone()).or_insert(0) += 1;
        }

        let tf_json: serde_json::Map<String, serde_json::Value> = tf
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
            .collect();

        let sections = extract_sections(&content);

        doc_entries.push(serde_json::json!({
            "n": file.page_name,
            "s": slug,
            "dl": dl,
            "tf": tf_json,
            "secs": sections,
        }));
    }

    let avg_dl: f64 = if data.files.is_empty() {
        0.0
    } else {
        total_dl as f64 / data.files.len() as f64
    };

    let df_json: serde_json::Map<String, serde_json::Value> = df
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
        .collect();

    let index = serde_json::json!({
        "avgDl": avg_dl,
        "docs": doc_entries,
        "df": df_json,
    });

    let json_str = serde_json::to_string(&index).context("serializing search-index.json")?;
    std::fs::write(out_dir.join("search-index.json"), &json_str)
        .context("writing search-index.json")?;

    Ok(json_str)
}

/// Write history-index.json to `{out_dir}/history-index.json` and return the
/// JSON string so it can be embedded as a template variable.
///
/// Returns an empty string when history is unavailable; the file is not
/// written in that case.
#[cfg(feature = "history")]
fn write_history_index_json(
    data: &VaultData,
    vault_root: &Path,
    out_dir: &Path,
    verbose: bool,
) -> String {
    let page_names: Vec<&str> = data.files.iter().map(|f| f.page_name.as_str()).collect();
    // OBS-013: time history-index export.
    let export_start = std::time::Instant::now();
    let Some(json_str) = crate::history::build_history_index_json(vault_root, &page_names) else {
        return String::new();
    };
    if let Err(e) = std::fs::write(out_dir.join("history-index.json"), &json_str) {
        eprintln!("warning: could not write history-index.json: {e}");
    }
    let export_ms = export_start.elapsed().as_millis();
    if verbose {
        let size_kb = json_str.len() / 1024;
        eprintln!(
            "[zetl] history-export: wrote history-index.json ({} KB, {} pages) duration_ms={}",
            size_kb,
            page_names.len(),
            export_ms
        );
    }
    json_str
}

/// Generate a complete static HTML site from the vault data.
pub fn build_static(
    data: &VaultData,
    vault_root: &Path,
    out_dir: &str,
    theme: &str,
    verbose: bool,
    public: Option<&str>,
) -> Result<()> {
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out)
        .with_context(|| format!("Cannot create output directory: {out_dir}"))?;

    let engine = TemplateEngine::new(vault_root, theme, false, verbose);
    let vault_name = vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    let mut vault_ctx = build_vault_context(data, &vault_name);
    #[cfg(feature = "history")]
    {
        // OBS-013: time vault history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) = crate::history::build_template_history_context(vault_root) {
            let hist_ms = hist_start.elapsed().as_millis();
            if verbose {
                eprintln!(
                    "[zetl] history-context: vault trend={} points recent={} changes duration_ms={}",
                    hist.trend.len(),
                    hist.recent_changes.len(),
                    hist_ms
                );
            }
            vault_ctx.history = serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
        }
    }

    // ── search-index.json (written as external file, not inlined into pages) ──
    write_search_index_json(data, vault_root, out)?;
    let bm25_json = String::new();

    // ── history-index.json ───────────────────────────────────────────────
    #[cfg(feature = "history")]
    let history_json = write_history_index_json(data, vault_root, out, verbose);
    #[cfg(not(feature = "history"))]
    let history_json = String::new();

    // ── index.html ──────────────────────────────────────────────────────
    let index_html = engine
        .render_index(&vault_ctx, "build", &bm25_json, &history_json)
        .map_err(|e| {
            eprintln!("{}", e.stderr_line("index"));
            anyhow::anyhow!("{e}")
        })?;
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

        let root_path = compute_root_path(&slug);
        let is_fountain = file.path.extension().map_or(false, |e| e == "fountain");
        let rendered = if is_fountain {
            // Pass raw fountain text to the template; the theme's JS parser handles it.
            let body = strip_fountain_frontmatter(&content);
            format!(
                "<script type=\"text/fountain\" id=\"fountain-source\">{}</script>\n<div id=\"fountain-render\"></div>",
                html_escape(&body)
            )
        } else {
            markdown::render_to_html(&content, &data.page_slug_map, &root_path, "index.html")
        };
        let mut page_ctx = build_page_context(data, &file.page_name, &slug, &rendered, &content);
        page_ctx.transclusion_cards =
            build_transclusion_cards(data, vault_root, &file.page_name, &root_path);
        #[cfg(feature = "history")]
        {
            // OBS-013: time per-page history context build.
            let hist_start = std::time::Instant::now();
            if let Some(hist) =
                crate::history::build_template_page_history_context(&file.page_name, vault_root)
            {
                let hist_ms = hist_start.elapsed().as_millis();
                if verbose {
                    eprintln!(
                        "[zetl] history-context: page {:?} trend={} points created={} duration_ms={}",
                        file.page_name,
                        hist.link_trend.len(),
                        hist.created_at,
                        hist_ms
                    );
                }
                page_ctx.history = serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
            }
        }
        #[cfg(feature = "history")]
        {
            let sources: Vec<String> = page_ctx.backlinks.iter().map(|b| b.title.clone()).collect();
            let since_map =
                crate::history::build_backlink_since_map(&file.page_name, &sources, vault_root);
            if !since_map.is_empty() {
                for bl in &mut page_ctx.backlinks {
                    bl.since = since_map.get(&bl.title.to_lowercase()).cloned();
                }
            }
        }

        let page_html = engine
            .render_page(&vault_ctx, &page_ctx, "build", &bm25_json, &history_json)
            .map_err(|e| {
                eprintln!("{}", e.stderr_line(&slug));
                anyhow::anyhow!("{e}")
            })?;
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
            .render_folder(&vault_ctx, &folder_ctx, "build", &bm25_json, &history_json)
            .map_err(|e| {
                eprintln!("{}", e.stderr_line(folder));
                anyhow::anyhow!("{e}")
            })?;
        std::fs::write(folder_dir.join("index.html"), folder_html)?;
        folder_count += 1;
    }

    // ── static assets ─────────────────────────────────────────────────
    let static_copied = copy_static_assets(vault_root, out, theme)?;

    // ── public overlay (copies over output root, overwriting generated pages) ──
    let public_copied = if let Some(pub_dir) = public {
        let pub_path = Path::new(pub_dir);
        if pub_path.is_dir() {
            copy_dir_recursive(pub_path, out)?;
            if verbose {
                eprintln!("[zetl] public overlay: copied {} → {}", pub_path.display(), out.display());
            }
            true
        } else {
            eprintln!("warning: --public directory does not exist: {pub_dir}");
            false
        }
    } else {
        false
    };

    let suffix = match (static_copied, public_copied) {
        (true, true) => " (static assets + public overlay copied)",
        (true, false) => " (static assets copied)",
        (false, true) => " (public overlay copied)",
        (false, false) => "",
    };
    eprintln!(
        "zetl build  →  {count} pages + {folder_count} folder indexes written to {out_dir}/{suffix}",
    );
    Ok(())
}

/// Copy static assets from `.zetl/static/`, `.zetl/themes/<theme>/static/`, and
/// the bundled theme's `static/` directory into `{out}/_static/`.
///
/// Priority (lowest to highest): bundled theme → vault shared → installed theme.
/// Returns `true` if any files were copied.
fn copy_static_assets(vault_root: &Path, out: &Path, theme: &str) -> Result<bool> {
    let shared_static = vault_root.join(".zetl/static");
    let theme_static = vault_root.join(format!(".zetl/themes/{theme}/static"));

    let shared_exists = shared_static.is_dir();
    let theme_exists = theme != "default" && theme_static.is_dir();

    // Collect bundled theme static files (paths beginning with "static/").
    let bundled_statics: Vec<_> = bundled_theme_files(theme)
        .into_iter()
        .filter(|(p, _)| p.starts_with("static"))
        .collect();
    let bundled_exists = !bundled_statics.is_empty();

    if !shared_exists && !theme_exists && !bundled_exists {
        return Ok(false);
    }

    let dest = out.join("_static");
    std::fs::create_dir_all(&dest)
        .with_context(|| format!("Cannot create _static directory: {}", dest.display()))?;

    // (1) Bundled theme static files first (lowest priority)
    if bundled_exists {
        for (rel_path, bytes) in &bundled_statics {
            // Strip the leading "static/" component.
            let file_rel = rel_path
                .strip_prefix("static")
                .unwrap_or(rel_path.as_path());
            let target = dest.join(file_rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Cannot create directory: {}", parent.display()))?;
            }
            std::fs::write(&target, bytes)
                .with_context(|| format!("Cannot write {}", target.display()))?;
        }
    }

    // (2) Shared vault static (overwrites bundled on conflict)
    if shared_exists {
        copy_dir_recursive(&shared_static, &dest)?;
    }
    // (3) Installed theme-specific static (overwrites everything on conflict)
    if theme_exists {
        copy_dir_recursive(&theme_static, &dest)?;
    }

    Ok(true)
}

/// Recursively copy the contents of `src` into `dest`, preserving directory structure.
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("Cannot read directory: {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dest.join(entry.file_name());

        if file_type.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir_recursive(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
        // Skip symlinks and other special file types
    }
    Ok(())
}

/// Build transclusion card HTML for a page's forward links.
fn build_transclusion_cards(
    data: &VaultData,
    vault_root: &Path,
    page_name: &str,
    root_path: &str,
) -> String {
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
            .map(|content| {
                markdown::render_preview_html(
                    &content,
                    &data.page_slug_map,
                    root_path,
                    "index.html",
                )
            })
            .unwrap_or_else(|| format!("<p><em>{}</em></p>", html_escape("(page does not exist)")));

        cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="{root_path}{href}/index.html" style="border-left-color: {color};">
  <a href="{root_path}{href}/index.html" class="tc-title" style="color: {color};">{name}</a>
  <div class="tc-excerpt prose prose-sm max-w-none">{preview}</div>
</div>"#,
            root_path = root_path,
            href = href,
            color = color,
            name = html_escape(target),
            preview = preview_html,
        ));
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_static_dirs_skips_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let result = copy_static_assets(tmp.path(), &out, "default").unwrap();
        assert!(!result);
        assert!(!out.join("_static").exists());
    }

    #[test]
    fn shared_static_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("app.css"), "body{}").unwrap();

        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let result = copy_static_assets(tmp.path(), &out, "default").unwrap();
        assert!(result);
        assert_eq!(
            std::fs::read_to_string(out.join("_static/app.css")).unwrap(),
            "body{}"
        );
    }

    #[test]
    fn theme_static_copies() {
        let tmp = tempfile::tempdir().unwrap();
        let theme_dir = tmp.path().join(".zetl/themes/ocean/static");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("theme.js"), "alert(1)").unwrap();

        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let result = copy_static_assets(tmp.path(), &out, "ocean").unwrap();
        assert!(result);
        assert_eq!(
            std::fs::read_to_string(out.join("_static/theme.js")).unwrap(),
            "alert(1)"
        );
    }

    #[test]
    fn theme_overwrites_shared_on_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("style.css"), "shared").unwrap();

        let theme_dir = tmp.path().join(".zetl/themes/custom/static");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("style.css"), "theme").unwrap();

        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        copy_static_assets(tmp.path(), &out, "custom").unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("_static/style.css")).unwrap(),
            "theme"
        );
    }

    #[test]
    fn preserves_nested_directory_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join(".zetl/static/fonts/woff2");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("inter.woff2"), "fontdata").unwrap();

        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        copy_static_assets(tmp.path(), &out, "default").unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("_static/fonts/woff2/inter.woff2")).unwrap(),
            "fontdata"
        );
    }

    #[test]
    fn both_sources_merge_non_conflicting() {
        let tmp = tempfile::tempdir().unwrap();
        let shared = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("shared.js"), "shared").unwrap();

        let theme_dir = tmp.path().join(".zetl/themes/duo/static");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("theme.js"), "theme").unwrap();

        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        copy_static_assets(tmp.path(), &out, "duo").unwrap();
        assert_eq!(
            std::fs::read_to_string(out.join("_static/shared.js")).unwrap(),
            "shared"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("_static/theme.js")).unwrap(),
            "theme"
        );
    }
}
