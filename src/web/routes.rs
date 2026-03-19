use std::collections::HashSet;
use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderName, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use base64::Engine as _;
use crate::hooks;
use crate::hooks::context::{build_hook_context, HookSaved};
use crate::scanner::{body_text_ranges, page_slug_from_path};
use crate::search::{
    byte_offset_to_line_col, detect_headings, extract_search_context, find_heading_for_offset,
    in_body_text, SearchMatch, SearchOutput,
};
use crate::search_index::SearchIndex;
use crate::web::context::{build_folder_context, build_page_context, build_vault_context};
use crate::web::engine::TemplateError;
use crate::web::html::{html_escape, urlencoding};
use crate::web::markdown;
use crate::web::{reindex, WebState};

/// Collect recent git edits (up to `limit`) from the vault's git log.
///
/// Returns `Vec<(summary, author_name, ISO-8601 time, files_changed)>`.
fn recent_git_edits(
    git_lock: &Option<std::sync::Arc<crate::web::git_commit::GitCommitLock>>,
    limit: usize,
) -> Vec<serde_json::Value> {
    let Some(lock) = git_lock else {
        return Vec::new();
    };
    let repo = match lock.lock() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let Ok(mut revwalk) = repo.revwalk() else {
        return Vec::new();
    };
    let _ = revwalk.push_head();
    revwalk.set_sorting(git2::Sort::TIME).ok();

    let mut edits = Vec::new();
    for oid in revwalk.flatten().take(limit) {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let summary = commit
            .summary()
            .unwrap_or("")
            .to_string();
        let author = commit
            .author()
            .name()
            .unwrap_or("unknown")
            .to_string();
        let time = commit.time();
        let secs = time.seconds();
        // Format as a simple relative/absolute time string
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let age = now.saturating_sub(secs);
        let dt = if age < 60 {
            "just now".to_string()
        } else if age < 3600 {
            format!("{} min ago", age / 60)
        } else if age < 86400 {
            format!("{} hours ago", age / 3600)
        } else {
            format!("{} days ago", age / 86400)
        };

        // Count changed files by diffing with parent
        let file_count = if let Ok(parent) = commit.parent(0) {
            let old_tree = parent.tree().ok();
            let new_tree = commit.tree().ok();
            repo.diff_tree_to_tree(old_tree.as_ref(), new_tree.as_ref(), None)
                .map(|d| d.deltas().count())
                .unwrap_or(0)
        } else {
            // Initial commit — count all files in tree
            commit
                .tree()
                .ok()
                .map(|t| {
                    let mut n = 0usize;
                    t.walk(git2::TreeWalkMode::PreOrder, |_, _| {
                        n += 1;
                        git2::TreeWalkResult::Ok
                    })
                    .ok();
                    n
                })
                .unwrap_or(0)
        };

        edits.push(serde_json::json!({
            "summary": summary,
            "author": author,
            "time": dt,
            "file_count": file_count,
        }));
    }
    edits
}

