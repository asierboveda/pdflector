---
name: pdflector-rendimiento
description: Procedimiento completo de medición de rendimiento y despliegue de PDFLector — benchmark criterion en desktop (`cargo bench -p pdf_bench`), sweep binario (`cargo run --release -p pdf_bench`), cross-compile a aarch64-linux-android (NDK r28 / API 26-35), despliegue y harness automatizado en la tablet TCL NXTPaper 11 Plus (9469X) vía adb (`tools/adb-bench.sh`), y profiling de la app `pdf_android` (cargo-apk, run-as, dumpsys meminfo, logcat, screencap + análisis de píxeles), con pantalla ON y registro de evidencia en `docs/benchmark-results.md`. Úsalo para medir o verificar rendimiento en desktop o tablet y cerrar criterios de aceptación.
---

# Rendimiento y despliegue — PDFLector

Procedimiento unificado para compilar, desplegar y medir rendimiento en PDFLector:
1. **Desktop**: sweep binario rápido y suite Criterion en el host (banco de pruebas del core, no producto).
2. **Tablet TCL NXTPaper 11 Plus (9469X)**: cross-compile a `aarch64-linux-android`, harness automatizado (`tools/adb-bench.sh`), sweep manual de `pdf_bench` y perfilado de la app nativa `pdf_android` (`cargo-apk`, `dumpsys meminfo`, `logcat`, screencap).

Toda medición se registra en un único fichero append-only: `docs/benchmark-results.md`.

## Objetivos a comprobar (AGENTS.md MUST y Definición de hecho)

| Métrica | Techo / Objetivo | Cómo se mide |
|---|---|---|
| Render de página en tablet | < 25 ms | Sweep `pdf_bench` en la tablet (`render1x`) |
| Frame time p95 (interacción / scroll) | < 16,6 ms (60 fps); objetivo 8,33 ms (120 Hz) | Logcat `pdf_android:V` ("frame p95=...ms") en la TCL; overlay en `pdf_app` |
| Memoria en arranque (PSS) | < 150 MB | App release: `dumpsys meminfo com.pdflector.app` |
| Memoria en reposo (PSS) | ≤ 180 MB | App release: `dumpsys meminfo com.pdflector.app` |
| Memoria tras 15 ciclos Library→Viewer | ≤ 200 MB (PSS pico) | App release: `dumpsys meminfo com.pdflector.app` |
| Peak RSS en sweep tablet | < 150 MB | `PEAK_RSS_KB` reportado por `pdf_bench` |

Regla vinculante (AGENTS.md MUST): medir antes de afirmar. Toda medición debe registrarse con **fecha + hardware + flujo medido + métrica** en `docs/benchmark-results.md`. Si un flujo no se ha medido, consignar `SIN MEDIR`.

## 1. Medición en desktop (host)

El binario de escritorio (`pdf_app`) es un banco de pruebas del core, no la plataforma de producto.

### Sweep binario (rápido: open, render 1x/2x, zoom B3 + RSS)

```bash
cargo run --release -p pdf_bench
```

- Resuelve el corpus (`corpus/` del workspace) mediante `pdf_core::corpus_dir()`.
- Para apuntar a otra carpeta de PDFs, definir la variable de entorno:
  ```bash
  PDFLECTOR_CORPUS_DIR=/ruta/a/corpus cargo run --release -p pdf_bench
  ```
- Mide mediana de 3 corridas en páginas representativas (primera, central, última) y muestra `PEAK_RSS_KB` (VmHWM de `/proc/self/status`).
- Incluye comparativa de zoom B3 (`scale_bitmap` por software vs re-render nítido MuPDF).

### Benchmark Criterion (preciso, por grupo)

```bash
cargo bench -p pdf_bench                        # todos los benchmarks
cargo bench -p pdf_bench --bench cache_scroll   # ejecutar un fichero concreto
cargo bench -p pdf_bench --bench cache_scroll -- --quick # modo rápido
```

Los 8 ficheros de bench en `crates/pdf_bench/benches/` son:
1. `open_render`: apertura de documento y render a escala 1x y 2x.
2. `cache_scroll`: política de caché LRU vs naive, aislando el pico VmHWM en subproceso.
3. `render_perf`: rendimiento puro de renderizado por página.
4. `zoom`: re-renderizado en escala vs escalado por software (`scale_bitmap`).
5. `annotations`: creación, serialización JSON y mutación de `AnnotationSet`.
6. `blit`: copiado de búfers de píxeles y transferencias de memoria (espejo de las rutas CPU de `pdf_android/src/draw/`).
7. `highlight`: detección de quads de texto y generación de resaltados.
8. `prefetch`: comportamiento del prefetching predictivo de páginas adyacentes (±1).

## 2. Configuración de cross-compilación a Android

