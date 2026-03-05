#!/bin/sh
# Скачивание GGML-модели Whisper
# Запуск: sh models/download.sh [tiny|base|small|medium|large]

MODEL=${1:-base}
BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
OUTPUT="models/ggml-${MODEL}.bin"

echo "Скачивание модели: ${MODEL}"
echo "→ ${OUTPUT}"
echo ""

curl -L --progress-bar -o "${OUTPUT}" "${BASE_URL}/ggml-${MODEL}.bin"

echo ""
echo "Готово: ${OUTPUT}"