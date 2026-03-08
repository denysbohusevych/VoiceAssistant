// src/pipeline.rs
use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};

use crate::config::SharedConfig;
use crate::events::{PipelineAction, WorkerEvent};
use crate::recorder::AudioRecorder;
use crate::transcriber::Transcriber;

pub fn spawn_audio_worker(
    action_rx:   Receiver<PipelineAction>,
    event_tx:    Sender<WorkerEvent>,
    recorder:    Box<dyn AudioRecorder>,
    mut transcriber: Box<dyn Transcriber>,
    cfg:         SharedConfig,
) {
    std::thread::spawn(move || {
        let mut is_recording     = false;
        let mut audio_stream     = None;
        let mut audio_rx: Option<Receiver<f32>> = None;

        let mut audio_buffer      = Vec::new();
        let mut last_process_time = Instant::now();
        let mut last_rms_idx      = 0usize;

        loop {
            // ─── Читаем актуальные пороги из конфига ──────────────────────────
            let (silence_threshold, chunk_duration, sliding_window_samples, overlap_samples) = {
                let c = cfg.read().unwrap();
                let sr = 16000u64;
                (
                    c.audio.silence_threshold,
                    Duration::from_millis(c.audio.chunk_duration_ms),
                    (c.audio.sliding_window_max_seconds * sr as f32) as usize,
                    (c.audio.overlap_seconds * sr as f32) as usize,
                )
            };

            // ─── Команды от оркестратора ──────────────────────────────────────
            while let Ok(action) = action_rx.try_recv() {
                match action {
                    PipelineAction::StartSession { .. } => {
                        is_recording = true;
                        audio_buffer.clear();
                        last_rms_idx      = 0;
                        last_process_time = Instant::now();

                        match recorder.start_recording() {
                            Ok((stream, rx)) => {
                                audio_stream = Some(stream);
                                audio_rx     = Some(rx);
                                eprintln!("  [audio] 🎙️ Запись пошла...");
                            }
                            Err(e) => {
                                let _ = event_tx.send(WorkerEvent::AudioError(e));
                                is_recording = false;
                            }
                        }
                    }
                    PipelineAction::StopSession => {
                        is_recording = false;
                        eprintln!("  [audio] ⏹️ Остановка записи...");
                        audio_stream = None;

                        if let Some(rx) = &audio_rx {
                            while let Ok(s) = rx.try_recv() { audio_buffer.push(s); }
                        }
                        audio_rx = None;

                        if !audio_buffer.is_empty() {
                            let rms = compute_rms(&audio_buffer);
                            if rms >= silence_threshold {
                                if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                                    if !text.is_empty() {
                                        let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                                    }
                                }
                            } else {
                                eprintln!("  [audio] 🔇 Тишина при стопе (RMS={:.4})", rms);
                            }
                        }
                        let _ = event_tx.send(WorkerEvent::AudioFinished);
                    }
                }
            }

            // ─── Real-time обработка ──────────────────────────────────────────
            if is_recording {
                if let Some(rx) = &audio_rx {
                    while let Ok(s) = rx.try_recv() { audio_buffer.push(s); }
                }

                if last_process_time.elapsed() >= chunk_duration
                    && audio_buffer.len() > last_rms_idx
                {
                    let new_rms = compute_rms(&audio_buffer[last_rms_idx..]);
                    last_rms_idx = audio_buffer.len();

                    if new_rms < silence_threshold {
                        // VAD: тишина → финализируем
                        if !audio_buffer.is_empty() && compute_rms(&audio_buffer) >= silence_threshold {
                            if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                                if !text.is_empty() {
                                    let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                                }
                            }
                        }
                        audio_buffer.clear();
                        last_rms_idx = 0;

                    } else if audio_buffer.len() > sliding_window_samples {
                        // Sliding Window: принудительная нарезка
                        if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                            if !text.is_empty() {
                                let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                            }
                        }
                        let overlap = overlap_samples.min(audio_buffer.len());
                        let tail = audio_buffer[audio_buffer.len() - overlap..].to_vec();
                        audio_buffer = tail;
                        last_rms_idx = audio_buffer.len();

                    } else {
                        // Growing Window: частичное обновление
                        if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                            if !text.is_empty() {
                                let _ = event_tx.send(WorkerEvent::PartialTranscription(text));
                            }
                        }
                    }
                    last_process_time = Instant::now();
                }
            }

            std::thread::sleep(Duration::from_millis(10));
        }
    });
}

// ─── Утилиты ──────────────────────────────────────────────────────────────────

fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() { return 0.0; }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}