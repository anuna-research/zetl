//! SPEC-040 mobile-specific serve routes (REQ-4005, CON-4004).
//!
//! Gated behind `--features mobile`. These routes provide the
//! capture / onboarding / sync surfaces consumed by the Tauri Mobile
//! shell. When the feature is off (the default for desktop serve),
//! the entire module is excluded from the build and no `/_mobile/*`
//! routes are registered.
//!
//! v0.1-strawman scope:
//!
//! - `/_mobile/onboarding` — wizard that drives BIP39 → ed25519 →
//!   ssh pubkey → git clone (REQ-4011, REQ-4002, REQ-4003)
//! - `/_mobile/capture` — placeholder, real form lands when capture
//!   POST handler ships (REQ-4006)
//! - `/_mobile/sync` — placeholder, manual pull/push controls land
//!   alongside REQ-4009 / REQ-4010 wiring
//!
//! All handlers are state-free at the axum level — they read process-
//! wide globals from [`crate::mobile_state`] (KeyStore, vault root)
//! so the router stays generic over `S` and unit tests can exercise
//! it against `Router<()>` without building a full `WebState`.
//!
//! HTML is inlined for v0.1; the follow-up "templates polish" slice
//! moves these surfaces into Minijinja templates rendered through the
//! active theme so they pick up theme typography / colours
//! automatically.

use axum::{
    extract::Form,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

/// Build the `/_mobile/*` router. Generic over state so callers
/// (the main router or unit tests) decide the parameterisation.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/_mobile/onboarding", get(onboarding_handler))
        .route("/_mobile/onboarding/seed", post(onboarding_seed_handler))
        .route("/_mobile/onboarding/clone", post(onboarding_clone_handler))
        .route(
            "/_mobile/capture",
            get(capture_handler).post(capture_post_handler),
        )
        .route("/_mobile/share", post(share_post_handler))
        .route("/_mobile/sync", get(sync_handler))
        .route("/_mobile/sync/pull", post(sync_pull_handler))
        .route("/_mobile/sync/push", post(sync_push_handler))
        .route("/_mobile/reset", post(reset_handler))
        .route("/_mobile/vaults", get(vaults_handler))
        .route("/_mobile/vaults/switch", post(vaults_switch_handler))
        .route("/_mobile/vaults/add", get(vaults_add_handler))
        .route(
            "/_mobile/vaults/remove",
            get(vaults_remove_confirm_handler).post(vaults_remove_handler),
        )
        .route(
            "/_mobile/vaults/pick",
            get(pick_subpath_handler).post(pick_subpath_post_handler),
        )
        .route("/_mobile/recovery", get(recovery_handler))
}

/// `GET /_mobile/recovery` — show the BIP39 recovery phrase for the
/// currently-loaded SSH key. The seed is the master secret; the user
/// should write it down and store it somewhere safe so they can
/// recover the same identity on a new device via the "Advanced:
/// use my desktop's BIP39 seed" path in onboarding.
async fn recovery_handler() -> Response {
    Html(render_recovery_page()).into_response()
}

// ── /_mobile/onboarding ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SeedForm {
    mnemonic: String,
}

#[derive(Deserialize)]
struct CloneForm {
    remote_url: String,
    /// Optional personal-access-token for private HTTPS remotes.
    /// Empty for SSH or public HTTPS.
    #[serde(default)]
    pat: String,
}

#[derive(Deserialize)]
struct CaptureForm {
    /// Optional explicit title; when empty, auto-titled from the
    /// first meaningful line of content or `Inbox YYYY-MM-DD-HHMM`.
    #[serde(default)]
    title: String,
    /// Markdown body. Written verbatim to the new file.
    content: String,
}

/// `GET /_mobile/onboarding` — single-step wizard.
///
/// State-driven render:
///
/// - keystore loaded AND vault is a working tree → onboarding is
///   already complete; redirect to `/` so the user lands on the
///   page list.
/// - keystore loaded, no `.git` in vault → render the pubkey + clone
///   form (the common path: the Tauri shell auto-generated a fresh
///   per-device key at startup; user just needs to add it to their
///   git host and paste the remote URL).
/// - keystore not loaded at all → fall back to the seed-paste form
///   (tests + odd-edge cases; production never hits this because the
///   shell auto-generates on launch).
async fn onboarding_handler(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let keystore = crate::mobile_state::global();

    let pub_line = match keystore.pub_openssh() {
        None => return Html(render_step_seed(None)).into_response(),
        Some(p) => p,
    };

    // "Add another vault" flow passes ?add=1 so the auto-redirect-
    // when-onboarded logic is bypassed and the user sees the clone
    // form even though an active vault exists.
    let force_clone_form = matches!(q.get("add").map(String::as_str), Some("1"));

    if !force_clone_form {
        if let Some(vault_root) = crate::mobile_state::vault_root() {
            if vault_root.join(".git").is_dir()
                || (vault_root.is_symlink() && vault_root.exists())
            {
                // Emit a 200 OK HTML response with a client-side
                // meta-refresh instead of a 303 redirect. WKWebView
                // (and some other WebViews) occasionally don't render
                // a 303-then-200 chain on the *initial* page load,
                // which manifests as a blank window. A normal HTML
                // response with `<meta http-equiv="refresh">` lands
                // reliably everywhere.
                return Html(MOBILE_REDIRECT_HOME).into_response();
            }
        }
    }

    Html(render_step_clone(&pub_line, None)).into_response()
}

const MOBILE_REDIRECT_HOME: &str = "\
<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta http-equiv=\"refresh\" content=\"0;url=/\">\
<title>zetl mobile</title></head>\
<body>Loading vault…</body></html>";

/// `POST /_mobile/onboarding/seed` — accept a 12-word BIP39 mnemonic,
/// derive the ed25519 SSH key, store it in the process keystore, and
/// persist it to disk so the user does not re-enter the seed on
/// future launches. Seed-persistence failures do not block the flow;
/// they are logged and the user proceeds with the in-memory key.
async fn onboarding_seed_handler(Form(form): Form<SeedForm>) -> Response {
    let keystore = crate::mobile_state::global();
    match keystore.import_mnemonic(form.mnemonic.trim()) {
        Ok(_) => {
            if let Some(dir) = crate::mobile_state::app_data_dir() {
                if let Err(e) = keystore.persist(&dir) {
                    eprintln!("[zetl-mobile] persist ssh key failed: {e:#}");
                }
            }
            Redirect::to("/_mobile/onboarding").into_response()
        }
        Err(e) => Html(render_step_seed(Some(&format!("{e:#}")))).into_response(),
    }
}

