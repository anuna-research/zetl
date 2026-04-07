//! Stdio and HTTP transport setup.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use axum::routing::get;

use super::server::McpServer;

/// Minimal state needed by the /health endpoint.
pub struct HealthState {
    pub vault_pages: usize,
    pub started_at: Instant,
}

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
///
/// When `require_auth` is true, the `/mcp` endpoint is protected by JWT
/// bearer-token authentication via an axum middleware layer.  The `/health`
/// route is always unauthenticated.
pub async fn serve_http(
    server: McpServer,
    host: &str,
    port: u16,
    require_auth: bool,
    allowed_issuers: Arc<HashMap<String, String>>,
) -> Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
        session::local::LocalSessionManager,
    };

    let config = StreamableHttpServerConfig::default();

    // Build HealthState from server state before moving server.
    let health_state = Arc::new(HealthState {
        vault_pages: server.state.page_names.len(),
        started_at: server.state.started_at,
    });

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

    // Build the MCP sub-router, optionally wrapped in auth middleware.
    let mcp_router = if require_auth {
        axum::Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(axum::middleware::from_fn_with_state(
                allowed_issuers.clone(),
                auth_middleware,
            ))
    } else {
        axum::Router::new().nest_service("/mcp", mcp_service)
    };

    // Health is always unauthenticated — it lives outside the auth layer.
    let router = axum::Router::new()
        .route("/health", get(health_handler))
        .with_state(health_state)
        .merge(mcp_router);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    eprintln!(
        "zetl-mcp listening on http://{addr}  (MCP at /mcp, health at /health{})",
        if require_auth { ", auth required" } else { "" }
    );

    axum::serve(listener, router).await?;
    Ok(())
}

/// Axum middleware that verifies a JWT bearer token on every request.
///
/// Extracts the `Authorization: Bearer <token>` header, verifies the JWT
/// signature against the allowed issuers map, and either passes through
/// (on success) or returns 401 Unauthorized (on failure / missing token).
async fn auth_middleware(
    axum::extract::State(issuers): axum::extract::State<Arc<HashMap<String, String>>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h[7..],
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({
                    "error": "unauthorized",
                    "reason": "missing or malformed bearer token"
                })),
            )
                .into_response();
        }
    };

    match crate::mcp::auth::verify_jwt(token, &issuers) {
        Ok(_claims) => next.run(request).await,
        Err(e) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": "unauthorized",
                "reason": e.to_string()
            })),
        )
            .into_response(),
    }
}

/// Health endpoint handler — returns vault page count, uptime, and version.
pub async fn health_handler(
    axum::extract::State(hs): axum::extract::State<Arc<HealthState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": "zetl-mcp",
        "version": env!("CARGO_PKG_VERSION"),
        "vault_pages": hs.vault_pages,
        "uptime_seconds": hs.started_at.elapsed().as_secs(),
    }))
}
