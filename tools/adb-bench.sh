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
trap 'adb shell svc power stayon false >/dev/null 2>&1 || true' EXIT

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
  adb shell dumpsys meminfo "$PKG" > /tmp/dumpsys-full.txt 2>&1 || true
  grep -E "TOTAL|Native Heap|EGL" /tmp/dumpsys-full.txt | head -n 20 | tee /tmp/dumpsys.txt || true
  if ! grep -q "TOTAL" /tmp/dumpsys.txt 2>/dev/null; then
    echo "(app instalada pero sin proceso: arráncala y repite dumpsys; PSS pendiente)"
  fi
  echo "== screencap =="
  adb exec-out screencap -p > /tmp/screen-TCL.png && echo "→ /tmp/screen-TCL.png $(wc -c < /tmp/screen-TCL.png) bytes" || echo "(screencap falló)"
  echo "== logcat frame p95 (si existe) =="
  adb logcat -d -s pdf_android:V 2>/dev/null | grep -i "frame\|p95\|render" | tail -n 20 | tee /tmp/frame-p95.txt || echo "(sin frame logs aún - Fase A1 pendiente)"
else
  echo "(paquete $PKG no instalado: sin dumpsys/screencap de app)"
fi

echo "== JSON consolidado =="
JSON="bench-results-TCL-$(date +%Y%m%d-%H%M%S).json"
MODEL=$(adb shell getprop ro.product.model | tr -d '\r')
SDK=$(adb shell getprop ro.build.version.sdk | tr -d '\r')
python3 - "$JSON" "$MODEL" "$SDK" "$RUNS" <<'EOF' || echo "(JSON no generado: python3 ausente)"
import json, re, sys
out, model, sdk, runs = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
doc = {"date": out.split("bench-results-TCL-")[1].rsplit(".json", 1)[0],
       "device": model, "sdk": sdk, "runs": []}
for i in range(1, runs + 1):
    try:
        txt = open(f"/tmp/bench-TCL-run{i}.txt").read()
    except OSError:
        continue
    entry = {"run": i}
    for m in re.finditer(r"(\w+) pages=(\d+) open=([\d.]+)ms render1x=([\d.]+)ms render2x=([\d.]+)ms", txt):
        entry[m.group(1)] = {"pages": int(m.group(2)), "open_ms": float(m.group(3)),
                             "render1x_ms": float(m.group(4)), "render2x_ms": float(m.group(5))}
    m = re.search(r"PEAK_RSS_KB=(\d+)", txt)
    if m:
        entry["peak_rss_kb"] = int(m.group(1))
    doc["runs"].append(entry)
try:
    mem = open("/tmp/dumpsys.txt").read()
    m = re.search(r"TOTAL PSS:\s+(\d+)", mem) or re.search(r"TOTAL\s+(\d+)\s+\d+\s+\d+", mem)
    doc["pss_kb"] = int(m.group(1)) if m else None
except OSError:
    doc["pss_kb"] = None
try:
    frames = open("/tmp/frame-p95.txt").read()
    vals = re.findall(r"frame p95=([\d.]+)ms \((\d+) frames\)", frames)
    doc["frame_p95_ms"] = float(vals[-1][0]) if vals else None
    doc["frame_presents"] = int(vals[-1][1]) if vals else None
except OSError:
    doc["frame_p95_ms"] = None
    doc["frame_presents"] = None
json.dump(doc, open(out, "w"), indent=2)
print(f"→ {out}")
EOF

adb shell svc power stayon false || true
echo "✅ Harness TCL completo. Resultados en /tmp/bench-TCL-run*.txt, /tmp/dumpsys.txt y ./${JSON:-bench-results-TCL-*.json}"