/// `POST /_mobile/onboarding/clone` — clone the user-supplied git
/// remote into `vaults/<label>/`, point the active-vault symlink at
/// it, and reindex the embedded serve. On success, redirect to `/`;
/// on failure, re-render the clone form with the error.
///
/// Multi-vault: the clone target is derived from the remote URL
/// (`derive_vault_label`) and lives at `app_data_dir/vaults/<label>/`.
/// The `app_data_dir/vault` symlink is repointed at the new vault so
/// the embedded serve (whose `vault_root` is fixed at boot to that
/// symlink path) follows it transparently.
async fn onboarding_clone_handler(Form(form): Form<CloneForm>) -> Response {
    let keystore = crate::mobile_state::global();
    let pub_line = match keystore.pub_openssh() {
        Some(p) => p,
        None => {
            return Html(render_step_seed(Some(
                "no SSH key in keystore — paste your seed phrase first",
            )))
            .into_response();
        }
    };

    let app_data = match crate::mobile_state::app_data_dir() {
        Some(p) => p,
        None => {
            return Html(render_step_clone(
                &pub_line,
                Some("app data dir not registered — Tauri shell did not initialise"),
            ))
            .into_response();
        }
    };

    let remote_url = form.remote_url.trim().to_string();
    let label = crate::mobile_state::derive_vault_label(&remote_url);
    let vaults_dir = app_data.join("vaults");
    let target_dir = vaults_dir.join(&label);

    // If a vault with this label already exists (same remote previously
    // cloned, or label collision), switch to it rather than re-cloning.
    if target_dir.join(".git").is_dir() {
        match crate::mobile_state::set_active_vault(&label, None) {
            Ok(_) => {
                let _ = crate::mobile_state::trigger_reindex();
                return Redirect::to("/").into_response();
            }
            Err(e) => {
                return Html(render_step_clone(
                    &pub_line,
                    Some(&format!("could not activate existing vault '{label}': {e:#}")),
                ))
                .into_response();
            }
        }
    }

    if let Err(e) = std::fs::create_dir_all(&vaults_dir) {
        return Html(render_step_clone(
            &pub_line,
            Some(&format!("create vaults dir failed: {e:#}")),
        ))
        .into_response();
    }

    // The clone is blocking I/O against libgit2. Run it on a blocking
    // pool thread so the axum request task is not held for the full
    // duration of the network fetch.
    let clone_target = target_dir.clone();
    let clone_url = remote_url.clone();
    let clone_pat: Option<String> = {
        let trimmed = form.pat.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };
    let clone_result = tokio::task::spawn_blocking(move || {
        crate::mobile_git::clone(&clone_url, &clone_target, clone_pat.as_deref())
    })
    .await;

    match clone_result {
        Ok(Ok(_repo)) => {
            // Scan the cloned repo for plausible vault subdirectories.
            // Some repos hold the vault at the root; others nest it
            // under `notes/`, `docs/`, etc.
            let candidates = crate::mobile_state::detect_vault_subpath_candidates(&target_dir);

            // Pick the best candidate (or repo root if none found) so
            // the symlink is always pointing at something valid before
            // we redirect.
            let best_subpath = candidates.first().map(|c| c.subpath.clone());
            let activate_result = crate::mobile_state::set_active_vault(
                &label,
                best_subpath.as_deref().filter(|s| !s.is_empty()),
            );
            if let Err(e) = activate_result {
                return Html(render_step_clone(
                    &pub_line,
                    Some(&format!("clone succeeded but could not activate vault: {e:#}")),
                ))
                .into_response();
            }
            if let Err(e) = crate::mobile_state::trigger_reindex() {
                eprintln!("[zetl-mobile] reindex after clone failed: {e:#}");
            }

            // If the candidate is ambiguous, send the user to the
            // picker to confirm (or choose a different folder). Single
            // candidate (or none at all) → straight to the vault page
            // list.
            if candidates.len() > 1 {
                let encoded_label = urlencoding::encode(&label).into_owned();
                Redirect::to(&format!("/_mobile/vaults/pick?label={encoded_label}"))
                    .into_response()
            } else {
                Redirect::to("/").into_response()
            }
        }
        Ok(Err(e)) => Html(render_step_clone(&pub_line, Some(&format!("{e:#}")))).into_response(),
        Err(join_err) => Html(render_step_clone(
            &pub_line,
            Some(&format!("clone task panicked: {join_err}")),
        ))
        .into_response(),
    }
}

// ── /_mobile/vaults/pick — subpath picker ─────────────────────────────────────

#[derive(Deserialize)]
struct PickQuery {
    label: String,
}

#[derive(Deserialize)]
struct PickForm {
    label: String,
    subpath: String,
}

async fn pick_subpath_handler(
    axum::extract::Query(q): axum::extract::Query<PickQuery>,
) -> Response {
    let label = q.label.trim();
    if label.is_empty() {
        return Redirect::to("/_mobile/vaults").into_response();
    }
    let app_data = match crate::mobile_state::app_data_dir() {
        Some(p) => p,
        None => return Redirect::to("/_mobile/vaults").into_response(),
    };
    let repo_root = app_data.join("vaults").join(label);
    if !repo_root.is_dir() {
        return Redirect::to("/_mobile/vaults").into_response();
    }
    let candidates = crate::mobile_state::detect_vault_subpath_candidates(&repo_root);
    let active_subpath: String = crate::mobile_state::vault_root()
        .and_then(|link| std::fs::read_link(&link).ok())
        .and_then(|t| {
            let s = t.to_string_lossy().to_string();
            let prefix = format!("vaults/{label}");
            s.strip_prefix(&prefix)
                .map(|rest| rest.trim_start_matches('/').to_string())
        })
        .unwrap_or_default();
    Html(render_pick_subpath(label, &candidates, &active_subpath)).into_response()
}

async fn pick_subpath_post_handler(Form(form): Form<PickForm>) -> Response {
    let label = form.label.trim();
    let subpath = form.subpath.trim();
    if label.is_empty() {
        return Redirect::to("/_mobile/vaults").into_response();
    }
    let opt_sub = if subpath.is_empty() { None } else { Some(subpath) };
    match crate::mobile_state::set_active_vault(label, opt_sub) {
        Ok(_) => {
            if let Err(e) = crate::mobile_state::trigger_reindex() {
                eprintln!("[zetl-mobile] reindex after subpath pick failed: {e:#}");
            }
            Redirect::to("/").into_response()
        }
        Err(e) => Html(render_vaults_page(Some(VaultsMsg::Error(format!(
            "could not activate vault '{label}' at subpath '{subpath}': {e:#}"
        )))))
        .into_response(),
    }
}


// ── /_mobile/capture ──────────────────────────────────────────────────────────

/// `GET /_mobile/capture` — quick-capture form. When called with
/// `?from=share`, drains the share-extension inbox and prefills the
/// form with the most recent entry's title + body (REQ-4007).
async fn capture_handler(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let from_share = q.get("from").map(String::as_str) == Some("share");
    let (title, body) = if from_share {
        let entries = crate::mobile_state::drain_share_inbox();
        if let Some(first) = entries.into_iter().next() {
            (first.title, first.body)
        } else {
            (String::new(), String::new())
        }
    } else {
        (String::new(), String::new())
    };
    Html(render_capture_form(None, &title, &body)).into_response()
}

#[derive(Deserialize)]
struct ShareIncoming {
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: String,
}

/// `POST /_mobile/share` — receive a share payload from a native
/// share extension (Android share-target Activity is the primary
/// caller; iOS Share Extensions write directly to the inbox file
/// via the app-group container). Also useful for testing without
/// platform-native code: `curl --data "title=X&body=Y" .../_mobile/share`.
async fn share_post_handler(Form(form): Form<ShareIncoming>) -> Response {
    let kind = if !form.url.is_empty() && form.body.is_empty() {
        "url"
    } else if !form.url.is_empty() {
        "url_with_title"
    } else {
        "text"
    };
    let body = if !form.body.is_empty() {
        form.body
    } else {
        form.url
    };
    let entry = crate::mobile_state::ShareInboxEntry {
        received_at: iso8601_now(),
        kind: kind.into(),
        title: form.title,
        body,
    };
    match crate::mobile_state::append_share_entry(&entry) {
        Ok(()) => Redirect::to("/_mobile/capture?from=share").into_response(),
        Err(e) => Html(format!(
            "<h1>share inbox write failed</h1><pre>{}</pre>",
            html_escape(&format!("{e:#}"))
        ))
        .into_response(),
    }
}

fn iso8601_now() -> String {
    let now = crate::mobile_capture::SystemNow::real();
    let (y, mo, d, h, mi) = now.ymd_hm;
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:00Z")
}

