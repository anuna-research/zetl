use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use petgraph::visit::EdgeRef;

use crate::graph::{GraphIndexContext, GraphIndexEdge, GraphIndexNode};
use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::web::context::{build_folder_context, build_page_context, build_vault_context};
use crate::web::engine::{
    build_llms_txt, build_search_index, build_sitemap, bundled_theme_files, TemplateEngine,
};
use crate::web::html::{html_escape, urlencoding};
use crate::web::markdown;
use crate::web::VaultData;

/// Build SPEC-038 `<link rel="alternate">` tags for the root + scoped
/// feeds advertised by the `[feed]` section. Mirrors
/// [`crate::feed::build::discovery_tags`] but consumes the parsed
/// config-lens directly so we don't have to build a full
/// [`crate::feed::types::FeedConfig`] just to emit the tags.
fn build_feed_discovery_tags(
    feed: &crate::feed::config::FeedSection,
    base_url: &str,
    title: &str,
) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let paths = feed.paths.as_ref();
    let rss_path = paths
        .and_then(|p| p.rss.clone())
        .unwrap_or_else(|| crate::feed::build::DEFAULT_RSS_PATH.to_string());
    let atom_path = paths
        .and_then(|p| p.atom.clone())
        .unwrap_or_else(|| crate::feed::build::DEFAULT_ATOM_PATH.to_string());
    let json_path = paths
        .and_then(|p| p.jsonfeed.clone())
        .unwrap_or_else(|| crate::feed::build::DEFAULT_JSONFEED_PATH.to_string());
    let title_attr = title.replace('&', "&amp;").replace('"', "&quot;");
    let mut out = vec![
        format!(
            r#"<link rel="alternate" type="application/rss+xml" title="{title_attr} (RSS)" href="{base}{rss_path}" />"#
        ),
        format!(
            r#"<link rel="alternate" type="application/atom+xml" title="{title_attr} (Atom)" href="{base}{atom_path}" />"#
        ),
    ];
    if feed.enable_json.unwrap_or(false) {
        out.push(format!(
            r#"<link rel="alternate" type="application/feed+json" title="{title_attr} (JSON Feed)" href="{base}{json_path}" />"#
        ));
    }
    // Per-scope feeds get their own discovery tags so feed readers
    // surface them alongside the root.
    for scope in &feed.scopes {
        out.push(format!(
            r#"<link rel="alternate" type="application/atom+xml" title="{title_attr} ({scope_title})" href="{base}{scope_path}" />"#,
            scope_title = scope.title.replace('&', "&amp;").replace('"', "&quot;"),
            scope_path = scope.path,
        ));
    }
    out
}

/// Owned per-page data for the SPEC-038 feed pipeline. Lives at the
/// caller; [`page_views_from_owned`] borrows from this Vec to satisfy
/// `feed::select::PageView<'a>` lifetimes.
pub(crate) struct OwnedFeedPage {
    pub(crate) slug: String,
    pub(crate) path: PathBuf,
    pub(crate) tags: Vec<String>,
    pub(crate) item: crate::feed::types::FeedItem,
    pub(crate) frontmatter_feed_optin: bool,
}

/// Build the page set the feed pipeline operates on. Reads each page
/// from disk; for pages that did not opt in via frontmatter `feed:
/// true`, only synthesises a stub `FeedItem` unless `populate_all` is
/// set (scoped/cohort feed families need full items for non-opt-in
/// pages so their `select` rules can match).
///
/// Returns `None` when `[feed]` is absent / disabled / missing
/// `base_url`.
pub(crate) fn build_owned_pages(
    lens: &crate::feed::config::FeedConfigLens,
    data: &VaultData,
    vault_root: &Path,
) -> Option<Vec<OwnedFeedPage>> {
    use crate::feed::types::{FeedItem, SourceMetadata};
    use crate::web::markdown::parse_frontmatter;

    let feed = lens.feed.as_ref()?;
    if matches!(feed.enabled, Some(false)) {
        return None;
    }
    let base_url = feed.base_url.as_deref()?;
    let populate_all = needs_full_population(lens);
    let base_url_trimmed = base_url.trim_end_matches('/');
    let id_namespace = url::Url::parse(base_url_trimmed)
        .or_else(|_| url::Url::parse(&format!("{base_url_trimmed}/")))
        .unwrap_or_else(|_| url::Url::parse("https://example.invalid/").unwrap());
    let render_root = format!("{base_url_trimmed}/");

    let mut owned: Vec<OwnedFeedPage> = Vec::with_capacity(data.files.len());
    for parsed in &data.files {
        let abs_path = vault_root.join(&parsed.path);
        let slug = data
            .page_slug_map
            .get(&parsed.page_name)
            .cloned()
            .unwrap_or_else(|| parsed.page_name.clone());
        let body = std::fs::read_to_string(&abs_path).unwrap_or_default();
        let opted_in = matches!(scan_frontmatter_feed_optin(&body), Some(true));
        if !opted_in && !populate_all {
            owned.push(OwnedFeedPage {
                slug: slug.clone(),
                path: parsed.path.clone(),
                tags: Vec::new(),
                item: FeedItem {
                    id: format!("urn:zetl:{slug}"),
                    title: String::new(),
                    url: String::new(),
                    date_published: String::new(),
                    date_modified: None,
                    summary: None,
                    content_html: None,
                    author: None,
                    tags: Vec::new(),
                    license: None,
                    source_metadata: SourceMetadata::default(),
                },
                frontmatter_feed_optin: false,
            });
            continue;
        }
        let frontmatter = parse_frontmatter(&body);
        let title = frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| parsed.page_name.clone());
        let date_published = frontmatter_date(&frontmatter)
            .or_else(|| mtime_to_rfc3339(parsed.mtime))
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
        let mut tags: Vec<String> = frontmatter
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        tags.sort();
        tags.dedup();
        let url = format!(
            "{base_url_trimmed}/{}",
            slug.split('/')
                .map(percent_encode_segment)
                .collect::<Vec<_>>()
                .join("/")
        );
        let id = crate::feed::item_id::item_id(&slug, &id_namespace, 2026)
            .unwrap_or_else(|_| format!("urn:zetl:{slug}"));
        let summary = crate::web::markdown::extract_description(&body, &frontmatter);
        let summary = if summary.is_empty() {
            None
        } else {
            Some(summary.chars().take(400).collect::<String>())
        };
        let content_html = {
            let mut rendered = crate::web::markdown::render_to_html(
                &body,
                &data.page_slug_map,
                &render_root,
                "index.html",
            );
            const MAX_BYTES: usize = 65_536;
            if rendered.is_empty() {
                None
            } else if rendered.len() > MAX_BYTES {
                let mut cut = MAX_BYTES;
                while cut > 0 && !rendered.is_char_boundary(cut) {
                    cut -= 1;
                }
                rendered.truncate(cut);
                rendered.push_str(
                    "\n<!-- content truncated for feed; visit the page URL for the full body -->\n",
                );
                Some(rendered)
            } else {
                Some(rendered)
            }
        };
        let item = FeedItem {
            id,
            title,
            url,
            date_published,
            date_modified: None,
            summary,
            content_html,
            author: None,
            tags: tags.clone(),
            license: None,
            source_metadata: SourceMetadata {
                source_path: Some(parsed.path.clone()),
                object_id: Some(slug.clone()),
                content_hash: parsed
                    .file_merkle
                    .as_ref()
                    .map(|m| crate::feed::xml::hex_encode(&m.root_hash)),
                ..Default::default()
            },
        };
        owned.push(OwnedFeedPage {
            slug,
            path: parsed.path.clone(),
            tags,
            item,
            frontmatter_feed_optin: opted_in,
        });
    }
    Some(owned)
}

