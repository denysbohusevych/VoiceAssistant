// build.rs
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // 1. Скачивание модели (если она еще не скачана)
    let model_path = PathBuf::from("models/ggml-base.bin");
    if !model_path.exists() {
        println!("cargo:warning=Модель ggml-base.bin не найдена. Начинаю скачивание...");
        let status = Command::new("sh")
            .args(&["models/download.sh", "base"])
            .status()
            .expect("Не удалось запустить models/download.sh");

        if !status.success() {
            panic!("Ошибка при скачивании модели Whisper!");
        }
    }

    // 2. Сборка Swift-бинарника ax-helper
    println!("cargo:warning=Сборка ax-helper...");
    let swift_status = Command::new("sh")
        .arg("ax-helper/buildSwift.sh")
        .status()
        .expect("Не удалось запустить ax-helper/buildSwift.sh");

    if !swift_status.success() {
        panic!("Ошибка при сборке ax-helper!");
    }

    // Говорим Cargo пересобрать проект, если мы изменили код Swift-хелпера
    println!("cargo:rerun-if-changed=ax-helper/main.swift");
    println!("cargo:rerun-if-changed=ax-helper/buildSwift.sh");

    // 3. Твой оригинальный код для копирования ggml-metal.metal
    let home = std::env::var("HOME").unwrap_or_default();
    let registry = PathBuf::from(&home).join(".cargo/registry/src");

    let metal_file = find_file(&registry, "ggml-metal.metal");

    match metal_file {
        Some(src) => {
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
            let dst = PathBuf::from(&manifest_dir).join("ggml-metal.metal");

            if !dst.exists() {
                std::fs::copy(&src, &dst).expect("Не могу скопировать ggml-metal.metal");
                println!("cargo:warning=Скопирован ggml-metal.metal из {}", src.display());
            }

            println!("cargo:rerun-if-changed=ggml-metal.metal");
        }
        None => {
            println!("cargo:warning=ggml-metal.metal не найден в cargo registry — Metal может не работать");
        }
    }
}

fn find_file(dir: &PathBuf, name: &str) -> Option<PathBuf> {
    if !dir.exists() { return None; }

    let read = std::fs::read_dir(dir).ok()?;
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}