#!/bin/sh
# Собирает ax-helper Swift бинарь и кладет куда нужно

set -e

OUT="ax-helper-bin"

echo "Building ax-helper..."
swiftc ax-helper/main.swift \
    -O \
    -framework Cocoa \
    -framework ApplicationServices \
    -framework ScreenCaptureKit \
    -framework UniformTypeIdentifiers \
    -framework ImageIO \
    -framework Vision \
    -o "$OUT"

chmod +x "$OUT"

# ГАРАНТИРОВАННО копируем бинарник туда, где его ищет Rust
mkdir -p target/debug target/release
cp "$OUT" target/debug/
cp "$OUT" target/release/

echo "✓ Built and copied to target directories!"