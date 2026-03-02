use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::scanner::page_slug_from_path;
use crate::web::chain::{compute_chain_position, extract_chain_target, resolve_page_name};
use crate::web::markdown::parse_frontmatter;
use crate::web::VaultData;

// ── Shared structs ──────────────────────────────────────────────────

#[derive(Serialize)]
pub struct VaultContext {
    pub name: String,
    pub pages: Vec<PageEntry>,
    pub stats: StatsContext,
}

#[derive(Serialize)]
pub struct PageEntry {
    pub title: String,
    pub slug: String,
    pub outlink_count: usize,
    pub backlink_count: usize,
    /// True if this page is the head (first page) of a chain (prev=None, next=Some).
    pub is_chain_head: bool,
    /// True if this page appears in any chain (as head, middle, or tail).
    pub is_in_chain: bool,
    /// For chain heads, the total number of pages in the chain; None otherwise.
    pub chain_length: Option<usize>,
}

#[derive(Serialize)]
pub struct StatsContext {
    pub total_pages: usize,
    pub total_links: usize,
    pub dead_links: usize,
    pub orphans: usize,
}

#[derive(Serialize)]
pub struct ZetlMeta {
    pub version: String,
}

#[derive(Serialize)]
pub struct BreadcrumbEntry {
    pub title: String,
    pub slug: String,
}

// ── Page-specific structs ───────────────────────────────────────────

#[derive(Serialize)]
pub struct ChainLink {
    pub title: String,
    pub slug: String,
}

#[derive(Serialize)]
pub struct BacklinkEntry {
    pub title: String,
    pub slug: String,
    pub line: usize,
}

#[derive(Serialize)]
pub struct OutlinkEntry {
    pub title: String,
    pub slug: String,
    pub is_dead: bool,
    pub color: String,
}

#[derive(Serialize)]
pub struct PageContext {
    pub title: String,
    pub slug: String,
    pub content_html: String,
    pub content_raw: String,
    pub frontmatter: serde_json::Value,
    pub backlinks: Vec<BacklinkEntry>,
    pub outlinks: Vec<OutlinkEntry>,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub transclusion_cards: String,
    pub is_new: bool,
    pub raw_escaped: Option<String>,
    pub prev_page: Option<ChainLink>,
    pub next_page: Option<ChainLink>,
    pub chain_position: Option<usize>,
    pub chain_length: Option<usize>,
    pub chain_head_slug: Option<String>,
}

// ── Folder-specific structs ─────────────────────────────────────────

#[derive(Serialize)]
pub struct SubfolderEntry {
    pub name: String,
    pub slug: String,
    pub page_count: usize,
}

#[derive(Serialize)]
pub struct FolderContext {
    pub name: String,
    pub slug: String,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub subfolders: Vec<SubfolderEntry>,
    pub pages: Vec<PageEntry>,
    pub total_pages: usize,
}

// ── Builder functions ───────────────────────────────────────────────

/// Build a `VaultContext` from `VaultData` and a vault name.
pub fn build_vault_context(data: &VaultData, vault_name: &str) -> VaultContext {
    use crate::web::chain::find_chain_heads;

    let graph_stats = data.graph.stats(0);

    let chain_heads_set: HashSet<String> =
        find_chain_heads(&data.chain_prev_next).into_iter().collect();

    let mut pages: Vec<PageEntry> = data
        .files
        .iter()
        .map(|file| {
            let slug = page_slug_from_path(&file.path);
            let outlink_count = data.graph.forward_links(&file.page_name).len();
            let backlink_count = data.graph.backlinks(&file.page_name).len();
            let is_chain_head = chain_heads_set.contains(&file.page_name);
            let is_in_chain = data.chain_prev_next.contains_key(&file.page_name);
            let chain_length = if is_chain_head {
                Some(walk_chain_length(&file.page_name, &data.chain_prev_next))
            } else {
                None
            };
            PageEntry {
                title: file.page_name.clone(),
                slug,
                outlink_count,
                backlink_count,
                is_chain_head,
                is_in_chain,
                chain_length,
            }
        })
        .collect();
    pages.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let stats = StatsContext {
        total_pages: graph_stats.pages,
        total_links: graph_stats.links,
        dead_links: graph_stats.dead_links,
        orphans: graph_stats.orphans,
    };

    VaultContext {
        name: vault_name.to_string(),
        pages,
        stats,
    }
}

