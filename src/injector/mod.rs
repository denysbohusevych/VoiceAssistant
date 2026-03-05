/// Вставка текста в конкретное приложение по сохранённому снимку.

#[cfg(target_os = "macos")]
pub mod macos;

use std::fmt;
use crate::context::AppSnapshot;

#[derive(Debug)]
pub enum InjectorError {
    ClipboardError(String),
    EventError(String),
    PermissionDenied,
}

impl fmt::Display for InjectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardError(s) => write!(f, "Ошибка буфера обмена: {s}"),
            Self::EventError(s)     => write!(f, "Ошибка ввода событий: {s}"),
            Self::PermissionDenied  => write!(
                f,
                "Нет прав для симуляции ввода.\n\
                 System Settings → Privacy & Security → Accessibility"
            ),
        }
    }
}

impl std::error::Error for InjectorError {}

/// Интерфейс вставки текста.
pub trait TextInjector: Send + Sync {
    /// Вставить `text` в приложение описанное снимком `snapshot`.
    /// Работает даже если сейчас активно другое приложение.
    fn inject(&self, text: &str, snapshot: &AppSnapshot) -> Result<(), InjectorError>;
}