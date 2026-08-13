---
name: pdflector-rendimiento
description: Procedimiento completo de medición de rendimiento del PDFLector —
benchmark criterion en desktop (`cargo bench -p pdf_bench`), sweep binario
(`cargo run --release -p pdf_bench`), cross-compile a aarch64-linux-android
(NDK r28), harness en la tablet TCL vía adb + PDFLECTOR_CORPUS_DIR, y app
pdf_android (cargo-apk, run-as, dumpsys meminfo, logcat, screencap + análisis
de píxeles con pillow), con pantalla ON y comparación contra los objetivos de
AGENTS.md §8. Úsalo siempre que toque medir o verificar rendimiento (desktop o
tablet) o cerrar el criterio de aceptación de una fase.
---

# Medición de rendimiento — PDFLector

Procedimiento unificado de medición: desktop (criterion + sweep binario),
cross-compile Android y tablet (harness `pdf_bench` + app `pdf_android`).
Consolida lo que ya se hizo en el repo: `docs/benchmark-results.md`
(Fase 0.5 desktop/Android, Fase 1 B1 caché, B3 zoom, mediciones TCL NXTPaper
11 Plus), `memory.md` (entrada 2026-08-13: app Android nativa) y el skill
`.opencode/skills/android-tablet-adb/SKILL.md`.

## Objetivos a comprobar (AGENTS.md §8)

| Métrica | Objetivo | Cómo se mide |
|---|---|---|
| Render de página en tablet | < 25 ms | Sweep `pdf_bench` en la tablet (`render1x`) |
| RSS en tablet, PDF 500 pág. | < 150 MB | `PEAK_RSS_KB` del sweep; app: `dumpsys meminfo` |
| Frame time p95 en scroll | < 16,6 ms (60 fps) | Overlay de debug en `pdf_app` (escritorio) |

Regla: medir antes de optimizar. Ninguna afirmación de rendimiento sin datos
(ver AGENTS.md §3). Anota cada medición con fecha y hardware en el doc de la
fase o ADR correspondiente.

## 1. Medición desktop

### Sweep binario (rápido, open/render1x/render2x + RSS)

```bash
cargo run --release -p pdf_bench
```

- Barre el corpus (`corpus/` del workspace, resuelto por `pdf_core::corpus_dir`)
  con MuPDF: `open`, `render1x`, `render2x` (mediana de 3, páginas 0/mitad/última)
  y termina con `PEAK_RSS_KB` (VmHWM de `/proc/self/status`, sin polling).
- Incluye la sección zoom B3: `scale2x/scale4x` (upscale software de `scale_bitmap`)
  vs `rerender1/rerender2` (re-render nítido MuPDF) en la página 0 de
  large/dense. Hallazgo documentado: `scale_bitmap` es ~16-18× más lento que el
  re-render en desktop y ~4-5,6× en la tablet — no usarlo como fast path de UI.
- Si el corpus está fuera del workspace (p. ej. otra máquina), apuntar
  `PDFLECTOR_CORPUS_DIR` a la carpeta con los PDFs.

### Benchmark criterion (preciso, por grupo)

```bash
cargo bench -p pdf_bench                 # todos los grupos
cargo bench -p pdf_bench -- cache_scroll # filtrar un grupo
```

Grupos en `crates/pdf_bench/benches/`: `open_render` (open+render),
`cache_scroll` (caché LRU vs naive, con VMHWM de proceso hijo separado por
escenario — el pico del kernel es monotónico y lo contaminaría el escenario
naive), `zoom` (escalado software vs re-render) y `render_perf`. Usar
`criterion --quick` para iteraciones rápidas (p. ej. `-- --quick`).

### Frame time p95 (scroll, escritorio)

Overlay de debug en `pdf_app` (preferencia de eframe storage): muestra p95 de
frame time, RSS y estado de caché. Objetivo p95 < 16,6 ms (60 fps sostenidos).
Prueba manual; no invertir en tests de UI del prototipo.

## 2. Cross-compile a Android (harness `pdf_bench`)

Precondiciones: NDK r28 en `~/Android/Sdk/ndk/`, target instalado
(`rustup target add aarch64-linux-android`), tablet TCL NXTPaper 11 Plus
(9469X, serial A06B4A8E6774623) por USB (`adb devices`).

El repo ya trae el cross configurado en `.cargo/config.toml`
(`linker = "aarch64-linux-android24-clang"`) — **NO reconfigurar**. Requisitos
de entorno en tiempo de build:

```bash
export ANDROID_NDK_HOME=~/Android/Sdk/ndk/<r28...>
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

cargo build -p pdf_bench --target aarch64-linux-android --release
```

- `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` es obligatoria para
  `mupdf-sys` (bindgen no usa el wrapper clang del NDK): sin ella falla con
  `pthreadtypes-arch.h` (glibc del host). El linker corto del NDK debe estar en
  PATH; sin él, "file in wrong format" al linkear.
- El resultado va a `target/aarch64-linux-android/release/pdf_bench`.

### Despliegue y ejecución en la tablet

```bash
adb push target/aarch64-linux-android/release/pdf_bench /data/local/tmp/pdflector/
adb push corpus/*.pdf /data/local/tmp/pdflector/corpus/
adb shell chmod +x /data/local/tmp/pdflector/pdf_bench
adb shell 'PDFLECTOR_CORPUS_DIR=/data/local/tmp/pdflector/corpus /data/local/tmp/pdflector/pdf_bench'
```

