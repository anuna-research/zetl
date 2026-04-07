//! Stdio and HTTP transport setup.

use std::sync::Arc;

use anyhow::Result;
use axum::routing::get;

use super::server::McpServer;

/// Serve MCP over stdio (stdin/stdout JSON-RPC).
pub async fn serve_stdio(server: McpServer) -> Result<()> {
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    // Block until the client disconnects or the service shuts down.
    service.waiting().await?;
    Ok(())
}

/// Serve MCP over HTTP using the Streamable HTTP transport.
///
/// Mounts:
///   GET  /health  — unauthenticated health-check (JSON)
///   POST /mcp     — MCP Streamable HTTP transport endpoint
pub async fn serve_http(server: McpServer, host: &str, port: u16) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };

    let config = StreamableHttpServerConfig::default();

    // The factory is called once per MCP session (stateful mode, default).
    // McpServer is Clone + Send + 'static so this is safe.
    let mcp_service: StreamableHttpService<McpServer, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let server = server.clone();
                move || Ok(server.clone())
            },
            Arc::new(LocalSessionManager::default()),
            config,
        );

    let router = axum::Router::new()
        .route("/health", get(health_handler))
        .nest_service("/mcp", mcp_service);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    eprintln!("zetl-mcp listening on http://{addr}  (MCP at /mcp, health at /health)");

    axum::serve(listener, router).await?;
    Ok(())
}

/// Health endpoint handler — returns `{"status":"ok","server":"zetl-mcp","version":"x.y.z"}`.
pub async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": "zetl-mcp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