/// `POST /_mobile/capture` — write a new note + commit, then redirect
/// to the new page. Best-effort push runs in the background on
/// success; offline → commit stays in local repo, retried on next
/// online event (REQ-4010).
async fn capture_post_handler(Form(form): Form<CaptureForm>) -> Response {
    let vault_root = match crate::mobile_state::vault_root() {
        Some(p) => p,
        None => {
            return Html(render_capture_form(
                Some("vault root not registered — Tauri shell did not initialise"),
                &form.title,
                &form.content,
            ))
            .into_response();
        }
    };

    // Filesystem + git work is blocking; offload from the request task.
    let title = form.title.clone();
    let content = form.content.clone();
    let root = vault_root.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        crate::mobile_capture::capture(
            &root,
            &title,
            &content,
            crate::mobile_capture::SystemNow::real(),
        )
    })
    .await;

    match outcome {
        Ok(Ok(o)) => {
            // Best-effort push: spawn-blocking-and-forget so the redirect
            // doesn't wait on network. Failures are logged and surface
            // through /_mobile/sync's pending count once that ships.
            let push_root = vault_root.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = crate::mobile_git::push(&push_root) {
                    eprintln!("[zetl-mobile] best-effort push failed: {e:#}");
                }
            });
            // url-encode the slug for the redirect target.
            let encoded = urlencoding::encode(&o.slug).into_owned();
            Redirect::to(&format!("/{encoded}")).into_response()
        }
        Ok(Err(e)) => Html(render_capture_form(
            Some(&format!("{e:#}")),
            &form.title,
            &form.content,
        ))
        .into_response(),
        Err(join_err) => Html(render_capture_form(
            Some(&format!("capture task panicked: {join_err}")),
            &form.title,
            &form.content,
        ))
        .into_response(),
    }
}

// ── /_mobile/sync ─────────────────────────────────────────────────────────────

/// `GET /_mobile/sync` — sync status + manual pull/push buttons.
async fn sync_handler() -> Response {
    Html(render_sync_page(None)).into_response()
}

/// `POST /_mobile/sync/pull` — invoke FF-only pull, then redirect
/// back to `/_mobile/sync` with status or conflict notice.
async fn sync_pull_handler() -> Response {
    let vault_root = match crate::mobile_state::vault_root() {
        Some(p) => p,
        None => {
            return Html(render_sync_page(Some(SyncMsg::Error(
                "vault root not registered — Tauri shell did not initialise".to_string(),
            ))))
            .into_response()
        }
    };

    let outcome =
        tokio::task::spawn_blocking(move || crate::mobile_git::pull_ff_only(&vault_root)).await;

    let msg = match outcome {
        Ok(Ok(crate::mobile_git::PullOutcome::UpToDate)) => {
            SyncMsg::Ok("Already up to date.".into())
        }
        Ok(Ok(crate::mobile_git::PullOutcome::FastForwarded { from, to })) => SyncMsg::Ok(format!(
            "Fast-forwarded {} → {}",
            &from[..8.min(from.len())],
            &to[..8.min(to.len())]
        )),
        Ok(Ok(crate::mobile_git::PullOutcome::Conflict)) => SyncMsg::Conflict,
        Ok(Err(e)) => SyncMsg::Error(format!("{e:#}")),
        Err(join_err) => SyncMsg::Error(format!("pull task panicked: {join_err}")),
    };

    Html(render_sync_page(Some(msg))).into_response()
}

/// `POST /_mobile/sync/push` — invoke push, redirect back with status.
async fn sync_push_handler() -> Response {
    let vault_root = match crate::mobile_state::vault_root() {
        Some(p) => p,
        None => {
            return Html(render_sync_page(Some(SyncMsg::Error(
                "vault root not registered — Tauri shell did not initialise".to_string(),
            ))))
            .into_response()
        }
    };

    let outcome = tokio::task::spawn_blocking(move || crate::mobile_git::push(&vault_root)).await;

    let msg = match outcome {
        Ok(Ok(())) => SyncMsg::Ok("Pushed.".into()),
        Ok(Err(e)) => SyncMsg::Error(format!("{e:#}")),
        Err(join_err) => SyncMsg::Error(format!("push task panicked: {join_err}")),
    };

    Html(render_sync_page(Some(msg))).into_response()
}

enum SyncMsg {
    Ok(String),
    Conflict,
    Error(String),
}

// ── /_mobile/vaults — multi-vault picker ──────────────────────────────────────

#[derive(Deserialize)]
struct SwitchForm {
    label: String,
}

/// `GET /_mobile/vaults` — list every cloned vault and let the user
/// switch the active one. Each non-active row has a "Switch" button
/// that POSTs to `/_mobile/vaults/switch`. An "Add another vault"
/// link sends the user back to `/_mobile/onboarding` with a flag so
/// the auto-redirect-when-onboarded logic is bypassed.
async fn vaults_handler() -> Response {
    Html(render_vaults_page(None)).into_response()
}

/// `POST /_mobile/vaults/switch` — repoint the `vault` symlink at
/// `vaults/<label>/` and trigger a reindex so the embedded serve's
/// page list reflects the new working tree.
async fn vaults_switch_handler(Form(form): Form<SwitchForm>) -> Response {
    let label = form.label.trim().to_string();
    if label.is_empty() {
        return Html(render_vaults_page(Some(VaultsMsg::Error(
            "switch label is empty".into(),
        ))))
        .into_response();
    }
    match crate::mobile_state::set_active_vault(&label, None) {
        Ok(_) => {
            if let Err(e) = crate::mobile_state::trigger_reindex() {
                eprintln!("[zetl-mobile] reindex after switch failed: {e:#}");
            }
            Redirect::to("/").into_response()
        }
        Err(e) => Html(render_vaults_page(Some(VaultsMsg::Error(format!("{e:#}")))))
            .into_response(),
    }
}

/// `GET /_mobile/vaults/add` — redirect to onboarding. The
/// onboarding handler's "redirect to / when .git exists" logic is
/// keyed off the *active* vault's `.git` directory; clicking
/// "Add another vault" first repoints the symlink to a sentinel
/// (no-op when there's no active vault) so onboarding renders the
/// clone form rather than auto-redirecting. v0.1 implementation: we
/// rely on the fact that the user can paste a *different* remote URL
/// and the clone handler short-circuits to a switch if the label
/// matches an existing vault. So a plain redirect to /_mobile/onboarding
/// works — the user pastes a new URL and the handler does the right
/// thing (clone-new or switch-to-existing).
async fn vaults_add_handler() -> Response {
    // Force the onboarding GET to render the clone form (step 2) by
    // temporarily relying on the keystore-loaded + symlink-target-
    // does-not-exist state. We can't easily fake that without races,
    // so the simpler path: redirect straight to the clone-form
    // render with a marker query so the onboarding handler always
    // shows it. v0.1 implementation: redirect to a new URL that
    // forces the form. For minimal code we just send the user to a
    // direct clone-form render via /_mobile/onboarding?add=1 which
    // the handler interprets as "always show clone step, never
    // redirect".
    Redirect::to("/_mobile/onboarding?add=1").into_response()
}

enum VaultsMsg {
    Error(String),
    Ok(String),
}

#[derive(Deserialize)]
struct RemoveForm {
    label: String,
}

#[derive(Deserialize)]
struct RemoveQuery {
    label: String,
}

