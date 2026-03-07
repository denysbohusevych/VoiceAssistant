#[cfg(target_os = "macos")]
pub mod macos;

use std::fmt;
use crossbeam_channel::{Receiver, Sender};
use crate::events::{PipelineAction, WorkerEvent};
use crate::vision::layout::build_layout_from_dump;

/// Снимок активного приложения.
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub app_name: String,
    pub pid:      u32,
    pub cursor:   (f64, f64),
    pub window_id: Option<u32>,
    pub screenshot: Option<Vec<u8>>,
    pub ax_element_path: Option<String>,
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
    fn capture_for_pid(&self, pid: u32) -> Result<AppSnapshot, ContextError>;
}

/// Запускает Воркер Контекста и Зрения в отдельном потоке
pub fn spawn_worker(
    action_rx: Receiver<PipelineAction>,
    event_tx: Sender<WorkerEvent>,
    capture_impl: Box<dyn ContextCapture>,
) {
    std::thread::spawn(move || {
        while let Ok(action) = action_rx.recv() {
            match action {
                PipelineAction::StartSession { target_pid } => {
                    // 1. Мгновенно собираем базовый контекст (для инжекта)
                    match capture_impl.capture_for_pid(target_pid) {
                        Ok(snapshot) => {
                            let _ = event_tx.send(WorkerEvent::ContextCaptured {
                                app_name: snapshot.app_name.clone(),
                                ax_path_json: snapshot.ax_element_path.clone(),
                            });

                            // 2. Асинхронно обрабатываем скриншот (Зрение / OCR)
                            if let Some(png_bytes) = snapshot.screenshot {
                                let tmp_path = format!("/tmp/vision_{}.png", target_pid);

                                if std::fs::write(&tmp_path, &png_bytes).is_ok() {
                                    let mut ax_helper = std::env::current_exe().unwrap_or_default();
                                    ax_helper.pop();
                                    ax_helper.push("ax-helper-bin");

                                    // Запускаем тяжелый процесс парсинга экрана
                                    match std::process::Command::new(&ax_helper)
                                        .arg("dump-screen")
                                        .arg(target_pid.to_string())
                                        .arg(&tmp_path)
                                        .output()
                                    {
                                        Ok(out) if out.status.success() => {

                                            let stderr = String::from_utf8_lossy(&out.stderr);
                                            if !stderr.trim().is_empty() {
                                                eprintln!("{}", stderr);
                                            }

                                            let json_str = String::from_utf8_lossy(&out.stdout);
                                            let md_text = build_layout_from_dump(&json_str);
                                            let _ = event_tx.send(WorkerEvent::VisionProcessed(md_text));
                                        }
                                        Ok(out) => {
                                            let err = String::from_utf8_lossy(&out.stderr);
                                            let _ = event_tx.send(WorkerEvent::VisionError(format!("ax-helper err: {}", err.trim())));
                                        }
                                        Err(e) => {
                                            let _ = event_tx.send(WorkerEvent::VisionError(e.to_string()));
                                        }
                                    }

                                    // Убираем за собой temp файл
                                    let _ = std::fs::remove_file(&tmp_path);
                                } else {
                                    let _ = event_tx.send(WorkerEvent::VisionError("Ошибка записи temp PNG".into()));
                                }
                            } else {
                                let _ = event_tx.send(WorkerEvent::VisionProcessed("*(Скриншот не захвачен: нет прав Screen Recording)*\n".into()));
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(WorkerEvent::VisionError(e.to_string()));
                        }
                    }
                }
                PipelineAction::StopSession => {
                    // Контекст реагирует только на старт сессии, остановку игнорируем
                }
            }
        }
    });
}