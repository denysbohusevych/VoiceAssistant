// src/ai/gemini.rs
use serde_json::json;
use crate::config::SharedConfig;
use super::{AiClient, AiError};
use super::http::{make_agent, parse_json_response};

pub struct GeminiClient {
    api_key: String,
    model:   String,
    cfg:     SharedConfig,
}

impl GeminiClient {
    pub fn new(
        api_key: impl Into<String>,
        model:   impl Into<String>,
        cfg:     SharedConfig,
    ) -> Self {
        Self { api_key: api_key.into(), model: model.into(), cfg }
    }
}

impl AiClient for GeminiClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let (max_tokens, temperature, timeouts) = {
            let c = self.cfg.read().unwrap();
            (c.ai.max_tokens, c.ai.temperature, c.ai.http_timeouts.clone())
        };

        let agent = make_agent(&timeouts);

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let body = json!({
            "system_instruction": { "parts": [{ "text": system_prompt }] },
            "contents": [{ "parts": [{ "text": user_message }] }],
            "generationConfig": {
                "temperature":     temperature,
                "maxOutputTokens": max_tokens
            }
        });

        let resp = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(400, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    if body.contains("API_KEY_INVALID") || body.contains("API key not valid") {
                        AiError::Auth("Невалидный GEMINI_API_KEY. Получи: https://aistudio.google.com/apikey".into())
                    } else {
                        AiError::Api { status: 400, body }
                    }
                }
                ureq::Error::Status(status, resp) => {
                    AiError::Api { status, body: resp.into_string().unwrap_or_default() }
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