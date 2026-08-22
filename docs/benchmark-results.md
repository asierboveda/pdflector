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
- Android cross (ver memory): PDFium 1 comando / 22 s; MuPDF 17 s + 1 env var
  (`BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`) — ambos validados en la Fase 0.5.

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

## Android — TCL NXTPaper 11 Plus (modelo 9469X, MT8781 8× A55, Android 15, pantalla 1440×2200, medición con pantalla ON)

Hardware: TCL NXTPaper 11 Plus (modelo 9469X, MediaTek MT8781 8× Cortex-A55
solo eficiencia sin big cores, 8 GB RAM, pantalla 1440×2200 @ 320 dpi,
Android 15 / SDK 35, ABI arm64-v8a). Medición con pantalla ON
(KEYCODE_WAKEUP + `svc power stayon true`, limpiado después).

| PDF (páginas) | open (ms) | render 1x (ms) | render 2x (ms) |
|---|---|---|---|
| dense (93) | 0.40 | 14.51 | 44.18 |
| scanned (30) | 0.15 | 31.34 | 119.01 |
| paper (12) | 0.16 | 11.64 | 38.44 |
| large (500) | 0.25 | 15.40 | 44.73 |

PEAK_RSS_KB = 26688 (~26,7 MB)

### render 1x MISMO ESCALA: Tablet TCL vs Xiaomi phone vs Desktop (MuPDF)

| PDF (páginas) | render 1x TCL (ms) | render 1x Xiaomi (ms) | render 1x Desktop (ms) | Ratio TCL/Desktop |
|---|---|---|---|---|
| dense (93) | 14.51 | 6.28 | 3.53 | 4.11× |
| scanned (30) | 31.34 | 15.97 | 8.93 | 3.51× |
| paper (12) | 11.64 | 3.88 | 2.18 | 5.34× |
| large (500) | 15.40 | 7.48 | 3.98 | 3.87× |

Nota HONESTA: a la misma escala render 1x, la TCL es ~2,3× **más lenta** que el
Xiaomi phone (no más rápida). Razón: el Xiaomi 2412DPC0AG tiene big cores
(Cortex-A78/A715-class), mientras el MT8781 de la TCL tiene 8× Cortex-A55 solo
eficiencia — tablet enfocada a lectura, no a rendimiento. El desktop
(AMD Ryzen 7 5800H, Fase 0.5) es 3,5-5,3× más rápido que la TCL (ratios
calculados sobre los datos de la Fase 0.5). Ojo metodológico: el Xiaomi se
midió con pantalla OFF — posiblemente pesimista para Xiaomi (governor/doze a
pantalla apagada puede reducir frecuencias); la TCL se midió con pantalla ON.

### fps estimados (1000 / render 1x) — TCL

| PDF (páginas) | render 1x (ms) | fps estimado | ¿cumple 60 fps? |
|---|---|---|---|
| dense (93) | 14.51 | 69 | ✓ |
| scanned (30) | 31.34 | 32 | ✗ (worst case raster) |
| paper (12) | 11.64 | 86 | ✓ |
| large (500) | 15.40 | 65 | ✓ |

Cumple 60 fps en 3/4 PDFs; único fallo: scanned (PDF raster).

### Aceptación Fase 1 (PLAN.md)

- render < 25 ms → **3/4 cumplen** ✓ (dense 14.5, paper 11.6, large 15.4; solo
  scanned 31 ms lo excede — worst case raster esperable).
- RSS < 150 MB → **26,7 MB** ✓ con ~6× de margen.
- Conclusión: la tablet cumple para PDFs vectoriales (la mayoría); scanned y
  zoom 2x requieren optimización futura (B3 zoom / tile-render cache).

### Notas de método

- Build: MuPDF release cross-compilado a `aarch64-linux-android` (NDK r28 +
  `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=...`), pdf_core con
  los módulos B1/B2 (cache/scroll/prefetch) incluidos.
- Despliegue: `adb push` a `/data/local/tmp/pdflector/`; env
  `PDFLECTOR_CORPUS_DIR` apuntando al corpus en el dispositivo.
- Métrica: mediana de 3 intentos del propio sweep de `pdf_bench`; 2 corridas
  estables (difieren <5%).
- Pantalla ON durante la medición: `input keyevent KEYCODE_WAKEUP` + `svc power
  stayon true` antes de la prueba, `svc power stayon false` después — evita el
  pesimismo de governor/doze.

## Fase 1 / B3 — Zoom: escala software vs re-render (large_document.pdf pág. 0, desktop)

Benchmark: `crates/pdf_bench/benches/zoom.rs` (criterion, grupo `zoom`).
Hardware: AMD Ryzen 7 5800H, release bench, criterion `--quick` (2026-08-13).

