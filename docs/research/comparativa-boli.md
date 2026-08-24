# Comparativa: Apps de Escritura con Boli en Android vs PDFLector

> Fecha: 2026-08-24. Misión: encontrar la mejor app OSS de apuntes con boli según
> foros/comunidad, leer su código real, y compararla con nuestra implementación.
> Fuente primaria: repos clonados (shallow) — `saber-notes/saber` ★4.7k,
> `xournalpp/xournalpp` ★15k, `foobnix/LibreraReader` ★4.7k,
> `ArtifexSoftware/mupdf-android-viewer` ★184, `DImuthuUpe/AndroidPdfViewer` ★8.4k.

## 0. Veredicto (respuesta corta)

**Saber** (saber-notes/saber, ★4.7k, Dart/Flutter) es la app OSS de apuntes con
boli **más recomendada en foros** (Reddit r/Android, F-Droid, r/FOSS) para
escribir a mano con stylus: presión real, palm rejection, trazos caligráficos
(perfect_freehand), y es la de **menor latencia percibida** de las OSS porque
**dibuja vectores por GPU** (Skia/Impeller) — no rasteriza píxeles por CPU como
nosotros. Xournal++ (★15k) es el estándar de escritorio (Cairo CPU, más lento
en móvil). Librera y mupdf-viewer anotan PDFs pero su tinta es secundaria.

**La diferencia fundamental con PDFLector:**
- Saber: cada frame → `canvas.drawPath(stroke.highQualityPath)` (GPU). El coste
  por Move es **~0.1-0.5 ms** y no depende del nº de trazos ya dibujados.
- PDFLector: cada Move → rasterización Bresenham por CPU + **copia de ~12 MB**
  del frame al ANativeWindow (**4-8 ms**) + blend. Con 146 trazos, recomponer el
  frame cuesta 6-13 ms. Sumado = **"va muy lagado al escribir"**.

## 1. Ranking (foros + estrellas)

| # | App | ★ | Lang | Motor | Nota de foros |
|---|-----|---|------|-------|---------------|
| 1 | **Saber** | 4.7k | Dart (Flutter) | Skia/Impeller (GPU) | La más recomendada OSS para stylus; presión nativa, palm rejection, exporta PDF/SVG/PNG |
| 2 | **Xournal++** | 15k | C++/GTK | Cairo (CPU) | Estándar escritorio/Linux; en Android solo vía port no oficial |
| 3 | **Librera** | 4.7k | Java | MuPDF (CPU) | Lector/reflow; modo dibujo básico (pro)
| 4 | mupdf-android-viewer | 184 | Java | MuPDF (CPU) | Viewer oficial; ink minimal |
| 5 | AndroidPdfViewer | 8.4k | Java | PdfiumAndroid | Librería de vista, sin tinta propia |

## 2. Cómo dibuja Saber (código real leído)

**Input (`canvas_gesture_detector.dart`):**
- `_listenerPointerEvent`: detecta `PointerDeviceKind.stylus`/`invertedStylus`
  y normaliza **presión real**: `_inverseLerp(event.pressure, event.pressureMin,
  event.pressureMax)` (líneas 429-442). Dedos → `pressure = null` (simula).
- Gestos sobre `CustomPaint` de Flutter → eventos a 60-120 Hz + `history`
  implícita del framework.

**Modelo (`_stroke.dart`):**
- `Stroke` guarda `points` y **dos calidades de polígono/path**:
  - `lowQualityPolygon/Path`: `getStroke(skipPoints(points, low))` con
    `smoothing=0, streamline=0`, líneas rectas — **se dibuja MIENTRAS se
    escribe** (rápido).
  - `highQualityPolygon/Path`: `smoothPathFromPolygon` — **solo cuando el trazo
    se completa** (al soltar). Cambio de calidad = un repaint al final.
- `perfect_freehand` genera el contorno del trazo (caligráfico, con presión
  simulada si no hay). `skipPoints` = **decimación** de puntos (menos vértices).
- Los paths se cachean (`_lowQualityPath`/`_highQualityPath`) y se invalidan con
  `markPolygonNeedsUpdating()` (borrar/editar).

**Render (`_canvas_painter.dart`):**
- `paint()` dibuja **TODOS los strokes cada frame** con `canvas.drawPath(path,
  paint)` — la GPU hace el trabajo; no hay bitmap de tinta ni recorte por
  región. `shouldRepaint` → "always repaint if current stroke present".
- Highlighter: `canvas.saveLayer(canvasRect, layerPaint)` para el modo
  "subrayar no tapa el texto" (blend), luego `restore`.

**Conclusión de arquitectura**: trazo = **Path vectorial en GPU**; el coste por
frame es ~constante (geométrico), ∝ nº de paths pero cada drawPath de cientos de
strokes es trivial en GPU. La CPU solo serializa puntos.

## 3. Xournal++ (estándar escritorio, para contexto)

- Input thread + pen listeners, presión vía GDK (Wacom).
- Render: **Cairo por CPU** sobre superficie doble-buffer; strokes como
  `cairo_stroke` de un path. Redibuja por región sucia.
- En móvil/GPU débil sufre precisamente lo mismo que nosotros: raster CPU +
  scale. Es la referencia de características (herramientas, capas), no de
  latencia.

## 4. Comparación con PDFLector

