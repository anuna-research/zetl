use serde::Serialize;
use std::collections::HashMap;

use crate::scanner::page_slug_from_path;
use crate::web::markdown::{extract_description, parse_frontmatter};
use crate::web::VaultData;

// ── Shared structs ──────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct SidebarNode {
    pub name: String,
    pub is_folder: bool,
    pub slug: String,
    pub extension: String,
    pub children: Vec<SidebarNode>,
    /// Whether this page is denied but visible (grayed/locked). REQ-020-031.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_style: Option<String>,
}

#[derive(Serialize)]
pub struct VaultContext {
    pub name: String,
    pub pages: Vec<PageEntry>,
    pub sidebar_tree: Vec<SidebarNode>,
    pub stats: StatsContext,
    /// Vault snapshot history summary available as `vault.history` in templates.
    /// `null` (JSON) when history is unavailable.
    pub history: serde_json::Value,
    /// Whether the semantic vector index is available (REQ-100).
    /// Always `false` in build mode; `true` in serve mode when the index exists.
    pub semantic_available: bool,
    /// Canonical site URL (no trailing slash) for emitting absolute URLs in
    /// meta tags (og:image, twitter:image, canonical). Empty string when
    /// unset — templates treat it as "use root-relative URLs".
    #[serde(default)]
    pub site_url: String,
}

#[derive(Serialize)]
pub struct PageEntry {
    pub title: String,
    pub slug: String,
    pub outlink_count: usize,
    pub backlink_count: usize,
    /// File extension without the dot (e.g. "md", "fountain", "spl").
    pub extension: String,
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
pub struct BacklinkEntry {
    pub title: String,
    pub slug: String,
    pub line: usize,
    /// RFC 3339 timestamp of the earliest snapshot where this backlink existed.
    /// `null` (JSON) when history is unavailable.
    pub since: Option<String>,
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
    /// Plain-text description for `<meta name="description">` / social previews.
    /// Derived from frontmatter `description`, falling back to the first paragraph.
    pub description: String,
    pub backlinks: Vec<BacklinkEntry>,
    pub outlinks: Vec<OutlinkEntry>,
    pub breadcrumbs: Vec<BreadcrumbEntry>,
    pub transclusion_cards: String,
    pub is_new: bool,
    pub raw_escaped: Option<String>,
    /// Page snapshot history summary available as `page.history` in templates.
    /// `null` (JSON) when history is unavailable.
    pub history: serde_json::Value,
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

/// Build a sidebar tree from a flat list of pages, grouping by slug path components.
///
/// Each level is sorted folders-first (alphabetically), then pages (alphabetically).
pub fn build_sidebar_tree(pages: &[PageEntry]) -> Vec<SidebarNode> {
    // Collect all entries as (path_parts, page_entry) tuples
    let mut entries: Vec<(Vec<&str>, &PageEntry)> = pages
        .iter()
        .map(|p| (p.slug.split('/').collect::<Vec<_>>(), p))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    fn build_level(
        entries: &[(Vec<&str>, &PageEntry)],
        depth: usize,
        prefix: &str,
    ) -> Vec<SidebarNode> {
        let mut folders: Vec<SidebarNode> = Vec::new();
        let mut leaves: Vec<SidebarNode> = Vec::new();

        // Group entries by their component at `depth`
        let mut i = 0;
        while i < entries.len() {
            let parts = &entries[i].0;
            if parts.len() == depth + 1 {
                // This is a leaf page at this level
                let page = entries[i].1;
                leaves.push(SidebarNode {
                    name: page.title.clone(),
                    is_folder: false,
                    slug: page.slug.clone(),
                    extension: page.extension.clone(),
                    children: vec![],
                    denied_style: None,
                });
                i += 1;
            } else {
                // This entry is deeper — group all entries sharing this folder component
                let folder_name = parts[depth];
                let mut j = i + 1;
                while j < entries.len()
                    && entries[j].0.len() > depth
                    && entries[j].0[depth] == folder_name
                {
                    j += 1;
                }
                let folder_slug = if prefix.is_empty() {
                    folder_name.to_string()
                } else {
                    format!("{prefix}/{folder_name}")
                };
                let children = build_level(&entries[i..j], depth + 1, &folder_slug);
                folders.push(SidebarNode {
                    name: folder_name.to_string(),
                    is_folder: true,
                    slug: folder_slug,
                    extension: String::new(),
                    children,
                    denied_style: None,
                });
                i = j;
            }
        }

        folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        leaves.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        folders.extend(leaves);
        folders
    }

    build_level(&entries, 0, "")
}

/// Build a `VaultContext` from `VaultData` and a vault name.
pub fn build_vault_context(data: &VaultData, vault_name: &str) -> VaultContext {
    let graph_stats = data.graph.stats(0);

    let mut pages: Vec<PageEntry> = data
        .files
        .iter()
        .map(|file| {
            let slug = page_slug_from_path(&file.path);
            let outlink_count = data.graph.forward_links(&file.page_name).len();
            let backlink_count = data.graph.backlinks(&file.page_name).len();
            let extension = file
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("md")
                .to_string();
            PageEntry {
                title: file.page_name.clone(),
                slug,
                outlink_count,
                backlink_count,
                extension,
            }
        })
        .collect();
    pages.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));

