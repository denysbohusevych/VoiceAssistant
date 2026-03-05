use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, Stream};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{AudioRecorder, DeviceInfo, RecorderError, RecordingConfig};

type AudioSample = f32;

/// cpal::Stream не реализует Send на macOS (CoreAudio) — обёртка безопасна,
/// т.к. мы управляем потоком только из одного потока.
struct StreamWrapper(Stream);
unsafe impl Send for StreamWrapper {}

/// Внутреннее состояние активной записи.
struct ActiveStream {
    _stream: StreamWrapper,
    /// Реальные параметры устройства (могут отличаться от запрошенных)
    actual_sample_rate: u32,
    actual_channels:    u16,
}

pub struct CpalRecorder {
    host:        Host,
    active:      Option<ActiveStream>,
    /// Канал с сырыми (ещё не ресэмплированными) сэмплами от устройства
    raw_tx:      Option<Sender<Vec<AudioSample>>>,
    /// Открытый конец канала — можно передать pipeline'у
    sample_rx:   Option<Receiver<Vec<AudioSample>>>,
    /// Для file-based режима: накопленный буфер
    samples:     Arc<Mutex<Vec<AudioSample>>>,
    config:      Option<RecordingConfig>,
}

impl CpalRecorder {
    pub fn new() -> Result<Self, RecorderError> {
        Ok(Self {
            host:      cpal::default_host(),
            active:    None,
            raw_tx:    None,
            sample_rx: None,
            samples:   Arc::new(Mutex::new(Vec::new())),
            config:    None,
        })
    }

    fn get_input_device(&self, name: Option<&str>) -> Result<Device, RecorderError> {
        match name {
            Some(n) => self
                .host
                .input_devices()
                .map_err(|e| RecorderError::InitError(e.to_string()))?
                .find(|d| d.name().map(|dn| dn == n).unwrap_or(false))
                .ok_or(RecorderError::DeviceNotFound),
            None => self
                .host
                .default_input_device()
                .ok_or(RecorderError::DeviceNotFound),
        }
    }

    /// Возвращает Receiver с сэмплами (16kHz, mono, f32) для стримингового режима.
    /// Вызывать после `start()`. Можно взять только один раз — владение передаётся.
    pub fn take_sample_stream(&mut self) -> Option<Receiver<Vec<AudioSample>>> {
        self.sample_rx.take()
    }

    /// Фактический sample rate устройства (после старта).
    pub fn actual_sample_rate(&self) -> Option<u32> {
        self.active.as_ref().map(|a| a.actual_sample_rate)
    }
}

