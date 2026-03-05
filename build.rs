// build.rs
// Находит ggml-metal.metal в cargo registry и копирует его в OUT_DIR,
// а также выводит путь чтобы можно было скопировать рядом с бинарём.

use std::path::PathBuf;

fn main() {
    // Ищем ggml-metal.metal в ~/.cargo/registry
    let home = std::env::var("HOME").unwrap_or_default();
    let registry = PathBuf::from(&home).join(".cargo/registry/src");

    let metal_file = find_file(&registry, "ggml-metal.metal");

    match metal_file {
        Some(src) => {
            // Копируем в корень проекта (CARGO_MANIFEST_DIR)
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
            let dst = PathBuf::from(&manifest_dir).join("ggml-metal.metal");

            if !dst.exists() {
                std::fs::copy(&src, &dst).expect("Не могу скопировать ggml-metal.metal");
                println!("cargo:warning=Скопирован ggml-metal.metal из {}", src.display());
            }

            // Говорим компилятору пересобрать если файл изменился
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