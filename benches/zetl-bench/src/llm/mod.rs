// LLM client trait and implementations

pub mod anthropic;
pub mod groq;
pub mod openai_compat;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Message content: either plain text or a tool result being fed back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

impl MessageContent {
    pub fn text(s: impl Into<String>) -> Self {
        MessageContent::Text(s.into())
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        MessageContent::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
        }
    }

    /// Return the text payload regardless of variant.
    pub fn as_str(&self) -> &str {
        match self {
            MessageContent::Text(s) => s,
            MessageContent::ToolResult { content, .. } => content,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: MessageContent::text(text),
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: MessageContent::text(text),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: MessageContent::text(text),
        }
    }

    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: MessageContent::tool_result(id, content),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    /// Send a chat completion request.
    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Response>;

    /// Provider name for logging (e.g. "groq", "anthropic").
    fn provider_name(&self) -> &str;

    /// Model identifier (e.g. "llama-3.1-8b-instant").
    fn model_name(&self) -> &str;

    /// Estimated cost in USD for the given token counts.
    fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64;
}

/// Factory: create a client by provider name.
///
/// - `"groq"` — Groq cloud (OpenAI-compatible, uses `GROQ_API_KEY`)
/// - `"anthropic"` — Anthropic Messages API (uses `ANTHROPIC_API_KEY`)
/// - `"openai"` — OpenAI or any compatible endpoint (uses `OPENAI_API_KEY`)
/// - `"ollama"` — shorthand for OpenAI-compatible at `http://localhost:11434/v1`
///
/// `base_url` overrides the default endpoint for openai/ollama providers.
pub fn create_client(
    provider: &str,
    model: &str,
    base_url: Option<&str>,
) -> Result<Box<dyn LlmClient>> {
    match provider {
        "groq" => {
            let api_key = std::env::var("GROQ_API_KEY")
                .map_err(|_| anyhow::anyhow!("GROQ_API_KEY not set"))?;
            Ok(Box::new(groq::GroqClient::new(api_key, model)))
        }
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
            Ok(Box::new(anthropic::AnthropicClient::new(api_key, model)))
        }
        "openai" => {
            let api_key = std::env::var("OPENAI_API_KEY")
                .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set"))?;
            let url = base_url.unwrap_or("https://api.openai.com/v1");
            Ok(Box::new(openai_compat::OpenAICompatibleClient::new(
                api_key,
                model,
                url,
                "openai",
                "OPENAI_API_KEY",
            )))
        }
        "ollama" => {
            let url = base_url.unwrap_or("http://localhost:11434/v1");
            // Ollama doesn't require an API key but the field must exist
            let api_key = std::env::var("OLLAMA_API_KEY").unwrap_or_default();
            Ok(Box::new(openai_compat::OpenAICompatibleClient::new(
                api_key,
                model,
                url,
                "ollama",
                "",
            )))
        }
        other => bail!("unknown LLM provider: {other}"),
    }
}