/// Borrow [`OwnedFeedPage`]s as [`feed::select::PageView`]s. Caller
/// owns the `Vec<OwnedFeedPage>`; returned references live as long as
/// the input slice.
pub(crate) fn page_views_from_owned(
    owned: &[OwnedFeedPage],
) -> Vec<crate::feed::select::PageView<'_>> {
    owned
        .iter()
        .map(|o| crate::feed::select::PageView {
            slug: &o.slug,
            path: &o.path,
            frontmatter_feed_optin: o.frontmatter_feed_optin,
            tags: &o.tags,
            matches_spl_query: false,
            item: o.item.clone(),
        })
        .collect()
}

/// Combined emission set for the SPEC-038 outbound feed families:
/// the root feed, every `[[feed.scopes]]` entry, the
/// `/.well-known/zetl-subscriptions.json` catalog, and every
/// `[[capability_cohorts]]` with `feed_enabled = true`.
pub(crate) struct OutboundEmission {
    pub(crate) files: Vec<(String, Vec<u8>)>,
    pub(crate) root_stats: crate::feed::build::BuildStats,
    pub(crate) scoped_count: usize,
    pub(crate) cohort_count: usize,
    pub(crate) catalog_emitted: bool,
}

/// True iff any scoped/cohort feed family needs `FeedItem`s for
/// non-opt-in pages. Scopes that select via `folder`/`tag`/`spl` are
/// authoritative (independent of frontmatter `feed: true`); cohort
/// feeds select via `cohort.select` glob, also independent of opt-in.
fn needs_full_population(lens: &crate::feed::config::FeedConfigLens) -> bool {
    use crate::feed::config::SelectionRuleWire;
    if let Some(feed) = lens.feed.as_ref() {
        for scope in &feed.scopes {
            if matches!(&scope.select, SelectionRuleWire::Detailed(_)) {
                return true;
            }
        }
    }
    lens.capability_cohorts.iter().any(|c| c.feed_enabled)
}

