// src/config.rs
//
// ╔══════════════════════════════════════════════════════════════════════════════╗
// ║  Централизованный конфиг приложения                                         ║
// ║                                                                              ║
// ║  Использование:                                                              ║
// ║    let cfg = config::load_shared(config::CONFIG_PATH);                       ║
// ║    config::spawn_hot_reload_watcher(cfg.clone(), CONFIG_PATH.into(), 3);     ║
// ║                                                                              ║
// ║    // Читать конфиг (любой поток):                                           ║
// ║    let silence = cfg.read().unwrap().audio.silence_threshold;                ║
// ║                                                                              ║
// ║  Формат файла: TOML (config.toml в рабочей директории)                      ║
// ║  Hot-reload:   файл опрашивается каждые N секунд по mtime                   ║
// ╚══════════════════════════════════════════════════════════════════════════════╝

use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use serde::{Deserialize, Serialize};

pub const CONFIG_PATH: &str = "config.toml";

/// Потокобезопасный разделяемый конфиг.
/// Клонируйте `SharedConfig` в каждый воркер — это дёшево (Arc).
/// Читайте через `.read().unwrap()`, пишите через `.write().unwrap()`.
pub type SharedConfig = Arc<RwLock<AppConfig>>;

// ─── Корень конфига ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub whisper: WhisperConfig,
    pub audio:   AudioConfig,
    pub ai:      AiConfig,
    pub hotkey:  HotkeyConfig,
    pub vision:  VisionConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            whisper: WhisperConfig::default(),
            audio:   AudioConfig::default(),
            ai:      AiConfig::default(),
            hotkey:  HotkeyConfig::default(),
            vision:  VisionConfig::default(),
        }
    }
}

// ─── Whisper ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    /// Путь к GGML-модели Whisper
    pub model_path:         String,
    /// Язык: "auto", "ru", "en", ...
    pub language:           String,
    /// Минимальное число сэмплов (< этого — шум, не транскрибируем)
    pub min_audio_samples:  usize,
    /// Порог вероятности тишины (no_speech_thold в whisper.cpp)
    pub no_speech_threshold: f32,
    /// Порог энтропии (entropy_thold в whisper.cpp)
    pub entropy_threshold:  f32,
    /// Подсказка для модели (initial_prompt)
    pub initial_prompt:     String,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model_path:          "models/ggml-large-v3-turbo.bin".into(),
            language:            "auto".into(),
            min_audio_samples:   4000,
            no_speech_threshold: 0.4,
            entropy_threshold:   2.4,
            initial_prompt:      "Привет, это test voice command. Я пишу код на Rust, Python. System initialization.".into(),
        }
    }
}

// ─── Аудио пайплайн ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// RMS ниже этого порога = тишина, транскрипция не запускается
    pub silence_threshold:           f32,
    /// Интервал проверки буфера (мс)
    pub chunk_duration_ms:           u64,
    /// Принудительная нарезка (Sliding Window) через N секунд
    pub sliding_window_max_seconds:  f32,
    /// Перекрытие между кусками при нарезке (сек)
    pub overlap_seconds:             f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            silence_threshold:          0.005,
            chunk_duration_ms:          500,
            sliding_window_max_seconds: 12.0,
            overlap_seconds:            1.5,
        }
    }
}

// ─── AI провайдер ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// "ollama" | "gemini" | "openai" | "claude" | "deepseek"
    pub provider:           String,
    /// Имя модели (зависит от провайдера)
    pub model:              String,
    /// Максимальное число токенов в ответе
    pub max_tokens:         u32,
    /// Температура генерации
    pub temperature:        f32,
    /// Опциональный путь к файлу с системным промптом
    pub system_prompt_file: Option<String>,
    /// HTTP таймауты (для облачных провайдеров)
    pub http_timeouts:      HttpTimeoutsConfig,
    /// Параметры Ollama
    pub ollama:             OllamaConfig,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider:           "ollama".into(),
            model:              "qwen3.5:0.8b".into(),
            max_tokens:         2048,
            temperature:        0.7,
            system_prompt_file: None,
            http_timeouts:      HttpTimeoutsConfig::default(),
            ollama:             OllamaConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpTimeoutsConfig {
    pub connect_secs: u64,
    pub read_secs:    u64,
    pub write_secs:   u64,
}

impl Default for HttpTimeoutsConfig {
    fn default() -> Self {
        Self { connect_secs: 15, read_secs: 120, write_secs: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub base_url:    String,
    pub num_predict: u32,
    pub num_ctx:     u32,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url:    "http://localhost:11434".into(),
            num_predict: 5000,
            num_ctx:     6000,
        }
    }
}

// ─── Хоткей ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// Клавиша PTT: "AltRight", "CapsLock", "F5", "a", ...
    pub push_to_talk: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { push_to_talk: "AltRight".into() }
    }
}

// ─── Vision ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VisionConfig {
    /// Минимальное число полезных узлов AX Tree (меньше → OCR-фолбек)
    pub ax_tree_min_useful_nodes:    usize,
    /// Минимальная доля покрытия экрана контентом (меньше → OCR-фолбек)
    pub ax_tree_min_coverage_ratio:  f64,
    /// Геометрические пороги раскладки
    pub layout:                      LayoutConfig,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            ax_tree_min_useful_nodes:   5,
            ax_tree_min_coverage_ratio: 0.35,
            layout:                     LayoutConfig::default(),
        }
    }
}

