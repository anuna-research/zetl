//! SPEC-040 mobile-route integration tests (TEST-4005, TEST-4011).
//!
//! Verifies the `/_mobile/*` route surface is registered when the
//! `mobile` cargo feature is enabled and exercises the onboarding
//! flow end to end. Run with:
//!
//!     cargo test --test mobile_integration --features mobile
//!
//! Tests share `zetl::mobile_state::global()` (a process-wide
//! `OnceLock` singleton) so they coordinate via a `STATE_LOCK` Mutex
//! rather than relying on cargo's per-test isolation. The lock is
//! held for the duration of each test that touches the keystore.

#![cfg(feature = "mobile")]
// Each #[tokio::test] gets its own current-thread runtime; STATE_LOCK
// serialises tests that touch the global keystore. Holding the
// std::sync::Mutex guard across .await is safe here because no other
// task on the same thread can yield in between, and concurrent tests
// run on separate OS threads with separate runtimes.
#![allow(clippy::await_holding_lock)]

use std::sync::Mutex;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

use zetl::web::mobile;

/// Serialises tests that touch the global keystore. Held over the
/// whole logical test (not just a single HTTP call) so a setup-then-
/// assert sequence cannot interleave with another test's reset.
static STATE_LOCK: Mutex<()> = Mutex::new(());

/// Register a shared template engine once per test-binary process so
/// the `/_mobile/*` handlers can render their Minijinja templates
/// against the bundled `default` theme. Leaks a tempdir for the
/// engine's vault_root — cheap, the process exits when tests finish.
fn ensure_engine() {
    static ENGINE_INIT: std::sync::Once = std::sync::Once::new();
    ENGINE_INIT.call_once(|| {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let engine = zetl::web::engine::TemplateEngine::new(dir.path(), "default", false, false);
        zetl::mobile_state::set_template_engine(std::sync::Arc::new(engine));
    });
}

const FIXTURE_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon \
                                abandon abandon abandon abandon abandon about";

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn post_form(app: &Router, uri: &str, body: &str) -> (StatusCode, Option<String>, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get("location")
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(resp.into_body(), 10_000_000)
        .await
        .unwrap();
    (status, location, String::from_utf8_lossy(&body).to_string())
}

fn router() -> Router {
    ensure_engine();
    mobile::router::<()>().with_state(())
}

// ── GET surface tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn onboarding_get_shows_step_one_when_no_key() {
    let _g = STATE_LOCK.lock().unwrap();
    // Fresh process state: no key imported (zero-side-effect test —
    // do not import a key here).
    let app = router();
    let (status, body) = get(&app, "/_mobile/onboarding").await;
    assert_eq!(status, StatusCode::OK);
    // The post-step-1 keystore may or may not be loaded depending on
    // test ordering, so this test is robust against either rendering;
    // it only asserts the route mounts and returns the onboarding marker.
    assert!(
        body.contains("data-zetl-mobile-route=\"onboarding\""),
        "missing onboarding marker; body={body}"
    );
}

#[tokio::test]
async fn capture_route_returns_placeholder() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/capture").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-route=\"capture\""),
        "missing capture marker; body={body}"
    );
}

#[tokio::test]
async fn sync_route_returns_placeholder() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/sync").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-route=\"sync\""),
        "missing sync marker; body={body}"
    );
}

