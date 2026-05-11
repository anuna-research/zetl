pub mod build;
pub mod context;
pub mod engine;
pub mod flush;
pub mod fs_watch;
pub mod git_commit;
pub mod git_poll;
pub mod html;
pub mod markdown;
#[cfg(feature = "mobile")]
pub mod mobile;
pub mod og;
pub mod rate_limit;
pub mod robots;
pub mod routes;
pub mod session;
pub mod theme;
pub mod wal;
pub mod ws;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

#[cfg(feature = "reason")]
use crate::acl::{AclDecision, Action};

use axum::extract::Request;
use axum::http::header;
use axum::http::StatusCode;
use axum::middleware;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;

use crate::assets::store::StorageCounterGuard;
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
    /// Maps lowercased page_name → slug for O(1) case-insensitive lookup.
    pub page_slug_map_lower: HashMap<String, String>,
    /// Page names that appear in more than one folder (need disambiguation in display)
    pub collision_names: HashSet<String>,
}

impl VaultData {
    /// Look up the slug for a page name (case-insensitive). O(1) via pre-built lowercase map.
    pub fn slug_for_page(&self, page_name: &str) -> String {
        self.page_slug_map_lower
            .get(&page_name.to_ascii_lowercase())
            .cloned()
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
    pub fn lookup(&self, user_id: &str, page_slug: &str, action: Action) -> Option<&AclDecision> {
        self.entries
            .get(&(user_id.to_owned(), page_slug.to_owned(), action))
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
    /// Whether the server is behind a TLS-terminating proxy (enables Secure cookie flag).
    pub tls: bool,
    /// Whether to trust proxy headers (X-Forwarded-For, X-Real-IP) for client IP extraction.
    pub trust_proxy: bool,
    /// In-memory session store for authenticated users.
    pub sessions: SessionStore,
    /// In-memory recovery challenge store (CON-020-002).
    pub recovery_challenges: Arc<RecoveryChallengeStore>,
    /// Tracks user_ids whose mnemonic has already been displayed (one-time serve).
    pub mnemonic_shown: Arc<Mutex<HashSet<String>>>,
    /// One-time flag: /auth/bootstrap has already been consumed this process.
    pub bootstrap_used: Arc<std::sync::atomic::AtomicBool>,
    /// Authentication rate limiters (per-user and per-IP).
    pub rate_limiters: AuthRateLimiters,
    /// Lazy ACL decision cache, invalidated on vault_root_hash change (REQ-020-013).
    #[cfg(feature = "reason")]
    pub acl_cache: Arc<Mutex<AclCache>>,
    /// Git repository lock for serializing auto-commits on save
    /// (REQ-020-015, CON-020-006). Wrapped in an interior-mutable slot
    /// so the SPEC-040 mobile flows can install a repo handle after
    /// `/_mobile/onboarding/clone` succeeds — the embedded serve
    /// launches before any vault exists, so the slot starts empty
    /// for mobile and is populated lazily.
    pub git_commit_lock: git_commit::GitCommitLockSlot,
    /// WebSocket editing hub — manages per-slug broadcast rooms (REQ-020-028).
    pub ws_hub: ws::WsHub,
    /// One-time ticket store for WebSocket auth (agents can't send cookies).
    pub ticket_store: ws::TicketStore,
    /// CRDT document store — manages lifecycle: load, eviction, flush (REQ-020-029).
    pub crdt_store: ws::CrdtDocStore,
    /// CRDT write-ahead log for crash recovery (REQ-020-044).
    pub wal_store: Arc<wal::WalStore>,
    /// Shared PasskeyManager for WebAuthn registration/authentication ceremonies.
    /// Must persist across /start and /finish requests to retain challenge state.
    pub passkey_mgr: Option<Arc<crate::user::passkey::PasskeyManager>>,
    /// Tracks files zetl is currently writing, for external edit detection (REQ-020-039).
    pub pending_writes: fs_watch::PendingWrites,
    /// Public directory whose files override generated pages in serve mode.
    pub public_dir: Option<PathBuf>,
    /// Vault-scan exclusion options propagated from CLI flags (SPEC-026).
    /// Reused on every reindex / on-demand scan_vault call so that flags
    /// like `--exclude` take effect for the whole serve lifetime.
    pub scan_options: crate::scanner::ScanOptions,
    /// Pre-loaded vector index for semantic/hybrid search in serve mode (REQ-100).
    /// `None` when the semantic feature is inactive or the index has not been built.
    #[cfg(feature = "semantic")]
    pub vector_index: Option<Arc<std::sync::Mutex<crate::semantic::VectorIndex>>>,
    /// Asset storage counter (REQ-3519).
    pub asset_storage: StorageCounterGuard,
    /// Per-file upload limit in bytes (REQ-3508).
    pub asset_max_file_bytes: u64,
    /// Total asset storage limit in bytes (REQ-3509).
    pub asset_max_total_bytes: u64,
}

/// Re-scan the vault and return a fresh `VaultData` snapshot.
///
/// Convenience wrapper for callers that don't need CLI-driven exclusion
/// options (currently only test fixtures and the legacy entry point).
/// Production code paths use [`reindex_with`] with the active
/// `WebState.scan_options`.
pub fn reindex(vault_root: &Path) -> anyhow::Result<VaultData> {
    // SCAN-OPTS: intentional — legacy wrapper retained for tests.
    reindex_with(vault_root, &crate::scanner::ScanOptions::default())
}

/// Re-scan with explicit options. Prefer this from serve handlers so CLI
/// `--exclude` / `--include-hidden` flags propagate.
pub fn reindex_with(
    vault_root: &Path,
    opts: &crate::scanner::ScanOptions,
) -> anyhow::Result<VaultData> {
    let files = scan_vault(vault_root, opts)?;

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

    // Build lowercased lookup map for O(1) case-insensitive slug_for_page.
    let page_slug_map_lower: HashMap<String, String> = page_slug_map
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .collect();

    Ok(VaultData {
        files,
        graph,
        page_names,
        resolved: graph_resolved,
        page_slug_map,
        page_slug_map_lower,
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

pub async fn run(
    state: WebState,
    port: u16,
    bind_addr: &str,
    git_poll_interval: std::time::Duration,
) -> anyhow::Result<()> {
    // ── WAL replay on startup (REQ-020-044) ─────────────────────────────
    // If the server crashed with dirty CRDT state, the WAL contains the
    // operations that were not yet flushed. Replay them now.
    wal::replay_pending_wals(&state);

    // Spawn the CRDT flush lifecycle task (REQ-020-034).
    // Runs the unified 10-step pipeline on quiescence and handles TTL evictions.
    let _flush_handle = flush::spawn_flush_lifecycle_task(state.clone());

    // Spawn filesystem watcher for external edit detection (REQ-020-039).
    let _fs_watch_handle = fs_watch::spawn_fs_watcher(state.clone());

    // Spawn git HEAD poller for external commit detection (REQ-020-041).
    let _git_poll_handle = state
        .git_commit_lock
        .current()
        .and_then(|lock| git_poll::spawn_git_poller(state.clone(), git_poll_interval, lock));

    // Spawn comment auto-prune task (REQ-020-051).
    // Runs once per hour, removes comments older than 30 days.
    let _prune_handle = spawn_comment_prune_task(state.vault_root.clone());

    // Spawn session purge task: remove expired sessions every 5 minutes.
    let _session_purge_handle = {
        let sessions = state.sessions.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                sessions.purge_expired();
            }
        })
    };

    let asset_max_file_bytes = state.asset_max_file_bytes;

    // ── Auth routes (always public, even in --collab mode) ───────────
    let auth_routes = Router::new()
        .route("/auth/login", get(routes::login_handler))
        .route(
            "/api/passkey/auth/start",
            post(routes::passkey_auth_start_handler),
        )
        .route(
            "/api/passkey/auth/finish",
            post(routes::passkey_auth_finish_handler),
        )
        .route("/auth/bootstrap", get(routes::bootstrap_handler))
        .route(
            "/auth/recover",
            get(routes::recover_challenge_handler).post(routes::recover_verify_handler),
        )
        .route("/recovery/show", get(routes::recovery_show_handler))
        .route("/recovery/confirm", post(routes::recovery_confirm_handler))
        .route(
            "/auth/recovery",
            get(routes::recovery_page_handler).post(routes::recovery_form_handler),
        )
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
        .route("/help", get(routes::help_handler))
        .route("/help/", get(routes::help_handler))
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
        .route("/graph-index.json", get(routes::graph_index_handler))
        .route("/pages.json", get(routes::pages_json_handler))
        .route("/sitemap.xml", get(routes::sitemap_handler))
        .route("/llms.txt", get(routes::llms_txt_handler))
        // SPEC-038 outbound feeds (REQ-3801): RSS 2.0 + Atom 1.0 + optional JSON Feed v1.1.
        .route("/feed.xml", get(routes::feed_xml_handler))
        .route("/atom.xml", get(routes::atom_xml_handler))
        .route("/feed.json", get(routes::feed_json_handler))
        .route("/_graph", get(routes::vault_graph_handler))
        .route("/api/index", post(routes::api_index_handler))
        .route("/_me", get(routes::dashboard_handler))
        .route("/api/access-request", post(routes::access_request_handler))
        .route(
            "/api/comments/{*slug}",
            get(routes::api_comments_get_handler).post(routes::api_comments_post_handler),
        )
        .route("/_print", get(routes::print_handler))
        .route("/api/history/file-diff", get(routes::api_file_diff_handler))
        .route("/api/history/restore", post(routes::api_restore_handler))
        .route("/edit/{*slug}", get(routes::edit_handler))
        .route("/_static/{*path}", get(routes::static_handler))
        // Asset management API (SPEC-035)
        .route("/api/assets", get(routes::list_assets_handler))
        .route(
            "/api/assets/{*slug}",
            post(routes::upload_asset_handler)
                .layer(axum::extract::DefaultBodyLimit::max(
                    asset_max_file_bytes as usize,
                ))
                .delete(routes::delete_asset_handler),
        )
        .route("/_admin/assets", get(routes::admin_assets_handler))
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

    // ── WebSocket routes (auth handled inside the handler via tickets) ──
    // No collab_gate here: WS auth uses single-use tickets issued by the
    // edit handler (which IS behind collab_gate). The ticket endpoint
    // validates session cookies internally when in collab mode.
    let ws_routes = Router::new()
        .route("/ws/edit/{*slug}", get(ws::ws_edit_handler))
        .route("/api/ws/ticket", post(routes::ws_ticket_handler));

    // ── Public asset routes (not gated by collab_gate — ACL checked inside handler)
    let asset_routes = Router::new().route("/assets/{*path}", get(routes::serve_asset_handler));

    let app = Router::new()
        .merge(auth_routes)
        .merge(ws_routes)
        .merge(asset_routes)
        .merge(content_routes);

    // SPEC-040 REQ-4005 / CON-4004: mobile-specific routes for the Tauri
    // Mobile shell. Not behind collab_gate / csrf_guard — mobile is
    // single-user and the embedded server binds to loopback only.
    #[cfg(feature = "mobile")]
    let app = app.merge(mobile::router());

    let app = app
        .with_state(state)
        .layer(middleware::map_response(|mut resp: axum::response::Response| async {
            // Only set CSP if the handler didn't already set one (e.g. the editor).
            resp.headers_mut().entry(header::CONTENT_SECURITY_POLICY).or_insert(
                "script-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net https://esm.sh https://unpkg.com; \
                 style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net https://esm.sh https://unpkg.com https://fonts.googleapis.com; \
                 connect-src 'self' ws: wss: https://esm.sh https://cdn.jsdelivr.net; \
                 font-src 'self' https://cdn.jsdelivr.net https://unpkg.com https://fonts.gstatic.com; \
                 frame-ancestors 'none'"
                    .parse()
                    .unwrap(),
            );
            resp
        }))
        // ETag + conditional GET for HTML responses (task-html-etag).
        // Runs before compression so the ETag is computed from the raw body
        // and is stable regardless of which encoding the client negotiates.
        .layer(middleware::from_fn(etag_middleware))
        // Response compression (PERF-AUDIT-2026-04-19 / task-compression-layer).
        // Negotiates gzip or brotli based on Accept-Encoding; text responses
        // typically shrink 5–8× with gzip and 7–10× with brotli. Applied
        // *after* the CSP layer so the header still lands on every response.
        .layer(tower_http::compression::CompressionLayer::new().gzip(true).br(true));

    let addr = format!("{bind_addr}:{port}");
    eprintln!("zetl serve  →  http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// SPEC-040 REQ-4004: convenience entry-point that builds a minimal
/// `WebState` for embedded use (the Tauri Mobile shell, plus any
/// other in-process embedder that wants the full serve UI without
/// CLI plumbing) and hands it to [`run`].
///
/// Defaults applied:
///
/// - `collab = false`, `tls = false`, `trust_proxy = false`
/// - `passkey_mgr = None` (single-user; no WebAuthn)
/// - `git_commit_lock = None` (auto-commit disabled; mobile commits via
///   the explicit capture / save paths in subsequent slices)
/// - `public_dir = None`
/// - `scan_options = ScanOptions::default()`
/// - `theme = "default"` (override post-construction if needed)
/// - asset limits at the same defaults the CLI uses unless caller
///   overrides via env vars (left untouched for now)
///
/// Gated to `feature = "mobile"` so non-mobile builds don't pull
/// in the extra public surface unintentionally.
#[cfg(feature = "mobile")]
pub async fn launch_default(
    vault_root: std::path::PathBuf,
    bind_addr: &str,
    port: u16,
) -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::RwLock;

    // Make sure the working tree directory exists. On first run before
    // onboarding clones a remote, it may not yet — the embedded server
    // still needs to start so the user can reach /_mobile/onboarding.
    let _ = std::fs::create_dir_all(&vault_root);

    // Index the vault (may be empty on first run).
    let data = match reindex(&vault_root) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "[zetl-mobile] vault reindex on launch failed: {e:?}; starting with empty data"
            );
            VaultData {
                files: Vec::new(),
                graph: LinkGraph::build(&[], &HashMap::new()),
                page_names: Vec::new(),
                resolved: HashSet::new(),
                page_slug_map: HashMap::new(),
                page_slug_map_lower: HashMap::new(),
                collision_names: HashSet::new(),
            }
        }
    };

