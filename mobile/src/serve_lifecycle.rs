//! SPEC-040 embedded zetl serve lifecycle (REQ-4004).
//!
//! Boots a single-user `zetl serve` instance bound to loopback inside
//! the mobile process. The Tauri WebView then loads pages from
//! `http://127.0.0.1:<port>/` (configured via `devUrl` in
//! `tauri.conf.json`). The whole UI surface — Minijinja templates,
//! themes, editor, backlinks, search, plus the `/_mobile/*` routes
//! gated by the `mobile` cargo feature — is delivered by this server.
//!
//! Vault root is resolved per platform by `lib.rs` (iOS/Android use the
//! Tauri-provided app data dir; desktop falls back to
//! `$HOME/.zetl-mobile/vault`). Port + bind are hardcoded to
//! `127.0.0.1:23423` to match `tauri.conf.json` `devUrl`. The async
//! task spawned from `setup()` calls into [`zetl::web::launch_default`]
//! which builds a single-user `WebState` (no collab, no passkey, no
//! semantic, no public_dir, no git auto-commit) and runs the existing
//! axum router — including the `/_mobile/*` routes registered by the
//! `mobile` feature.

use std::path::PathBuf;

/// Spawn an embedded `zetl serve` instance.
///
/// Returns only when the server exits (typically because the app is
/// shutting down) or fails to bind.
pub(crate) async fn spawn_embedded_serve(
    vault_root: PathBuf,
    bind_addr: String,
    port: u16,
) -> anyhow::Result<()> {
    tracing::info!(
        vault = %vault_root.display(),
        bind = %bind_addr,
        port,
        "starting embedded zetl serve",
    );

    zetl::web::launch_default(vault_root, &bind_addr, port).await
}
