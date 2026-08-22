# Barra de herramientas de anotación en pdf_android (Fase 3.5) — resumen de UI y prueba manual pendiente

Fecha: 2026-08-22 — Integración del resaltador con detección de texto y del
boli (lápiz/tablet) en el visor de pdf_android, sobre la API de pdf_core que
dejó la tarea de motor (selection.rs / overlay.rs / annotations.rs).

## Qué se añadió

- **Barra de herramientas discreta** (píldora centrada arriba, estilo del
  visor, paleta warm-neutral de `lib.rs mod theme`): botones **Resaltar**,
  **Boli**, **↶** (deshacer), **●** (cicla el color del boli; se dibuja con el
  color actual) y **→** (volver a navegación y cerrar la barra). La muestra/
  oculta un botón flotante **✎/✕** (esquina superior derecha). Ocultar la
  barra vuelve a modo NAVEGACIÓN (decisión: nunca queda una herramienta
  activa sin barra visible).
- **Resaltador**: con la herramienta activa, arrastrar con el dedo O el lápiz
  sobre texto genera un gesto; al levantar, `pdf_core::highlight_under_gesture`
  selecciona las líneas bajo el trazo y crea un `Highlight` vectorial alineado
  al texto (extracción de texto perezosa, solo al soltar). Feedback en vivo:
  rect amarillo translúcido durante el arrastre.
- **Boli**: trazo freehand suavizado con `pdf_core::smooth_polyline`
  (Catmull-Rom, 6 subdivisiones) al soltar; en vivo se ve el trazo crudo como
  capa temporal por frame. Grosor fijo `STROKE_WIDTH_PT` (2 pt), color
  configurable por paleta (`INK_PALETTE`).
- **Rendimiento (req. 5)**: mientras hay un gesto de herramienta el blit usa
  el FRAME COMPUESTO (`draw::compose_frame` + `draw::blit_composed`, el mismo
  pipeline del sheet): la página se compone UNA vez al bajar el dedo y cada
  `Move` solo rasteriza el bbox del trazo en curso con
  `pdf_core::overlay::composite_annotations` (sin allocaciones por píxel) y
  lo copia con alfa-blend (`draw::copy_region_blend`) — la página NO se
  re-blitea por evento de movimiento.
- **Persistencia**: las anotaciones nuevas se guardan con el `AnnotationStore`
  existente (sidecar SQLite por PDF, `save_annotations`) y se cargan al abrir
  (`load_annotations`, ya existente). **Undo** = deshacer la ÚLTIMA anotación
  de la sesión (pila `Reader::session_ids`; nunca toca anotaciones de otras
  sesiones).
- **Gestos existentes intactos**: con la herramienta Navegar (barra cerrada)
  todo sigue igual (tap página, pinch zoom, long-press selección, pull del
  sheet). Con herramienta activa: el arrastre dibuja, el pinch sigue
  funcionando (cancela el trazo en curso), el tap simple no navega, la
  selección de texto (long-press) queda desactivada y el tap en el chrome
  (✎/barra) siempre funciona.

## API de UI (firmas principales, crate pdf_android)

```rust
// annotations.rs
pub(crate) enum ToolKind { Navigate, Highlight, Ink }
pub(crate) struct ToolGesture { page: u32, tool: ToolKind, anchor: (f32,f32), points: Vec<(f32,f32)> }
pub(crate) const STROKE_WIDTH_PT: f32;   // 2.0 pt
pub(crate) const DEFAULT_INK_COLOR: Color;
pub(crate) const INK_PALETTE: [Color; 4]; // negro azulado / sepia / azul apagado / vino

// reader.rs (métodos usados por input/draw)
pub(crate) fn set_tool(&mut self, ToolKind);       // activar herramienta
pub(crate) fn toggle_toolbar(&mut self);           // ✎/✕: mostrar/ocultar barra
pub(crate) fn close_toolbar(&mut self);            // →: navegación + cerrar
pub(crate) fn cycle_ink_color(&mut self);          // ●: ciclar color del boli
pub(crate) fn undo_last_annotation(&mut self);     // ↶: deshacer último de la sesión
pub(crate) fn begin_tool_gesture(&mut self, sx, sy);   // Down del dedo/lápiz
pub(crate) fn update_tool_gesture(&mut self, sx, sy);  // cada Move (frame compuesto + capa temporal)
pub(crate) fn end_tool_gesture(&mut self);             // Up: Highlight o Stroke suavizado + guardar
pub(crate) fn cancel_tool_gesture(&mut self);          // segundo dedo / Cancel
pub(crate) fn chrome_hit(&self, x, y) -> bool;         // ¿tap en ✎ o barra?

// draw.rs
pub(crate) fn toolbar_rect(win_w, win_h) -> (f32,f32,f32,f32);   // píldora arriba
pub(crate) fn tool_fab_rect(win_w, win_h) -> (f32,f32,f32,f32);  // botón ✎
pub(crate) fn toolbar_buttons(reader, win_w, win_h) -> Vec<(&'static str, ButtonRect)>;
pub(crate) fn render_toolbar(reader) -> Option<Bitmap>;   // cacheado en Reader::toolbar_bitmap
pub(crate) fn render_tool_fab(reader) -> Option<Bitmap>;
pub(crate) fn raster_tool_layer(win_w, win_h, xform, &Annotated, pad) -> Option<(Bitmap,i32,i32)>;
pub(crate) fn copy_region_blend(dst, ...);   // overlay del trazo con alfa-blend

// input.rs
pub(crate) fn toolbar_tap(reader, app, x, y) -> bool;   // taps de la barra
// + GestureKind::ToolDrawing y el manejo de Down/Move/Up/Cancel en handle_motion
```

## Prueba manual pendiente (la hace el autor, tablet TCL NXTPaper 11 Plus)

No se ha probado en hardware: compilar/renderear en la tablet y verificar:

1. Abrir un PDF con texto (p. ej. `corpus/dense_textbook.pdf`) y con el
   **Resaltar**: el arrastre debe mostrar el rect amarillo en vivo y al soltar
   debe quedar un subrayado alineado a las líneas (no una caja suelta); en
   `Download/.../annotations/<stem>.db` (o `internal/pdfs/annotations/`)
   debe aparecer el `Highlight` con `rects` por línea.
2. **Boli** con el lápiz: trazo visible en vivo sin saltos, suave al soltar
   (Catmull-Rom), color del botón ●; al volver a abrir el PDF el trazo sigue
   ahí (sidecar SQLite).
3. **↶**: deshace el último trazo de la sesión (también el del subrayado por
   selección de texto, que ahora se apunta a la sesión).
4. **Ocultar barra**: ✎ oculta/muestra; al ocultar se vuelve a navegación y
   los taps cambian de página otra vez; el pinch sigue funcionando con
   herramienta activa; el pull del sheet y el long-press de selección no se
   rompen con la barra cerrada.
5. **Rendimiento**: dibujar con el lápiz no debe dar saltos (frame + overlay
   del trazo; sin re-blit de página por Move) — comprobar con el log de frame
   time del blit si se ve degradación.