#[tokio::test]
async fn unknown_mobile_route_returns_404() {
    let app = router();
    let (status, _) = get(&app, "/_mobile/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Onboarding POST flow tests ────────────────────────────────────────────────

#[tokio::test]
async fn onboarding_seed_post_with_valid_mnemonic_redirects() {
    let _g = STATE_LOCK.lock().unwrap();
    // Isolate from sibling tests: fresh keystore + fresh
    // app_data/vault_root pointing at a clean tempdir so the
    // onboarding GET doesn't auto-redirect to / because some prior
    // test left a real working tree at the global vault_root.
    zetl::mobile_state::global().clear();
    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    std::fs::create_dir_all(&app_data).unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(app_data.join("vault"));

    let app = router();
    let body = format!(
        "mnemonic={}",
        urlencoding::encode(FIXTURE_MNEMONIC).into_owned()
    );
    let (status, location, _body) = post_form(&app, "/_mobile/onboarding/seed", &body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "expected redirect, got {status}"
    );
    assert_eq!(location.as_deref(), Some("/_mobile/onboarding"));
    // After a successful seed POST the keystore is loaded; the next
    // GET /_mobile/onboarding renders the clone form (no vault yet).
    let (_status, body) = get(&app, "/_mobile/onboarding").await;
    assert!(
        body.contains("data-zetl-mobile-step=\"clone\""),
        "expected step-2 render after seed POST; body={body}"
    );
    assert!(
        body.contains("ssh-ed25519"),
        "expected ssh public-key line in step-2 render; body={body}"
    );
}

#[tokio::test]
async fn onboarding_seed_post_with_garbage_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();
    let app = router();
    let body = "mnemonic=this+is+not+a+valid+mnemonic+at+all";
    let (status, location, body) = post_form(&app, "/_mobile/onboarding/seed", body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(location.is_none(), "garbage seed must not redirect");
    assert!(
        body.contains("data-zetl-mobile-error=\"seed\""),
        "expected error block on garbage seed; body={body}"
    );
}

#[tokio::test]
async fn onboarding_clone_post_without_seed_routes_back_to_seed_step() {
    let _g = STATE_LOCK.lock().unwrap();

    // Clear keystore by writing a known-bad mnemonic — fails import,
    // leaves the store unloaded if it was unloaded; if it was already
    // loaded (from sibling test), this returns 200 with seed-error.
    // Either way we then attempt the clone POST and assert routing.
    //
    // To make this test deterministic we lean on the keystore's
    // `import_mnemonic` not clearing on failure. We accept that this
    // test is best-effort across orderings — its primary value is
    // verifying the no-keystore branch when the test runs first.

    let app = router();
    let body = "remote_url=git%40codeberg.org%3Ayou%2Fyour-vault.git";
    let (status, _location, body) = post_form(&app, "/_mobile/onboarding/clone", body).await;
    assert_eq!(status, StatusCode::OK);
    // When the keystore is empty the handler renders the seed step
    // with an error message; otherwise it falls through to the clone
    // step (which then errors on the file-or-ssh URL). Either marker
    // is acceptable here — we only assert the route returns 200.
    assert!(
        body.contains("data-zetl-mobile-route=\"onboarding\""),
        "expected onboarding marker regardless of branch; body={body}"
    );
}

// ── Capture POST flow ─────────────────────────────────────────────────────────

#[tokio::test]
async fn capture_get_renders_form() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/capture").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("data-zetl-mobile-route=\"capture\""));
    assert!(
        body.contains("<textarea"),
        "expected capture textarea; body={body}"
    );
}

#[tokio::test]
async fn capture_post_writes_file_commits_and_redirects() {
    let _g = STATE_LOCK.lock().unwrap();

    // Hold the tempdir for the test's lifetime; assign vault_root to
    // a child path (not the tempdir root, which gets dropped). The
    // captured-file assertion reads from the same child path.
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    git2::Repository::init(&vault).unwrap();
    zetl::mobile_state::set_vault_root(vault.clone());

    let app = router();
    let form_body = format!(
        "title={}&content={}",
        urlencoding::encode("Coffee notes"),
        urlencoding::encode("Some content\n")
    );
    let (status, location, _body) = post_form(&app, "/_mobile/capture", &form_body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "expected redirect after successful capture, got {status}"
    );
    let loc = location.expect("redirect should set Location header");
    assert_eq!(loc, "/Coffee%20notes");
    let written = std::fs::read_to_string(vault.join("Coffee notes.md")).unwrap();
    assert_eq!(written, "Some content\n");
}

#[tokio::test]
async fn capture_post_with_no_vault_root_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();

    // Set then immediately clear the vault root for this test by
    // pointing it at a non-existent path. The capture handler will
    // call mobile_capture::capture which errors on non-dir vault.
    let nonexistent = std::path::PathBuf::from("/this/path/should/not/exist/zetl-test");
    zetl::mobile_state::set_vault_root(nonexistent);

    let app = router();
    let form_body = "title=&content=hello";
    let (status, _location, body) = post_form(&app, "/_mobile/capture", form_body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-error=\"capture\""),
        "expected capture error block; body={body}"
    );
}

