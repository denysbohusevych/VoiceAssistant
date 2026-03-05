use std::time::Duration;
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

        loop {
            // 1. Проверяем команды от Оркестратора (хоткеи)
            while let Ok(action) = action_rx.try_recv() {
                match action {
                    PipelineAction::StartSession { .. } => {
                        is_recording = true;
                        audio_buffer.clear();

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

                        // ФИНАЛЬНАЯ ТРАНСКРИПЦИЯ:
                        // Делается ровно один раз, на всём собранном буфере.
                        // Это исключает любые дубликаты и заикания!
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

            // 2. Если мы сейчас пишем звук, ПРОСТО СОБИРАЕМ ЕГО (без промежуточного инференса)
            if is_recording {
                if let Some(rx) = &audio_rx {
                    while let Ok(sample) = rx.try_recv() {
                        audio_buffer.push(sample);
                    }
                }
            }

            // Спим 50 мс, чтобы не грузить CPU бесконечным циклом
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}