// src/ai/http.rs
//
// HTTP-агент с таймаутами из конфига.

use ureq::Agent;
use std::time::Duration;
use serde_json::Value;
use crate::config::HttpTimeoutsConfig;
use super::AiError;

/// Создаёт агента с таймаутами из конфига.
pub fn make_agent(timeouts: &HttpTimeoutsConfig) -> Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(timeouts.connect_secs))
        .timeout_read(Duration::from_secs(timeouts.read_secs))
        .timeout_write(Duration::from_secs(timeouts.write_secs))
        .build()
}

/// Создаёт агента с дефолтными таймаутами.
pub fn make_default_agent() -> Agent {
    make_agent(&HttpTimeoutsConfig::default())
}

/// Читает тело ответа как строку и парсит JSON.
/// Защищает от частичного чтения BufReader'а при больших ответах.
pub fn parse_json_response(resp: ureq::Response) -> Result<Value, AiError> {
    let body = resp
        .into_string()
        .map_err(|e| AiError::Parse(format!("Не удалось прочитать тело ответа: {}", e)))?;

    serde_json::from_str(&body).map_err(|e| {
        let preview = if body.len() > 300 { &body[..300] } else { &body };
        AiError::Parse(format!("JSON parse error: {}\nТело: {}…", e, preview))
    })
}