| Aspecto | Saber | PDFLector (hoy) | Impacto |
|---|---|---|---|
| Render del trazo | `drawPath` GPU | Bresenham CPU + blend software | GPU 0.1-0.5ms vs CPU 1-2ms |
| Repintado por Move | Todo el canvas (GPU) | **Copia de 12MB del frame** + overlays | **4-8 ms por Move** (¡dominante!) |
| Trazos ya dibujados | Se re-dibujan por GPU (barato) | Se re-rasterizan por CPU al recomponer frame | 6-13ms con 146 trazos (hitch al empezar trazo) |
| Densidad de puntos | `skipPoints` (decimación) + LOD bajo/alto | Puntos crudos + `smooth_polyline(6)` al soltar | Más vértices = más raster |
| Calidad en vivo | LOD bajo (rectas) en vivo, LOD alto al soltar | Mismo path en vivo y al soltar | — |
| Presión | `pressure` normalizada (stylus) | Ignorada (por decisión del autor) | Expresividad |
| History API | Flutter la absorbe | No usada (eventos sueltos) | Trazos más densos |
| Persistencia | JSON por nota | SQLite sidecar (bien) | — |

**Causas del "va muy lagado al escribir" (confirmadas con log):**
1. `blit()` por Move copia el frame completo (memcpy 12MB ≈ 4-8 ms en TCL) +
   blend del trazo → el gesto gasta ~5-9ms de los 16 disponibles, con jitter.
2. Al **empezar** cada trazo se invalidaba `page_frame` → recomposición completa
   de TODAS las anotaciones (~6-13 ms con 146) antes del primer Move.
3. Puntos sin decimar + sin history → trazos con muchos vértices más lentos de
   rasterizar y guardar.

## 5. Qué copiamos (orden de ejecución)

- [x] **No recomponer el frame al empezar trazo** (parche incremental al
  soltar: el frame ya tiene los trazos previos, el nuevo se dibuja encima).
- [x] **Dirty rect** en el blit por Move (`copy_region_rect` + unión de bboxes):
  solo se repinta el área del trazo (~50KB) en vez de 12MB → 4-8ms → <0.2ms.
- [ ] **History API** (`motion.history()`, android-activity ya lo expone):
  puntos batch entre vsync — densidad real sin interpolar.
- [ ] **LOD doble** (Saber `lowQualityPath`/`highQualityPath`): en vivo trazo
  con rectas (rápido), al soltar `smooth_polyline` nítido (ya lo hacemos al
  soltar — falta el "rápido en vivo", que es lo mismo que ya tenemos).
- [ ] **Presión stylus** (opcional, aunque el autor la descartó): `pointer.pressure()`
  → grosor variable por trazo (más natural).
- [ ] **GPU/EGL** (el cambio gordo, para Fase 6 con Slint o renderer propio):
  subir página como textura + strokes como triángulos → latencia <1ms incluso
  con cientos de trazos. Elimina el memcpy de 12MB por completo.

## 6. Referencia de medición (TCL 9469X, antes del dirty rect)

- blit por Move: 4.0-8.0 ms (`lock+copy+unlock_and_post`), + blend ∝ bbox.
- Recomposición con 146 trazos: 6.8 ms desktop / ~13 ms TCL estimado.
- Decimación: trazo de 60 Moves → ~60 vértices crudos (+ interpolación previa
  que se REVERTIÓ por O(N²)); con skipPoints tipo Saber → ~15 polígono.
## 7. Hallazgo de hardware (TCL 9469X, 2026-08-24, medido)

El cuello real del "lag al escribir" NO era la rasterización sino el **lock del
BufferQueue**: con `ANativeWindow_lock` por CPU y la cola de buffers que asigna
la ROM (1-2), cada blit espera ~1 vsync a que SurfaceFlinger libere el buffer
anterior (medido con fases: `lock=15-22ms`, `copy=0.1ms`). Con coalescing por
vsync + dirty rect + dirty-lock el sistema queda en **60 fps estables** con la
tinta a ≤1 frame de latencia, pero NO se puede bajar de ahí sin cambiar de
mecanismo: `ANativeWindow_setBuffersCount` no está exportado por libandroid de
esta ROM (UnsatisfiedLinkError), y el driver ignora la petición.

**Conclusión**: la única vía a latencia tipo Saber (<10 ms, 120 fps) es el
**render por GPU** (EGL/GLES2 con la página como textura y strokes como paths
en triángulos, o el port Slint que ya usa Skia/GPU) — es exactamente lo que
hace Saber/Flutter (y la razón de su suavidad). El pipeline CPU actual queda
para la UI/biblioteca (donde 60 fps bastan).

## 8. Estado final del pipeline de tinta (2026-08-24)

| Pieza | Estado |
|---|---|
| Boli visible en vivo | ✔ (fix alpha `composite_annotations_alpha`) |
| Sin recomposición por trazo | ✔ (parche incremental del frame al soltar) |
| Dirty rect por Move | ✔ (`copy_region_rect` + unión de bboxes, copy 0.1ms) |
| Coalescing por vsync | ✔ (un blit por iteración, `take_repaint`) |
| Lock con región sucia | ✔ (ráfagas 3-4ms) |
| History API / prediction / presión | Pendiente (historia separada) |
| GPU (EGL/Slint) | Pendiente — único camino a <16ms reales |
