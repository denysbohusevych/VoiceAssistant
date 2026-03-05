use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{unbounded, Receiver};
use super::{AudioRecorder, AudioStream};

pub struct CpalStreamWrapper(cpal::Stream);
impl AudioStream for CpalStreamWrapper {}

pub struct CpalRecorder;

impl CpalRecorder {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}

impl AudioRecorder for CpalRecorder {
    fn start_recording(&self) -> Result<(Box<dyn AudioStream>, Receiver<f32>), String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("Не удалось найти микрофон по умолчанию")?;

        // 1. Получаем РОДНУЮ конфигурацию устройства (например: 48000Hz, Stereo).
        // Это гарантирует, что драйвер ОС не выдаст ошибку "not supported".
        let custom_config = device
            .default_input_config()
            .map_err(|e| format!("Ошибка конфигурации микрофона: {}", e))?;

        let sample_format = custom_config.sample_format();
        let config: cpal::StreamConfig = custom_config.into();

        let channels = config.channels;
        let input_sample_rate = config.sample_rate.0;
        let target_sample_rate = 16000; // То, что требует Whisper

        eprintln!(
            "  [cpal] Подключение к микрофону: {} Гц, каналов: {} -> Конвертация в {} Гц",
            input_sample_rate, channels, target_sample_rate
        );

        let (tx, rx) = unbounded();
        let err_fn = |err| eprintln!("[cpal] Ошибка аудио потока: {}", err);

        // 2. В зависимости от сырого формата данных микрофона создаем поток.
        // Внутри callback-а мы пропускаем данные через наш Resampler.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut resampler = Resampler::new(channels, input_sample_rate, target_sample_rate);
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &_| resampler.process(data, &tx),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let mut resampler = Resampler::new(channels, input_sample_rate, target_sample_rate);
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &_| {
                        // Нормализация i16 -> f32
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        resampler.process(&f32_data, &tx);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut resampler = Resampler::new(channels, input_sample_rate, target_sample_rate);
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &_| {
                        // Нормализация u16 -> f32
                        let f32_data: Vec<f32> = data.iter().map(|&s| (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0)).collect();
                        resampler.process(&f32_data, &tx);
                    },
                    err_fn,
                    None,
                )
            }
            _ => return Err("Неподдерживаемый формат аудио сэмплов микрофона".to_string()),
        }.map_err(|e| format!("Ошибка создания аудио потока: {}", e))?;

        // Запускаем физический сбор звука
        stream.play().map_err(|e| format!("Ошибка запуска потока: {}", e))?;

        Ok((Box::new(CpalStreamWrapper(stream)), rx))
    }
}

/// Простой линейный ресемплер "на лету"
/// Преобразует любую частоту и количество каналов в 16000Hz Mono.
struct Resampler {
    channels: u16,
    input_sr: u32,
    target_sr: u32,
    input_pos: f32,
    prev_mono: f32,
}

impl Resampler {
    fn new(channels: u16, input_sr: u32, target_sr: u32) -> Self {
        Self {
            channels,
            input_sr,
            target_sr,
            input_pos: 0.0,
            prev_mono: 0.0,
        }
    }

    fn process(&mut self, data: &[f32], tx: &crossbeam_channel::Sender<f32>) {
        let ratio = self.input_sr as f32 / self.target_sr as f32;

        for frame in data.chunks(self.channels as usize) {
            // 1. Downmix (усреднение всех каналов в один моно-канал)
            let mut mono = 0.0;
            for &s in frame {
                mono += s;
            }
            mono /= self.channels as f32;

            // 2. Линейная интерполяция для изменения частоты дискретизации
            while self.input_pos <= 1.0 {
                let sample = self.prev_mono + (mono - self.prev_mono) * self.input_pos;
                let _ = tx.send(sample);
                self.input_pos += ratio;
            }

            self.input_pos -= 1.0;
            self.prev_mono = mono;
        }
    }
}