// src/ai/deepseek.rs
use serde_json::{json, Value};
use crate::config::SharedConfig;
use super::{AiClient, AiError};
use super::http::make_agent;

pub struct DeepSeekClient {
    api_key: String,
    model:   String,
    cfg:     SharedConfig,
}

impl DeepSeekClient {
    pub fn new(
        api_key: impl Into<String>,
        model:   impl Into<String>,
        cfg:     SharedConfig,
    ) -> Self {
        Self { api_key: api_key.into(), model: model.into(), cfg }
    }
}

impl AiClient for DeepSeekClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let (max_tokens, temperature, timeouts) = {
            let c = self.cfg.read().unwrap();
            (c.ai.max_tokens, c.ai.temperature, c.ai.http_timeouts.clone())
        };

        let agent = make_agent(&timeouts);

        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_message  }
            ],
            "temperature": temperature,
            "max_tokens":  max_tokens
        });

        let resp = agent
            .post("https://api.deepseek.com/v1/chat/completions")
            .set("Content-Type",  "application/json")
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => AiError::Auth(
                    "Невалидный DEEPSEEK_API_KEY. Проверь: https://platform.deepseek.com".into()
                ),
                ureq::Error::Status(status, resp) => {
                    AiError::Api { status, body: resp.into_string().unwrap_or_default() }
                }
                other => AiError::Http(other.to_string()),
            })?;

        let json: Value = resp.into_json()
            .map_err(|e| AiError::Parse(e.to_string()))?;

        json["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::Parse(format!("Unexpected DeepSeek response: {}", json)))
    }

    fn model_name(&self)    -> &str { &self.model }
    fn provider_name(&self) -> &str { "DeepSeek" }
}