/// GET /_me — User dashboard with recent edits, accessible pages, role summary, etc.
#[allow(unused_variables)]
pub async fn dashboard_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionUser,
) -> Response {
    let vault_root = &*state.vault_root;
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Load user profile
    let profile = match crate::user::load_profile(vault_root, &session.user_id) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "user profile not found").into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response()
        }
    };

    let role = crate::user::Role::for_profile_with_vault(&profile, vault_root);
    let is_admin = role >= crate::user::Role::Admin;

    // Recent git edits (last 20 commits)
    let recent_edits = recent_git_edits(&state.git_commit_lock, 20);

    // Accessible pages — filter by read ACL in collab mode (REQ-020-031).
    let data = state.data.read().unwrap();
    let accessible_pages: Vec<serde_json::Value> = data
        .page_names
        .iter()
        .filter(|name| {
            #[cfg(feature = "reason")]
            if state.collab {
                let slug = data.slug_for_page(name);
                return check_page_acl_read(&state, &session.user_id, &slug).is_ok();
            }
            true
        })
        .map(|name| {
            let slug = data.slug_for_page(name);
            serde_json::json!({ "name": name, "slug": slug })
        })
        .collect();
    let page_count = accessible_pages.len();
    drop(data);

    // Pending invitations (admin only)
    let pending_invites: Vec<serde_json::Value> = if is_admin {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        crate::user::invite::load_pending_invitations(vault_root)
            .unwrap_or_default()
            .into_iter()
            .filter(|i| i.exp > now && !i.revoked)
            .map(|i| {
                serde_json::json!({
                    "role": i.role,
                    "pages": i.pages,
                    "nonce": &i.nonce[..8],
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // Access requests (admin only, REQ-020-047)
    let access_requests: Vec<serde_json::Value> = if is_admin {
        crate::user::access_request::load_access_requests(vault_root)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.status == "pending")
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "user": r.user,
                    "page": r.page,
                    "requested_at": r.requested_at,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // Active session count
    let active_sessions = state.sessions.active_session_count(&session.user_id);

    // CSRF token for the session
    let csrf_token = state
        .sessions
        .csrf_token(&session.token)
        .unwrap_or_default();

    // Passkey count
    let passkey_count = profile.credentials.len();

    match state.engine.render_dashboard(
        &vault_name,
        &csrf_token,
        &profile.name,
        &profile.id,
        &role.to_string(),
        is_admin,
        &recent_edits,
        &accessible_pages,
        page_count,
        &pending_invites,
        &access_requests,
        active_sessions,
        passkey_count,
    ) {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// GET / — Landing page with vault stats and page grid.
#[allow(unused_variables, unused_mut)]
pub async fn index_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let data = state.data.read().unwrap();
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let mut vault_ctx = build_vault_context(&data, &vault_name);

    // ── Visibility filtering for sidebar (REQ-020-031) ────────────────
    #[cfg(feature = "reason")]
    if state.collab {
        if let Some(ref uid) = extract_session_user_id(&state, &headers) {
            let sidebar_denied = build_sidebar_denied_map(&state, &data, uid);
            if !sidebar_denied.is_empty() {
                crate::web::context::filter_vault_context_for_visibility(
                    &mut vault_ctx,
                    &sidebar_denied,
                );
            }
        }
    }

    #[cfg(feature = "history")]
    {
        // OBS-013: time vault history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) = crate::history::build_template_history_context(&state.vault_root) {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
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
    #[cfg(feature = "semantic")]
    {
        vault_ctx.semantic_available = state.vector_index.is_some();
    }
    match state.engine.render_index(&vault_ctx, "serve", "", "") {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// GET /{*path} — Rendered markdown page with backlinks, or folder index.
#[allow(unused_variables)]
pub async fn page_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // Intercept /_history suffix → render page history UI.
    if let Some(page_slug) = slug.strip_suffix("/_history") {
        return page_history_handler_inner(State(state), page_slug.to_string()).await;
    }

    let data = state.data.read().unwrap();

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Find the file by matching slug (relative path without extension)
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));

    // ── ACL check for collab mode (REQ-020-030, REQ-020-033) ──────────
    #[cfg(feature = "reason")]
    if state.collab && file.is_some() {
        if let Some(user_id) = extract_session_user_id(&state, &headers) {
            let page_slug_str = file.map(|f| page_slug_from_path(&f.path)).unwrap_or_default();
            let page_spl: Vec<crate::types::SplBlock> = data
                .files
                .iter()
                .filter(|f| page_slug_from_path(&f.path) == page_slug_str)
                .flat_map(|f| f.spl_blocks.iter().cloned())
                .collect();
            let all_slugs: Vec<String> = data
                .files
                .iter()
                .map(|f| page_slug_from_path(&f.path))
                .collect();

            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;

            let is_agent = crate::web::session::bearer_token_from_headers(&headers)
                .and_then(|t| crate::web::session::verify_bearer_token(&state.vault_root, &t))
                .is_some();

            let query = crate::acl::AclQuery {
                user_id: user_id.clone(),
                page_slug: page_slug_str.clone(),
                action: crate::acl::Action::Read,
                is_agent,
                now_epoch_ms: now_ms,
            };

            // Check ACL cache first, then evaluate
            let decision = {
                let cache = state.acl_cache.lock().unwrap();
                if let Some(cached) = cache.lookup(&user_id, &page_slug_str, crate::acl::Action::Read) {
                    cached.clone()
                } else {
                    drop(cache);
                    match crate::acl::evaluate(&state.vault_root, &query, &page_spl, &all_slugs) {
                        Ok(d) => {
                            let mut cache = state.acl_cache.lock().unwrap();
                            cache.insert(user_id.clone(), page_slug_str.clone(), crate::acl::Action::Read, d.clone());
                            d
                        }
                        Err(_) => {
                            // On error, default to denied
                            crate::acl::AclDecision::Denied {
                                tag: crate::acl::ConclusionTag::DefeasiblyNotProvable,
                                rule_trace: vec![],
                            }
                        }
                    }
                }
            };

            if !decision.is_allowed() {
                let vis_mode = crate::acl::query_visibility_mode(&state.vault_root);
                let page_override = crate::acl::query_page_visibility_override(
                    &state.vault_root,
                    &user_id,
                    &page_slug_str,
                    &page_spl,
                    &all_slugs,
                );
                let effective = crate::acl::effective_visibility(vis_mode, page_override);

                let page_name = file.map(|f| f.page_name.clone()).unwrap_or_default();
                drop(data);

                return match effective {
                    crate::acl::VisibilityMode::Hidden => {
                        // REQ-020-033: Return 404 identical to nonexistent page
                        (StatusCode::NOT_FOUND, Html(format!(
                            "<html><body><h1>404 — Not Found</h1><p>The page <code>/{slug}</code> does not exist.</p></body></html>",
                            slug = crate::web::html::html_escape(slug),
                        ))).into_response()
                    }
                    _ => {
                        // REQ-020-033/REQ-020-047/REQ-020-054: Return 403 with page title,
                        // lock icon, admin contact names, and "Request Access" button.
                        let admin_names = crate::user::admin_names(&state.vault_root);
                        let contact_line = if admin_names.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "<p style=\"color:#7aa2f7;font-size:0.85rem;\">Contact {} to request access.</p>",
                                crate::web::html::html_escape(&admin_names.join(", "))
                            )
                        };

                        (StatusCode::FORBIDDEN, Html(format!(
                            "<html><head>\
                            <meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
                            <title>Access Denied</title>\
                            <style>\
                            body {{ font-family: system-ui, -apple-system, sans-serif; background: #1a1b26; color: #a9b1d6; display: flex; justify-content: center; align-items: center; min-height: 100vh; margin: 0; }}\
                            .box {{ text-align: center; max-width: 420px; padding: 2rem; }}\
                            h1 {{ font-size: 1.5rem; margin-bottom: 0.5rem; }}\
                            p {{ color: #565f89; font-size: 0.9rem; }}\
                            a {{ color: #7aa2f7; text-decoration: none; }}\
                            a:hover {{ text-decoration: underline; }}\
                            .btn {{ display: inline-block; margin-top: 1rem; padding: 0.6rem 1.5rem; background: #7aa2f7; color: #1a1b26; border: none; border-radius: 6px; font-size: 0.9rem; font-weight: 600; cursor: pointer; }}\
                            .btn:hover {{ background: #89b4fa; }}\
                            .btn:disabled {{ opacity: 0.5; cursor: default; }}\
                            #ar-status {{ margin-top: 0.75rem; font-size: 0.85rem; min-height: 1.2em; }}\
                            </style>\
                            </head><body>\
                            <div class=\"box\">\
                            <h1>\u{1f512} {title}</h1>\
                            <p>You don't have access to this page.</p>\
                            {contact_line}\
                            <p><a href=\"/api/acl/explain?page={slug_encoded}&amp;action=read\">Why?</a></p>\
                            <button class=\"btn\" id=\"ar-btn\" onclick=\"requestAccess()\">Request Access</button>\
                            <div id=\"ar-status\"></div>\
                            </div>\
                            <script>\
                            async function requestAccess() {{\
                              var btn = document.getElementById('ar-btn');\
                              var st = document.getElementById('ar-status');\
                              btn.disabled = true;\
                              btn.textContent = 'Requesting...';\
                              try {{\
                                var csrf = document.cookie.split(';').map(function(c){{return c.trim();}}).find(function(c){{return c.startsWith('zetl_csrf=');}});\
                                var csrfVal = csrf ? csrf.split('=')[1] : '';\
                                var r = await fetch('/api/access-request', {{\
                                  method: 'POST',\
                                  headers: {{'Content-Type': 'application/json', 'X-CSRF-Token': csrfVal}},\
                                  body: JSON.stringify({{page: '{page_slug}'}})\
                                }});\
                                var d = await r.json();\
                                if (d.data && d.data.status === 'already_pending') {{\
                                  st.textContent = 'Request already pending.';\
                                  st.style.color = '#e0af68';\
                                }} else if (r.ok) {{\
                                  st.textContent = 'Request sent! An admin will review it.';\
                                  st.style.color = '#9ece6a';\
                                }} else {{\
                                  st.textContent = 'Failed to send request.';\
                                  st.style.color = '#f7768e';\
                                  btn.disabled = false;\
                                  btn.textContent = 'Request Access';\
                                }}\
                              }} catch(e) {{\
                                st.textContent = 'Network error.';\
                                st.style.color = '#f7768e';\
                                btn.disabled = false;\
                                btn.textContent = 'Request Access';\
                              }}\
                            }}\
                            </script>\
                            </body></html>",
                            title = crate::web::html::html_escape(&page_name),
                            slug_encoded = crate::web::html::urlencoding(&page_slug_str),
                            page_slug = crate::web::html::html_escape(&page_slug_str),
                        ))).into_response()
                    }
                };
            }
        }
    }

    // If no page matches, check if slug is a folder prefix → render folder index
    if file.is_none() {
        let folder_prefix = format!("{}/", slug.to_lowercase());
        let has_pages = data.files.iter().any(|f| {
            let s = page_slug_from_path(&f.path);
            s.to_lowercase().starts_with(&folder_prefix)
        });

        if has_pages {
            let folder_name = slug.rsplit('/').next().unwrap_or(slug);
            let vault_ctx = build_vault_context(&data, &vault_name);
            #[cfg(feature = "history")]
            {
                // OBS-013: time vault history context build.
                let hist_start = std::time::Instant::now();
                if let Some(hist) =
                    crate::history::build_template_history_context(&state.vault_root)
                {
                    let hist_ms = hist_start.elapsed().as_millis();
                    if state.verbose {
                        eprintln!(
                            "[zetl] history-context: vault trend={} points recent={} changes duration_ms={}",
                            hist.trend.len(),
                            hist.recent_changes.len(),
                            hist_ms
                        );
                    }
                    vault_ctx.history =
                        serde_json::to_value(hist).unwrap_or(serde_json::Value::Null);
                }
            }
            #[cfg(feature = "semantic")]
            {
                vault_ctx.semantic_available = state.vector_index.is_some();
            }
            let folder_ctx = build_folder_context(&data, slug, folder_name);
            return match state
                .engine
                .render_folder(&vault_ctx, &folder_ctx, "serve", "", "")
            {
                Ok(html) => Html(html).into_response(),
                Err(e) => render_error_response(e),
            };
        }
    }

    // Build denied-pages map for visibility-aware wikilink rendering (REQ-020-032).
    #[cfg(feature = "reason")]
    let denied_pages_map: std::collections::HashMap<String, markdown::DeniedLinkStyle> =
        if state.collab {
            if let Some(ref uid) = extract_session_user_id(&state, &headers) {
                build_denied_pages_map(&state, &data, uid)
            } else {
                std::collections::HashMap::new()
            }
        } else {
            std::collections::HashMap::new()
        };

    let (rendered, page_name, current_slug, raw_content) = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        let file_slug = page_slug_from_path(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let is_fountain = file.path.extension().map_or(false, |e| e == "fountain");
                let html = if is_fountain {
                    let body = crate::web::build::strip_fountain_frontmatter(&content);
                    format!(
                        "<script type=\"text/fountain\" id=\"fountain-source\">{}</script>\n<div id=\"fountain-render\"></div>",
                        html_escape(&body)
                    )
                } else {
                    #[cfg(feature = "reason")]
                    {
                        if denied_pages_map.is_empty() {
                            markdown::render_to_html(&content, &data.page_slug_map, "/", "")
                        } else {
                            markdown::render_to_html_with_visibility(
                                &content,
                                &data.page_slug_map,
                                &denied_pages_map,
                                "/",
                                "",
                            )
                        }
                    }
                    #[cfg(not(feature = "reason"))]
                    {
                        markdown::render_to_html(&content, &data.page_slug_map, "/", "")
                    }
                };
                (html, file.page_name.clone(), file_slug, Some(content))
            }
            Err(_) => (
                "<p class=\"text-error\">Could not read file.</p>".to_string(),
                file.page_name.clone(),
                file_slug,
                None,
            ),
        }
    } else {
        // Page doesn't exist yet — open directly in edit mode
        let name = slug.rsplit('/').next().unwrap_or(slug).to_string();
        (
            String::new(),
            name.clone(),
            slug.to_string(),
            Some(format!("# {name}\n")),
        )
    };

    // Build transclusion cards HTML (still assembled as raw HTML for the template)
    let forward_links = data.graph.forward_links(&page_name);
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

    let mut transclusion_cards = String::new();
    for (i, target) in unique_targets.iter().enumerate() {
        let color = colors[i % colors.len()];
        let target_slug = data.slug_for_page(target);
        let href = urlencoding(&target_slug);

        let preview_html = data
            .files
            .iter()
            .find(|f| f.page_name.eq_ignore_ascii_case(target))
            .and_then(|file| {
                let full_path = state.vault_root.join(&file.path);
                std::fs::read_to_string(&full_path).ok()
            })
            .map(|content| markdown::render_preview_html(&content, &data.page_slug_map, "/", ""))
            .unwrap_or_else(|| format!("<p><em>{}</em></p>", html_escape("(page does not exist)")));

        transclusion_cards.push_str(&format!(
            r#"<div class="transclusion-card" data-target-href="/{href}/" style="border-left-color: {color};">
  <a href="/{href}/" class="tc-title" style="color: {color};">{name}</a>
  <div class="tc-excerpt prose prose-sm max-w-none">{preview}</div>
</div>"#,
            href = href,
            color = color,
            name = html_escape(target),
            preview = preview_html,
        ));
    }

    // Build the page context using the builder, then fill in transclusion + raw_escaped
    let content_raw = raw_content.as_deref().unwrap_or("");
    let mut page_ctx = build_page_context(&data, &page_name, &current_slug, &rendered, content_raw);
    page_ctx.transclusion_cards = transclusion_cards;
    page_ctx.raw_escaped = raw_content.map(|c| html_escape(&c));
    #[cfg(feature = "history")]
    {
        // OBS-013: time per-page history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) =
            crate::history::build_template_page_history_context(&page_name, &state.vault_root)
        {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
                eprintln!(
                    "[zetl] history-context: page {:?} trend={} points created={} duration_ms={}",
                    page_name,
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
            crate::history::build_backlink_since_map(&page_name, &sources, &state.vault_root);
        if !since_map.is_empty() {
            for bl in &mut page_ctx.backlinks {
                bl.since = since_map.get(&bl.title.to_lowercase()).cloned();
            }
        }
    }

    let vault_ctx = build_vault_context(&data, &vault_name);
    #[cfg(feature = "history")]
    {
        // OBS-013: time vault history context build.
        let hist_start = std::time::Instant::now();
        if let Some(hist) = crate::history::build_template_history_context(&state.vault_root) {
            let hist_ms = hist_start.elapsed().as_millis();
            if state.verbose {
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
    #[cfg(feature = "semantic")]
    {
        vault_ctx.semantic_available = state.vector_index.is_some();
    }
    match state
        .engine
        .render_page(&vault_ctx, &page_ctx, "serve", "", "")
    {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// GET /edit/{*slug} — Serve the collaborative editor page.
pub async fn edit_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');
    let data = state.data.read().unwrap();

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Find the file by matching slug
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));

    // Read raw content
    let (page_name, raw_content, file_slug) = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        let content = std::fs::read_to_string(&full_path).unwrap_or_default();
        let file_slug = page_slug_from_path(&file.path);
        (file.page_name.clone(), content, file_slug)
    } else {
        // New page — start with empty content
        let name = slug
            .rsplit('/')
            .next()
            .unwrap_or(slug)
            .replace('-', " ");
        let capitalized = if let Some(first) = name.chars().next() {
            first.to_uppercase().to_string() + &name[first.len_utf8()..]
        } else {
            name.clone()
        };
        (capitalized, String::new(), slug.to_string())
    };

    // Issue a WS ticket for this user
    let user_name = if state.collab {
        extract_session_user_id(&state, &headers).unwrap_or_else(|| "anonymous".to_string())
    } else {
        "anonymous".to_string()
    };
    let ticket = state.ticket_store.issue(&user_name);

    // Build breadcrumbs
    let breadcrumbs = crate::web::context::build_breadcrumbs(&file_slug);

    // CSRF token for save requests in collab mode
    let csrf_token = crate::web::session::token_from_cookies(&headers)
        .and_then(|t| state.sessions.csrf_token(&t))
        .unwrap_or_default();

    // Build editor JSON data
    let editor_json = serde_json::json!({
        "slug": file_slug,
        "ticket": ticket,
        "content": raw_content,
        "user_name": user_name,
        "csrf_token": csrf_token,
    })
    .to_string();

    let vault_ctx = build_vault_context(&data, &vault_name);

    match state
        .engine
        .render_editor(&vault_ctx, &page_name, &file_slug, &breadcrumbs, &editor_json)
    {
        Ok(html) => {
            let mut resp = Html(html).into_response();
            // Editor CSP: allow esm.sh CDN for CodeMirror 6 modules
            resp.headers_mut().insert(
                HeaderName::from_static("content-security-policy"),
                "default-src 'self'; \
                 script-src 'self' 'unsafe-inline' https://esm.sh; \
                 style-src 'self' 'unsafe-inline' https://esm.sh https://cdn.jsdelivr.net; \
                 connect-src 'self' ws: wss:; \
                 font-src 'self' https://cdn.jsdelivr.net; \
                 frame-ancestors 'none'"
                    .parse()
                    .unwrap(),
            );
            resp
        }
        Err(e) => render_error_response(e),
    }
}

/// PUT /{*path} — Save edited markdown back to the vault file, then re-index.
pub async fn save_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
    body: String,
) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // Look up file path under read lock, then drop it before writing.
    // For new pages, create at vault_root/{slug}.md.
    let full_path = {
        let data = state.data.read().unwrap();
        let file = data
            .files
            .iter()
            .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));
        if let Some(file) = file {
            state.vault_root.join(&file.path)
        } else {
            state.vault_root.join(format!("{slug}.md"))
        }
    };

    // Ensure parent directory exists for nested paths
    if let Some(parent) = full_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("mkdir error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Cannot create directory").into_response();
        }
    }

    if let Err(e) = std::fs::write(&full_path, &body) {
        eprintln!("save error: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Write failed").into_response();
    }

    // ── Git auto-commit (REQ-020-015, CON-020-006) ────────────────────
    // Resolve user identity early so we can attribute the git commit.
    let session_user_id: Option<String> = crate::web::session::token_from_cookies(&headers)
        .and_then(|token| state.sessions.validate(&token));

    if let Some(ref lock) = state.git_commit_lock {
        let rel_path_for_git = full_path
            .strip_prefix(state.vault_root.as_ref())
            .unwrap_or(&full_path)
            .to_path_buf();

        // Resolve author from authenticated session, fallback to "zetl".
        let (author_name, author_id) = if let Some(ref uid) = session_user_id {
            match crate::user::load_profile(&state.vault_root, uid) {
                Ok(Some(profile)) => (profile.name.clone(), profile.id.clone()),
                _ => ("zetl".to_string(), "zetl".to_string()),
            }
        } else {
            ("zetl".to_string(), "zetl".to_string())
        };

        let custom_message = headers
            .get("X-Commit-Message")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match lock.lock() {
            Ok(repo) => {
                match super::git_commit::auto_commit(
                    &repo,
                    &rel_path_for_git,
                    &author_name,
                    &author_id,
                    custom_message.as_deref(),
                ) {
                    Ok(_oid) => {
                        // Synchronize jj's view of the git repo (step 4).
                        super::git_commit::jj_git_import(&state.vault_root);
                    }
                    Err(e) => {
                        eprintln!("warning: git auto-commit failed: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("warning: git commit lock poisoned: {e}");
            }
        }
    }

    // Re-index the vault so the graph/links and search index reflect the edit.
    match reindex(&state.vault_root) {
        Ok(new_data) => {
            // Rebuild Tantivy search index so the reader picks up the new content
            // (ReloadPolicy::OnCommitWithDelay causes the existing reader to reload).
            if let Err(e) = SearchIndex::build(&state.vault_root, &new_data.files) {
                eprintln!("search index rebuild error: {e}");
            }
            *state.data.write().unwrap() = new_data;
        }
        Err(e) => {
            eprintln!("reindex error: {e}");
            // File was saved; index is stale but not fatal
        }
    }

    // REQ-020-020: X-No-Hooks suppression — skip on-save hooks when the header
    // is present and the request is authenticated.
    let no_hooks = headers
        .get("X-No-Hooks")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
        && session_user_id.is_some();

    // Fire on-save hooks asynchronously so the response returns immediately.
    if !no_hooks {
        let vault_root = state.vault_root.clone();
        let theme = state.theme.clone();
        let content_length = body.len();
        let rel_path = full_path
            .strip_prefix(vault_root.as_ref())
            .unwrap_or(&full_path)
            .to_string_lossy()
            .into_owned();
        let page_name = full_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        let hook_user_id = session_user_id.clone();
        tokio::task::spawn_blocking(move || {
            let theme_hooks = hooks::resolve_theme_hooks(&vault_root, &theme);
            let manifest = hooks::discover_hooks(&vault_root, theme_hooks.path());

            if hooks::hooks_for(&manifest, "on-save").is_empty() {
                return;
            }

            // Re-scan vault for fresh graph data used by hook context.
            let files = match crate::scanner::scan_vault(&vault_root, &[]) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("on-save hook: scan error: {e}");
                    return;
                }
            };
            let file_index: Vec<(String, std::path::PathBuf)> = files
                .iter()
                .map(|f| (f.page_name.clone(), f.path.clone()))
                .collect();
            let mut resolved = std::collections::HashMap::new();
            for file in &files {
                for link in &file.links {
                    let key = link.raw_target.clone();
                    if !resolved.contains_key(&key) {
                        if let Some(r) =
                            crate::scanner::resolve_page_name(&link.target_page, &file_index)
                        {
                            resolved.insert(key, r);
                        }
                    }
                }
            }
            let graph = crate::graph::LinkGraph::build(&files, &resolved);

            let mut ctx = build_hook_context(
                "on-save",
                &vault_root,
                &theme,
                env!("CARGO_PKG_VERSION"),
                &files,
                &graph,
            );
            ctx.saved = Some(HookSaved {
                file: rel_path.clone(),
                page: page_name.clone(),
                content_length,
                is_external: false,
            });

            // Attach authenticated user identity if a session was present.
            if let Some(ref uid) = hook_user_id {
                if let Ok(Some(profile)) = crate::user::load_profile(&vault_root, uid) {
                    ctx.user = Some(crate::hooks::context::HookUser::from_profile(&profile, false, &vault_root));
                }
            }

            let context_json = match serde_json::to_vec(&ctx) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("on-save hook: json error: {e}");
                    return;
                }
            };

            let hook_env = hooks::HookEnv {
                vault_root: vault_root.to_path_buf(),
                theme: theme.clone(),
                zetl_version: env!("CARGO_PKG_VERSION").to_string(),
                extra_vars: vec![
                    ("ZETL_SAVED_FILE".into(), rel_path),
                    ("ZETL_SAVED_PAGE".into(), page_name),
                    ("ZETL_HOOK_DEPTH".into(), "0".into()),
                ],
            };

            let results = hooks::run_hooks(&manifest, "on-save", &context_json, &hook_env);
            for result in results {
                match result {
                    Ok(output) if !output.success() => {
                        eprintln!(
                            "warning: on-save hook '{}' ({}) exited with code {}",
                            output.path.display(),
                            output.source,
                            output.exit_code.unwrap_or(-1),
                        );
                        if !output.stderr.is_empty() {
                            eprintln!("  stderr: {}", output.stderr.trim_end());
                        }
                    }
                    Err(e) => {
                        eprintln!("warning: on-save hook failed to execute: {e}");
                    }
                    _ => {}
                }
            }
        });
    }

    StatusCode::OK.into_response()
}

/// GET /preview/{*path} — Returns a short HTML preview (for tooltip).
pub async fn preview_handler(
    State(state): State<WebState>,
    Path(slug): Path<String>,
) -> Html<String> {
    let slug = urldecode(&slug);
    let data = state.data.read().unwrap();

    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));

    let preview = if let Some(file) = file {
        let full_path = state.vault_root.join(&file.path);
        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                let text = markdown::render_preview(&content);
                format!(
                    r#"<div class="font-semibold mb-1">{}</div><div class="opacity-80">{}</div>"#,
                    html_escape(&file.page_name),
                    text
                )
            }
            Err(_) => "<em>Could not read file.</em>".to_string(),
        }
    } else {
        format!("<em>{} (does not exist)</em>", html_escape(&slug))
    };

    Html(preview)
}

/// Query parameters for GET /api/search.
#[derive(Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
    /// Search mode: "bm25" (default), "semantic", or "hybrid". REQ-100.
    pub mode: Option<String>,
}

/// GET /api/search — Full-text, semantic, or hybrid search over the vault index.
///
/// Query parameters:
///   - q (required): search query string
///   - limit (optional, default 20): max results
///   - mode (optional, default "bm25"): search mode — "bm25", "semantic", or "hybrid"
///
/// Returns 400 if `q` is absent or whitespace-only.
/// Returns 503 if mode=semantic or mode=hybrid but the vector index is unavailable.
///
/// REQ-013-012, REQ-100, CON-013-003.
pub async fn api_search_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<SearchParams>,
) -> Response {
    let q = match params.q.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            return (StatusCode::BAD_REQUEST, "Missing or empty 'q' parameter").into_response();
        }
    };

    let limit = params.limit.unwrap_or(20).max(1);
    let mode = params.mode.as_deref().unwrap_or("bm25");

    // Build set of denied page slugs for search filtering (REQ-020-031).
    // Search MUST NEVER return content snippets for pages the user cannot read.
    #[cfg(feature = "reason")]
    let denied_page_slugs: HashSet<String> = if state.collab {
        if let Some(ref uid) = extract_session_user_id(&state, &headers) {
            let data = state.data.read().unwrap();
            let denied = build_denied_pages_map(&state, &data, uid);
            drop(data);
            denied.keys().cloned().collect()
        } else {
            HashSet::new()
        }
    } else {
        HashSet::new()
    };

    #[cfg(not(feature = "reason"))]
    let denied_page_slugs: HashSet<String> = HashSet::new();
    let _ = &headers; // suppress unused warning when reason feature is off

    // When the semantic feature is not compiled, reject semantic/hybrid modes immediately.
    #[cfg(not(feature = "semantic"))]
    if mode == "semantic" || mode == "hybrid" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Semantic search requires the `semantic` feature. \
             Rebuild with: cargo build --features semantic",
        )
            .into_response();
    }

    // ── semantic / hybrid modes (REQ-100) ────────────────────────────────────
    #[cfg(feature = "semantic")]
    if mode == "semantic" || mode == "hybrid" {
        let vec_index_arc = match &state.vector_index {
            Some(arc) => arc.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Vector index not available. Run `zetl index` first.",
                )
                    .into_response();
            }
        };

        if mode == "semantic" {
            // Pure vector search: embed query, scan index, return chunk-level hits.
            let q_owned = q.clone();
            let vec_limit = limit;
            let hits = tokio::task::spawn_blocking(move || {
                let idx = vec_index_arc.lock().map_err(|_| {
                    anyhow::anyhow!("vector index lock poisoned")
                })?;
                idx.query_text(&q_owned, vec_limit)
            })
            .await;

            let hits = match hits {
                Ok(Ok(h)) => h,
                Ok(Err(e)) => {
                    eprintln!("api/search semantic error: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Semantic search failed")
                        .into_response();
                }
                Err(e) => {
                    eprintln!("api/search semantic join error: {e}");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "Semantic search failed")
                        .into_response();
                }
            };

            // Filter out denied pages from semantic results (REQ-020-031)
            let hits: Vec<_> = if denied_page_slugs.is_empty() {
                hits
            } else {
                hits.into_iter()
                    .filter(|h| !denied_page_slugs.contains(&h.page_name))
                    .collect()
            };

            let total = hits.len();
            let results: Vec<SearchMatch> = hits
                .into_iter()
                .map(|hit| SearchMatch {
                    page: hit.page_name,
                    path: hit.path,
                    line: 0,
                    column: 0,
                    context: hit.heading.clone(),
                    heading: hit.heading,
                    heading_level: None,
                    score: hit.score as f64,
                })
                .collect();

            let output = SearchOutput {
                query: q,
                total_matches: total,
                near: None,
                depth: None,
                neighbourhood_size: None,
                results,
            };
            return Json(output).into_response();
        }

        // mode == "hybrid": RRF fusion of BM25 + vector search (ADR-053, REQ-095).
        let bm25_hits = match state.search_index.query(&q, limit.saturating_mul(2)) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("api/search bm25 error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Search failed").into_response();
            }
        };

        let q_owned = q.clone();
        let vec_limit = limit.saturating_mul(2);
        let vec_hits = tokio::task::spawn_blocking(move || {
            let idx = vec_index_arc.lock().map_err(|_| {
                anyhow::anyhow!("vector index lock poisoned")
            })?;
            idx.query_text(&q_owned, vec_limit)
        })
        .await;

        let vec_hits = match vec_hits {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                eprintln!("api/search vector error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Hybrid search failed")
                    .into_response();
            }
            Err(e) => {
                eprintln!("api/search hybrid join error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Hybrid search failed")
                    .into_response();
            }
        };

        // Build ranked lists at page level for RRF.
        let bm25_ranks: Vec<(String, usize)> = bm25_hits
            .iter()
            .enumerate()
            .map(|(i, h)| (h.page_name.clone(), i + 1))
            .collect();
        let vec_ranks: Vec<(String, usize)> = {
            // Deduplicate chunks by page: keep highest-scoring chunk per page.
            let mut seen: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for hit in &vec_hits {
                let rank = seen.len() + 1;
                let entry = seen.entry(hit.page_name.clone()).or_insert(0);
                // rank is 1-based; we record order of first appearance
                if *entry == 0 {
                    *entry = rank;
                }
            }
            let mut page_order: Vec<(String, usize)> = seen.into_iter().collect();
            page_order.sort_by_key(|(_, r)| *r);
            page_order
        };

        let fused = crate::semantic::core::reciprocal_rank_fusion(
            &bm25_ranks,
            &vec_ranks,
            crate::semantic::RRF_K,
        );

        // Build a score map from page_name → fused score.
        let score_map: std::collections::HashMap<String, f64> =
            fused.into_iter().collect();

        // Collect BM25 hits for pages in the fused set, scored by RRF.
        let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut all_matches: Vec<SearchMatch> = Vec::new();

        for hit in &bm25_hits {
            let rrf_score = match score_map.get(&hit.page_name) {
                Some(&s) => s,
                None => continue,
            };
            let abs_path = state.vault_root.join(&hit.path);
            let content = match std::fs::read_to_string(&abs_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let body_ranges = body_text_ranges(&content);
            let line_starts: Vec<usize> = std::iter::once(0)
                .chain(content.match_indices('\n').map(|(i, _)| i + 1))
                .collect();
            let headings = detect_headings(&content, &body_ranges);
            let search_content = content.to_lowercase();

            for term in &terms {
                let mut start = 0usize;
                while let Some(pos) = search_content[start..].find(term.as_str()) {
                    let byte_offset = start + pos;
                    start = byte_offset + 1;
                    if !in_body_text(byte_offset, &body_ranges) {
                        continue;
                    }
                    let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                    let ctx = extract_search_context(&content, byte_offset, term.len(), 80);
                    let (heading, heading_level) = find_heading_for_offset(&headings, byte_offset);
                    all_matches.push(SearchMatch {
                        page: hit.page_name.clone(),
                        path: hit.path.clone(),
                        line,
                        column: col,
                        context: ctx,
                        heading,
                        heading_level,
                        score: rrf_score,
                    });
                }
            }
        }

        // Filter out denied pages from hybrid search results (REQ-020-031)
        if !denied_page_slugs.is_empty() {
            all_matches.retain(|m| !denied_page_slugs.contains(&m.page));
        }

        all_matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.path.cmp(&b.path))
                .then(a.line.cmp(&b.line))
        });

        let total = all_matches.len();
        all_matches.truncate(limit);

        let output = SearchOutput {
            query: q,
            total_matches: total,
            near: None,
            depth: None,
            neighbourhood_size: None,
            results: all_matches,
        };
        return Json(output).into_response();
    }

    // ── BM25 mode (default) ──────────────────────────────────────────────────
    let hits = match state.search_index.query(&q, limit) {
        Ok(hits) => hits,
        Err(e) => {
            eprintln!("api/search error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Search failed").into_response();
        }
    };

    let terms: Vec<String> = q.split_whitespace().map(|t| t.to_lowercase()).collect();

    let mut all_matches: Vec<SearchMatch> = Vec::new();

    for hit in &hits {
        let abs_path = state.vault_root.join(&hit.path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body_ranges = body_text_ranges(&content);
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(content.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let headings = detect_headings(&content, &body_ranges);
        let search_content = content.to_lowercase();

        for term in &terms {
            let mut start = 0usize;
            while let Some(pos) = search_content[start..].find(term.as_str()) {
                let byte_offset = start + pos;
                start = byte_offset + 1;

                if !in_body_text(byte_offset, &body_ranges) {
                    continue;
                }

                let (line, col) = byte_offset_to_line_col(&line_starts, byte_offset);
                let ctx = extract_search_context(&content, byte_offset, term.len(), 80);
                let (heading, heading_level) = find_heading_for_offset(&headings, byte_offset);

                all_matches.push(SearchMatch {
                    page: hit.page_name.clone(),
                    path: hit.path.clone(),
                    line,
                    column: col,
                    context: ctx,
                    heading,
                    heading_level,
                    score: hit.score,
                });
            }
        }
    }

    // Filter out denied pages from search results (REQ-020-031)
    if !denied_page_slugs.is_empty() {
        all_matches.retain(|m| !denied_page_slugs.contains(&m.page));
    }

    all_matches.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });

    let total = all_matches.len();
    all_matches.truncate(limit);

    let output = SearchOutput {
        query: q,
        total_matches: total,
        near: None,
        depth: None,
        neighbourhood_size: None,
        results: all_matches,
    };

    Json(output).into_response()
}

