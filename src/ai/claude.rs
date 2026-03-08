// src/ai/claude.rs
//
// Провайдер: Anthropic Claude
// Документация: https://docs.anthropic.com/en/api/messages
//
// Дефолтная модель: claude-sonnet-4-6
// Получить ключ: https://console.anthropic.com/settings/keys

use serde_json::{json, Value};
use super::{AiClient, AiError};

pub struct ClaudeClient {
    api_key: String,
    model:   String,
}

impl ClaudeClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: model.into() }
    }
}

impl AiClient for ClaudeClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let body = json!({
            "model": self.model,
            "max_tokens": 2048,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": user_message }
            ]
        });

        let resp = ureq::post("https://api.anthropic.com/v1/messages")
            .set("Content-Type",      "application/json")
            .set("x-api-key",         &self.api_key)
            .set("anthropic-version", "2023-06-01")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => AiError::Auth(
                    "Невалидный ANTHROPIC_API_KEY. Проверь ключ на https://console.anthropic.com".into()
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