/// Build a `ZetlMeta` using the crate version from Cargo.toml.
pub fn build_zetl_meta() -> ZetlMeta {
    ZetlMeta {
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Build breadcrumbs from a slug (e.g. "architecture/Scanner" → [{title: "architecture", slug: "architecture"}, ...]).
pub fn build_breadcrumbs(slug: &str) -> Vec<BreadcrumbEntry> {
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() <= 1 {
        return Vec::new();
    }
    // All segments except the last are folder breadcrumbs
    let mut crumbs = Vec::new();
    let mut path = String::new();
    for part in &parts[..parts.len() - 1] {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(part);
        crumbs.push(BreadcrumbEntry {
            title: part.to_string(),
            slug: path.clone(),
        });
    }
    crumbs
}

/// Build a `PageContext` from vault data and page-specific inputs.
///
/// `content_html` and `content_raw` are passed in because rendering is handled
/// by the caller (markdown module). Frontmatter is parsed from `content_raw`.
pub fn build_page_context(
    data: &VaultData,
    page_name: &str,
    slug: &str,
    content_html: &str,
    content_raw: &str,
) -> PageContext {
    let backlinks: Vec<BacklinkEntry> = data
        .graph
        .backlinks(page_name)
        .into_iter()
        .map(|bl| {
            let bl_slug = data
                .page_slug_map
                .get(&bl.source)
                .cloned()
                .unwrap_or_default();
            BacklinkEntry {
                title: bl.source,
                slug: bl_slug,
                line: bl.line as usize,
            }
        })
        .collect();

    let outlinks: Vec<OutlinkEntry> = data
        .graph
        .forward_links(page_name)
        .into_iter()
        .map(|fl| {
            let is_dead = !data.resolved.contains(&fl.target);
            let fl_slug = data
                .page_slug_map
                .get(&fl.target)
                .cloned()
                .unwrap_or_default();
            let color = if is_dead {
                "red".to_string()
            } else {
                "blue".to_string()
            };
            OutlinkEntry {
                title: fl.target,
                slug: fl_slug,
                is_dead,
                color,
            }
        })
        .collect();

    let breadcrumbs = build_breadcrumbs(slug);
    let is_new = !data.resolved.contains(page_name);
    let frontmatter = parse_frontmatter(content_raw);

    let prev_page = extract_chain_target(&frontmatter, "prev").and_then(|name| {
        resolve_page_name(&name, &data.page_slug_map).map(|s| ChainLink { title: name, slug: s })
    });
    let next_page = extract_chain_target(&frontmatter, "next").and_then(|name| {
        resolve_page_name(&name, &data.page_slug_map).map(|s| ChainLink { title: name, slug: s })
    });

    let (chain_position, chain_length, chain_head_slug) =
        compute_chain_position(page_name, &data.chain_prev_next, &data.page_slug_map)
            .map(|(pos, len, head)| (Some(pos), Some(len), Some(head)))
            .unwrap_or((None, None, None));

    PageContext {
        title: page_name.to_string(),
        slug: slug.to_string(),
        content_html: content_html.to_string(),
        content_raw: content_raw.to_string(),
        frontmatter,
        backlinks,
        outlinks,
        breadcrumbs,
        transclusion_cards: String::new(),
        is_new,
        raw_escaped: None,
        prev_page,
        next_page,
        chain_position,
        chain_length,
        chain_head_slug,
    }
}

/// Build a `FolderContext` from vault data for a given folder slug.
///
/// Scans `page_slug_map` to find pages and subfolders within the folder.
pub fn build_folder_context(
    data: &VaultData,
    folder_slug: &str,
    folder_name: &str,
) -> FolderContext {
    let prefix = if folder_slug.is_empty() {
        String::new()
    } else {
        format!("{folder_slug}/")
    };

    let mut pages = Vec::new();
    let mut subfolder_counts: HashMap<String, usize> = HashMap::new();

    for (page_name, slug) in &data.page_slug_map {
        // For root folder, all pages are descendants.
        // For nested folders, match pages that start with the prefix.
        let is_match = if prefix.is_empty() {
            true
        } else {
            slug.starts_with(&prefix)
        };

        if !is_match {
            continue;
        }

        // Remainder after stripping the folder prefix
        let remainder = if prefix.is_empty() {
            slug.as_str()
        } else {
            &slug[prefix.len()..]
        };

        if remainder.contains('/') {
            // This is in a subfolder — extract the immediate subfolder name
            let sub_name = remainder.split('/').next().unwrap();
            *subfolder_counts.entry(sub_name.to_string()).or_insert(0) += 1;
        } else {
            // Direct child page
            let outlink_count = data.graph.forward_links(page_name).len();
            let backlink_count = data.graph.backlinks(page_name).len();
            let is_chain_head = data
                .chain_prev_next
                .get(page_name)
                .map_or(false, |(prev, next)| prev.is_none() && next.is_some());
            let is_in_chain = data.chain_prev_next.contains_key(page_name);
            let chain_length = if is_chain_head {
                Some(walk_chain_length(page_name, &data.chain_prev_next))
            } else {
                None
            };
            pages.push(PageEntry {
                title: page_name.clone(),
                slug: slug.clone(),
                outlink_count,
                backlink_count,
                is_chain_head,
                is_in_chain,
                chain_length,
            });
        }
    }

    pages.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let mut subfolders: Vec<SubfolderEntry> = subfolder_counts
        .into_iter()
        .map(|(name, page_count)| {
            let sub_slug = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}{name}")
            };
            SubfolderEntry {
                name,
                slug: sub_slug,
                page_count,
            }
        })
        .collect();
    subfolders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    let total_pages = pages.len();
    let breadcrumbs = build_breadcrumbs(folder_slug);

    FolderContext {
        name: folder_name.to_string(),
        slug: folder_slug.to_string(),
        breadcrumbs,
        subfolders,
        pages,
        total_pages,
    }
}

