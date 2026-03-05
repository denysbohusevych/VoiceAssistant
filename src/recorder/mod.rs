pub mod cpal_recorder;

use std::path::PathBuf;

/// Конфигурация записи.
/// Whisper требует: 16kHz, mono, 16-bit PCM (WAV).
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    /// Частота дискретизации (Whisper ожидает 16000 Hz)
    pub sample_rate: u32,
    /// Количество каналов (Whisper ожидает 1 = mono)
    pub channels: u16,
    /// Путь для сохранения файла
    pub output_path: PathBuf,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            channels: 1,
            output_path: PathBuf::from("recording.wav"),
        }
    }
}

impl RecordingConfig {
    pub fn new(output_path: impl Into<PathBuf>) -> Self {
        Self {
            output_path: output_path.into(),
            ..Default::default()
        }
    }
}

/// Информация об устройстве записи.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// Ошибки при работе с аудио.
#[derive(Debug)]
pub enum RecorderError {
    DeviceNotFound,
    InitError(String),
    StreamError(String),
    FileError(String),
    AlreadyRecording,
    NotRecording,
}

impl std::fmt::Display for RecorderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceNotFound   => write!(f, "Устройство записи не найдено"),
            Self::InitError(s)     => write!(f, "Ошибка инициализации аудио: {s}"),
            Self::StreamError(s)   => write!(f, "Ошибка потока записи: {s}"),
            Self::FileError(s)     => write!(f, "Ошибка записи файла: {s}"),
            Self::AlreadyRecording => write!(f, "Запись уже запущена"),
            Self::NotRecording     => write!(f, "Запись не запущена"),
        }
    }
}

impl std::error::Error for RecorderError {}

/// Общий интерфейс для аудио-рекордеров.
/// Позволяет добавлять новые бэкенды (например, AVFoundation, WASAPI, ALSA)
/// без изменения остального кода.
pub trait AudioRecorder: Send {
    /// Список доступных устройств записи.
    fn list_devices(&self) -> Result<Vec<DeviceInfo>, RecorderError>;

    /// Начать запись. Аудио будет сохранено в `config.output_path`.
    fn start(&mut self, config: RecordingConfig) -> Result<(), RecorderError>;

    /// Остановить запись и сохранить файл.
    fn stop(&mut self) -> Result<PathBuf, RecorderError>;

    /// Запись активна?
    fn is_recording(&self) -> bool;
}