| Ruta | Escenario | Tiempo |
|---|---|---|
| scale_bitmap (software) | z1.5 (→ nivel 1) | 31,2–32,2 ms |
| scale_bitmap (software) | z2 (→ nivel 1) | 55,9 ms |
| scale_bitmap (software) | z4 (→ nivel 2) | 214,5–219,9 ms |
| re-render nítido (MuPDF) | nivel 1 (×2) | 3,3–3,4 ms |
| re-render nítido (MuPDF) | nivel 2 (×4) | 11,9–12,5 ms |
| trim_to_scale_level | soltar nivel 0, conservar nivel 1 | 6,3–6,5 ms |

### Hallazgo honesto

- El escalador software `scale_bitmap` NO es un camino "rápido" en CPU: a tamaño
  de página completa es **~16–18× más lento que el re-render nativo de MuPDF**
  (55,9 ms vs 3,4 ms a ×2; 215 ms vs 12 ms a ×4). Es correcto y determinista,
  pero el escalado naïve por píxel en Rust sin SIMD es lento.
- Consecuencia de diseño: el camino "inmediato" del zoom en `pdf_app` usa el
  **reescalado de textura por GPU** (egui, ~gratis), NO `scale_bitmap`.
  `scale_bitmap` queda como utilidad pura y testeable para contextos headless
  (p. ej. harness Android sin GPU), documentada como tal; si algún día hace
  falta escalado software rápido, la optimización es SIMD/tiling (fuera de
  scope actual).
- El re-render nítido (MuPDF) es barato (3,4 ms ×2; 12 ms ×4), dentro del
  presupuesto de 60 fps; por eso el diseño "mostrar borroso en GPU + re-render
  nítido async" es el correcto.

## Fase 1 / B3 — Zoom en tablet TCL NXTPaper 11 Plus (Ola 8, 2026-08-13)

Hardware: TCL NXTPaper 11 Plus (9469X, MT8781 8× Cortex-A55, Android 15,
pantalla ON, batería 66% cargando a 33 °C). Binario `aarch64-linux-android`
release (NDK r28). Mediana de 3, 2 corridas.

| Caso | scale_bitmap (fast) | re-render (sharp) | Ratio |
|---|---|---|---|
| large p0 → 2x | 69,4–70,2 ms | 14,9–16,6 ms | ~4,5× |
| large p0 → 4x | 275,8–281,5 ms | 53,2–56,1 ms | ~5,2× |
| dense p0 → 2x | 69,9 ms | 16,1–16,9 ms | ~4,3× |
| dense p0 → 4x | 321,4–325,1 ms | 57,3–59,4 ms | ~5,6× |

### Hallazgo (confirma el escritorio, amplificado)

- En la tablet, `scale_bitmap` (software) es **~4–5,6× más lento que el re-render
  nítido** y muy por encima del presupuesto de 16,6 ms (70 ms a 2x; ~280–325 ms a
  4x). El escalado software naïve por píxel (sin SIMD/NEON) no es un "fast path"
  viable: el upscale es el camino CARO y el re-render MuPDF el barato.
- Consecuencia: en `pdf_app` el camino inmediato es el reescalado de textura por
  GPU (ya implementado). `scale_bitmap` queda solo para contextos headless y
  requiere optimización (SIMD/NEON, aritmética entera) o replanteo antes de
  cualquier uso en UI.

### Nota sobre la comparación con Ola 7 (para no malinterpretar)

- El sweep render1x/render2x de Ola 8 da valores más altos que Ola 7 en algunos
  PDFs, PERO el path de render (`mupdf.rs`) no cambió. Dos causas lo explican:
  1. **Confound del corpus**: entre Ola 7 y Ola 8 se corrigió el bug de
     `tools/generate_corpus.py` (scanned_pages.pdf ahora embebe 30 imágenes
     DISTINTAS en vez de 30 referencias a la misma). Render 3 páginas de scanned
     ahora decodifica 3 imágenes distintas (antes 1 + 2 cache hits), lo que
     explica el aumento de render de scanned (+65–104%) y del RSS (~+5 MB:
     3 pixmaps decodificados ~2,2 MB c/u en la caché de imágenes de MuPDF frente
     a 1).
  2. **Varianza termal/governor**: dense/paper muestran saltos no reproducibles
     entre corridas (p. ej. scanned render2x 128→186 ms, +45%); la tablet estaba
     cargando a 33 °C. No es regresión de código.
- Conclusión: no hay evidencia de regresión de render por el código de B3; el RSS
  estable se explica por el corpus corregido (más realista). Para una comparación
  limpia haría falta fijar governor y repetir N≥5 corridas.


