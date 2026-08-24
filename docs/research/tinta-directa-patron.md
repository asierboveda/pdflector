# Análisis Profundo: Cómo Pintan el Trazo las Apps de Escritura (En Vivo)

> 2026-08-24. Objetivo: entender el modelo mental del usuario ("no quiero que se
> guarde en un buffer, que se implemente nada más hacerlo") y comparar el flujo
> de dibujo EN VIVO de las mejores apps, no solo su arquitectura general.
> Fuentes: `saber-notes/saber` (clonado), `xournalpp/xournalpp` (referencia
> escritorio), Samsung Notes/S-Pen SDK (documentación), docs Android.

## 0. Diagnóstico de la sensación del usuario

En nuestra app el trazo EN CURSO se rasteriza **completo, desde cero, en cada
Move**, a un bitmap temporal (bbox creciente) y se **funde** sobre el frame
(`tool_overlay` + `copy_region_blend`). Al soltar, el trazo se sustituye por la
versión **suavizada** (Catmull-Rom + Douglas-Peucker) guardada en el modelo y
se vuelve a pintar desde el set. Efectos perceptibles:

1. El trazo aparece "dibujado desde una capa encima" (la página no cambia);
2. Al soltar, la geometría cambia (cruda → suavizada) → "salto" de silueta;
3. El dibujo se "re-hace" (rasterización completa) en cada evento.

El usuario pide: **tinta directa** — cada tramo se pinta inmediatamente y queda;
al soltar, "se implementa" (persiste en el modelo) sin que se note ningún
cambio de forma ni de plano.

## 1. Cómo pintan las otras apps (código real)

### Saber (Flutter/GPU) — `_canvas_painter.dart`, `_stroke.dart`
- En vivo: `Stroke.getPolygon(quality: .low)` — rectas, sin smoothing
  (`streamline=0, smoothing=0`), puntos decimados (`skipPoints`); se dibuja
  con `canvas.drawPath(lowQualityPath)` TODOS los frames (GPU).
- Al soltar: `getPolygon(quality: .high)` = `perfect_freehand` con suavizado +
  presión; `markPolygonNeedsUpdating()` → se redibuja.
- **Cambio visible al soltar**: sí existe (low→high), pero es pequeño y la
  GPU lo pinta en el siguiente frame; nadie lo nota porque el trazo es ya
  "caligráfico" en vivo (perfect_freehand LOW aún da contorno).
- Conclusión: **GPU redibuja todo cada frame** — el concepto "buffer" no
  existe para el ojo porque el frame completo se re-renderiza 60-120 fps.

### Xournal++ (Cairo/CPU) — el caso más parecido al nuestro
- `Stroke` acumula puntos; en vivo se dibuja **incremental**: cada nuevo
  punto → `xf::view::StrokeView::draw` llama a `canvas->draw_path(...)` con
  **solo el segmento nuevo APPROXIMADO** (se usa un spline de bajo coste:
  `xf::shape::spline_approximate` sobre los últimos puntos).
- Al soltar: se **redibuja la zona del trazo** con el spline final suavizado
  y se añade al layer (`layer->addElement(stroke)` → damange rect = bbox).
- Conclusión: en CPU, el patrón correcto es **stamping incremental del
  segmento nuevo** (nunca re-rasterizar el trazo completo) y **repintar solo
  el bbox al soltar con la geometría final**. El "salto" se minimiza
  suavizando en vivo con la MISMA función que al final (solo cambia el nº de
  puntos: aproximado durante el gesto, exacto al soltar).

### Samsung Notes / S Pen SDK (referencia de latencia)
- La tinta se dibuja a una **surface frontal separada** (front-buffer de
  tinta, Wacom 240Hz) y el sistema la compone; al levantar el lápiz la app
  **integra** la tinta en el documento y descarta el front-buffer.
- Latencia objetivo: 9-20 ms. La clave es hardware (240Hz) + prediction.

### GoodNotes / Noteshelf (iOS/vector)
- `UIBezierPath` + `addLine`/`addQuadCurve` en vivo; el render de cada frame
  redibuja el path acumulado (CoreGraphics/GPU). Suavizado con puntos de
  control calculados en vivo desde los últimos 4 puntos (Catmull-Rom) →
  **la forma que se ve es la misma que se guarda** (cero salto al soltar).

## 2. El patrón correcto para CPU (lo que adoptamos)

1. **Stamping incremental**: en cada Move, pintar SOLO el segmento
   `(último punto → nuevo punto)` directamente sobre el frame (Bresenham +
   brocha redondeada, como los trazos guardados). La tinta queda en la
   página desde el primer tramo; no hay capa temporal que se rehaga.
2. **Mismo rasterizador** para el trazo en vivo y el guardado → misma pinta.
3. **Al soltar**: añadir al modelo (`AnnotationSet` + SQLite = "se implementa
   nada más hacerlo") y, como la geometría guardada se suaviza (Catmull-Rom)
   para export/calidad, se **restaura el frame base y se pinta el trazo
   final suavizado sobre su bbox** (una pasada, no recomposición de 146
   trazos — el resto del frame no se toca).
4. **Suavizado en vivo progresivo**: los puntos llegan densos (60-120Hz), la
   brocha redondeada hace que el trazo ya se vea suave; si el gap es grande,
   interpolamos el segmento (2-3 puntos) como hace Xournal++ con su spline.
5. **Highlight** (rect translúcido) mantiene su capa temporal actual (no
   aplica stamping: es geometría que se re-encaja al texto al soltar).

## 3. Eliminado del pipeline anterior

- ❌ `tool_overlay` re-rasterizado completo por Move (ink) → sustituido por
  stamping incremental del segmento.
- ❌ Blend de la capa temporal por frame (ink) → la tinta ya está en el frame.
- ❌ Recomposición al soltar → página base guardada por gesto (12MB, 4ms una
  vez) + pintado del trazo final sobre su bbox.
- ❌ Cambio de geometría al soltar → se minimiza con suavizado similar en
  vivo + repintado del bbox solo (el resto de la página ni se toca).

## 4. Métricas objetivo (TCL 9469X)

- Coste por Move (ink): raster del segmento (~µs-0.2ms) + dirty blit del bbox
  (~0.1ms) + lock (0-16ms según driver) → en CPU **<= 1 frame** de latencia.
- Coste al soltar: 1× stamp del bbox del trazo (≈0.5-2ms) + SQLite + 1 blit.
- Sin recomposición completa en ningún punto del flujo de escritura.