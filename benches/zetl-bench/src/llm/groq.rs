// Groq API client (OpenAI-compatible)

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

use super::{LlmClient, Message, MessageContent, Response, Tool, ToolCall};

pub struct GroqClient {
    api_key: String,
    model: String,
    http: Client,
}

impl GroqClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let model = if model.is_empty() {
            "llama-3.1-8b-instant".into()
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

/// Convert our Message list into OpenAI-format JSON messages.
pub(crate) fn messages_to_openai(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| match &m.content {
            MessageContent::Text(text) => {
                json!({ "role": m.role, "content": text })
            }
            MessageContent::ToolResult {
                tool_use_id,
                content,
            } => {
                json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content,
                })
            }
        })
        .collect()
}

/// Convert our Tool list into OpenAI function-calling format.
pub(crate) fn tools_to_openai(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

/// Parse an OpenAI-shaped response body into our Response type.
pub(crate) fn parse_openai_response(body: &Value) -> Result<Response> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .context("no choices in response")?;

    let message = choice.get("message").context("no message in choice")?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .map(String::from);

    let mut tool_calls = Vec::new();
    if let Some(tcs) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for tc in tcs {
            let id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let func = tc.get("function").context("tool_call missing function")?;
            let name = func
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args_str = func
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let arguments: Value =
                serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    let usage = body.get("usage");
    let tokens_in = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tokens_out = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    Ok(Response {
        content,
        tool_calls,
        tokens_in,
        tokens_out,
    })
}

#[async_trait::async_trait]
impl LlmClient for GroqClient {
    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Response> {
        let mut body = json!({
            "model": self.model,
            "messages": messages_to_openai(&messages),
        });

        if let Some(ref tools) = tools {
            if !tools.is_empty() {
                body["tools"] = json!(tools_to_openai(tools));
            }
        }

        let resp = self
            .http
            .post("https://api.groq.com/openai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("groq request failed")?;

        let status = resp.status();
        let resp_body: Value = resp.json().await.context("groq response not json")?;

        if !status.is_success() {
            let err_msg = resp_body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("groq API error ({}): {}", status, err_msg);
        }

        parse_openai_response(&resp_body)
    }

    fn provider_name(&self) -> &str {
        "groq"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        // Groq pricing: ~$0.05/1M input, ~$0.08/1M output for llama-3.1-8b-instant
        let cost_in = tokens_in as f64 * 0.05 / 1_000_000.0;
        let cost_out = tokens_out as f64 * 0.08 / 1_000_000.0;
        cost_in + cost_out
    }
}