/// GET /_print — Combined print view of all pages for PDF export.
pub async fn print_handler(State(state): State<WebState>) -> Response {
    let data = state.data.read().unwrap();

    // Collect only fountain scenes, sorted alphabetically by title (sidebar order)
    let mut fountain_files: Vec<_> = data
        .files
        .iter()
        .filter(|f| f.path.extension().map_or(false, |e| e == "fountain"))
        .collect();
    fountain_files.sort_by(|a, b| a.page_name.to_lowercase().cmp(&b.page_name.to_lowercase()));

    let mut sections = Vec::new();
    for file in &fountain_files {
        let full_path = state.vault_root.join(&file.path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let body = crate::web::build::strip_fountain_frontmatter(&content);
        sections.push(format!(
            "<section class=\"print-section\">\
             <script type=\"text/fountain\">{src}</script>\
             <div class=\"fountain-out\"></div></section>",
            src = html_escape(&body),
        ));
    }

    // Inline the Fountain.js parser from the bundled theme
    let fountain_js = crate::web::engine::bundled_template("fountain", "base.html")
        .and_then(|base| {
            let start_marker = "<!-- fountain-js";
            let end_marker = "</script>\n\n  <!-- Render fountain";
            let start = base.find(start_marker)?;
            let end = base.find(end_marker).map(|i| i + "</script>".len())?;
            Some(base[start..end].to_string())
        })
        .unwrap_or_default();

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Print — Script</title>
<style>
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Regular.woff2') format('woff2');
    font-weight: 400; font-style: normal; font-display: swap;
  }}
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Bold.woff2') format('woff2');
    font-weight: 700; font-style: normal; font-display: swap;
  }}
  @font-face {{
    font-family: 'Courier Prime';
    src: url('/_static/CourierPrime-Italic.woff2') format('woff2');
    font-weight: 400; font-style: italic; font-display: swap;
  }}
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{
    font-family: 'Courier Prime', 'Courier New', Courier, monospace;
    font-size: 12pt;
    line-height: 1;
    color: #1a1a1a;
    background: #c8c8c8;
  }}
  .print-wrap {{
    max-width: 8.5in;
    margin: 1rem auto;
    background: white;
    padding: 1in 1in 1in 1.5in;
    box-shadow: 0 2px 14px rgba(0,0,0,0.22);
  }}
  .scene-heading {{ font-weight: 700; text-transform: uppercase; margin-top: 2em; margin-bottom: 1em; }}
  .action {{ margin: 1em 0; }}
  .character {{ text-transform: uppercase; margin-left: 2in; margin-top: 1em; margin-bottom: 0; }}
  .dialog {{ margin: 0; padding: 0; }}
  .dialog .character {{ margin-top: 1em; }}
  .lines, .dialogue {{ margin-left: 1in; width: 3.5in; margin-top: 0; margin-bottom: 0; }}
  .paren, .parenthetical {{ margin-left: 1.6in; width: 2in; margin-top: 0; margin-bottom: 0; }}
  .trans, .transition {{ text-transform: uppercase; text-align: right; width: 100%; margin: 1em 0; }}
  .center {{ text-align: center; width: 100%; margin: 1em 0; }}
  hr.page-break {{ border: none; page-break-after: always; margin: 0; }}
  .bold {{ font-weight: 700; }}
  .italic {{ font-style: italic; }}
  .underline {{ text-decoration: underline; }}
  a.wikilink, a.wikilink-dead {{ color: inherit; text-decoration: none; }}
  /* Title page */
  .title-page {{
    display: flex; flex-direction: column; align-items: center;
    justify-content: center; min-height: 9in; text-align: center;
    page-break-after: always;
  }}
  .title-page h1 {{ font-size: 24pt; font-weight: 400; text-decoration: underline; margin-bottom: 1em; }}
  .title-page .credit {{ margin-top: 2em; }}
  .title-page .authors {{ margin-top: 0.5em; }}
  .title-page .source {{ margin-top: 0.5em; }}
  .title-page .draft-date, .title-page .date {{ margin-top: 2em; }}
  .title-page .contact {{ margin-top: auto; align-self: flex-start; text-align: left; }}
  .title-page .copyright {{ margin-top: 0.5em; align-self: flex-start; text-align: left; }}
  .title-page .notes {{ margin-top: 1em; font-style: italic; }}
  /* Keep character name + dialogue/parenthetical together across page breaks */
  .dialog {{ page-break-inside: avoid; }}
  .character {{ page-break-after: avoid; }}
  .scene-heading {{ page-break-after: avoid; page-break-before: auto; }}
  .scene-heading + .action {{ page-break-before: avoid; }}

  .title-page {{ counter-reset: page; }}
  @page {{
    size: letter;
    margin: 1in 1in 1in 1.5in;
    @bottom-right {{
      content: counter(page) ".";
      font-family: 'Courier Prime', 'Courier New', Courier, monospace;
      font-size: 12pt;
    }}
  }}
  @page :first {{
    @bottom-right {{ content: none; }}
  }}
  @media print {{
    body {{ background: white; }}
    .print-wrap {{ box-shadow: none; padding: 0; max-width: none; margin: 0; }}
    hr.page-break {{ page-break-after: always; }}
  }}
</style>
</head>
<body>
<div class="print-wrap">
{sections}
</div>

{fountain_js}

<script>
(function(){{
  document.querySelectorAll('.print-section').forEach(function(sec){{
    var src = sec.querySelector('script[type="text/fountain"]');
    var dest = sec.querySelector('.fountain-out');
    if(!src||!dest) return;
    var result = window.fountain.parse(src.textContent);
    var out = '';
    if(result.html.title_page){{
      var d=new Date();
      var months=['January','February','March','April','May','June','July','August','September','October','November','December'];
      var dateStr=months[d.getMonth()]+' '+d.getDate()+', '+d.getFullYear();
      out += '<div class="title-page">' + result.html.title_page + '<p class="draft-date">' + dateStr + '</p></div>';
    }}
    out += result.html.script;
    dest.innerHTML = out;
  }});
  setTimeout(function(){{ window.print(); }}, 600);
}})();
</script>
</body>
</html>"##,
        sections = sections.join("\n"),
        fountain_js = fountain_js,
    );

    Html(html).into_response()
}

/// GET /_static/{*path} — Serve static assets with two-tier lookup.
///
/// Lookup order:
///   1. .zetl/themes/<active-theme>/static/<path>  (per-theme override)
///   2. .zetl/static/<path>                         (vault-wide fallback)
///
/// Returns 404 if neither location has the file (or if the directories don't exist).
pub async fn static_handler(
    State(state): State<WebState>,
    Path(req_path): Path<String>,
) -> Response {
    // Reject path traversal: no ".." components, no null bytes
    if req_path.contains('\0') || req_path.split('/').any(|seg| seg == "..") {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Also reject empty path
    if req_path.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Build candidate paths (two-tier lookup)
    let zetl_dir = state.vault_root.join(".zetl");
    let candidates: Vec<PathBuf> = {
        let mut c = Vec::with_capacity(2);
        // 1. Theme-specific static dir
        if !state.theme.is_empty() {
            c.push(
                zetl_dir
                    .join("themes")
                    .join(&state.theme)
                    .join("static")
                    .join(&req_path),
            );
        }
        // 2. Vault-wide static dir
        c.push(zetl_dir.join("static").join(&req_path));
        c
    };

    for candidate in &candidates {
        // Canonicalize and ensure it's still within the expected static dir
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if !canonical.is_file() {
            continue;
        }

        // Safety: ensure canonical path is under .zetl/
        let Ok(zetl_canonical) = zetl_dir.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(&zetl_canonical) {
            return StatusCode::NOT_FOUND.into_response();
        }

        let Ok(body) = std::fs::read(&canonical) else {
            continue;
        };

        let mime = mime_from_ext(&req_path);
        return ([(header::CONTENT_TYPE, mime)], body).into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

/// Infer a MIME type from a file path's extension.
fn mime_from_ext(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "js" | "mjs" => "application/javascript",
        "css" => "text/css",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "xml" => "application/xml",
        "txt" => "text/plain",
        "map" => "application/json",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Convert a TemplateError into a 500 response with a styled HTML error page.
/// Build a map of page names → denied link styles for the current user (REQ-020-032).
///
/// For each page in the vault, checks if the user can read it. For denied pages,
/// determines the appropriate link rendering style based on visibility mode and
/// per-page overrides.
#[cfg(feature = "reason")]
fn build_denied_pages_map(
    state: &WebState,
    data: &crate::web::VaultData,
    user_id: &str,
) -> std::collections::HashMap<String, markdown::DeniedLinkStyle> {
    use crate::acl::{self, VisibilityMode};

    let vis_mode = acl::query_visibility_mode(&state.vault_root);
    let all_slugs: Vec<String> = data
        .files
        .iter()
        .map(|f| page_slug_from_path(&f.path))
        .collect();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut denied = std::collections::HashMap::new();

    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        let query = acl::AclQuery {
            user_id: user_id.to_string(),
            page_slug: slug.clone(),
            action: acl::Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms,
        };

        // Check cache first
        let decision = {
            let cache = state.acl_cache.lock().unwrap();
            if let Some(cached) = cache.lookup(user_id, &slug, acl::Action::Read) {
                cached.clone()
            } else {
                drop(cache);
                let page_spl: Vec<crate::types::SplBlock> = file.spl_blocks.clone();
                match acl::evaluate(&state.vault_root, &query, &page_spl, &all_slugs) {
                    Ok(d) => {
                        let mut cache = state.acl_cache.lock().unwrap();
                        cache.insert(user_id.to_string(), slug.clone(), acl::Action::Read, d.clone());
                        d
                    }
                    Err(_) => continue,
                }
            }
        };

        if !decision.is_allowed() {
            let page_override = acl::query_page_visibility_override(
                &state.vault_root,
                user_id,
                &slug,
                &file.spl_blocks,
                &all_slugs,
            );
            let effective = acl::effective_visibility(vis_mode, page_override);
            let style = match effective {
                VisibilityMode::Transparent => markdown::DeniedLinkStyle::GrayedOut,
                VisibilityMode::Mixed => markdown::DeniedLinkStyle::Locked,
                VisibilityMode::Hidden => markdown::DeniedLinkStyle::DeadLink,
            };
            denied.insert(file.page_name.clone(), style);
        }
    }

    denied
}

/// Build a map of page slug → sidebar denied style for the current user (REQ-020-031).
///
/// Determines which pages should be hidden, grayed-out, or locked in the sidebar
/// based on the vault's visibility mode and per-page overrides.
#[cfg(feature = "reason")]
fn build_sidebar_denied_map(
    state: &WebState,
    data: &crate::web::VaultData,
    user_id: &str,
) -> std::collections::HashMap<String, crate::web::context::SidebarDeniedStyle> {
    use crate::acl::{self, VisibilityMode};
    use crate::web::context::SidebarDeniedStyle;

    let vis_mode = acl::query_visibility_mode(&state.vault_root);
    let all_slugs: Vec<String> = data
        .files
        .iter()
        .map(|f| page_slug_from_path(&f.path))
        .collect();

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let mut denied = std::collections::HashMap::new();

    for file in &data.files {
        let slug = page_slug_from_path(&file.path);
        let query = acl::AclQuery {
            user_id: user_id.to_string(),
            page_slug: slug.clone(),
            action: acl::Action::Read,
            is_agent: false,
            now_epoch_ms: now_ms,
        };

        let decision = {
            let cache = state.acl_cache.lock().unwrap();
            if let Some(cached) = cache.lookup(user_id, &slug, acl::Action::Read) {
                cached.clone()
            } else {
                drop(cache);
                let page_spl: Vec<crate::types::SplBlock> = file.spl_blocks.clone();
                match acl::evaluate(&state.vault_root, &query, &page_spl, &all_slugs) {
                    Ok(d) => {
                        let mut cache = state.acl_cache.lock().unwrap();
                        cache.insert(user_id.to_string(), slug.clone(), acl::Action::Read, d.clone());
                        d
                    }
                    Err(_) => continue,
                }
            }
        };

        if !decision.is_allowed() {
            let page_override = acl::query_page_visibility_override(
                &state.vault_root,
                user_id,
                &slug,
                &file.spl_blocks,
                &all_slugs,
            );
            let effective = acl::effective_visibility(vis_mode, page_override);
            let style = match effective {
                VisibilityMode::Transparent => SidebarDeniedStyle::GrayedOut,
                VisibilityMode::Mixed => SidebarDeniedStyle::Hidden,
                VisibilityMode::Hidden => SidebarDeniedStyle::Hidden,
            };
            // ForceVisible override → show with lock, even in mixed/hidden
            if page_override == acl::PageVisibilityOverride::ForceVisible {
                denied.insert(slug, SidebarDeniedStyle::Locked);
            } else {
                denied.insert(slug, style);
            }
        }
    }

    denied
}

// ── Page history UI /{slug}/_history ─────────────────────────────────────

/// Inner handler for `/{slug}/_history` — renders chronological edit list.
async fn page_history_handler_inner(
    State(state): State<WebState>,
    page_slug: String,
) -> Response {
    let data = state.data.read().unwrap();

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Find the file by matching slug.
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&page_slug));

    let Some(file) = file else {
        return (
            StatusCode::NOT_FOUND,
            Html(format!(
                "<html><body><h1>404 — Not Found</h1><p>The page <code>/{}</code> does not exist.</p></body></html>",
                html_escape(&page_slug),
            )),
        ).into_response();
    };

    let page_name = file.page_name.clone();
    let file_path = file.path.clone();
    let current_slug = page_slug_from_path(&file.path);

    // Check for active CRDT draft (non-empty WAL file).
    let has_draft = {
        let wal_path = state.wal_store.wal_path(&current_slug);
        std::fs::metadata(&wal_path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    };

    // Collect git log entries for this file.
    let git_entries = if let Some(ref lock) = state.git_commit_lock {
        let repo = lock.lock().unwrap();
        crate::web::git_commit::file_log(&repo, &file_path, 100)
    } else {
        Vec::new()
    };

    let vault_ctx = build_vault_context(&data, &vault_name);
    let breadcrumbs: Vec<crate::web::context::BreadcrumbEntry> = {
        let parts: Vec<&str> = current_slug.split('/').collect();
        let mut crumbs = Vec::new();
        for i in 0..parts.len().saturating_sub(1) {
            let slug = parts[..=i].join("/");
            crumbs.push(crate::web::context::BreadcrumbEntry {
                title: parts[i].to_string(),
                slug,
            });
        }
        crumbs
    };

    let history_json = serde_json::to_string(&git_entries).unwrap_or_else(|_| "[]".to_string());

    match state.engine.render_page_history(
        &vault_ctx,
        &page_name,
        &current_slug,
        &breadcrumbs,
        &history_json,
        has_draft,
    ) {
        Ok(html) => Html(html).into_response(),
        Err(e) => render_error_response(e),
    }
}

/// GET /api/history/file-diff — unified diff of a file at a specific commit.
///
/// Query params: `commit` (required), `slug` (required).
pub async fn api_file_diff_handler(
    State(state): State<WebState>,
    Query(params): Query<FileDiffParams>,
) -> Response {
    let commit = match params.commit.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'commit'" })),
            )
                .into_response();
        }
    };
    let slug = match params.slug.as_deref() {
        Some(s) if !s.trim().is_empty() => urldecode(s),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'slug'" })),
            )
                .into_response();
        }
    };

    let data = state.data.read().unwrap();
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));

    let Some(file) = file else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "page not found" }))).into_response();
    };

    let file_path = file.path.clone();
    drop(data);

    let Some(ref lock) = state.git_commit_lock else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "git not available" }))).into_response();
    };

    let repo = lock.lock().unwrap();
    let diff = crate::web::git_commit::file_diff_at_commit(&repo, &commit, &file_path);

    Json(serde_json::json!({ "diff": diff })).into_response()
}

/// POST /api/history/restore — restore a page to the content from a specific commit.
///
/// JSON body: `{ "commit": "...", "slug": "..." }`.
pub async fn api_restore_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<RestoreBody>,
) -> Response {
    let commit = body.commit.trim();
    let slug = urldecode(body.slug.trim());

    if commit.is_empty() || slug.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "commit and slug are required" })),
        )
            .into_response();
    }

    let data = state.data.read().unwrap();
    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(&slug));

    let Some(file) = file else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "page not found" }))).into_response();
    };

    let file_path = file.path.clone();
    drop(data);

    // In collab mode, verify the user has edit permission on this page.
    #[cfg(feature = "reason")]
    if state.collab {
        let user_id = match extract_session_user_id(&state, &headers) {
            Some(uid) => uid,
            None => {
                return ApiResponse::err(
                    StatusCode::UNAUTHORIZED,
                    "INVALID_TOKEN",
                    "authentication required",
                )
                .into_response();
            }
        };
        if let Err(resp) = check_page_acl_edit(&state, &user_id, &slug) {
            return resp;
        }
    }

    let Some(ref lock) = state.git_commit_lock else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "git not available" }))).into_response();
    };

    // Read the file content at the specified commit.
    let content = {
        let repo = lock.lock().unwrap();
        crate::web::git_commit::file_at_commit(&repo, commit, &file_path)
    };

    let Some(content) = content else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "file not found at commit" }))).into_response();
    };

    // Write the content back to disk.
    let full_path = state.vault_root.join(&file_path);
    if let Err(e) = std::fs::write(&full_path, &content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("write failed: {e}") })),
        )
            .into_response();
    }

    // Auto-commit the restore.
    {
        let repo = lock.lock().unwrap();
        let user_name = extract_session_user_id(&state, &headers)
            .unwrap_or_else(|| "system".to_string());
        let msg = format!("restore: {} to {}", slug, &commit[..7.min(commit.len())]);
        let _ = crate::web::git_commit::auto_commit(
            &repo,
            &file_path,
            &user_name,
            &user_name,
            Some(&msg),
        );
        crate::web::git_commit::jj_git_import(&state.vault_root);
    }

    // Re-index the vault.
    match reindex(&state.vault_root) {
        Ok(new_data) => {
            let _ = SearchIndex::build(&state.vault_root, &new_data.files);
            *state.data.write().unwrap() = new_data;
        }
        Err(e) => eprintln!("reindex error after restore: {e}"),
    }

    Json(serde_json::json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub struct FileDiffParams {
    pub commit: Option<String>,
    pub slug: Option<String>,
}

#[derive(Deserialize)]
pub struct RestoreBody {
    pub commit: String,
    pub slug: String,
}

fn render_error_response(err: TemplateError) -> Response {
    eprintln!("template error: {err}");
    (StatusCode::INTERNAL_SERVER_ERROR, Html(err.to_error_html())).into_response()
}

/// Decode %20-style URL encoding.
fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next().unwrap_or(b'0');
            let lo = chars.next().unwrap_or(b'0');
            let byte = hex_byte(hi, lo);
            result.push(byte as char);
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

fn hex_byte(hi: u8, lo: u8) -> u8 {
    (hex_nibble(hi) << 4) | hex_nibble(lo)
}

fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from Howard Hinnant's chrono-compatible date math
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── History API handlers (REQ-087, CON-027, ADR-050) ─────────────────────────
//
// All four handlers are compiled only with `--features history`.

/// Query parameters for GET /api/history.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryLogParams {
    pub since: Option<String>,
    pub limit: Option<usize>,
}