/// SPEC-038 outbound feed emission helper. Walks the parsed vault,
/// synthesises a [`feed::types::FeedItem`] per page that opted in via
/// `frontmatter.feed: true` (or every page when scoped/cohort feeds
/// require it), runs the pure-core feed pipelines (root + scoped +
/// catalog + cohort), and returns the combined file set.
///
/// Returns `Ok(None)` when the [feed] section is absent, disabled,
/// or has no `base_url` — REQ-3809 self-publication safety. Returns
/// `Err` only for hard fatal cases (the pure pipeline asserts).
pub(crate) fn emit_outbound_feeds(
    lens: &crate::feed::config::FeedConfigLens,
    data: &VaultData,
    vault_root: &Path,
    verbose: bool,
) -> Result<Option<OutboundEmission>> {
    use crate::feed::build::emit_root_feed;
    use crate::feed::cap_feed::emit_cohort_feeds;
    use crate::feed::scoped::{emit_catalog, emit_scoped_feeds};
    use crate::feed::select::PageView;
    use crate::feed::types::SelectionRule;

    let Some(feed) = lens.feed.as_ref() else {
        return Ok(None);
    };
    let walk_start = std::time::Instant::now();
    let Some(owned) = build_owned_pages(lens, data, vault_root) else {
        return Ok(None);
    };
    let optin_count = owned.iter().filter(|o| o.frontmatter_feed_optin).count();
    let pages = page_views_from_owned(&owned);

    // Public visibility: exclude cap-protected pages per SPEC-038
    // REQ-3829. The cap-protection check is delegated to feed::cap_feed.
    let visibility: Box<dyn Fn(&PageView<'_>) -> bool> =
        Box::new(crate::feed::cap_feed::public_visibility());
    let rule = SelectionRule::FrontmatterOptIn;

    if verbose {
        eprintln!(
            "[zetl] feed-perf: walk={}ms pages={} optin={}",
            walk_start.elapsed().as_millis(),
            data.files.len(),
            optin_count,
        );
    }
    let root_emission = emit_root_feed(lens, &pages, &visibility, &rule)?;
    let mut combined: Vec<(String, Vec<u8>)> = root_emission.files;

    // ── per-scope feeds (REQ-3813..REQ-3815) ─────────────────────────
    let scoped = emit_scoped_feeds(lens, &pages, &visibility)?;
    let scoped_count = scoped.len();
    for em in scoped {
        combined.extend(em.files);
    }

    // ── /.well-known/zetl-subscriptions.json catalog (REQ-3813) ──────
    let mut catalog_emitted = false;
    if !feed.scopes.is_empty() {
        let identity = build_wiki_identity(lens);
        let (catalog_path, catalog_body) = emit_catalog(lens, &identity)?;
        combined.push((catalog_path, catalog_body));
        catalog_emitted = true;
    }

    // ── per-cohort feeds (REQ-3829, NFR-3809) ────────────────────────
    // Build mode emits cohort feeds for every `feed_enabled = true`
    // cohort. The token segment in the URL gates access; we use a
    // permissive ACL here because per-request ACL doesn't apply at
    // build time (the static body is what each cap holder fetches).
    let cohort_acl = |_cohort_id: &str, _p: &PageView<'_>| true;
    let cohort_emissions = emit_cohort_feeds(lens, &pages, &cohort_acl)?;
    let cohort_count = cohort_emissions.len();
    for em in cohort_emissions {
        combined.extend(em.files);
    }

    Ok(Some(OutboundEmission {
        files: combined,
        root_stats: root_emission.stats,
        scoped_count,
        cohort_count,
        catalog_emitted,
    }))
}

/// Build a [`crate::feed::scoped::WikiIdentity`] from `lens.wiki`. The
/// `[wiki]` section's `id`, `canonical_repo`, and `title` keys
/// round-trip via the `extras` catch-all (see `WikiSection::extras`).
fn build_wiki_identity(
    lens: &crate::feed::config::FeedConfigLens,
) -> crate::feed::scoped::WikiIdentity {
    let Some(wiki) = lens.wiki.as_ref() else {
        return crate::feed::scoped::WikiIdentity::default();
    };
    let extra_str = |k: &str| {
        wiki.extras
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    crate::feed::scoped::WikiIdentity {
        id: extra_str("id"),
        canonical_repo: extra_str("canonical_repo"),
        title: extra_str("title"),
    }
}

fn frontmatter_date(fm: &serde_json::Value) -> Option<String> {
    for key in &["published", "date", "created"] {
        let Some(v) = fm.get(key) else {
            continue;
        };
        if v.is_null() {
            continue;
        }
        // Coerce: prefer `as_str` (handles quoted YAML strings); fall
        // back to JSON stringification (handles unquoted YAML dates
        // like `date: 2026-04-15` that serde_yaml_ng has decoded
        // into a timestamp-shaped value).
        let raw = v
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| v.to_string().trim_matches('"').to_string());
        if raw.is_empty() {
            continue;
        }
        // Bare date YYYY-MM-DD → RFC 3339 midnight UTC.
        if raw.len() == 10 && raw.as_bytes()[4] == b'-' && raw.as_bytes()[7] == b'-' {
            return Some(format!("{raw}T00:00:00Z"));
        }
        return Some(raw);
    }
    None
}

fn mtime_to_rfc3339(mtime: std::time::SystemTime) -> Option<String> {
    crate::feed::datetime::system_time_to_rfc3339(mtime)
}

/// Byte-scan a file body for `feed: <bool>` inside the YAML
/// frontmatter block. Returns:
///   * `Some(true)`  — `feed: true` is set in frontmatter
///   * `Some(false)` — `feed: false` is explicitly set
///   * `None`        — no `feed:` key found, or no frontmatter
///
/// The vast majority of vault pages won't opt in. Returning early
/// from a byte-scan avoids a full `serde_yaml` parse for every page;
/// on a 1009-page benchmark this dropped YAML cost from 3 ms to 0 ms.
/// The full YAML parser only runs on pages this scan flags
/// `Some(true)`.
#[doc(hidden)]
pub fn scan_frontmatter_feed_optin(prelude: &str) -> Option<bool> {
    let trimmed = prelude.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_first = &trimmed[3..];
    let close = after_first.find("\n---")?;
    let frontmatter_block = &after_first[..close];
    // Walk lines looking for `feed:`. Whitespace-tolerant; doesn't
    // handle nested mappings (zetl frontmatter is flat by convention).
    for line in frontmatter_block.lines() {
        let stripped = line.trim_start();
        if let Some(rest) = stripped.strip_prefix("feed:") {
            let value = rest.trim();
            return match value {
                "true" => Some(true),
                "false" => Some(false),
                _ => None, // unknown shape: be conservative
            };
        }
    }
    None
}

fn percent_encode_segment(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            other => write!(out, "%{other:02X}").expect("write to String never fails"),
        }
    }
    out
}

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

