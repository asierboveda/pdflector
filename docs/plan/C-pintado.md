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

- [x] C1. **Simplificar trazo en captura**: `pdf_core::simplify_polyline` (Douglas-Peucker iterativo) + integración ink (solo trazos ≥40 pts, ε=0.8 pt para conservar la forma manuscrita).
- [ ] C2. **Cache de strokes rasterizados**: `StrokeCache` por página (clave `page_idx + zoom`, valor `Bitmap` de la capa de tinta). Invalida solo si añades/quitas trazo. `composite_annotations` compone `fill_rect` (highlights) + `blit` de la capa cacheada (1 memcpy), no re-rasteriza 200 strokes por frame.
- [x] C3. **Fast path durante el gesto**: `tool_overlay` + `copy_region_blend` por Move. **BUG CRÍTICO corregido (2026-08-24)**: `raster_tool_layer` usaba `composite_annotations` (que no escribe alpha — diseñado para el bitmap opaco de página) → el trazo en curso tenía alpha=0 → `copy_region_blend` lo saltaba → **boli invisible en tiempo real** (confirmado con screencap: 0 px durante el gesto). Fix: `pdf_core::overlay::composite_annotations_alpha` (escribe alpha = cobertura × color.a). Verificado en TCL: trazo visible creciendo (928 px a mitad en zona limpia, 2205 px al soltar, bbox exacto al dedo).
- [ ] C4. **Medir**: `benches/composite.rs` 200 strokes, `cargo bench -- --quick` p95 <5ms en TCL (Fase A harness).

## Criterio de cierre

- [ ] 200 trazos en una página: scroll <16.6ms p95, pintado vivo <8ms p95 en TCL

## Cómo modificar

- Si quieres boli con alpha variable: añade `pressure` a `Stroke` (ahora `width` fijo).

## Referencias

- `crates/pdf_core/src/overlay.rs`, `annotations.rs`, `pdf_android/src/draw.rs`
