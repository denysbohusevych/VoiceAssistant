# Voice Recorder + Whisper STT

Записывает голос и транскрибирует его локально через whisper.cpp.

## Зависимости (macOS)

```bash
brew install cmake
```
cmake нужен для компиляции whisper.cpp при первой сборке.

## Установка

```bash
# 1. Скачать модель (base — хороший баланс скорости и качества)
bash models/download.sh base

# 2. Собрать (первый раз долго — компилирует whisper.cpp)
cargo build --release

# 3. Запустить
cargo run --release
```

## Управление

| Действие | Клавиши |
|----------|---------|
| Начать запись | Enter |
| Остановить запись | Ctrl+C |
| Выйти из программы | Ctrl+C (когда не пишет) |

## Настройки (src/main.rs)

```rust
const MODEL_PATH: &str = "models/ggml-base.bin"; // путь к модели
const LANGUAGE: Option<&str> = None;              // None = авто, Some("ru") = русский
```

## Модели

| Модель  | Размер | Скорость | Качество |
|---------|--------|----------|---------|
| tiny    | 75 MB  | ⚡⚡⚡⚡   | ★★☆☆   |
| base    | 142 MB | ⚡⚡⚡    | ★★★☆   |
| small   | 466 MB | ⚡⚡      | ★★★★   |
| medium  | 1.5 GB | ⚡       | ★★★★★  |
| large   | 3 GB   | 🐢       | ★★★★★  |

## Структура проекта

```
src/
├── main.rs                        # Главный цикл
├── recorder/
│   ├── mod.rs                     # trait AudioRecorder
│   └── cpal_recorder.rs           # Реализация через cpal (macOS/Win/Linux)
└── transcriber/
    ├── mod.rs                     # trait Transcriber
    └── whisper.rs                 # Реализация через whisper-rs
```