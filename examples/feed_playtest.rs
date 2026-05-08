//! Standalone playtest harness for SPEC-038 feeds.
//!
//! Reads `*.md` files from the supplied vault directory, synthesises a
//! `FeedItem` for each, runs `feed::build::emit_root_feed` against an
//! in-memory `[feed]` config, and writes the per-format outputs plus a
//! minimal `index.html` carrying the rel=alternate discovery tags to
//! the supplied output directory.
//!
//! Run via:
//!
//!     cargo run --example feed_playtest -- demo-vault /tmp/zetl-feed-demo
//!
//! Then serve the output dir via any static HTTP server (e.g.
//! `python3 -m http.server -d /tmp/zetl-feed-demo 8088`) and drive a
//! browser at http://localhost:8088/.

use std::fs;
use std::path::{Path, PathBuf};

use zetl::feed::build::emit_root_feed;
use zetl::feed::config::parse_config;
use zetl::feed::select::PageView;
use zetl::feed::types::{FeedItem, License, SelectionRule, SourceMetadata};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let vault = args
        .next()
        .ok_or("usage: feed_playtest <vault-dir> <out-dir>")?;
    let out = args
        .next()
        .ok_or("usage: feed_playtest <vault-dir> <out-dir>")?;
    let vault = PathBuf::from(vault);
    let out = PathBuf::from(out);
    fs::create_dir_all(&out)?;

    let pages = collect_pages(&vault)?;
    println!("[playtest] {} pages collected from {}", pages.len(), vault.display());

    // Synthesise feed config in-memory. enable_json on so the JSON
    // Feed output is also produced.
    let lens = parse_config(
        r#"
        [feed]
        base_url = "http://localhost:8088"
        title = "zetl demo-vault feed"
        description = "SPEC-038 playtest feed emitted from demo-vault."
        max_items = 50
        enable_json = true
        author = "Anuna"
        language = "en-AU"
        copyright = "(c) Anuna 2026 — licensed CC-BY-4.0 https://creativecommons.org/licenses/by/4.0/"
        "#,
    )?;

    let visibility: Box<dyn Fn(&PageView<'_>) -> bool> = Box::new(|_p| true);
    let rule = SelectionRule::FrontmatterOptIn;

    // Inject `frontmatter_feed_optin = true` on every page so the
    // demo set populates without per-page feed: true frontmatter.
    let pages_optin: Vec<PageView<'_>> = pages
        .iter()
        .map(|p| PageView {
            slug: p.slug.as_str(),
            path: p.path.as_path(),
            frontmatter_feed_optin: true,
            tags: &p.tags,
            matches_spl_query: false,
            item: p.item.clone(),
        })
        .collect();

    let emission = emit_root_feed(&lens, &pages_optin, &visibility, &rule)?;
    println!(
        "[playtest] emitted {} feed file(s); items_selected={} items_emitted={} duration={}us",
        emission.files.len(),
        emission.stats.items_selected,
        emission.stats.items_emitted,
        emission.stats.duration.as_micros()
    );

    for (rel_path, body) in &emission.files {
        let target = out.join(rel_path.trim_start_matches('/'));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, body)?;
        println!("[playtest]   wrote {} ({} bytes)", target.display(), body.len());
    }

    // index.html with rel=alternate tags + a small page list so ar-crawl
    // / a human visiting http://localhost:8088/ sees something useful.
    let mut html = String::with_capacity(2048);
    html.push_str("<!doctype html>\n<html lang=\"en-AU\">\n<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str("  <title>zetl demo-vault feed playtest</title>\n");
    for tag in &emission.discovery_tags {
        html.push_str("  ");
        html.push_str(tag);
        html.push('\n');
    }
    html.push_str("</head>\n<body>\n");
    html.push_str("  <h1>zetl demo-vault — SPEC-038 feed playtest</h1>\n");
    html.push_str("  <p>This page advertises three rel=alternate feeds. Open the page source to see the discovery tags, or hit the URLs directly:</p>\n");
    html.push_str("  <ul>\n");
    html.push_str("    <li><a href=\"/feed.xml\">/feed.xml</a> (RSS 2.0)</li>\n");
    html.push_str("    <li><a href=\"/atom.xml\">/atom.xml</a> (Atom 1.0)</li>\n");
    html.push_str("    <li><a href=\"/feed.json\">/feed.json</a> (JSON Feed v1.1)</li>\n");
    html.push_str("  </ul>\n");
    html.push_str("  <h2>Pages emitted to the feed</h2>\n");
    html.push_str("  <ol>\n");
    for p in &pages {
        html.push_str(&format!(
            "    <li>{} <small>({})</small></li>\n",
            html_escape(&p.item.title),
            html_escape(&p.item.date_published)
        ));
    }
    html.push_str("  </ol>\n</body>\n</html>\n");
    fs::write(out.join("index.html"), html.as_bytes())?;
    println!("[playtest]   wrote {}", out.join("index.html").display());

    println!(
        "\n[playtest] ready. start a static server with:\n  python3 -m http.server -d {} 8088\nthen open http://localhost:8088/",
        out.display()
    );
    Ok(())
}

