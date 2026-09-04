# Fase B — Subrayado sin latencia + detección de texto (3-4 días)

> Tu dolor: "tienen latencia, debe funcionar más veloz y detectando el texto cuando subraya". Auditado: `highlight_under_gesture` es O(spans×puntos) y `Document::text()` se llama en el hilo UI durante el gesto (stext de MuPDF no es gratis).

## Auditoría

- `selection.rs`: algoritmo correcto (rotulador real, 2 columnas, BAND_TOL 1pt). **B2 hecho 2026-09-04**: camino indexado `highlight_under_gesture_sorted` (banda Y por `partition_point` + paseo atrás sobre spans altos, `O(log N + K)`, cero alocación extra) con equivalencia exacta vs oráculo lineal (tests `tests/selection.rs`).
- `engine/mupdf.rs: text()` hace `load_page + to_text_page + structured()` síncrono. **B1 hecho (verificado en árbol)**: `PageTextCache` LRU + `prefetch` (±2 al abrir, `reader.rs`) + `get_or_extract` en el gesto (hit sin stext); el resaltado se calcula al soltar (`Up`) desde spans cacheados, no hay `text()` en el frame de `Move`.
- `pdf_android/input.rs` convierte gesto pantalla→página; el resaltado final corre en `Up` (ver `reader.rs` `end_tool_gesture`).
- Competencia: Xodo/Adobe pre-extraen texto al abrir y usan R-tree; `mupdf-android-viewer` cachea `fz_stext_page` en `fz_store`.

## Objetivo

Gesto del dedo/lápiz → rects amarillas alineadas al texto en <16ms, sin jank, aunque el PDF tenga 2 columnas.

## Tareas

- [x] B1. **Pre-extraer y cachear `PageText`**: `PageTextCache` LRU en `pdf_core` + `prefetch` visible ±2 al abrir (`pdf_android/reader.rs`). Gesto lee de caché (`get_or_extract`); sin `Document::text()` en el frame del gesto.
- [x] B2. **Índice espacial**: `sort_spans_by_y` (una vez por extracción) + `highlight_under_gesture_sorted` (banda Y por búsqueda binaria std, sin deps). Host 2026-09-04 x86_64 `--quick`: pts100×200líneas 28.07µs→9.82µs; marquee 260ns→122ns (ambos ≪1ms).
- [x] B3. **Feedback visual inmediato + camino indexado en Android** (2026-09-04): `ToolGesture.hl_spans` (spans pre-ordenados UNA vez en el `Down` por peek sin I/O) → preview tentativo alineado al texto por present en la capa wet (`render_wet`) + cálculo final al soltar por `highlight_under_gesture_sorted` (fallback a la vía clásica si la página no estaba cacheada). `Move` solo `set_cur` + `mark_repaint` (coalescado por vsync); `save` sigue solo en `Up`. Sin regresión en TCL (páginas OK, 0 FATAL/panic). Nota: solo el stylus dibuja (dedo = navegar); el E2E con lápiz queda en B4.
- [ ] B4. **Test TCL con stylus** (bloqueado a lápiz físico; `adb` inyecta dedo=tap/pan, nunca `ToolDrawing`): ① abrir `scientific_paper.pdf` (2 cols, en `/sdcard/Download`), ② activar el resaltador con el botón lateral del lápiz (sin barra en la app: el modo persiste en `tool_state.json`), ③ subrayar 3 líneas de la columna IZQUIERDA con el lápiz, ④ `screencap`: rects amarillos solo en la izquierda + `logcat -s pdf_android:V | grep highlighted`, ⑤ 10 trazos seguidos y `frame p95=` en logcat (<16.6ms objetivo con gesto continuo). Debe ser <5ms p95.

## Criterio de cierre

- [ ] Subrayar 50 líneas en TCL no baja de 60fps (p95 <16.6ms, `FrameTimer` de Fase A)
- [ ] 2 columnas: subrayar izq no marca dcha (test ya existe, validar en TCL con screencap)

## Cómo modificar

- Si quieres subrayado solo con rect (marquee) y no rotulador: simplifica `Gesture::Points` a `Gesture::Rect`.
- Si quieres snapping a palabra: añade `word_boxes` a `TextSpan`.

## Referencias

- `crates/pdf_core/src/selection.rs`, `engine/mupdf.rs: text()`, `pdf_android/src/input.rs`
- `docs/research/annotations-selection.md`
