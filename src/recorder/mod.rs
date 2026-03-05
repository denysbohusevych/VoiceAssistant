pub mod cpal_recorder;

use crossbeam_channel::Receiver;

/// Пустой трейт-маркер для удержания аудио-потока.
/// Мы намеренно убрали требование `: Send`, так как потоки `cpal`
/// не являются потокобезопасными из-за использования сырых указателей (*mut ()).
/// В нашем пайплайне этот стрим безопасно живет внутри одного запущенного worker-потока.
pub trait AudioStream {}

pub trait AudioRecorder: Send + Sync {
    /// Запускает запись. Возвращает объект стрима (чтобы держать микрофон открытым)
    /// и канал, из которого будут сыпаться сэмплы (f32).
    fn start_recording(&self) -> Result<(Box<dyn AudioStream>, Receiver<f32>), String>;
}