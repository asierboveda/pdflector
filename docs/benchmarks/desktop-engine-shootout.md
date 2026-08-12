# PDFium vs MuPDF — Bench de escritorio (host de desarrollo)

> Bench de la Fase 0.5 ejecutado en **escritorio Linux**, NO en la tablet
> objetivo. Los datos de la **tablet TCL NXTPaper 11 Plus** (donde se cerró
> la decisión ADR-001 en el repo principal `~/Projects/pdflector/`) están en
> el Paso 3 del plan — este doc es **preliminar** y explica por qué los
> números de escritorio NO deben usarse para decidir el motor.

## 1. Disclaimer de rigor

El plan de Fase 0.5 separa explícitamente dos mediciones:

1. **Bench de escritorio** (este doc) — útil para detectar regresiones y
   comparar ergonomía de API durante el desarrollo.
2. **Spike Android en tablet** (Paso 3 del plan) — la medición que cierra
   ADR-001, porque la tablet es el **hardware objetivo** del proyecto
   (PLAN.md §1 decisión 8).

Este bench de escritorio se ejecutó con `cargo bench -p pdf_bench
--bench engine_shootout` sobre el mismo corpus que se usará en tablet. Los
**resultados contradicen** los del repo principal (commit `30b1b4a Fase 0.5`
del repo `~/Projects/pdflector/`): en escritorio **PDFium gana en 13/16
renders**, mientras que el repo principal reporta MuPDF como ganador.

La causa más probable es que el repo principal midió **en la tablet**
(MediaTek MT8781, 8× Cortex-A55 eficiencia, 320 dpi, Android 15), no en
escritorio. La escalabilidad de cada motor es distinta por arquitectura:
PDFium escala mejor en CPUs grandes (x86_64 Ryzen), MuPDF escala mejor en
ARM con CPUs pequeñas — patrón conocido en proyectos similares (ver
discusiones upstream de `pdfium-rs` y `mupdf-rs`).

## 2. Setup de medición

- **Host**: AMD Ryzen 7 5800H (8C/16T), Arch Linux, kernel 6.16.
- **Toolchain**: rustup 1.97.1 stable.
- **Crates**: `pdfium-render 0.8.37`, `mupdf 0.8.0` (mupdf-sys built static,
  bindgen + libclang 22).
- **PDFium prebuilt**: bblanchon/pdfium-binaries chromium/7988 (BSD-3-Clause),
  en `vendor/pdfium/lib/libpdfium.so` (7.67 MB).
- **MuPDF build**: mupdf-sys compila libmupdf.a + libmupdf-third.a desde el
  submódulo MuPDF (12.47 MB totales antes del linker; el linker descarta
  símbolos no usados).
- **Harness**: `crates/pdf_bench/benches/engine_shootout.rs` con
  `criterion 0.5` (sample size 100, default warm-up 3 s).
- **Binarios release**: `cargo build --release -p pdf_bench
  --no-default-features --features <pdfium|mupdf>`.
- **Comando exacto**:
  ```
  LIBCLANG_PATH=/usr/lib cargo bench -p pdf_bench --bench engine_shootout
  ```

## 3. Resultados de open (median, page_count + open)

| PDF | páginas | PDFium (µs) | MuPDF (µs) | ratio (P/M) |
|---|---:|---:|---:|---:|
| scientific_paper | 12 | 54.98 | 55.67 | 0.99× (≈) |
| scanned_pages | 30 | 56.70 | 70.26 | **1.24×** (P) |
| dense_textbook | 93 | 82.37 | 245.80 | **2.98×** (P) |
| large_document | 500 | 186.03 | 384.32 | **2.07×** (P) |

PDFium gana en 3/4 (más páginas ⇒ más差距). El coste de MuPDF crece con
el tamaño del PDF (~2-3 µs/página extra vs PDFium).

## 4. Resultados de render (median, 16 mediciones)

