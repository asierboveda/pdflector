# Resultados del benchmark — Fase 0.5 (2026-08-05)

Hardware: AMD Ryzen 7 5800H, 16 hilos. Rust 1.97.1, release build
(`cargo run --release -p pdf_bench`).

## Tabla comparativa

| PDF (páginas) | Motor | open (ms) | render 1x (ms) | render 2x (ms) | RSS pico (KB) |
|---|---|---|---|---|---|
| dense (93) | PDFium | 0.17 | 9.69 | 35.34 | 32520 |
| dense (93) | MuPDF | 0.11 | 3.53 | 8.51 | 25572 |
| scanned (30) | PDFium | 0.09 | 20.01 | 66.20 | 32520 |
| scanned (30) | MuPDF | 0.07 | 8.93 | 35.38 | 25572 |
| paper (12) | PDFium | 0.08 | 1.72 | 26.44 | 32520 |
| paper (12) | MuPDF | 0.07 | 2.18 | 6.95 | 25572 |
| large (500) | PDFium | 0.21 | 6.86 | 35.10 | 32520 |
| large (500) | MuPDF | 0.09 | 3.98 | 10.19 | 25572 |

## Notas
- Métricas: mediana de 3 runs (páginas 0/mitad/última) para render; open una vez.
- RSS pico: VmHWM de /proc/self/status (cada motor en proceso separado).
- Build: PDFium host ~0.5 s (lib precompilada `vendor/pdfium/lib/libpdfium.so`,
  solo compilación Rust), MuPDF host 29.96 s la 1ª vez (C de `mupdf-sys` 0.8.0).
- Android cross (ver memory): PDFium 1 comando; MuPDF pendiente de C2.

## Android (Xiaomi 2412DPC0AG, arm64-v8a, Android 16, 8 cores, 7,5 GB RAM)

Hardware: Xiaomi 2412DPC0AG (arm64-v8a, Android 16 / SDK 36, 8 cores,
MemTotal 7.483.884 kB). Build: MuPDF release cross-compilado a
`aarch64-linux-android` (NDK r28 + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`),
binario subido con `adb push` a `/data/local/tmp/pdflector/pdf_bench`, corpus en
`/data/local/tmp/pdflector/corpus/`, barrido con `PDFLECTOR_CORPUS_DIR` definido.

| PDF (páginas) | open (ms) | render 1x (ms) | render 2x (ms) |
|---|---|---|---|
| dense (93) | 0.42 | 6.28 | 37.14 |
| scanned (30) | 0.13 | 15.97 | 84.33 |
| paper (12) | 0.27 | 3.88 | 35.79 |
| large (500) | 0.27 | 7.48 | 36.98 |

PEAK_RSS_KB = 31220 (~30,5 MB)

### render 1x: Android vs Desktop (MuPDF, ambos)

| PDF (páginas) | render 1x Android (ms) | render 1x Desktop (ms) | Ratio (Android/Desktop) |
|---|---|---|---|
| dense (93) | 6.28 | 3.53 | 1.78× |
| scanned (30) | 15.97 | 8.93 | 1.79× |
| paper (12) | 3.88 | 2.18 | 1.78× |
| large (500) | 7.48 | 3.98 | 1.88× |

### Notas
- Método: mediana de 3 intentos, barrido a escala 1x/2x con `pdf_bench`
  cross-compilado a `aarch64-linux-android` (MuPDF release), `adb push` a
  `/data/local/tmp/pdflector/`, env `PDFLECTOR_CORPUS_DIR` apuntando al corpus.
- render 1x 3,88–15,97 ms: 3 de 4 PDFs superan 120 fps y todos mantienen ≥60 fps
  (frame <16,6 ms). A 2x cae a 12–28 fps (scanned 84,33 ms).
- scanned (raster) es el peor caso (15,97 ms 1x / 84,33 ms 2x) → candidato a
  optimización futura del render de bitmaps.
- PEAK_RSS 30,5 MB frente al objetivo <150 MB → margen ~5×; `large` (500 p) no
  eleva el RSS (carga perezosa / caché por bytes).

## Fase 1 / B1 — Caché LRU vs naive (large_document.pdf, 50 pág, escala 1x)

Benchmark: `crates/pdf_bench/benches/cache_scroll.rs` (criterion, grupo
`cache_scroll`).

| Escenario | Tiempo (ms) | VMHWM_KB |
|---|---|---|
| naive_hold_50p_1x | 108.02 | 107412 |
| cache_8mb_firstpass_50p_1x | 74.78 | 21104 |
| cache_8mb_pass2_50p_1x | 0.35 | 21184 |

**Reducción RAM pico: 5× (105 MB → 20,6 MB). Cumple objetivo <150 MB con margen.**

### Notas
- Método: mediana criterion (warm-up 500 ms, sample_size 15, medición 3 s),
  `cache_scroll` bench de `cargo bench -p pdf_bench`, 50 páginas de
  `large_document.pdf` a escala 1x (72 dpi) con MuPDF release.
- VMHWM (RSS pico) de `/proc/self/status`, medido en un **proceso hijo
  separado** por escenario (el pico del kernel es monotónico y lo contaminaría
  el escenario naive).
- Hardware: escritorio AMD Ryzen 7 5800H (16 hilos), release build.
- Hallazgo: en 8 MB caben 4 páginas de large_document a 1x (cada una ~2 MB);
  `current_bytes <= byte_budget` se cumple en todas las iteraciones.
- Honestidad: el escenario pass2 recorre solo las páginas **residentes** en la
  caché de 8 MB (4 de 50); "todo hits en 50 páginas" es matemáticamente imposible
  con una caché byte-limitada menor que el barrido. Mide el coste puro del hit path.