## Auditoría del pipeline de render + optimizaciones blit/prefetch (2026-08-22)

Hardware: AMD Ryzen 7 5800H (16 hilos), Rust 1.97.1, release. Carga ambiental
del host ALTA durante toda la sesión (IDE + agentes en paralelo, load avg ~5–7;
otras sesiones midiendo a la vez): las rutas de código SIN tocar muestran
±10–20 % de varianza run-to-run; las optimizadas ganan 31–89 %, muy por encima
del ruido. **El corpus fue regenerado el 2026-08-22 18:24** (ficheros más
pesados que el histórico), así que los absolutos de este día NO son comparables
1:1 con los de 2026-08-05/13: las comparaciones de esta sección son dentro del
mismo día (baseline vs optimizado, misma sesión).

### 1) Baseline del inventario (criterion, corpus 2026-08-22) — antes de optimizar

| Bench (grupo) | Caso | Mediana |
|---|---|---|
| open_render/open | dense / scanned / paper / large | 42,8 / 39,0 / 38,2 / 66,8 µs |
| open_render/render_1x | dense / large | 4,11 / 6,81 ms |
| open_render/render_2x | dense / large | 6,65 / 9,01 ms |
| render_perf/render | dense p1·1x / large p1·1x | 1,89 / 5,24 ms |
| cache_scroll | naive_hold / firstpass / pass2 | 131,9 / 95,9 ms / 433 µs |
| zoom | scale z1.5 / z2 / z4 · rerender l1 / l2 · trim | 32,3 / 57,7 / 228,9 · 3,89 / 12,79 · 7,93 ms |
| annotations | add n100/n1000 · for_page 200 · to/n1000 · from/n1000 · store n1000 | 6,5/62,4 µs · 81 ns · 285,7 µs · 444,6 µs · 9,02 ms |

Sweep de humo (`cargo run --release -p pdf_bench`, corpus regenerado):
dense render1x 4,45 ms · scanned 10,30 · paper 6,29 · large 4,61 ms; render2x
11,0 / 45,1 / 14,6 / 13,5 ms; PEAK_RSS 30 072 KB. (El "open" del sweep, 6,8–11
ms, incluye el warmup perezoso de MuPDF del primer open del proceso; el open
puro del criterion es 38–67 µs.)

### 2) Bench nuevo: camino de blit por frame (crates/pdf_bench/benches/blit.rs)

Espejo fiel de las primitivas CPU de `pdf_android/src/draw.rs` (`fill_buffer`,
`copy_region`, `rgb565`, `blit_page_scaled`, `fill_rect_lut`,
`draw_sel_rect`, `compose_frame`, `blit_composed`) — pdf_android no compila en
host (android-activity), así que el espejo vive en pdf_bench y **debe
mantenerse en sync** con draw.rs. Ventana 2000×1200 (landscape típico de
tablet), contenido real de `large_document.pdf` pág. 0 a escala cover
(849×1200 px, 4 MiB), bpp 4 y 2, zoom 1.0 (reposo) y 1.35 (frame de pinch sin
re-render), composición de frame completo (sheet) y su copia por frame.

| Ruta (2000×1200, mediana) | Baseline | Optimizada | Δ |
|---|---|---|---|
| blit/page_1to1_bpp4_light | 332 µs | 368 µs (≈392 el run final) | ~0 (no tocada; ruido) |
| blit/page_1to1_bpp4_dark | 1,376 ms | 154 µs | **−89 %** |
| blit/page_1to1_bpp2 (RGB565) | 1,153 ms | 1,167 ms | ~0 (no tocada) |
| blit/page_zoom135_bpp4_light | 1,292 ms | 889 µs (727 final) | **−31/−44 %** |
| blit/page_zoom135_bpp4_dark | 2,391 ms | 731 µs (556 final) | **−69/−77 %** |
| blit/fill_buffer | 193 µs | 189 µs | ~0 (no tocada) |
| blit/compose_frame_2k1k | 1,161 ms | 1,363–1,396 ms | ~0 (no tocada; ruido +10–20 %) |
| blit/blit_composed_2k1k | 796 µs | 841–876 µs | ~0 (no tocada; ruido) |

### 3) Optimizaciones aplicadas (cambios mínimos, medidos antes/después)