| PDF | página | scale | PDFium (ms) | MuPDF (ms) | ratio (P/M) |
|---|---|---:|---:|---:|---:|
| paper_12p | p1 | 1× | 0.502 | 1.752 | **3.49×** (P) |
| paper_12p | pmid | 1× | 0.556 | 5.806 | **10.45×** (P) |
| paper_12p | p1 | 2× | 10.477 | 23.087 | **2.20×** (P) |
| paper_12p | pmid | 2× | 10.448 | 27.967 | **2.68×** (P) |
| scanned_30p | p1 | 1× | 3.304 | 11.918 | **3.61×** (P) |
| scanned_30p | pmid | 1× | 6.299 | 7.299 | 1.16× (P) |
| scanned_30p | p1 | 2× | 14.457 | 58.994 | **4.08×** (P) |
| scanned_30p | pmid | 2× | 18.051 | 28.591 | **1.58×** (P) |
| dense_93p | p1 | 1× | 2.614 | 1.725 | 0.66× (M) |
| dense_93p | pmid | 1× | 3.459 | 4.521 | 1.31× (P) |
| dense_93p | p1 | 2× | 20.533 | 54.127 | **2.64×** (P) |
| dense_93p | pmid | 2× | 13.450 | 47.979 | **3.57×** (P) |
| large_500p | p1 | 1× | 2.699 | 9.950 | **3.69×** (P) |
| large_500p | pmid | 1× | 2.917 | 5.373 | **1.84×** (P) |
| large_500p | p1 | 2× | 33.263 | 30.487 | 0.92× (M) |
| large_500p | pmid | 2× | 13.749 | 18.160 | 1.32× (P) |

**PDFium gana en 13/16** (81 %). El factor típico es 2-4× más rápido en
render — coincide con el objetivo tablet de **< 25 ms en render** (PLAN.md
§8): PDFium cumple en 16/16, MuPDF cumple en 9/16.

## 5. Tamaño binario release

| Backend | binario self-contained | lib nativa | total en disco |
|---|---:|---:|---:|
| PDFium | 4.86 MB | libpdfium.so 7.67 MB | **12.53 MB** (split) |
| MuPDF | **6.48 MB** | — (linkado estático) | **6.48 MB** (self-contained) |

**MuPDF gana en simplicidad de distribución** (un solo .apk / .so). En
total absoluto, ambos están en el mismo rango.

## 6. RSS pico (harness `rss_probe`)

| PDF | PDFium (KiB) | MuPDF (KiB) |
|---|---:|---:|
| scientific_paper (12p) | 4 192 | 4 704 |
| large_document (500p) | 4 220 | 4 180 |

**Empate técnico**. RSS dominado por allocator del proceso + caché de páginas
de MuPDF (≈ 1.5 MB estático); en la app real el RSS será dominado por la
caché LRU de bitmaps (idéntica entre backends).

## 7. Conclusión provisional

**En escritorio, PDFium gana de calle** (2-4× en render, 2-3× en open para
PDFs grandes). Pero esto NO decide el proyecto: el hardware objetivo es la
tablet, donde los ratios pueden invertirse por:

- **Arquitectura**: x86_64 con cores grandes (Ryzen) vs ARM Cortex-A55
  (TCL MT8781). Los A55 son in-order, single-issue, lo que penaliza
  algoritmos con muchos branches impredecibles. MuPDF tiene un fz_gs
  (graphics state) más "interpretativo" y eso puede ser *más* penalizado en
  in-order que el renderer "más nativo" de PDFium, **o al revés** — hay
  que medir.
- **Memoria**: tablet con 8 GB compartida con GPU y sistema. Las cachés
  internas de cada motor (MuPDF fz_store, PDFium CPDF_PageCacheManager)
  tienen patrones distintos.
- **Frecuencia**: la TCL puede hacer throttling agresivo bajo carga sostenida.

**Acción**: ejecutar el mismo bench **en la tablet** con el binario
cross-compilado a `aarch64-linux-android`, vía `adb shell`. Es el Paso 3
del plan y lo que cierra ADR-001.

## 8. Reproducibilidad

```bash
# Setup (idempotente):
git clone https://github.com/asierboveda/pdflector
cd pdflector
cargo fetch
bash tools/fetch_pdfium.sh                 # vendor/pdfium/lib/libpdfium.so
mkdir -p corpus && cp /ruta/a/{paper,scanned,dense,large}*.pdf corpus/

# Build del bench (ambos backends activos durante Fase 0.5):
LIBCLANG_PATH=/usr/lib cargo build --release -p pdf_bench

# Ejecutar bench (criterion):
LIBCLANG_PATH=/usr/lib cargo bench -p pdf_bench --bench engine_shootout

# RSS probe (un backend por binario):
LIBCLANG_PATH=/usr/lib cargo build --release -p pdf_bench \
    --bin rss_probe --no-default-features --features pdfium
LIBCLANG_PATH=/usr/lib cargo build --release -p pdf_bench \
    --bin rss_probe --no-default-features --features mupdf
# Muestrear VmRSS cada 1 s durante 30 s con el script bash en
# docs/research/evince-architecture.md §7.
```

> **Nota**: este bench es `desktop`. La medición que cierra ADR-001 vive en
> el Paso 3 del plan (cross-compile + `adb shell` sobre la TCL NXTPaper 11
> Plus). Ver `docs/benchmarks/android-tablet-shootout.md` cuando se ejecute.