/// Query parameters for GET /api/history/page/{name}.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryPageParams {
    pub limit: Option<usize>,
}

/// Query parameters for GET /api/history/at.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryAtParams {
    pub t: Option<String>,
}

/// Query parameters for GET /api/history/diff.
#[cfg(feature = "history")]
#[derive(Deserialize)]
pub struct HistoryDiffParams {
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Map a history error message to the appropriate HTTP status code.
///
/// `NO_HISTORY`, `SNAPSHOT_NOT_FOUND`, and `PAGE_NOT_FOUND` error codes
/// map to 404; everything else maps to 500.
#[cfg(feature = "history")]
fn history_status_code(msg: &str) -> StatusCode {
    if msg.contains("NO_HISTORY")
        || msg.contains("SNAPSHOT_NOT_FOUND")
        || msg.contains("PAGE_NOT_FOUND")
    {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

/// Build a JSON error response for a history error.
#[cfg(feature = "history")]
fn history_err_response(err: anyhow::Error) -> Response {
    let msg = err.to_string();
    let status = history_status_code(&msg);
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// GET /api/history — vault timeline (REQ-087, CON-027).
///
/// Query params: `since` (optional time expression), `limit` (optional, default 50).
/// Returns `Vec<HistoryEntry>` as JSON.
/// Returns 404 when history has never been initialised (NO_HISTORY).
#[cfg(feature = "history")]
pub async fn api_history_log_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryLogParams>,
) -> Response {
    let vault_root = state.vault_root.clone();
    let since = params.since.clone();
    let limit = params.limit.unwrap_or(50).max(1);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::core::build_vault_history;
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();
        build_vault_history(&snapshots, &vault_root, since.as_deref(), limit, now)
    })
    .await;

    match result {
        Ok(Ok(entries)) => Json(entries).into_response(),
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/page/{name} — per-page evolution timeline (REQ-087, CON-027).
///
/// Path param: `name` (URL-decoded page name).
/// Query params: `limit` (optional, default 50).
/// Returns `Vec<PageHistoryEntry>` as JSON.
/// Returns 404 when NO_HISTORY or when the page is not found in any snapshot.
#[cfg(feature = "history")]
pub async fn api_history_page_handler(
    State(state): State<WebState>,
    Path(name): Path<String>,
    Query(params): Query<HistoryPageParams>,
) -> Response {
    let vault_root = state.vault_root.clone();
    let page_name = urldecode(&name);
    let limit = params.limit.unwrap_or(50).max(1);

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{
            extract_page_history, extract_vault_root_hash_from_description,
        };
        use crate::history::jj_backend::VcsBackend as _;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;

        let cache = HistoricalIndexCache::with_default_capacity();
        let files_per_snapshot: Vec<Option<Vec<_>>> = snapshots
            .iter()
            .map(|snap| {
                let hash = extract_vault_root_hash_from_description(&snap.description)?;
                let file_map = cache.load(&vault_root, &hash).ok().flatten()?;
                Some(file_map.into_values().collect())
            })
            .collect();

        let entries = extract_page_history(&page_name, &snapshots, &files_per_snapshot, limit);

        if entries.is_empty() {
            anyhow::bail!("PAGE_NOT_FOUND: page '{page_name}' not found in any snapshot");
        }

        Ok(entries)
    })
    .await;

    match result {
        Ok(Ok(entries)) => Json(entries).into_response(),
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/at — vault state at a point in time (REQ-087, CON-027).
///
/// Query params: `t` (required time expression).
/// Returns 400 when `t` is absent or blank.
/// Returns 404 when NO_HISTORY or SNAPSHOT_NOT_FOUND.
/// Returns 202 Accepted (non-blocking) when the historical index is not yet
/// cached for the resolved snapshot (ADR-050).
#[cfg(feature = "history")]
pub async fn api_history_at_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryAtParams>,
) -> Response {
    let t = match params.t.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 't'" })),
            )
                .into_response();
        }
    };

    let vault_root = state.vault_root.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{extract_vault_root_hash_from_description, resolve_snapshot};
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();
        let snap = resolve_snapshot(&t, now, &snapshots)?;

        let vault_root_hash = extract_vault_root_hash_from_description(&snap.description);
        let cache = HistoricalIndexCache::with_default_capacity();
        let pages = vault_root_hash.as_deref().and_then(|hash| {
            let file_map = cache.load(&vault_root, hash).ok().flatten()?;
            let mut pages: Vec<_> = file_map
                .into_values()
                .map(|f| {
                    serde_json::json!({
                        "page_name": f.page_name,
                        "link_count": f.links.len(),
                    })
                })
                .collect();
            pages.sort_by(|a, b| {
                a["page_name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["page_name"].as_str().unwrap_or(""))
            });
            Some(pages)
        });

        Ok((
            snap.change_id.clone(),
            snap.timestamp.to_rfc3339(),
            vault_root_hash,
            pages,
        ))
    })
    .await;

    match result {
        Ok(Ok((change_id, timestamp, vault_root_hash, Some(pages)))) => Json(serde_json::json!({
            "snapshot": {
                "change_id": change_id,
                "timestamp": timestamp,
                "vault_root_hash": vault_root_hash,
            },
            "status": "ok",
            "pages": pages,
        }))
        .into_response(),
        Ok(Ok((change_id, timestamp, vault_root_hash, None))) => {
            // Non-blocking cache miss: index not yet cached for this snapshot.
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "snapshot": {
                        "change_id": change_id,
                        "timestamp": timestamp,
                        "vault_root_hash": vault_root_hash,
                    },
                    "status": "pending",
                    "pages": [],
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /api/history/diff — graph delta between two points in time (REQ-087, CON-027).
///
/// Query params: `from` (required), `to` (required time expressions).
/// Returns 400 when either param is absent.
/// Returns 404 when NO_HISTORY or SNAPSHOT_NOT_FOUND.
/// Returns 202 Accepted when cached index is unavailable for either endpoint.
#[cfg(feature = "history")]
pub async fn api_history_diff_handler(
    State(state): State<WebState>,
    Query(params): Query<HistoryDiffParams>,
) -> Response {
    let from_expr = match params.from.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'from'" })),
            )
                .into_response();
        }
    };
    let to_expr = match params.to.as_deref() {
        Some(s) if !s.trim().is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "missing required parameter 'to'" })),
            )
                .into_response();
        }
    };

    let vault_root = state.vault_root.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        use crate::history::cache::HistoricalIndexCache;
        use crate::history::core::{
            compute_graph_delta, extract_vault_root_hash_from_description, resolve_snapshot,
        };
        use crate::history::jj_backend::VcsBackend as _;
        use chrono::Local;

        let backend = crate::history::open_history(&vault_root)?;
        let snapshots = backend.list_changes(10_000)?;
        let now = Local::now().fixed_offset();

        let from_snap = resolve_snapshot(&from_expr, now, &snapshots)?;
        let to_snap = resolve_snapshot(&to_expr, now, &snapshots)?;

        let cache = HistoricalIndexCache::with_default_capacity();
        let from_hash = extract_vault_root_hash_from_description(&from_snap.description);
        let to_hash = extract_vault_root_hash_from_description(&to_snap.description);

        let from_files = from_hash
            .as_deref()
            .and_then(|h| cache.load(&vault_root, h).ok().flatten())
            .map(|m| m.into_values().collect::<Vec<_>>());
        let to_files = to_hash
            .as_deref()
            .and_then(|h| cache.load(&vault_root, h).ok().flatten())
            .map(|m| m.into_values().collect::<Vec<_>>());

        let delta = match (&from_files, &to_files) {
            (Some(from), Some(to)) => Some(compute_graph_delta(from, to)),
            _ => None,
        };

        Ok((
            from_snap.change_id.clone(),
            from_snap.timestamp.to_rfc3339(),
            to_snap.change_id.clone(),
            to_snap.timestamp.to_rfc3339(),
            delta,
        ))
    })
    .await;

    match result {
        Ok(Ok((from_id, from_ts, to_id, to_ts, Some(delta)))) => Json(serde_json::json!({
            "from": { "change_id": from_id, "timestamp": from_ts },
            "to":   { "change_id": to_id,   "timestamp": to_ts },
            "delta": delta,
        }))
        .into_response(),
        Ok(Ok((from_id, from_ts, to_id, to_ts, None))) => {
            // Non-blocking cache miss.
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "from": { "change_id": from_id, "timestamp": from_ts },
                    "to":   { "change_id": to_id,   "timestamp": to_ts },
                    "status": "pending",
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => history_err_response(e),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Passkey registration guidance routes (REQ-020-046) ──────────────────

/// GET /passkey/register — Passkey registration guidance screen.
pub async fn passkey_register_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<PasskeyRegisterParams>,
) -> Response {
    // In collab mode, redirect to bootstrap if no valid session exists.
    // This handles stale cookies after a server restart.
    if state.collab && extract_session_user_id(&state, &headers).is_none() {
        return axum::response::Redirect::temporary("/auth/bootstrap").into_response();
    }

    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let user_id = params.user_id.as_deref().unwrap_or("");

    match state.engine.render_passkey_register(&vault_name, user_id) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("{}", e.stderr_line("passkey/register"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PasskeyRegisterParams {
    pub user_id: Option<String>,
}

/// POST /api/passkey/register/start — Begin passkey registration ceremony.
pub async fn passkey_register_start_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PasskeyApiRequest>,
) -> Response {
    let vault_root = &*state.vault_root;

    // In collab mode, require an authenticated session and verify the caller
    // is registering a passkey for their own account.
    if state.collab {
        match extract_session_user_id(&state, &headers) {
            Some(session_uid) if session_uid == body.user_id => { /* ok */ }
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    "cannot register a passkey for another user".to_string(),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    "authentication required".to_string(),
                )
                    .into_response();
            }
        }
    }

    // Per-user rate limit on passkey registration attempts
    if let Err(retry_after) = state.rate_limiters.passkey_per_user.check(&body.user_id) {
        return crate::web::rate_limit::too_many_requests(retry_after);
    }

    let passkey_mgr = match &state.passkey_mgr {
        Some(mgr) => mgr.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "passkey manager not available".to_string(),
            )
                .into_response();
        }
    };

    let profile = match crate::user::load_profile(vault_root, &body.user_id) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                format!("user not found: {}", body.user_id),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response();
        }
    };

    let existing =
        crate::user::passkey::load_passkeys(vault_root, &body.user_id).unwrap_or_default();

    match passkey_mgr.start_registration(&body.user_id, &profile.name, &existing) {
        Ok(ccr) => Json(ccr).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("registration start failed: {e}"),
        )
            .into_response(),
    }
}

/// POST /api/passkey/register/finish — Complete passkey registration ceremony.
pub async fn passkey_register_finish_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let vault_root = &*state.vault_root;

    let user_id = match body.get("user_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (StatusCode::BAD_REQUEST, "missing user_id").into_response();
        }
    };

    // In collab mode, require an authenticated session matching the target user.
    if state.collab {
        match extract_session_user_id(&state, &headers) {
            Some(session_uid) if session_uid == user_id => { /* ok */ }
            Some(_) => {
                return (
                    StatusCode::FORBIDDEN,
                    "cannot register a passkey for another user".to_string(),
                )
                    .into_response();
            }
            None => {
                return (
                    StatusCode::UNAUTHORIZED,
                    "authentication required".to_string(),
                )
                    .into_response();
            }
        }
    }

    // Per-user rate limit on passkey registration finish attempts
    if let Err(retry_after) = state.rate_limiters.passkey_per_user.check(&user_id) {
        return crate::web::rate_limit::too_many_requests(retry_after);
    }

    let passkey_mgr = match &state.passkey_mgr {
        Some(mgr) => mgr.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "passkey manager not available".to_string(),
            )
                .into_response();
        }
    };

    let reg_cred: webauthn_rs::prelude::RegisterPublicKeyCredential =
        match serde_json::from_value(body.clone()) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("invalid credential response: {e}"),
                )
                    .into_response();
            }
        };

    match passkey_mgr.finish_registration(&user_id, &reg_cred) {
        Ok(passkey) => {
            let mut passkeys =
                crate::user::passkey::load_passkeys(vault_root, &user_id).unwrap_or_default();
            passkeys.push(passkey.clone());
            if let Err(e) = crate::user::passkey::save_passkeys(vault_root, &user_id, &passkeys) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to save passkey: {e}"),
                )
                    .into_response();
            }

            // Update the profile credentials list
            let now = {
                let d = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default();
                let secs = d.as_secs();
                // Simple UTC ISO-8601 without chrono dependency
                let days = secs / 86400;
                let day_secs = secs % 86400;
                let h = day_secs / 3600;
                let m = (day_secs % 3600) / 60;
                let s = day_secs % 60;
                // Days since 1970-01-01 → Y/M/D (good enough for timestamps)
                let (y, mo, d) = days_to_ymd(days);
                format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
            };
            let cred = crate::user::passkey::passkey_to_credential(&passkey, None, &now);
            if let Ok(Some(mut profile)) = crate::user::load_profile(vault_root, &user_id) {
                profile.credentials.push(cred);
                let _ = crate::user::save_profile(vault_root, &profile);
            }

            (StatusCode::OK, "passkey registered").into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            format!("registration failed: {e}"),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct PasskeyApiRequest {
    pub user_id: String,
}

// -- Recovery display (CON-020-002) --

#[derive(Debug, Deserialize)]
pub struct RecoveryShowParams {
    pub user_id: Option<String>,
}

/// GET /recovery/show?user_id=<id> — generate and display the 12-word recovery
/// phrase.  The mnemonic is generated fresh, the derived public key is saved to
/// the user profile, and the phrase is rendered once (never stored server-side).
pub async fn recovery_show_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<RecoveryShowParams>,
) -> Response {
    let vault_root = &*state.vault_root;
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let user_id = params.user_id.as_deref().unwrap_or("");
    if user_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing user_id parameter").into_response();
    }

    // In collab mode, require authentication and verify the caller is requesting
    // their own recovery phrase (prevents unauthenticated account takeover).
    if state.collab {
        match extract_session_user_id(&state, &headers) {
            Some(session_uid) if session_uid == user_id => { /* ok */ }
            Some(_) => {
                return (StatusCode::FORBIDDEN, "cannot access another user's recovery phrase")
                    .into_response();
            }
            None => {
                return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
            }
        }
    }

    // One-time serve: reject if mnemonic was already shown for this user (REQ-020-056).
    {
        let mut shown = state.mnemonic_shown.lock().unwrap();
        if shown.contains(user_id) {
            return (
                StatusCode::GONE,
                "recovery phrase has already been displayed",
            )
                .into_response();
        }
        shown.insert(user_id.to_string());
    }

    // Load profile to confirm user exists
    let mut profile = match crate::user::load_profile(vault_root, user_id) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "user not found").into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response();
        }
    };

    // Refuse to regenerate if the user already has a recovery pubkey persisted
    // on disk.  The in-memory mnemonic_shown guard only survives one process
    // lifetime; this check is durable across restarts.
    if !profile.recovery_pubkey.is_empty() {
        return (
            StatusCode::GONE,
            "recovery phrase has already been generated for this user",
        )
            .into_response();
    }

    // Generate the recovery keypair
    let keypair = match crate::user::recovery::generate_recovery_keypair() {
        Ok(kp) => kp,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to generate recovery phrase: {e}"),
            )
                .into_response();
        }
    };

    // Persist the recovery public key to the user profile
    profile.recovery_pubkey = keypair.recovery_pubkey;
    if let Err(e) = crate::user::save_profile(vault_root, &profile) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save profile: {e}"),
        )
            .into_response();
    }

    let words: Vec<&str> = keypair.mnemonic.split_whitespace().collect();
    let continue_url = format!("/passkey/register?user_id={}", user_id);

    // CSP header for mnemonic display page (REQ-020-056, REQ-020-063).
    let csp = "default-src 'self'; script-src 'self' 'unsafe-inline'; \
               style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
               connect-src 'self' wss:; frame-ancestors 'none'";

    match state
        .engine
        .render_recovery_show(&vault_name, &keypair.mnemonic, &words, &continue_url)
    {
        Ok(html) => {
            let mut resp = Html(html).into_response();
            let hdrs = resp.headers_mut();
            hdrs.insert(
                HeaderName::from_static("content-security-policy"),
                csp.parse().unwrap(),
            );
            hdrs.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            resp
        }
        Err(e) => {
            eprintln!("{}", e.stderr_line("recovery/show"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

// -- Recovery endpoints (CON-020-002, REQ-020-002) --

#[derive(Debug, Deserialize)]
pub struct RecoverQuery {
    pub user: String,
}

/// GET /auth/recover?user=<id> — issue a 256-bit recovery challenge.
pub async fn recover_challenge_handler(
    State(state): State<WebState>,
    Query(query): Query<RecoverQuery>,
) -> Response {
    let vault_root = &*state.vault_root;
    let user_id = &query.user;

    // Per-user rate limit on recovery challenge requests
    if let Err(retry_after) = state.rate_limiters.recovery_per_user.check(user_id) {
        return crate::web::rate_limit::too_many_requests(retry_after);
    }

    // Verify user exists and has a recovery_pubkey
    match crate::user::load_profile(vault_root, user_id) {
        Ok(Some(profile)) if !profile.recovery_pubkey.is_empty() => {}
        Ok(Some(_)) => {
            return (StatusCode::BAD_REQUEST, "user has no recovery key").into_response();
        }
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "user not found").into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response();
        }
    }

    match state.recovery_challenges.issue_challenge(user_id) {
        Ok(challenge) => {
            let challenge_b64 =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge);
            let body = serde_json::json!({ "challenge": challenge_b64 });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("too many") {
                (StatusCode::TOO_MANY_REQUESTS, msg).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RecoverRequest {
    pub user_id: String,
    pub challenge_response: String,
}

/// POST /auth/recover — verify signed challenge, issue session, redirect to passkey registration.
pub async fn recover_verify_handler(
    State(state): State<WebState>,
    Json(body): Json<RecoverRequest>,
) -> Response {
    let vault_root = &*state.vault_root;

    // Per-user rate limit on recovery verify requests
    if let Err(retry_after) = state.rate_limiters.recovery_per_user.check(&body.user_id) {
        return crate::web::rate_limit::too_many_requests(retry_after);
    }

    // Load user profile
    let profile = match crate::user::load_profile(vault_root, &body.user_id) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "user not found").into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load profile: {e}"),
            )
                .into_response();
        }
    };

    if profile.recovery_pubkey.is_empty() {
        return (StatusCode::BAD_REQUEST, "user has no recovery key").into_response();
    }

    // Decode the challenge_response: base64url(challenge || signature)
    // challenge = 32 bytes, signature = 64 bytes → 96 bytes total
    let response_bytes = match base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&body.challenge_response)
    {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::BAD_REQUEST, "invalid base64url encoding").into_response();
        }
    };

    if response_bytes.len() != 96 {
        return (
            StatusCode::BAD_REQUEST,
            "challenge_response must be 96 bytes (32 challenge + 64 signature)",
        )
            .into_response();
    }

    let mut challenge = [0u8; 32];
    challenge.copy_from_slice(&response_bytes[..32]);
    let signature = &response_bytes[32..];

    // Consume the challenge from the store
    match state.recovery_challenges.consume_challenge(&body.user_id, &challenge) {
        Ok(crate::user::recovery::ChallengeResult::Valid) => {}
        Ok(crate::user::recovery::ChallengeResult::Expired) => {
            return (StatusCode::GONE, "challenge expired").into_response();
        }
        Ok(crate::user::recovery::ChallengeResult::NotFound) => {
            return (StatusCode::BAD_REQUEST, "unknown challenge").into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    // Verify the signature against the stored recovery pubkey
    match crate::user::recovery::verify_challenge(
        &profile.recovery_pubkey,
        &challenge,
        signature,
    ) {
        Ok(true) => {
            // Issue session and redirect to passkey registration
            let token = state.sessions.create(&body.user_id);
            let cookie = crate::web::session::session_cookie(&token);
            (
                StatusCode::OK,
                [
                    (header::SET_COOKIE, cookie),
                    (header::CONTENT_TYPE, "application/json".to_string()),
                ],
                serde_json::json!({
                    "status": "ok",
                    "redirect": format!("/passkey/register?user_id={}", body.user_id)
                })
                .to_string(),
            )
                .into_response()
        }
        Ok(false) => (StatusCode::UNAUTHORIZED, "invalid signature").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("verification error: {e}"),
        )
            .into_response(),
    }
}