/// `GET /_mobile/vaults/remove?label=<label>` — confirmation page
/// shown to the user before actually wiping. Avoids relying on
/// `window.confirm()` which Tauri WebViews on some platforms silently
/// suppress (the JS returns false and the form never submits, making
/// it look like the button does nothing).
async fn vaults_remove_confirm_handler(
    axum::extract::Query(q): axum::extract::Query<RemoveQuery>,
) -> Response {
    let label = q.label.trim();
    if label.is_empty() {
        return Redirect::to("/_mobile/vaults").into_response();
    }
    Html(render_remove_confirm_page(label)).into_response()
}

/// `POST /_mobile/vaults/remove` — wipe the local working tree at
/// `vaults/<label>/`. The git remote is untouched: the user can
/// re-clone via /_mobile/onboarding?add=1 at any time. If the
/// removed vault was active, the symlink is cleared too — the
/// embedded serve will show an empty page list until another vault
/// is activated or cloned.
async fn vaults_remove_handler(Form(form): Form<RemoveForm>) -> Response {
    let label = form.label.trim().to_string();
    if label.is_empty() {
        return Html(render_vaults_page(Some(VaultsMsg::Error(
            "remove label is empty".into(),
        ))))
        .into_response();
    }

    let app_data = match crate::mobile_state::app_data_dir() {
        Some(p) => p,
        None => {
            return Html(render_vaults_page(Some(VaultsMsg::Error(
                "app_data_dir not registered".into(),
            ))))
            .into_response();
        }
    };

    let target = app_data.join("vaults").join(&label);
    if !target.exists() {
        return Html(render_vaults_page(Some(VaultsMsg::Error(format!(
            "vault '{label}' does not exist"
        )))))
        .into_response();
    }

    // If this was the active vault, drop the symlink first so we
    // don't leave a dangling pointer after the dir is gone.
    let was_active = crate::mobile_state::active_vault_label().as_deref() == Some(label.as_str());
    if was_active {
        if let Some(link) = crate::mobile_state::vault_root() {
            if link.is_symlink() {
                let _ = std::fs::remove_file(&link);
            }
        }
    }

    if let Err(e) = std::fs::remove_dir_all(&target) {
        return Html(render_vaults_page(Some(VaultsMsg::Error(format!(
            "remove vaults/{label}: {e:#}"
        )))))
        .into_response();
    }

    // If the removed vault was active and other vaults remain,
    // auto-activate one so the embedded serve has somewhere to point.
    if was_active {
        let remaining = crate::mobile_state::list_vaults();
        if let Some(next) = remaining.first() {
            let _ = crate::mobile_state::set_active_vault(&next.label, None);
            let _ = crate::mobile_state::trigger_reindex();
        } else {
            // No vaults left — embedded serve still runs but with an
            // empty page list. Trigger reindex to clear stale data.
            let _ = crate::mobile_state::trigger_reindex();
        }
    }

    Html(render_vaults_page(Some(VaultsMsg::Ok(format!(
        "Removed '{label}' from local storage. The remote is untouched — clone again via Add another vault."
    )))))
    .into_response()
}

/// `POST /_mobile/reset` — multi-vault aware: removes the **active**
/// vault's working tree from `vaults/<active-label>/`, unsets the
/// symlink, and forgets the in-memory + on-disk SSH key. Other
/// cloned vaults under `vaults/` are preserved. Redirects to
/// `/_mobile/onboarding` or `/_mobile/vaults` depending on whether
/// any vaults remain.
async fn reset_handler() -> Response {
    let keystore = crate::mobile_state::global();

    // Resolve the active vault's actual working-tree path (the
    // symlink's target) before we remove the symlink.
    let active_target: Option<std::path::PathBuf> =
        crate::mobile_state::vault_root().and_then(|link| std::fs::read_link(&link).ok())
            .and_then(|target| {
                // Symlink target is relative to app_data_dir
                crate::mobile_state::app_data_dir().map(|root| root.join(target))
            });

    if let Some(target) = active_target {
        if target.exists() {
            if let Err(e) = std::fs::remove_dir_all(&target) {
                return Html(render_sync_page(Some(SyncMsg::Error(format!(
                    "wipe vault {}: {e:#}",
                    target.display()
                )))))
                .into_response();
            }
        }
    }
    // Remove the symlink itself so onboarding renders cleanly.
    if let Some(link) = crate::mobile_state::vault_root() {
        if link.is_symlink() {
            let _ = std::fs::remove_file(&link);
        } else if link.exists() {
            let _ = std::fs::remove_dir_all(&link);
        }
    }

    // Forget the persisted key (keyring + file) + legacy vault meta.
    if let Some(app_data) = crate::mobile_state::app_data_dir() {
        keystore.forget_persistent(&app_data);
        let _ = std::fs::remove_file(app_data.join("vault_meta.json"));
    }
    keystore.clear();

    // If other vaults remain, the user probably wants to switch to
    // one of them, not re-onboard from scratch.
    if !crate::mobile_state::list_vaults().is_empty() {
        Redirect::to("/_mobile/vaults").into_response()
    } else {
        Redirect::to("/_mobile/onboarding").into_response()
    }
}

// ── HTML rendering helpers ────────────────────────────────────────────────────