**a) Inversión de color en dark mode (bpp 4, camino 1:1 y zoom) en
`pdf_android/src/draw.rs::blit_page_scaled`** y su gemelo
`pdf_android/src/zoom.rs::blit_scaled_nearest` (camino de pinch de la
Biblioteca), más el espejo del bench:
el bucle por bytes (`255 − v` por canal) pasa a **XOR de u32 con
`0x00FF_FFFF`**: invierte R/G/B y preserva el alfa byte a byte — la MISMA
transformación que `pdf_core::dark::invert_bitmap`, sin cambio de resultado
(píxel a píxel idéntico). El camino por bytes no auto-vectorizaba
(1,38 ms → 154 µs a pantalla completa).

**b) Ruta de zoom (vecino-más-cercano, bpp 4):** accesos por u32 directos con
la `x_map` precalculada, sin bounds-check de slice por píxel (antes
`src_row[x_map[x]*4..]` por píxel). `x_map[x] ∈ [0, src_w)` está garantizado
por construcción (división entera truncada con `dst_rel < dw`), por lo que las
lecturas crudas son seguras (comentado). 1,29 ms → 889 µs en pinch light;
2,39 ms → 731 µs en pinch dark. `unsafe` acotado y comentado (AGENTS §3).

**c) Prefetch efectivo (`pdf_core/src/prefetch.rs`):** el worker ahora
**preempciona** una wishlist stale cuando llega una `Request` nueva: abandona
la lista antigua en la frontera de página y empieza la nueva (las páginas
visibles primero). Antes el worker molía TODAS las wishlists encoladas
(contrato B2 "in-flight no se cancela"); ahora la rafaga de scroll solo
renderiza la última ventana + las páginas en vuelo. Contabilidad de
`requested/completed` preservada: cada Request recibido libera exactamente un
waiter (al ser preempido o al terminar), así que `await_idle_timeout` sigue
siendo correcto. Doc del módulo y 2 tests de regresión actualizados al nuevo
contrato (se conserva la garantía que protegían: `await_idle == true ⟹ la
reissue ya está renderizada`); test nuevo `newer_request_preempts_stale_wishlist`.

Medición (`pdf_bench/benches/prefetch.rs`, burst de 10 viewports no solapados
de 11 páginas c/u, ráfaga encolada back-to-back):

| Métrica | Contrafactual sin preempeón | Con preempeón | Δ |
|---|---|---|---|
| Páginas renderizadas en la ráfaga | 110 | **22** | **−80 %** |
| Tiempo hasta residente el viewport final | — | 35,8 ms | — |

(Unidad de test: wishlist stale de ~400 págs + request pequeña → 400 → 1–3
renders.)

**d) Descartada y documentada:** `cache.rs` — eliminar el segundo lookup del
camino de hit (`get_or_render` hace 2 `map.get` por hit). Medido con
micro-harness: **10,8 ns/hit** (lookup + promoción LRU); ahorrar un lookup son
~11 ns y queda muy por debajo del umbral del 5 %; además borrowck lo obligaría
a un refactor invasivo del camino de miss. Queda la estructura original.

**e) No optimizada (documentada):** rutas bpp 2 (RGB565): el visor fuerza
R8G8B8A8_UNORM, no es el camino real. `scale_bitmap` (zoom software) sigue
siendo lento (57,7 ms a ×2) — decisión ya tomada en B3: el camino inmediato
del zoom es la textura por GPU.

### 4) Verificación (2026-08-22)

- `cargo test -p pdf_core` (32+21+5+8+7+9 tests): **OK**, incluidos los 9 de
  prefetch (3 corridas estables) y los de cache/zoom/scroll.
- `cargo clippy --all-targets -- -D warnings`: **limpio**.
- `cargo fmt --all -- --check`: **limpio**.
- Cross Android: `cargo build -p pdf_android --target aarch64-linux-android --release`
  (CARGO_TARGET_DIR=/tmp/cargo-tgt-perf, NDK r28, API keys placeholder
  gitignored) **OK 0 warnings**; `cargo build -p pdf_app` (host) **OK**.
- Benches sin regresión: open/render/zoom/annotations/cache_scroll dentro del
  ruido de carga (±10–20 %, varios mejoran por caída de load); blit: las rutas
  tocadas mejoran 31–89 %, las no tocadas quedan dentro del ruido.

### Notas de método

- Los números de blit/prefetch son de escritorio Ryzen 7 5800H (host): en la
  tablet TCL (A55, ~3–4× más lento por núcleo) los tiempos absolutos serán
  mayores, pero las ganancias relativas de los caminos tocados se transfieren
  (aritmética por u32 y XOR son independientes de la arquitectura; la
  medición en tablet queda pendiente, Fase 6).
- El espejo del bench (blit.rs) y draw.rs/zoom.rs deben evolucionar juntos:
  cualquier cambio de las primitivas de blit se aplica en los tres sitios y se
  re-mide con `cargo bench -p pdf_bench --bench blit`.