// ── Invitation acceptance flow (REQ-020-007) ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AcceptQuery {
    pub token: Option<String>,
}

/// GET /auth/accept?token=<JWT> — validate invitation and show registration form.
pub async fn accept_invite_handler(
    State(state): State<WebState>,
    Query(query): Query<AcceptQuery>,
) -> Response {
    let vault_root = &*state.vault_root;
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    let token = match query.token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => {
            return (StatusCode::BAD_REQUEST, "missing token parameter").into_response();
        }
    };

    // Decode and verify the JWT
    let claims = match crate::user::invite::decode_jwt(vault_root, token) {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("expired") {
                return (StatusCode::GONE, "invitation has expired").into_response();
            }
            return (StatusCode::BAD_REQUEST, format!("invalid invitation: {msg}"))
                .into_response();
        }
    };

    // Check if nonce has already been consumed (replay → 410 Gone)
    match crate::user::invite::is_nonce_used(vault_root, &claims.nonce) {
        Ok(true) => {
            return (StatusCode::GONE, "invitation has already been used").into_response();
        }
        Ok(false) => {} // good — nonce is fresh
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to check nonce: {e}"),
            )
                .into_response();
        }
    }

    // Look up inviter name for display
    let inviter_display = match crate::user::load_profile(vault_root, &claims.iss) {
        Ok(Some(p)) => p.name,
        _ => claims.iss.clone(),
    };

    match state.engine.render_invite_accept(
        &vault_name,
        token,
        &inviter_display,
        &claims.role,
        claims.pages.as_deref(),
    ) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("{}", e.stderr_line("auth/accept"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AcceptForm {
    pub token: String,
    pub name: String,
}

/// POST /auth/accept — create user profile, inject SPL facts, redirect to recovery flow.
pub async fn accept_invite_submit_handler(
    State(state): State<WebState>,
    axum::Form(form): axum::Form<AcceptForm>,
) -> Response {
    let vault_root = &*state.vault_root;

    // Per-token rate limit on invitation acceptance attempts
    if let Err(retry_after) = state.rate_limiters.invite_per_user.check(&form.token) {
        return crate::web::rate_limit::too_many_requests(retry_after);
    }

    let name = form.name.trim();
    if name.is_empty() || name.len() > 64 {
        return (StatusCode::BAD_REQUEST, "name must be 1-64 characters").into_response();
    }

    // Re-validate JWT (it could have expired between GET and POST)
    let claims = match crate::user::invite::decode_jwt(vault_root, &form.token) {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("expired") {
                return (StatusCode::GONE, "invitation has expired").into_response();
            }
            return (StatusCode::BAD_REQUEST, format!("invalid invitation: {msg}"))
                .into_response();
        }
    };

    // Consume the nonce (single-use enforcement)
    match crate::user::invite::is_nonce_used(vault_root, &claims.nonce) {
        Ok(true) => {
            return (StatusCode::GONE, "invitation has already been used").into_response();
        }
        Ok(false) => {}
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to check nonce: {e}"),
            )
                .into_response();
        }
    }

    // Create the user profile
    let user_id = crate::user::generate_user_id(name);
    let now = {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = secs / 86400;
        let day_secs = secs % 86400;
        let h = day_secs / 3600;
        let m = (day_secs % 3600) / 60;
        let s = day_secs % 60;
        let (y, mo, d) = days_to_ymd(days);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    };

    let profile = crate::user::UserProfile {
        id: user_id.clone(),
        name: name.to_string(),
        created_at: now,
        invited_by: Some(claims.iss.clone()),
        owner: false,
        credentials: vec![],
        recovery_pubkey: String::new(),
        agent_token_generation: 0,
    };

    if let Err(e) = crate::user::save_profile(vault_root, &profile) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create profile: {e}"),
        )
            .into_response();
    }

    // Inject SPL facts into .zetl/collab/access.spl
    if let Err(e) = inject_access_spl(vault_root, &user_id, &claims.role, claims.pages.as_deref())
    {
        eprintln!("warning: failed to write access.spl: {e}");
    }

    // Mark nonce as used (after successful user creation to avoid partial state)
    if let Err(e) = crate::user::invite::mark_nonce_used(vault_root, &claims.nonce, claims.exp) {
        eprintln!("warning: failed to mark nonce as used: {e}");
    }

    // Remove from pending invitations list
    let _ = crate::user::invite::mark_invitation_consumed(vault_root, &claims.nonce);

    // Issue a session for the new user
    let token = state.sessions.create(&user_id);
    let cookie = crate::web::session::session_cookie(&token);

    // Redirect to recovery phrase display (which then chains to passkey registration)
    let location = format!("/recovery/show?user_id={user_id}");
    (
        StatusCode::FOUND,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, location),
        ],
    )
        .into_response()
}

/// Append SPL access facts for an invited user to `.zetl/collab/access.spl`.
fn inject_access_spl(
    vault_root: &std::path::Path,
    user_id: &str,
    role: &str,
    pages: Option<&str>,
) -> anyhow::Result<()> {
    use std::io::Write;

    let collab_dir = vault_root.join(".zetl/collab");
    std::fs::create_dir_all(&collab_dir)?;
    let spl_path = collab_dir.join("access.spl");

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spl_path)?;

    // Write a comment header for this user's facts
    writeln!(file)?;
    writeln!(file, ";; access granted to {user_id} (role: {role})")?;

    let scope = pages.unwrap_or("**");

    match role {
        "admin" => {
            writeln!(file, "(given (can-read {user_id} \"{scope}\"))")?;
            writeln!(file, "(given (can-edit {user_id} \"{scope}\"))")?;
            writeln!(file, "(given (can-admin {user_id} \"{scope}\"))")?;
        }
        "editor" => {
            writeln!(file, "(given (can-read {user_id} \"{scope}\"))")?;
            writeln!(file, "(given (can-edit {user_id} \"{scope}\"))")?;
        }
        _ => {
            // reader or unknown → read-only
            writeln!(file, "(given (can-read {user_id} \"{scope}\"))")?;
        }
    }

    Ok(())
}

// ── Bootstrap owner flow (REQ-020-005) ──────────────────────────────────

/// GET /auth/bootstrap — entry point for owner bootstrap.
///
/// Creates a session for the bootstrap owner and redirects to passkey
/// registration.  The recovery mnemonic is already printed to stderr by the
/// CLI at startup, so this route should NOT redirect to `/recovery/show`
/// (which requires an authenticated session and would regenerate a different
/// key).  The owner profile must already exist (created by `--init-owner`
/// before the server starts).
pub async fn bootstrap_handler(State(state): State<WebState>) -> Response {
    let vault_root = &*state.vault_root;

    // Find the owner profile
    let profiles = match crate::user::list_profiles(vault_root) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list profiles: {e}"),
            )
                .into_response();
        }
    };

    // One-time guard: reject if bootstrap has already been consumed.
    if state
        .bootstrap_used
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return (StatusCode::GONE, "bootstrap has already been used").into_response();
    }

    let owner = profiles.iter().find(|p| p.owner);
    match owner {
        Some(profile) => {
            // Create a bootstrap session so the owner is authenticated for
            // subsequent flows (passkey registration, recovery, etc.).
            let token = state.sessions.create(&profile.id);
            let cookie = format!(
                "zetl_session={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400"
            );
            let location = format!("/passkey/register?user_id={}", profile.id);
            (
                StatusCode::FOUND,
                [
                    (header::LOCATION, location),
                    (header::SET_COOKIE, cookie),
                ],
            )
                .into_response()
        }
        None => (
            StatusCode::CONFLICT,
            "no owner profile found — run with --collab --init-owner first",
        )
            .into_response(),
    }
}

// ── Admin invitation management (/_admin/invite) ────────────────────────

/// GET /_admin/invite — render the admin invitation management page.
pub async fn admin_invite_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionRole,
    headers: axum::http::HeaderMap,
) -> Response {
    // Require Admin role
    if let Err(status) = crate::web::session::require_role(session.role, crate::user::Role::Admin) {
        return status.into_response();
    }

    let vault_root = &*state.vault_root;
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Load pending invitations
    let invitations = match crate::user::invite::load_pending_invitations(vault_root) {
        Ok(invites) => invites,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load invitations: {e}"),
            )
                .into_response();
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Convert to template-friendly JSON values
    let inv_values: Vec<serde_json::Value> = invitations
        .iter()
        .filter(|i| i.exp > now) // only show non-expired
        .map(|i| {
            let remaining = i.exp.saturating_sub(now);
            let expires_display = if remaining > 86400 {
                format!("{} days", remaining / 86400)
            } else if remaining > 3600 {
                format!("{} hours", remaining / 3600)
            } else if remaining > 60 {
                format!("{} minutes", remaining / 60)
            } else {
                "< 1 minute".to_string()
            };
            serde_json::json!({
                "nonce": i.nonce,
                "role": i.role,
                "pages": i.pages,
                "expires_display": expires_display,
                "revoked": i.revoked,
            })
        })
        .collect();

    // Get CSRF token from the session cookie
    let csrf_token = crate::web::session::token_from_cookies(&headers)
        .and_then(|t| state.sessions.csrf_token(&t))
        .unwrap_or_default();

    match state
        .engine
        .render_admin_invite(&vault_name, &csrf_token, &inv_values)
    {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("{}", e.stderr_line("_admin/invite"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub role: String,
    pub pages: Option<String>,
    pub expiry_hours: Option<u64>,
}

/// POST /_admin/invite — generate a new invitation and return the link as JSON.
pub async fn admin_invite_create_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionRole,
    Json(body): Json<CreateInviteRequest>,
) -> Response {
    if let Err(status) = crate::web::session::require_role(session.role, crate::user::Role::Admin) {
        return status.into_response();
    }

    let vault_root = &*state.vault_root;

    // Validate role
    let role = body.role.to_lowercase();
    if !matches!(role.as_str(), "reader" | "editor" | "admin") {
        return (StatusCode::BAD_REQUEST, "invalid role").into_response();
    }

    let pages = body.pages.as_deref().filter(|p| !p.is_empty());
    let expiry_secs = body.expiry_hours.map(|h| h * 3600);

    // Generate the invitation JWT
    let (token, nonce) = match crate::user::invite::generate_invitation(
        vault_root,
        &session.user_id,
        &role,
        pages,
        expiry_secs,
    ) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to generate invitation: {e}"),
            )
                .into_response();
        }
    };

    // Look up inviter display name
    let inviter_name = crate::user::load_profile(vault_root, &session.user_id)
        .ok()
        .flatten()
        .map(|p| p.name)
        .unwrap_or_else(|| session.user_id.clone());

    // Decode claims to get exp
    let claims = crate::user::invite::decode_jwt(vault_root, &token).unwrap();

    // Save pending invitation record
    let pending = crate::user::invite::PendingInvitation {
        nonce: nonce.clone(),
        token: token.clone(),
        inviter_id: session.user_id.clone(),
        inviter_name,
        role: role.clone(),
        pages: pages.map(|s| s.to_string()),
        exp: claims.exp,
        revoked: false,
    };
    if let Err(e) = crate::user::invite::save_pending_invitation(vault_root, &pending) {
        eprintln!("warning: failed to save pending invitation: {e}");
    }

    // Build invite URL (use the Host header or fall back to localhost)
    let invite_url = format!("/auth/accept?token={token}");

    Json(serde_json::json!({
        "invite_url": invite_url,
        "nonce": nonce,
        "role": role,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct RevokeInviteRequest {
    pub nonce: String,
}

/// POST /_admin/invite/revoke — revoke a pending invitation.
pub async fn admin_invite_revoke_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionRole,
    Json(body): Json<RevokeInviteRequest>,
) -> Response {
    if let Err(status) = crate::web::session::require_role(session.role, crate::user::Role::Admin) {
        return status.into_response();
    }

    let vault_root = &*state.vault_root;

    match crate::user::invite::revoke_invitation(vault_root, &body.nonce) {
        Ok(true) => Json(serde_json::json!({"ok": true})).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "invitation not found").into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to revoke invitation: {e}"),
        )
            .into_response(),
    }
}

// ── Admin permissions management (/_admin/permissions) ──────────────────

/// Parse role/scope assignments from access.spl content.
///
/// Returns a map of user_id → (role, Option<scope>).
fn parse_access_spl_assignments(
    content: &str,
) -> std::collections::HashMap<String, (String, Option<String>)> {
    let mut roles: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut scopes: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        // Parse: (given (role "user-id" editor))
        if let Some(rest) = trimmed.strip_prefix("(given (role ") {
            if let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                if let Some(end) = after.find('"') {
                    let user_id = &after[..end];
                    let remaining = after[end + 1..].trim();
                    // remaining is like: editor))
                    let role = remaining.trim_end_matches(')').trim();
                    if !role.is_empty() {
                        roles.insert(user_id.to_string(), role.to_string());
                    }
                }
            }
        }
        // Parse: (given (scope "user-id" "pattern"))
        if let Some(rest) = trimmed.strip_prefix("(given (scope ") {
            if let Some(start) = rest.find('"') {
                let after = &rest[start + 1..];
                if let Some(end) = after.find('"') {
                    let user_id = &after[..end];
                    let remaining = &after[end + 1..];
                    if let Some(s2) = remaining.find('"') {
                        let after_s2 = &remaining[s2 + 1..];
                        if let Some(e2) = after_s2.find('"') {
                            let scope = &after_s2[..e2];
                            scopes.insert(user_id.to_string(), scope.to_string());
                        }
                    }
                }
            }
        }
    }

    let mut result = std::collections::HashMap::new();
    for (user_id, role) in &roles {
        let scope = scopes.get(user_id).cloned();
        result.insert(user_id.clone(), (role.clone(), scope));
    }
    // Include users with scopes but no explicit role entry
    for (user_id, scope) in &scopes {
        result
            .entry(user_id.clone())
            .or_insert_with(|| ("editor".to_string(), Some(scope.clone())));
    }
    result
}

/// Generate access.spl content from a list of permission entries.
fn generate_access_spl(permissions: &[PermissionEntry], visibility_mode: Option<&str>) -> String {
    let mut lines = vec![
        "; Vault access policy (access.spl)".to_string(),
        "; Generated by /_admin/permissions".to_string(),
        String::new(),
    ];

    for p in permissions {
        lines.push(format!("(given (role \"{}\" {}))", p.user_id, p.role));
        if let Some(ref scope) = p.scope {
            if !scope.is_empty() {
                lines.push(format!("(given (scope \"{}\" \"{}\"))", p.user_id, scope));
            }
        }
        if p.role == "admin" {
            lines.push(format!("(given (admin \"{}\"))", p.user_id));
        }
    }

    // REQ-020-054: persist visibility mode if explicitly set
    if let Some(mode) = visibility_mode {
        if matches!(mode, "transparent" | "hidden" | "mixed") {
            lines.push(String::new());
            lines.push(format!("(given (visibility-mode {mode}))"));
        }
    }

    lines.join("\n") + "\n"
}

