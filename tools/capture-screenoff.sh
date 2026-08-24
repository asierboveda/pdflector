#!/usr/bin/env bash
# Captura EN VIVO del bug "pantalla que se apaga/parpadea al escribir"
# (docs/plan/BUG-pantalla-apagada.md — Paso 0).
#
# Uso:
#   1) conecta la tablet por USB y lanza este script
#   2) reproduce el gesto con el LÁPIZ (trazo cruzando tinta + soltar, x3-5)
#   3) Ctrl+C al terminar
set -euo pipefail
TS=$(date +%H%M%S)
DIR=/tmp/screenoff-$TS
mkdir -p "$DIR"
echo "== Captura en $DIR — reproduce el gesto con el lápiz ahora =="

adb shell dumpsys power > "$DIR/power-before.txt" 2>&1 || true
adb shell "dumpsys window windows | grep -B2 -A2 pdflector" > "$DIR/window-before.txt" 2>&1 || true
adb shell settings get system screen_off_timeout > "$DIR/timeout.txt" 2>&1 || true

adb logcat -v threadtime > "$DIR/logcat.txt" 2>&1 &
LOGCAT_PID=$!
trap 'kill $LOGCAT_PID 2>/dev/null || true' EXIT

echo ">>> Dibuja ahora. Esperando 60s (o Ctrl+C)..."
sleep 60

adb shell dumpsys power > "$DIR/power-after.txt" 2>&1 || true
adb shell "dumpsys window windows | grep -B2 -A2 pdflector" > "$DIR/window-after.txt" 2>&1 || true
adb shell dumpsys activity top > "$DIR/activity.txt" 2>&1 || true
kill $LOGCAT_PID 2>/dev/null || true

echo "== Resumen (pista rápida) =="
grep -E "mWakefulness=|mHoldingWakeLockSuspendBlocker" "$DIR/power-before.txt" "$DIR/power-after.txt" || true
grep -nE "Destroyed Activity|Displayed com.pdflector|ActivityTaskManager" "$DIR/logcat.txt" | tail -n 10 || true
grep -nE "surfaceflinger|MALI|egl" "$DIR/logcat.txt" | tail -n 10 || true
grep -nE "nvr|NvrHung|InputDispatcher.*timeout|ANR" "$DIR/logcat.txt" | tail -n 10 || true
echo "== Logs en $DIR =="
