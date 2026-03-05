//! macOS вставка текста через ax-helper Swift бинарь.
//!
//! Протокол:
//!   1. При захвате контекста: ax-helper capture <pid>
//!      → JSON с ElementPath (путь через AX-дерево до поля ввода)
//!
//!   2. При вставке:  ax-helper inject <pid> <path.json-tmpfile> <text>
//!      → активирует приложение, восстанавливает AX-фокус, Cmd+V

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use crate::context::AppSnapshot;
use super::{InjectorError, TextInjector};

/// Путь к ax-helper бинарю — рядом с исполняемым файлом.
fn ax_helper_path() -> PathBuf {
    // Ищем рядом с текущим exe
    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidate = exe.join("ax-helper-bin");
        if candidate.exists() { return candidate; }
    }
    // Fallback — рядом с рабочей директорией
    PathBuf::from("ax-helper/ax-helper")
}

pub struct MacOsTextInjector {
    helper: PathBuf,
}

impl MacOsTextInjector {
    pub fn new() -> Self {
        let helper = ax_helper_path();
        if !helper.exists() {
            eprintln!(
                "⚠  ax-helper-ишт не найден: {}\n   Собери его: sh ax-helper/build.sh",
                helper.display()
            );
        }
        Self { helper }
    }
}

impl TextInjector for MacOsTextInjector {
    fn inject(&self, text: &str, snapshot: &AppSnapshot) -> Result<(), InjectorError> {
        let ax_path_json = snapshot.ax_element_path.as_deref()
            .ok_or_else(|| InjectorError::EventError(
                "Нет сохранённого AX-пути — захват контекста не включал ax-helper".into()
            ))?;

        // Пишем JSON во временный файл (избегаем проблем с экранированием в argv)
        let tmp = write_tmp_json(ax_path_json)?;

        let output = Command::new(&self.helper)
            .args(["inject", &snapshot.pid.to_string(), tmp.to_str().unwrap(), text])
            .output()
            .map_err(|e| InjectorError::EventError(format!("Не могу запустить ax-helper: {e}")))?;

        // Удаляем tmp файл
        let _ = std::fs::remove_file(&tmp);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(InjectorError::EventError(format!("ax-helper inject: {stderr}")));
        }

        Ok(())
    }
}

/// Пишет строку в temp-файл, возвращает путь.
fn write_tmp_json(json: &str) -> Result<PathBuf, InjectorError> {
    let path = std::env::temp_dir().join(format!("va_ax_{}.json", std::process::id()));
    let mut f = std::fs::File::create(&path)
        .map_err(|e| InjectorError::EventError(e.to_string()))?;
    f.write_all(json.as_bytes())
        .map_err(|e| InjectorError::EventError(e.to_string()))?;
    Ok(path)
}

/// Захватить AX-путь для PID — вызывается из context::capture().
/// Возвращает JSON-строку с ElementPath.
pub fn capture_ax_path(pid: u32) -> Option<String> {
    let helper = ax_helper_path();
    if !helper.exists() { return None; }

    let output = Command::new(&helper)
        .args(["capture", &pid.to_string()])
        .output()
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("ax-helper capture: {stderr}");
        return None;
    }

    let json = String::from_utf8(output.stdout).ok()?;
    if json.trim().is_empty() { return None; }
    Some(json.trim().to_string())
}