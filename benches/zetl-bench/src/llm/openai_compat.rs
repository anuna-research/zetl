// Generic OpenAI-compatible API client

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};

use super::groq::{messages_to_openai, parse_openai_response, tools_to_openai};
use super::{LlmClient, Message, Response, Tool};

pub struct OpenAICompatibleClient {
    api_key: String,
    model: String,
    base_url: String,
    provider: String,
    http: Client,
}

impl OpenAICompatibleClient {
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        provider: impl Into<String>,
        _env_key: &str,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            provider: provider.into(),
            http: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl LlmClient for OpenAICompatibleClient {
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

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .context("openai-compat request failed")?;

        let status = resp.status();
        let resp_body: Value = resp.json().await.context("response not json")?;

        if !status.is_success() {
            let err_msg = resp_body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("{} API error ({}): {}", self.provider, status, err_msg);
        }

        parse_openai_response(&resp_body)
    }

    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn estimate_cost(&self, tokens_in: u64, tokens_out: u64) -> f64 {
        // Generic estimate
        let cost_in = tokens_in as f64 * 1.0 / 1_000_000.0;
        let cost_out = tokens_out as f64 * 2.0 / 1_000_000.0;
        cost_in + cost_out
    }
}
