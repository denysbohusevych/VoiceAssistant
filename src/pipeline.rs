use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{bounded, Receiver};

use crate::recorder::cpal_recorder::{resample_linear, CpalRecorder};
use crate::recorder::{AudioRecorder, RecorderError, RecordingConfig};
use crate::transcriber::{TranscribeConfig, Transcriber};

#[derive(Clone)]
pub struct PipelineConfig {
    pub min_chunk_secs:   f32,
    pub max_chunk_secs:   f32,
    pub vad_window_secs:  f32,
    pub vad_silence_rms:  f32,
    pub vad_silence_secs: f32,
    pub overlap_secs:     f32,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            min_chunk_secs:   1.0,
            max_chunk_secs:   15.0,
            vad_window_secs:  0.3,
            vad_silence_rms:  0.01,
            vad_silence_secs: 0.6,
            overlap_secs:     0.2,
        }
    }
}

pub enum PipelineEvent {
    Text(String),
    TranscribeError(String),
    Stopped,
}

/// Принимает `Arc<Mutex<dyn Transcriber>>` — модель загружается один раз,
/// переиспользуется для всех нажатий PTT.
pub fn start_pipeline(
    mut recorder:    CpalRecorder,
    transcriber:     Arc<Mutex<dyn Transcriber>>,
    transcribe_cfg:  TranscribeConfig,
    pipeline_cfg:    PipelineConfig,
    stop_flag:       Arc<AtomicBool>,
) -> Result<Receiver<PipelineEvent>, RecorderError> {
    const TARGET_RATE: u32 = 16_000;

    let (chunk_tx, chunk_rx) = bounded::<Vec<f32>>(4);
    let (event_tx, event_rx) = bounded::<PipelineEvent>(32);

    recorder.start(RecordingConfig::new("_streaming.wav"))?;

    let device_rate = recorder.actual_sample_rate()
        .ok_or_else(|| RecorderError::InitError("Нет sample rate".into()))?;

    let raw_rx = recorder.take_sample_stream()
        .ok_or_else(|| RecorderError::InitError("Нет канала сэмплов".into()))?;

    // ── Поток 1: накопитель + VAD ─────────────────────────────────────────────
    {
        let stop_flag = Arc::clone(&stop_flag);
        let event_tx  = event_tx.clone();
        let cfg       = pipeline_cfg.clone();

        std::thread::spawn(move || {
            let min_samples     = (cfg.min_chunk_secs   * TARGET_RATE as f32) as usize;
            let max_samples     = (cfg.max_chunk_secs   * TARGET_RATE as f32) as usize;
            let vad_window      = (cfg.vad_window_secs  * TARGET_RATE as f32) as usize;
            let silence_samples = (cfg.vad_silence_secs * TARGET_RATE as f32) as usize;
            let overlap_samples = (cfg.overlap_secs     * TARGET_RATE as f32) as usize;

            let mut buf:          Vec<f32> = Vec::with_capacity(max_samples * 2);
            let mut overlap:      Vec<f32> = Vec::new();
            let mut silent_count: usize    = 0;

            while stop_flag.load(Ordering::Relaxed) {
                let raw = match raw_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(r)  => r,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };

                let resampled = resample_linear(&raw, device_rate, TARGET_RATE);
                buf.extend_from_slice(&resampled);

                // VAD
                if buf.len() >= vad_window {
                    let r = rms(&buf[buf.len() - vad_window..]);
                    if r < cfg.vad_silence_rms {
                        silent_count += resampled.len();
                    } else {
                        silent_count = 0;
                    }
                }

                let flush = (buf.len() >= min_samples && silent_count >= silence_samples)
                    || buf.len() >= max_samples;

                if flush {
                    let mut chunk = overlap.clone();
                    chunk.extend_from_slice(&buf);

                    let keep = overlap_samples.min(buf.len());
                    overlap = buf[buf.len() - keep..].to_vec();
                    buf.clear();
                    silent_count = 0;

                    if rms(&chunk) > cfg.vad_silence_rms * 0.5 {
                        if chunk_tx.send(chunk).is_err() { break; }
                    }
                }
            }

            // Флашим остаток
            if !buf.is_empty() && rms(&buf) > pipeline_cfg.vad_silence_rms * 0.5 {
                let mut chunk = overlap;
                chunk.extend_from_slice(&buf);
                let _ = chunk_tx.send(chunk);
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = event_tx.send(PipelineEvent::Stopped);
        });
    }

    // ── Поток 2: Whisper ──────────────────────────────────────────────────────
    // transcriber живёт снаружи, передаём Arc — не перезагружаем модель
    {
        std::thread::spawn(move || {
            while let Ok(chunk) = chunk_rx.recv() {
                let result = transcriber
                    .lock()
                    .unwrap()
                    .transcribe(&chunk, &transcribe_cfg);

                match result {
                    Ok(segs) => {
                        let text = segs.iter()
                            .map(|s| s.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim()
                            .to_string();
                        if !text.is_empty() {
                            let _ = event_tx.send(PipelineEvent::Text(text));
                        }
                    }
                    Err(e) => {
                        let _ = event_tx.send(PipelineEvent::TranscribeError(e.to_string()));
                    }
                }
            }
        });
    }

    // Держим recorder живым пока не поднят stop_flag
    std::thread::spawn(move || {
        while stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        drop(recorder);
    });

    Ok(event_rx)
}

#[inline]
fn rms(s: &[f32]) -> f32 {
    if s.is_empty() { return 0.0; }
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}