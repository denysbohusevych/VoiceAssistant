// src/transcriber/whisper.rs
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};
use std::collections::HashSet;
use crate::config::SharedConfig;
use super::Transcriber;

pub struct WhisperTranscriber {
    context: WhisperContext,
    state:   Option<WhisperState>,
    cfg:     SharedConfig,
}

impl Drop for WhisperTranscriber {
    fn drop(&mut self) { self.state.take(); }
}

impl WhisperTranscriber {
    pub fn new(model_path: &str, cfg: SharedConfig) -> Result<Self, String> {
        std::env::set_var("GGML_METAL_NDEBUG", "1");

        let params  = WhisperContextParameters::default();
        let context = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| format!("Ошибка загрузки модели Whisper: {}", e))?;

        Ok(Self { context, state: None, cfg })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, audio_data: &[f32]) -> Result<String, String> {
        // Читаем конфиг один раз на вызов
        let (min_samples, language, no_speech_thold, entropy_thold, initial_prompt) = {
            let c = self.cfg.read().unwrap();
            (
                c.whisper.min_audio_samples,
                c.whisper.language.clone(),
                c.whisper.no_speech_threshold,
                c.whisper.entropy_threshold,
                c.whisper.initial_prompt.clone(),
            )
        };

        if audio_data.len() < min_samples {
            return Ok(String::new());
        }

        if self.state.is_none() {
            let s = self.context.create_state()
                .map_err(|e| format!("Ошибка создания state: {}", e))?;
            self.state = Some(s);
        }

        let state = self.state.as_mut().unwrap();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        let lang = if language == "auto" { "auto" } else { &language };
        params.set_language(Some(lang));
        params.set_translate(false);
        params.set_initial_prompt(&initial_prompt);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_no_context(true);
        params.set_no_speech_thold(no_speech_thold);
        params.set_entropy_thold(entropy_thold);

        state.full(params, audio_data)
            .map_err(|e| format!("Ошибка инференса: {}", e))?;

        let num_segments = state.full_n_segments();
        if num_segments < 0 {
            return Err(format!("Ошибка получения сегментов: {}", num_segments));
        }

        let mut raw_result = String::new();
        for i in 0..num_segments {
            if let Some(seg) = state.get_segment(i) {
                if let Ok(text) = seg.to_str() {
                    raw_result.push_str(text);
                    raw_result.push(' ');
                }
            }
        }

        let final_text = cleanup_and_deduplicate(&raw_result).trim().to_string();

        // Фильтр галлюцинаций
        let normalized: String = final_text.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect();

        const HALLUCINATIONS: &[&str] = &[
            "okay", "yep", "sounds good", "cool", "damn",
            "hello", "это я", "как меня слышно", "я не знаю что я делаю",
            "алла халол", "hello hello", "hello hello hello",
        ];

        if HALLUCINATIONS.iter().any(|&h| normalized.trim() == h
            || normalized.trim().starts_with("hello hello"))
        {
            return Ok(String::new());
        }

        Ok(final_text)
    }
}

// ─── Вспомогательные функции ──────────────────────────────────────────────────

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
        if matches!(c, '.' | '!' | '?') {
            if !current.trim().is_empty() { parts.push(current.trim().to_string()); }
            current.clear();
        }
    }
    if !current.trim().is_empty() { parts.push(current.trim().to_string()); }

    let mut dedup   = Vec::new();
    let mut seen    = HashSet::new();
    for part in parts {
        let norm: String = part.to_lowercase().chars()
            .filter(|c| c.is_alphanumeric()).collect();
        if !norm.is_empty() && seen.insert(norm) { dedup.push(part); }
    }
    dedup.join(" ")
}