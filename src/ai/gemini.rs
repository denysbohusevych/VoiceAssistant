// src/ai/gemini.rs
//
// Провайдер: Google Gemini
// Документация: https://ai.google.dev/api/generate-content
//
// Дефолтная модель: gemini-2.0-flash
// Получить ключ: https://aistudio.google.com/apikey

use serde_json::json;
use super::{AiClient, AiError};
use super::http::{make_agent, parse_json_response};

pub struct GeminiClient {
    api_key: String,
    model:   String,
    agent:   ureq::Agent,
}

impl GeminiClient {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: model.into(), agent: make_agent() }
    }
}

impl AiClient for GeminiClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let body = json!({
            "system_instruction": {
                "parts": [{ "text": system_prompt }]
            },
            "contents": [{
                "parts": [{ "text": user_message }]
            }],
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 20000
            }
        });

        let resp = self.agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(400, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    if body.contains("API_KEY_INVALID") || body.contains("API key not valid") {
                        AiError::Auth("Невалидный GEMINI_API_KEY. Получи ключ: https://aistudio.google.com/apikey".into())
                    } else {
                        AiError::Api { status: 400, body }
                    }
                }
                ureq::Error::Status(status, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    AiError::Api { status, body }
                }
                other => AiError::Http(other.to_string()),
            })?;

        let json = parse_json_response(resp)?;

        json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                let reason = json["candidates"][0]["finishReason"].as_str().unwrap_or("unknown");
                AiError::Parse(format!("Пустой ответ Gemini. finishReason: {}", reason))
            })
    }

    fn model_name(&self)    -> &str { &self.model }
    fn provider_name(&self) -> &str { "Gemini" }
}