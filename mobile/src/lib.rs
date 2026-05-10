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
        .plugin(tauri_plugin_setup())
        .setup(|app| {
            let app_data_dir = resolve_app_data_dir(app);
            let vault_root = resolve_vault_root(app);
            let bind_addr = "127.0.0.1".to_string();
            let port: u16 = 23423; // matches tauri.conf.json devUrl

            // Register both roots before the serve task spawns so the
            // /_mobile/* handlers can read them from mobile_state.
            zetl::mobile_state::set_app_data_dir(app_data_dir.clone());
            zetl::mobile_state::set_vault_root(vault_root.clone());

            // Best-effort restore of a previously-persisted SSH key so
            // the user does not re-enter their seed every launch.
            // Failures are logged but never block startup — the
            // onboarding wizard can still recover.
            match zetl::mobile_state::global().restore(&app_data_dir) {
                Ok(true) => tracing::info!("restored SSH key from app data dir"),
                Ok(false) => tracing::info!("no persisted SSH key — onboarding required"),
                Err(e) => tracing::warn!("ssh key restore failed: {e:#}"),
            }

            tauri::async_runtime::spawn(async move {
                if let Err(e) =
                    serve_lifecycle::spawn_embedded_serve(vault_root, bind_addr, port).await
                {
                    tracing::error!("embedded zetl serve failed: {e:?}");
                }
            });

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
