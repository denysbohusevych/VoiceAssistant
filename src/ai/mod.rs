// src/ai/mod.rs
//
// ╔══════════════════════════════════════════════════════════════════════════════╗
// ║  AI-модуль: единый интерфейс для всех провайдеров                          ║
// ║                                                                              ║
// ║  Поддерживаемые провайдеры:                                                 ║
// ║    • Gemini  (Google)    — по умолчанию: gemini-2.0-flash                  ║
// ║    • OpenAI              — gpt-4o, gpt-4o-mini, o1, ...                    ║
// ║    • Claude  (Anthropic) — claude-opus-4-6, claude-sonnet-4-6, ...         ║
// ║    • DeepSeek            — deepseek-chat, deepseek-reasoner, ...            ║
// ║    • Ollama  (локально)  — llama3, mistral, qwen2.5, ...                   ║
// ║                                                                              ║
// ║  Как переключить модель (одна строка в main.rs):                            ║
// ║    let ai = AiConfig::gemini("...key...").build();                          ║
// ║    let ai = AiConfig::openai("...key...").model("gpt-4o").build();          ║
// ║    let ai = AiConfig::ollama().model("llama3").build();                     ║
// ╚══════════════════════════════════════════════════════════════════════════════╝

pub mod gemini;
pub mod openai;
pub mod claude;
pub mod ollama;
pub mod deepseek;
pub mod http;

use std::fmt;

// ─── Ошибки ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AiError {
    /// HTTP-запрос не отправился (сеть, таймаут)
    Http(String),
    /// Сервер ответил, но JSON неожиданный
    Parse(String),
    /// Ключ API невалидный или отсутствует
    Auth(String),
    /// Сервер вернул код ошибки с телом
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

// ─── Единый трейт ─────────────────────────────────────────────────────────────

/// Любой LLM-провайдер реализует этот трейт.
/// Создай свой провайдер → реализуй два метода → передай `Box<dyn AiClient>`.
pub trait AiClient: Send + Sync {
    /// Отправить запрос, получить ответ.
    ///
    /// `system_prompt` — системный промпт (инструкции для модели).
    /// `user_message`  — пользовательский запрос (маркдаун с контекстом экрана + голос).
    fn complete(&self, system_prompt: &str, user_message: &str) -> Result<String, AiError>;

    /// Имя модели для логов.
    fn model_name(&self) -> &str;

    /// Имя провайдера для логов.
    fn provider_name(&self) -> &str;
}

// ─── Системный промпт по умолчанию ────────────────────────────────────────────

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
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

// ─── Конфиг и фабрика ─────────────────────────────────────────────────────────

/// Удобный строитель для создания клиента.
///
/// Примеры:
/// ```
/// // Gemini (по умолчанию — gemini-2.0-flash)
/// let ai = AiConfig::gemini("AIza...").build();
///
/// // OpenAI с другой моделью
/// let ai = AiConfig::openai("sk-...").model("gpt-4o-mini").build();
///
/// // Локальная Ollama
/// let ai = AiConfig::ollama().model("qwen2.5:7b").build();
///
/// // Claude
/// let ai = AiConfig::claude("sk-ant-...").model("claude-sonnet-4-6").build();
///
/// // DeepSeek
/// let ai = AiConfig::deepseek("...").model("deepseek-reasoner").build();
/// ```
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
    /// Google Gemini. Дефолтная модель: `gemini-2.0-flash`.
    pub fn gemini(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Gemini { api_key: api_key.into() }, model: None }
    }

    /// OpenAI. Дефолтная модель: `gpt-4o`.
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::OpenAi { api_key: api_key.into() }, model: None }
    }

    /// Anthropic Claude. Дефолтная модель: `claude-sonnet-4-6`.
    pub fn claude(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Claude { api_key: api_key.into() }, model: None }
    }

    /// DeepSeek. Дефолтная модель: `deepseek-chat`.
    pub fn deepseek(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::DeepSeek { api_key: api_key.into() }, model: None }
    }

    /// Ollama (локально). Дефолтный адрес: `http://localhost:11434`. Дефолтная модель: `llama3`.
    pub fn ollama() -> Self {
        Self::ollama_at("http://localhost:11434")
    }

    /// Ollama с кастомным адресом.
    pub fn ollama_at(base_url: impl Into<String>) -> Self {
        Self { provider: Provider::Ollama { base_url: base_url.into() }, model: None }
    }

    /// Переопределить модель.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Создать клиента.
    pub fn build(self) -> Box<dyn AiClient> {
        match self.provider {
            Provider::Gemini { api_key } => Box::new(
                gemini::GeminiClient::new(api_key, self.model.unwrap_or_else(|| "gemini-2.5-flash".into()))
            ),
            Provider::OpenAi { api_key } => Box::new(
                openai::OpenAiClient::new(api_key, self.model.unwrap_or_else(|| "gpt-4o".into()))
            ),
            Provider::Claude { api_key } => Box::new(
                claude::ClaudeClient::new(api_key, self.model.unwrap_or_else(|| "claude-sonnet-4-6".into()))
            ),
            Provider::DeepSeek { api_key } => Box::new(
                deepseek::DeepSeekClient::new(api_key, self.model.unwrap_or_else(|| "deepseek-chat".into()))
            ),
            Provider::Ollama { base_url } => Box::new(
                ollama::OllamaClient::new(base_url, self.model.unwrap_or_else(|| "qwen2.5:7b".into()))
            ),
        }
    }
}