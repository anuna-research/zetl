// Anthropic Claude API client

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

use super::{LlmClient, Message, MessageContent, Response, Tool, ToolCall};

pub struct AnthropicClient {
    api_key: String,
    model: String,
    http: Client,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let model = if model.is_empty() {
            "claude-haiku-4-20250414".into()
        } else {
            model
        };
        Self {
            api_key: api_key.into(),
            model,
            http: Client::new(),
        }
    }
}

/// Convert our messages into Anthropic format.
/// Anthropic separates the system prompt from the messages list.
fn messages_to_anthropic(messages: &[Message]) -> (Option<String>, Vec<Value>) {
    let mut system_text = None;
    let mut out = Vec::new();

    for m in messages {
        if m.role == "system" {
            system_text = Some(m.content.as_str().to_string());
            continue;
        }

        match &m.content {
            MessageContent::Text(text) => {
                out.push(json!({
                    "role": if m.role == "tool" { "user" } else { &m.role },
                    "content": text,
                }));
            }
            MessageContent::ToolResult {
                tool_use_id,
                content,
            } => {
                // Anthropic expects tool results as user messages with tool_result content blocks
                out.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                    }],
                }));
            }
        }
    }

    (system_text, out)
}

/// Convert our Tool list into Anthropic tool format.
fn tools_to_anthropic(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

#[async_trait::async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Response> {
        let (system_text, api_messages) = messages_to_anthropic(&messages);

        let mut body = json!({
            "model": self.model,
            "messages": api_messages,
            "max_tokens": 4096,
        });

        if let Some(sys) = system_text {
            body["system"] = json!(sys);
        }

        if let Some(ref tools) = tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools_to_anthropic(tools));
            }
        }

        let resp = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("anthropic request failed")?;

        let status = resp.status();
        let resp_body: Value = resp.json().await.context("anthropic response not json")?;

        if !status.is_success() {
            let err_msg = resp_body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("anthropic API error ({}): {}", status, err_msg);
        }

        // Parse content blocks — handle both text and tool_use
        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        if let Some(content_blocks) = resp_body.get("content").and_then(|c| c.as_array()) {
            for block in content_blocks {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            text_parts.push(text.to_string());
                        }
                    }
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let arguments = block
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                    _ => {}
                }
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let usage = resp_body.get("usage");
        let tokens_in = usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tokens_out = usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(Response {
            content,
            tool_calls,
            tokens_in,
            tokens_out,
        })
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        if self.model.contains("haiku") {
            let cost_in = tokens_in as f64 * 0.25 / 1_000_000.0;
            let cost_out = tokens_out as f64 * 1.25 / 1_000_000.0;
            cost_in + cost_out
        } else if self.model.contains("sonnet") {
            let cost_in = tokens_in as f64 * 3.0 / 1_000_000.0;
            let cost_out = tokens_out as f64 * 15.0 / 1_000_000.0;
            cost_in + cost_out
        } else if self.model.contains("opus") {
            let cost_in = tokens_in as f64 * 15.0 / 1_000_000.0;
            let cost_out = tokens_out as f64 * 75.0 / 1_000_000.0;
            cost_in + cost_out
        } else {
            // Default to haiku pricing
            let cost_in = tokens_in as f64 * 0.25 / 1_000_000.0;
            let cost_out = tokens_out as f64 * 1.25 / 1_000_000.0;
            cost_in + cost_out
        }
    }
}
