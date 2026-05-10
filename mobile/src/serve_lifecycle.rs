//! SPEC-040 embedded zetl serve lifecycle (REQ-4004).
//!
//! Boots a single-user `zetl serve` instance bound to loopback inside
//! the mobile process. The Tauri WebView then loads pages from
//! `http://127.0.0.1:<port>/` (configured via `devUrl` in
//! `tauri.conf.json`). The whole UI surface — Minijinja templates,
//! themes, editor, backlinks, search, plus the `/_mobile/*` routes
//! gated by the `mobile` cargo feature — is delivered by this server.
//!
//! v0.1-strawman status: this module is wired into the Tauri shell
//! but the actual `zetl::web::run()` invocation is stubbed because
//! that function takes a fully-constructed `WebState` whose builder
//! lives inside `src/main.rs` of the main crate (see the desktop
//! `Commands::Serve` arm). The follow-up slice introduces a
//! `zetl::web::launch_default(vault_root, bind, port, opts)`
//! convenience function and replaces the stub below with a single
//! call to it.
//!
//! The rest of the architecture is intact: vault root resolved per
//! platform, port + bind hardcoded to `127.0.0.1:23423` to match
//! `tauri.conf.json` `devUrl`, async task spawned from the shell's
//! `setup()` hook. Until the real boot lands, the WebView will sit on
//! the loading screen until its 15s timeout fires.

use std::path::PathBuf;

/// Spawn an embedded `zetl serve` instance.
///
/// Strawman stub. See module docs for the follow-up slice that
/// replaces this with a real `zetl::web::launch_default()` call.
pub(crate) async fn spawn_embedded_serve(
    vault_root: PathBuf,
    bind_addr: String,
    port: u16,
) -> anyhow::Result<()> {
    tracing::info!(
        vault = %vault_root.display(),
        bind = %bind_addr,
        port,
        "embedded zetl serve: scaffold present, real boot follows in next slice",
    );

    // Pretend to be running so this future does not return immediately;
    // an early return would let the spawning task exit and (in some
    // future runtime configurations) drop the work-stealing slot. A
    // real implementation calls `zetl::web::run(state, port, bind, …)`
    // and that future stays pending for the lifetime of the server.
    std::future::pending::<()>().await;

    Ok(())
}