- El sweep reporta `open/render1x/render2x` por PDF + `PEAK_RSS_KB`.
- `PDFLECTOR_CORPUS_DIR` sobreescribe la resolución de corpus de
  `pdf_core::corpus_dir()` (que por defecto asume el workspace del host).
- Método ya usado (docs/benchmark-results.md, sección TCL): mediana de 3
  intentos, 2 corridas estables (difieren <5 %). Si hay varianza termal
  (tablet cargando, governor), repetir N≥5 corridas antes de concluir
  regresiones.

## 3. App en tablet (`pdf_android`)

Crate `crates/pdf_android` (cdylib, android-activity 0.6 native-activity,
package `com.pdflector.app`). Empaquetado con cargo-apk v0.10.0.

### Build + install

```bash
cargo apk build --release --target aarch64-linux-android -p pdf_android
adb install -r target/release/apk/pdf_android.apk
```

- El APK release va a `target/release/apk/pdf_android.apk` (debug:
  `target/debug/apk/pdf_android.apk`). El objetivo <150 MB RSS aplica a
  RELEASE: un build debug con debuginfo dio ~205 MB RSS (TOTAL RSS, dumpsys) —
  no concluir nada de un APK debug.
- Ojo: `cargo apk build` no deja el binario en la misma ruta que el build de
  `pdf_bench` (mismo target dir); el APK es lo que se despliega.

### Inyectar el PDF (SELinux)

SELinux impide que un untrusted_app lea `/data/local/tmp`; la app lee
`internal_data_path()/demo.pdf`. El APK debe ser debuggable (debug) para
`run-as`:

```bash
adb push corpus/scientific_paper.pdf /data/local/tmp/demo.pdf
adb shell run-as com.pdflector.app sh -c 'cp /data/local/tmp/demo.pdf files/demo.pdf'
```

(`run-as` arranca en el dir de datos de la app; `files/` = `internal_data_path()`).

### Lanzar y medir

```bash
adb shell am start -n com.pdflector.app/android.app.NativeActivity
adb shell dumpsys meminfo com.pdflector.app   # TOTAL PSS / TOTAL RSS
adb logcat -d -s pdf_android:V                # tiempos ("opened N pages", render...)
```

- El tag de logcat es `pdf_android` (android_logger, ver
  `crates/pdf_android/src/lib.rs`).
- RSS de la app: `TOTAL RSS` de `dumpsys meminfo`.

### Verificación visual por píxeles

```bash
adb exec-out screencap -p > /tmp/screen.png
uv run --with pillow python - <<'EOF'
from PIL import Image
im = Image.open("/tmp/screen.png").convert("RGB")
px = list(im.getdata())
print("mean rgb:", tuple(sum(c[i] for c in px)//len(px) for i in range(3)))
print("white %:", 100*sum(1 for c in px if c[0]>240 and c[1]>240 and c[2]>240)/len(px))
print("red px:", sum(1 for c in px if c[0]>200 and c[1]<80 and c[2]<80))
EOF
```

Ya usado en la verificación de la Fase 1 (memory.md 2026-08-13): media
236,236,236, ~91 % blanco, 0 píxeles rojos → página renderizada sobre letterbox
gris sin errores. El red-check detecta buffers sin inicializar / fallos de
formato (defensa RGB565 → forzado R8G8B8A8_UNORM).

## 4. Pantalla ON durante la medición

Con pantalla apagada el governor/doze puede bajar frecuencias y pesimizar el
resultado (nota metodológica de docs/benchmark-results.md). Antes de medir:

```bash
adb shell input keyevent KEYCODE_WAKEUP
adb shell svc power stayon true
```

Después (limpieza obligatoria):

```bash
adb shell svc power stayon false
adb shell input keyevent KEYCODE_SLEEP   # opcional: volver a apagar
```

## 5. Limpieza del dispositivo

```bash
adb shell rm -f /data/local/tmp/pdflector/pdf_bench /data/local/tmp/pdflector/corpus/*.pdf
# opcional: adb uninstall com.pdflector.app
```

## Registro de resultados

- Anotar cada medición con fecha, hardware (máquina / tablet, build debug o
  release) y las condiciones (pantalla ON/OFF, battery/thermal) en el doc de la
  fase o ADR correspondiente (docs/benchmark-results.md lleva el historial).
- Al cerrar una fase: verificar el criterio de aceptación con estas mediciones y
  marcar el hito en `docs/PLAN.md` (AGENTS.md §12).
- Comparaciones entre corridas: ojo a confounds (p. ej. corpus corregido que
  sube render de scanned y RSS; varianza termal). No declarar regresiones sin
  N≥5 corridas con governor fijo.

## Problemas conocidos

- `pthreadtypes-arch.h` al compilar mupdf-sys → falta
  `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` (sysroot del NDK).
- "file in wrong format" al linkear → falta el linker del NDK en PATH
  (config ya en `.cargo/config.toml`).
- App no ve el PDF tras instalar → inyectar con `run-as` (SELinux) y verificar
  que el APK es debuggable.
- RSS alto en APK debug (con debuginfo) → irrelevante; medir el APK release.
