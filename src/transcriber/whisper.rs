use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use super::Transcriber;
use std::collections::HashSet;

// C-совместимая функция для перехвата логов whisper.cpp
unsafe extern "C" fn silent_log_callback(
    _level: u32,
    _text: *const i8,
    _user_data: *mut std::ffi::c_void,
) {
    // Ничего не делаем, логи летят в пустоту
}

// Вспомогательная функция для очистки от мусора датасета и любых заиканий
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
        .replace("А.Синецкая", "")
        .replace("А.Егорова", "")
        .replace("Н. Новикова", "")
        .replace("Н. Закомолдина", "");

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
    // Убрали state отсюда. Храним только контекст модели (саму нейросеть).
    context: WhisperContext,
}

impl WhisperTranscriber {
    pub fn new(model_path: &str) -> Result<Self, String> {
        std::env::set_var("GGML_METAL_NDEBUG", "1");

        unsafe {
            whisper_rs::set_log_callback(Some(silent_log_callback), std::ptr::null_mut());
        }

        let params = WhisperContextParameters::default();
        let context = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| format!("Ошибка загрузки модели Whisper: {}", e))?;

        Ok(Self { context })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, audio_data: &[f32]) -> Result<String, String> {
        if audio_data.is_empty() {
            return Ok(String::new());
        }

        // 🛡️ АППАРАТНЫЙ ФИЛЬТР ТИШИНЫ (RMS Amplitude)
        // Вычисляем среднюю громкость куска аудио.
        // Если это просто белый шум или полная тишина - вообще не будим нейросеть.
        let mut sum_squares = 0.0;
        for &sample in audio_data {
            sum_squares += sample * sample;
        }
        let rms = (sum_squares / audio_data.len() as f32).sqrt();

        // Порог громкости (0.001 обычно отсекает пустой шум микрофона).
        // Если текст всё равно не распознается, когда ты говоришь тихо - уменьши до 0.0005.
        if rms < 0.001 {
            return Ok(String::new());
        }

        // 🧠 СВЕЖИЙ STATE НА КАЖДЫЙ ЗАПУСК
        // Решает проблему "протекающего" контекста и диких галлюцинаций
        let mut state = self.context.create_state()
            .map_err(|e| format!("Ошибка создания state: {}", e))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(None);

        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);

        // Настройки строгости
        params.set_no_context(true);
        params.set_no_speech_thold(0.4);
        params.set_entropy_thold(2.4);

        state.full(params, audio_data)
            .map_err(|e| format!("Ошибка инференса: {}", e))?;

        let num_segments = state.full_n_segments()
            .map_err(|e| format!("Ошибка получения сегментов: {}", e))?;

        let mut raw_result = String::new();

        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                raw_result.push_str(&segment);
                raw_result.push(' ');
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

        let normalized_final = normalized_final.trim();

        if hallucinations.iter().any(|&h| normalized_final == h || normalized_final.starts_with("hello hello")) {
            return Ok(String::new());
        }

        Ok(final_text)
    }
}