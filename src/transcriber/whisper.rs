use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};
use super::Transcriber;

// C-совместимая функция для перехвата логов whisper.cpp
unsafe extern "C" fn silent_log_callback(
    _level: u32,
    _text: *const i8,
    _user_data: *mut std::ffi::c_void,
) {
    // Ничего не делаем, логи летят в пустоту
}

pub struct WhisperTranscriber {
    // ВАЖНО: Поле state должно быть объявлено ДО context!
    // В Rust поля структуры удаляются в порядке их объявления (сверху вниз).
    // Это гарантирует, что мы освободим C++ память state строго до того,
    // как будет удален сам context, избегая ошибок use-after-free.
    state: Option<WhisperState<'static>>,
    context: WhisperContext,
}

impl WhisperTranscriber {
    pub fn new(model_path: &str) -> Result<Self, String> {
        // 1. Отключаем хардварные логи Apple Metal (ggml_metal_init)
        // Это заставит бэкенд Metal замолчать навсегда и не спамить аллокациями.
        std::env::set_var("GGML_METAL_NDEBUG", "1");

        // 2. Глушим общие текстовые логи whisper.cpp
        unsafe {
            whisper_rs::set_log_callback(Some(silent_log_callback), std::ptr::null_mut());
        }

        let params = WhisperContextParameters::default();
        let context = WhisperContext::new_with_params(model_path, params)
            .map_err(|e| format!("Ошибка загрузки модели Whisper (путь: {}): {}", model_path, e))?;

        Ok(Self {
            state: None,
            context,
        })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&mut self, audio_data: &[f32]) -> Result<String, String> {
        if audio_data.is_empty() {
            return Ok(String::new());
        }

        // Инициализируем (аллоцируем) state только ОДИН раз при первой записи
        if self.state.is_none() {
            let state = self.context.create_state()
                .map_err(|e| format!("Ошибка создания state: {}", e))?;

            // SAFETY: whisper-rs хранит реальные данные в куче (через C++ pointers).
            // Перемещение структуры WhisperTranscriber в другой поток не меняет
            // адреса context в памяти. Мы можем искусственно продлить время жизни
            // ссылки до 'static. Удаление безопасно из-за порядка полей в struct.
            let static_state: WhisperState<'static> = unsafe { std::mem::transmute(state) };
            self.state = Some(static_state);
            eprintln!("  [whisper] 🧠 Буферы нейросети выделены (состояние кэшировано)");
        }

        let state = self.state.as_mut().unwrap();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(None);

        // Жестко запрещаем внутренние принты
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);

        // 🚀 НОВЫЕ ПАРАМЕТРЫ: БОРЬБА С ГАЛЛЮЦИНАЦИЯМИ И СКОРОСТЬ

        // 1. Ускоряет обработку, объединяя всё в один сегмент (нам не нужны субтитры с таймкодами)
        params.set_single_segment(true);

        // 2. Порог тишины (0.6). Если вероятность наличия речи ниже, модель сразу прерывает работу.
        // Это мгновенно решает проблему долгих тормозов на пустом аудио.
        params.set_no_speech_thold(0.6);

        // 3. Порог энтропии (2.4). Как только модель начинает зацикливаться
        // ("Okay. Yep. Okay. Yep."), она останавливается.
        params.set_entropy_thold(2.4);

        // Функция whisper_full сама корректно очищает рабочую память
        // внутри state перед каждым новым запуском, поэтому переиспользование абсолютно безопасно.
        state.full(params, audio_data)
            .map_err(|e| format!("Ошибка инференса: {}", e))?;

        let num_segments = state.full_n_segments()
            .map_err(|e| format!("Ошибка получения сегментов: {}", e))?;

        let mut result = String::new();
        for i in 0..num_segments {
            // Вытаскиваем текст только тех сегментов, где модель уверена, что это речь
            if let Ok(segment) = state.full_get_segment_text(i) {
                // Дополнительная зачистка от мусорных тегов, которые иногда прорываются
                let clean = segment
                    .replace("[BLANK_AUDIO]", "")
                    .replace("[Silence]", "")
                    .replace("(silence)", "")
                    .replace("[Музыка]", "")
                    .replace("[Music]", "");

                result.push_str(&clean);
                result.push(' ');
            }
        }

        let final_text = result.trim().to_string();

        // Если после зачистки остался только мусор - возвращаем пустую строку
        let hallucinations = ["Okay.", "Yep.", "Sounds good.", "Cool.", "Damn."];
        if hallucinations.iter().any(|&h| final_text.trim() == h) {
            return Ok(String::new());
        }

        Ok(final_text)
    }
}