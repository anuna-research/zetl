pub mod build;
pub mod context;
pub mod engine;
pub mod git_commit;
pub mod html;
pub mod markdown;
pub mod routes;
pub mod rate_limit;
pub mod session;
pub mod theme;
pub mod ws;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

#[cfg(feature = "reason")]
use crate::acl::{AclDecision, Action};

use axum::http::header;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::graph::LinkGraph;
use crate::scanner::{page_slug_from_path, resolve_page_name, scan_vault};
use crate::search_index::SearchIndex;
use crate::types::ParsedFile;

use self::engine::TemplateEngine;
use self::rate_limit::AuthRateLimiters;
use self::session::SessionStore;
use crate::user::recovery::RecoveryChallengeStore;

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

impl VaultData {
    /// Look up the slug for a page name (case-insensitive).
    pub fn slug_for_page(&self, page_name: &str) -> String {
        self.page_slug_map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(page_name))
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| page_name.to_string())
    }
}

/// Lazy ACL decision cache keyed by `(user_id, page_slug, action)` (REQ-020-013).
///
/// The cache is invalidated (cleared) whenever the vault's `vault_root_hash`
/// changes — i.e. on any file save or re-index — because policy files or
/// page-level SPL blocks may have been modified.
#[cfg(feature = "reason")]
#[derive(Debug, Default)]
pub struct AclCache {
    /// The vault_root_hash that was current when entries were inserted.
    vault_root_hash: Option<String>,
    /// Cached decisions: `(user_id, page_slug, action) → AclDecision`.
    entries: HashMap<(String, String, Action), AclDecision>,
}

#[cfg(feature = "reason")]
impl AclCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached decision.  Returns `None` on cache miss.
    pub fn lookup(
        &self,
        user_id: &str,
        page_slug: &str,
        action: Action,
    ) -> Option<&AclDecision> {
        self.entries.get(&(
            user_id.to_owned(),
            page_slug.to_owned(),
            action,
        ))
    }

    /// Insert a decision into the cache.
    pub fn insert(
        &mut self,
        user_id: String,
        page_slug: String,
        action: Action,
        decision: AclDecision,
    ) {
        self.entries.insert((user_id, page_slug, action), decision);
    }

    /// If the `vault_root_hash` has changed, clear all cached decisions and
    /// store the new hash.  Returns `true` when the cache was invalidated.
    pub fn invalidate_if_changed(&mut self, current_hash: Option<&str>) -> bool {
        if self.vault_root_hash.as_deref() != current_hash {
            self.entries.clear();
            self.vault_root_hash = current_hash.map(|s| s.to_owned());
            true
        } else {
            false
        }
    }

    /// Unconditionally clear every cached entry (e.g. after a save).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.vault_root_hash = None;
    }

    /// Number of cached entries (useful for diagnostics / testing).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Shared state passed to all handlers via axum State.
///
/// `search_index` is thread-safe and shared across requests via Arc.
/// REQ-013-012.
#[derive(Clone)]
pub struct WebState {
    pub data: Arc<RwLock<VaultData>>,
    pub vault_root: Arc<PathBuf>,
    pub search_index: Arc<SearchIndex>,
    pub engine: Arc<TemplateEngine>,
    pub theme: String,
    /// Whether --verbose was set; controls history-context timing output (OBS-013).
    pub verbose: bool,
    /// Whether --collab mode is active (multi-user authentication required).
    pub collab: bool,
    /// In-memory session store for authenticated users.
    pub sessions: SessionStore,
    /// In-memory recovery challenge store (CON-020-002).
    pub recovery_challenges: Arc<RecoveryChallengeStore>,
    /// Tracks user_ids whose mnemonic has already been displayed (one-time serve).
    pub mnemonic_shown: Arc<Mutex<HashSet<String>>>,
    /// Authentication rate limiters (per-user and per-IP).
    pub rate_limiters: AuthRateLimiters,
    /// Lazy ACL decision cache, invalidated on vault_root_hash change (REQ-020-013).
    #[cfg(feature = "reason")]
    pub acl_cache: Arc<Mutex<AclCache>>,
    /// Git repository lock for serializing auto-commits on save (REQ-020-015, CON-020-006).
    /// `None` when the vault is not inside a git repository.
    pub git_commit_lock: Option<Arc<git_commit::GitCommitLock>>,
    /// WebSocket editing hub — manages per-slug broadcast rooms (REQ-020-028).
    pub ws_hub: ws::WsHub,
    /// One-time ticket store for WebSocket auth (agents can't send cookies).
    pub ticket_store: ws::TicketStore,
    /// Pre-loaded vector index for semantic/hybrid search in serve mode (REQ-100).
    /// `None` when the semantic feature is inactive or the index has not been built.
    #[cfg(feature = "semantic")]
    pub vector_index: Option<Arc<std::sync::Mutex<crate::semantic::VectorIndex>>>,
}

