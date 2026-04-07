//! Stdio and HTTP transport setup.

use anyhow::Result;

use super::server::McpServer;

/// Serve MCP over stdio (stdin/stdout JSON-RPC).
pub async fn serve_stdio(server: McpServer) -> Result<()> {
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    // Block until the client disconnects or the service shuts down.
    service.waiting().await?;
    Ok(())
}

/// Serve MCP over HTTP (stub — Task 14 will implement this).
pub async fn serve_http(_server: McpServer, _host: &str, _port: u16) -> Result<()> {
    eprintln!("HTTP transport not yet implemented (see Task 14)");
    Ok(())
}

/// Health endpoint handler (for HTTP transport).
#[allow(dead_code)]
pub async fn health_handler() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": "zetl-mcp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