### Hardware y SDK
- Tablet: TCL NXTPaper 11 Plus (**9469X**, Android 15, API 35, pantalla 120 Hz, serial `A06B4A8E6774623`).
- Android NDK: r28 (instalado habitualmente en `~/Android/Sdk/ndk/android-ndk-r28` o `~/Android/Sdk/ndk/28.*`).
- Target Rust: `aarch64-linux-android` (`rustup target add aarch64-linux-android`).

### Configuración de toolchain y variables de entorno
En `.cargo/config.toml` del repositorio se define únicamente el linker para el target:
```toml
[target.aarch64-linux-android]
linker = "aarch64-linux-android26-clang"
```
El linker es `aarch64-linux-android26-clang` (coherente con `min_sdk_version = 26` en `crates/pdf_android/Cargo.toml`).

`.cargo/config.toml` **no** define `CC`, `AR` ni las rutas de `bindgen`. Para que el build resuelva el linker y las cabeceras de `mupdf-sys`, se deben exportar las siguientes variables de entorno:

```bash
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/android-ndk-r28}"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
```

- `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` es obligatoria para compilar `mupdf-sys`: bindgen no invoca el wrapper clang del NDK y, sin esta sysroot, intentaría leer cabeceras del host fallando con errores como `pthreadtypes-arch.h`.
- `PATH` debe incluir el directorio de binarios LLVM del NDK para que `aarch64-linux-android26-clang` esté accesible.

### Ficheros de claves API (requisito indispensable de build)
`crates/pdf_android/src/lib.rs` incluye dos claves mediante `include_str!("../groq_key.txt")` y `include_str!("../google_key.txt")`. En un clon limpio o entorno sin claves configuradas, `cargo apk build` o `cargo check -p pdf_android` **fallará** si no existen estos ficheros.

Para compilar, crear los placeholders (igual que hace el CI en `.github/workflows/ci.yml`):
```bash
echo placeholder > crates/pdf_android/groq_key.txt
echo placeholder > crates/pdf_android/google_key.txt
```
*(Nota: ambos ficheros están en `.gitignore` y jamás deben commitearse claves reales).*

## 3. Harness automatizado en la TCL (`tools/adb-bench.sh`)

El procedimiento estándar y reproducible de medición en la tablet es el script `tools/adb-bench.sh` (Fase A3):

```bash
./tools/adb-bench.sh
```

Parámetros opcionales soportados:
```bash
RUNS=5 CORPUS_DIR=corpus ./tools/adb-bench.sh
```

El script ejecuta automáticamente el flujo completo:
1. Comprueba conexión USB con la tablet (`adb devices`).
2. Despierta la pantalla y activa `adb shell svc power stayon true` para evitar throttling por ahorro de energía o modo doze.
3. Compila `pdf_bench` para `aarch64-linux-android` en modo release con las variables de entorno del NDK.
4. Despliega el binario a `/data/local/tmp/pdflector/pdf_bench` y el corpus a `/data/local/tmp/pdflector/corpus/`.
5. Ejecuta el sweep N veces (por defecto 5) con `PDFLECTOR_CORPUS_DIR=/data/local/tmp/pdflector/corpus`.
6. Si la app `com.pdflector.app` está instalada y en ejecución: captura `dumpsys meminfo` (PSS/RSS), toma captura de pantalla (`screencap`) y extrae métricas de frame p95 de logcat.
7. Consolida los resultados en un fichero JSON: `bench-results-TCL-<timestamp>.json`.
8. Restaura `svc power stayon false`.

## 4. Ejecución manual del sweep `pdf_bench` en la tablet

Si se requiere ejecutar únicamente el sweep binario de forma manual:

```bash
# 1. Compilación cruzada
cargo build -p pdf_bench --target aarch64-linux-android --release

# 2. Despliegue de binario y corpus
adb shell mkdir -p /data/local/tmp/pdflector/corpus
adb push target/aarch64-linux-android/release/pdf_bench /data/local/tmp/pdflector/
adb push corpus/*.pdf /data/local/tmp/pdflector/corpus/
adb shell chmod +x /data/local/tmp/pdflector/pdf_bench

# 3. Pantalla activa
adb shell input keyevent KEYCODE_WAKEUP
adb shell svc power stayon true

# 4. Ejecución del sweep
# NOTA: pdf_bench NO lee argumentos de línea de comandos. La ruta del corpus
# se pasa mediante la variable de entorno PDFLECTOR_CORPUS_DIR.
adb shell 'PDFLECTOR_CORPUS_DIR=/data/local/tmp/pdflector/corpus /data/local/tmp/pdflector/pdf_bench'

# 5. Restaurar energía
adb shell svc power stayon false
```

- El sweep reporta: `open`, `render1x`, `render2x` por PDF y `PEAK_RSS_KB` global.
- Metodología: realizar N≥5 corridas para descartar varianza térmica o gobernadores de CPU.

## 5. App en la tablet (`pdf_android`)

Crate `crates/pdf_android` (cdylib, `android-activity` 0.6 con backend `native-activity`, paquete `com.pdflector.app`), empaquetada con `cargo-apk` v0.10.0.