/// Write graph-index.json to `{out_dir}/graph-index.json` and return the
/// JSON string so it can be embedded as a template variable when a theme opts
/// in via `graph_inline = true` (SPEC-028 REQ-102, REQ-105).
///
/// Emits OBS-101 `[zetl] graph-export: pages=N edges=M duration_ms=X bytes=Y`
/// under `--verbose`, mirroring the existing `history-export:` instrumentation.
fn write_graph_index_json(
    data: &VaultData,
    vault_root: &Path,
    vault_name: &str,
    out_dir: &Path,
    verbose: bool,
) -> Result<String> {
    let export_start = std::time::Instant::now();

    let mut tags_by_page: HashMap<String, Vec<String>> = HashMap::new();
    for file in &data.files {
        let full_path = vault_root.join(&file.path);
        let Ok(content) = std::fs::read_to_string(&full_path) else {
            continue;
        };
        let fm = markdown::parse_frontmatter(&content);
        let Some(arr) = fm.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        let tags: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !tags.is_empty() {
            tags_by_page.insert(file.page_name.clone(), tags);
        }
    }

    let generated_at = crate::user::access_request::now_iso8601();
    let index = crate::graph::serialize_graph_index(
        &data.graph,
        &data.page_slug_map,
        &tags_by_page,
        vault_name,
        &generated_at,
    );
    let json_str = serde_json::to_string(&index).context("serializing graph-index.json")?;
    std::fs::write(out_dir.join("graph-index.json"), &json_str)
        .context("writing graph-index.json")?;

    if verbose {
        let pages = index
            .get("attributes")
            .and_then(|a| a.get("vault"))
            .and_then(|v| v.get("pages"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let edges = index
            .get("edges")
            .and_then(|e| e.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        eprintln!(
            "[zetl] graph-export: pages={} edges={} duration_ms={} bytes={}",
            pages,
            edges,
            export_start.elapsed().as_millis(),
            json_str.len(),
        );
    }

    Ok(json_str)
}

/// Build a `GraphIndexContext` from the current vault snapshot.
///
/// Pure data assembly on top of `VaultData` + frontmatter I/O (for `tags`).
/// Shared between `zetl build` (writes `graph-index.json` to disk) and
/// `zetl serve` (`GET /graph-index.json`) so the two outputs are byte-equal
/// for the same vault state — REQ-102 / REQ-103.
pub fn build_graph_index_context<'a>(
    data: &VaultData,
    vault_root: &Path,
    vault_name: &'a str,
) -> GraphIndexContext<'a> {
    let graph = &data.graph;

    // Map from page_name → file path, for cheap frontmatter lookup.
    let file_by_name: HashMap<&str, &Path> = data
        .files
        .iter()
        .map(|f| (f.page_name.as_str(), f.path.as_path()))
        .collect();

    // Phantom (dead-link) targets have no entry in `page_slug_map`. Derive a
    // stable kebab-case slug from the page name so node keys remain URL-safe
    // and match build-mode output.
    fn phantom_slug(name: &str) -> String {
        name.to_ascii_lowercase().replace(' ', "-")
    }

    let mut nodes: Vec<GraphIndexNode> = graph
        .node_map
        .keys()
        .map(|name| {
            let slug = data
                .page_slug_map
                .get(name)
                .cloned()
                .unwrap_or_else(|| phantom_slug(name));
            let outlink_count = graph
                .graph
                .edges_directed(graph.node_map[name], petgraph::Direction::Outgoing)
                .count();
            let backlink_count = graph
                .graph
                .edges_directed(graph.node_map[name], petgraph::Direction::Incoming)
                .count();
            let is_real = graph.resolved.contains(name);
            let is_dead = !is_real;
            let is_orphan = is_real && backlink_count == 0;

            let tags: Vec<String> = if is_real {
                file_by_name
                    .get(name.as_str())
                    .and_then(|rel| std::fs::read_to_string(vault_root.join(rel)).ok())
                    .map(|content| {
                        let fm = markdown::parse_frontmatter(&content);
                        fm.get("tags")
                            .and_then(|t| t.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            GraphIndexNode {
                label: name.clone(),
                slug,
                outlink_count,
                backlink_count,
                is_orphan,
                is_dead,
                tags,
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.slug.cmp(&b.slug));

    let resolve_slug = |name: &str| -> String {
        data.page_slug_map
            .get(name)
            .cloned()
            .unwrap_or_else(|| phantom_slug(name))
    };
    let edges: Vec<GraphIndexEdge> = graph
        .graph
        .edge_references()
        .map(|e| {
            let src_name = &graph.graph[e.source()];
            let tgt_name = &graph.graph[e.target()];
            GraphIndexEdge {
                source: resolve_slug(src_name),
                target: resolve_slug(tgt_name),
            }
        })
        .collect();

    let stats = graph.stats(0);

    GraphIndexContext {
        vault_name,
        total_pages: stats.pages,
        total_links: stats.links,
        nodes,
        edges,
    }
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

/// Summary of a completed build operation.
pub struct BuildResult {
    pub pages: usize,
    pub folder_indexes: usize,
    pub out_dir: String,
}

/// Generate a complete static HTML site from the vault data.
pub fn build_static(
    data: &VaultData,
    vault_root: &Path,
    out_dir: &str,
    theme: &str,
    verbose: bool,
    public: Option<&str>,
    site_url: Option<&str>,
) -> Result<BuildResult> {
    // Normalise: strip trailing '/' so `{site_url}/{slug}/og.png` stays clean.
    let site_url = site_url
        .map(|u| u.trim_end_matches('/').to_string())
        .unwrap_or_default();
    let out = Path::new(out_dir);
    std::fs::create_dir_all(out)
        .with_context(|| format!("Cannot create output directory: {out_dir}"))?;

    let engine = TemplateEngine::new(vault_root, theme, false, verbose);
    let vault_name = vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());
    #[allow(unused_mut)]
    let mut vault_ctx = build_vault_context(data, &vault_name);
    vault_ctx.site_url = site_url.clone();
    // ── SPEC-038 feed setup: discovery tags + body emission ──────
    //
    // Hard-fatal classes (cohort token below NFR-3809 floor; [feed]
    // configured but missing base_url per REQ-3809 self-publication
    // safety; duplicate ids; etc.) bubble up from
    // [`crate::feed::config::parse_config`] and fail the build with
    // a non-zero exit code. Soft-fatal classes (e.g. unknown TOML
    // keys — `deny_unknown_fields` rejects, surfaced as `Toml(...)`)
    // also fail the build because typos are the only signal an
    // operator gets that they've misconfigured something.
    //
    // The only non-fatal case is "no config.toml at all" — vaults
    // without `[feed]` deserve to build silently.
    let config_path = vault_root.join(".zetl/config.toml");
    let feed_setup = match std::fs::read_to_string(&config_path) {
        Ok(body) => match crate::feed::config::parse_config(&body) {
            Ok(lens) => Some(lens),
            Err(crate::feed::config::FeedConfigError::Toml(msg)) => {
                // Toml errors are usually typos. Hard-fail with a clear
                // pointer to the file + the exact message.
                anyhow::bail!(
                    "[zetl] feed-config: {} (REQ-3809 / NFR-3807): {msg}",
                    config_path.display(),
                );
            }
            Err(other) => {
                // Validation errors (cohort entropy, retention, mode,
                // license, max_items, archive_size, scope/sub id
                // collisions, no-permission republish) all reach here.
                anyhow::bail!(
                    "[zetl] feed-config: {} (REQ-3809 / NFR-3807 / NFR-3809): {other}",
                    config_path.display(),
                );
            }
        },
        Err(_) => None, // No .zetl/config.toml -> no feed setup. Fine.
    };
    if let Some(lens) = feed_setup.as_ref() {
        if let Some(feed) = lens.feed.as_ref() {
            // `enabled = false` is an explicit operator opt-out: skip
            // discovery-tag injection AND the later emit_outbound_feeds
            // call so the build doesn't fail with BuildError::Disabled.
            if !matches!(feed.enabled, Some(false)) {
                // [feed] is configured but base_url is the only required
                // outbound field. Missing it is REQ-3809 self-publication
                // safety: hard-fail.
                let base_url = feed.base_url.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "[zetl] feed-config: [feed] is configured but base_url is missing (REQ-3809)"
                    )
                })?;
                let title = feed.title.as_deref().unwrap_or("Untitled feed");
                vault_ctx.feed_discovery = build_feed_discovery_tags(feed, base_url, title);
            }
        }
    }
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

    // ── search-index.json + pages.json (external files, not inlined) ──
    // Both are fetched lazily by the Cmd+K search JS on first open, so they
    // stay off the critical rendering path.
    write_search_index_json(data, vault_root, out)?;
    let pages_json = build_search_index(&vault_ctx);
    std::fs::write(out.join("pages.json"), &pages_json).context("writing pages.json")?;
    let bm25_json = String::new();

    // ── graph-index.json (SPEC-028 REQ-102 / OBS-101) ───────────────────
    let graph_json = write_graph_index_json(data, vault_root, &vault_ctx.name, out, verbose)?;

    // ── _graph/index.html (SPEC-028 REQ-107) ────────────────────────────
    // Nested under its own slug directory so the inline `root: "../"` and
    // `<script src="../_static/...">` references the partial emits resolve
    // correctly (they assume one level of nesting, matching every other
    // build-mode page). Sidebar/expand links point at `_graph/index.html`.
    let graph_html = engine
        .render_vault_graph(&vault_ctx, "build", &graph_json)
        .map_err(|e| {
            eprintln!("{}", e.stderr_line("_graph"));
            anyhow::anyhow!("{e}")
        })?;
    let graph_dir = out.join("_graph");
    std::fs::create_dir_all(&graph_dir)?;
    std::fs::write(graph_dir.join("index.html"), graph_html)?;

    // ── history-index.json ───────────────────────────────────────────────
    #[cfg(feature = "history")]
    let history_json = write_history_index_json(data, vault_root, out, verbose);
    #[cfg(not(feature = "history"))]
    let history_json = String::new();

    // ── index.html ──────────────────────────────────────────────────────
    let index_html = engine
        .render_index(&vault_ctx, "build", &bm25_json, &history_json, "")
        .map_err(|e| {
            eprintln!("{}", e.stderr_line("index"));
            anyhow::anyhow!("{e}")
        })?;
    std::fs::write(out.join("index.html"), index_html)?;

    // ── OG images ───────────────────────────────────────────────────────
    // Generate a 1200×630 PNG per page for social sharing. A user-supplied
    // background (at .zetl/static/og-background.png or the theme's static
    // directory) is composited underneath when present.
    let og_bg = crate::web::og::load_background(vault_root, None);
    let og_start = std::time::Instant::now();
    match crate::web::og::render_og_png(&vault_ctx.name, "a knowledge vault", og_bg.as_ref()) {
        Ok(bytes) => std::fs::write(out.join("og.png"), &bytes).context("writing vault og.png")?,
        Err(e) => {
            if verbose {
                eprintln!("[zetl] og: skipping vault image: {e}");
            }
        }
    }
    let mut og_count = 1usize;

    // ── help page ───────────────────────────────────────────────────────
    let help_html = engine.render_help(&vault_ctx, "build").map_err(|e| {
        eprintln!("{}", e.stderr_line("help"));
        anyhow::anyhow!("{e}")
    })?;
    let help_dir = out.join("help");
    std::fs::create_dir_all(&help_dir)?;
    std::fs::write(help_dir.join("index.html"), help_html)?;

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
        let is_fountain = file.path.extension().is_some_and(|e| e == "fountain");
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
            .render_page(
                &vault_ctx,
                &page_ctx,
                "build",
                &bm25_json,
                &history_json,
                "",
            )
            .map_err(|e| {
                eprintln!("{}", e.stderr_line(&slug));
                anyhow::anyhow!("{e}")
            })?;
        std::fs::write(page_dir.join("index.html"), page_html)?;

        // Per-page raw source alongside the rendered HTML, so LLM agents can
        // fetch the original markdown (or fountain, spl, etc.) without
        // parsing HTML. Extension matches the source file.
        let src_ext = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("md");
        std::fs::write(page_dir.join(format!("index.{src_ext}")), &content)?;

        // SPEC-027 REQ-302 / adversarial defect 5: static emission of
        // per-page history HTML. Gate on last_changed presence so the
        // static file is never emitted when page.html's inline link
        // guard (`page.history.last_changed`) would suppress the link —
        // prevents orphan artefacts.
        #[cfg(feature = "history")]
        if page_ctx
            .history
            .get("last_changed")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
        {
            let hist_start = std::time::Instant::now();
            let breadcrumbs: Vec<crate::web::context::BreadcrumbEntry> = {
                let parts: Vec<&str> = slug.split('/').collect();
                let mut crumbs = Vec::new();
                for i in 0..parts.len().saturating_sub(1) {
                    let s = parts[..=i].join("/");
                    crumbs.push(crate::web::context::BreadcrumbEntry {
                        title: parts[i].to_string(),
                        slug: s,
                    });
                }
                crumbs
            };
            // Adversarial defect 2 (REQ-302 parity): serve passes real
            // git log entries; build previously passed `"[]"` which left
            // the template's client-side chart empty. Load git file_log
            // directly when the vault is inside a git repo.
            let git_entries_json = match git2::Repository::discover(vault_root) {
                Ok(repo) => {
                    let entries = crate::web::git_commit::file_log(&repo, &file.path, 100);
                    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
                }
                Err(_) => "[]".to_string(),
            };
            let ph_html = engine
                .render_page_history(
                    &vault_ctx,
                    &file.page_name,
                    &slug,
                    &breadcrumbs,
                    &git_entries_json,
                    &page_ctx.history,
                    false,
                    "build",
                )
                .map_err(|e| {
                    eprintln!("{}", e.stderr_line(&slug));
                    anyhow::anyhow!("{e}")
                })?;
            let hist_bytes = ph_html.len();
            std::fs::write(page_dir.join("_history.html"), ph_html)?;
            if verbose {
                eprintln!(
                    "[zetl] history-static: page {:?} slug={:?} bytes={} duration_ms={}",
                    file.page_name,
                    slug,
                    hist_bytes,
                    hist_start.elapsed().as_millis()
                );
            }
        }

        // Per-page OG image.
        match crate::web::og::render_og_png(&file.page_name, &vault_ctx.name, og_bg.as_ref()) {
            Ok(bytes) => {
                std::fs::write(page_dir.join("og.png"), &bytes).context("writing page og.png")?;
                og_count += 1;
            }
            Err(e) if verbose => {
                eprintln!("[zetl] og: skipping {}: {e}", file.page_name);
            }
            Err(_) => {}
        }

        count += 1;
    }
    if verbose {
        eprintln!(
            "[zetl] og: wrote {og_count} images in {} ms",
            og_start.elapsed().as_millis()
        );
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
            .render_folder(
                &vault_ctx,
                &folder_ctx,
                "build",
                &bm25_json,
                &history_json,
                "",
            )
            .map_err(|e| {
                eprintln!("{}", e.stderr_line(folder));
                anyhow::anyhow!("{e}")
            })?;
        std::fs::write(folder_dir.join("index.html"), folder_html)?;
        folder_count += 1;
    }

    // ── sitemap.xml + llms.txt (agent discovery artefacts) ──
    // robots.txt itself is emitted below via `merge_robots_txt` so the
    // SPEC-034 REQ-3418 / CON-3406 disallow lines layer correctly onto
    // any operator-supplied file from the public overlay.
    std::fs::write(out.join("sitemap.xml"), build_sitemap(&vault_ctx))
        .context("writing sitemap.xml")?;
    std::fs::write(out.join("llms.txt"), build_llms_txt(&vault_ctx, "build"))
        .context("writing llms.txt")?;

    // ── SPEC-038 feed bodies (REQ-3801 + REQ-3808 + REQ-3809) ─────
    // Emit dist/feed.xml + dist/atom.xml + (optional) dist/feed.json
    // backing the rel=alternate discovery tags injected into <head>.
    // Feeds are only emitted when [feed] is configured AND base_url
    // is set; otherwise the discovery tags would advertise URLs that
    // 404 (the bug this branch fixes).
    if let Some(lens) = feed_setup.as_ref() {
        if let Some(emission) = emit_outbound_feeds(lens, data, vault_root, verbose)? {
            for (rel_path, body) in &emission.files {
                let target = out.join(rel_path.trim_start_matches('/'));
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&target, body)
                    .with_context(|| format!("writing feed body {}", target.display()))?;
            }
            if verbose {
                eprintln!(
                    "[zetl] feed: emitted {} file(s); items_selected={} items_emitted={} scopes={} cohorts={} catalog={} root_duration_ms={}",
                    emission.files.len(),
                    emission.root_stats.items_selected,
                    emission.root_stats.items_emitted,
                    emission.scoped_count,
                    emission.cohort_count,
                    emission.catalog_emitted,
                    emission.root_stats.duration.as_millis(),
                );
            }
        }
    }
    // Cloudflare Pages / Netlify-style _headers: long-cache hashed static assets and JSON indexes.
    let headers = "/_static/*\n  Cache-Control: public, max-age=31536000, immutable\n\n/*.json\n  Cache-Control: public, max-age=3600\n";
    if !out.join("_headers").exists() {
        std::fs::write(out.join("_headers"), headers)?;
    }

    // ── static assets ─────────────────────────────────────────────────
    let static_copied = copy_static_assets(vault_root, out, theme)?;

    // ── public overlay (copies over output root, overwriting generated pages) ──
    let public_copied = if let Some(pub_dir) = public {
        let pub_path = Path::new(pub_dir);
        if pub_path.is_dir() {
            copy_dir_recursive(pub_path, out)?;
            if verbose {
                eprintln!(
                    "[zetl] public overlay: copied {} → {}",
                    pub_path.display(),
                    out.display()
                );
            }
            true
        } else {
            eprintln!("warning: --public directory does not exist: {pub_dir}");
            false
        }
    } else {
        false
    };

    // ── robots.txt (SPEC-034 REQ-3418 / CON-3406) ─────────────────────
    // Written after the public overlay so the operator's own
    // `public/robots.txt` (if any) forms the base of the merge. Emitted
    // unconditionally — even when `--capability` is off, the `/c/` and
    // `/_zetl/` paths are reserved for the capability build and should
    // stay off search-engine index for any vault that will ever ship
    // encrypted pages. Operator's content is preserved verbatim;
    // stricter rules (e.g. `Disallow: /`) are never relaxed.
    let operator_robots = std::fs::read_to_string(out.join("robots.txt")).ok();
    let merged_robots = crate::web::robots::merge_robots_txt(operator_robots.as_deref());
    std::fs::write(out.join("robots.txt"), merged_robots)?;

    // SPEC-027 REQ-303: static emission of vault-wide history page.
    // Renders even when history is absent — the template's graceful-absence
    // branch produces a short "No history yet" body.
    #[cfg(feature = "history")]
    {
        use crate::history::jj_backend::VcsBackend;
        let vh_start = std::time::Instant::now();
        // REQ-306: template-overridable via vault.history.recent_limit.
        let vh_limit: usize = vault_ctx
            .history
            .get("recent_limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(50);
        let recent_changes = match crate::history::open_history(vault_root) {
            Ok(backend) => match backend.list_changes(10_000) {
                Ok(snaps) => {
                    crate::history::core::build_recent_changes(&snaps, vault_root, vh_limit)
                        .unwrap_or_default()
                }
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        let sparkline: Vec<f32> = vault_ctx
            .history
            .get("trend")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|pt| {
                        pt.get("total_links")
                            .and_then(|v| v.as_f64())
                            .map(|f| f as f32)
                    })
                    .collect()
            })
            .unwrap_or_default();
        match engine.render_vault_history(&vault_ctx, &recent_changes, &sparkline, "build") {
            Ok(html) => {
                std::fs::write(out.join("_history.html"), html)?;
                if verbose {
                    eprintln!(
                        "[zetl] history-static: vault entries={} duration_ms={}",
                        recent_changes.len(),
                        vh_start.elapsed().as_millis()
                    );
                }
            }
            Err(e) => {
                eprintln!("[zetl] history-static: vault render failed: {e}");
            }
        }
    }

    // ── tag-cloud page (/tag-cloud/index.html) ──────────────────────────
    // Skip when a real file already claims the `tag-cloud` slug so we don't
    // shadow a user-authored `Tag Cloud.md`.
    let tag_cloud_taken = data
        .files
        .iter()
        .any(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case("tag-cloud"));
    if !tag_cloud_taken {
        let tag_cloud_ctx = crate::web::context::build_tag_cloud_context(data, vault_root);
        match engine.render_tag_cloud(&vault_ctx, &tag_cloud_ctx, "build") {
            Ok(html) => {
                let tc_dir = out.join("tag-cloud");
                std::fs::create_dir_all(&tc_dir)?;
                std::fs::write(tc_dir.join("index.html"), html)?;
                if verbose {
                    eprintln!(
                        "[zetl] tag-cloud: tags={} tagged_pages={}",
                        tag_cloud_ctx.total_tags, tag_cloud_ctx.total_tagged_pages
                    );
                }
            }
            Err(e) => {
                eprintln!("{}", e.stderr_line("tag-cloud"));
                return Err(anyhow::anyhow!("{e}"));
            }
        }
    }

    if let Some(warning) = graph_index_size_warning(out, count) {
        eprintln!("warning: {warning}");
    }

    let suffix = match (static_copied, public_copied) {
        (true, true) => " (static assets + public overlay copied)",
        (true, false) => " (static assets copied)",
        (false, true) => " (public overlay copied)",
        (false, false) => "",
    };
    eprintln!(
        "zetl build  →  {count} pages + {folder_count} folder indexes written to {out_dir}/{suffix}",
    );

    // Brotli pre-compression (task-brotli-precompress-build). Walk the output
    // and emit `{file}.br` alongside each `.html`, `.css`, `.js`, `.json` so
    // static hosts (Netlify, Cloudflare, nginx with brotli_static on) can
    // serve the precompressed byte-for-byte. Opt-out via ZETL_NO_BROTLI=1.
    if std::env::var("ZETL_NO_BROTLI").ok().as_deref() != Some("1") {
        let (files, bytes_in, bytes_out) = precompress_brotli(out, verbose)?;
        if verbose {
            eprintln!("[zetl] brotli: {files} files precompressed, {bytes_in} → {bytes_out} bytes");
        } else if files > 0 {
            eprintln!("  brotli: {files} files precompressed");
        }
    }

    Ok(BuildResult {
        pages: count,
        folder_indexes: folder_count,
        out_dir: out_dir.to_string(),
    })
}

/// Walk `out` and write a `.br` sibling for every HTML/CSS/JS/JSON file whose
/// brotli-compressed form is at least 10% smaller. Returns (count, bytes_in,
/// bytes_out) for reporting. Errors on individual files are logged but don't
/// abort the build — a partial precompress set is still useful to deploy.
fn precompress_brotli(out: &Path, verbose: bool) -> Result<(usize, u64, u64)> {
    use std::io::Write;
    fn should_compress(p: &Path) -> bool {
        matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("html" | "htm" | "css" | "js" | "mjs" | "json" | "svg" | "txt")
        )
    }
    fn visit(
        dir: &Path,
        files: &mut usize,
        bytes_in: &mut u64,
        bytes_out: &mut u64,
        verbose: bool,
    ) -> Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, files, bytes_in, bytes_out, verbose)?;
                continue;
            }
            if !path.is_file() || !should_compress(&path) {
                continue;
            }
            let raw = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    if verbose {
                        eprintln!("[zetl] brotli: skip {} ({e})", path.display());
                    }
                    continue;
                }
            };
            // Quality 5 is a good server-side-precompress trade-off: ~15 ms
            // per 100 KB on a laptop, output size within 2% of max (quality 11).
            let mut encoded = Vec::new();
            {
                let mut w = brotli::CompressorWriter::new(&mut encoded, 4096, 5, 22);
                if let Err(e) = w.write_all(&raw) {
                    if verbose {
                        eprintln!("[zetl] brotli: encode failed for {} ({e})", path.display());
                    }
                    continue;
                }
            }
            // Skip if compression gave less than 10% — below that the extra
            // .br file isn't worth the deploy-time bytes.
            if encoded.len() as f64 > raw.len() as f64 * 0.9 {
                continue;
            }
            let br_path = {
                let mut p = path.clone().into_os_string();
                p.push(".br");
                std::path::PathBuf::from(p)
            };
            if let Err(e) = std::fs::write(&br_path, &encoded) {
                if verbose {
                    eprintln!(
                        "[zetl] brotli: write failed for {} ({e})",
                        br_path.display()
                    );
                }
                continue;
            }
            *files += 1;
            *bytes_in += raw.len() as u64;
            *bytes_out += encoded.len() as u64;
        }
        Ok(())
    }
    let mut files = 0usize;
    let mut bytes_in = 0u64;
    let mut bytes_out = 0u64;
    visit(out, &mut files, &mut bytes_in, &mut bytes_out, verbose)?;
    Ok((files, bytes_in, bytes_out))
}

