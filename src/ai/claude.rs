// src/ai/claude.rs
use serde_json::{json, Value};
use crate::config::SharedConfig;
use super::{AiClient, AiError};
use super::http::make_agent;

pub struct ClaudeClient {
    api_key: String,
    model:   String,
    cfg:     SharedConfig,
}

impl ClaudeClient {
    pub fn new(
        api_key:  impl Into<String>,
        model:    impl Into<String>,
        cfg:      SharedConfig,
    ) -> Self {
        Self { api_key: api_key.into(), model: model.into(), cfg }
    }
}

impl AiClient for ClaudeClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let (max_tokens, timeouts) = {
            let c = self.cfg.read().unwrap();
            (c.ai.max_tokens, c.ai.http_timeouts.clone())
        };

        let agent = make_agent(&timeouts);

        let body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "system": system_prompt,
            "messages": [{ "role": "user", "content": user_message }]
        });

        let resp = agent
            .post("https://api.anthropic.com/v1/messages")
            .set("Content-Type",      "application/json")
            .set("x-api-key",         &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => AiError::Auth(
                    "Невалидный ANTHROPIC_API_KEY. Проверь: https://console.anthropic.com".into()
                ),
                ureq::Error::Status(status, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    AiError::Api { status, body }
                }
                other => AiError::Http(other.to_string()),
            })?;

        let json: Value = resp.into_json()
            .map_err(|e| AiError::Parse(e.to_string()))?;

        json["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::Parse(format!("Unexpected Claude response: {}", json)))
    }

    fn model_name(&self)    -> &str { &self.model }
    fn provider_name(&self) -> &str { "Claude" }
}