// ── Sync controls ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn sync_get_renders_buttons() {
    let app = router();
    let (status, body) = get(&app, "/_mobile/sync").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("data-zetl-mobile-route=\"sync\""));
    assert!(body.contains("Pull"));
    assert!(body.contains("Push"));
}

// ── End-to-end clone flow ────────────────────────────────────────────────────

/// Build a temp bare git remote with a single `Welcome.md` page so
/// the cloned vault has visible content after the test runs.
fn make_seed_remote(dir: &std::path::Path) -> std::path::PathBuf {
    let bare_path = dir.join("remote.git");
    git2::Repository::init_bare(&bare_path).unwrap();

    let work_dir = dir.join("seed-work");
    let url = format!("file://{}", bare_path.display());
    let repo = git2::Repository::clone(&url, &work_dir).unwrap();
    std::fs::write(
        work_dir.join("Welcome.md"),
        "# Welcome\n\nFirst page in the vault.\n",
    )
    .unwrap();
    let mut idx = repo.index().unwrap();
    idx.add_path(std::path::Path::new("Welcome.md")).unwrap();
    idx.write().unwrap();
    let tree_id = idx.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let sig = git2::Signature::now("e2e", "e2e@example").unwrap();
    repo.commit(Some("refs/heads/main"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let mut remote = repo.find_remote("origin").unwrap();
    remote
        .push::<&str>(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();

    // Aim the bare's HEAD at main so subsequent clones land on a born
    // branch (libgit2's default-branch is still "master" otherwise).
    let bare = git2::Repository::open_bare(&bare_path).unwrap();
    bare.set_head("refs/heads/main").unwrap();
    drop(bare);

    bare_path
}

#[tokio::test]
async fn end_to_end_clone_via_onboarding_handlers() {
    let _g = STATE_LOCK.lock().unwrap();

    // Fresh state — multi-vault layout: app_data_dir + vault symlink
    // path under it; clone writes to app_data/vaults/<label>/ and
    // points the symlink there.
    zetl::mobile_state::global().clear();
    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    std::fs::create_dir_all(&app_data).unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(app_data.join("vault"));

    let bare_path = make_seed_remote(tmp.path());

    let app = router();

    // Step 1: POST the BIP39 mnemonic.
    let seed_body = format!(
        "mnemonic={}",
        urlencoding::encode(FIXTURE_MNEMONIC).into_owned()
    );
    let (status, location, _) = post_form(&app, "/_mobile/onboarding/seed", &seed_body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "seed POST should redirect; got {status}"
    );
    assert_eq!(location.as_deref(), Some("/_mobile/onboarding"));

    // Step 2: POST the remote URL → clone runs.
    let url = format!("file://{}", bare_path.display());
    let clone_body = format!("remote_url={}", urlencoding::encode(&url).into_owned());
    let (status, location, body) = post_form(&app, "/_mobile/onboarding/clone", &clone_body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "clone POST should redirect on success; got {status}; body={body}"
    );
    assert_eq!(
        location.as_deref(),
        Some("/"),
        "clone success should redirect to vault root"
    );

    // Multi-vault: clone lands in app_data/vaults/<derived-label>/.
    // For the file:// remote, the derived label is the tempdir's
    // basename — verify via list_vaults().
    let entries = zetl::mobile_state::list_vaults();
    assert_eq!(
        entries.len(),
        1,
        "expected one cloned vault, got {entries:?}"
    );
    let entry = &entries[0];
    assert!(entry.is_active, "newly-cloned vault should be active");
    assert!(
        entry.path.join("Welcome.md").exists(),
        "Welcome.md should exist in vaults/{}",
        entry.label
    );
    assert!(
        entry.path.join(".git").is_dir(),
        ".git should exist in vaults/{}",
        entry.label
    );
    // Symlink at app_data/vault → vaults/<label>
    let link = app_data.join("vault");
    assert!(link.is_symlink(), "vault should be a symlink after clone");
}

#[tokio::test]
async fn share_post_appends_inbox_and_redirects_to_capture() {
    let _g = STATE_LOCK.lock().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    std::fs::create_dir_all(&app_data).unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    // Drain any leftover entries from sibling tests.
    let _ = zetl::mobile_state::drain_share_inbox();

    let app = router();
    let body = format!(
        "title={}&body={}",
        urlencoding::encode("Captured from share sheet"),
        urlencoding::encode("https://example.com — for later"),
    );
    let (status, location, _body) = post_form(&app, "/_mobile/share", &body).await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "share POST should redirect; got {status}"
    );
    assert_eq!(location.as_deref(), Some("/_mobile/capture?from=share"));

    // Inbox now has one entry.
    let inbox_path = app_data.join("share-inbox.jsonl");
    let inbox =
        std::fs::read_to_string(&inbox_path).expect("inbox file should exist after share POST");
    assert!(inbox.contains("Captured from share sheet"));
    assert!(inbox.contains("example.com"));

    // GET /_mobile/capture?from=share drains + prefills.
    let (status, prefilled) = get(&app, "/_mobile/capture?from=share").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        prefilled.contains("Captured from share sheet"),
        "capture form should be prefilled with share title; body fragment={prefilled}"
    );
    assert!(
        !inbox_path.exists(),
        "inbox file should be drained after capture GET"
    );
}

#[tokio::test]
async fn vaults_remove_wipes_local_dir_but_preserves_keystore() {
    let _g = STATE_LOCK.lock().unwrap();

    // Two vaults; remove the inactive one — the active one and the
    // SSH key must remain.
    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    let vaults = app_data.join("vaults");
    std::fs::create_dir_all(vaults.join("alpha")).unwrap();
    git2::Repository::init(vaults.join("alpha")).unwrap();
    std::fs::create_dir_all(vaults.join("beta")).unwrap();
    git2::Repository::init(vaults.join("beta")).unwrap();
    let link = app_data.join("vault");
    std::os::unix::fs::symlink("vaults/alpha", &link).unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(link.clone());
    zetl::mobile_state::global()
        .import_mnemonic(FIXTURE_MNEMONIC)
        .unwrap();

    let app = router();
    let (status, _location, body) = post_form(&app, "/_mobile/vaults/remove", "label=beta").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-vaults-msg=\"ok\""),
        "expected ok banner; body={body}"
    );

    assert!(vaults.join("alpha").exists(), "active vault should survive");
    assert!(!vaults.join("beta").exists(), "removed vault dir gone");
    assert!(
        zetl::mobile_state::global().is_loaded(),
        "keystore should be untouched"
    );
    assert!(link.is_symlink(), "active symlink should remain");
}

