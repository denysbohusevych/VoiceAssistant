// src/coordinator.rs
//
// Оркестратор сессий.
// Собирает данные от всех воркеров (аудио, контекст, зрение),
// затем отправляет финальный маркдаун в LLM и сохраняет ответ.

use crossbeam_channel::{unbounded, Receiver, Sender};
use crate::events::WorkerEvent;
use crate::ai::{AiClient, DEFAULT_SYSTEM_PROMPT};

// ─── Данные сессии ────────────────────────────────────────────────────────────

#[derive(Default, Debug)]
pub struct SessionData {
    pub target_pid:             u32,
    pub app_name:               Option<String>,
    pub ax_path_json:           Option<String>,
    pub transcription:          String,
    pub partial_transcription:  Option<String>,
    pub screen_markdown:        Option<String>,
    pub is_audio_finished:      bool,
    pub is_vision_finished:     bool,
}

impl SessionData {
    pub fn new(pid: u32) -> Self {
        Self { target_pid: pid, ..Default::default() }
    }

    pub fn is_ready_for_processing(&self) -> bool {
        self.is_audio_finished && self.is_vision_finished
    }

    /// Маркдаун с полным контекстом — он уйдёт в LLM как user message.
    pub fn build_context_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!(
            "# Запрос к приложению: {}\n\n",
            self.app_name.as_deref().unwrap_or("Unknown")
        ));

        md.push_str("## Контекст экрана:\n");
        md.push_str(
            self.screen_markdown
                .as_deref()
                .unwrap_or("*(Экран пуст или скриншот не обработан)*\n")
        );

        md.push_str("\n## Транскрипция голоса:\n");
        md.push_str(self.transcription.trim());

        if let Some(partial) = &self.partial_transcription {
            if !self.transcription.is_empty() { md.push(' '); }
            md.push_str(partial.trim());
        }
        md.push('\n');

        md
    }
}

// ─── Координатор ──────────────────────────────────────────────────────────────

pub struct Coordinator {
    rx:              Receiver<WorkerEvent>,
    tx:              Sender<WorkerEvent>,
    current_session: Option<SessionData>,

    /// LLM-клиент (Gemini / OpenAI / Claude / DeepSeek / Ollama)
    ai_client:       Box<dyn AiClient>,

    /// Системный промпт — можно заменить из main.rs
    system_prompt:   String,
}

impl Coordinator {
    /// Создать с кастомным AI-клиентом и системным промптом.
    pub fn new(ai_client: Box<dyn AiClient>, system_prompt: impl Into<String>) -> Self {
        let (tx, rx) = unbounded();
        Self {
            rx,
            tx,
            current_session: None,
            ai_client,
            system_prompt: system_prompt.into(),
        }
    }

    /// Создать с дефолтным системным промптом.
    pub fn with_client(ai_client: Box<dyn AiClient>) -> Self {
        Self::new(ai_client, DEFAULT_SYSTEM_PROMPT)
    }

    pub fn get_sender(&self) -> Sender<WorkerEvent> {
        self.tx.clone()
    }

    pub fn start_new_session(&mut self, pid: u32) {
        self.current_session = Some(SessionData::new(pid));
        println!(
            "\n[Coordinator] 🚀 Новая сессия | PID: {} | AI: {} ({})",
            pid,
            self.ai_client.provider_name(),
            self.ai_client.model_name(),
        );
    }

    // ─── Главный цикл ─────────────────────────────────────────────────────────