/// Walk a chain starting at `head`, following `next` pointers, and return the total length.
///
/// Stops when there is no next pointer or a cycle is detected.
fn walk_chain_length(
    head: &str,
    chain_prev_next: &HashMap<String, (Option<String>, Option<String>)>,
) -> usize {
    let mut length = 0;
    let mut current = head.to_string();
    let mut seen = HashSet::new();
    loop {
        if seen.contains(&current) {
            break;
        }
        seen.insert(current.clone());
        length += 1;
        match chain_prev_next
            .get(&current)
            .and_then(|(_, n)| n.as_ref())
        {
            Some(n) => current = n.clone(),
            None => break,
        }
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::LinkGraph;
    use crate::types::{ParsedFile, WikiLink};
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn make_file(name: &str, links: Vec<(&str, u32)>) -> ParsedFile {
        ParsedFile {
            path: PathBuf::from(format!("{name}.md")),
            page_name: name.to_string(),
            links: links
                .into_iter()
                .map(|(target, line)| WikiLink {
                    target_page: target.to_string(),
                    raw_target: target.to_string(),
                    heading: None,
                    block_ref: None,
                    alias: None,
                    is_embed: false,
                    line,
                    column: 1,
                })
                .collect(),
            spl_blocks: vec![],
            diagnostics: vec![],
            mtime: SystemTime::now(),
            merkle_leaves: vec![],
            file_merkle: None,
        }
    }

    fn make_vault_data(files: Vec<ParsedFile>) -> VaultData {
        let mut resolved_pages: HashMap<String, String> = HashMap::new();
        for f in &files {
            for link in &f.links {
                resolved_pages.insert(link.raw_target.clone(), link.target_page.clone());
            }
        }
        let graph = LinkGraph::build(&files, &resolved_pages);
        let resolved = graph.resolved.clone();
        let mut page_names: Vec<String> = files.iter().map(|f| f.page_name.clone()).collect();
        page_names.sort();
        let page_slug_map: HashMap<String, String> = files
            .iter()
            .map(|f| {
                let slug = f.path.to_string_lossy().trim_end_matches(".md").to_string();
                (f.page_name.clone(), slug)
            })
            .collect();
        VaultData {
            files,
            graph,
            page_names,
            resolved,
            page_slug_map,
            collision_names: HashSet::new(),
            chain_prev_next: HashMap::new(),
        }
    }

    #[test]
    fn test_build_vault_context() {
        let files = vec![
            make_file("Alpha", vec![("Beta", 1)]),
            make_file("Beta", vec![]),
        ];
        let data = make_vault_data(files);
        let ctx = build_vault_context(&data, "my-vault");

        assert_eq!(ctx.name, "my-vault");
        assert_eq!(ctx.pages.len(), 2);
        assert_eq!(ctx.stats.total_pages, 2);
        assert_eq!(ctx.stats.total_links, 1);
        assert_eq!(ctx.stats.dead_links, 0);
        assert_eq!(ctx.stats.orphans, 1); // Alpha has no backlinks
    }

    #[test]
    fn test_build_zetl_meta() {
        let meta = build_zetl_meta();
        assert!(!meta.version.is_empty());
    }

    #[test]
    fn test_build_breadcrumbs_nested() {
        let crumbs = build_breadcrumbs("architecture/decisions/Scanner");
        assert_eq!(crumbs.len(), 2);
        assert_eq!(crumbs[0].title, "architecture");
        assert_eq!(crumbs[0].slug, "architecture");
        assert_eq!(crumbs[1].title, "decisions");
        assert_eq!(crumbs[1].slug, "architecture/decisions");
    }

    #[test]
    fn test_build_breadcrumbs_root() {
        let crumbs = build_breadcrumbs("Scanner");
        assert!(crumbs.is_empty());
    }

    #[test]
    fn test_build_page_context() {
        let files = vec![
            make_file("Alpha", vec![("Beta", 1), ("Ghost", 5)]),
            make_file("Beta", vec![("Alpha", 2)]),
        ];
        let data = make_vault_data(files);
        let ctx = build_page_context(&data, "Alpha", "Alpha", "<p>hello</p>", "hello");

        assert_eq!(ctx.title, "Alpha");
        assert_eq!(ctx.slug, "Alpha");
        assert_eq!(ctx.content_html, "<p>hello</p>");
        assert_eq!(ctx.content_raw, "hello");
        assert_eq!(ctx.frontmatter, serde_json::json!({}));
        assert_eq!(ctx.backlinks.len(), 1);
        assert_eq!(ctx.backlinks[0].title, "Beta");
        assert_eq!(ctx.outlinks.len(), 2);
        assert!(!ctx.is_new);
        assert!(ctx.raw_escaped.is_none());
        assert!(ctx.transclusion_cards.is_empty());
    }

    #[test]
    fn test_outlink_dead_detection() {
        let files = vec![make_file("A", vec![("Ghost", 1)])];
        let data = make_vault_data(files);
        let ctx = build_page_context(&data, "A", "A", "", "");

        assert_eq!(ctx.outlinks.len(), 1);
        assert!(ctx.outlinks[0].is_dead);
        assert_eq!(ctx.outlinks[0].color, "red");
    }

    #[test]
    fn test_build_folder_context() {
        let mut files = vec![make_file("Alpha", vec![]), make_file("Beta", vec![])];
        // Simulate nested structure: put Beta in a subfolder
        files[1].path = PathBuf::from("sub/Beta.md");

        let mut data = make_vault_data(files);
        data.page_slug_map
            .insert("Alpha".to_string(), "Alpha".to_string());
        data.page_slug_map
            .insert("Beta".to_string(), "sub/Beta".to_string());

        let ctx = build_folder_context(&data, "", "root");

        assert_eq!(ctx.name, "root");
        assert_eq!(ctx.pages.len(), 1);
        assert_eq!(ctx.pages[0].title, "Alpha");
        assert_eq!(ctx.subfolders.len(), 1);
        assert_eq!(ctx.subfolders[0].name, "sub");
        assert_eq!(ctx.subfolders[0].page_count, 1);
    }

    #[test]
    fn test_page_context_serializes() {
        let files = vec![make_file("A", vec![])];
        let data = make_vault_data(files);
        let ctx = build_page_context(&data, "A", "A", "<p>hi</p>", "hi");
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["title"], "A");
        assert_eq!(json["frontmatter"], serde_json::json!({}));
    }

    #[test]
    fn test_page_context_parses_frontmatter() {
        let files = vec![make_file("A", vec![])];
        let data = make_vault_data(files);
        let raw = "---\ntitle: My Page\ntags:\n  - rust\n  - zetl\n---\n# Hello";
        let ctx = build_page_context(&data, "A", "A", "<h1>Hello</h1>", raw);
        assert_eq!(ctx.frontmatter["title"], "My Page");
        assert_eq!(ctx.frontmatter["tags"][0], "rust");
        assert_eq!(ctx.frontmatter["tags"][1], "zetl");
    }

    #[test]
    fn test_page_context_no_frontmatter() {
        let files = vec![make_file("A", vec![])];
        let data = make_vault_data(files);
        let raw = "# Just a heading\nSome content.";
        let ctx = build_page_context(&data, "A", "A", "<h1>Just a heading</h1>", raw);
        assert_eq!(ctx.frontmatter, serde_json::json!({}));
    }

    #[test]
    fn test_chain_fields_none_without_frontmatter() {
        let files = vec![make_file("A", vec![])];
        let data = make_vault_data(files);
        let ctx = build_page_context(&data, "A", "A", "", "");
        assert!(ctx.prev_page.is_none());
        assert!(ctx.next_page.is_none());
        assert!(ctx.chain_position.is_none());
        assert!(ctx.chain_length.is_none());
        assert!(ctx.chain_head_slug.is_none());
    }

    #[test]
    fn test_chain_prev_next_resolved() {
        let files = vec![
            make_file("A", vec![]),
            make_file("B", vec![]),
            make_file("C", vec![]),
        ];
        let data = make_vault_data(files);
        let raw = "---\nprev: A\nnext: C\n---";
        let ctx = build_page_context(&data, "B", "B", "", raw);
        let prev = ctx.prev_page.unwrap();
        assert_eq!(prev.title, "A");
        assert_eq!(prev.slug, "A");
        let next = ctx.next_page.unwrap();
        assert_eq!(next.title, "C");
        assert_eq!(next.slug, "C");
    }

    #[test]
    fn test_chain_unknown_target_is_none() {
        let files = vec![make_file("A", vec![])];
        let data = make_vault_data(files);
        let raw = "---\nprev: Ghost\n---";
        let ctx = build_page_context(&data, "A", "A", "", raw);
        assert!(ctx.prev_page.is_none());
    }

    #[test]
    fn test_chain_wikilink_syntax_in_frontmatter() {
        let files = vec![make_file("A", vec![]), make_file("B", vec![])];
        let data = make_vault_data(files);
        let raw = "---\nnext: \"[[B]]\"\n---";
        let ctx = build_page_context(&data, "A", "A", "", raw);
        let next = ctx.next_page.unwrap();
        assert_eq!(next.title, "B");
    }
}
