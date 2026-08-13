# Zoom y pan (pinch) en lectores PDF/móviles — investigación

> Investigación de referencia para PDFLector (2026-08-13). Objetivo: cómo
> implementan los lectores maduros el **zoom/pan suave y sin bugs** — factor de
> zoom, punto de anclaje, clamp a los bordes, "escalar y renderizar" (fast path
> borroso + re-render nítido), límites min/max y transición sin salto.
>
> **Estado actual de PDFLector** (lo que esta investigación debe contrastar):
> - Android (`crates/pdf_android`): pinch con **factor RELATIVO a la distancia
>   inicial** (`zoom = z0 × dist / start_dist`), anclaje al centro del pinch
>   (`PinchAnchor` + `anchor_pan`, `reader.rs`), clamp a los bordes de la hoja
>   (`clamp_pan`), fast path por **vecino-más-cercano en CPU** del bitmap
>   cacheado (`zoom.rs`/`blit_fast`) y **re-render nítido UNA vez al soltar**
>   (`set_zoom_sharp`). `PINCH_MIN = 1.0` (fit-page, sin zoom-out),
>   `PINCH_MAX = 8.0`.
> - Escritorio (`crates/pdf_core::zoom` + `pdf_app`): escalado **bilinear
>   software** inmediato + re-render nítido asíncrono a la escala de la
>   "escalera" `scale_level_for_zoom` (ceil-log2), `RenderCache::trim_to_scale_level`.
>
> **Fuentes** (código capturado en `/tmp/pdf-research/` durante esta sesión):
> - `koreader/` (rama `master`, KOReader)
> - `pdf.js/` (rama `master`, mozilla/pdf.js)
> - `mupdf-android-viewer/` (rama `master`, ArtifexSoftware)
> - `LibreraReader/` (rama `master`, foobnix)
> - `slint/` (rama `master`, slint-ui)

## 1. Resumen ejecutivo (qué copiar tal cual)

1. **El anclaje es aritmética simple que se repite en todos**: mantener fijo el
   punto bajo los dedos = desplazar el scroll por `punto × (1 − factor)` cuando
   el zoom cambia de factor. MuPDF (`mXScroll += viewFocusX − viewFocusX·factor`),
   pdf.js (`dx -= (origin[0]−left)·(newScale/prevScale − 1)`) y Librera
   (`(scroll + half)·ratio − half`) son tres formulaciones del mismo invariante.
2. **El fast path borroso es universal, pero la calidad percibida la da el
   re-render nítido DURANTE el gesto, no solo al soltar**: pdf.js re-renderiza
   con `drawingDelay = 400 ms` debounced; MuPDF re-renderiza en segundo plano la
   **región visible** ("HQ patch") cancelando el render en vuelo y swapando
   bitmaps para no parpadear; Librera distingue `committed` (re-decode) vs
   `inZoom` (solo reescalar). PDFLector Android hoy solo re-renderiza al soltar:
   zooms intermedios sostenidos se ven borrosos.
3. **La deriva de precisión se combate acumulando el resto**: pdf.js acumula el
   factor fraccional (`_accumulateFactor`) cuando redondea la escala a 2
   decimales, y `panBy` conserva las fracciones de píxel del scroll entre Moves.
   PDFLector guarda `zoom` y `pan` en f32 continuos y redondea solo en el blit:
   el patrón no aplica hoy, pero aplicará si se introduce redondeo.
4. **El clamp no se limita a recortar: se anima**. MuPDF, al soltar, si la
   página quedó parcialmente fuera de pantalla, hace *snap-back* animado
   (`slideViewOntoScreen`, 400 ms). PDFLector clampea el pan en cada Move: es
   correcto, pero no hay "resorte" al soltar fuera de los bordes.
5. **Diseño de gestos**: KOReader (e-ink, sin multitáctil fiable) mapea el
   pinch a **modos de zoom** (page/pagewidth/pageheight) en vez de zoom libre;
   Librera usa factor **incremental por evento** `sqrt(new/old)` y ancla al
   **centro de pantalla**; MuPDF usa el factor incremental de
   `ScaleGestureDetector` (que ya suaviza) con anclaje al **foco** y
   compensación de deriva del foco. PDFLector (relativo a inicio + anclaje al
   centro del pinch) está entre los más robustos: no necesita acumulador ni
   compensación mientras no se mueva el centro o se redondee.

