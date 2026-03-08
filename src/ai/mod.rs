// src/ai/mod.rs

pub mod gemini;
pub mod openai;
pub mod claude;
pub mod ollama;
pub mod deepseek;
pub mod http;

use std::fmt;
use crate::config::{AppConfig, SharedConfig};

// ─── Ошибки ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AiError {
    Http(String),
    Parse(String),
    Auth(String),
    Api { status: u16, body: String },
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(s)              => write!(f, "HTTP error: {s}"),
            Self::Parse(s)             => write!(f, "Parse error: {s}"),
            Self::Auth(s)              => write!(f, "Auth error: {s}"),
            Self::Api { status, body } => write!(f, "API error {status}: {body}"),
        }
    }
}

impl std::error::Error for AiError {}

// ─── Трейт ────────────────────────────────────────────────────────────────────

pub trait AiClient: Send + Sync {
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError>;
    fn model_name(&self)    -> &str;
    fn provider_name(&self) -> &str;
}

// ─── Системный промпт по умолчанию ────────────────────────────────────────────

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a concise assistant.
Do not output reasoning, thinking, or chain-of-thought.
Return only the final answer.
You are a UI layout analyzer. You receive raw OCR output structured as XML geometry — \
coordinates, font sizes, and text. No semantic roles are pre-assigned.

Your task is to reason step by step about the interface and describe what you see. \
This is a DEBUG session — be explicit about uncertainty, guesses, and anything that looks broken.

---

## STEP 1 — App & Screen Type
- What application is this likely from? What clues tell you that?
- What type of screen is this? (chat, document, settings, dashboard, login, feed, editor, etc.)
- How confident are you? What could make you wrong?

## STEP 2 — Column Structure
- How many visual columns/panels do you see?
- What is the purpose of each column? (sidebar, main content, detail pane, toolbar, etc.)
- Are any columns ambiguous or hard to distinguish?

## STEP 3 — Block-by-Block Walkthrough
For EACH <Block> in reading order (top-to-bottom, left-to-right within column):
  - What is this block? (message bubble, button, label, header, list item, timestamp, icon label, etc.)
  - What is its content?
  - Is anything suspicious about it? (merged lines that shouldn't be, split blocks that should be one, etc.)

Format each block as:
  [col=N, y=NNN] TYPE — \"content\" — notes

## STEP 4 — Layout Problems Detected
List every anomaly you notice:
  - Lines that look incorrectly merged (e.g. timestamp glued to message text)
  - Blocks that should probably be split (two separate UI elements clustered together)
  - Blocks that should probably be merged (one logical element split across blocks)
  - Text that is clearly OCR noise or garbled
  - Anything that seems geometrically impossible or contradictory

Format each problem as:
  PROBLEM [col=N, y=NNN]: description — likely cause — suggested fix

## STEP 5 — Reading Order
Write the full content of the screen in natural reading order, as a human would read it top-to-bottom. \
Skip obvious UI chrome (timestamps, read receipts, scrollbars) unless they carry meaning. \
Mark uncertain parts with [?].

## STEP 6 — Confidence Summary
- Overall confidence in your interpretation: HIGH / MEDIUM / LOW
- What additional context would most improve your understanding? \
  (e.g. screenshot, app name, screen resolution, language hint)
- Which columns/blocks are you least confident about and why?
";

// ─── Фабрика ──────────────────────────────────────────────────────────────────

/// Создаёт AI-клиент на основе конфига.
/// `shared` передаётся в клиент для чтения temperature/max_tokens в runtime.
pub fn build_ai_client_from_config(
    cfg: &AppConfig,
    shared: SharedConfig,
) -> Box<dyn AiClient> {
    let provider  = cfg.ai.provider.as_str();
    let model     = cfg.ai.model.clone();

    match provider {
        "gemini" => Box::new(gemini::GeminiClient::new(
            env_key("GEMINI_API_KEY"),
            model,
            shared,
        )),
        "openai" => Box::new(openai::OpenAiClient::new(
            env_key("OPENAI_API_KEY"),
            model,
            shared,
        )),
        "claude" => Box::new(claude::ClaudeClient::new(
            env_key("ANTHROPIC_API_KEY"),
            model,
            shared,
        )),
        "deepseek" => Box::new(deepseek::DeepSeekClient::new(
            env_key("DEEPSEEK_API_KEY"),
            model,
            shared,
        )),
        "ollama" | _ => {
            let base_url = cfg.ai.ollama.base_url.clone();
            Box::new(ollama::OllamaClient::new(base_url, model, shared))
        }
    }
}

/// Читает ключ API из переменной окружения с понятным сообщением при отсутствии.
pub fn env_key(var: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| {
        eprintln!(
            "⚠️  Переменная окружения {} не установлена.\n   \
             Установи: export {}=\"твой-ключ\"\n   \
             Или добавь в ~/.zshrc",
            var, var
        );
        String::new()
    })
}

// ─── Устаревший строитель (оставлен для совместимости) ────────────────────────

/// Используй `build_ai_client_from_config` для новых проектов.
pub struct AiConfig {
    provider: Provider,
    model:    Option<String>,
}

enum Provider {
    Gemini   { api_key: String },
    OpenAi   { api_key: String },
    Claude   { api_key: String },
    DeepSeek { api_key: String },
    Ollama   { base_url: String },
}

impl AiConfig {
    pub fn gemini(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Gemini { api_key: api_key.into() }, model: None }
    }
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::OpenAi { api_key: api_key.into() }, model: None }
    }
    pub fn claude(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Claude { api_key: api_key.into() }, model: None }
    }
    pub fn deepseek(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::DeepSeek { api_key: api_key.into() }, model: None }
    }
    pub fn ollama() -> Self {
        Self::ollama_at("http://localhost:11434")
    }
    pub fn ollama_at(base_url: impl Into<String>) -> Self {
        Self { provider: Provider::Ollama { base_url: base_url.into() }, model: None }
    }
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}