    pub fn run(&mut self) {
        while let Ok(event) = self.rx.recv() {
            match event {
                WorkerEvent::SessionStarted(pid) => {
                    self.start_new_session(pid);
                }

                WorkerEvent::ContextCaptured { app_name, ax_path_json } => {
                    if let Some(session) = &mut self.current_session {
                        session.app_name     = Some(app_name);
                        session.ax_path_json = ax_path_json;
                    }
                }

                WorkerEvent::VisionProcessed(md) => {
                    if let Some(session) = &mut self.current_session {
                        session.screen_markdown  = Some(md);
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
                    print!("\r\x1B[2K[Частично] {}", text);
                    use std::io::Write;
                    std::io::stdout().flush().ok();
                    if let Some(session) = &mut self.current_session {
                        session.partial_transcription = Some(text);
                    }
                }

                WorkerEvent::FinalTranscription(text) => {
                    println!("\r\x1B[2K[Финально] {}", text);
                    if let Some(session) = &mut self.current_session {
                        if !session.transcription.is_empty() {
                            session.transcription.push(' ');
                        }
                        session.transcription.push_str(&text);
                        session.partial_transcription = None;
                    }
                }

                WorkerEvent::AudioFinished => {
                    if let Some(session) = &mut self.current_session {
                        session.is_audio_finished = true;
                        println!("\n[Coordinator] 🎤 Аудио поток завершён.");
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

    // ─── Финализация сессии ───────────────────────────────────────────────────

    fn check_completion(&mut self) {
        let is_ready = self.current_session
            .as_ref()
            .map(|s| s.is_ready_for_processing())
            .unwrap_or(false);

        if !is_ready { return; }

        if let Some(session) = self.current_session.take() {
            self.process_completed_session(session);
        }
    }

    fn process_completed_session(&self, session: SessionData) {
        println!("[Coordinator] ✅ Сессия собрана. Отправляю в AI...");

        let context_md   = session.build_context_markdown();
        let ts           = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let app_slug     = session.app_name
            .as_deref()
            .unwrap_or("unknown")
            .replace(' ', "_")
            .to_lowercase();

        // Сохраняем исходный контекст (опционально, для отладки)
        let context_file = format!("context_{}_{}.md", app_slug, ts);
        if let Err(e) = std::fs::write(&context_file, &context_md) {
            eprintln!("[Coordinator] ⚠️ Не удалось сохранить контекст: {}", e);
        } else {
            println!("[Coordinator] 📋 Контекст сохранён: {}", context_file);
        }

        // Отправляем в LLM
        println!(
            "[Coordinator] 🤖 {} ({})...",
            self.ai_client.provider_name(),
            self.ai_client.model_name()
        );

        match self.ai_client.complete(&self.system_prompt, &context_md) {
            Ok(ai_response) => {
                println!("[Coordinator] 💬 Ответ получен ({} символов).", ai_response.len());

                // Формируем финальный файл: контекст + ответ AI
                let final_md = build_final_markdown(&context_md, &ai_response, &self.ai_client);

                let response_file = format!("response_{}_{}.md", app_slug, ts);
                match std::fs::write(&response_file, &final_md) {
                    Ok(_) => println!("[Coordinator] 📝 Ответ сохранён: {}", response_file),
                    Err(e) => eprintln!("[Coordinator] ❌ Ошибка сохранения ответа: {}", e),
                }

                // Выводим ответ в терминал
                println!("\n┌─ Ответ AI ─────────────────────────────────────────");
                for line in ai_response.lines() {
                    println!("│ {}", line);
                }
                println!("└─────────────────────────────────────────────────────\n");
            }
            Err(e) => {
                eprintln!("[Coordinator] ❌ Ошибка AI: {}", e);

                // При ошибке сохраняем контекст без ответа, чтобы не потерять данные
                let fallback_file = format!("failed_request_{}_{}.md", app_slug, ts);
                let fallback_md = format!(
                    "{}\n\n---\n\n## ❌ Ошибка AI\n\n{}\n",
                    context_md, e
                );
                let _ = std::fs::write(&fallback_file, &fallback_md);
                eprintln!("[Coordinator] 💾 Контекст сохранён без ответа: {}", fallback_file);
            }
        }
    }
}

// ─── Вспомогательные функции ──────────────────────────────────────────────────

fn build_final_markdown(
    context_md: &str,
    ai_response: &str,
    client: &Box<dyn AiClient>,
) -> String {
    format!(
        "{}\n\n---\n\n## 🤖 Ответ AI ({} / {})\n\n{}\n",
        context_md,
        client.provider_name(),
        client.model_name(),
        ai_response,
    )
}