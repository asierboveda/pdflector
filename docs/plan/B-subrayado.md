# Fase B — Subrayado sin latencia + detección de texto (CERRADA)

Subrayado interactivo con detección automática de líneas de texto, soporte para dos columnas y persistencia sin bloqueo del hilo de interfaz.

## Auditoría

- `crates/pdf_core/src/selection.rs`: Algoritmo de rotulador con tolerancia de banda Y (`BAND_TOL = 1.0 pt`).
  - **Nota de diseño (orden de lectura)**: El resaltador evolucionó a orden de lectura mediante pre-ordenación de spans (`sort_spans_by_y`) y búsqueda espacial acelerada (`highlight_under_gesture_sorted`), resolviendo la selección en tiempo $O(\log N + K)$ por búsqueda binaria sin asignación de memoria dinámica y garantizando correcta asignación en documentos a doble columna.
- `crates/pdf_android/src/reader/tools.rs`:
  - `begin_tool_gesture` (:84-92): peek sin I/O a `PageTextCache` para clonar y ordenar spans en memoria en el evento `Down`.
  - `update_tool_gesture` (:201-205): en eventos `Move`, únicamente ejecuta `g.set_cur(pt)` y `mark_repaint()`, sin extraer texto ni realizar I/O en el frame del gesto.
  - `end_tool_gesture`: en el evento `Up`, calcula el conjunto final de rectángulos alineados y persiste la anotación `Highlight` en SQLite de forma asíncrona.

## Objetivo

Gesto del dedo o lápiz → rectángulos de resaltado alineados al texto en < 16 ms p95, sin jank ni bloqueos de presentación, incluso en PDFs complejos a dos columnas.

## Tareas

- [x] B1. **Pre-extraer y cachear `PageText`**: `PageTextCache` LRU en `pdf_core` con prefetch de páginas visibles (±2) al abrir documento. El gesto consume directamente de caché (`get_or_extract`) sin invocar `Document::text()` síncrono durante `Move`.
- [x] B2. **Índice espacial y orden de lectura**: `sort_spans_by_y` + `highlight_under_gesture_sorted` implementado y verificado en `tests/selection.rs`. Tiempos en microbenchmarks: pts100×200líneas 9.82 µs, marquee 122 ns.
- [x] B3. **Feedback visual inmediato en Android**: Preview tentativo en la capa GPU Wet (`render_wet`) alimentado por `ToolGesture.hl_spans`, con cálculo definitivo al soltar (`Up`).
- [x] B4. **Validación en hardware real TCL con stylus**: Medido el 2026-09-05 en TCL NXTPaper 11 Plus (9469X) con stylus USI físico. Tiempos de frame durante el trazo continuo: 1.08–5.33 ms (p50 ~2.8 ms, p95 ~3.5 ms), manteniendo 60–120 fps sostenidos.

## Criterio de cierre

- [x] Subrayado interactivo en hardware real sostenido a 60–120 fps (medido p95 ~3.5 ms en TCL 9469X con stylus físico).
- [x] Precisión de recorte y soporte de doble columna verificado en suite de regresión y en pantalla física.

## Cómo modificar

- Para alternar entre resaltado a mano alzada y selección por caja (marquee): conmutar la variante `Gesture::Points` o `Gesture::Rect`.
- Para añadir alineación exacta a palabra completa: incorporar `word_boxes` en `TextSpan`.

## Referencias

- `crates/pdf_core/src/selection.rs`
- `crates/pdf_android/src/reader/tools.rs`
- `crates/pdf_android/src/input/motion.rs`
- `docs/benchmark-results.md`
