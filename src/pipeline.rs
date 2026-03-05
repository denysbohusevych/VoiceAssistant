use std::time::{Duration, Instant};
use crossbeam_channel::{Receiver, Sender};

use crate::events::{PipelineAction, WorkerEvent};
use crate::recorder::AudioRecorder;
use crate::transcriber::Transcriber;

/// Запускает независимый поток для обработки звука
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
        let mut last_transcribe_time = Instant::now();

        loop {
            // 1. Проверяем команды от Оркестратора (хоткеи)
            while let Ok(action) = action_rx.try_recv() {
                match action {
                    PipelineAction::StartSession { .. } => {
                        is_recording = true;
                        audio_buffer.clear();
                        last_transcribe_time = Instant::now();

                        match recorder.start_recording() {
                            Ok((stream, rx)) => {
                                audio_stream = Some(stream);
                                audio_rx = Some(rx);
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
                        eprintln!("  [audio] ⏹️ Остановка записи, финальный инференс...");

                        // Дропаем стрим, чтобы освободить микрофон
                        audio_stream = None;

                        // Дочитываем остатки сэмплов из канала
                        if let Some(rx) = &audio_rx {
                            while let Ok(sample) = rx.try_recv() {
                                audio_buffer.push(sample);
                            }
                        }
                        audio_rx = None;

                        // Финальная транскрипция всего буфера
                        if !audio_buffer.is_empty() {
                            match transcriber.transcribe(&audio_buffer) {
                                Ok(text) if !text.is_empty() => {
                                    let _ = event_tx.send(WorkerEvent::FinalTranscription(text));
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    let _ = event_tx.send(WorkerEvent::AudioError(e));
                                }
                            }
                        }

                        // Сообщаем Оркестратору, что аудио-модуль полностью завершил работу с сессией
                        let _ = event_tx.send(WorkerEvent::AudioFinished);
                    }
                }
            }

            // 2. Если мы сейчас пишем звук, собираем его и периодически делаем частичный инференс
            if is_recording {
                if let Some(rx) = &audio_rx {
                    while let Ok(sample) = rx.try_recv() {
                        audio_buffer.push(sample);
                    }

                    // Каждые 1.5 секунды пробуем распознать промежуточный текст
                    // (чтобы пользователь видел, что система его слышит)
                    if last_transcribe_time.elapsed() >= Duration::from_millis(1500) && audio_buffer.len() > 16000 {
                        // В реальном приложении здесь лучше использовать VAD (Voice Activity Detection),
                        // но для прототипа сойдет инференс по таймеру
                        match transcriber.transcribe(&audio_buffer) {
                            Ok(text) if !text.is_empty() => {
                                let _ = event_tx.send(WorkerEvent::PartialTranscription(text));
                            }
                            _ => {}
                        }
                        last_transcribe_time = Instant::now();
                    }
                }
            }

            // Немного спим, чтобы не сжечь CPU пустим циклом (Polling)
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}