/// NFR-104: graph-index.json uncompressed size budget (1 MB).
pub const GRAPH_INDEX_MAX_BYTES: u64 = 1024 * 1024;

/// NFR-104: page-count proxy threshold above which the build emits the
/// size-budget warning even when graph-index.json is not yet emitted.
pub const GRAPH_PAGE_WARN_THRESHOLD: usize = 5_000;

/// NFR-104 (TEST-204): produce the warning text when the emitted
/// graph-index.json exceeds the 1 MB budget, or — as a page-count proxy —
/// when the vault itself exceeds [`GRAPH_PAGE_WARN_THRESHOLD`] pages. Returns
/// `None` when both checks pass.
pub fn graph_index_size_warning(out_dir: &Path, page_count: usize) -> Option<String> {
    let graph_index_path = out_dir.join("graph-index.json");
    let graph_index_bytes = std::fs::metadata(&graph_index_path).ok().map(|m| m.len());
    let graph_over_budget = graph_index_bytes.is_some_and(|n| n > GRAPH_INDEX_MAX_BYTES);
    let vault_over_budget = page_count >= GRAPH_PAGE_WARN_THRESHOLD;
    if !graph_over_budget && !vault_over_budget {
        return None;
    }
    let detail = match graph_index_bytes {
        Some(n) if graph_over_budget => format!(
            "graph-index.json is {:.2} MB (> 1 MB budget, NFR-104)",
            n as f64 / (1024.0 * 1024.0)
        ),
        _ => format!(
            "vault has {page_count} pages (≥ {GRAPH_PAGE_WARN_THRESHOLD}); graph-index.json will exceed the 1 MB budget (NFR-104)"
        ),
    };
    Some(format!(
        "{detail} — consider `zetl serve` (server-mode deployment) or link-graph filtering"
    ))
}

