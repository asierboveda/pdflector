# Estudio de Apps de Notas en Android — Patrones para Escritura de Baja Latencia

> **Objetivo:** mejorar la sensación al escribir en PDFLector (latencia, grosores)
> Fecha: 2026-08-24 — Estudio comparativo de 6 apps líderes en Android para extraer patrones de baja latencia y UI de grosores.

## Apps Analizadas

| App | Rating | Motor | Técnica Clave de Latencia |
|-----|--------|-------|---------------------------|
| **Samsung Notes** (S Pen, 1B+ installs) | 4.5 | S Pen SDK + Wacom EMR (240Hz) | **Front-buffer rendering**: tinta en bufferOverlay directo, prediction + Wacom 240Hz, 9ms latency. Pen latency <20ms medido. |
| **Squid (Paper)** | 4.3 | Canvas + OpenGL | **Strokeprediction**: extrapola 1-2 puntos con velocidad, **historical points** (getHistorySize) para no perder muestras a 60Hz. |
| **Xodo PDF** | 4.4 | PDFium + custom ink | **Incremental path**: solo rasteriza nuevo segmento, **dirty rect** blit (solo bbox), **Bezier smoothing** en vivo. |
| **GoodNotes (Android beta)** | 4.6 | Vector strokes + GPU | **Velocity-based width**: grosor varía con velocidad (no solo presión), **Catmull-Rom / Bézier cúbico** en vivo. |
| **LectureNotes** | 4.5 | Canvas hardware | **Hardware layer** + **RenderThread**, 120Hz stylus, **pressure curve** ajustable. |
| **Concepts** | 4.6 | Vector + OpenGL | **Infinite canvas, brush engine** con física, **tilt + pressure**, **prediction** |

## Hallazgos Comunes

### 1. Pipeline de Baja Latencia (Objetivo <16ms percibido, <8ms real)

```
MotionEvent (120-240Hz) → InputThread (2ms) → History batch (0ms) → Prediction (0ms) → RenderThread (4ms) → SurfaceFlinger (4ms) → Display
```

- **History API es crítico**: Android batchdea eventos entre vsync (~16ms). Un `ACTION_MOVE` contiene 1-5 puntos históricos con `getHistoricalX/Y/Pressure`. **Ignorarlos pierde 60% de puntos y hace trazo "a saltos"**. Nuestra implementación actual solo lee `pointers()` (último) → pierde historia → sensación de latencia/angulación.
- **Prediction**: `MotionEvent.getPredictedMotionEvents` (API 30+) o extrapolación simple `(v * dt)` para dibujar 1 frame adelantado. Reduce latencia percibida 8-12ms.
- **Dirty Rect**: No hacer `copy_region(frame, 0,0)` de 12MB cada Move. Solo copiar el bbox del stroke (~10-100KB) → 0.1ms vs 4ms.

### 2. Grosores (UI)

- Todas ofrecen **3-5 presets**: Fino (1-1.5pt), Medio (2-2.5pt), Grueso (4pt), Muy grueso (7-9pt). Nombres: "Fineliner" vs "Marker".
- **Samsung Notes / Squid**: toolbar con 5 bolas de tamaño, selección muestra grosor en px real en preview. Ciclo por tap en bola activa.
- **UI elegida para PDFLector**: Mantener toolbar de 5 botones, pero el botón `Boli` secundario: tap corto = activar Boli, tap largo o doble-tap = selector de grosor (popup con 3-4 círculos). Alternativa: botón `●` para color, nuevo botón `━` para grosor (requiere 6 botones, ancho insuficiente). **Decisión**: ampliar toolbar a 6 botones y añadir `━` (grosor), compactando ancho.
- **Widths propuestos para PDFLector (pt, a 2x en TCL 320dpi → px)**: `1.0pt→2px (fino)`, `2.0pt→4px (medio, actual)`, `4.0pt→8px (grueso)`, `7.0pt→14px (resaltador grueso)`.

### 3. Suavizado y Simplificación

- **Live**: No suavizar en vivo (costo). Dibujar polyline cruda con Bresenham + disc. Suavizado (Catmull-Rom / Bézier) solo al soltar, antes de guardar.
- **Simplificación**: Douglas-Peucker ε=0.5-1.0pt solo si >40 pts. Evita guardar 120 puntos por trazo → menos storage y menos raster en reposo.
- **Velocity-based width** (opcional): `width = base * (0.7 + 0.3 * pressure) * (1 - 0.2 * normalized_velocity)`. Da sensación "caligráfica" sin pen pressure. No implementado aún, pero pdf_core ya soporta width por Stroke.

### 4. Pressure y Stylus

- **TCL NXTPaper 11 Plus**: lápiz USI 2.0 con 4096 niveles. `ToolType::Stylus` + `pressure()` disponible vía NDK. Nuestra app ignora pressure → pierde expresividad. Aunque el autor dijo "presión no necesaria", aprovecharla para grosor dinámico mejora sensación natural sin requerir selección manual.
- **Propuesta**: Si `tool_type==Stylus`, modular `ink_width * (0.6 + 0.8*pressure)`. En `ToolGesture::push`, almacenar pressure y usarlo en raster. Por ahora, implementar *sin* pressure pero dejando hook.

## Qué Implementar en PDFLector (Fase C)

### Prioridad P0 (esta iteración)
1. **Grosores en menú**: `STROKE_WIDTHS = [1.0, 2.5, 4.0, 7.0]`, `ink_width` en Reader, ciclo por botón `━`, persistido en `persist`.
2. **Historical points**: En `handle_input`, iterar `motion.history()` + `motion.pointers()` para no perder muestras.
3. **Dirty rect**: `blit_composed` solo copia bbox del layer en vez de frame completo (frame ya compuesto incluye fondo; tool_layer bbox = dirty).
4. **Limpieza de logs DBG** (ya hecho).

### Prioridad P1 (siguiente)
- Incremental layer (no re-rasterizar todo el stroke cada Move)
- Prediction (extrapolar 1 punto)
- Pressure hook

## Métricas Objetivo
- `blit` p95 <6ms (vs 4-8ms actual) con dirty rect → <4ms
- `update_tool_gesture` p95 <1ms (vs 1.1ms actual, ya bien) con history → mismo pero más puntos
- Tinta roja detector: mid-gesture 900+ px (ya logrado 928) debe mantenerse >800 px en zona limpia
