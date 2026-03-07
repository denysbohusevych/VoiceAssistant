use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};
use super::Transcriber;
use std::collections::HashSet;

fn cleanup_and_deduplicate(text: &str) -> String {
    let clean = text
        .replace("[BLANK_AUDIO]", "")
        .replace("[Silence]", "")
        .replace("(silence)", "")
        .replace("[Музыка]", "")
        .replace("[Music]", "")
        .replace("Редактор субтитров", "")
        .replace("Корректор", "")
        .replace("А.Семкин", "")
        .replace("А.Синецкая", "");

    let mut parts = Vec::new();
    let mut current = String::new();
    for c in clean.chars() {
        current.push(c);
        if c == '.' || c == '!' || c == '?' {
            if !current.trim().is_empty() {
                parts.push(current.trim().to_string());
            }
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    let mut deduplicated = Vec::new();
    let mut seen = HashSet::new();

    for part in parts {
        let normalized: String = part.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();

        if !normalized.is_empty() && !seen.contains(&normalized) {
            seen.insert(normalized);
            deduplicated.push(part);
        }
    }

    deduplicated.join(" ")
}

pub struct WhisperTranscriber {
    context: WhisperContext,
    // В 0.15.1 лайфтаймы были убраны, храним просто State
    state: Option<WhisperState>,
}

impl Drop for WhisperTranscriber {
    fn drop(&mut self) {
        // Очищаем стейт при удалении
        self.state.take();
    }
}

impl WhisperTranscriber {
    pub fn new(model_path: &str) -> Result<Self, String> {
        std::env::set_var("GGML_METAL_NDEBUG", "1");

        let params = WhisperContextParameters::default();
        let context = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| format!("Ошибка загрузки модели Whisper: {}", e))?;

        Ok(Self { context, state: None })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, audio_data: &[f32]) -> Result<String, String> {
        if audio_data.len() < 4000 {
            return Ok(String::new());
        }

        if audio_data.is_empty() {
            return Ok(String::new());
        }

        // Если это первый запуск сессии, аллоцируем State ровно 1 раз
        if self.state.is_none() {
            let s = self.context.create_state()
                .map_err(|e| format!("Ошибка создания state: {}", e))?;
            self.state = Some(s);
        }

        let state = self.state.as_mut().unwrap();

        // В 0.15.1 возвращаем best_of: 1
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Явно передаем "auto", чтобы избежать ошибки вывода типа для Option<str>
        params.set_language(Some("auto"));
        // Запрещаем перевод (чтобы билингвальный текст оставался "как есть")
        params.set_translate(false);

        // Prompt-инъекция: заставляем модель понимать программирование и переключение языков
        params.set_initial_prompt("Привет, это test voice command. Я пишу код на Rust, Python. System initialization.");

        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        params.set_suppress_blank(true);
        // set_suppress_non_speech_tokens был удален в последних версиях whisper.cpp

        params.set_no_context(true);
        params.set_no_speech_thold(0.4);
        params.set_entropy_thold(2.4);

        state.full(params, audio_data)
            .map_err(|e| format!("Ошибка инференса: {}", e))?;

        let num_segments = state.full_n_segments();
        if num_segments < 0 {
            return Err(format!("Ошибка получения сегментов: {}", num_segments));
        }

        let mut raw_result = String::new();

        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(text) = segment.to_str() {
                    raw_result.push_str(text);
                    raw_result.push(' ');
                }
            }
        }

        let final_text = cleanup_and_deduplicate(&raw_result).trim().to_string();

        let hallucinations = [
            "okay", "yep", "sounds good", "cool", "damn",
            "hello", "это я", "как меня слышно", "я не знаю что я делаю",
            "алла халол", "hello hello", "hello hello hello", "как ты меня сейчас слышишь",
            "привет можно я тебя послать"
        ];

        let normalized_final: String = final_text.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();

        if hallucinations.iter().any(|&h| normalized_final.trim() == h || normalized_final.trim().starts_with("hello hello")) {
            return Ok(String::new());
        }

        Ok(final_text)
    }
}