    let sidebar_tree = build_sidebar_tree(&pages);

    let stats = StatsContext {
        total_pages: graph_stats.pages,
        total_links: graph_stats.links,
        dead_links: graph_stats.dead_links,
        orphans: graph_stats.orphans,
    };

    VaultContext {
        name: vault_name.to_string(),
        pages,
        sidebar_tree,
        stats,
        history: serde_json::Value::Null,
        semantic_available: false,
        site_url: String::new(),
    }
}

/// Visibility filtering style for a denied page in the sidebar (REQ-020-031).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarDeniedStyle {
    /// Show page grayed-out (transparent mode).
    GrayedOut,
    /// Show page with lock icon (force-visible override).
    Locked,
    /// Hide page completely (mixed/hidden mode).
    Hidden,
}

/// Filter a `VaultContext` for visibility, removing or marking denied pages (REQ-020-031).
///
/// `denied_pages` maps page slug → style. Pages with `Hidden` style are removed;
/// pages with `GrayedOut` or `Locked` style are kept with `denied_style` set.
pub fn filter_vault_context_for_visibility(
    ctx: &mut VaultContext,
    denied_pages: &HashMap<String, SidebarDeniedStyle>,
) {
    // Filter the pages list
    ctx.pages.retain(|p| match denied_pages.get(&p.slug) {
        Some(SidebarDeniedStyle::Hidden) => false,
        _ => true,
    });

    // Rebuild sidebar tree from filtered pages
    ctx.sidebar_tree = build_sidebar_tree(&ctx.pages);

    // Apply denied_style to sidebar leaves
    fn apply_denied_style(nodes: &mut [SidebarNode], denied: &HashMap<String, SidebarDeniedStyle>) {
        for node in nodes.iter_mut() {
            if node.is_folder {
                apply_denied_style(&mut node.children, denied);
            } else if let Some(style) = denied.get(&node.slug) {
                match style {
                    SidebarDeniedStyle::GrayedOut => {
                        node.denied_style = Some("grayed".to_string());
                    }
                    SidebarDeniedStyle::Locked => {
                        node.denied_style = Some("locked".to_string());
                    }
                    SidebarDeniedStyle::Hidden => {} // already filtered out
                }
            }
        }
    }
    apply_denied_style(&mut ctx.sidebar_tree, denied_pages);
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
                since: None,
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
    let description = extract_description(content_raw, &frontmatter);

    PageContext {
        title: page_name.to_string(),
        slug: slug.to_string(),
        content_html: content_html.to_string(),
        content_raw: content_raw.to_string(),
        frontmatter,
        description,
        backlinks,
        outlinks,
        breadcrumbs,
        transclusion_cards: String::new(),
        is_new,
        raw_escaped: None,
        history: serde_json::Value::Null,
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
            let extension = data
                .files
                .iter()
                .find(|f| &f.page_name == page_name)
                .and_then(|f| f.path.extension().and_then(|e| e.to_str()))
                .unwrap_or("md")
                .to_string();
            pages.push(PageEntry {
                title: page_name.clone(),
                slug: slug.clone(),
                outlink_count,
                backlink_count,
                extension,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::LinkGraph;
    use crate::types::{ParsedFile, WikiLink};
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
        let page_slug_map_lower: HashMap<String, String> = page_slug_map
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .collect();
        VaultData {
            files,
            graph,
            page_names,
            resolved,
            page_slug_map,
            page_slug_map_lower,
            collision_names: std::collections::HashSet::new(),
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
        assert_eq!(ctx.sidebar_tree.len(), 2); // two root-level pages
        assert!(!ctx.sidebar_tree[0].is_folder);
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

    // ── Sidebar tree tests ──────────────────────────────────────────────

    fn make_page_entry(title: &str, slug: &str) -> PageEntry {
        PageEntry {
            title: title.to_string(),
            slug: slug.to_string(),
            outlink_count: 0,
            backlink_count: 0,
            extension: "md".to_string(),
        }
    }

    #[test]
    fn test_build_sidebar_tree_flat() {
        let pages = vec![
            make_page_entry("Cache", "Cache"),
            make_page_entry("Scanner", "Scanner"),
            make_page_entry("Alpha", "Alpha"),
        ];
        let tree = build_sidebar_tree(&pages);
        // All root pages, sorted alphabetically
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].name, "Alpha");
        assert_eq!(tree[1].name, "Cache");
        assert_eq!(tree[2].name, "Scanner");
        assert!(!tree[0].is_folder);
        assert!(tree[0].children.is_empty());
    }

    #[test]
    fn test_build_sidebar_tree_nested() {
        let pages = vec![
            make_page_entry("Cache", "Cache"),
            make_page_entry("Scanner", "architecture/Scanner"),
            make_page_entry("Overview", "architecture/Overview"),
            make_page_entry("Wikilinks", "concepts/Wikilinks"),
        ];
        let tree = build_sidebar_tree(&pages);
        // Folders first (alpha), then pages (alpha)
        assert_eq!(tree.len(), 3); // architecture/, concepts/, Cache
        assert!(tree[0].is_folder);
        assert_eq!(tree[0].name, "architecture");
        assert_eq!(tree[0].slug, "architecture");
        assert_eq!(tree[0].children.len(), 2);
        // Children sorted: Overview, Scanner
        assert_eq!(tree[0].children[0].name, "Overview");
        assert_eq!(tree[0].children[1].name, "Scanner");

        assert!(tree[1].is_folder);
        assert_eq!(tree[1].name, "concepts");
        assert_eq!(tree[1].children.len(), 1);
        assert_eq!(tree[1].children[0].name, "Wikilinks");

        assert!(!tree[2].is_folder);
        assert_eq!(tree[2].name, "Cache");
    }

    #[test]
    fn test_build_sidebar_tree_deeply_nested() {
        let pages = vec![make_page_entry("Leaf", "a/b/c/Leaf")];
        let tree = build_sidebar_tree(&pages);
        // a/ -> b/ -> c/ -> Leaf
        assert_eq!(tree.len(), 1);
        assert!(tree[0].is_folder);
        assert_eq!(tree[0].name, "a");
        assert_eq!(tree[0].slug, "a");

        let b = &tree[0].children;
        assert_eq!(b.len(), 1);
        assert!(b[0].is_folder);
        assert_eq!(b[0].name, "b");
        assert_eq!(b[0].slug, "a/b");

        let c = &b[0].children;
        assert_eq!(c.len(), 1);
        assert!(c[0].is_folder);
        assert_eq!(c[0].name, "c");
        assert_eq!(c[0].slug, "a/b/c");

        let leaf = &c[0].children;
        assert_eq!(leaf.len(), 1);
        assert!(!leaf[0].is_folder);
        assert_eq!(leaf[0].name, "Leaf");
        assert_eq!(leaf[0].slug, "a/b/c/Leaf");
    }

    // ── Visibility filtering tests (TEST-020-031) ────────────────────

    #[test]
    fn filter_vault_context_hides_denied_pages() {
        let files = vec![
            make_file("Alpha", vec![]),
            make_file("Secret", vec![]),
            make_file("Beta", vec![]),
        ];
        let data = make_vault_data(files);
        let mut ctx = build_vault_context(&data, "vault");
        assert_eq!(ctx.pages.len(), 3);

        // Slugs are lowercased by page_slug_from_path
        let mut denied = HashMap::new();
        denied.insert("secret".to_string(), SidebarDeniedStyle::Hidden);
        filter_vault_context_for_visibility(&mut ctx, &denied);

        assert_eq!(ctx.pages.len(), 2);
        assert!(ctx.pages.iter().all(|p| p.title != "Secret"));
        // Sidebar tree should also not contain Secret
        assert!(ctx.sidebar_tree.iter().all(|n| n.name != "Secret"));
    }

    #[test]
    fn filter_vault_context_grays_out_denied_pages() {
        let files = vec![make_file("Alpha", vec![]), make_file("Secret", vec![])];
        let data = make_vault_data(files);
        let mut ctx = build_vault_context(&data, "vault");

        let mut denied = HashMap::new();
        denied.insert("secret".to_string(), SidebarDeniedStyle::GrayedOut);
        filter_vault_context_for_visibility(&mut ctx, &denied);

        // Page should still be in the list (not hidden, just grayed)
        assert_eq!(ctx.pages.len(), 2);
        // But sidebar node should have denied_style set
        let secret_node = ctx.sidebar_tree.iter().find(|n| n.name == "Secret");
        assert!(secret_node.is_some());
        assert_eq!(
            secret_node.unwrap().denied_style,
            Some("grayed".to_string())
        );
    }

    #[test]
    fn filter_vault_context_locks_denied_pages() {
        let files = vec![make_file("Alpha", vec![]), make_file("Roadmap", vec![])];
        let data = make_vault_data(files);
        let mut ctx = build_vault_context(&data, "vault");

        let mut denied = HashMap::new();
        denied.insert("roadmap".to_string(), SidebarDeniedStyle::Locked);
        filter_vault_context_for_visibility(&mut ctx, &denied);

        assert_eq!(ctx.pages.len(), 2);
        let roadmap_node = ctx.sidebar_tree.iter().find(|n| n.name == "Roadmap");
        assert!(roadmap_node.is_some());
        assert_eq!(
            roadmap_node.unwrap().denied_style,
            Some("locked".to_string())
        );
    }
}