/// GET /_admin/permissions — render the admin permissions management page.
pub async fn admin_permissions_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionRole,
    headers: axum::http::HeaderMap,
) -> Response {
    if let Err(status) = crate::web::session::require_role(session.role, crate::user::Role::Admin) {
        return status.into_response();
    }

    let vault_root = &*state.vault_root;
    let vault_name = state
        .vault_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".to_string());

    // Load all user profiles
    let profiles = match crate::user::list_profiles(vault_root) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load user profiles: {e}"),
            )
                .into_response();
        }
    };

    // Load current access.spl assignments
    let access_path = vault_root.join(".zetl/collab/access.spl");
    let access_content = std::fs::read_to_string(&access_path).unwrap_or_default();
    let assignments = parse_access_spl_assignments(&access_content);

    // Build template data
    let users: Vec<serde_json::Value> = profiles
        .iter()
        .map(|p| {
            let (role, scope) = if p.owner {
                ("admin".to_string(), None)
            } else {
                assignments
                    .get(&p.id)
                    .cloned()
                    .unwrap_or_else(|| ("editor".to_string(), None))
            };
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "owner": p.owner,
                "role": role,
                "scope": scope,
            })
        })
        .collect();

    // SPL preview: current file content or generated from current state
    let spl_preview = if access_content.trim().is_empty() {
        let entries: Vec<PermissionEntry> = profiles
            .iter()
            .filter(|p| !p.owner)
            .map(|p| {
                let (role, scope) = assignments
                    .get(&p.id)
                    .cloned()
                    .unwrap_or_else(|| ("editor".to_string(), None));
                PermissionEntry {
                    user_id: p.id.clone(),
                    role,
                    scope,
                }
            })
            .collect();
        generate_access_spl(&entries, None)
    } else {
        access_content
    };

    let csrf_token = crate::web::session::token_from_cookies(&headers)
        .and_then(|t| state.sessions.csrf_token(&t))
        .unwrap_or_default();

    match state
        .engine
        .render_admin_permissions(&vault_name, &csrf_token, &users, &spl_preview)
    {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("{}", e.stderr_line("_admin/permissions"));
            (StatusCode::INTERNAL_SERVER_ERROR, Html(e.to_error_html())).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PermissionEntry {
    pub user_id: String,
    pub role: String,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SavePermissionsRequest {
    pub permissions: Option<Vec<PermissionEntry>>,
    pub remove_user: Option<String>,
    /// Optional visibility mode override (REQ-020-054).
    pub visibility_mode: Option<String>,
}

/// POST /_admin/permissions — save permission changes or remove a user.
pub async fn admin_permissions_save_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionRole,
    Json(body): Json<SavePermissionsRequest>,
) -> Response {
    if let Err(status) = crate::web::session::require_role(session.role, crate::user::Role::Admin) {
        return status.into_response();
    }

    let vault_root = &*state.vault_root;

    // Handle user removal
    if let Some(ref user_id) = body.remove_user {
        // Don't allow removing yourself
        if user_id == &session.user_id {
            return (StatusCode::BAD_REQUEST, "cannot remove yourself").into_response();
        }

        // Check the user exists and isn't the owner
        match crate::user::load_profile(vault_root, user_id) {
            Ok(Some(profile)) => {
                if profile.owner {
                    return (StatusCode::BAD_REQUEST, "cannot remove the vault owner")
                        .into_response();
                }
            }
            Ok(None) => {
                return (StatusCode::NOT_FOUND, "user not found").into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to load profile: {e}"),
                )
                    .into_response();
            }
        }

        // Delete profile
        if let Err(e) = crate::user::delete_profile(vault_root, user_id) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to delete user: {e}"),
            )
                .into_response();
        }

        // Remove from access.spl
        let access_path = vault_root.join(".zetl/collab/access.spl");
        let content = std::fs::read_to_string(&access_path).unwrap_or_default();
        let new_content: String = content
            .lines()
            .filter(|line| !line.contains(&format!("\"{}\"", user_id)))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let _ = std::fs::write(&access_path, &new_content);

        // Auto-commit
        if let Some(ref lock) = state.git_commit_lock {
            if let Ok(repo) = lock.lock() {
                let rel_path = std::path::Path::new(".zetl/collab/access.spl");
                let user_name = crate::user::load_profile(vault_root, &session.user_id)
                    .ok()
                    .flatten()
                    .map(|p| p.name)
                    .unwrap_or_else(|| session.user_id.clone());
                let _ = crate::web::git_commit::auto_commit(
                    &repo,
                    rel_path,
                    &user_name,
                    &session.user_id,
                    Some(&format!("permissions: remove user {}", user_id)),
                );
            }
        }

        return Json(serde_json::json!({"ok": true})).into_response();
    }

    // Handle permission updates
    let permissions = match body.permissions {
        Some(p) => p,
        None => return (StatusCode::BAD_REQUEST, "missing permissions or remove_user").into_response(),
    };

    // Validate roles
    for p in &permissions {
        if !matches!(p.role.as_str(), "reader" | "editor" | "admin") {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid role '{}' for user {}", p.role, p.user_id),
            )
                .into_response();
        }
    }

    // REQ-020-054: capture old visibility mode before writing new access.spl
    #[cfg(feature = "reason")]
    let old_mode = crate::acl::query_visibility_mode(vault_root);

    // Generate and write access.spl
    let spl_content = generate_access_spl(&permissions, body.visibility_mode.as_deref());
    let access_path = vault_root.join(".zetl/collab/access.spl");

    // Ensure the directory exists
    if let Some(parent) = access_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Err(e) = std::fs::write(&access_path, &spl_content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to write access.spl: {e}"),
        )
            .into_response();
    }

    // REQ-020-054: broadcast visibility mode change to all connected WebSocket clients
    #[cfg(feature = "reason")]
    {
        let new_mode = crate::acl::query_visibility_mode(vault_root);
        if old_mode != new_mode {
            let changer_name = crate::user::load_profile(vault_root, &session.user_id)
                .ok()
                .flatten()
                .map(|p| p.name)
                .unwrap_or_else(|| session.user_id.clone());
            state.ws_hub.broadcast_all(crate::web::ws::ServerMsg::VisibilityModeChanged {
                old_mode: format!("{old_mode:?}").to_lowercase(),
                new_mode: format!("{new_mode:?}").to_lowercase(),
                changed_by: changer_name,
            });
        }
    }

    // Auto-commit the change
    if let Some(ref lock) = state.git_commit_lock {
        if let Ok(repo) = lock.lock() {
            let rel_path = std::path::Path::new(".zetl/collab/access.spl");
            let user_name = crate::user::load_profile(vault_root, &session.user_id)
                .ok()
                .flatten()
                .map(|p| p.name)
                .unwrap_or_else(|| session.user_id.clone());
            let _ = crate::web::git_commit::auto_commit(
                &repo,
                rel_path,
                &user_name,
                &session.user_id,
                Some("permissions: update access policy"),
            );
        }
    }

    Json(serde_json::json!({
        "ok": true,
        "spl_preview": spl_content,
    }))
    .into_response()
}

// ── Agent API (CON-020-007, REQ-020-017, REQ-020-018) ─────────────────────────

/// Standard JSON envelope for all `/api/*` agent endpoints (CON-020-007).
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proof: Vec<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Json<ApiResponse<T>> {
        Json(ApiResponse {
            ok: true,
            data: Some(data),
            error: None,
        })
    }
}

impl ApiResponse<()> {
    pub fn err(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
        let body: ApiResponse<()> = ApiResponse {
            ok: false,
            data: None,
            error: Some(ApiError {
                code,
                message: message.into(),
                proof: Vec::new(),
            }),
        };
        (status, Json(body)).into_response()
    }

    #[cfg(feature = "reason")]
    pub fn err_with_proof(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        proof: Vec<String>,
    ) -> Response {
        let body: ApiResponse<()> = ApiResponse {
            ok: false,
            data: None,
            error: Some(ApiError {
                code,
                message: message.into(),
                proof,
            }),
        };
        (status, Json(body)).into_response()
    }
}

/// Resolve the authenticated user ID from an `AuthUser` extractor result,
/// returning an API error response if authentication fails.
/// Extract the session user_id from cookies or bearer token (non-failing).
///
/// Returns `None` if no valid session is found. Used by page_handler and
/// other HTML-serving routes that need the user_id for ACL checks without
/// returning an API error.
fn extract_session_user_id(state: &WebState, headers: &axum::http::HeaderMap) -> Option<String> {
    // Try session cookie first
    if let Some(token) = crate::web::session::token_from_cookies(headers) {
        if let Some(user_id) = state.sessions.validate(&token) {
            return Some(user_id);
        }
    }
    // Try Bearer token
    if let Some(token) = crate::web::session::bearer_token_from_headers(headers) {
        if let Some(user_id) = crate::web::session::verify_bearer_token(&state.vault_root, &token)
        {
            return Some(user_id);
        }
    }
    None
}

fn require_auth(
    state: &WebState,
    headers: &axum::http::HeaderMap,
) -> Result<(String, bool), Response> {
    // Try session cookie first
    if let Some(token) = crate::web::session::token_from_cookies(headers) {
        if let Some(user_id) = state.sessions.validate(&token) {
            return Ok((user_id, false));
        }
    }
    // Try Bearer token
    if let Some(token) = crate::web::session::bearer_token_from_headers(headers) {
        if let Some(user_id) = crate::web::session::verify_bearer_token(&state.vault_root, &token)
        {
            return Ok((user_id, true));
        }
    }
    Err(ApiResponse::err(
        StatusCode::UNAUTHORIZED,
        "INVALID_TOKEN",
        "valid session or agent token required",
    ))
}

// ── GET /api/pages — list all pages (REQ-020-018) ─────────────────────────

#[derive(Serialize)]
struct ApiPageEntry {
    name: String,
    slug: String,
}

pub async fn api_pages_list_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if state.collab {
        if let Err(resp) = require_auth(&state, &headers) {
            return resp;
        }
    }

    let data = state.data.read().unwrap();

    // Build denied page set for filtering (REQ-020-031)
    #[cfg(feature = "reason")]
    let denied_names: HashSet<String> = if state.collab {
        if let Some(ref uid) = extract_session_user_id(&state, &headers) {
            let denied = build_denied_pages_map(&state, &data, uid);
            denied.keys().cloned().collect()
        } else {
            HashSet::new()
        }
    } else {
        HashSet::new()
    };

    let pages: Vec<ApiPageEntry> = data
        .page_names
        .iter()
        .filter(|name| {
            #[cfg(feature = "reason")]
            {
                !denied_names.contains(name.as_str())
            }
            #[cfg(not(feature = "reason"))]
            {
                let _ = name;
                true
            }
        })
        .map(|name| {
            let slug = data.slug_for_page(name);
            ApiPageEntry {
                name: name.clone(),
                slug,
            }
        })
        .collect();

    ApiResponse::ok(pages).into_response()
}

// ── GET /api/pages/{*slug} — get page content as markdown (REQ-020-018) ───

#[derive(Serialize)]
struct ApiPageContent {
    name: String,
    slug: String,
    content: String,
    forward_links: Vec<String>,
    backlinks: Vec<String>,
}

pub async fn api_pages_get_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    if state.collab {
        if let Err(resp) = require_auth(&state, &headers) {
            return resp;
        }
    }

    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');
    let data = state.data.read().unwrap();

    let file = data
        .files
        .iter()
        .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));

    // ACL check for API page access (REQ-020-030, REQ-020-033)
    #[cfg(feature = "reason")]
    if state.collab {
        if let (Some(f), Some(ref uid)) = (file, extract_session_user_id(&state, &headers)) {
            let page_slug_str = page_slug_from_path(&f.path);
            let is_agent = crate::web::session::bearer_token_from_headers(&headers)
                .and_then(|t| crate::web::session::verify_bearer_token(&state.vault_root, &t))
                .is_some();
            let all_slugs: Vec<String> = data.files.iter().map(|f2| page_slug_from_path(&f2.path)).collect();
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
            let query = crate::acl::AclQuery {
                user_id: uid.clone(),
                page_slug: page_slug_str.clone(),
                action: crate::acl::Action::Read,
                is_agent,
                now_epoch_ms: now_ms,
            };
            let decision = {
                let cache = state.acl_cache.lock().unwrap();
                if let Some(cached) = cache.lookup(uid, &page_slug_str, crate::acl::Action::Read) {
                    cached.clone()
                } else {
                    drop(cache);
                    match crate::acl::evaluate(&state.vault_root, &query, &f.spl_blocks, &all_slugs) {
                        Ok(d) => {
                            let mut cache = state.acl_cache.lock().unwrap();
                            cache.insert(uid.clone(), page_slug_str.clone(), crate::acl::Action::Read, d.clone());
                            d
                        }
                        Err(_) => crate::acl::AclDecision::Denied {
                            tag: crate::acl::ConclusionTag::DefeasiblyNotProvable,
                            rule_trace: vec![],
                        },
                    }
                }
            };
            if !decision.is_allowed() {
                let vis_mode = crate::acl::query_visibility_mode(&state.vault_root);
                let effective = crate::acl::effective_visibility(vis_mode, crate::acl::PageVisibilityOverride::None);
                return if effective == crate::acl::VisibilityMode::Hidden {
                    ApiResponse::err(StatusCode::NOT_FOUND, "NOT_FOUND", format!("page not found: {slug}"))
                } else {
                    ApiResponse::err(StatusCode::FORBIDDEN, "ACL_DENIED", "you don't have access to this page")
                };
            }
        }
    }

    let file = match file {
        Some(f) => f,
        None => {
            return ApiResponse::err(
                StatusCode::NOT_FOUND,
                "NOT_FOUND",
                format!("page not found: {slug}"),
            );
        }
    };

    let full_path = state.vault_root.join(&file.path);
    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => {
            return ApiResponse::err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                format!("failed to read page: {e}"),
            );
        }
    };

    let page_name = &file.page_name;
    let forward: Vec<String> = data
        .graph
        .forward_links(page_name)
        .iter()
        .map(|l| l.target.clone())
        .collect();
    let back: Vec<String> = data
        .graph
        .backlinks(page_name)
        .iter()
        .map(|b| b.source.clone())
        .collect();

    let file_slug = page_slug_from_path(&file.path);

    let mut resp = ApiResponse::ok(ApiPageContent {
        name: page_name.clone(),
        slug: file_slug.clone(),
        content,
        forward_links: forward,
        backlinks: back,
    })
    .into_response();

    // Inject CRDT-aware headers when the document is loaded in the CRDT store.
    if let Some(meta) = state.crdt_store.crdt_meta(&file_slug) {
        let hdrs = resp.headers_mut();
        hdrs.insert(
            HeaderName::from_static("x-crdt-dirty"),
            meta.dirty.to_string().parse().unwrap(),
        );
        hdrs.insert(
            HeaderName::from_static("x-crdt-last-flush"),
            meta.secs_since_flush.to_string().parse().unwrap(),
        );
        hdrs.insert(
            HeaderName::from_static("x-crdt-clients"),
            meta.client_count.to_string().parse().unwrap(),
        );
        if let Some(ref hash) = meta.content_hash {
            hdrs.insert(
                HeaderName::from_static("x-crdt-hash"),
                hash.parse().unwrap(),
            );
        }
    }

    resp
}

// ── PUT /api/pages/{*slug} — create or edit page (REQ-020-017) ────────────

pub async fn api_pages_put_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
    body: String,
) -> Response {
    let (user_id, _is_agent) = if state.collab {
        match require_auth(&state, &headers) {
            Ok(auth) => auth,
            Err(resp) => return resp,
        }
    } else {
        ("zetl".to_string(), false)
    };

    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // In collab mode, verify the user has edit permission on this page.
    #[cfg(feature = "reason")]
    if state.collab {
        if let Err(resp) = check_page_acl_edit(&state, &user_id, slug) {
            return resp;
        }
    }

    let x_create = headers
        .get("X-Create")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    // Resolve file path
    let (full_path, is_new) = {
        let data = state.data.read().unwrap();
        let file = data
            .files
            .iter()
            .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));
        if let Some(file) = file {
            // REQ-020-054: hidden-mode collision warning — if the user thinks they're
            // creating a new page (X-Create) but a hidden/denied page already exists,
            // warn instead of silently overwriting.
            #[cfg(feature = "reason")]
            if x_create && state.collab {
                let vis_mode = crate::acl::query_visibility_mode(&state.vault_root);
                if matches!(vis_mode, crate::acl::VisibilityMode::Hidden) {
                    let all_slugs: Vec<String> = data.files.iter()
                        .map(|f| page_slug_from_path(&f.path))
                        .collect();
                    let q = crate::acl::AclQuery {
                        user_id: user_id.clone(),
                        page_slug: slug.to_string(),
                        action: crate::acl::Action::Read,
                        is_agent: false,
                        now_epoch_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                    };
                    if let Ok(decision) = crate::acl::evaluate(&state.vault_root, &q, &[], &all_slugs) {
                        if !decision.is_allowed() {
                            return ApiResponse::err(
                                StatusCode::CONFLICT,
                                "HIDDEN_PAGE_COLLISION",
                                "A restricted page with this name already exists. Contact an admin.",
                            );
                        }
                    }
                }
            }
            (state.vault_root.join(&file.path), false)
        } else {
            // X-Create header required for new page creation
            if !x_create {
                return ApiResponse::err(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    format!(
                        "page not found: {slug} (set X-Create: true header to create a new page)"
                    ),
                );
            }
            (state.vault_root.join(format!("{slug}.md")), true)
        }
    };

    // ── CRDT conflict detection (If-Match / X-CRDT-Hash) ───────────────
    // When the document has an active CRDT session with dirty edits, an API
    // PUT must prove it has seen the latest CRDT state.  The client passes
    // the `X-CRDT-Hash` (or `If-Match`) header received from a prior GET;
    // if it doesn't match the live CRDT hash, the write is rejected with 409.
    if let Some(meta) = state.crdt_store.crdt_meta(slug) {
        if meta.dirty {
            let client_hash = headers
                .get("if-match")
                .or_else(|| headers.get("x-crdt-hash"))
                .and_then(|v| v.to_str().ok());
            match (client_hash, &meta.content_hash) {
                (Some(client), Some(live)) if client == live => { /* match — proceed */ }
                (None, _) => {
                    return ApiResponse::err(
                        StatusCode::CONFLICT,
                        "CRDT_CONFLICT",
                        "page has active CRDT edits; include X-CRDT-Hash or If-Match header from GET",
                    );
                }
                (Some(_), _) => {
                    return ApiResponse::err(
                        StatusCode::CONFLICT,
                        "CRDT_CONFLICT",
                        "CRDT content has changed since your last read; re-GET to obtain the latest hash",
                    );
                }
            }
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = full_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ApiResponse::err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                format!("cannot create directory: {e}"),
            );
        }
    }

    if let Err(e) = std::fs::write(&full_path, &body) {
        return ApiResponse::err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            format!("write failed: {e}"),
        );
    }

    // Git auto-commit
    if let Some(ref lock) = state.git_commit_lock {
        let rel_path = full_path
            .strip_prefix(state.vault_root.as_ref())
            .unwrap_or(&full_path)
            .to_path_buf();

        let (author_name, author_id) =
            match crate::user::load_profile(&state.vault_root, &user_id) {
                Ok(Some(profile)) => (profile.name.clone(), profile.id.clone()),
                _ => ("zetl".to_string(), "zetl".to_string()),
            };

        let custom_message = headers
            .get("X-Commit-Message")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if let Ok(repo) = lock.lock() {
            match super::git_commit::auto_commit(
                &repo,
                &rel_path,
                &author_name,
                &author_id,
                custom_message.as_deref(),
            ) {
                Ok(_) => super::git_commit::jj_git_import(&state.vault_root),
                Err(e) => eprintln!("warning: git auto-commit failed: {e}"),
            }
        }
    }

    // Re-index
    match reindex(&state.vault_root) {
        Ok(new_data) => {
            let _ = SearchIndex::build(&state.vault_root, &new_data.files);
            *state.data.write().unwrap() = new_data;
        }
        Err(e) => eprintln!("reindex error: {e}"),
    }

    let status = if is_new {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    let mut resp = (status, ApiResponse::ok(serde_json::json!({ "slug": slug }))).into_response();

    // Inject CRDT headers on the response so clients can track state.
    if let Some(meta) = state.crdt_store.crdt_meta(slug) {
        let hdrs = resp.headers_mut();
        hdrs.insert(
            HeaderName::from_static("x-crdt-dirty"),
            meta.dirty.to_string().parse().unwrap(),
        );
        hdrs.insert(
            HeaderName::from_static("x-crdt-last-flush"),
            meta.secs_since_flush.to_string().parse().unwrap(),
        );
        hdrs.insert(
            HeaderName::from_static("x-crdt-clients"),
            meta.client_count.to_string().parse().unwrap(),
        );
        if let Some(ref hash) = meta.content_hash {
            hdrs.insert(
                HeaderName::from_static("x-crdt-hash"),
                hash.parse().unwrap(),
            );
        }
    }

    resp
}

