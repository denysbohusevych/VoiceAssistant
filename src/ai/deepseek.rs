// src/ai/deepseek.rs
//
// Провайдер: DeepSeek
// Документация: https://platform.deepseek.com/api-docs
// API совместим с OpenAI — тот же формат запросов.
//
// Дефолтная модель: deepseek-chat  (DeepSeek-V3)
// Другие модели:    deepseek-reasoner  (DeepSeek-R1 — медленнее, но лучше рассуждает)
//
// Получить ключ: https://platform.deepseek.com/api_keys

use serde_json::{json, Value};
use super::{AiClient, AiError};

pub struct DeepSeekClient {
    api_key: String,
    model:   String,
}

impl DeepSeekClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: model.into() }
    }
}

impl AiClient for DeepSeekClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_message  }
            ],
            "temperature": 0.7,
            "max_tokens": 2048
        });

        let resp = ureq::post("https://api.deepseek.com/v1/chat/completions")
            .set("Content-Type",  "application/json")
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => AiError::Auth(
                    "Невалидный DEEPSEEK_API_KEY. Проверь ключ на https://platform.deepseek.com".into()
                ),
                ureq::Error::Status(status, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    AiError::Api { status, body }
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