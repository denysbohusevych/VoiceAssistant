// src/coordinator.rs
use crossbeam_channel::{unbounded, Receiver, Sender};
use crate::events::WorkerEvent;

#[derive(Default, Debug)]
pub struct SessionData {
    pub target_pid: u32,
    pub app_name: Option<String>,
    pub ax_path_json: Option<String>,
    pub transcription: String,
    pub screen_markdown: Option<String>,
    pub is_audio_finished: bool,
    pub is_vision_finished: bool,
}

impl SessionData {
    pub fn new(pid: u32) -> Self {
        Self {
            target_pid: pid,
            ..Default::default()
        }
    }

    pub fn is_ready_for_processing(&self) -> bool {
        self.is_audio_finished && self.is_vision_finished
    }

    pub fn build_final_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!("# Запрос к приложению: {}\n\n", self.app_name.as_deref().unwrap_or("Unknown")));

        md.push_str("## Контекст экрана:\n");
        md.push_str(self.screen_markdown.as_deref().unwrap_or("*(Экран пуст или скриншот не обработан)*\n"));

        md.push_str("\n## Транскрипция голоса:\n");
        md.push_str(self.transcription.trim());
        md.push('\n');

        md
    }
}

pub struct Coordinator {
    rx: Receiver<WorkerEvent>,
    tx: Sender<WorkerEvent>,
    current_session: Option<SessionData>,
}

impl Coordinator {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self { rx, tx, current_session: None }
    }

    pub fn get_sender(&self) -> Sender<WorkerEvent> {
        self.tx.clone()
    }

    pub fn start_new_session(&mut self, pid: u32) {
        self.current_session = Some(SessionData::new(pid));
        println!("\n[Coordinator] 🚀 Новая сессия начата для PID: {}", pid);
    }

    pub fn run(&mut self) {
        while let Ok(event) = self.rx.recv() {
            match event {
                // НОВОЕ: Оркестратор слушает старт сессии
                WorkerEvent::SessionStarted(pid) => {
                    self.start_new_session(pid);
                }
                WorkerEvent::ContextCaptured { app_name, ax_path_json } => {
                    if let Some(session) = &mut self.current_session {
                        session.app_name = Some(app_name);
                        session.ax_path_json = ax_path_json;
                    }
                }
                WorkerEvent::VisionProcessed(md) => {
                    if let Some(session) = &mut self.current_session {
                        session.screen_markdown = Some(md);
                        session.is_vision_finished = true;
                        println!("[Coordinator] 👁️ Зрение обработано.");
                        self.check_completion();
                    }
                }
                WorkerEvent::VisionError(err) => {
                    eprintln!("[Coordinator] ⚠️ Ошибка зрения: {}", err);
                    if let Some(session) = &mut self.current_session {
                        session.is_vision_finished = true;
                        self.check_completion();
                    }
                }
                WorkerEvent::PartialTranscription(text) => {
                    print!("{} ", text);
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                    if let Some(session) = &mut self.current_session {
                        if !session.transcription.is_empty() {
                            session.transcription.push(' ');
                        }
                        session.transcription.push_str(&text);
                    }
                }
                WorkerEvent::FinalTranscription(text) => {
                    if let Some(session) = &mut self.current_session {
                        if !session.transcription.is_empty() {
                            session.transcription.push(' ');
                        }
                        session.transcription.push_str(&text);
                    }
                }
                WorkerEvent::AudioFinished => {
                    if let Some(session) = &mut self.current_session {
                        session.is_audio_finished = true;
                        println!("\n[Coordinator] 🎤 Аудио поток завершен.");
                        self.check_completion();
                    }
                }
                WorkerEvent::AudioError(err) => {
                    eprintln!("\n[Coordinator] ⚠️ Ошибка аудио: {}", err);
                    if let Some(session) = &mut self.current_session {
                        session.is_audio_finished = true;
                        self.check_completion();
                    }
                }
            }
        }
    }

    fn check_completion(&mut self) {
        let is_ready = self.current_session.as_ref().map(|s| s.is_ready_for_processing()).unwrap_or(false);

        if is_ready {
            if let Some(session) = self.current_session.take() {
                println!("[Coordinator] ✅ Сессия собрана! Готовим финальный Markdown...");
                let final_markdown = session.build_final_markdown();

                let ts = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
                let filename = format!("final_request_{}.md", ts);
                if let Err(e) = std::fs::write(&filename, &final_markdown) {
                    eprintln!("[Coordinator] ❌ Ошибка сохранения файла: {}", e);
                } else {
                    println!("[Coordinator] 📝 Результат успешно сохранен в файл: {}", filename);
                }
            }
        }
    }
}