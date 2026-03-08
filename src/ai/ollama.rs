// src/ai/ollama.rs
use serde_json::{json, Value};
use crate::config::SharedConfig;
use super::{AiClient, AiError};

pub struct OllamaClient {
    base_url: String,
    model:    String,
    cfg:      SharedConfig,
}

impl OllamaClient {
    pub fn new(
        base_url: impl Into<String>,
        model:    impl Into<String>,
        cfg:      SharedConfig,
    ) -> Self {
        Self { base_url: base_url.into(), model: model.into(), cfg }
    }
}

impl AiClient for OllamaClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let (temperature, num_predict, num_ctx) = {
            let c = self.cfg.read().unwrap();
            (c.ai.temperature, c.ai.ollama.num_predict, c.ai.ollama.num_ctx)
        };

        let url = format!("{}/api/chat", self.base_url);

        let body = json!({
            "model":  self.model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_message  }
            ],
            "options": {
                "temperature": temperature,
                "num_predict": num_predict,
                "num_ctx":     num_ctx
            }
        });

        let resp = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(status, resp) => {
                    AiError::Api { status, body: resp.into_string().unwrap_or_default() }
                }
                other => AiError::Http(format!(
                    "Не удалось подключиться к Ollama ({}). Запусти: ollama serve\n{}",
                    self.base_url, other
                )),
            })?;

        let json: Value = resp.into_json()
            .map_err(|e| AiError::Parse(e.to_string()))?;

        json["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AiError::Parse(format!("Unexpected Ollama response: {}", json)))
    }

    fn model_name(&self)    -> &str { &self.model }
    fn provider_name(&self) -> &str { "Ollama" }
}