### Compilación e instalación

```bash
# Asegurar placeholders de claves si es necesario
echo placeholder > crates/pdf_android/groq_key.txt
echo placeholder > crates/pdf_android/google_key.txt

# Build e instalación del APK release
cargo apk build --release --target aarch64-linux-android -p pdf_android
adb install -r target/release/apk/pdf_android.apk
```

> **Atención**: las mediciones de memoria solo son válidas en compilaciones **release**. Las versiones debug con debuginfo producen un RSS significativamente superior (~205 MB) debido a símbolos e instrumentación.

### Inyección de PDF de prueba (SELinux)

SELinux impide que una aplicación sin permisos especiales lea directamente de `/data/local/tmp/`. Para inyectar un PDF en el almacenamiento interno de la app (en APKs debuggables):

```bash
adb push corpus/scientific_paper.pdf /data/local/tmp/demo.pdf
adb shell run-as com.pdflector.app sh -c 'cp /data/local/tmp/demo.pdf files/demo.pdf'
```
(`files/` corresponde a `internal_data_path()` en la app).

En compilaciones release se puede abrir el PDF mediante el Intent estándar de Android (`ACTION_VIEW`):
```bash
adb push corpus/scientific_paper.pdf /sdcard/Download/demo.pdf
adb shell am start -a android.intent.action.VIEW -d "file:///sdcard/Download/demo.pdf" -t "application/pdf" com.pdflector.app
```

### Medición de memoria y logs de rendimiento

```bash
# Lanzar la actividad principal
adb shell am start -n com.pdflector.app/android.app.NativeActivity

# Memoria de la app (TOTAL PSS y TOTAL RSS)
adb shell dumpsys meminfo com.pdflector.app

# Tiempos de render y eventos del visor en logcat
adb logcat -d -s pdf_android:V
```

- El tag de logcat es `pdf_android` (manejado por `android_logger`).
- Para monitorear la tasa de cuadros y latencia, buscar líneas con `frame p95=...ms`.

### Verificación visual de renderizado (screencap)

Comprueba que el frame renderizado no presente buffers vacíos o corrupción de color:

```bash
adb exec-out screencap -p > /tmp/screen.png
python3 - <<'EOF'
from PIL import Image
im = Image.open("/tmp/screen.png").convert("RGB")
px = list(im.getdata())
print("mean rgb:", tuple(sum(c[i] for c in px)//len(px) for i in range(3)))
print("white %:", 100*sum(1 for c in px if c[0]>240 and c[1]>240 and c[2]>240)/len(px))
print("red px:", sum(1 for c in px if c[0]>200 and c[1]<80 and c[2]<80))
EOF
```
Valores de referencia esperados: media RGB ~236,236,236 (página sobre letterbox gris), >90 % blanco, 0 píxeles rojos (ausencia de patrones de error o buffers no inicializados).

## 6. Limpieza en el dispositivo

```bash
adb shell rm -rf /data/local/tmp/pdflector
# Opcional: desinstalar la app
# adb uninstall com.pdflector.app
```

## 7. Registro de resultados de rendimiento

- **Destino único**: cualquier resultado obtenido debe incorporarse al final de `docs/benchmark-results.md` (registro append-only organizado por fecha).
- **Formato obligatorio**:
  - **Fecha**: `YYYY-MM-DD`
  - **Hardware**: ej. `TCL NXTPaper 11 Plus (9469X, Android 15, NDK r28, build release)`
  - **Flujo medido**: ej. `Arranque en frío`, `Sweep 5 documentos corpus`, `15 ciclos Library→Viewer`
  - **Métrica**: ej. `PSS = 118 MB`, `render1x p50 = 14.2 ms`, `frame p95 = 7.9 ms`
- Para actualizar el estado o avance de las tareas, editar `docs/plan/NEXT-PLAN.md` o cerrar los issues pertinentes en GitHub Issues.

## 8. Diagnóstico de problemas comunes

- **Error `pthreadtypes-arch.h` al compilar `mupdf-sys`**: falta la variable `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` apuntando al sysroot del NDK.
- **Error "file in wrong format" al linkear**: el ejecutable `aarch64-linux-android26-clang` no está en el `PATH`.
- **Fallo de compilación con `include_str!` de claves**: faltan `crates/pdf_android/groq_key.txt` y `google_key.txt`. Crear placeholders con `echo placeholder > ...`.
- **`run-as: package not debuggable`**: `run-as` solo funciona en compilaciones debug. Para release, colocar el PDF en `/sdcard/Download/` y abrirlo vía Intent o MediaStore.
- **Resultados de latencia anómalos o muy dispersos**: la pantalla se apagó o entró en ahorro de energía. Ejecutar siempre `adb shell svc power stayon true` antes de la prueba.
- **Valores de PSS/RSS disparados (>200 MB en reposo)**: verificar que el APK desplegado sea `target/release/apk/pdf_android.apk` y no una versión de debug.
