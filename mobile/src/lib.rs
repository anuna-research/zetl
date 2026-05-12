//! SPEC-040 zetl mobile — Tauri shell entry point.
//!
//! The shell's only responsibilities are: (a) host a WebView,
//! (b) spawn the embedded `zetl serve` (single-user, mobile-feature)
//! at startup, (c) point the WebView at the embedded server's URL.
//!
//! All UI rendering, theme handling, editing, search, backlinks, and
//! the `/_mobile/*` routes (REQ-4005) come from the embedded server
//! per [[ADR-4001]] of SPEC-040. This file is intentionally tiny.

use std::path::PathBuf;

mod serve_lifecycle;

/// Tauri Mobile entry point. The `mobile_entry_point` attribute is
/// applied only when targeting iOS or Android; desktop builds enter
/// through `main.rs` calling `run()` directly.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_setup())
        .setup(|app| {
            let app_data_dir = resolve_app_data_dir(app);
            let vault_root = resolve_vault_root(app);
            let bind_addr = "127.0.0.1".to_string();
            let port: u16 = 23423;

            // Register both roots before the serve task spawns so the
            // /_mobile/* handlers can read them from mobile_state.
            zetl::mobile_state::set_app_data_dir(app_data_dir.clone());
            zetl::mobile_state::set_vault_root(vault_root.clone());

            // Set $HOME + seed an empty `.ssh/known_hosts` BEFORE
            // anything else touches libgit2. libgit2's SSH transport
            // resolves HOME once at lib-init time (the first
            // `Repository::*` call) and caches it; doing this later
            // is a no-op for the cached path, which would make the
            // SPEC-040 clone fail with `error loading known_hosts;
            // class=Ssh (23)`. The keystore generate/restore step
            // below uses git2 transitively, so this must run first.
            zetl::mobile_git::ensure_known_hosts_file();

            // Migrate any legacy single-vault layout into the multi-
            // vault `vaults/<label>/` structure (idempotent).
            if let Err(e) = zetl::mobile_state::migrate_single_vault_layout(&app_data_dir) {
                tracing::warn!("vault layout migration failed: {e:#}");
            }

            // Prefer the OS keychain over the on-disk ssh_key.json
            // for KeyStore persist/restore. Falls back to the file
            // automatically if the platform backend isn't available
            // (e.g., headless Linux without Secret Service).
            zetl::mobile_state::enable_keyring();

            // Restore previously-persisted SSH key if present;
            // otherwise auto-generate a fresh per-device keypair.
            let keystore = zetl::mobile_state::global();
            match keystore.restore(&app_data_dir) {
                Ok(true) => tracing::info!("restored SSH key from app data dir"),
                Ok(false) => match keystore.generate_new() {
                    Ok(_pub_line) => {
                        tracing::info!("auto-generated fresh per-device SSH key");
                        if let Err(e) = keystore.persist(&app_data_dir) {
                            tracing::warn!("ssh key persist failed: {e:#}");
                        }
                    }
                    Err(e) => tracing::error!("ssh key generation failed: {e:#}"),
                },
                Err(e) => tracing::warn!("ssh key restore failed: {e:#}"),
            }

            tauri::async_runtime::spawn(async move {
                if let Err(e) =
                    serve_lifecycle::spawn_embedded_serve(vault_root, bind_addr, port).await
                {
                    tracing::error!("embedded zetl serve failed: {e:?}");
                }
            });

            // Bypass the bundled dist/index.html entirely: as soon as
            // the embedded serve is reachable, navigate the main
            // WebView directly at the right /_mobile/* surface.
            //
            // Landing page picks itself based on state:
            //   - share-inbox non-empty → /_mobile/capture?from=share
            //     (capture-first, the spec's primary scenario)
            //   - otherwise → /_mobile/vaults (multi-vault picker)
            //
            // The poll loop is generous: server typically binds in
            // <1s but we keep retrying for 30s in case the host is
            // slow on app cold-start.
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                tauri::async_runtime::spawn(async move {
                    let target = if zetl::mobile_state::share_inbox_count() > 0 {
                        "http://127.0.0.1:23423/_mobile/capture?from=share"
                    } else {
                        "http://127.0.0.1:23423/_mobile/vaults"
                    };
                    let mut tries = 0;
                    loop {
                        if std::net::TcpStream::connect_timeout(
                            &"127.0.0.1:23423".parse().unwrap(),
                            std::time::Duration::from_millis(500),
                        )
                        .is_ok()
                        {
                            if let Ok(url) = target.parse() {
                                let _ = window.navigate(url);
                            }
                            break;
                        }
                        tries += 1;
                        if tries > 60 {
                            tracing::error!(
                                "embedded serve not reachable after 30s — WebView stuck"
                            );
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running zetl-mobile");
}

/// Resolve the platform-appropriate app-data directory. Used for the
/// vault working tree (under `vault/`) and the persisted SSH key
/// (`ssh_key.json`).
fn resolve_app_data_dir(app: &mut tauri::App) -> PathBuf {
    use tauri::Manager;

    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let p = PathBuf::from(home).join(".zetl-mobile");
    let _ = std::fs::create_dir_all(&p);
    p
}

/// Vault working-tree root, a subdirectory of the app data dir so
/// onboarding has a stable place to clone into.
fn resolve_vault_root(app: &mut tauri::App) -> PathBuf {
    let p = resolve_app_data_dir(app).join("vault");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .try_init();
}

/// No-op plugin registration hook. Real Tauri plugins (keychain,
/// share-extension intake) land in subsequent slices.
fn tauri_plugin_setup<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("zetl-mobile-plugins").build()
}