// ── DELETE /api/pages/{*slug} — delete page ───────────────────────────────

pub async fn api_pages_delete_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let (user_id, _is_agent) = if state.collab {
        match require_auth(&state, &headers) {
            Ok(auth) => auth,
            Err(resp) => return resp,
        }
    } else {
        ("zetl".to_string(), false)
    };

    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // In collab mode, verify the user has edit permission on this page.
    #[cfg(feature = "reason")]
    if state.collab {
        if let Err(resp) = check_page_acl_edit(&state, &user_id, slug) {
            return resp;
        }
    }

    let full_path = {
        let data = state.data.read().unwrap();
        let file = data
            .files
            .iter()
            .find(|f| page_slug_from_path(&f.path).eq_ignore_ascii_case(slug));
        match file {
            Some(f) => state.vault_root.join(&f.path),
            None => {
                return ApiResponse::err(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    format!("page not found: {slug}"),
                );
            }
        }
    };

    if let Err(e) = std::fs::remove_file(&full_path) {
        return ApiResponse::err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            format!("delete failed: {e}"),
        );
    }

    // Git auto-commit the deletion
    if let Some(ref lock) = state.git_commit_lock {
        let rel_path = full_path
            .strip_prefix(state.vault_root.as_ref())
            .unwrap_or(&full_path)
            .to_path_buf();

        let (author_name, author_id) =
            match crate::user::load_profile(&state.vault_root, &user_id) {
                Ok(Some(profile)) => (profile.name.clone(), profile.id.clone()),
                _ => ("zetl".to_string(), "zetl".to_string()),
            };

        let page_name = slug.rsplit('/').next().unwrap_or(slug);
        let commit_msg = format!("delete: {page_name}");

        if let Ok(repo) = lock.lock() {
            match super::git_commit::auto_commit(
                &repo,
                &rel_path,
                &author_name,
                &author_id,
                Some(&commit_msg),
            ) {
                Ok(_) => super::git_commit::jj_git_import(&state.vault_root),
                Err(e) => eprintln!("warning: git auto-commit failed: {e}"),
            }
        }
    }

    // Re-index
    match reindex(&state.vault_root) {
        Ok(new_data) => {
            let _ = SearchIndex::build(&state.vault_root, &new_data.files);
            *state.data.write().unwrap() = new_data;
        }
        Err(e) => eprintln!("reindex error: {e}"),
    }

    ApiResponse::ok(serde_json::json!({ "deleted": slug })).into_response()
}

// ── GET /api/graph — export link graph as JSON (REQ-020-018) ──────────────

#[derive(Serialize)]
struct ApiGraphNode {
    name: String,
    slug: String,
    is_real: bool,
    /// Whether this node represents a restricted page (REQ-020-032).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    locked: bool,
}

#[derive(Serialize)]
struct ApiGraphEdge {
    source: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    is_embed: bool,
}

#[derive(Serialize)]
struct ApiGraph {
    nodes: Vec<ApiGraphNode>,
    edges: Vec<ApiGraphEdge>,
    stats: crate::graph::GraphStats,
}

pub async fn api_graph_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if state.collab {
        if let Err(resp) = require_auth(&state, &headers) {
            return resp;
        }
    }

    let data = state.data.read().unwrap();
    let graph = &data.graph;

    // Build denied pages map for graph filtering (REQ-020-032).
    #[cfg(feature = "reason")]
    let (denied_pages, vis_mode) = if state.collab {
        if let Some(ref uid) = extract_session_user_id(&state, &headers) {
            let denied = build_denied_pages_map(&state, &data, uid);
            let mode = crate::acl::query_visibility_mode(&state.vault_root);
            (denied, mode)
        } else {
            (std::collections::HashMap::new(), crate::acl::VisibilityMode::Mixed)
        }
    } else {
        (std::collections::HashMap::new(), crate::acl::VisibilityMode::Mixed)
    };

    let nodes: Vec<ApiGraphNode> = graph
        .node_map
        .iter()
        .filter_map(|(name, _idx)| {
            let slug = data.slug_for_page(name);
            let is_real = graph.resolved.contains(name);

            #[cfg(feature = "reason")]
            {
                if let Some(_style) = denied_pages.get(name) {
                    // In hidden mode: omit denied nodes entirely
                    if vis_mode == crate::acl::VisibilityMode::Hidden {
                        return None;
                    }
                    // In mixed/transparent mode: show as "(restricted)" with locked flag
                    return Some(ApiGraphNode {
                        name: "(restricted)".to_string(),
                        slug,
                        is_real,
                        locked: true,
                    });
                }
            }

            Some(ApiGraphNode {
                name: name.clone(),
                slug,
                is_real,
                locked: false,
            })
        })
        .collect();

    // Collect denied page names for edge filtering
    #[cfg(feature = "reason")]
    let denied_set: HashSet<&str> = denied_pages.keys().map(|s| s.as_str()).collect();

    let edges: Vec<ApiGraphEdge> = graph
        .graph
        .edge_indices()
        .filter_map(|ei| {
            let (src_idx, tgt_idx) = graph.graph.edge_endpoints(ei)?;
            let src_name = graph.graph.node_weight(src_idx)?;
            let tgt_name = graph.graph.node_weight(tgt_idx)?;
            let meta = graph.graph.edge_weight(ei)?;

            // In hidden mode: omit edges to/from denied nodes (REQ-020-032)
            #[cfg(feature = "reason")]
            if vis_mode == crate::acl::VisibilityMode::Hidden {
                if denied_set.contains(src_name.as_str()) || denied_set.contains(tgt_name.as_str()) {
                    return None;
                }
            }

            Some(ApiGraphEdge {
                source: src_name.clone(),
                target: tgt_name.clone(),
                alias: meta.alias.clone(),
                is_embed: meta.is_embed,
            })
        })
        .collect();

    let stats = graph.stats(10);

    ApiResponse::ok(ApiGraph {
        nodes,
        edges,
        stats,
    })
    .into_response()
}

// ── POST /api/index — trigger re-index (REQ-020-018) ─────────────────────

pub async fn api_index_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if state.collab {
        if let Err(resp) = require_auth(&state, &headers) {
            return resp;
        }
    }

    match reindex(&state.vault_root) {
        Ok(new_data) => {
            let page_count = new_data.page_names.len();
            let link_count = new_data.graph.graph.edge_count();
            let _ = SearchIndex::build(&state.vault_root, &new_data.files);
            *state.data.write().unwrap() = new_data;
            ApiResponse::ok(serde_json::json!({
                "pages": page_count,
                "links": link_count,
            }))
            .into_response()
        }
        Err(e) => ApiResponse::err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            format!("reindex failed: {e}"),
        ),
    }
}

// ── POST /api/ws/ticket — issue a one-time WS auth ticket (REQ-020-028) ──

pub async fn ws_ticket_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
) -> Response {
    if state.collab {
        match require_auth(&state, &headers) {
            Ok((user_id, _)) => {
                let ticket = state.ticket_store.issue(&user_id);
                ApiResponse::ok(serde_json::json!({ "ticket": ticket })).into_response()
            }
            Err(resp) => resp,
        }
    } else {
        let ticket = state.ticket_store.issue("anonymous");
        ApiResponse::ok(serde_json::json!({ "ticket": ticket })).into_response()
    }
}

// ── POST /api/access-request — request access to a denied page (REQ-020-047) ──

#[derive(Deserialize)]
pub struct AccessRequestBody {
    pub page: String,
}

/// POST /api/access-request — authenticated user requests access to a page.
pub async fn access_request_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionUser,
    Json(body): Json<AccessRequestBody>,
) -> Response {
    let vault_root = &*state.vault_root;
    let page_slug = body.page;

    // Load requesting user's profile
    let profile = match crate::user::load_profile(vault_root, &session.user_id) {
        Ok(Some(p)) => p,
        _ => {
            return (StatusCode::BAD_REQUEST, "user profile not found").into_response();
        }
    };

    // Append the access request
    match crate::user::access_request::append_access_request(
        vault_root,
        &session.user_id,
        &profile.name,
        &page_slug,
    ) {
        Ok(true) => {
            // Broadcast to all connected WebSocket clients (admins will see it)
            state.ws_hub.broadcast_all(crate::web::ws::ServerMsg::AccessRequest {
                user: profile.name.clone(),
                page: page_slug.clone(),
            });

            // Fire on-access-request hook asynchronously
            let vault_root_owned = state.vault_root.clone();
            let theme = state.theme.clone();
            let user_id = session.user_id.clone();
            let user_name = profile.name.clone();
            let page = page_slug.clone();
            tokio::task::spawn_blocking(move || {
                fire_access_request_hook(
                    &vault_root_owned,
                    &theme,
                    &user_id,
                    &user_name,
                    &page,
                );
            });

            ApiResponse::ok(serde_json::json!({ "status": "requested" })).into_response()
        }
        Ok(false) => {
            // Already pending
            ApiResponse::ok(serde_json::json!({ "status": "already_pending" })).into_response()
        }
        Err(e) => {
            eprintln!("access-request error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to record request").into_response()
        }
    }
}

/// Fire the on-access-request hook (blocking, called from spawn_blocking).
fn fire_access_request_hook(
    vault_root: &std::path::Path,
    theme: &str,
    user_id: &str,
    user_name: &str,
    page_slug: &str,
) {
    let theme_hooks = hooks::resolve_theme_hooks(vault_root, theme);
    let manifest = hooks::discover_hooks(vault_root, theme_hooks.path());

    if hooks::hooks_for(&manifest, "on-access-request").is_empty() {
        return;
    }

    let files = match crate::scanner::scan_vault(vault_root, &[]) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("on-access-request hook: scan error: {e}");
            return;
        }
    };
    let file_index: Vec<(String, std::path::PathBuf)> = files
        .iter()
        .map(|f| (f.page_name.clone(), f.path.clone()))
        .collect();
    let mut resolved = std::collections::HashMap::new();
    for file in &files {
        for link in &file.links {
            let key = link.raw_target.clone();
            if !resolved.contains_key(&key) {
                if let Some(r) =
                    crate::scanner::resolve_page_name(&link.target_page, &file_index)
                {
                    resolved.insert(key, r);
                }
            }
        }
    }
    let graph = crate::graph::LinkGraph::build(&files, &resolved);

    let mut ctx = build_hook_context(
        "on-access-request",
        vault_root,
        theme,
        env!("CARGO_PKG_VERSION"),
        &files,
        &graph,
    );

    // Attach requesting user identity
    if let Ok(Some(profile)) = crate::user::load_profile(vault_root, user_id) {
        ctx.user = Some(crate::hooks::context::HookUser::from_profile(&profile, false, vault_root));
    }

    // Attach access request context
    ctx.access_request = Some(crate::hooks::context::HookAccessRequest {
        user_id: user_id.to_string(),
        user_name: user_name.to_string(),
        page: page_slug.to_string(),
        requested_at: crate::user::access_request::now_iso8601(),
    });

    let context_json = match serde_json::to_vec(&ctx) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("on-access-request hook: json error: {e}");
            return;
        }
    };

    let hook_env = hooks::HookEnv {
        vault_root: vault_root.to_path_buf(),
        theme: theme.to_string(),
        zetl_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_vars: vec![
            ("ZETL_ACCESS_REQUEST_USER".into(), user_id.to_string()),
            ("ZETL_ACCESS_REQUEST_PAGE".into(), page_slug.to_string()),
            ("ZETL_HOOK_DEPTH".into(), "0".into()),
        ],
    };

    let results = hooks::run_hooks(&manifest, "on-access-request", &context_json, &hook_env);
    for result in results {
        match result {
            Ok(output) if !output.success() => {
                eprintln!(
                    "warning: on-access-request hook '{}' ({}) exited with code {}",
                    output.path.display(),
                    output.source,
                    output.exit_code.unwrap_or(-1),
                );
                if !output.stderr.is_empty() {
                    eprintln!("  stderr: {}", output.stderr.trim_end());
                }
            }
            Err(e) => {
                eprintln!("warning: on-access-request hook failed to execute: {e}");
            }
            _ => {}
        }
    }
}

// ── GET /api/comments/{slug} — list comments for a page (REQ-020-051) ────

pub async fn api_comments_get_handler(
    State(state): State<WebState>,
    _session: crate::web::session::SessionUser,
    Path(slug): Path<String>,
) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // ACL: user must be able to read the page
    #[cfg(feature = "reason")]
    if state.collab {
        if let Err(resp) = check_page_acl_read(&state, &_session.user_id, slug) {
            return resp;
        }
    }

    // Load server key for HMAC verification (REQ-020-066)
    let server_key = match crate::user::invite::load_or_create_server_key(&state.vault_root) {
        Ok(k) => k.to_bytes().to_vec(),
        Err(e) => {
            eprintln!("error loading server key for comment verification: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to verify comments").into_response();
        }
    };

    match crate::user::comment::load_comments(&state.vault_root, slug) {
        Ok(comments) => {
            let verified = crate::user::comment::verify_comments(&server_key, &comments);
            ApiResponse::ok(serde_json::json!({ "comments": verified })).into_response()
        }
        Err(e) => {
            eprintln!("error loading comments for {slug}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to load comments").into_response()
        }
    }
}

// ── POST /api/comments/{slug} — add a comment to a page (REQ-020-051) ───

#[derive(Deserialize)]
pub struct AddCommentBody {
    pub text: String,
}

pub async fn api_comments_post_handler(
    State(state): State<WebState>,
    session: crate::web::session::SessionUser,
    Path(slug): Path<String>,
    Json(body): Json<AddCommentBody>,
) -> Response {
    let slug = urldecode(&slug);
    let slug = slug.trim_end_matches('/');

    // Validate: non-empty text, max 4096 chars
    let text = body.text.trim();
    if text.is_empty() {
        return ApiResponse::err(StatusCode::BAD_REQUEST, "EMPTY_COMMENT", "comment text is required").into_response();
    }
    if text.len() > 4096 {
        return ApiResponse::err(StatusCode::BAD_REQUEST, "COMMENT_TOO_LONG", "comment must be 4096 characters or fewer").into_response();
    }

    // ACL: user must be able to edit the page to post comments
    #[cfg(feature = "reason")]
    if state.collab {
        if let Err(resp) = check_page_acl_edit(&state, &session.user_id, slug) {
            return resp;
        }
    }

    // Load server key for HMAC signing (REQ-020-066)
    let server_key = match crate::user::invite::load_or_create_server_key(&state.vault_root) {
        Ok(k) => k.to_bytes().to_vec(),
        Err(e) => {
            eprintln!("error loading server key for comment HMAC: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to sign comment").into_response();
        }
    };

    match crate::user::comment::append_comment(&state.vault_root, slug, &session.user_id, text, &server_key) {
        Ok(comment) => {
            // Broadcast to all connected WebSocket clients viewing this page
            let room = state.ws_hub.room(slug);
            let _ = room.tx.send(crate::web::ws::ServerMsg::Comment {
                slug: slug.to_string(),
                user: comment.user.clone(),
                text: comment.text.clone(),
                at: comment.at.clone(),
            });

            ApiResponse::ok(serde_json::json!({ "comment": comment })).into_response()
        }
        Err(e) => {
            eprintln!("error posting comment on {slug}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to post comment").into_response()
        }
    }
}

/// Check that a user can read a page (used by comment GET).
#[cfg(feature = "reason")]
fn check_page_acl_read(
    state: &WebState,
    user_id: &str,
    page_slug: &str,
) -> Result<(), Response> {
    let (spl_blocks, all_slugs) = {
        let data = state.data.read().unwrap();
        let file = data.files.iter().find(|f| {
            crate::scanner::page_slug_from_path(&f.path).eq_ignore_ascii_case(page_slug)
        });
        let spl = file.map(|f| f.spl_blocks.clone()).unwrap_or_default();
        let slugs: Vec<String> = data.files.iter().map(|f| crate::scanner::page_slug_from_path(&f.path)).collect();
        (spl, slugs)
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let query = crate::acl::AclQuery {
        user_id: user_id.to_string(),
        page_slug: page_slug.to_string(),
        action: crate::acl::Action::Read,
        is_agent: false,
        now_epoch_ms: now_ms,
    };
    let decision = {
        let cache = state.acl_cache.lock().unwrap();
        if let Some(cached) = cache.lookup(user_id, page_slug, crate::acl::Action::Read) {
            cached.clone()
        } else {
            drop(cache);
            match crate::acl::evaluate(&state.vault_root, &query, &spl_blocks, &all_slugs) {
                Ok(d) => {
                    let mut cache = state.acl_cache.lock().unwrap();
                    cache.insert(user_id.to_string(), page_slug.to_string(), crate::acl::Action::Read, d.clone());
                    d
                }
                Err(_) => crate::acl::AclDecision::Denied {
                    tag: crate::acl::ConclusionTag::DefeasiblyNotProvable,
                    rule_trace: vec![],
                },
            }
        }
    };
    if decision.is_allowed() {
        Ok(())
    } else {
        Err(ApiResponse::err(StatusCode::FORBIDDEN, "ACL_DENIED", "you do not have read access to this page"))
    }
}

/// Check that a user can edit a page (used by comment POST).
#[cfg(feature = "reason")]
fn check_page_acl_edit(
    state: &WebState,
    user_id: &str,
    page_slug: &str,
) -> Result<(), Response> {
    let (spl_blocks, all_slugs) = {
        let data = state.data.read().unwrap();
        let file = data.files.iter().find(|f| {
            crate::scanner::page_slug_from_path(&f.path).eq_ignore_ascii_case(page_slug)
        });
        let spl = file.map(|f| f.spl_blocks.clone()).unwrap_or_default();
        let slugs: Vec<String> = data.files.iter().map(|f| crate::scanner::page_slug_from_path(&f.path)).collect();
        (spl, slugs)
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let query = crate::acl::AclQuery {
        user_id: user_id.to_string(),
        page_slug: page_slug.to_string(),
        action: crate::acl::Action::Edit,
        is_agent: false,
        now_epoch_ms: now_ms,
    };
    let decision = {
        let cache = state.acl_cache.lock().unwrap();
        if let Some(cached) = cache.lookup(user_id, page_slug, crate::acl::Action::Edit) {
            cached.clone()
        } else {
            drop(cache);
            match crate::acl::evaluate(&state.vault_root, &query, &spl_blocks, &all_slugs) {
                Ok(d) => {
                    let mut cache = state.acl_cache.lock().unwrap();
                    cache.insert(user_id.to_string(), page_slug.to_string(), crate::acl::Action::Edit, d.clone());
                    d
                }
                Err(_) => crate::acl::AclDecision::Denied {
                    tag: crate::acl::ConclusionTag::DefeasiblyNotProvable,
                    rule_trace: vec![],
                },
            }
        }
    };
    if decision.is_allowed() {
        Ok(())
    } else {
        Err(ApiResponse::err(StatusCode::FORBIDDEN, "ACL_DENIED", "you do not have edit access to this page"))
    }
}

// ── POST /api/reason — run SPL query (REQ-020-018) ───────────────────────

#[cfg(feature = "reason")]
#[derive(Deserialize)]
pub struct ApiReasonRequest {
    pub query: Option<String>,
}

#[cfg(feature = "reason")]
pub async fn api_reason_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ApiReasonRequest>,
) -> Response {
    // Reason endpoint requires admin in collab mode
    if state.collab {
        let (user_id, _is_agent) = match require_auth(&state, &headers) {
            Ok(auth) => auth,
            Err(resp) => return resp,
        };

        // Check admin role
        match crate::user::load_profile(&state.vault_root, &user_id) {
            Ok(Some(profile)) => {
                let role = crate::user::Role::for_profile_with_vault(&profile, &state.vault_root);
                if role < crate::user::Role::Admin {
                    return ApiResponse::err(
                        StatusCode::FORBIDDEN,
                        "ACL_DENIED",
                        "admin role required for /api/reason",
                    );
                }
            }
            _ => {
                return ApiResponse::err(
                    StatusCode::FORBIDDEN,
                    "ACL_DENIED",
                    "user profile not found",
                );
            }
        }
    }

    let data = state.data.read().unwrap();

    // Collect all SPL blocks from the vault
    let mut spl_blocks: Vec<crate::types::SplBlock> = Vec::new();
    for file in &data.files {
        spl_blocks.extend(file.spl_blocks.iter().cloned());
    }

    // If an additional query is provided, inject it as an extra block
    if let Some(ref query_spl) = body.query {
        if !query_spl.trim().is_empty() {
            spl_blocks.push(crate::types::SplBlock {
                source_file: std::path::PathBuf::from("<api-query>"),
                source_page: "<api-query>".to_string(),
                start_line: 0,
                end_line: 0,
                content: query_spl.clone(),
            });
        }
    }

    drop(data);

    match crate::reason::build_theory(&spl_blocks) {
        Ok(result) => ApiResponse::ok(serde_json::json!({
            "conclusions": result.conclusions,
            "diagnostics": result.diagnostics,
            "summary": result.summary,
        }))
        .into_response(),
        Err(e) => ApiResponse::err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "INTERNAL_ERROR",
            format!("reasoning failed: {e}"),
        ),
    }
}

