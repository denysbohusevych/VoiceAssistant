use std::path::{Path, PathBuf};

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{Segment, Transcriber, TranscriberError, TranscribeConfig};

/// Транскрибер на основе локального whisper.cpp.
/// Модель загружается один раз при создании и переиспользуется.
pub struct WhisperTranscriber {
    ctx: WhisperContext,
    model_path: PathBuf,
}

impl WhisperTranscriber {
    /// Создать транскрибер, загрузив GGML-модель из `model_path`.
    ///
    /// Модели можно скачать с:
    /// https://huggingface.co/ggerganov/whisper.cpp/tree/main
    ///
    /// Рекомендуемые варианты:
    ///   ggml-tiny.bin   (~75 MB)  — быстро, менее точно
    ///   ggml-base.bin   (~142 MB) — хороший баланс ← рекомендую начать с него
    ///   ggml-small.bin  (~466 MB) — точнее, медленнее
    ///   ggml-medium.bin (~1.5 GB) — очень точно
    ///   ggml-large.bin  (~3 GB)   — максимальная точность
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, TranscriberError> {
        let model_path = model_path.as_ref().to_path_buf();

        if !model_path.exists() {
            return Err(TranscriberError::ModelNotFound(
                model_path.display().to_string(),
            ));
        }

        let params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(
            model_path.to_str().unwrap_or_default(),
            params,
        )
            .map_err(|e| TranscriberError::InitError(e.to_string()))?;

        Ok(Self { ctx, model_path })
    }

    /// Путь к загруженной модели.
    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &mut self,
        samples: &[f32],
        config: &TranscribeConfig,
    ) -> Result<Vec<Segment>, TranscriberError> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscriberError::InitError(e.to_string()))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Язык
        match config.language.as_deref() {
            Some("auto") | None => params.set_language(None), // автоопределение
            Some(lang)          => params.set_language(Some(lang)),
        }

        // Режим перевода
        params.set_translate(config.translate);

        // Многопоточность
        params.set_n_threads(config.n_threads);

        // Убираем лишние логи whisper.cpp в stderr
        params.set_print_realtime(false);
        params.set_print_progress(false);
        params.set_print_timestamps(false);

        // Запуск инференса
        state
            .full(params, samples)
            .map_err(|e| TranscriberError::TranscribeError(e.to_string()))?;

        // Собираем сегменты
        let n = state
            .full_n_segments()
            .map_err(|e| TranscriberError::TranscribeError(e.to_string()))?;

        let mut segments = Vec::with_capacity(n as usize);
        for i in 0..n {
            let text = state
                .full_get_segment_text(i)
                .map_err(|e| TranscriberError::TranscribeError(e.to_string()))?
                .trim()
                .to_string();

            let start_ms = state
                .full_get_segment_t0(i)
                .map_err(|e| TranscriberError::TranscribeError(e.to_string()))?
                * 10; // whisper отдаёт в сантисекундах → мс

            let end_ms = state
                .full_get_segment_t1(i)
                .map_err(|e| TranscriberError::TranscribeError(e.to_string()))?
                * 10;

            if !text.is_empty() {
                segments.push(Segment { start_ms, end_ms, text });
            }
        }

        Ok(segments)
    }
}