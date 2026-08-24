# Separación Dedo ↔ Stylus: Cómo lo Hacen las Apps y Cómo Mejorarlo

> 2026-08-24. Objetivo: cuando la herramienta está activa, SOLO el lápiz
> dibuja; los gestos de la mano navegan (pan 1 dedo, pinch zoom 2 dedos).
> Investigación de patrones en Saber, Samsung Notes/S-Pen SDK, KOReader y
> la API Android (`tool_type`, `pressure`, stylus buttons).

## 1. Qué hacen las referencias

### Saber (Flutter) — `canvas_gesture_detector.dart`
- `Listener(onPointerDown/Move)`: detecta `PointerDeviceKind.stylus`/
  `invertedStylus` y normaliza `pressure` (min/max del sensor).
- `stows.autoDisableFingerDrawingWhenStylusDetected`: **en cuanto se detecta
  el stylus, se apaga el dibujo con dedo** (toggle, línea ~447).
- `InteractiveCanvasViewer.builder(isDrawGesture:…)`: el motor de gestos
  distingue draw vs navigate; con dedo: `panEnabled` (1 dedo pan con o sin
  "singleFingerPanLock", 2 dedos zoom, fling con fricción 0.3).

### Samsung Notes / S Pen SDK
- El stylus se reporta como `SOURCE_STYLUS` y la app solo dibuja con él;
  el dedo hace scroll/zoom. La palma (`TOOL_TYPE_PALM`) se REJECTA por
  defecto. Doble comprobación: cuando el lápiz está cerca (hover), los dedos
  no dibujan ni activan palm.
- Botón del lápiz (stylusPrimary/Secondary) → borrador / selección (sin
  cambiar de herramienta en la barra).

### KOReader (gestos)
- Slots separados por contacto: dedo y stylus tienen máquinas de gestos
  independientes (`ges_stylus_*` no sólo presión).

### Android NDK (android-activity 0.6 — disponible en nuestro stack)
- `Pointer::tool_type() -> ToolType::{Stylus, Finger, Palm, Mouse, Eraser}`.
- `Pointer::pressure() -> f32` (0..1, normalizable con min/max del sensor).
- `ButtonState::stylus_primary()/stylus_secondary()`.
- `Pointer::orientation()` (inclinación del lápiz).

## 2. Lo implementado (2026-08-24)

| Comportamiento | Antes | Ahora |
|---|---|---|
| Stylus + herramienta activa | dibuja | dibuja (igual) |
| Dedo + herramienta activa | DIBUJABA (bug) | **pan 1 dedo** (mover documento) |
| Dos dedos + herramienta activa | pinch (ya) | pinch (zoom) — igual |
| Palma | dibujaba | no dibuja (no es stylus) |
| Tool overlay/blend | (ya eliminado en tinta directa) | — |

Implementación:
- `handle_input`: `stylus = pointers().any(tool_type ∈ {Stylus, Eraser})`.
- Down del visor con tool activa: stylus → `ToolDrawing`; dedo → nuevo
  `GestureKind::Pan { start, pan0 }` (sin tap ni long-press: la mano sólo
  navega, nunca cambia de página mientras la herramienta está activa).
- Move Pan: `Reader::set_pan(pan0 + Δ)` + repaint por vsync (coalescing).
- PointerDown 2º dedo: Pan → Pinch (zoom) como hasta ahora.
- Up: fin de Pan sin acción.

## 3. Cómo mejorarlo a partir de aquí (medidas concretas)

1. **Confirmar tool_type real del lápiz USI de la TCL** (log de diagnóstico
   ya añadido): `begin_tool_gesture` loguea el gesto stylus; si la ROM
   reporta el lápiz como `Finger`, activar fallback: presión
   (`pressureMin != pressureMax` → es stylus de 4096 niveles).
2. **Presión → grosor/opacidad en vivo** (S-Pen): `width = ink_width *
   (0.6 + 0.8 * pressure)` — el usuario ya pidió grosores fijos, pero la
   presión da la sensación natural sin cambiar preset; se guarda por punto
   (Stroke ya soporta width fija; evolución: `widths: Vec<f32>` por punto).
3. **Botón del lápiz = borrador** (Samsung): `stylus_secondary()` durante
   gesto → borra lo que pisa (o alterna Eraser tool). Poco coste, alto
   valor para anotar.
4. **Hover del lápiz** (`Action::HoverMove`): cursor/indicador de donde va a
   caer la tinta — mejora la puntería al escribir (latencia percibida).
5. **Palm rejection activa**: ya implícita (palm no es stylus → nunca
   dibuja); opcional: ignorar palm también en Navigate (evita scrolls
   fantasma al apoyar la mano).
6. **Pan con inercia (fling)** al soltar el dedo (Saber fricción 0.3):
   animar `pan` por `tick` con decaimiento — el zoom/pan "se mueve" mejor.
7. **Clamp del pan** razonable (no perder la página fuera de pantalla
   del todo): límite ±(página·zoom)/2 — evita "perder" el documento.
8. **Zoom con doble-tap del dedo** (1x↔2x) con herramienta activa: el dedo
   ya no hace tap de página; liberar el doble-tap para zoom es natural.
9. GPU/EGL (documentado): la vía a <16 ms reales en esta ROM (backpressure
   del BufferQueue con 1 buffer).

## 4. Referencias

- `saber-notes/saber` (`lib/components/canvas/canvas_gesture_detector.dart`
  líneas 421-450, 480-520).
- Samsung S Pen SDK / Android `MotionEvent.getToolType`.
- `android-activity` 0.6 (`src/input.rs`: ToolType, Pointer::pressure,
  ButtonState stylus).
- KOReader slot de gestos por contacto.