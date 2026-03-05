pub mod whisper;

/// Ошибки транскрипции.
#[derive(Debug)]
pub enum TranscriberError {
    /// Модель не найдена по указанному пути
    ModelNotFound(String),
    /// Ошибка инициализации движка
    InitError(String),
    /// Ошибка во время транскрипции
    TranscribeError(String),
}

impl std::fmt::Display for TranscriberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelNotFound(p)   => write!(f, "Модель не найдена: {p}"),
            Self::InitError(s)       => write!(f, "Ошибка инициализации: {s}"),
            Self::TranscribeError(s) => write!(f, "Ошибка транскрипции: {s}"),
        }
    }
}

impl std::error::Error for TranscriberError {}

/// Конфигурация транскрипции.
#[derive(Debug, Clone)]
pub struct TranscribeConfig {
    /// Язык аудио. None = автоопределение.
    /// Примеры: "ru", "en", "uk", "auto"
    pub language: Option<String>,
    /// Переводить в английский? (false = оставить оригинальный язык)
    pub translate: bool,
    /// Количество потоков CPU для инференса
    pub n_threads: i32,
}

impl Default for TranscribeConfig {
    fn default() -> Self {
        Self {
            language: None,     // автоопределение
            translate: false,
            n_threads: num_cpus(),
        }
    }
}

impl TranscribeConfig {
    pub fn with_language(lang: impl Into<String>) -> Self {
        Self {
            language: Some(lang.into()),
            ..Default::default()
        }
    }
}

/// Результат транскрипции — текст с временны́ми метками.
#[derive(Debug, Clone)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// Общий интерфейс транскрибера.
/// Позволяет подключить любой движок (whisper.cpp, cloud API и т.д.)
pub trait Transcriber: Send {
    /// Транскрибировать аудио из PCM f32 (16kHz, mono).
    fn transcribe(
        &mut self,
        samples: &[f32],
        config: &TranscribeConfig,
    ) -> Result<Vec<Segment>, TranscriberError>;
}

/// Определяем количество логических CPU для параллелизма.
fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8) // whisper плохо масштабируется > 8 потоков
}