## 2. Repos estudiados

| Repo | URL | Qué aporta a esta investigación |
|------|-----|---------------------------------|
| **KOReader** | https://github.com/koreader/koreader | Clamp de viewport (`Geom:offsetWithin`/`centerWithin`), modos de zoom (fit page/width/height/content/columns/rows/manual), pan por % de viewport, decisión de diseño: pinch → modos en e-ink |
| **pdf.js** | https://github.com/mozilla/pdf.js | `_accumulateFactor` (anti-deriva por redondeo), `panBy` fraccional, anclaje por `origin`, `drawingDelay = 400 ms` (re-render debounced durante el pinch), clamps `MIN_SCALE=0.1`/`MAX_SCALE=25` |
| **MuPDF Android viewer** | https://github.com/ArtifexSoftware/mupdf-android-viewer | El patrón completo: escala GPU del bitmap completo durante el pinch + **HQ patch** de la región visible en background (cancelable, con swap de bitmaps), anclaje al foco + compensación de deriva, snap-back animado, fling con margen. `MIN_SCALE=1.0`/`MAX_SCALE=64` |
| **Librera** (EBookDroid) | https://github.com/foobnix/LibreraReader | `MultiTouchGestureDetector` propio, factor incremental `sqrt(new/old)`, anclaje al **centro de pantalla**, flag `committed` (re-decode al soltar vs solo reescalar durante el gesto). `MIN_ZOOM=0.5`/`MAX_ZOOM=32`, redondeo de zoom configurable |
| **Slint** | https://github.com/slint-ui/slint (ejemplo `native-gestures`) | API declarativa `ScaleRotateGestureHandler` con `transform-origin: gesture.center` (el anclaje lo hace el renderer), acumulación de `base-scale ×= gesture.scale` al finalizar. Referencia directa para la UI final pendiente (Fase 6) |

## 3. Técnicas (con cita)

### 3.1 Factor de zoom: relativo-a-inicio vs incremental vs acumulado

Tres modelos coexisten; PDFLector usa el primero (el más a prueba de deriva):

- **Relativo a la distancia inicial del gesto** (PDFLector, `input.rs`):
  `zoom = start_zoom * d / start_dist`. Pinch-out/in sin mover los dedos
  devuelve exactamente el zoom de partida; nunca hay deriva. Ningún repo
  estudiado usa exactamente este modelo — es el más robusto.
- **Incremental por evento** (Librera, `AbstractViewController.java:787`):
  ```java
  public void onTwoFingerPinch(final MotionEvent e, final float oldDistance, final float newDistance) {
      final float factor = (float) Math.sqrt(newDistance / oldDistance);
      base.getZoomModel().scaleZoom(factor);
  }
  ```
  `scaleZoom` multiplica el zoom actual (`ZoomModel.java:56`). Cada evento
  re-ancla, así que un error de redondeo por evento se acumula a lo largo del
  gesto (mitigado porque Librera no redondea: `ZOOM_ROUND_FACTOR = 0`).
- **Incremental con suavizado del sistema** (MuPDF, `ReaderView.java:490`):
  ```java
  mScale = Math.min(Math.max(mScale * detector.getScaleFactor(), MIN_SCALE), MAX_SCALE);
  float factor = mScale/previousScale;
  ```
  `ScaleGestureDetector.getScaleFactor()` ya suaviza internamente (media
  móvil), y el factor se recalcula por evento; igual que Librera, encadena
  errores de redondeo por evento (el `Math.min/max` puede "comerse" un
  fragmento de gesto al tocar los límites).
- **Incremental con acumulador del resto** (pdf.js, `app.js:2641`):
  ```js
  _accumulateFactor(previousScale, factor, prop) {
      if (factor === 1) return 1;
      // Si cambia la dirección, reinicia el acumulador.
      if ((this[prop] > 1 && factor < 1) || (this[prop] < 1 && factor > 1)) this[prop] = 1;
      const newFactor =
          Math.floor(previousScale * factor * this[prop] * 100) / (100 * previousScale);
      this[prop] = factor / newFactor;
      return newFactor;
  }
  ```
  pdf.js redondea la escala a 2 decimales (`Math.round(newScale * 100) / 100`
  en `updateScale`); el resto de cada evento se guarda en `_touchUnusedFactor`
  y se aplica en el siguiente, de modo que el redondeo no se pierde ni se
  acumula. Es la corrección necesaria **solo si se redondea el zoom**.

