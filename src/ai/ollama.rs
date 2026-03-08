// src/ai/ollama.rs
//
// Провайдер: Ollama (локальные модели)
// Документация: https://github.com/ollama/ollama/blob/main/docs/api.md
//
// Дефолтный адрес: http://localhost:11434
// Дефолтная модель: llama3
//
// Установка: https://ollama.com
// Скачать модель: ollama pull llama3
// Популярные модели: llama3, mistral, qwen2.5, phi3, gemma3, codellama

use serde_json::{json, Value};
use super::{AiClient, AiError};

pub struct OllamaClient {
    base_url: String,
    model:    String,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), model: model.into() }
    }
}

impl AiClient for OllamaClient {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError> {
        let url = format!("{}/api/chat", self.base_url);

        let body = json!({
            "model": self.model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_message  }
            ],
            "options": {
                "temperature": 0.7,
                "num_predict": 2048
            }
        });

        let resp = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| match e {
                ureq::Error::Status(status, resp) => {
                    let body = resp.into_string().unwrap_or_default();
                    AiError::Api { status, body }
                }
                other => AiError::Http(format!(
                    "Не удалось подключиться к Ollama ({}). \
                     Убедись что запущен: ollama serve\n{}",
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