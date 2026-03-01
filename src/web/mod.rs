pub mod build;
pub mod context;
pub mod engine;
pub mod html;
pub mod markdown;
pub mod routes;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::Router;
use axum::routing::get;

use crate::graph::LinkGraph;
use crate::scanner::{page_slug_from_path, resolve_page_name, scan_vault};
use crate::types::ParsedFile;

/// Snapshot of vault data that can be swapped after re-indexing.
pub struct VaultData {
    pub files: Vec<ParsedFile>,
    pub graph: LinkGraph,
    pub page_names: Vec<String>,
    pub resolved: HashSet<String>,
    /// Maps page_name → page_slug (relative path without extension, e.g. "architecture/Scanner")
    pub page_slug_map: HashMap<String, String>,
    /// Page names that appear in more than one folder (need disambiguation in display)
    pub collision_names: HashSet<String>,
}

/// Shared state passed to all handlers via axum State.
#[derive(Clone)]
pub struct WebState {
    pub data: Arc<RwLock<VaultData>>,
    pub vault_root: Arc<PathBuf>,
}

/// Re-scan the vault and return a fresh `VaultData` snapshot.
pub fn reindex(vault_root: &PathBuf) -> anyhow::Result<VaultData> {
    let files = scan_vault(vault_root, &[])?;

    let file_index: Vec<(String, PathBuf)> = files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();

    let mut resolved_pages: HashMap<String, String> = HashMap::new();
    for file in &files {
        for link in &file.links {
            let key = link.raw_target.clone();
            if resolved_pages.contains_key(&key) {
                continue;
            }
            if let Some(resolved) = resolve_page_name(&link.target_page, &file_index) {
                resolved_pages.insert(key, resolved);
            }
        }
    }

    let graph = LinkGraph::build(&files, &resolved_pages);
    let graph_resolved = graph.resolved.clone();

    let mut page_names: Vec<String> = files.iter().map(|f| f.page_name.clone()).collect();
    page_names.sort_by_key(|a| a.to_lowercase());

    // Build page_slug_map: page_name → slug (kebab-case relative path)
    let (page_slug_map, collision_names) = build_slug_map(&files);

    Ok(VaultData {
        files,
        graph,
        page_names,
        resolved: graph_resolved,
        page_slug_map,
        collision_names,
    })
}

/// Build the page_slug_map and collision_names from a list of parsed files.
///
/// Warns to stderr if two different pages produce the same kebab-case slug
/// (e.g. `Foo Bar.md` and `foo-bar.md` in the same folder).
pub fn build_slug_map(files: &[ParsedFile]) -> (HashMap<String, String>, HashSet<String>) {
    let mut page_slug_map: HashMap<String, String> = HashMap::new();
    // Track slug → list of original paths for collision detection
    let mut slug_sources: HashMap<String, Vec<String>> = HashMap::new();

    for file in files {
        let slug = page_slug_from_path(&file.path);
        slug_sources
            .entry(slug.clone())
            .or_default()
            .push(file.path.to_string_lossy().to_string());
        page_slug_map.insert(file.page_name.clone(), slug);
    }

    // Warn about slug collisions
    for (slug, sources) in &slug_sources {
        if sources.len() > 1 {
            eprintln!(
                "warning: slug collision — the following files all map to /{slug}:"
            );
            for src in sources {
                eprintln!("  - {src}");
            }
        }
    }

    // Detect page-name collisions (same filename stem in multiple folders)
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for file in files {
        *name_counts.entry(file.page_name.clone()).or_insert(0) += 1;
    }
    let collision_names: HashSet<String> = name_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect();

    (page_slug_map, collision_names)
}

pub async fn run(state: WebState, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(routes::index_handler))
        .route("/preview/{*path}", get(routes::preview_handler))
        .route("/{*path}", get(routes::page_handler).put(routes::save_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    eprintln!("zetl serve  →  http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
