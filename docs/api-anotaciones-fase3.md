# API de anotaciones — resaltador con detección de texto + boli (Fase 3.5)

Fecha: 2026-08-22 — Módulos nuevos: `pdf_core::selection`, `pdf_core::overlay`;
suavizado añadido a `pdf_core::annotations`.

Todo el código es lógica pura en `pdf_core` (sin UI), en coordenadas de página
(PDF points, origen top-left, y crece hacia abajo).

## 1. Resaltador con detección de texto — `crates/pdf_core/src/selection.rs`

```rust
pub struct Gesture { ... }                     // enum: Points(Vec<(f32,f32)>) | Rect(Rect)
pub const HIGHLIGHT_COLOR: Color;              // amarillo rotulador por defecto (r255 g240 b0 a128)

pub fn highlight_under_gesture(
    spans: &[TextSpan],        // páginas extraídas con Document::text (engine.rs)
    gesture: &Gesture,         // trazo de puntos o marquee, en coords de página
    color: Color,
) -> Option<Highlight>;        // None si no hay líneas bajo el gesto
```

Selecciona las líneas (`TextSpan`) cuyo bbox intersecta el gesto y genera **una**
`Highlight` con un rect por línea, recortado al tramo horizontal del gesto (el
rotulador para donde para el trazo; no pinta la línea entera). Los rects son
por-línea alineados al texto (equivalentes a los quads del modelo del PDF).

## 2. Boli (tinta freehand) — suavizado Catmull-Rom en `crates/pdf_core/src/annotations.rs`

```rust
pub fn smooth_polyline(points: &[(f32, f32)], segs: u32) -> Vec<(f32, f32)>;
// 1 + (n-1)*segs puntos; pasa EXACTAMENTE por cada vértice; extremos fijos.
// entradas degeneradas (<2 pts o segs=0) se devuelven sin cambios.
```

Uso típico: capturar el trazo → `smooth_polyline(&pts, 4)` → guardar como
`Annotation::Stroke(Stroke::new(suavizado, width, color))`. No requiere estado
ni allocate nada más que el vec de salida.

## 3. Capa de rasterizado — `crates/pdf_core/src/overlay.rs`

```rust
pub struct ViewTransform { pub zoom: f32, pub offset_x: f32, pub offset_y: f32 }

pub fn composite_annotations(
    buf: &mut [u8],             // buffer RGBA8 del bitmap cacheado de página (width*height*4 bytes)
    width: u32,
    height: u32,
    anns: &[&Annotated],        // set.for_page(page_idx) → en orden z
    xform: &ViewTransform,      // screen = page * zoom + offset
);
// Infalible: no devuelve Result. Mezcla source-over; nunca toca el byte alpha.
```

- Highlights: fill por scanline con cobertura 1-D exacta (AA en bordes a O(h+w)),
  sin alocaciones por píxel.
- Ink: cada segmento se rasteriza sobre su bbox acotado, grosor = `Stroke.width`
  en puntos × zoom, AA de 1 px (fringe).
- Geometry fuera del bitmap se descarta en el clamp inicial.
- `TextNote` no pinta nada (lo dibuja la UI: lleva texto, no geometría).

Llamada esperada por frame en `pdf_android`:

```rust
composite_annotations(&mut page_buffer, w, h, &set.for_page(page_idx), &ViewTransform { zoom, offset_x, offset_y });
```

## 4. Persistencia — sin cambios de formato

`store.rs` ya serializa `Stroke`/`Highlight`/`TextNote` (kind + payload JSON por
fila). Highlight e Ink usan las variantes existentes, así que no se rompe el
formato previo y no hay migración. Las rects de Highlight se normalizan en
`AnnotationSet::add` (ya existente).

## Re-exportaciones en `lib.rs`

```rust
pub use annotations::{ ..., smooth_polyline };
pub use overlay::{ViewTransform, composite_annotations};
pub use selection::{Gesture, HIGHLIGHT_COLOR, highlight_under_gesture};
```

## Tests

- Unitarios: selección bajo trazo (clip x, bandas, marquee), suavizado Catmull-Rom
  (línea plana, esquina, degenerados), overlay (fill+alpha, transform, AA del
  trazo, clamp offscreen, notas no pintan).
- Integración (`crates/pdf_core/tests/annotations_pipeline.rs`, PDF real
  `tests/assets/simple.pdf`): gesto → highlight → sidecar SQLite round-trip →
  composición sobre el bitmap renderizado (píxel marcado cambia, zona limpia no).