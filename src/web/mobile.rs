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
        .route("/_mobile/capture", get(capture_handler))
        .route("/_mobile/sync", get(sync_handler))
}

// ── /_mobile/onboarding ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SeedForm {
    mnemonic: String,
}

#[derive(Deserialize)]
struct CloneForm {
    remote_url: String,
}

/// `GET /_mobile/onboarding` — guided seed-import + remote-URL +
/// clone wizard. State-driven render: if no key has been imported,
/// show the seed-input form; if a key is present, show the pubkey +
/// the remote-URL / clone form.
async fn onboarding_handler() -> Response {
    let keystore = crate::mobile_state::global();

    let body = match keystore.pub_openssh() {
        None => render_step_seed(None),
        Some(pub_line) => render_step_clone(&pub_line, None),
    };
    Html(body).into_response()
}

/// `POST /_mobile/onboarding/seed` — accept a 12-word BIP39 mnemonic,
/// derive the ed25519 SSH key, store it in the process keystore.
async fn onboarding_seed_handler(Form(form): Form<SeedForm>) -> Response {
    let keystore = crate::mobile_state::global();
    match keystore.import_mnemonic(form.mnemonic.trim()) {
        Ok(_) => Redirect::to("/_mobile/onboarding").into_response(),
        Err(e) => Html(render_step_seed(Some(&format!("{e:#}")))).into_response(),
    }
}

/// `POST /_mobile/onboarding/clone` — clone the user-supplied git
/// remote into the registered vault root, using the in-keystore SSH
/// key for auth. On success, redirect to the page list (`/`); on
/// failure, re-render the clone form with the error.
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

    let vault_root = match crate::mobile_state::vault_root() {
        Some(p) => p,
        None => {
            return Html(render_step_clone(
                &pub_line,
                Some("vault root not registered — Tauri shell did not initialise"),
            ))
            .into_response();
        }
    };

    let remote_url = form.remote_url.trim().to_string();

    // The clone is blocking I/O against libgit2. Run it on a blocking
    // pool thread so the axum request task is not held for the full
    // duration of the network fetch.
    let clone_root = vault_root.clone();
    let clone_url = remote_url.clone();
    let clone_result =
        tokio::task::spawn_blocking(move || crate::mobile_git::clone(&clone_url, &clone_root))
            .await;

    match clone_result {
        Ok(Ok(_repo)) => Redirect::to("/").into_response(),
        Ok(Err(e)) => Html(render_step_clone(&pub_line, Some(&format!("{e:#}")))).into_response(),
        Err(join_err) => Html(render_step_clone(
            &pub_line,
            Some(&format!("clone task panicked: {join_err}")),
        ))
        .into_response(),
    }
}

// ── /_mobile/capture (placeholder) ────────────────────────────────────────────

async fn capture_handler() -> Response {
    Html(MOBILE_PLACEHOLDER_CAPTURE).into_response()
}

// ── /_mobile/sync (placeholder) ───────────────────────────────────────────────

async fn sync_handler() -> Response {
    Html(MOBILE_PLACEHOLDER_SYNC).into_response()
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
</body></html>"#,
    )
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
<title>zetl mobile · onboarding · step 2</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 30em; margin: 0 auto; padding: 1.5em; }}
  pre {{ background: #f4f4f4; padding: 0.7em; border-radius: 6px; word-break: break-all; white-space: pre-wrap; font-size: 0.85rem; }}
  input[type="url"] {{ width: 100%; font-family: ui-monospace, monospace; font-size: 1rem; padding: 0.6em; box-sizing: border-box; }}
  button {{ width: 100%; padding: 0.8em; font-size: 1rem; margin-top: 0.6em; }}
  .step {{ font-size: 0.85em; opacity: 0.65; margin-bottom: 0.4em; }}
  h1 {{ font-size: 1.2rem; margin: 0 0 1rem; }}
  p {{ line-height: 1.5; }}
  .copy {{ font-size: 0.85em; }}
</style></head>
<body data-zetl-mobile-route="onboarding" data-zetl-mobile-step="clone">
<div class="step">Step 2 of 2</div>
<h1>Add this SSH key to your git host, then clone</h1>
<p>Add the public key below to <em>Codeberg / GitHub / Gitea / your SSH-config</em> as a deploy key (or your account key). Then paste your vault's git remote URL and tap Clone.</p>
<pre data-zetl-mobile-pubkey>{pub}</pre>
<p class="copy"><button type="button" onclick="navigator.clipboard.writeText(document.querySelector('[data-zetl-mobile-pubkey]').textContent).then(() =&gt; {{ this.textContent = 'Copied'; }})">Copy public key</button></p>
{error_block}
<form method="post" action="/_mobile/onboarding/clone">
  <input type="url" name="remote_url" placeholder="git@codeberg.org:you/your-vault.git" autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" required>
  <button type="submit">Clone vault →</button>
</form>
</body></html>"#,
        pub = html_escape(pub_line),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const MOBILE_PLACEHOLDER_CAPTURE: &str = "\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>zetl mobile · capture</title></head>\
<body data-zetl-mobile-route=\"capture\">\
<h1>zetl mobile · capture</h1>\
<p>Placeholder. Capture form lands with POST /_mobile/capture.</p>\
</body></html>";

const MOBILE_PLACEHOLDER_SYNC: &str = "\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>zetl mobile · sync</title></head>\
<body data-zetl-mobile-route=\"sync\">\
<h1>zetl mobile · sync</h1>\
<p>Placeholder. Sync status + pull/push controls land with git module.</p>\
</body></html>";
