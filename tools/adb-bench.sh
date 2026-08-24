#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Harness TCL NXTPaper 11 Plus (Fase A): dumpsys + screencap + logcat + pdf_bench sweep
# Uso: ./tools/adb-bench.sh [--pdf corpus/large_document.pdf] [--runs 5]
set -euo pipefail
PKG="${PKG:-com.pdflector.app}"
CORPUS_DIR="${CORPUS_DIR:-corpus}"
RUNS="${RUNS:-5}"
PDF="${PDF:-}"

if ! adb devices | grep -q "device$"; then
  echo "❌ No hay TCL conectada (adb devices vacío). Conecta por USB y autoriza RSA." >&2
  exit 1
fi

echo "== TCL: $(adb shell getprop ro.product.model | tr -d '\r') SDK $(adb shell getprop ro.build.version.sdk | tr -d '\r') =="
adb shell input keyevent KEYCODE_WAKEUP || true
adb shell svc power stayon true || true

echo "== Build pdf_bench aarch64 =="
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/android-ndk-r28}"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
cargo build -p pdf_bench --target aarch64-linux-android --release

DEST="/data/local/tmp/pdflector"
adb shell mkdir -p "$DEST/corpus"
adb push target/aarch64-linux-android/release/pdf_bench "$DEST/pdf_bench"
adb shell chmod +x "$DEST/pdf_bench"
for f in "$CORPUS_DIR"/*.pdf; do adb push "$f" "$DEST/corpus/"; done

echo "== Sweep $RUNS runs =="
for i in $(seq 1 "$RUNS"); do
  echo "--- run $i ---"
  adb shell "PDFLECTOR_CORPUS_DIR=$DEST/corpus $DEST/pdf_bench" | tee "/tmp/bench-TCL-run$i.txt"
  cat "/tmp/bench-TCL-run$i.txt"
done

if adb shell pm list packages | grep -q "$PKG"; then
  echo "== dumpsys meminfo $PKG =="
  adb shell dumpsys meminfo "$PKG" | grep -E "TOTAL|Native Heap|EGL" | head -n 20 | tee /tmp/dumpsys.txt
  cat /tmp/dumpsys.txt
  echo "== screencap =="
  adb exec-out screencap -p > /tmp/screen-TCL.png && echo "→ /tmp/screen-TCL.png $(wc -c < /tmp/screen-TCL.png) bytes"
  echo "== logcat frame p95 (si existe) =="
  adb logcat -d -s pdf_android:V | grep -i "frame\|p95\|render" | tail -n 20 || echo "(sin frame logs aún - Fase A1 pendiente)"
fi

adb shell svc power stayon false || true
echo "✅ Harness TCL completo. Resultados en /tmp/bench-TCL-run*.txt y /tmp/dumpsys.txt"