/// Геометрические параметры построения layout из OCR-данных.
/// Все ratio — доли от median_font_height (высоты символа).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Выравнивание по Y: слова в пределах ratio*h считаются одной строкой
    pub line_y_alignment_ratio:     f64,
    /// Максимальный горизонтальный зазор для слияния слов в строку
    pub word_merge_x_gap_ratio:     f64,
    pub word_merge_x_overlap_ratio: f64,
    /// Вертикальный зазор для группировки строк в блок
    pub block_y_gap_ratio:          f64,
    pub block_x_overlap_ratio:      f64,
    pub block_x_alignment_ratio:    f64,
    /// Минимальная ширина "просвета" между колонками
    pub column_gutter_ratio:        f64,
    /// Минимальное число блоков для самостоятельной колонки
    pub column_min_blocks:          usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            line_y_alignment_ratio:     0.4,
            word_merge_x_gap_ratio:     1.5,
            word_merge_x_overlap_ratio: 1.0,
            block_y_gap_ratio:          1.2,
            block_x_overlap_ratio:      0.5,
            block_x_alignment_ratio:    3.0,
            column_gutter_ratio:        4.0,
            column_min_blocks:          3,
        }
    }
}

impl LayoutConfig {
    pub fn messenger() -> Self {
        Self { column_gutter_ratio: 4.0, column_min_blocks: 3, ..Default::default() }
    }
    pub fn desktop_app() -> Self {
        Self { column_gutter_ratio: 6.0, column_min_blocks: 2, ..Default::default() }
    }
    pub fn document() -> Self {
        Self {
            column_gutter_ratio:     5.0,
            column_min_blocks:       2,
            block_y_gap_ratio:       1.5,
            block_x_alignment_ratio: 4.0,
            ..Default::default()
        }
    }
}

// ─── Загрузка, сохранение, перезагрузка ──────────────────────────────────────

impl AppConfig {
    /// Загружает конфиг из файла. Если файл не существует — создаёт с дефолтами.
    pub fn load_or_create(path: &str) -> Self {
        let p = Path::new(path);

        if p.exists() {
            match fs::read_to_string(p) {
                Ok(content) => match toml::from_str::<AppConfig>(&content) {
                    Ok(cfg) => {
                        println!("✅ Конфиг загружен: {}", path);
                        return cfg;
                    }
                    Err(e) => eprintln!("⚠️  Ошибка парсинга {}: {}. Используется дефолт.", path, e),
                },
                Err(e) => eprintln!("⚠️  Не удалось прочитать {}: {}. Используется дефолт.", path, e),
            }
        }

        let cfg = AppConfig::default();
        cfg.save(path);
        println!("📝 Создан дефолтный конфиг: {}", path);
        cfg
    }

    /// Сохраняет конфиг в файл (pretty TOML).
    pub fn save(&self, path: &str) {
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = fs::write(path, &s) {
                    eprintln!("⚠️  Не удалось сохранить конфиг в {}: {}", path, e);
                }
            }
            Err(e) => eprintln!("⚠️  Ошибка сериализации конфига: {}", e),
        }
    }

    /// Перезагружает конфиг из файла in-place. Возвращает true при успехе.
    pub fn reload(&mut self, path: &str) -> bool {
        match fs::read_to_string(path) {
            Ok(content) => match toml::from_str::<AppConfig>(&content) {
                Ok(new_cfg) => { *self = new_cfg; true }
                Err(e) => {
                    eprintln!("⚠️  Ошибка парсинга при перезагрузке: {}", e);
                    false
                }
            },
            Err(e) => {
                eprintln!("⚠️  Не удалось прочитать конфиг: {}", e);
                false
            }
        }
    }
}

// ─── Фабрика SharedConfig ─────────────────────────────────────────────────────

/// Загружает конфиг и оборачивает в `Arc<RwLock>`.
pub fn load_shared(path: &str) -> SharedConfig {
    Arc::new(RwLock::new(AppConfig::load_or_create(path)))
}

// ─── Hot-reload воркер ────────────────────────────────────────────────────────

/// Запускает фоновый поток, который отслеживает изменения файла конфига
/// и перезагружает его при изменении mtime.
///
/// `interval_secs` — интервал опроса (рекомендуется 3).
pub fn spawn_hot_reload_watcher(shared: SharedConfig, path: String, interval_secs: u64) {
    std::thread::spawn(move || {
        let mut last_modified: Option<SystemTime> = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok();

        loop {
            std::thread::sleep(Duration::from_secs(interval_secs));

            let current = fs::metadata(&path).and_then(|m| m.modified()).ok();

            let changed = match (&last_modified, &current) {
                (Some(old), Some(new)) => new != old,
                (None, Some(_))        => true,
                _                      => false,
            };

            if changed {
                last_modified = current;
                match shared.write() {
                    Ok(mut cfg) => {
                        if cfg.reload(&path) {
                            println!("🔄 Конфиг перезагружен: {}", path);
                        }
                    }
                    Err(e) => eprintln!("⚠️  Не удалось захватить write lock конфига: {}", e),
                }
            }
        }
    });
}

// ─── Вспомогательные функции ──────────────────────────────────────────────────

/// Загружает системный промпт: из файла (если задан в конфиге) или дефолтный.
pub fn load_system_prompt(cfg: &AppConfig) -> String {
    if let Some(ref file_path) = cfg.ai.system_prompt_file {
        match fs::read_to_string(file_path) {
            Ok(s) => {
                println!("📄 Системный промпт загружен из: {}", file_path);
                return s;
            }
            Err(e) => eprintln!("⚠️  Не удалось прочитать системный промпт {}: {}", file_path, e),
        }
    }
    crate::ai::DEFAULT_SYSTEM_PROMPT.to_string()
}