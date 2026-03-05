pub mod whisper;

pub trait Transcriber: Send + Sync {
    /// Обрабатывает кусок аудио и возвращает распознанный текст
    fn transcribe(&mut self, audio_data: &[f32]) -> Result<String, String>;
}