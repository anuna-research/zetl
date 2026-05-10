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
            let vault_root = resolve_vault_root(app);
            let bind_addr = "127.0.0.1".to_string();
            let port: u16 = 23423; // matches tauri.conf.json devUrl

            // Register the vault root before the serve task spawns so
            // the /_mobile/onboarding handlers can read it from
            // mobile_state::vault_root() without needing WebState plumbing.
            zetl::mobile_state::set_vault_root(vault_root.clone());

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

/// On iOS and Android the app data directory is provided by the OS
/// and lives under the Tauri app's resolver. On desktop we use the
/// dev fallback at `$HOME/.zetl-mobile/vault` so the dev shell has a
/// stable place to clone the vault during local iteration.
fn resolve_vault_root(app: &mut tauri::App) -> PathBuf {
    use tauri::Manager;

    if let Ok(dir) = app.path().app_data_dir() {
        let p = dir.join("vault");
        let _ = std::fs::create_dir_all(&p);
        return p;
    }

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let p = PathBuf::from(home).join(".zetl-mobile").join("vault");
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