### 3.2 Punto de anclaje (mantener fijo el punto bajo los dedos)

Tres formulaciones del mismo invariante ("punto de documento bajo el foco
permanece en pantalla"):

- **MuPDF** (`ReaderView.java:495-513`) — anclaje al foco + compensación de
  deriva del foco (los dedos se mueven durante el gesto):
  ```java
  float factor = mScale/previousScale;
  int viewFocusX = (int)currentFocusX - (v.getLeft() + mXScroll);
  int viewFocusY = (int)currentFocusY - (v.getTop() + mYScroll);
  mXScroll += viewFocusX - viewFocusX * factor;      // = viewFocusX·(1 − factor)
  mYScroll += viewFocusY - viewFocusY * factor;
  if (mLastScaleFocusX >= 0)
      mXScroll += currentFocusX - mLastScaleFocusX;  // deriva del foco
  if (mLastScaleFocusY >= 0)
      mYScroll += currentFocusY - mLastScaleFocusY;
  mLastScaleFocusX = currentFocusX; mLastScaleFocusY = currentFocusY;
  ```
  Además, `onScaleBegin` (`ReaderView.java:521`) hace `mXScroll = mYScroll = 0`
  con el comentario: *"Ignore any scroll amounts yet to be accounted for: the
  screen is not showing the effect of them, so they can only confuse the user"*
  — descarta restos de pan previos no visualizados para que el gesto arranque
  limpio.
- **pdf.js** (`pdf_viewer.js:1647-1656`) — mismo invariante en coordenadas del
  contenedor:
  ```js
  const scaleDiff = newScale / previousScale - 1;
  const [top, left] = this.containerTopLeft;
  dx -= (origin[0] - left) * scaleDiff;
  dy -= (origin[1] - top) * scaleDiff;
  ```
  Y el pan del gesto se aplica **en el mismo scroll update** que el origen
  ("a single scroll update only loses the fractions once", comentario en
  `pdf_viewer.js:1641`): dos updates separados perderían fracciones de píxel
  dos veces.
- **Librera** (`PdfSurfaceView.java:89-101`) — anclaje al **centro de
  pantalla** (no al foco; más simple, el centro nunca se mueve):
  ```java
  final float ratio = newZoom / oldZoom;
  final float halfWidth = getWidth() / 2.0f;
  final int x = (int) ((getScrollX() + halfWidth) * ratio - halfWidth);
  final int y = (int) ((getScrollY() + halfHeight) * ratio - halfHeight);
  ```
- **KOReader** (`Geom:centerWithin`, `geometry.lua:364`) — zoom centrado en un
  punto con clamp integrado: `x = x_centro − w/2`, luego recorte a los bordes.
- **Slint** (`examples/native-gestures/native-gestures.slint`) — anclaje
  declarativo: `transform-origin: { x: gesture.center.x − visible-area.x, ... }`
  sobre el `ScaleRotateGestureHandler`; el renderer mantiene fijo el punto. Al
  terminar, acumula `TransformData.base-scale *= gesture.scale` (factor
  relativo al gesto, mismo patrón que PDFLector).

### 3.3 Clamp del pan a los bordes de la página

- **KOReader** (`Geom:offsetWithin`, `geometry.lua:331-355`) — el patrón canónico:
  ```lua
  -- encoger el viewport si es más grande que la página
  if self.w > rect_b.w then self.w = rect_b.w end
  if self.h > rect_b.h then self.h = rect_b.h end
  self.x = self.x + dx; self.y = self.y + dy
  -- recortar a los bordes
  if self.x < rect_b.x then self.x = rect_b.x end
  if self.y < rect_b.y then self.y = rect_b.y end
  if self.x + self.w > rect_b.x + rect_b.w then self.x = rect_b.x + rect_b.w - self.w end
  if self.y + self.h > rect_b.y + rect_b.h then self.y = rect_b.y + rect_b.h - self.h end
  ```
  Idéntico en intención a `clamp_pan` de PDFLector (`reader.rs:927`, con el
  desplazamiento del centrado en X). La diferencia: KOReader clampea el
  viewport **después de cada operación de pan o zoom** (único punto de
  entrada), PDFLector clampea solo el pan de anclaje en `set_zoom_fast`.
- **MuPDF** — doble mecanismo: clamp duro en `getScrollBounds` +
  **snap-back animado** (`ReaderView.java:874`):
  ```java
  private void slideViewOntoScreen(View v) {
      Point corr = getCorrection(getScrollBounds(v));
      if (corr.x != 0 || corr.y != 0) {
          mScrollerLastX = mScrollerLastY = 0;
          mScroller.startScroll(0, 0, corr.x, corr.y, 400);
          mStepper.prod();
      }
  }
  ```
  Al soltar (`onTouchEvent`, ACTION_UP, `ReaderView.java:555-565`): si el
  scroller inercial terminó, `slideViewOntoScreen(v)` anima la página de vuelta
  a los límites en 400 ms y después `postSettle(v)` dispara el re-render HQ
  ("When the layout has settled ask the page to render in HQ",
  `onSettle`, `ReaderView.java:967`). El gesto se ve con "resorte" en vez de
  corte seco.
- **Fling con margen** (MuPDF, `ReaderView.java:455-465`): el fling inercial
  usa `FLING_MARGIN` y solo se lanza si el borde en la dirección de viaje ya
  está dentro de límites (`withinBoundsInDirectionOfTravel`): evita el
  "rebote" inercial contra un borde.

### 3.4 "Escalar y renderizar": fast path borroso + re-render nítido

- **MuPDF — el patrón HQ patch más completo** (`PageView.java`):
  - Durante el pinch, el bitmap completo (`mEntire`) se escala por **GPU** con
    `ImageView.setImageMatrix` (`onLayout`, `PageView.java:414`): barato y
    borroso.
  - `onScaleBegin` → `removeHq()` (`ReaderView.java:974`): se quita el patch
    nítido anterior.
  - `updateHq()` (`PageView.java:453`) re-renderiza **solo la región visible**
    a resolución exacta en un `CancellableAsyncTask`, y **cancela el render en
    vuelo** si llega uno nuevo (`mDrawPatch.cancel()`, `PageView.java:487`).
  - **Swap de bitmaps para no parpadear** (`PageView.java:491-497`): el bitmap
    viejo se muestra mientras el nuevo se dibuja en background; al terminar se
    coloca el nuevo (`mPatch.setImageBitmap(mPatchBm)`). El área se valida con
    `patchArea.equals(mPatchArea) && patchViewSize.equals(mPatchViewSize)`
    para no re-renderizar lo mismo dos veces.
- **pdf.js — re-render debounced durante el pinch** (`pdf_viewer.js:1602-1614`):
  ```js
  const postponeDrawing = drawingDelay >= 0 && drawingDelay < 1000;
  this.refresh(true, { scale: newScale, drawingDelay: postponeDrawing ? drawingDelay : -1 });
  if (postponeDrawing) {
      this.#scaleTimeoutId = setTimeout(() => {
          this.#scaleTimeoutId = null;
          this.refresh();
      }, drawingDelay);
  }
  ```
  Con `defaultZoomDelay = 400 ms` (`app_options.js:267`): si el gesto se
  detiene 400 ms, el re-render nítido entra **a mitad de gesto**; si sigue,
  el timeout se reinicia. Es el término medio entre "solo al soltar" (actual
  de PDFLector) y "re-render en cada Move" (lag).
- **Librera — flag `committed`** (`AbstractEventZoom.java:47-53` +
  `AbstractViewController.java:194`): durante el gesto (`!committed`) el evento
  solo hace `invalidateScroll(newZoom, oldZoom)` (reescalar lo existente); al
  soltar (`commit()` en `ZoomModel.java:68`) se dispara el re-decode a la
  nueva `PageTreeLevel` (nivel de detalle por zoom). El mismo par
  fast/committed que PDFLector, con la diferencia de que Librera también
  reescala con `invalidateScroll` por evento sin re-decodificar.
- **PDFLector hoy**: fast path por CPU (vecino-más-cercano, `blit_fast`) y
  re-render nítido único al soltar (`set_zoom_sharp`). El escritorio ya tiene
  el "ladder" de niveles (bilinear inmediato + nítido asíncrono a
  `scale_level_for_zoom`); Android no tiene equivalente del `drawingDelay`.

### 3.5 Límites min/max y redondeo

| Lector | Mínimo | Máximo | Nota |
|--------|--------|--------|------|
| PDFLector | 1.0 (fit-page) | 8.0 | `lib.rs:417-418`; sin zoom-out deliberado |
| pdf.js | 0.1 | 25.0 | `ui_utils.js:19-20`; escala relativa a fit-width |
| MuPDF | 1.0 | 64.0 | `ReaderView.java:45-46` |
| Librera | 0.5 | 32.0 | `ZoomModel.java:11-13`; permite zoom-out |
| KOReader | — (modos) | — | `kopt_zoom_factor = 1.5` por defecto en pan mode |

- pdf.js aplica el clamp **después** del redondeo a 2 decimales
  (`updateScale` → `MathClamp(newScale, MIN_SCALE, MAX_SCALE)`) y evita
  re-render cuando el cambio es < `1e-15` (`#isSameScale`,
  `pdf_viewer.js:1519`).
- Librera tiene redondeo de zoom configurable (`ZOOM_ROUND_FACTOR`), 0 por
  defecto: redondear sin acumulador (3.1) introduce deriva.

### 3.6 Pan y gestos complementarios

- **KOReader pan por % de viewport** (`readerpanning.lua:46-51`): `dx *
  panning_steps.normal * visible_area.w * 1/100` — velocidad consistente
  independiente del zoom (un paso = 1% del ancho visible), patrón útil para
  botones/d-pad.
- **KOReader pinch → modos de zoom** (`readerzooming.lua:285-294`): el pinch
  diagonal/horizontal/vertical cambia a modo page/pagewidth/pageheight; el
  zoom libre continuo está disponible pero no es el gesto principal (e-ink sin
  multitáctil fiable). Decisión de diseño: en pantallas lentas, gestos
  discretos y predecibles > zoom analógico.
- **MuPDF fling**: `mScroller.fling(0, 0, velocityX, velocityY, bounds.left,
  bounds.right, bounds.top, bounds.bottom)` — pan inercial con límites del
  scroller, combinado con `withinBoundsInDirectionOfTravel` + `FLING_MARGIN`.
- **Slint**: `gesture.scale`/`gesture.rotation`/`gesture.center` y callbacks
  `started`/`ended`/`cancelled` — la API que consumiría la UI Slint si se
  elige en Fase 6.

## 4. Aplicable a PDFLector (qué arreglar, con prioridad)

Estado actual: el núcleo (factor relativo, anclaje, clamp, fast+sharp) ya está
implementado y es estructuralmente correcto — está a la altura de los
referentes. Las mejoras son de **calidad percibida** y de **casos límite**,
no de arquitectura.

### P1 — Re-render nítido debounced DURANTE el pinch (pdf.js `drawingDelay`)

- **Problema**: hoy el zoom intermedio se ve borroso (nearest en CPU) durante
  todo el gesto; si el usuario se detiene a medio zoom sin soltar (común al
  buscar nitidez), la borrosidad se mantiene indefinidamente.
- **Fix**: en `set_zoom_fast`, si `zoom` no cambia durante ~400 ms (un timer
  que se reinicia con cada Move), lanzar el re-render nítido a la escala
  actual igual que `set_zoom_sharp` — sin cambiar el modelo (el siguiente Move
  del pinch vuelve al fast path). Es un timer adicional en `Reader`, sin tocar
  el pipeline del blit.
- **Cómo medir**: frame time p95 durante un pinch sostenido (overlay debug) y
  tiempo hasta la primera imagen nítida con el dedo quieto (objetivo < 500 ms
  desde el último Move).

### P1 — Render solo de la región visible y cancelación del render en vuelo (MuPDF HQ patch)

- **Problema**: `set_zoom_sharp` re-renderiza la página COMPLETA a la nueva
  escala; con zoom-in fuerte la región visible es un recorte y el resto es
  trabajo y RAM desperdiciados. Además, si el usuario vuelve a hacer pinch
  mientras se re-renderiza, el render en curso compite con el fast path.
- **Fix (a medio plazo)**: re-render del **rect visible** en un hilo de fondo
  (patrón MuPDF): cancelar el render anterior (la caché ya tiene cancelación
  cooperativa en escritorio), mostrar el bitmap viejo mientras el nuevo se
  dibuja, swap al terminar. MuPDF valida que el área no haya cambiado antes de
  re-renderizar.
- **Cómo medir**: RSS durante zoom-in a 4× con página 500 pág. (objetivo
  actual < 150 MB) y tiempo hasta patch nítido tras soltar.

### P2 — Snap-back animado al soltar fuera de los bordes (MuPDF `slideViewOntoScreen`)

- **Problema**: el clamp de `set_zoom_fast` mantiene los bordes dentro durante
  el gesto, pero al soltar con la página parcialmente fuera de pantalla el
  corte es instantáneo (la última posición del dedo queda como pan final).
  MuPDF anima el retorno en 400 ms y solo después re-renderiza en HQ.
- **Fix**: en `PointerUp` del pinch, si `clamp_pan` recortó el pan, animar
  (interpolación lineal ~200-400 ms en `Reader::tick`) hasta el pan clampeado
  y entonces `set_zoom_sharp`. También evita el doble re-render (animación +
  sharp a la vez).
- **Cómo medir**: subjetivo (grabación de pantalla); frame time p95 durante la
  animación (< 16,6 ms).

### P2 — Compensación de deriva del foco (MuPDF `mLastScaleFocus`)

- **Problema**: PDFLector ancla al centro del pinch del `PointerDown`; si los
  dedos se desplazan (el centro se mueve) durante el gesto, el punto bajo los
  dedos deriva respecto del punto anclado — sutil, pero perceptible en gestos
  largos.
- **Fix**: en `Move`, además del factor, sumar al pan el desplazamiento del
  centro del pinch respecto del Move anterior (dos líneas, mismo modelo que
  MuPDF), reclampeando después.
- **Cómo medir**: overlay que pinte el punto de documento bajo el centro del
  pinch durante el gesto: debe permanecer fijo en pantalla a ±1 px.

### P2 — Confirmar que no se pierden fracciones de píxel entre Moves (pdf.js `panBy`)

- **Problema**: pdf.js documenta explícitamente que el scroll debe conservar
  las fracciones de píxel entre Moves o el contenido "drifta" del gesto.
- **Estado**: PDFLector guarda `pan_x/pan_y` en f32 y redondea solo en el blit
  (`blit_fast` y `blit`), así que las fracciones se conservan. Verificar con
  un test que pinch-out/in largo devuelve el pan inicial exacto (no hay test
  hoy). Si se introduce redondeo de `zoom` (p. ej. para persistir limpio),
  añadir acumulador tipo `_accumulateFactor`.

### P3 — Fling/pan inercial con un dedo en modo zoom (MuPDF fling + margen)

- Solo si el visor vuelve a tener pan con un dedo (hoy es página a página sin
  pan). El patrón a copiar: fling limitado a los bordes con `FLING_MARGIN` y
  `withinBoundsInDirectionOfTravel` para no rebotar contra un borde.

### P3 — Zoom-out por debajo de fit-page / modos fit-width

- `PINCH_MIN = 1.0` es una decisión documentada del autor. Si algún día se
  quiere (fit-width en horizontal, página pequeña en vertical), los modos de
  KOReader (page/pagewidth/pageheight + manual) son la referencia de diseño;
  MuPDF y Librera sí permiten zoom-out (0.5-1.0). Requiere decisión del autor
  (§6 de AGENTS.md), no asumir.

### No aplica (ya correcto en PDFLector)

- **Factor relativo a la distancia inicial** — más robusto que incremental;
  sin deriva por construcción. No cambiar.
- **Anclaje al centro del pinch** — equivalente a los referentes; solo falta
  la compensación de deriva (P2).
- **Clamp de pan** — mismo invariante que KOReader `offsetWithin`.
- **Fast path separado del re-render** (`blit_fast` vs `set_zoom_sharp`) —
  el mismo par fast/committed que Librera y MuPDF.
- **f32 continuo sin redondeo** — evita la clase de bugs que pdf.js combate
  con `_accumulateFactor`.

## 5. Pendiente de investigación (si se retoma)

- Verificar el suavizado interno de `android ScaleGestureDetector.getScaleFactor()`
  (media móvil de la razón de distancias) frente al factor relativo crudo de
  PDFLector — si la tablet reporta Moves ruidosos, un promedio de
  `dist/start_dist` puede reducir el "temblor" del zoom.
- MuPDF `getDrawPageTask` renderiza el patch con recorte de página: estudiar
  si MuPDF (crate `mupdf`) permite renderizar un sub-rect de página
  (`FZ_*` clip) para el HQ patch — hoy `MupdfDocument::render_page` no lo
  expone.