    // SearchIndex::build accepts an empty file list and yields a usable
    // (empty) index. Treat any failure here as fatal — without an index
    // we cannot serve search requests.
    let search_index = SearchIndex::build(&vault_root, &data.files)
        .map_err(|e| anyhow::anyhow!("search index build failed: {e:?}"))?;

    let theme = "default";
    let engine = Arc::new(engine::TemplateEngine::new(
        &vault_root,
        theme,
        false,
        false,
    ));

    // SPEC-040 multi-vault: register the template engine so
    // /_mobile/* handlers can render Minijinja templates instead of
    // inline HTML.
    crate::mobile_state::set_template_engine(engine.clone());

    let vault_root_arc = Arc::new(vault_root);

    let data = Arc::new(RwLock::new(data));
    // SPEC-040 multi-vault: register the data handle in mobile_state
    // so /_mobile/vaults/switch can swap the embedded serve's
    // in-memory VaultData when the active-vault symlink moves.
    crate::mobile_state::set_vault_data_handle(data.clone());

    let state = WebState {
        data,
        crdt_store: ws::CrdtDocStore::new(vault_root_arc.clone()),
        vault_root: vault_root_arc.clone(),
        search_index: Arc::new(search_index),
        engine: engine.clone(),
        theme: theme.to_string(),
        verbose: false,
        collab: false,
        tls: false,
        trust_proxy: false,
        sessions: session::SessionStore::new(),
        recovery_challenges: Arc::new(crate::user::recovery::RecoveryChallengeStore::new()),
        mnemonic_shown: Arc::new(Mutex::new(HashSet::new())),
        bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        rate_limiters: rate_limit::AuthRateLimiters::new(),
        #[cfg(feature = "reason")]
        acl_cache: Arc::new(Mutex::new(AclCache::new())),
        // Mobile boot: the vault symlink may have no `.git` yet (the
        // user hasn't onboarded). Start the slot empty; the clone /
        // switch / pick handlers populate it via
        // `mobile_state::refresh_git_commit_lock` once a working tree
        // is in place.
        git_commit_lock: {
            let slot = git_commit::GitCommitLockSlot::empty();
            if let Some(repo) = git_commit::open_repo(&vault_root_arc) {
                slot.set(Some(Arc::new(repo)));
            }
            crate::mobile_state::set_git_commit_lock_slot(slot.clone());
            slot
        },
        ws_hub: ws::WsHub::new(),
        ticket_store: ws::TicketStore::new(),
        wal_store: Arc::new(wal::WalStore::new(&vault_root_arc)),
        pending_writes: fs_watch::PendingWrites::new(),
        passkey_mgr: None,
        public_dir: None,
        scan_options: crate::scanner::ScanOptions::default(),
        #[cfg(feature = "semantic")]
        vector_index: None,
        asset_storage: crate::assets::store::StorageCounterGuard::new(0),
        asset_max_file_bytes: 10 * 1024 * 1024,
        asset_max_total_bytes: 100 * 1024 * 1024,
    };

