#!/bin/sh
# Скачивание GGML-модели Whisper
# Запуск: sh models/download.sh [tiny|base|small|medium|large|large-v3-turbo]

MODEL=${1:-base}
BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

# large-v3-turbo обрабатывается особым образом, если у него отличается постфикс в оригинальном репо,
# но обычно он скачивается именно как ggml-large-v3-turbo.bin
OUTPUT="models/ggml-${MODEL}.bin"

echo "Скачивание модели: ${MODEL}"
echo "→ ${OUTPUT}"
echo ""

curl -L --progress-bar -o "${OUTPUT}" "${BASE_URL}/ggml-${MODEL}.bin"

echo ""
echo "Готово: ${OUTPUT}"