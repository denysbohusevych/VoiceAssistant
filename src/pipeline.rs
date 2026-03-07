use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};

use crate::events::{PipelineAction, WorkerEvent};
use crate::recorder::AudioRecorder;
use crate::transcriber::Transcriber;

/// Запускает независимый потоковый (Streaming) воркер для обработки звука
pub fn spawn_audio_worker(
    action_rx: Receiver<PipelineAction>,
    event_tx: Sender<WorkerEvent>,
    recorder: Box<dyn AudioRecorder>,
    mut transcriber: Box<dyn Transcriber>,
) {
    std::thread::spawn(move || {
        let mut is_recording = false;
        let mut audio_stream = None;
        let mut audio_rx: Option<Receiver<f32>> = None;

        let mut audio_buffer = Vec::new();
        let mut last_process_time = Instant::now();
        let mut last_rms_idx = 0;
        let chunk_duration = Duration::from_millis(500);

        // Порог тишины (увеличили с 0.001 до 0.005, чтобы отсекать фоновый шум комнаты)
        let silence_threshold = 0.005;

        loop {
            // 1. Проверяем команды от Оркестратора (хоткеи)
            while let Ok(action) = action_rx.try_recv() {
                match action {
                    PipelineAction::StartSession { .. } => {
                        is_recording = true;
                        audio_buffer.clear();
                        last_rms_idx = 0;
                        last_process_time = Instant::now();

                        match recorder.start_recording() {
                            Ok((stream, rx)) => {
                                audio_stream = Some(stream);
                                audio_rx = Some(rx);
                                eprintln!("  [audio] 🎙️ Запись пошла (потоковый режим)...");
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

                        // Дочитываем остатки
                        if let Some(rx) = &audio_rx {
                            while let Ok(sample) = rx.try_recv() {
                                audio_buffer.push(sample);
                            }
                        }
                        audio_rx = None;

                        // Финализируем всё, что осталось в буфере (если это не тишина)
                        if !audio_buffer.is_empty() {
                            let mut sum_sq = 0.0;
                            for &s in &audio_buffer { sum_sq += s * s; }
                            let rms = (sum_sq / audio_buffer.len() as f32).sqrt();

                            // Защита от тишины при отпускании кнопки
                            if rms >= silence_threshold {
                                if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                                    if !text.is_empty() {
                                        let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                                    }
                                }
                            } else {
                                eprintln!("  [audio] 🔇 Пропуск транскрибации (чистая тишина, RMS: {:.4})", rms);
                            }
                        }

                        let _ = event_tx.send(WorkerEvent::AudioFinished);
                    }
                }
            }

            // 2. Real-time обработка "Растущим Окном" и VAD
            if is_recording {
                if let Some(rx) = &audio_rx {
                    while let Ok(sample) = rx.try_recv() {
                        audio_buffer.push(sample);
                    }
                }

                // Каждые ~500мс проверяем буфер
                if last_process_time.elapsed() >= chunk_duration && audio_buffer.len() > last_rms_idx {
                    let new_samples = &audio_buffer[last_rms_idx..];

                    let mut sum_sq = 0.0;
                    for &s in new_samples { sum_sq += s * s; }
                    let rms = (sum_sq / new_samples.len() as f32).sqrt();

                    last_rms_idx = audio_buffer.len();

                    if rms < silence_threshold {
                        // VAD: Тишина обнаружена -> Финализируем (Slice)
                        if !audio_buffer.is_empty() {
                            // Проверяем RMS всего накопленного буфера перед отправкой
                            let mut full_sum_sq = 0.0;
                            for &s in &audio_buffer { full_sum_sq += s * s; }
                            let full_rms = (full_sum_sq / audio_buffer.len() as f32).sqrt();

                            if full_rms >= silence_threshold {
                                if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                                    if !text.is_empty() {
                                        let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                                    }
                                }
                            }
                            audio_buffer.clear();
                            last_rms_idx = 0;
                        }
                    } else if audio_buffer.len() > 16000 * 12 {
                        // Sliding Window: принудительный рез каждые 12 секунд
                        if let Ok(text) = transcriber.transcribe(&audio_buffer) {
                            if !text.is_empty() {
                                let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                            }
                        }
                        // Оставляем последние 1.5 секунды как нахлест (Overlap)
                        let overlap_len = 24000.min(audio_buffer.len());
                        let overlap = audio_buffer[audio_buffer.len() - overlap_len..].to_vec();
                        audio_buffer = overlap;
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

            // Небольшой сон, чтобы не жарить CPU
            std::thread::sleep(Duration::from_millis(10));
        }
    });
}