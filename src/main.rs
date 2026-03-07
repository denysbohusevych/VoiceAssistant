mod events;
mod coordinator;
pub mod context;
pub mod hotkey;
pub mod injector;
pub mod pipeline;
pub mod recorder;
pub mod transcriber;
pub mod vision;

use std::error::Error;
use coordinator::Coordinator;
use hotkey::HotkeyConfig;
use context::macos::MacOsContextCapture;
use recorder::cpal_recorder::CpalRecorder;
use transcriber::whisper::WhisperTranscriber;

// Подключаем оптимизированную модель large-v3-turbo
const WHISPER_MODEL_PATH: &str = "models/ggml-large-v3-turbo.bin";

fn main() -> Result<(), Box<dyn Error>> {
    println!("🎙️ VoiceAssistant AI запущен!");
    println!("Нажмите и удерживайте правый Alt (Option) для записи.");
    println!("Отпустите клавишу, чтобы отправить запрос.\n");

    let mut coordinator = Coordinator::new();
    let event_tx = coordinator.get_sender();

    let recorder = Box::new(CpalRecorder::new()?);
    let transcriber = Box::new(WhisperTranscriber::new(WHISPER_MODEL_PATH)?);
    let context_capture = Box::new(MacOsContextCapture::new());

    let (audio_action_tx, audio_action_rx) = crossbeam_channel::unbounded();
    let (context_action_tx, context_action_rx) = crossbeam_channel::unbounded();

    pipeline::spawn_audio_worker(audio_action_rx, event_tx.clone(), recorder, transcriber);
    context::spawn_worker(context_action_rx, event_tx.clone(), context_capture);

    let config = HotkeyConfig::default();
    let (hotkey_action_tx, hotkey_action_rx) = crossbeam_channel::unbounded::<events::PipelineAction>();
    let _hotkey_handle = hotkey::macos::spawn(config, hotkey_action_tx)?;

    let coordinator_tx = coordinator.get_sender();
    std::thread::spawn(move || {
        while let Ok(action) = hotkey_action_rx.recv() {
            let _ = audio_action_tx.send(action.clone());
            let _ = context_action_tx.send(action.clone());

            if let events::PipelineAction::StartSession { target_pid } = action {
                println!("\n--- Хоткей нажат ---");
                let _ = coordinator_tx.send(events::WorkerEvent::SessionStarted(target_pid));
            }
        }
    });

    // Блокируем главный поток Оркестратором
    coordinator.run();

    Ok(())
}