/// Re-scan the vault and return a fresh `VaultData` snapshot.
pub fn reindex(vault_root: &Path) -> anyhow::Result<VaultData> {
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
            eprintln!("warning: slug collision — the following files all map to /{slug}:");
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

pub async fn run(state: WebState, port: u16, bind_addr: &str) -> anyhow::Result<()> {
    // ── Auth routes (always public, even in --collab mode) ───────────
    let auth_routes = Router::new()
        .route("/auth/bootstrap", get(routes::bootstrap_handler))
        .route(
            "/auth/recover",
            get(routes::recover_challenge_handler).post(routes::recover_verify_handler),
        )
        .route("/recovery/show", get(routes::recovery_show_handler))
        .route("/passkey/register", get(routes::passkey_register_handler))
        .route(
            "/api/passkey/register/start",
            post(routes::passkey_register_start_handler),
        )
        .route(
            "/api/passkey/register/finish",
            post(routes::passkey_register_finish_handler),
        )
        .route(
            "/auth/accept",
            get(routes::accept_invite_handler).post(routes::accept_invite_submit_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit::auth_ip_rate_limit,
        ));

    // ── Admin routes (hardcoded owner/admin gate — not defeatable by SPL) ──
    let admin_routes = Router::new()
        .route(
            "/_admin/invite",
            get(routes::admin_invite_handler).post(routes::admin_invite_create_handler),
        )
        .route(
            "/_admin/invite/revoke",
            post(routes::admin_invite_revoke_handler),
        )
        .route(
            "/_admin/permissions",
            get(routes::admin_permissions_handler).post(routes::admin_permissions_save_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session::admin_gate,
        ));

    // ── Content routes (gated by collab_gate when --collab is active) ─
    let content_routes = Router::new()
        .route("/", get(routes::index_handler))
        .route("/api/search", get(routes::api_search_handler))
        // Agent API (CON-020-007, REQ-020-017, REQ-020-018)
        .route("/api/pages", get(routes::api_pages_list_handler))
        .route(
            "/api/pages/{*slug}",
            get(routes::api_pages_get_handler)
                .put(routes::api_pages_put_handler)
                .delete(routes::api_pages_delete_handler),
        )
        .route("/api/graph", get(routes::api_graph_handler))
        .route("/api/index", post(routes::api_index_handler))
        .route("/_me", get(routes::dashboard_handler))
        .route("/api/access-request", post(routes::access_request_handler))
        .route("/_print", get(routes::print_handler))
        .route("/_static/{*path}", get(routes::static_handler))
        .merge(admin_routes)
        .route("/preview/{*path}", get(routes::preview_handler))
        .route(
            "/{*path}",
            get(routes::page_handler).put(routes::save_handler),
        );

    // Reason API — only available with `--features reason` (REQ-020-018).
    #[cfg(feature = "reason")]
    let content_routes = content_routes
        .route("/api/reason", post(routes::api_reason_handler))
        .route("/api/acl/explain", get(routes::api_acl_explain_handler));

    // History API routes — only available with `--features history` (REQ-087, CON-027, ADR-050).
    #[cfg(feature = "history")]
    let content_routes = content_routes
        .route("/api/history", get(routes::api_history_log_handler))
        .route(
            "/api/history/page/{name}",
            get(routes::api_history_page_handler),
        )
        .route("/api/history/at", get(routes::api_history_at_handler))
        .route("/api/history/diff", get(routes::api_history_diff_handler));

    let content_routes = content_routes
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session::csrf_guard,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session::csrf_token_header,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session::collab_gate,
        ));

    // ── WebSocket routes (auth handled inside the handler) ───────────
    let ws_routes = Router::new()
        .route("/ws/edit/{*slug}", get(ws::ws_edit_handler))
        .route("/api/ws/ticket", post(routes::ws_ticket_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            session::collab_gate,
        ));

    let app = Router::new()
        .merge(auth_routes)
        .merge(ws_routes)
        .merge(content_routes)
        .with_state(state)
        .layer(middleware::map_response(|mut resp: axum::response::Response| async {
            resp.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                "script-src 'self'; frame-ancestors 'none'; connect-src 'self' ws: wss:"
                    .parse()
                    .unwrap(),
            );
            resp
        }));

    let addr = format!("{bind_addr}:{port}");
    eprintln!("zetl serve  →  http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(all(test, feature = "reason"))]
mod tests {
    use super::*;
    use crate::acl::{AclDecision, Action, ConclusionTag};

    #[test]
    fn acl_cache_lookup_miss() {
        let cache = AclCache::new();
        assert!(cache.lookup("alice", "readme", Action::Read).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn acl_cache_insert_and_hit() {
        let mut cache = AclCache::new();
        let decision = AclDecision::Allowed {
            tag: ConclusionTag::DefeasiblyProvable,
            rule_trace: vec![],
        };
        cache.insert("alice".into(), "readme".into(), Action::Read, decision);
        assert_eq!(cache.len(), 1);

        let hit = cache.lookup("alice", "readme", Action::Read);
        assert!(hit.is_some());
        assert!(hit.unwrap().is_allowed());

        // Different action → miss
        assert!(cache.lookup("alice", "readme", Action::Edit).is_none());
        // Different user → miss
        assert!(cache.lookup("bob", "readme", Action::Read).is_none());
    }

    #[test]
    fn acl_cache_invalidate_on_hash_change() {
        let mut cache = AclCache::new();
        cache.vault_root_hash = Some("aaa".into());
        cache.insert(
            "alice".into(),
            "readme".into(),
            Action::Read,
            AclDecision::Allowed {
                tag: ConclusionTag::DefeasiblyProvable,
                rule_trace: vec![],
            },
        );
        assert_eq!(cache.len(), 1);

        // Same hash → no invalidation
        assert!(!cache.invalidate_if_changed(Some("aaa")));
        assert_eq!(cache.len(), 1);

        // Different hash → invalidated
        assert!(cache.invalidate_if_changed(Some("bbb")));
        assert!(cache.is_empty());
    }

    #[test]
    fn acl_cache_clear() {
        let mut cache = AclCache::new();
        cache.vault_root_hash = Some("hash".into());
        cache.insert(
            "alice".into(),
            "readme".into(),
            Action::Read,
            AclDecision::Denied {
                tag: ConclusionTag::DefeasiblyNotProvable,
                rule_trace: vec![],
            },
        );
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.vault_root_hash.is_none());
    }
}