struct Page {
    slug: String,
    path: PathBuf,
    tags: Vec<String>,
    item: FeedItem,
}

fn collect_pages(vault: &Path) -> Result<Vec<Page>, Box<dyn std::error::Error>> {
    let mut pages = Vec::new();
    walk(vault, vault, &mut pages)?;
    pages.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(pages)
}

fn walk(
    root: &Path,
    here: &Path,
    out: &mut Vec<Page>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(here)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Skip hidden + build artifacts.
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap_or_default();
        let title = derive_title(&path, &content);
        let summary = derive_summary(&content);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let slug = rel
            .trim_end_matches(".md")
            .replace(std::path::MAIN_SEPARATOR, "/");
        // Use a deterministic per-page date — month rotation across the
        // page set so ar-crawl sees a varied feed.
        let day = (out.len() % 28) as u32 + 1;
        let date_published = format!("2026-04-{:02}T00:00:00Z", day);
        let id = format!("tag:localhost,2026:zetl/{slug}");
        let url = format!(
            "http://localhost:8088/{}",
            slug.split('/').map(url_segment_encode).collect::<Vec<_>>().join("/")
        );
        let item = FeedItem {
            id,
            title: title.clone(),
            url,
            date_published,
            date_modified: None,
            summary: Some(summary),
            content_html: Some(format!(
                "<p>Page slug: <code>{}</code>.</p><p>This is a synthetic feed item emitted by the SPEC-038 playtest harness. Source: <code>{}</code>.</p>",
                html_escape(&slug),
                html_escape(&rel)
            )),
            author: None,
            tags: vec!["demo-vault".to_string(), "spec-038".to_string()],
            license: Some(License::CcBy4_0),
            source_metadata: SourceMetadata {
                source_path: Some(PathBuf::from(rel.clone())),
                object_id: Some(slug.clone()),
                ..Default::default()
            },
        };
        out.push(Page {
            slug,
            path: path.clone(),
            tags: vec!["demo-vault".to_string()],
            item,
        });
    }
    Ok(())
}

fn derive_title(path: &Path, content: &str) -> String {
    // Frontmatter title: simple grep, not a TOML parser.
    if let Some(stripped) = content.strip_prefix("---") {
        if let Some(end) = stripped.find("\n---") {
            for line in stripped[..end].lines() {
                if let Some(t) = line.strip_prefix("title:") {
                    return t.trim().trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }
    // Fallback: file stem.
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".to_string())
}

fn derive_summary(content: &str) -> String {
    // First non-empty non-frontmatter line, truncated.
    let mut after_fm = content;
    if let Some(stripped) = content.strip_prefix("---") {
        if let Some(end) = stripped.find("\n---") {
            after_fm = &stripped[end + 4..];
        }
    }
    for line in after_fm.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let truncated: String = line.chars().take(180).collect();
        return truncated;
    }
    "No summary available.".to_string()
}

fn url_segment_encode(s: &str) -> String {
    // Conservative percent-encoding for slug segments that may carry
    // spaces / non-ASCII. Good enough for the playtest harness — the
    // production code path uses the scanner's already-clean slugs.
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            other => out.push_str(&format!("%{:02X}", other)),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
