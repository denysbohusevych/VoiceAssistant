use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{unbounded, Receiver};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};
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

        let custom_config = device
            .default_input_config()
            .map_err(|e| format!("Ошибка конфигурации микрофона: {}", e))?;

        let sample_format = custom_config.sample_format();
        let config: cpal::StreamConfig = custom_config.into();

        let channels = config.channels;
        let input_sample_rate = config.sample_rate.0;
        let target_sample_rate = 16000;

        eprintln!(
            "  [cpal] Подключение к микрофону: {} Гц, каналов: {} -> Конвертация в {} Гц (Rubato Sinc)",
            input_sample_rate, channels, target_sample_rate
        );

        let (tx, rx) = unbounded();
        let err_fn = |err| eprintln!("[cpal] Ошибка аудио потока: {}", err);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let mut resampler = RubatoResampler::new(channels as usize, input_sample_rate, target_sample_rate)?;
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &_| resampler.process_f32(data, &tx),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let mut resampler = RubatoResampler::new(channels as usize, input_sample_rate, target_sample_rate)?;
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &_| resampler.process_i16(data, &tx),
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut resampler = RubatoResampler::new(channels as usize, input_sample_rate, target_sample_rate)?;
                let tx = tx.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _: &_| resampler.process_u16(data, &tx),
                    err_fn,
                    None,
                )
            }
            _ => return Err("Неподдерживаемый формат аудио сэмплов микрофона".to_string()),
        }.map_err(|e| format!("Ошибка создания аудио потока: {}", e))?;

        stream.play().map_err(|e| format!("Ошибка запуска потока: {}", e))?;

        Ok((Box::new(CpalStreamWrapper(stream)), rx))
    }
}

/// Продвинутый Sinc-ресемплер через крейт `rubato`
struct RubatoResampler {
    resampler: FastFixedIn<f32>,
    input_buffer: Vec<Vec<f32>>,
    frames_in_buffer: usize,
    chunk_size: usize,
    channels: usize,
}

impl RubatoResampler {
    fn new(channels: usize, input_sr: u32, target_sr: u32) -> Result<Self, String> {
        let chunk_size = 1024;
        let resampler = FastFixedIn::<f32>::new(
            target_sr as f64 / input_sr as f64,
            1.0,
            PolynomialDegree::Cubic,
            chunk_size,
            1, // Сразу делаем Mono
        ).map_err(|e| format!("Ошибка инициализации rubato: {}", e))?;

        Ok(Self {
            resampler,
            input_buffer: vec![vec![0.0f32; chunk_size]],
            frames_in_buffer: 0,
            chunk_size,
            channels,
        })
    }

    fn process_mono_sample(&mut self, mono: f32, tx: &crossbeam_channel::Sender<f32>) {
        self.input_buffer[0][self.frames_in_buffer] = mono;
        self.frames_in_buffer += 1;

        if self.frames_in_buffer == self.chunk_size {
            if let Ok(out) = self.resampler.process(&self.input_buffer, None) {
                for &s in &out[0] {
                    let _ = tx.send(s);
                }
            }
            self.frames_in_buffer = 0;
        }
    }

    fn process_f32(&mut self, data: &[f32], tx: &crossbeam_channel::Sender<f32>) {
        for frame in data.chunks(self.channels) {
            let mut mono = 0.0;
            for &s in frame { mono += s; }
            mono /= self.channels as f32;
            self.process_mono_sample(mono, tx);
        }
    }

    fn process_i16(&mut self, data: &[i16], tx: &crossbeam_channel::Sender<f32>) {
        for frame in data.chunks(self.channels) {
            let mut mono = 0.0;
            for &s in frame { mono += s as f32 / i16::MAX as f32; }
            mono /= self.channels as f32;
            self.process_mono_sample(mono, tx);
        }
    }

    fn process_u16(&mut self, data: &[u16], tx: &crossbeam_channel::Sender<f32>) {
        for frame in data.chunks(self.channels) {
            let mut mono = 0.0;
            for &s in frame {
                mono += (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0);
            }
            mono /= self.channels as f32;
            self.process_mono_sample(mono, tx);
        }
    }
}