// ── GET /api/acl/explain — explain ACL decision (REQ-020-014) ─────────────

#[cfg(feature = "reason")]
#[derive(Deserialize)]
pub struct AclExplainParams {
    pub page: String,
    pub action: Option<String>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct AclExplainResponse {
    decision: String,
    tag: String,
    user_id: String,
    page: String,
    action: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    proof_trace: Vec<AclExplainProofEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    why_not: Vec<AclExplainWhyNotRule>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deontic: Vec<AclExplainDeonticEntry>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct AclExplainDeonticEntry {
    modality: String,
    predicate: String,
    tag: String,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct AclExplainProofEntry {
    rule_label: Option<String>,
    source_file: String,
    source_line: u32,
    contribution: String,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct AclExplainWhyNotRule {
    rule_label: String,
    rule_type: String,
    blockers: Vec<AclExplainBlocker>,
}

#[cfg(feature = "reason")]
#[derive(Serialize)]
struct AclExplainBlocker {
    blocker_type: String,
    literal: String,
    explanation: String,
}

#[cfg(feature = "reason")]
pub async fn api_acl_explain_handler(
    State(state): State<WebState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<AclExplainParams>,
) -> Response {
    // Requires authentication
    let (user_id, is_agent) = match require_auth(&state, &headers) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let action = match params.action.as_deref().unwrap_or("read") {
        "read" => crate::acl::Action::Read,
        "edit" => crate::acl::Action::Edit,
        other => {
            return ApiResponse::err(
                StatusCode::BAD_REQUEST,
                "INVALID_ACTION",
                format!("action must be 'read' or 'edit', got '{other}'"),
            );
        }
    };

    let page_slug = params.page.clone();

    // Collect page-level SPL blocks and all page slugs
    let (page_spl_blocks, all_page_slugs) = {
        let data = state.data.read().unwrap();
        let page_spl: Vec<crate::types::SplBlock> = data
            .files
            .iter()
            .filter(|f| {
                crate::scanner::page_slug_from_path(&f.path) == page_slug
            })
            .flat_map(|f| f.spl_blocks.iter().cloned())
            .collect();
        let slugs: Vec<String> = data
            .files
            .iter()
            .map(|f| crate::scanner::page_slug_from_path(&f.path))
            .collect();
        (page_spl, slugs)
    };

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let query = crate::acl::AclQuery {
        user_id: user_id.clone(),
        page_slug: page_slug.clone(),
        action,
        is_agent,
        now_epoch_ms: now_ms,
    };

    let (decision, deontic_overlay, theory_result) = match crate::acl::evaluate_with_theory(
        &state.vault_root,
        &query,
        &page_spl_blocks,
        &all_page_slugs,
    ) {
        Ok(r) => r,
        Err(e) => {
            return ApiResponse::err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                format!("ACL evaluation failed: {e}"),
            );
        }
    };

    let (decision_str, tag, rule_trace) = match &decision {
        crate::acl::AclDecision::Allowed { tag, rule_trace } => {
            ("allowed", format_tag(*tag), rule_trace.clone())
        }
        crate::acl::AclDecision::Denied { tag, rule_trace } => {
            ("denied", format_tag(*tag), rule_trace.clone())
        }
    };

    let proof_trace: Vec<AclExplainProofEntry> = if decision.is_allowed() {
        rule_trace
            .iter()
            .map(|r| AclExplainProofEntry {
                rule_label: r.label.clone(),
                source_file: r.source_file.to_string_lossy().to_string(),
                source_line: r.source_line,
                contribution: r.contribution.clone(),
            })
            .collect()
    } else {
        vec![]
    };

    // For denied decisions, build why-not analysis
    let why_not: Vec<AclExplainWhyNotRule> = if !decision.is_allowed() {
        build_why_not_for_acl(&query, &theory_result)
    } else {
        vec![]
    };

    // Build deontic entries from overlay (REQ-020-012)
    let deontic_entries: Vec<AclExplainDeonticEntry> = {
        let mut entries = Vec::new();
        for c in &deontic_overlay.forbidden {
            entries.push(AclExplainDeonticEntry {
                modality: "[F]".to_string(),
                predicate: c.predicate.clone(),
                tag: format_tag(c.tag),
            });
        }
        for c in &deontic_overlay.permitted {
            entries.push(AclExplainDeonticEntry {
                modality: "[P]".to_string(),
                predicate: c.predicate.clone(),
                tag: format_tag(c.tag),
            });
        }
        for c in &deontic_overlay.obligations {
            entries.push(AclExplainDeonticEntry {
                modality: "[O]".to_string(),
                predicate: c.predicate.clone(),
                tag: format_tag(c.tag),
            });
        }
        entries
    };

    ApiResponse::ok(AclExplainResponse {
        decision: decision_str.to_string(),
        tag,
        user_id,
        page: page_slug,
        action: action.to_string(),
        proof_trace,
        why_not,
        deontic: deontic_entries,
    })
    .into_response()
}

#[cfg(feature = "reason")]
fn format_tag(tag: crate::acl::ConclusionTag) -> String {
    match tag {
        crate::acl::ConclusionTag::DefinitelyProvable => "+D".to_string(),
        crate::acl::ConclusionTag::DefeasiblyProvable => "+d".to_string(),
        crate::acl::ConclusionTag::DefinitelyNotProvable => "-D".to_string(),
        crate::acl::ConclusionTag::DefeasiblyNotProvable => "-d".to_string(),
    }
}

/// Build why-not analysis for a denied ACL decision (REQ-020-014).
///
/// Examines candidate rules that could prove the target literal, identifies
/// which body literals are missing (failed preconditions) and which defeaters
/// are active, mirroring the CLI `reason why-not` logic.
#[cfg(feature = "reason")]
fn build_why_not_for_acl(
    query: &crate::acl::AclQuery,
    result: &crate::reason::types::TheoryResult,
) -> Vec<AclExplainWhyNotRule> {
    use crate::reason::types::{ConclusionType, RuleType};

    let target_literal = match query.action {
        crate::acl::Action::Read => {
            format!("can-read({}, {})", query.user_id, query.page_slug)
        }
        crate::acl::Action::Edit => {
            format!("can-edit({}, {})", query.user_id, query.page_slug)
        }
    };

    // Set of all provable literals
    let provable_set: std::collections::HashSet<String> = result
        .conclusions
        .iter()
        .filter(|c| {
            matches!(
                c.conclusion_type,
                ConclusionType::DefinitelyProvable | ConclusionType::DefeasiblyProvable
            )
        })
        .map(|c| c.literal.clone())
        .collect();

    // Find candidate rules whose head matches
    let candidate_rules: Vec<_> = result
        .rules
        .iter()
        .filter(|r| r.head.to_string() == target_literal)
        .collect();

    let negated_literal = format!("~{}", target_literal);

    let mut analyses = Vec::new();
    for rule in &candidate_rules {
        let mut blockers = Vec::new();

        // Check each body literal
        for body_lit in &rule.body {
            let body_str = body_lit.to_string();
            if !provable_set.contains(&body_str) {
                blockers.push(AclExplainBlocker {
                    blocker_type: "failed_body".to_string(),
                    literal: body_str,
                    explanation: "precondition not provable".to_string(),
                });
            }
        }

        // Check for active defeaters
        for def_rule in &result.rules {
            if def_rule.rule_type == RuleType::Defeater
                && def_rule.head.to_string() == negated_literal
            {
                let body_satisfied = def_rule
                    .body
                    .iter()
                    .all(|b| provable_set.contains(&b.to_string()));
                if body_satisfied {
                    blockers.push(AclExplainBlocker {
                        blocker_type: "defeated".to_string(),
                        literal: negated_literal.clone(),
                        explanation: format!(
                            "blocked by defeater '{}'",
                            def_rule.label
                        ),
                    });
                }
            }
        }

        // Check for superior competing rules
        for other_rule in &result.rules {
            if other_rule.head.to_string() == negated_literal
                && other_rule.rule_type != RuleType::Defeater
            {
                let other_superior = result
                    .theory
                    .superiorities()
                    .iter()
                    .any(|s| s.superior == other_rule.label && s.inferior == rule.label);

                if other_superior {
                    let body_satisfied = other_rule
                        .body
                        .iter()
                        .all(|b| provable_set.contains(&b.to_string()));
                    if body_satisfied {
                        blockers.push(AclExplainBlocker {
                            blocker_type: "defeated".to_string(),
                            literal: negated_literal.clone(),
                            explanation: format!(
                                "defeated by superior rule '{}'",
                                other_rule.label
                            ),
                        });
                    }
                }
            }
        }

        analyses.push(AclExplainWhyNotRule {
            rule_label: rule.label.clone(),
            rule_type: format!("{:?}", rule.rule_type),
            blockers,
        });
    }

    analyses
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};
    use tower::ServiceExt;

    use crate::graph::LinkGraph;
    use crate::search_index::SearchIndex;
    use crate::web::engine::TemplateEngine;
    use crate::web::{VaultData, WebState};

    /// Build a minimal WebState pointing at a temp dir.
    fn test_state(vault_root: &std::path::Path, theme: &str) -> WebState {
        let data = VaultData {
            files: vec![],
            graph: LinkGraph::build(&[], &HashMap::new()),
            page_names: vec![],
            resolved: HashSet::new(),
            page_slug_map: HashMap::new(),
            collision_names: HashSet::new(),
        };
        let search_index = SearchIndex::build(vault_root, &[]).unwrap();
        WebState {
            data: Arc::new(RwLock::new(data)),
            vault_root: Arc::new(vault_root.to_path_buf()),
            search_index: Arc::new(search_index),
            engine: Arc::new(TemplateEngine::new(vault_root, theme, false, false)),
            theme: theme.to_string(),
            verbose: false,
            collab: false,
            sessions: crate::web::session::SessionStore::new(),
            recovery_challenges: Arc::new(
                crate::user::recovery::RecoveryChallengeStore::new(),
            ),
            mnemonic_shown: Arc::new(std::sync::Mutex::new(HashSet::new())),
            bootstrap_used: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            rate_limiters: crate::web::rate_limit::AuthRateLimiters::new(),
            #[cfg(feature = "reason")]
            acl_cache: Arc::new(std::sync::Mutex::new(crate::web::AclCache::new())),
            git_commit_lock: None,
            ws_hub: crate::web::ws::WsHub::new(),
            ticket_store: crate::web::ws::TicketStore::new(),
            crdt_store: crate::web::ws::CrdtDocStore::new(Arc::new(vault_root.to_path_buf())),
            wal_store: Arc::new(crate::web::wal::WalStore::new(vault_root)),
            pending_writes: crate::web::fs_watch::PendingWrites::new(),
            passkey_mgr: crate::user::passkey::PasskeyManager::new(
                "localhost",
                "http://localhost:3000",
                "zetl vault",
            )
            .ok()
            .map(Arc::new),
            #[cfg(feature = "semantic")]
            vector_index: None,
        }
    }

    fn static_router(state: WebState) -> Router {
        Router::new()
            .route("/_static/{*path}", get(static_handler))
            .with_state(state)
    }

    async fn get_status(app: &Router, uri: &str) -> StatusCode {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        resp.status()
    }

    async fn get_body(app: &Router, uri: &str) -> (StatusCode, Vec<u8>, String) {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap()
            .to_vec();
        (status, body, ct)
    }

    #[test]
    fn test_mime_from_ext() {
        assert_eq!(mime_from_ext("app.js"), "application/javascript");
        assert_eq!(mime_from_ext("style.css"), "text/css");
        assert_eq!(mime_from_ext("image.png"), "image/png");
        assert_eq!(mime_from_ext("photo.jpg"), "image/jpeg");
        assert_eq!(mime_from_ext("icon.svg"), "image/svg+xml");
        assert_eq!(mime_from_ext("font.woff2"), "font/woff2");
        assert_eq!(mime_from_ext("font.woff"), "font/woff");
        assert_eq!(mime_from_ext("data.json"), "application/json");
        assert_eq!(mime_from_ext("page.html"), "text/html");
        assert_eq!(mime_from_ext("unknown.xyz"), "application/octet-stream");
        // Case-insensitive extension
        assert_eq!(mime_from_ext("IMG.PNG"), "image/png");
        assert_eq!(mime_from_ext("STYLE.CSS"), "text/css");
    }

    #[tokio::test]
    async fn static_serves_from_vault_static() {
        let tmp = tempfile::tempdir().unwrap();
        let static_dir = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&static_dir).unwrap();
        std::fs::write(static_dir.join("app.js"), b"console.log('hi');").unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/app.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"console.log('hi');");
        assert_eq!(ct, "application/javascript");
    }

    #[tokio::test]
    async fn static_theme_overrides_vault() {
        let tmp = tempfile::tempdir().unwrap();
        // Vault-wide static
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();
        std::fs::write(vault_static.join("style.css"), b"body{color:red}").unwrap();
        // Theme-specific static (should win)
        let theme_static = tmp.path().join(".zetl/themes/mytheme/static");
        std::fs::create_dir_all(&theme_static).unwrap();
        std::fs::write(theme_static.join("style.css"), b"body{color:blue}").unwrap();

        let state = test_state(tmp.path(), "mytheme");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/style.css").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"body{color:blue}");
        assert_eq!(ct, "text/css");
    }

    #[tokio::test]
    async fn static_falls_back_to_vault_when_theme_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();
        std::fs::write(vault_static.join("logo.png"), b"\x89PNG").unwrap();

        // Theme dir doesn't have logo.png
        let theme_static = tmp.path().join(".zetl/themes/mytheme/static");
        std::fs::create_dir_all(&theme_static).unwrap();

        let state = test_state(tmp.path(), "mytheme");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/logo.png").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"\x89PNG");
        assert_eq!(ct, "image/png");
    }

    #[tokio::test]
    async fn static_404_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/nope.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        // Put a secret file outside .zetl
        std::fs::write(tmp.path().join("secret.txt"), b"secret").unwrap();
        let vault_static = tmp.path().join(".zetl/static");
        std::fs::create_dir_all(&vault_static).unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/../secret.txt").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let status = get_status(&app, "/_static/../../etc/passwd").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_serves_nested_path() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join(".zetl/static/fonts/sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("inter.woff2"), b"woff2data").unwrap();

        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let (status, body, ct) = get_body(&app, "/_static/fonts/sub/inter.woff2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"woff2data");
        assert_eq!(ct, "font/woff2");
    }

    #[tokio::test]
    async fn static_no_dirs_returns_404_without_error() {
        let tmp = tempfile::tempdir().unwrap();
        // No .zetl directory at all
        let state = test_state(tmp.path(), "default");
        let app = static_router(state);

        let status = get_status(&app, "/_static/anything.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // ── days_to_ymd tests ────────────────────────────────────────────────

    #[test]
    fn test_days_to_ymd_epoch() {
        // 1970-01-01
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn test_days_to_ymd_known_date() {
        // 2026-03-18 is day 20530 since epoch
        assert_eq!(days_to_ymd(20530), (2026, 3, 18));
    }

    #[test]
    fn test_days_to_ymd_leap_year() {
        // 2024-02-29 is day 19782
        assert_eq!(days_to_ymd(19782), (2024, 2, 29));
    }

    // ── Passkey register handler tests ───────────────────────────────────

    #[tokio::test]
    async fn passkey_register_page_renders() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");

        let app = Router::new()
            .route("/passkey/register", axum::routing::get(passkey_register_handler))
            .with_state(state);

        let (status, body, _ct) = get_body(&app, "/passkey/register?user_id=alice-a1b2c3d4").await;
        assert_eq!(status, StatusCode::OK);
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("Register a Passkey"));
        assert!(html.contains("alice-a1b2c3d4"));
        assert!(html.contains("pk-spinner"));
        assert!(html.contains("Try Again"));
        assert!(html.contains("recovery"));
    }

    #[tokio::test]
    async fn passkey_register_start_no_user() {
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state(tmp.path(), "default");

        let app = Router::new()
            .route(
                "/api/passkey/register/start",
                axum::routing::post(passkey_register_start_handler),
            )
            .with_state(state);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/passkey/register/start")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"user_id":"nonexistent-12345678"}"#,
            ))
            .unwrap();

        let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