/// Copy static assets from `.zetl/static/`, `.zetl/themes/<theme>/static/`, and
/// the bundled theme's `static/` directory into `{out}/_static/`.
///
/// Priority (lowest to highest): bundled default → bundled <theme> → vault
/// shared → installed theme. A user-created theme (e.g. `quickstart`) that
/// only overrides templates still inherits bundled `default` static
/// assets (theme.css, shell.css, vendored sigma.js) so the built site
/// isn't broken.
/// Returns `true` if any files were copied.
fn copy_static_assets(vault_root: &Path, out: &Path, theme: &str) -> Result<bool> {
    let shared_static = vault_root.join(".zetl/static");
    let theme_static = vault_root.join(format!(".zetl/themes/{theme}/static"));

    let shared_exists = shared_static.is_dir();
    let theme_exists = theme != "default" && theme_static.is_dir();

    // Collect bundled theme static files (paths beginning with "static/").
    // Ultimate-fallback pass: bundled `default` supplies core assets when
    // the active theme is not a bundled one. Primary pass: the active
    // theme's bundled files (if it is a bundled theme) overwrite where
    // they overlap.
    let mut bundled_statics: Vec<(std::path::PathBuf, Vec<u8>)> = Vec::new();
    if theme != "default" {
        bundled_statics.extend(
            bundled_theme_files("default")
                .into_iter()
                .filter(|(p, _)| p.starts_with("static")),
        );
    }
    for (p, bytes) in bundled_theme_files(theme)
        .into_iter()
        .filter(|(p, _)| p.starts_with("static"))
    {
        // Overwrite any same-path entry from the default fallback.
        if let Some(existing) = bundled_statics.iter_mut().find(|(ep, _)| ep == &p) {
            existing.1 = bytes;
        } else {
            bundled_statics.push((p, bytes));
        }
    }
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
  <a href="{root_path}{href}/index.html" class="tc-title">{name}</a>
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
    fn user_theme_without_on_disk_dirs_ships_bundled_default() {
        // With the bundled-default fallback in place (so user-created
        // themes don't 404 on theme.css / shell.css), the old
        // "no dirs → no copy" skip path is only reachable in degenerate
        // builds where even the bundled `default` theme has no `static/`
        // subtree. A non-bundled theme name with no on-disk theme dir
        // and no shared vault static dir now inherits default's assets.
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let result = copy_static_assets(tmp.path(), &out, "__user_theme__").unwrap();
        assert!(result, "default-theme fallback should have kicked in");
        assert!(
            out.join("_static").is_dir(),
            "_static/ should exist after copy"
        );
        let copied = std::fs::read_dir(out.join("_static")).unwrap().count();
        assert!(
            copied > 0,
            "at least one bundled default asset should be copied"
        );
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
