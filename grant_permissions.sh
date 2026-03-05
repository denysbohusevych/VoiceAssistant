#!/bin/bash
# Выдаёт разрешения Screen Recording и Accessibility для бинаря VoiceAssistant

BINARY="$(pwd)/target/release/VoiceAssistant"

if [ ! -f "$BINARY" ]; then
    echo "❌ Сначала собери проект: cargo build --release"
    exit 1
fi

echo "Бинарь: $BINARY"
echo ""
echo "Открываю System Settings — добавь бинарь вручную:"
echo ""
echo "1. Screen Recording:"
open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
echo "   → нажми '+' → Cmd+Shift+G → вставь путь:"
echo "   $BINARY"
echo ""
read -p "Нажми Enter когда добавишь Screen Recording..."

echo ""
echo "2. Accessibility:"
open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
echo "   → нажми '+' → Cmd+Shift+G → вставь путь:"
echo "   $BINARY"
echo ""
read -p "Нажми Enter когда добавишь Accessibility..."

echo ""
echo "✅ Готово! Запускай: ./target/release/VoiceAssistant"