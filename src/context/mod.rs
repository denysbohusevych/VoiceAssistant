#[cfg(target_os = "macos")]
pub mod macos;

use std::fmt;

/// Снимок активного приложения в момент нажатия хоткея.
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub app_name: String,
    pub pid:      u32,
    pub cursor:   (f64, f64),
    pub window_id: Option<u32>,
    /// PNG-скриншот активного окна (None если нет Screen Recording прав)
    pub screenshot: Option<Vec<u8>>,
    /// JSON-путь через AX-дерево до поля ввода (из ax-helper capture).
    /// Стабилен при ресайзе окна — идёт по роли+индексу, не по координатам.
    pub ax_element_path: Option<String>,
}

impl AppSnapshot {
    pub fn save_screenshot(&self, path: &std::path::Path) -> std::io::Result<()> {
        match &self.screenshot {
            Some(png) => std::fs::write(path, png),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound, "Скриншот не захвачен",
            )),
        }
    }
}

impl fmt::Display for AppSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ss  = if self.screenshot.is_some()       { "📸" } else { "  " };
        let ax  = if self.ax_element_path.is_some()  { "🎯" } else { "  " };
        write!(f, "{} (pid={}) {ss}{ax}", self.app_name, self.pid)
    }
}

#[derive(Debug)]
pub enum ContextError {
    NoFrontmostApp,
    ApiError(String),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrontmostApp => write!(f, "Нет активного приложения"),
            Self::ApiError(s)    => write!(f, "Ошибка API: {s}"),
        }
    }
}

impl std::error::Error for ContextError {}

pub trait ContextCapture: Send + Sync {
    fn capture(&self) -> Result<AppSnapshot, ContextError>;
    fn capture_for_pid(&self, pid: u32) -> Result<AppSnapshot, ContextError>;
}