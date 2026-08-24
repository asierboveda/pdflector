# Fase C — Pintado/stroke sin latencia (3-4 días, paralelo a B)

> Stroke actual (bolo) tiene latencia por `draw_stroke` por segmento con `sqrt` y `blend_pixel` por píxel del bounding box.

## Auditoría

- `overlay.rs: draw_stroke` itera `w×h` del bbox de cada segmento, con `point_segment_distance` (sqrt) por píxel + blend. Para 200 trazos × 10 segmentos = 2000 boxes.
- `pdf_android/draw.rs: raster_tool_layer` llama `composite_annotations` cada frame sobre `page_frame` (correcto, no re-render MuPDF). Pero `Reader` guarda strokes sin simplificar (todos los puntos del dedo).
- `annotations.rs: smooth_polyline` Catmull-Rom con `segs` subdivide cada trazo (más puntos = más lento).
- Competencia: KOReader usa pressure-free con simplificación Douglas-Peucker; `prime-pdf-viewer` usa Skia path con GPU.

## Objetivo

Dibujar a mano alzada (boli) con feedback <8ms aunque haya 200 trazos en la página.

## Tareas

- [ ] C1. **Simplificar trazo en captura**: Douglas-Peucker (ε=1.5pt) sobre `points` antes de `AnnotationSet::add`. Reduce 100 puntos → 15 sin perder forma.
- [ ] C2. **Cache de strokes rasterizados**: `StrokeCache` por página (clave `page_idx + zoom`, valor `Bitmap` de la capa de tinta). Invalida solo si añades/quitas trazo. `composite_annotations` compone `fill_rect` (highlights) + `blit` de la capa cacheada (1 memcpy), no re-rasteriza 200 strokes por frame.
- [ ] C3. **Fast path durante el gesto**: mientras el dedo baja, dibuja solo el trazo vivo con `draw_segment` sobre `page_frame` (sin recomponer toda la capa). Al soltar, invalida `StrokeCache`.
- [ ] C4. **Medir**: `benches/composite.rs` 200 strokes, `cargo bench -- --quick` p95 <5ms en TCL (Fase A harness).

## Criterio de cierre

- [ ] 200 trazos en una página: scroll <16.6ms p95, pintado vivo <8ms p95 en TCL

## Cómo modificar

- Si quieres boli con alpha variable: añade `pressure` a `Stroke` (ahora `width` fijo).

## Referencias

- `crates/pdf_core/src/overlay.rs`, `annotations.rs`, `pdf_android/src/draw.rs`
