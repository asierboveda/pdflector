# Fase B — Subrayado sin latencia + detección de texto (3-4 días)

> Tu dolor: "tienen latencia, debe funcionar más veloz y detectando el texto cuando subraya". Auditado: `highlight_under_gesture` es O(spans×puntos) y `Document::text()` se llama en el hilo UI durante el gesto (stext de MuPDF no es gratis).

## Auditoría

- `selection.rs`: algoritmo correcto (rotulador real, 2 columnas, BAND_TOL 1pt) pero sin índice espacial. Para 200 líneas × 100 puntos del gesto = 20k checks por frame.
- `engine/mupdf.rs: text()` hace `load_page + to_text_page + structured()` síncrono. En TCL puede ser 5-10ms por página (no medido).
- `pdf_android/input.rs` convierte gesto pantalla→página y llama `highlight_under_gesture` en el evento Move (cada frame).
- Competencia: Xodo/Adobe pre-extraen texto al abrir y usan R-tree; `mupdf-android-viewer` cachea `fz_stext_page` en `fz_store`.

## Objetivo

Gesto del dedo/lápiz → rects amarillas alineadas al texto en <16ms, sin jank, aunque el PDF tenga 2 columnas.

## Tareas

- [ ] B1. **Pre-extraer y cachear `PageText`**: `PageTextCache` LRU en `pdf_core` (clave `page_idx`, valor `Vec<TextSpan>`), llenado en hilo fondo al abrir PDF (`prefetch.rs` ya tiene el patrón actor). Gesto lee de caché, nunca llama `Document::text()` en UI.
- [ ] B2. **Índice espacial**: para gesto `Points`, filtra spans por banda Y con `span.y ± BAND_TOL` usando `Vec` ordenado por Y (binary search), no scan lineal. Objetivo: <1ms para 200 líneas.
- [ ] B3. **Feedback visual inmediato**: en `Move`, pinta rects tentativos con `overlay::fill_rect` sobre `page_frame` cacheado (sin `save_annotations` hasta `Up`). `save` solo en `Up`.
- [ ] B4. **Test TCL**: 100 gestos aleatorios en `paper` 2 columnas, medir p95 `highlight_under_gesture` con bench + `adb` (Fase A). Debe ser <5ms p95.

## Criterio de cierre

- [ ] Subrayar 50 líneas en TCL no baja de 60fps (p95 <16.6ms, `FrameTimer` de Fase A)
- [ ] 2 columnas: subrayar izq no marca dcha (test ya existe, validar en TCL con screencap)

## Cómo modificar

- Si quieres subrayado solo con rect (marquee) y no rotulador: simplifica `Gesture::Points` a `Gesture::Rect`.
- Si quieres snapping a palabra: añade `word_boxes` a `TextSpan`.

## Referencias

- `crates/pdf_core/src/selection.rs`, `engine/mupdf.rs: text()`, `pdf_android/src/input.rs`
- `docs/research/annotations-selection.md`
