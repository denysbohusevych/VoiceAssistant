mod context;
mod hotkey;
mod injector;
mod pipeline;
mod recorder;
mod transcriber;
mod vision;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use recorder::cpal_recorder::CpalRecorder;
use transcriber::whisper::WhisperTranscriber;
use transcriber::TranscribeConfig;
use hotkey::{HotkeyConfig, HotkeyEvent};
use pipeline::{PipelineConfig, PipelineEvent, start_pipeline};
use context::ContextCapture;
use injector::TextInjector;

#[cfg(target_os = "macos")]
use context::macos::MacOsContextCapture;
#[cfg(target_os = "macos")]
use injector::macos::MacOsTextInjector;

const MODEL_PATH: &str = "models/ggml-small.bin";
const LANGUAGE: Option<&str> = Some("ru");

fn main() {
    println!("╔══════════════════════════════════════════╗");
    println!("║  🎙  Push-to-Talk → Whisper → Вставка    ║");
    println!("╚══════════════════════════════════════════╝");

    let hotkey_cfg = HotkeyConfig::default();
    println!("  Модель : {MODEL_PATH}");
    println!("  Язык   : {}", LANGUAGE.unwrap_or("авто"));
    println!("  Хоткей : {} (удерживай для записи)\n", hotkey_cfg.push_to_talk);

    // ── Платформенные сервисы ─────────────────────────────────────────────────
    #[cfg(target_os = "macos")]
    let ctx_capture: Box<dyn ContextCapture> = Box::new(MacOsContextCapture::new());
    #[cfg(target_os = "macos")]
    let injector: Box<dyn TextInjector> = Box::new(MacOsTextInjector::new());

    // ── Загрузка модели ───────────────────────────────────────────────────────
    print!("⏳ Загрузка модели...");
    std::io::stdout().flush().unwrap();
    let transcriber: Arc<Mutex<dyn transcriber::Transcriber>> =
        match WhisperTranscriber::new(MODEL_PATH) {
            Ok(t)  => { println!(" ✓"); Arc::new(Mutex::new(t)) }
            Err(e) => { eprintln!("\n❌ {e}"); std::process::exit(1); }
        };

    let transcribe_cfg = match LANGUAGE {
        Some(lang) => TranscribeConfig::with_language(lang),
        None       => TranscribeConfig::default(),
    };

    let pipeline_cfg = PipelineConfig {
        min_chunk_secs:   0.5,
        max_chunk_secs:   30.0,
        vad_window_secs:  0.3,
        vad_silence_rms:  0.01,
        vad_silence_secs: 0.5,
        overlap_secs:     0.2,
    };

    // ── Хоткеи ────────────────────────────────────────────────────────────────
    #[cfg(target_os = "macos")]
    let hotkey_result = hotkey::macos::spawn(hotkey_cfg);
    #[cfg(not(target_os = "macos"))]
    let hotkey_result = hotkey::rdev_impl::spawn(hotkey_cfg);

    let (_hotkey_handle, hotkey_rx) = match hotkey_result {
        Ok(r)  => r,
        Err(e) => { eprintln!("❌ {e}"); std::process::exit(1); }
    };

    // ── Ctrl+C ────────────────────────────────────────────────────────────────
    let running = Arc::new(AtomicBool::new(true));
    {
        let r = Arc::clone(&running);
        ctrlc::set_handler(move || r.store(false, Ordering::Relaxed)).unwrap();
    }

    println!("✅ Готово! Удерживай Right Option для записи, Ctrl+C для выхода.\n");

    // ── Главный цикл PTT ──────────────────────────────────────────────────────
    while running.load(Ordering::Relaxed) {

        // Ждём нажатия — событие несёт PID захваченный В МОМЕНТ нажатия
        let target_pid = match hotkey_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(HotkeyEvent::PushToTalkPressed { pid }) => pid,
            Ok(_) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };

        // Захватываем контекст для того PID который был активен при нажатии —
        // даже если пользователь уже переключился на другое приложение
        let snapshot = match ctx_capture.capture_for_pid(target_pid) {
            Ok(s) => {
                print!("📌 Цель: {s}");
                if s.screenshot.is_some() {
                    let ts   = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
                    let png_path = format!("screenshot_{ts}.png");
                    match s.save_screenshot(std::path::Path::new(&png_path)) {
                        Ok(_)  => {
                            println!("  →  {png_path}");

                            // ── Фоновый запуск OCR и сборки Markdown ──
                            let target_pid_clone = target_pid;
                            let ts_clone = ts.to_string().clone();
                            let png_path_clone = png_path.clone();

                            std::thread::spawn(move || {
                                let mut ax_helper_path = std::env::current_exe().unwrap_or_default();
                                ax_helper_path.pop();
                                ax_helper_path.push("ax-helper-bin");

                                match std::process::Command::new(&ax_helper_path)
                                    .arg("dump-screen")
                                    .arg(target_pid_clone.to_string())
                                    .arg(&png_path_clone)
                                    .output()
                                {
                                    Ok(out) if out.status.success() => {
                                        let json_str = String::from_utf8_lossy(&out.stdout);
                                        let md_text = vision::layout::process_dump_to_markdown(&json_str);
                                        let md_path = format!("screenshot_{}.md", ts_clone);

                                        if let Err(e) = std::fs::write(&md_path, md_text) {
                                            eprintln!("  [vision] ❌ Ошибка записи MD: {e}");
                                        } else {
                                            println!("  [vision] ✓ Markdown готов: {}", md_path);
                                        }
                                    }
                                    Ok(out) => {
                                        let err = String::from_utf8_lossy(&out.stderr);
                                        eprintln!("  [vision] ❌ Ошибка ax-helper: {}", err.trim());
                                    }
                                    Err(e) => {
                                        eprintln!("  [vision] ❌ Ошибка запуска ax-helper: {e}");
                                    }
                                }
                            });
                        },
                        Err(e) => println!("  →  screenshot err: {e}"),
                    }
                } else {
                    println!("  (нет Screen Recording прав)");
                }
                Some(s)
            }
            Err(e) => {
                eprintln!("⚠  Контекст: {e}");
                None
            }
        };

        println!("🔴 Говори...\n");

        let recorder = match CpalRecorder::new() {
            Ok(r)  => r,
            Err(e) => { eprintln!("❌ {e}"); continue; }
        };

        let record_flag = Arc::new(AtomicBool::new(true));
        let event_rx = match start_pipeline(
            recorder,
            Arc::clone(&transcriber),
            transcribe_cfg.clone(),
            pipeline_cfg.clone(),
            Arc::clone(&record_flag),
        ) {
            Ok(rx)  => rx,
            Err(e)  => { eprintln!("❌ {e}"); continue; }
        };

        let mut full_text = String::new();

        // Стримим текст пока зажата клавиша
        'recording: loop {
            match hotkey_rx.try_recv() {
                Ok(HotkeyEvent::PushToTalkReleased) => {
                    record_flag.store(false, Ordering::Relaxed);
                    print!("\n⏹  Обработка...");
                    std::io::stdout().flush().ok();
                    break 'recording;
                }
                _ => {}
            }
            if !running.load(Ordering::Relaxed) {
                record_flag.store(false, Ordering::Relaxed);
                break 'recording;
            }
            loop {
                match event_rx.try_recv() {
                    Ok(PipelineEvent::Text(text)) => {
                        print!("{text} ");
                        std::io::stdout().flush().ok();
                        if !full_text.is_empty() { full_text.push(' '); }
                        full_text.push_str(&text);
                    }
                    Ok(PipelineEvent::TranscribeError(e)) => eprintln!("\n⚠  {e}"),
                    Ok(PipelineEvent::Stopped)
                    | Err(crossbeam_channel::TryRecvError::Disconnected) => break 'recording,
                    Err(crossbeam_channel::TryRecvError::Empty) => break,
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // Дочитываем хвост буфера
        for event in &event_rx {
            match event {
                PipelineEvent::Text(text) => {
                    print!("{text} ");
                    std::io::stdout().flush().ok();
                    if !full_text.is_empty() { full_text.push(' '); }
                    full_text.push_str(&text);
                }
                PipelineEvent::TranscribeError(e) => eprintln!("\n⚠  {e}"),
                PipelineEvent::Stopped => break,
            }
        }
        println!("\n");

        // Вставляем текст в сохранённое приложение
        if !full_text.is_empty() {
            if let Some(ref snap) = snapshot {
                print!("💉 Вставка в {}...", snap.app_name);
                std::io::stdout().flush().ok();
                match injector.inject(&full_text, snap) {
                    Ok(_)  => println!(" ✓"),
                    Err(e) => eprintln!(" ❌ {e}"),
                }
            }
        }
        println!();
    }

    println!("👋 Пока!");
}