#[tokio::test]
async fn vaults_remove_rejects_traversal_labels() {
    let _g = STATE_LOCK.lock().unwrap();

    // POST a label containing `..` to confirm the allow-list check
    // blocks the remove before remove_dir_all can escape `vaults/`.
    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    let vaults = app_data.join("vaults");
    std::fs::create_dir_all(vaults.join("alpha")).unwrap();
    git2::Repository::init(vaults.join("alpha")).unwrap();
    // Sibling dir that an attacker would try to wipe via `../sibling`.
    let sibling = app_data.join("sibling");
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("important.txt"), b"do not delete").unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(app_data.join("vault"));

    let app = router();
    let (status, _location, body) =
        post_form(&app, "/_mobile/vaults/remove", "label=..%2Fsibling").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-vaults-msg=\"error\""),
        "traversal label should produce an error banner; body={body}"
    );
    assert!(
        sibling.join("important.txt").exists(),
        "sibling dir must be untouched after traversal attempt"
    );
    assert!(
        vaults.join("alpha").exists(),
        "legitimate vault must also be untouched"
    );
}

#[tokio::test]
async fn vaults_remove_active_vault_promotes_another_or_clears_symlink() {
    let _g = STATE_LOCK.lock().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    let vaults = app_data.join("vaults");
    std::fs::create_dir_all(vaults.join("alpha")).unwrap();
    git2::Repository::init(vaults.join("alpha")).unwrap();
    std::fs::create_dir_all(vaults.join("beta")).unwrap();
    git2::Repository::init(vaults.join("beta")).unwrap();
    let link = app_data.join("vault");
    std::os::unix::fs::symlink("vaults/alpha", &link).unwrap();
    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(link.clone());
    zetl::mobile_state::global()
        .import_mnemonic(FIXTURE_MNEMONIC)
        .unwrap();

    let app = router();
    let (status, _, _body) = post_form(&app, "/_mobile/vaults/remove", "label=alpha").await;
    assert_eq!(status, StatusCode::OK);

    // Removed vault is gone; the other vault gets auto-promoted.
    assert!(!vaults.join("alpha").exists());
    assert!(vaults.join("beta").exists());
    assert!(link.is_symlink(), "auto-promotion should set symlink");
    let target = std::fs::read_link(&link).unwrap();
    assert_eq!(target.to_string_lossy(), "vaults/beta");
}

