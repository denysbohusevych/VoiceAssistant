// src/ai/http.rs
//
// Единый HTTP-агент для всех AI-провайдеров.
// Решает две проблемы:
//   1. Дефолтный таймаут ureq (~30с) слишком короткий для длинных AI-ответов
//   2. into_json() может прерваться не дочитав поток — читаем тело как String,
//      затем парсим вручную. Это также даёт читабельный текст ошибки при сбое.

use ureq::Agent;
use std::time::Duration;
use serde_json::Value;
use super::AiError;

/// Создаёт агента с увеличенными таймаутами.
/// Вызывай один раз при инициализации провайдера.
pub fn make_agent() -> Agent {
    ureq::AgentBuilder::new()
        // Таймаут на установку TCP-соединения
        .timeout_connect(Duration::from_secs(15))
        // Таймаут на чтение всего тела ответа
        // 120с — достаточно даже для медленных моделей (o1, DeepSeek-R1)
        .timeout_read(Duration::from_secs(120))
        // Таймаут на запись тела запроса
        .timeout_write(Duration::from_secs(30))
        .build()
}

/// Читает тело ответа как строку, затем парсит JSON.
///
/// Почему не resp.into_json():
///   ureq v2 использует BufReader поверх сокета. При больших ответах (~4KB+)
///   BufReader может вернуть Ok раньше чем поток закрылся, и into_json()
///   завершится с частичным JSON. Чтение в String буферизирует полностью.
pub fn parse_json_response(resp: ureq::Response) -> Result<Value, AiError> {
    let body = resp
        .into_string()
        .map_err(|e| AiError::Parse(format!("Не удалось прочитать тело ответа: {}", e)))?;

    serde_json::from_str(&body).map_err(|e| {
        // Показываем первые 300 символов тела для диагностики
        let preview = if body.len() > 300 { &body[..300] } else { &body };
        AiError::Parse(format!("JSON parse error: {}\nТело (начало): {}…", e, preview))
    })
}