fn render_step_seed(error: Option<&str>) -> String {
    let error_block = match error {
        Some(msg) => format!(
            r#"<div data-zetl-mobile-error="seed" style="color:#b00;background:#fee;padding:0.75em;border-radius:6px;margin-bottom:1em;">Error: {}</div>"#,
            html_escape(msg)
        ),
        None => String::new(),
    };
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · onboarding · step 1</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  textarea {{ width: 100%; min-height: 6em; font-family: ui-monospace, monospace; font-size: 1rem; padding: 0.6em; box-sizing: border-box; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  .step {{ font-size: 0.85em; opacity: 0.65; margin-bottom: 0.4em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  p {{ line-height: 1.5; }}
</style></head>
<body data-zetl-mobile-route="onboarding" data-zetl-mobile-step="seed">
<div class="step">Step 1 of 2</div>
<h1>Paste your 12-word recovery phrase</h1>
<p>This is the same phrase you used with <code>zetl derive-ssh-key --mnemonic</code> on desktop. The phone derives the same SSH key locally — the phrase never leaves the device.</p>
{error_block}
<form method="post" action="/_mobile/onboarding/seed">
  <textarea name="mnemonic" placeholder="word1 word2 … word12" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" required></textarea>
  <button type="submit">Derive SSH key →</button>
</form>
{back}
{external_js}
</body></html>"#,
        back = render_onboarding_back_link(),
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

/// Back link rendered in onboarding pages when the user already has
/// at least one cloned vault — they got to onboarding via "+ Add
/// another vault" and might want to bail out. Empty string for fresh
/// installs so we don't show a dangling link to an empty picker.
fn render_onboarding_back_link() -> String {
    if crate::mobile_state::list_vaults().is_empty() {
        String::new()
    } else {
        r#"<p style="margin-top:1.5em;font-size:0.9em;"><a href="/_mobile/vaults" style="color:inherit;">← Back to Vaults</a></p>"#.to_string()
    }
}

fn render_step_clone(pub_line: &str, error: Option<&str>) -> String {
    let error_block = match error {
        Some(msg) => format!(
            r#"<div data-zetl-mobile-error="clone" style="color:#b00;background:#fee;padding:0.75em;border-radius:6px;margin-bottom:1em;">Error: {}</div>"#,
            html_escape(msg)
        ),
        None => String::new(),
    };
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · onboarding</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  pre {{ background: #f4f4f4; padding: 0.7em; border-radius: 6px; word-break: break-all; white-space: pre-wrap; font-size: 0.85rem; }}
  input[type="url"] {{ width: 100%; font-family: ui-monospace, monospace; font-size: 1rem; padding: 0.6em; box-sizing: border-box; }}
  textarea {{ width: 100%; min-height: 6em; font-family: ui-monospace, monospace; font-size: 1rem; padding: 0.6em; box-sizing: border-box; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  p {{ line-height: 1.5; }}
  .copy {{ font-size: 0.85em; }}
  details {{ margin-top: 2em; padding-top: 1em; border-top: 1px solid currentColor; opacity: 0.7; }}
  details summary {{ cursor: pointer; font-size: 0.9em; }}
  .topbar {{ display: flex; align-items: center; gap: 0.4em; margin-bottom: 0.8em; font-size: 0.9em; }}
  .topbar a {{ color: inherit; opacity: 0.8; text-decoration: none; }}
  .topbar a:hover {{ opacity: 1; }}
</style></head>
<body data-zetl-mobile-route="onboarding" data-zetl-mobile-step="clone">
<style>
  form.zetl-busy button {{ opacity: 0.6; cursor: wait; }}
  form.zetl-busy button::after {{ content: ''; display: inline-block; width: 0.9em; height: 0.9em; border: 2px solid currentColor; border-top-color: transparent; border-radius: 50%; animation: zspin 0.8s linear infinite; margin-left: 0.6em; vertical-align: middle; }}
  @keyframes zspin {{ to {{ transform: rotate(360deg); }} }}
</style>
<h1>Add this SSH key to your git host, then clone</h1>
<p>This phone has its own SSH key (generated on first launch). Add it to <em>any</em> git host where the vault lives — one key works for all of them.</p>
<pre data-zetl-mobile-pubkey>{pub}</pre>
<p class="copy"><button type="button" onclick="navigator.clipboard.writeText(document.querySelector('[data-zetl-mobile-pubkey]').textContent).then(() =&gt; {{ this.textContent = 'Copied'; }})">Copy public key</button></p>
<p class="hint-links" style="font-size:0.85em;opacity:0.75;line-height:1.7;">
  Open the SSH-keys settings for your host (paste in the form there):<br>
  <a href="https://github.com/settings/ssh/new" target="_blank" rel="noopener noreferrer">→ GitHub</a> ·
  <a href="https://gitlab.com/-/user_settings/ssh_keys" target="_blank" rel="noopener noreferrer">→ GitLab</a> ·
  <a href="https://codeberg.org/user/settings/keys" target="_blank" rel="noopener noreferrer">→ Codeberg</a>
</p>
{error_block}
<form method="post" action="/_mobile/onboarding/clone" data-zetl-busy-label="Cloning…">
  <input type="text" name="remote_url" id="remote_url" placeholder="git@codeberg.org:you/your-vault.git" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" required>
  <p class="hint" style="font-size:0.85em;opacity:0.7;margin:0.3em 0 0.6em;">
    <strong>SSH is recommended.</strong> Once you've added the public key above to your git host,
    SSH URLs like <code>git@github.com:owner/repo.git</code> work for clone and ongoing pull/push
    with no extra setup. HTTPS URLs of public repos also work; private HTTPS needs a PAT (advanced).
  </p>
  <div id="ssh-suggest" data-zetl-suggest hidden style="margin:0.4em 0;padding:0.6em 0.8em;background:#fff8e1;border:1px solid #f0d97c;border-radius:6px;font-size:0.85em;line-height:1.45;">
    💡 Looks like an HTTPS URL. Want to <button type="button" id="ssh-suggest-apply" style="width:auto;padding:0.15em 0.6em;background:transparent;border:1px solid currentColor;border-radius:4px;cursor:pointer;font-size:0.9em;">use the SSH form</button>
    instead? It's <code data-zetl-suggest-target></code>
    — your existing SSH key works.
  </div>
  <details style="margin-top:0.6em;font-size:0.85em;opacity:0.75;">
    <summary style="cursor:pointer;">Advanced: private HTTPS (personal access token)</summary>
    <p style="margin:0.4em 0 0.3em;font-size:0.9em;">
      <strong>Prefer SSH</strong> — it's auto-configured by this app. Only fall back to a PAT if your
      git host blocks SSH or you have a specific HTTPS-only constraint. The PAT is used once for the
      clone and never written to disk; pull / push will need it re-entered until v0.1.x adds
      keychain storage for HTTPS credentials.
    </p>
    <input type="password" name="pat" placeholder="ghp_ / glpat_ / your-pat-here" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false">
  </details>
  <button type="submit">Clone vault →</button>
</form>
<script>
// Detect HTTPS URLs for known git hosts and offer an SSH-form
// substitute. Keep it dumb on purpose: a regex with a 4-host whitelist
// covers GitHub / GitLab / Codeberg / Gitea-self-host patterns.
(function() {{
  var input = document.getElementById('remote_url');
  var box = document.getElementById('ssh-suggest');
  var slot = document.querySelector('[data-zetl-suggest-target]');
  var apply = document.getElementById('ssh-suggest-apply');
  if (!input || !box || !slot || !apply) return;
  function toSsh(url) {{
    var m = url.match(/^https?:\/\/([^\/]+)\/([^?#]+?)(?:\.git)?\/?$/);
    if (!m) return null;
    var host = m[1], path = m[2];
    if (host === 'github.com' || host === 'gitlab.com' || host === 'codeberg.org' || /^gitea\./.test(host)) {{
      return 'git@' + host + ':' + path + '.git';
    }}
    return null;
  }}
  function refresh() {{
    var ssh = toSsh(input.value.trim());
    if (ssh) {{
      slot.textContent = ssh;
      box.hidden = false;
    }} else {{
      box.hidden = true;
    }}
  }}
  input.addEventListener('input', refresh);
  apply.addEventListener('click', function() {{
    var ssh = toSsh(input.value.trim());
    if (ssh) {{ input.value = ssh; refresh(); }}
  }});
}})();
</script>

<details>
  <summary>Advanced: use my desktop's BIP39 seed phrase instead</summary>
  <p>Pasting your 12-word recovery phrase from <code>zetl derive-ssh-key --mnemonic</code> on desktop will <strong>replace</strong> the auto-generated per-device key above with the deterministic one derived from your seed. Only do this if you want this phone to share an identity with your desktop. The seed is never written to disk.</p>
  <form method="post" action="/_mobile/onboarding/seed">
    <textarea name="mnemonic" placeholder="word1 word2 … word12" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" required></textarea>
    <button type="submit">Replace key with seed-derived one →</button>
  </form>
</details>
{back}
<script>
document.querySelectorAll('form[data-zetl-busy-label]').forEach(function(f) {{
  f.addEventListener('submit', function() {{
    f.classList.add('zetl-busy');
    var btn = f.querySelector('button');
    if (btn) btn.textContent = f.dataset.zetlBusyLabel;
  }});
}});
</script>
{external_js}
</body></html>"#,
        pub = html_escape(pub_line),
        back = render_onboarding_back_link(),
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// JS snippet shared by every `/_mobile/*` page. Intercepts clicks on
/// `target="_blank"` links and forwards them to the Tauri
/// `plugin:opener|open_url` IPC command so the OS opens them in the
/// system browser instead of the Tauri WebView (where the user
/// would be trapped with no back button).
///
/// In a regular browser (e.g. `ar-crawl session` for testing) the
/// Tauri internals are absent and the click falls through to the
/// browser's default `_blank` handling.
const MOBILE_EXTERNAL_LINK_JS: &str = r#"<script>
document.addEventListener('click', function(e) {
  var a = e.target.closest('a[target="_blank"]');
  if (!a) return;
  if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
    e.preventDefault();
    window.__TAURI_INTERNALS__.invoke('plugin:opener|open_url', { url: a.href }).catch(function(err) {
      console.error('opener invoke failed:', err);
      window.open(a.href, '_blank');
    });
  }
});
</script>"#;

/// Standard top-bar for `/_mobile/*` pages with a `← Vaults` back
/// link. The pages that already have a more specific back affordance
/// (`/_mobile/capture` has `✕ Cancel`, the picker has `← Back to
/// Vaults` in the footer) skip this. `/_mobile/vaults` itself is the
/// destination and renders no back link.
fn render_topbar_back_to_vaults() -> &'static str {
    r#"<div data-zetl-mobile-topbar style="display:flex;align-items:center;gap:0.5em;margin-bottom:1em;font-size:0.85em;">
  <a href="/_mobile/vaults" style="color:inherit;text-decoration:none;padding:0.3em 0.6em;border:1px solid currentColor;border-radius:6px;opacity:0.75;">← Vaults</a>
</div>"#
}

fn render_remove_confirm_page(label: &str) -> String {
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · confirm remove</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  .warn {{ background: #fee; border: 1px solid #f0a0a0; padding: 0.8em 1em; border-radius: 6px; margin-bottom: 1.2em; line-height: 1.5; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; box-sizing: border-box; }}
  .danger {{ background: #b00; color: #fff; border: 1px solid #800; }}
  .cancel {{ background: transparent; color: inherit; border: 1px solid currentColor; opacity: 0.7; }}
</style></head>
<body data-zetl-mobile-route="vaults-remove-confirm" data-zetl-mobile-vault-label="{label}">
<h1>Remove this vault from local storage?</h1>
<div class="warn">
  <strong>{label}</strong><br>
  The git remote is NOT affected — you can re-clone any time via <em>+ Add another vault</em>. But <strong>any unpushed local changes in this vault will be lost</strong>.
</div>
<form method="post" action="/_mobile/vaults/remove">
  <input type="hidden" name="label" value="{label}">
  <button type="submit" class="danger" data-zetl-mobile-action="remove-confirm">Yes, remove '{label}'</button>
</form>
<form action="/_mobile/vaults" method="get">
  <button type="submit" class="cancel">Cancel</button>
</form>
</body></html>"#,
        label = html_escape(label),
    )
}

fn render_pick_subpath(
    label: &str,
    candidates: &[crate::mobile_state::VaultSubpathCandidate],
    selected_subpath: &str,
) -> String {
    let rows = if candidates.is_empty() {
        r#"<p class="hint">No directories with markdown files were found in this repo. The vault is treated as empty until you (or another peer) add content.</p>
<form method="post" action="/_mobile/vaults/pick">
  <input type="hidden" name="label" value="">
  <input type="hidden" name="subpath" value="">
</form>"#
            .to_string()
    } else {
        let opts = candidates
            .iter()
            .map(|c| {
                let display = if c.subpath.is_empty() {
                    "/ (repo root)".to_string()
                } else {
                    format!("/ {}", c.subpath)
                };
                let zetl_badge = if c.has_zetl_dir {
                    r#" <span style="color:#063;font-size:0.85em;">● .zetl/</span>"#
                } else {
                    ""
                };
                let checked = if c.subpath == selected_subpath {
                    " checked"
                } else {
                    ""
                };
                format!(
                    r#"<label style="display:block;padding:0.7em;border:1px solid #ddd;border-radius:6px;margin-bottom:0.5em;cursor:pointer;">
  <input type="radio" name="subpath" value="{value}"{checked}>
  <strong>{display}</strong>{zetl_badge}
  <br><span style="font-size:0.85em;opacity:0.7;">{md_count} markdown file{plural}</span>
</label>"#,
                    value = html_escape(&c.subpath),
                    checked = checked,
                    display = html_escape(&display),
                    zetl_badge = zetl_badge,
                    md_count = c.md_count,
                    plural = if c.md_count == 1 { "" } else { "s" },
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"<form method="post" action="/_mobile/vaults/pick">
  <input type="hidden" name="label" value="{label}">
  {opts}
  <button type="submit">Use this folder →</button>
</form>"#,
            label = html_escape(label),
            opts = opts,
        )
    };

    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · choose folder</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 0.4rem; }}
  .hint {{ font-size: 0.9em; opacity: 0.75; line-height: 1.5; margin-bottom: 1em; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  .links {{ font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }}
  .links a {{ color: inherit; }}
</style></head>
<body data-zetl-mobile-route="pick-subpath" data-zetl-mobile-vault-label="{label}">
<h1>Which folder is the vault?</h1>
<p class="hint">The repo <code>{label}</code> has more than one directory that looks like it could be a zetl vault. Pick the one to use. (Folders with a <code>.zetl/</code> config dir are usually the right choice.)</p>
{rows}
<div class="links"><a href="/_mobile/vaults">← Back to Vaults</a></div>
{external_js}
</body></html>"#,
        label = html_escape(label),
        rows = rows,
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

fn render_recovery_page() -> String {
    let keystore = crate::mobile_state::global();
    let mnemonic = keystore.mnemonic().unwrap_or_default();
    let pub_line = keystore.pub_openssh().unwrap_or_default();

    if mnemonic.is_empty() {
        // v1-schema install (pre-mnemonic) or no key loaded.
        return r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · recovery</title>
<style>
  body { font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }
  h1 { font-size: 1.2rem; margin: 0 0 1rem; }
  .hint { font-size: 0.9em; opacity: 0.7; }
  .links { font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }
  .links a { color: inherit; }
</style></head>
<body data-zetl-mobile-route="recovery">
<h1>No recovery phrase available</h1>
<p class="hint">This install pre-dates BIP39 persistence — there is no saved seed phrase for the SSH key in use. To get one, <a href="/_mobile/sync">reset the active vault</a> (you'll lose unpushed local edits in that vault only — other vaults are preserved), then re-onboard. The fresh onboarding generates and saves a new recovery phrase.</p>
<div class="links"><a href="/_mobile/vaults">Vaults</a> · <a href="/_mobile/sync">Sync</a></div>
</body></html>"#.to_string();
    }

    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · recovery</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  .hint {{ font-size: 0.9em; opacity: 0.75; line-height: 1.5; }}
  .warn {{ background: #fff8e1; border: 1px solid #f0d97c; padding: 0.7em 0.9em; border-radius: 6px; margin-bottom: 1em; }}
  .seed-box {{ position: relative; background: #f4f4f4; padding: 0.9em; border-radius: 6px; font-family: ui-monospace, monospace; font-size: 1rem; line-height: 1.55; word-spacing: 0.4em; user-select: all; }}
  .seed-blur {{ filter: blur(6px); transition: filter 200ms; cursor: pointer; }}
  .seed-blur::after {{ content: 'Tap to reveal'; position: absolute; inset: 0; display: grid; place-items: center; font-family: system-ui; font-size: 0.85em; filter: blur(0); background: rgba(244,244,244,0.7); border-radius: 6px; cursor: pointer; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  .copy-btn {{ width: auto; padding: 0.3em 0.7em; font-size: 0.85em; margin: 0.4em 0 0; }}
  .links {{ font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }}
  .links a {{ color: inherit; }}
  details {{ margin-top: 1.5em; padding-top: 1em; border-top: 1px solid currentColor; opacity: 0.8; }}
  details summary {{ cursor: pointer; font-size: 0.9em; }}
  pre {{ background: #f4f4f4; padding: 0.6em; border-radius: 6px; font-size: 0.78em; word-break: break-all; white-space: pre-wrap; margin: 0.4em 0; }}
</style></head>
<body data-zetl-mobile-route="recovery">
<h1>Recovery phrase</h1>
<div class="warn">
  <strong>Write these 12 words down</strong> and store them somewhere safe (paper notebook, password manager). Anyone with these words can derive the same SSH key and access whatever you've granted that key. This phrase is also what lets you restore the same identity on a new device — without it, a lost device means a new identity and re-adding the new pubkey to your git host.
</div>
<div class="seed-box seed-blur" data-zetl-mobile-seed onclick="this.classList.remove('seed-blur')">{mnemonic}</div>
<button type="button" class="copy-btn" onclick="navigator.clipboard.writeText(document.querySelector('[data-zetl-mobile-seed]').textContent).then(() =&gt; {{ this.textContent = 'Copied'; setTimeout(() =&gt; this.textContent = 'Copy phrase', 1500); }})">Copy phrase</button>

<details>
  <summary>SSH public key (for git-host setup)</summary>
  <p class="hint" style="margin-top:0.4em;">This is what you paste into GitHub / GitLab / Codeberg's SSH keys page. Derived from the phrase above.</p>
  <pre data-zetl-mobile-pubkey>{pub_line}</pre>
</details>

<div class="links"><a href="/_mobile/vaults">Vaults</a> · <a href="/_mobile/sync">Sync</a> · <a href="/">Pages</a></div>
{external_js}
</body></html>"#,
        mnemonic = html_escape(&mnemonic),
        pub_line = html_escape(&pub_line),
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

fn render_vaults_page(msg: Option<VaultsMsg>) -> String {
    let banner = match msg {
        Some(VaultsMsg::Error(text)) => format!(
            r#"<div data-zetl-mobile-vaults-msg="error" style="color:#b00;background:#fee;padding:0.7em;border-radius:6px;margin-bottom:1em;">Error: {}</div>"#,
            html_escape(&text)
        ),
        Some(VaultsMsg::Ok(text)) => format!(
            r#"<div data-zetl-mobile-vaults-msg="ok" style="color:#063;background:#efe;padding:0.7em;border-radius:6px;margin-bottom:1em;">{}</div>"#,
            html_escape(&text)
        ),
        None => String::new(),
    };

    let entries = crate::mobile_state::list_vaults();
    let rows = if entries.is_empty() {
        r#"<p class="hint">No vaults cloned yet. <a href="/_mobile/onboarding">Add one →</a></p>"#.to_string()
    } else {
        entries
            .iter()
            .map(|v| {
                let active_tag = if v.is_active {
                    r#"<span style="color:#063;font-size:0.8em;">● active</span>"#
                } else {
                    ""
                };
                let switch_form = if v.is_active {
                    String::new()
                } else {
                    format!(
                        r#"<form method="post" action="/_mobile/vaults/switch" style="display:inline;margin-left:0.4em;">
  <input type="hidden" name="label" value="{label}">
  <button type="submit" style="width:auto;padding:0.3em 0.7em;font-size:0.85em;">Switch</button>
</form>"#,
                        label = html_escape(&v.label)
                    )
                };
                let remove_link = format!(
                    r#"<button type="button" data-zetl-mobile-remove="{label}" style="margin-left:0.4em;padding:0.3em 0.7em;font-size:0.85em;background:#fee;color:#b00;border:1px solid #b00;border-radius:6px;cursor:pointer;font-family:inherit;width:auto;">Remove</button>"#,
                    label = html_escape(&v.label),
                );
                format!(
                    r#"<li style="margin:0.6em 0;line-height:1.5;">
  <strong data-zetl-mobile-vault-label="{label}">{label}</strong> {active_tag}{switch_form}{remove_link}
  <br><span style="font-size:0.8em;opacity:0.7;">{remote}</span>
</li>"#,
                    label = html_escape(&v.label),
                    active_tag = active_tag,
                    switch_form = switch_form,
                    remove_link = remove_link,
                    remote = html_escape(v.remote_url.as_deref().unwrap_or("(no remote)")),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · vaults</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  ul {{ list-style: none; padding: 0; }}
  .hint {{ font-size: 0.9em; opacity: 0.7; }}
  .links {{ font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }}
  .links a {{ color: inherit; }}
</style></head>
<body data-zetl-mobile-route="vaults">
<h1>Vaults</h1>
{banner}
<ul data-zetl-mobile-vaults-list>{rows}</ul>
<p><a href="/_mobile/onboarding?add=1"><button type="button">+ Add another vault</button></a></p>
<div class="links"><a href="/">Pages</a> · <a href="/_mobile/sync">Sync</a> · <a href="/_mobile/capture">Capture</a></div>

<!-- In-page confirmation modal for Remove. Custom dialog (not
     window.confirm) because Tauri WKWebView silently suppresses the
     native confirm() unless tauri-plugin-dialog is wired up. -->
<div id="zetl-confirm-overlay" data-zetl-modal="closed" style="display:none;position:fixed;inset:0;background:rgba(0,0,0,0.45);z-index:1000;align-items:center;justify-content:center;padding:1.5em;">
  <div role="dialog" aria-modal="true" aria-labelledby="zetl-confirm-title" style="background:#fff;color:#111;max-width:28em;width:100%;padding:1.5em;border-radius:8px;box-shadow:0 8px 32px rgba(0,0,0,0.2);">
    <h2 id="zetl-confirm-title" style="font-size:1.1rem;margin:0 0 0.7em;">Remove this vault from local storage?</h2>
    <p style="line-height:1.5;margin:0 0 1em;"><strong id="zetl-confirm-label" style="font-family:ui-monospace,monospace;"></strong></p>
    <p style="line-height:1.5;margin:0 0 1.2em;font-size:0.92em;opacity:0.85;">The git remote is NOT affected — you can re-clone any time via <em>+ Add another vault</em>. <strong>Any unpushed local changes in this vault will be lost.</strong></p>
    <form method="post" action="/_mobile/vaults/remove" id="zetl-confirm-form" style="margin:0;">
      <input type="hidden" name="label" id="zetl-confirm-input" value="">
      <button type="submit" data-zetl-mobile-action="remove-confirm" style="background:#b00;color:#fff;border:1px solid #800;width:100%;padding:0.75em;font-size:1rem;border-radius:6px;cursor:pointer;font-family:inherit;">Yes, remove</button>
    </form>
    <button type="button" id="zetl-confirm-cancel" style="background:transparent;color:inherit;border:1px solid currentColor;width:100%;padding:0.75em;font-size:1rem;margin-top:0.5em;border-radius:6px;cursor:pointer;font-family:inherit;opacity:0.75;">Cancel</button>
  </div>
</div>

<script>
(function() {{
  var overlay = document.getElementById('zetl-confirm-overlay');
  var input = document.getElementById('zetl-confirm-input');
  var labelEl = document.getElementById('zetl-confirm-label');
  var cancel = document.getElementById('zetl-confirm-cancel');
  var form = document.getElementById('zetl-confirm-form');
  if (!overlay || !input || !labelEl || !cancel || !form) return;
  function open(label) {{
    input.value = label;
    labelEl.textContent = label;
    overlay.style.display = 'flex';
    overlay.dataset.zetlModal = 'open';
  }}
  function close() {{
    overlay.style.display = 'none';
    overlay.dataset.zetlModal = 'closed';
  }}
  document.querySelectorAll('button[data-zetl-mobile-remove]').forEach(function(btn) {{
    btn.addEventListener('click', function(e) {{
      e.preventDefault();
      open(btn.dataset.zetlMobileRemove);
    }});
  }});
  cancel.addEventListener('click', close);
  overlay.addEventListener('click', function(e) {{
    if (e.target === overlay) close();
  }});
  document.addEventListener('keydown', function(e) {{
    if (e.key === 'Escape' && overlay.dataset.zetlModal === 'open') close();
  }});
}})();
</script>
{external_js}
</body></html>"#,
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

fn render_capture_form(error: Option<&str>, title_prefill: &str, body_prefill: &str) -> String {
    let error_block = match error {
        Some(msg) => format!(
            r#"<div data-zetl-mobile-error="capture" style="color:#b00;background:#fee;padding:0.75em;border-radius:6px;margin-bottom:1em;">Error: {}</div>"#,
            html_escape(msg)
        ),
        None => String::new(),
    };
    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · capture</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  input[type="text"] {{ width: 100%; font-size: 1rem; padding: 0.6em; box-sizing: border-box; margin-bottom: 0.6em; }}
  textarea {{ width: 100%; min-height: 12em; font-family: ui-monospace, monospace; font-size: 1rem; padding: 0.6em; box-sizing: border-box; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  h1 {{ font-size: 1.2rem; margin: 0; }}
  .hint {{ font-size: 0.85em; opacity: 0.65; margin-top: 0.2em; }}
  .links {{ font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }}
  .links a {{ color: inherit; }}
  .topbar {{ display: flex; align-items: center; justify-content: space-between; margin-bottom: 1em; }}
  .cancel {{ font-size: 0.9em; opacity: 0.8; color: inherit; text-decoration: none; padding: 0.3em 0.6em; border: 1px solid currentColor; border-radius: 6px; }}
  .cancel:hover {{ opacity: 1; }}
</style></head>
<body data-zetl-mobile-route="capture">
<div class="topbar">
  <h1>Capture</h1>
  <a href="/" class="cancel" data-zetl-mobile-action="cancel">✕ Cancel</a>
</div>
{error_block}
<form method="post" action="/_mobile/capture">
  <input type="text" name="title" placeholder="Title (optional — auto from first line / timestamp)" value="{title_val}" autocomplete="off" autocapitalize="sentences" autocorrect="off" spellcheck="false">
  <textarea name="content" placeholder="Markdown — [[wikilinks]] welcome" required autofocus>{body_val}</textarea>
  <div class="hint">Saved as <code>&lt;title&gt;.md</code> in the vault root, committed locally, pushed if online.</div>
  <button type="submit">Save</button>
</form>
<div class="links"><a href="/">Pages</a> · <a href="/_mobile/sync">Sync</a> · <a href="/_mobile/vaults">Vaults</a></div>
{external_js}
</body></html>"#,
        title_val = html_escape(title_prefill),
        body_val = html_escape(body_prefill),
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}

fn render_sync_page(msg: Option<SyncMsg>) -> String {
    let other_count = crate::mobile_state::list_vaults()
        .iter()
        .filter(|v| !v.is_active)
        .count();
    let switcher_link = if other_count > 0 {
        format!(
            r#"<p class="hint" style="margin-top:0.4em;"><a href="/_mobile/vaults">Switch vault ({} other)</a></p>"#,
            other_count
        )
    } else {
        r#"<p class="hint" style="margin-top:0.4em;"><a href="/_mobile/vaults">Manage vaults</a></p>"#.to_string()
    };
    let vault_header = match crate::mobile_state::vault_meta() {
        Some(meta) => format!(
            r#"<div data-zetl-mobile-vault-label="{label}" style="font-size:0.95em;background:#f4f4f4;padding:0.6em 0.8em;border-radius:6px;margin-bottom:1em;">
  <strong>Vault:</strong> {label}<br>
  <span style="font-size:0.8em;opacity:0.7;">{remote}</span>
  {switcher}
</div>"#,
            label = html_escape(&meta.label),
            remote = html_escape(&meta.remote_url),
            switcher = switcher_link,
        ),
        None => switcher_link.clone(),
    };
    let banner = match msg {
        Some(SyncMsg::Ok(text)) => format!(
            r#"<div data-zetl-mobile-sync="ok" style="color:#063;background:#efe;padding:0.7em;border-radius:6px;margin-bottom:1em;">{}</div>"#,
            html_escape(&text)
        ),
        Some(SyncMsg::Conflict) => format!(
            r#"<div data-zetl-mobile-sync="conflict" style="color:#b00;background:#fee;padding:0.7em;border-radius:6px;margin-bottom:1em;">{}</div>"#,
            "Remote diverged from local — resolve on desktop, then pull again. Push is blocked until the next fast-forward pull succeeds."
        ),
        Some(SyncMsg::Error(text)) => format!(
            r#"<div data-zetl-mobile-sync="error" style="color:#b00;background:#fee;padding:0.7em;border-radius:6px;margin-bottom:1em;">Error: {}</div>"#,
            html_escape(&text)
        ),
        None => String::new(),
    };

    let key_loaded = crate::mobile_state::global().is_loaded();
    let key_block = if key_loaded {
        r#"<p class="hint">SSH key loaded.</p>"#.to_string()
    } else {
        r#"<p class="hint" data-zetl-mobile-keystore="empty">No SSH key in keystore — visit <a href="/_mobile/onboarding">onboarding</a> to paste your seed.</p>"#
            .to_string()
    };

    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>zetl mobile · sync</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  .hint {{ font-size: 0.85em; opacity: 0.7; }}
  .links {{ font-size: 0.85em; opacity: 0.7; margin-top: 1.5em; display: flex; gap: 1em; }}
  .links a {{ color: inherit; }}
  form {{ display: inline; }}
  form.zetl-busy button {{ opacity: 0.6; cursor: wait; }}
  form.zetl-busy button::after {{ content: ''; display: inline-block; width: 0.9em; height: 0.9em; border: 2px solid currentColor; border-top-color: transparent; border-radius: 50%; animation: zspin 0.8s linear infinite; margin-left: 0.6em; vertical-align: middle; }}
  @keyframes zspin {{ to {{ transform: rotate(360deg); }} }}
</style></head>
<body data-zetl-mobile-route="sync">
{topbar}
<h1>Sync</h1>
{vault_header}
{banner}
{key_block}
<form method="post" action="/_mobile/sync/pull" data-zetl-busy-label="Pulling…"><button type="submit">Pull (fast-forward only)</button></form>
<form method="post" action="/_mobile/sync/push" data-zetl-busy-label="Pushing…"><button type="submit">Push</button></form>
<hr style="margin:1.6em 0 1em;opacity:0.25;">
<details>
  <summary class="hint">Reset this vault…</summary>
  <p class="hint" style="margin-top:0.6em;">Removes the <strong>active</strong> vault's local working tree and forgets the SSH key. Other cloned vaults under <code>vaults/</code> are preserved. <strong>Anything not pushed will be lost.</strong></p>
  <form method="post" action="/_mobile/reset" onsubmit="return confirm('Wipe the active vault and forget the SSH key? Unpushed changes will be lost.');">
    <button type="submit" data-zetl-mobile-action="reset" style="background:#fee;color:#b00;border-color:#b00;">Reset active vault</button>
  </form>
</details>
<div class="links"><a href="/">Pages</a> · <a href="/_mobile/capture">Capture</a> · <a href="/_mobile/onboarding">Onboarding</a></div>
<script>
// Show a busy state on any form decorated with data-zetl-busy-label.
// The form still submits normally — we just style it as "in flight"
// so the user gets immediate visual feedback while the request runs.
document.querySelectorAll('form[data-zetl-busy-label]').forEach(function(f) {{
  f.addEventListener('submit', function() {{
    f.classList.add('zetl-busy');
    var btn = f.querySelector('button');
    if (btn) btn.textContent = f.dataset.zetlBusyLabel;
  }});
}});
</script>
{external_js}
</body></html>"#,
        topbar = render_topbar_back_to_vaults(),
        external_js = MOBILE_EXTERNAL_LINK_JS,
    )
}
