// src/main.rs
//
// ╔══════════════════════════════════════════════════════════════════════════╗
// ║  Переключение AI-модели — ОДИН вызов в секции "AI конфигурация" ниже   ║
// ╚══════════════════════════════════════════════════════════════════════════╝

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
use hotkey::HotkeyConfig;
use context::macos::MacOsContextCapture;
use recorder::cpal_recorder::CpalRecorder;
use transcriber::whisper::WhisperTranscriber;
use ai::AiConfig;

// ─── Whisper ──────────────────────────────────────────────────────────────────

const WHISPER_MODEL_PATH: &str = "models/ggml-large-v3-turbo.bin";

// ─── AI конфигурация ──────────────────────────────────────────────────────────
//
// Раскомментируй нужную строку — остальные закомментируй.
//
// Ключи API берутся из переменных окружения (безопасно, не попадают в git).
// Установка ключей:
//   export GEMINI_API_KEY="AIza..."          # https://aistudio.google.com/apikey
//   export OPENAI_API_KEY="sk-..."           # https://platform.openai.com/api-keys
//   export ANTHROPIC_API_KEY="sk-ant-..."    # https://console.anthropic.com/settings/keys
//   export DEEPSEEK_API_KEY="sk-..."         # https://platform.deepseek.com/api_keys
//   # Для Ollama ключ не нужен, только `ollama serve`
//
// Или добавь в ~/.zshrc / ~/.bashrc чтобы не вводить каждый раз.

fn build_ai_client() -> Box<dyn ai::AiClient> {
    // ✅ АКТИВНО: Gemini 2.0 Flash (быстрый, дешёвый, хорошо работает с кодом)
    //AiConfig::gemini(env_key("GEMINI_API_KEY")).build()

    // Gemini 2.0 Pro (умнее, но медленнее)
    // AiConfig::gemini(env_key("GEMINI_API_KEY")).model("gemini-2.0-pro").build()

    // OpenAI GPT-4o
    // AiConfig::openai(env_key("OPENAI_API_KEY")).build()

    // OpenAI GPT-4o-mini (дешевле)
    // AiConfig::openai(env_key("OPENAI_API_KEY")).model("gpt-4o-mini").build()

    // Claude Sonnet (хорош для кода и длинных контекстов)
    // AiConfig::claude(env_key("ANTHROPIC_API_KEY")).build()

    // Claude Opus (самый умный, но медленный)
    // AiConfig::claude(env_key("ANTHROPIC_API_KEY")).model("claude-opus-4-6").build()

    // DeepSeek Chat (V3, очень дёшево)
    // AiConfig::deepseek(env_key("DEEPSEEK_API_KEY")).build()

    // DeepSeek Reasoner (R1, медленно, но глубокие рассуждения)
    // AiConfig::deepseek(env_key("DEEPSEEK_API_KEY")).model("deepseek-reasoner").build()

    // Ollama — локально, приватно, без ключей
    // AiConfig::ollama().model("llama3").build()
     AiConfig::ollama().model("qwen3.5:0.8b").build()
    // AiConfig::ollama().model("mistral").build()
}

// ─── Кастомный системный промпт ───────────────────────────────────────────────
//
// Замени на свой или оставь None для использования дефолтного.
// Дефолтный промпт: src/ai/mod.rs → DEFAULT_SYSTEM_PROMPT

fn custom_system_prompt() -> Option<&'static str> {
    None  // Использовать дефолтный

    // Пример кастомного промпта:
    // Some("Ты ассистент разработчика. Отвечай кратко и только по делу. \
    //       Для кода используй блоки ```rust или ```python.")
}

// ─── Точка входа ──────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn Error>> {
    println!("🎙️ VoiceAssistant AI запущен!");
    println!("Нажмите и удерживайте правый Alt (Option) для записи.");
    println!("Отпустите клавишу, чтобы отправить запрос.\n");

    let ai_client = build_ai_client();
    println!("🤖 AI: {} ({})\n", ai_client.provider_name(), ai_client.model_name());

    let system_prompt = custom_system_prompt()
        .unwrap_or(ai::DEFAULT_SYSTEM_PROMPT)
        .to_string();

    let mut coordinator = Coordinator::new(ai_client, system_prompt);
    let event_tx = coordinator.get_sender();

    let recorder    = Box::new(CpalRecorder::new()?);
    let transcriber = Box::new(WhisperTranscriber::new(WHISPER_MODEL_PATH)?);
    let context_capture = Box::new(MacOsContextCapture::new());

    let (audio_action_tx, audio_action_rx)     = crossbeam_channel::unbounded();
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

    coordinator.run();

    Ok(())
}

// ─── Утилиты ──────────────────────────────────────────────────────────────────

/// Читает ключ API из переменной окружения.
/// Выдаёт понятное сообщение если переменная не установлена.
fn env_key(var: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| {
        eprintln!(
            "⚠️  Переменная окружения {} не установлена.\n   \
             Установи: export {}=\"твой-ключ\"\n   \
             Или добавь в ~/.zshrc",
            var, var
        );
        String::new()
    })
}