    run(state, port, bind_addr, std::time::Duration::from_secs(60)).await
}

/// ETag + conditional-GET middleware for text/html responses.
///
/// Computes a weak ETag as the first 16 hex chars of blake3 over the
/// response body. On a request carrying `If-None-Match` that matches the
/// computed tag, the response is replaced with `304 Not Modified` and an
/// empty body — saving re-transfer of unchanged pages.
///
/// Scope: GET requests; 200 OK responses with a Content-Type starting
/// `text/html`; responses without an explicit `Cache-Control: no-store`.
/// Responses already carrying an ETag (set by a downstream handler) are
/// passed through unchanged.
async fn etag_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let if_none_match = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let resp = next.run(req).await;
    if method != axum::http::Method::GET {
        return resp;
    }
    if resp.status() != StatusCode::OK {
        return resp;
    }
    let headers = resp.headers();
    if headers.contains_key(header::ETAG) {
        return resp;
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct.starts_with("text/html") {
        return resp;
    }
    if let Some(cc) = headers.get(header::CACHE_CONTROL) {
        if let Ok(s) = cc.to_str() {
            if s.contains("no-store") {
                return resp;
            }
        }
    }

    let (parts, body) = resp.into_parts();
    let bytes = match axum::body::to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return Response::from_parts(parts, axum::body::Body::empty()),
    };
    let digest = blake3::hash(&bytes);
    let etag = format!("W/\"{}\"", &digest.to_hex()[..16]);

    if let Some(inm) = if_none_match {
        if inm.split(',').any(|t| t.trim() == etag) {
            let mut not_modified = Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .body(axum::body::Body::empty())
                .unwrap();
            *not_modified.headers_mut() = parts.headers.clone();
            not_modified
                .headers_mut()
                .insert(header::ETAG, etag.parse().unwrap());
            not_modified.headers_mut().remove(header::CONTENT_LENGTH);
            return not_modified;
        }
    }

    let mut new_resp = Response::from_parts(parts, axum::body::Body::from(bytes));
    new_resp
        .headers_mut()
        .insert(header::ETAG, etag.parse().unwrap());
    new_resp
}

/// Spawn a background task that prunes comments older than 30 days once per hour.
fn spawn_comment_prune_task(vault_root: Arc<PathBuf>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let vr = vault_root.clone();
            let _ =
                tokio::task::spawn_blocking(
                    move || match crate::user::comment::prune_all_comments(&vr) {
                        Ok(0) => {}
                        Ok(n) => eprintln!("comment prune: removed {n} expired comments"),
                        Err(e) => eprintln!("comment prune error: {e}"),
                    },
                )
                .await;
        }
    })
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
