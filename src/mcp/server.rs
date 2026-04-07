//! McpServer struct and ServerHandler impl.

use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::*;
use rmcp::service::RoleServer;

use super::tools;
use super::types::McpState;

/// MCP server wrapping vault state and parsed files.
#[derive(Clone)]
pub struct McpServer {
    pub state: McpState,
    pub files: Arc<Vec<crate::types::ParsedFile>>,
}

impl McpServer {
    pub fn new(state: McpState, files: Vec<crate::types::ParsedFile>) -> Self {
        Self {
            state,
            files: Arc::new(files),
        }
    }

    /// Build the list of tool definitions with JSON schemas.
    fn tool_definitions() -> Vec<Tool> {
        use serde_json::json;

        let mk = |name: &str, desc: &str, props: serde_json::Value| -> Tool {
            let mut schema = serde_json::Map::new();
            schema.insert("type".into(), json!("object"));
            schema.insert("properties".into(), props.clone());

            // Build required array from property keys.
            let required: Vec<serde_json::Value> = if let Some(obj) = props.as_object() {
                obj.keys()
                    .filter(|k| {
                        // Only require fields that don't have a "default" key.
                        obj.get(*k)
                            .and_then(|v| v.as_object())
                            .map(|o| !o.contains_key("default"))
                            .unwrap_or(true)
                    })
                    .map(|k| json!(k))
                    .collect()
            } else {
                vec![]
            };
            if !required.is_empty() {
                schema.insert("required".into(), json!(required));
            }

            Tool::new(name.to_string(), desc.to_string(), Arc::new(schema))
                .with_annotations(ToolAnnotations::new().read_only(true).open_world(false))
        };

        vec![
            mk(
                "get_page",
                "Retrieve the raw Markdown content of a vault page by name.",
                json!({
                    "page": { "type": "string", "description": "Page name (case-insensitive, partial match OK)" }
                }),
            ),
            mk(
                "search",
                "Full-text search over vault pages via Tantivy.",
                json!({
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max results (1-100)", "default": 20 }
                }),
            ),
            mk(
                "links",
                "List forward (outgoing) wikilinks from a page.",
                json!({
                    "page": { "type": "string", "description": "Page name" }
                }),
            ),
            mk(
                "backlinks",
                "List incoming wikilinks pointing to a page.",
                json!({
                    "page": { "type": "string", "description": "Page name" }
                }),
            ),
            mk(
                "path",
                "Find the shortest link-hop path between two pages.",
                json!({
                    "from": { "type": "string", "description": "Source page" },
                    "to": { "type": "string", "description": "Target page" },
                    "max_depth": { "type": "integer", "description": "Maximum hops (1-20)", "default": 10 }
                }),
            ),
            mk(
                "similar",
                "Find pages with names similar to a query via SimHash.",
                json!({
                    "query": { "type": "string", "description": "Query string" },
                    "threshold": { "type": "integer", "description": "Max Hamming distance (0-64)", "default": 10 },
                    "limit": { "type": "integer", "description": "Max results (1-100)", "default": 20 }
                }),
            ),
            mk(
                "check",
                "Health check: dead links, orphans, and graph statistics.",
                json!({}),
            ),
            mk(
                "status",
                "Server status and vault summary.",
                json!({}),
            ),
            mk(
                "reason",
                "Defeasible reasoning over SPL blocks in the vault (requires --features reason).",
                json!({
                    "query": { "type": "string", "description": "SPL query to evaluate", "default": "" },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Page names to scope reasoning to (empty = all)", "default": [] }
                }),
            ),
        ]
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        InitializeResult::new(capabilities)
            .with_server_info(Implementation::new(
                "zetl-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Use the available tools to query vault pages, search, traverse links, and check vault health."
                    .to_string(),
            )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        std::future::ready(Ok(ListToolsResult::with_all_items(
            Self::tool_definitions(),
        )))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        Self::tool_definitions()
            .into_iter()
            .find(|t| t.name == name)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        async move {
            let name = request.name.as_ref();
            let args = request.arguments.as_ref();

            let get_str = |key: &str| -> String {
                args.and_then(|a| a.get(key))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };

            let get_usize = |key: &str, default: usize| -> usize {
                args.and_then(|a| a.get(key))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(default)
            };

            let get_u32 = |key: &str, default: u32| -> u32 {
                args.and_then(|a| a.get(key))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(default)
            };

            let result = match name {
                "get_page" => tools::tool_get(&self.state, &get_str("page")),
                "search" => {
                    tools::tool_search(&self.state, &get_str("query"), get_usize("limit", 20))
                }
                "links" => tools::tool_links(&self.state, &get_str("page")),
                "backlinks" => tools::tool_backlinks(&self.state, &get_str("page")),
                "path" => tools::tool_path(
                    &self.state,
                    &get_str("from"),
                    &get_str("to"),
                    get_usize("max_depth", 10),
                ),
                "similar" => tools::tool_similar(
                    &self.state,
                    &get_str("query"),
                    get_u32("threshold", 10),
                    get_usize("limit", 20),
                ),
                "check" => tools::tool_check(&self.state),
                "status" => tools::tool_status(&self.state),
                "reason" => {
                    let query = get_str("query");
                    let files: Vec<String> = args
                        .and_then(|a| a.get("files"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    tools::tool_reason(&self.state, &query, &files)
                }
                _ => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Unknown tool: {name}"
                    ))]));
                }
            };

            match result {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(text)]))
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
            }
        }
    }
}
