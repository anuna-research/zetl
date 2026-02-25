pub mod html;
pub mod markdown;
pub mod routes;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::graph::LinkGraph;
use crate::types::ParsedFile;

/// Shared state passed to all handlers via axum State.
#[derive(Clone)]
pub struct WebState {
    pub files: Arc<Vec<ParsedFile>>,
    pub graph: Arc<LinkGraph>,
    pub vault_root: Arc<PathBuf>,
    pub page_names: Arc<Vec<String>>,
    /// Set of page names backed by real files (for dead-link detection).
    pub resolved: Arc<HashSet<String>>,
}

pub async fn run(state: WebState, port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(routes::index_handler))
        .route("/page/{page_name}", get(routes::page_handler).put(routes::save_handler))
        .route("/preview/{page_name}", get(routes::preview_handler))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    eprintln!("zetl serve  →  http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
