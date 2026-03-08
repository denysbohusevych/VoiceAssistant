mod config;
mod events;
mod coordinator;
pub mod ai;
pub mod context;
pub mod hotkey;
pub mod injector;
pub mod pipeline;
pub mod recorder;
pub mod transcriber;
pub mod vision;

use std::error::Error;
use coordinator::Coordinator;
use context::macos::MacOsContextCapture;
use recorder::cpal_recorder::CpalRecorder;
use transcriber::whisper::WhisperTranscriber;

fn main() -> Result<(), Box<dyn Error>> {
    // ─── Загрузка конфига ─────────────────────────────────────────────────────
    let cfg = config::load_shared(config::CONFIG_PATH);
    config::spawn_hot_reload_watcher(cfg.clone(), config::CONFIG_PATH.into(), 3);

    println!("🎙️  VoiceAssistant AI запущен!");

    // ─── AI клиент ───────────────────────────────────────────────────────────
    let ai_client = {
        let c = cfg.read().unwrap();
        ai::build_ai_client_from_config(&c, cfg.clone())
    };

    println!("🤖 AI: {} ({})\n", ai_client.provider_name(), ai_client.model_name());

    let system_prompt = {
        let c = cfg.read().unwrap();
        config::load_system_prompt(&c)
    };

    let mut coordinator = Coordinator::new(ai_client, system_prompt);
    let event_tx = coordinator.get_sender();

    // ─── Компоненты ──────────────────────────────────────────────────────────
    let recorder    = Box::new(CpalRecorder::new()?);
    let transcriber = {
        let model_path = cfg.read().unwrap().whisper.model_path.clone();
        Box::new(WhisperTranscriber::new(&model_path, cfg.clone())?)
    };
    let context_capture = Box::new(MacOsContextCapture::new());

    let (audio_action_tx, audio_action_rx)     = crossbeam_channel::unbounded();
    let (context_action_tx, context_action_rx) = crossbeam_channel::unbounded();

    pipeline::spawn_audio_worker(audio_action_rx, event_tx.clone(), recorder, transcriber, cfg.clone());
    context::spawn_worker(context_action_rx, event_tx.clone(), context_capture, cfg.clone());

    // ─── Хоткей ──────────────────────────────────────────────────────────────
    let hotkey_cfg = {
        let c = cfg.read().unwrap();
        hotkey::HotkeyConfig {
            push_to_talk: hotkey::parse_hotkey(&c.hotkey.push_to_talk),
        }
    };

    println!(
        "⌨️  Push-to-Talk: {} (удерживай для записи, отпусти для отправки)\n",
        hotkey_cfg.push_to_talk
    );

    let (hotkey_action_tx, hotkey_action_rx) = crossbeam_channel::unbounded::<events::PipelineAction>();
    let _hotkey_handle = hotkey::macos::spawn(hotkey_cfg, hotkey_action_tx)?;

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

    coordinator.run();

    Ok(())
}