impl AudioRecorder for CpalRecorder {
    fn list_devices(&self) -> Result<Vec<DeviceInfo>, RecorderError> {
        let default_name = self
            .host
            .default_input_device()
            .and_then(|d| d.name().ok());

        self.host
            .input_devices()
            .map_err(|e| RecorderError::InitError(e.to_string()))?
            .filter_map(|d| {
                d.name().ok().map(|name| {
                    let is_default = Some(&name) == default_name.as_ref();
                    DeviceInfo { name, is_default }
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .pipe_ok()
    }

    fn start(&mut self, config: RecordingConfig) -> Result<(), RecorderError> {
        if self.is_recording() {
            return Err(RecorderError::AlreadyRecording);
        }

        let device = self.get_input_device(None)?;

        // Ищем конфигурацию с нужной частотой (16kHz mono для Whisper)
        let stream_config = device
            .supported_input_configs()
            .map_err(|e| RecorderError::InitError(e.to_string()))?
            .filter(|c| {
                c.channels() == config.channels
                    && c.min_sample_rate().0 <= config.sample_rate
                    && c.max_sample_rate().0 >= config.sample_rate
            })
            .next()
            .map(|c| c.with_sample_rate(cpal::SampleRate(config.sample_rate)))
            .or_else(|| device.default_input_config().ok())
            .ok_or_else(|| RecorderError::InitError("Нет подходящей конфигурации".into()))?;

        let actual_rate     = stream_config.sample_rate().0;
        let actual_channels = stream_config.channels();
        let sample_format   = stream_config.sample_format();

        eprintln!("Устройство   : {}", device.name().unwrap_or_default());
        eprintln!("Sample rate  : {actual_rate} Hz  каналов: {actual_channels}  формат: {sample_format:?}");

        // Канал: cpal callback → pipeline
        let (tx, rx) = bounded::<Vec<AudioSample>>(512);

        let err_fn = |e| eprintln!("Ошибка потока: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => {
                let tx = tx.clone();
                device.build_input_stream(
                    &stream_config.into(),
                    move |data: &[f32], _| { let _ = tx.try_send(data.to_vec()); },
                    err_fn, None,
                )
            }
            SampleFormat::I16 => {
                let tx = tx.clone();
                device.build_input_stream(
                    &stream_config.into(),
                    move |data: &[i16], _| {
                        let v: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                        let _ = tx.try_send(v);
                    },
                    err_fn, None,
                )
            }
            SampleFormat::U16 => {
                let tx = tx.clone();
                device.build_input_stream(
                    &stream_config.into(),
                    move |data: &[u16], _| {
                        let v: Vec<f32> = data.iter()
                            .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let _ = tx.try_send(v);
                    },
                    err_fn, None,
                )
            }
            _ => return Err(RecorderError::InitError(
                format!("Неподдерживаемый формат: {sample_format:?}")
            )),
        }
            .map_err(|e| RecorderError::StreamError(e.to_string()))?;

        stream.play().map_err(|e| RecorderError::StreamError(e.to_string()))?;

        self.active    = Some(ActiveStream { _stream: StreamWrapper(stream), actual_sample_rate: actual_rate, actual_channels });
        self.sample_rx = Some(rx);
        self.raw_tx    = Some(tx);
        self.samples   = Arc::new(Mutex::new(Vec::new()));
        self.config    = Some(RecordingConfig {
            sample_rate: actual_rate,
            channels:    actual_channels,
            output_path: config.output_path,
        });

        Ok(())
    }

    fn stop(&mut self) -> Result<PathBuf, RecorderError> {
        if !self.is_recording() {
            return Err(RecorderError::NotRecording);
        }

        // Дроп стрима закрывает канал
        self.active    = None;
        self.raw_tx    = None;
        self.sample_rx = None;

        std::thread::sleep(std::time::Duration::from_millis(50));

        let config = self.config.take().unwrap();
        let samples = self.samples.lock()
            .map_err(|e| RecorderError::FileError(e.to_string()))?
            .clone();

        // Если pipeline брал канал — буфер пустой, файл не нужен
        if samples.is_empty() {
            return Ok(config.output_path);
        }

        save_wav(&samples, &config)
    }

    fn is_recording(&self) -> bool {
        self.active.is_some()
    }
}

// ─── Утилиты ──────────────────────────────────────────────────────────────────

fn save_wav(samples: &[f32], config: &RecordingConfig) -> Result<PathBuf, RecorderError> {
    const TARGET: u32 = 16_000;

    let mono: Vec<f32> = if config.channels > 1 {
        samples.chunks(config.channels as usize)
            .map(|f| f.iter().sum::<f32>() / f.len() as f32)
            .collect()
    } else {
        samples.to_vec()
    };

    let resampled = resample_linear(&mono, config.sample_rate, TARGET);

    let spec = hound::WavSpec {
        channels: 1, sample_rate: TARGET,
        bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&config.output_path, spec)
        .map_err(|e| RecorderError::FileError(e.to_string()))?;
    for s in &resampled {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| RecorderError::FileError(e.to_string()))?;
    }
    writer.finalize().map_err(|e| RecorderError::FileError(e.to_string()))?;

    Ok(config.output_path.clone())
}

pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to { return samples.to_vec(); }
    let ratio   = from as f64 / to as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    (0..out_len).map(|i| {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let s0 = samples.get(idx).copied().unwrap_or(0.0);
        let s1 = samples.get(idx + 1).copied().unwrap_or(0.0);
        s0 + (s1 - s0) * frac
    }).collect()
}

// Маленький хелпер чтобы не писать .collect::<Result<_,_>>() везде
trait PipeOk: Sized {
    fn pipe_ok(self) -> Result<Self, RecorderError> { Ok(self) }
}
impl<T> PipeOk for Vec<T> {}