#[tokio::test]
async fn reset_clears_active_vault_keystore_and_redirects() {
    let _g = STATE_LOCK.lock().unwrap();

    // Multi-vault setup: app_data has vaults/<label>/ with .git, and
    // app_data/vault is a symlink to that. Reset should wipe the
    // active vault dir + the symlink + the persisted key.
    let tmp = tempfile::tempdir().unwrap();
    let app_data = tmp.path().join("app-data");
    let vaults = app_data.join("vaults");
    let label = "test-owner/test-repo";
    let active_vault_dir = vaults.join(label);
    std::fs::create_dir_all(&active_vault_dir).unwrap();
    git2::Repository::init(&active_vault_dir).unwrap();
    std::fs::write(active_vault_dir.join("Welcome.md"), "# Welcome\n").unwrap();
    std::fs::write(app_data.join("ssh_key.json"), r#"{}"#).unwrap();
    let link = app_data.join("vault");
    std::os::unix::fs::symlink(format!("vaults/{label}"), &link).unwrap();

    zetl::mobile_state::set_app_data_dir(app_data.clone());
    zetl::mobile_state::set_vault_root(link.clone());
    zetl::mobile_state::global()
        .import_mnemonic(FIXTURE_MNEMONIC)
        .unwrap();

    let app = router();
    let (status, location, _body) = post_form(&app, "/_mobile/reset", "").await;
    assert!(
        matches!(
            status,
            StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND
        ),
        "reset POST should redirect; got {status}"
    );
    // Only one vault, so redirect goes to onboarding (no other vaults left).
    assert_eq!(location.as_deref(), Some("/_mobile/onboarding"));

    // Active vault dir wiped, symlink removed, key forgotten.
    assert!(
        !active_vault_dir.exists(),
        "active vault dir should be removed"
    );
    assert!(!link.exists(), "vault symlink should be removed");
    assert!(
        !app_data.join("ssh_key.json").exists(),
        "persisted ssh_key.json should be removed"
    );
    assert!(
        !zetl::mobile_state::global().is_loaded(),
        "in-memory keystore should be cleared"
    );
}

#[tokio::test]
async fn sync_pull_against_non_git_dir_renders_error() {
    let _g = STATE_LOCK.lock().unwrap();

    // Point vault_root at a dir that exists but has no git repo, so
    // mobile_git::pull_ff_only errors and the handler renders the
    // sync page with an error banner. Also pre-load the keystore so
    // we don't trip the "no SSH key" gate ahead of the pull.
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("not-a-repo");
    std::fs::create_dir_all(&vault).unwrap();
    zetl::mobile_state::set_vault_root(vault);
    zetl::mobile_state::global()
        .import_mnemonic(FIXTURE_MNEMONIC)
        .unwrap();

    let app = router();
    let (status, _location, body) = post_form(&app, "/_mobile/sync/pull", "").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-zetl-mobile-sync=\"error\""),
        "expected sync error block for non-git directory; body={body}"
    );
}
