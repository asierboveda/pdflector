# Investigación: pipeline de renderizado y caché de páginas en lectores PDF open-source

> Investigación de referencia para PDFLector. Complementa
> `docs/research/evince-architecture.md` (deep-dive de Evince ya aplicado a
> `pdf_core`). Este documento amplía el abanico con 6 proyectos de perfiles
> distintos — e-ink de bajo consumo (KOReader), escritorio KDE (Okular), el
> motor mismo (MuPDF), visor minimalista con backend MuPDF (zathura),
> Android (AndroidPdfViewer) y web (pdf.js) — y extrae las técnicas concretas
> de render/caché con cita del fichero donde se vieron.
>
> **Estado actual de `pdf_core`** (para contrastar al final, §4):
> `cache.rs` (LRU por bytes + escalera de zoom 1×/2×/4×), `prefetch.rs`
> (actor 1-worker con `std::thread` + `mpsc`), `engine/mupdf.rs`
> (`MupdfEngine` → `to_pixmap`), `zoom.rs`, `metrics.rs` (overlay de debug).
> Motor único MuPDF desde ADR-001.

## 0. Resumen ejecutivo

Las seis fuentes convergen en el mismo patrón de diseño, con variaciones de
presupuesto (memoria/CPU):

1. **Render a resolución de pantalla, re-render al cambiar de zoom, y nada más**
   — ningún proyecto renderiza a resolución máxima "por si acaso" (Evince
   `device_scale`, AndroidPdfViewer `bestQuality`, pdf.js CSS-only zoom).
2. **Dos niveles de caché**: (a) un *display list* / operator list retenido
   (parseo una vez, re-rasterizar muchas veces — MuPDF `fz_display_list`,
   pdf.js `getOperatorList`), y (b) un LRU de bitmaps finales limitado por
   **bytes** (Evince 50 MB, KOReader % de RAM libre) o por **count**
   (zathura, AndroidPdfViewer 120). Los de bajo consumo usan bytes; los de
   escritorio/web usan count + heurísticas.
3. **Un único hilo de render + cola con prioridades + cancelación**: KOReader
   (Lua single-thread, hinting), Evince (1 worker, 4 colas), zathura
   (GThreadPool + abort atómico), AndroidPdfViewer (1 Looper thread),
   pdf.js (1 worker + `AbortSignal`). El paralelismo N-por-página no aparece
   en ningún lector interactivo.
4. **Tiling para el caso "página gigante en zoom alto"**: Okular (quadtree de
   tiles, dirty-tile partial updates), AndroidPdfViewer (grid de 256 px),
   pdf.js (detail canvas + operaciones recortadas por bbox). MuPDF soporta
   tiling nativo (`fz_band_writer`, banding en `muraster`).
5. **Invalidación por zoom implícita en la clave de caché** (KOReader hash con
   zoom/rotación/rect; Okular `setSize` → `markDirty`; AndroidPdfViewer
   re-grid al cambiar zoom) y **prefetch direccional** (Evince, pdf.js).

## 1. Repos estudiados

| Repo | ⭐ | Qué aporta |
|------|-----|------------|
| [koreader/koreader](https://github.com/koreader/koreader) | 28 965 | Lector e-ink multi-formato con backend MuPDF. La referencia de bajo consumo: caché global LRU por bytes **con persistencia a disco**, render por rectángulo visible, hash multi-parámetro, pressure de memoria. |
| [mozilla/pdf.js](https://github.com/mozilla/pdf.js) | 53 730 | Visor web. Arquitectura worker + operator list (display list JS), cola de render con prioridad y prefetch direccional, CSS-only zoom, detail canvas, render parcial optimizado por bbox. |
| [DImuthuUpe/AndroidPdfViewer](https://github.com/DImuthuUpe/AndroidPdfViewer) | 8 465 | Visor Android con PDFium. Grid de tiles 256 px, caché de dos niveles (active+passive, "second chance"), render en hilo Looper único, ARGB_8888 vs RGB_565 según calidad. |
| [pwmt/zathura](https://github.com/pwmt/zathura) | 3 246 | Visor minimalista con plugins; [zathura-pdf-mupdf](https://github.com/pwmt/zathura-pdf-mupdf) (121⭐) usa exactamente el motor de PDFLector. Pool de hilos con sort de cola + abort atómico + LRU por nº de páginas. |
| [ArtifexSoftware/mupdf](https://github.com/ArtifexSoftware/mupdf) | 2 915 | El motor. Los mecanismos que hacen rápido cualquier visor MuPDF: display lists, `fz_store` (caché de recursos con scavenge), `fz_cookie` (abort cooperativo), banding. |
| [KDE/okular](https://github.com/KDE/okular) | 1 462 | Visor KDE con Poppler. El tiling más completo: quadtree de tiles, updates parciales, eviction por distancia al viewport, presupuesto desde `/proc/meminfo`. |

Complementarios (no clonados, solo consulta): [GNOME/evince](https://gitlab.gnome.org/GNOME/evince) — ya documentado en `evince-architecture.md`; [ajrcarey/pdfium-render](https://github.com/ajrcarey/pdfium-render) (695⭐) — crate Rust que PDFLector usó en Fase 0 (histórico).

## 2. Técnicas concretas encontradas

### 2.1 Caché de páginas: bytes vs nº de páginas

- **Por bytes (lectores de bajo consumo)**:
  - **Evince**: presupuesto fijo `DEFAULT_PIXBUF_CACHE_SIZE = 50 MB`
    (`libview/ev-view.c`), eviction por bytes (`ev-pixbuf-cache.c`). Ya
    documentado en `evince-architecture.md` §2.
  - **KOReader**: `DocCache` calcula el tamaño **dinámicamente desde la RAM
    libre** — `calcCacheMemSize()` = `memfree × DGLOBAL_CACHE_FREE_PROPORTION`,
    recortado entre mínimo/máximo; si el presupuesto resultante < 8 MB,
    degrada a 1 slot (caché desactivada) (`frontend/document/doccache.lua`).
    El nº de slots se deduce de `size / avg_itemsize`, donde
    `avg_itemsize = ancho·alto·(color?4:1) / 3` — el `/3` asume que el item
    medio es más pequeño que una pantalla completa.
- **Por count (escritorio/web/Android)**:
  - **zathura**: `page_cache` = array de índices con `num_cached_pages`;
    `page_cache_lru_invalidate()` evicta el índice menos reciente
    (`zathura/render.c`).
  - **AndroidPdfViewer**: `CACHE_SIZE = 120` bitmaps + `THUMBNAILS_CACHE_SIZE
    = 8`, con `Bitmap.recycle()` al evictar (`util/Constants.java`,
    `CacheManager.java`). Es **two-queue**: al cambiar de vista visible,
    `makeANewSet()` mueve el set activo a `passiveCache` ("second chance").
  - **pdf.js**: el canvas de cada página es el "cache"; se libera en idle
    (`PDFRenderingQueue` con `onIdle` → cleanup timeout).

### 2.2 Tiling / caché por tiles

- **Okular** — el más maduro: `TilesManager` (`core/tilesmanager_p.h` +
  `core/tilesmanager.cpp`) es un **quadtree**: la página se divide en una
  rejilla 4×4 y cada tile se subdivide recursivamente si supera
  `TILES_MAXSIZE` = 2 000 000 px² (~2 MP). API clave:
  - `setPixmap(pixmap, rect, isPartialUpdate)` — recorta el pixmap renderizado
    en los tiles que cubre; si el pixmap es parcial, marca los tiles cubiertos
    como *dirty* (para no pintar parciales sobre tiles ya completos, línea
    ~229).
  - `hasPixmap(rect)` — el view sabe si puede pintar una región solo con
    caché; `tilesAt(rect, PixmapTile)` — lista de tiles con pixmap que
    intersectan un rect.
  - `setSize(w, h)` — al cambiar el tamaño (zoom) **marca todos los tiles
    dirty** → re-render a la nueva resolución. `setRequest(rect,w,h)` /
    `isRequesting(...)` — registro de qué región se pidió, para **descartar
    pixmaps tardíos** de requests obsoletas.
  - `cleanupPixmapMemory(bytes, visibleRect, pageNum)` — libera primero
    pixmaps de páginas completas lejanas y luego tiles individuales
    **manteniendo los visibles** (llamado desde
    `DocumentPrivate::cleanupPixmapMemory`, `core/document.cpp`).
- **AndroidPdfViewer**: `PagesLoader` divide cada página en un **grid de
  tiles de 256 px** (`PART_SIZE`); la densidad del grid depende del zoom
  (`partWidth = PART_SIZE·ratioX / zoom` → más tiles cuanto más zoom). Solo
  se renderizan los tiles que intersectan el viewport
  (`getRenderRangeList`, `RenderRange` con `GridSize rows/cols`).
- **pdf.js**: `enableDetailCanvas` — si la página renderizada superaría
  `maxCanvasPixels` (32 MP por defecto) o `maxCanvasDim`, se pinta un
  **segundo canvas** sobre el CSS-zoomado que solo renderiza la parte
  cercana al viewport; con `enableOptimizedPartialRendering`, se recorta la
  ejecución a las operaciones cuyo bbox toca esa parte
  (`web/pdf_page_view.js` — documentación de opciones, líneas ~81-110).
- **KOReader**: sin quadtree, pero renderiza **solo el rectángulo visible**
  de la página (excerpt) en el tile (`renderOptimizedPagePartTile`,
  `frontend/document/koptinterface.lua`) y lo blitea desde el blitbuffer del
  tile (`drawContextPage` → `target:blitFrom(tile.bb, ..., rect)`).
- **MuPDF**: el motor soporta tiling nativo para impresión con presupuesto
  de RAM: `muraster.c` renderiza la página en **bandas horizontales**
  (`MURASTER_CONFIG_MIN_BAND_HEIGHT`, `max_band_memory`) con N workers.

### 2.3 Display lists (render retenido): el truco de MuPDF

- **MuPDF**: `fz_display_list` (`include/mupdf/fitz/display-list.h`) — lista
  de comandos de dibujo; se crea una vez con `fz_new_display_list` +
  `fz_new_list_device` al abrir/parsear y se re-ejecuta N veces con
  `fz_run_display_list(ctx, list, dev, ctm, scissor, cookie)` **a cualquier
  escala**. Es el mecanismo oficial para "render a resolución de pantalla
  sin re-parsear": el PDF se parsea una vez, la rasterización es barata y
  se puede recortar con el rectángulo `scissor` (equivalente a render
  parcial). El flujo completo está en `source/pdf/pdf-run.c` +
  `source/fitz/list-device.c`.
- **zathura-pdf-mupdf** muestra el patrón mínimo de uso:
  `pdf_page_render_to_buffer` (`zathura-pdf-mupdf/render.c`): crea display
  list → `fz_run_page` con `fz_scale(scalex, scaley)` → `fz_new_draw_device`
  → `fz_run_display_list` sobre el pixmap. **Nota**: zathura **no cachea el
  display list entre frames** (lo recrea en cada render) — es el caso
  simple, no el óptimo. También serializa con `g_mutex_lock(&mupdf_document->mutex)`
  porque su contexto MuPDF no es thread-safe (igual que el nuestro: el
  `MupdfDocument` vive en el TLS del hilo creador).
- **pdf.js**: el análogo exacto en JS es `getOperatorList()` en el worker
  (`src/display/api.js`): se parsea el PDF una vez a una lista de operadores
  y cada `render()` ejecuta esa lista a la escala pedida. La caché de
  "página ya dibujada" es la misma idea en el otro extremo (canvas por
  página).
- **KOReader**: su "display list" particular es el `KoptContext` (kctx)
  cacheado por página (`DocCache` con hash `kctx|...`), que permite reflow y
  re-render optimizado sin reabrir la página; y el hash de render
  (`getContextHash`, `koptinterface.lua:177`) incluye fichero, mtime,
  color/bw, render_mode, página, opciones configurables (zoom, rotación,
  gamma, márgenes), bbox y tamaño de canvas — **el zoom está en la clave**,
  así que al hacer zoom se genera una clave nueva, se re-renderiza, y el
  tile antiguo permanece en el LRU hasta ser evictado (caché multi-resolución).

### 2.4 Hilos de fondo / cola de render

- **KOReader**: UI y render en **un solo hilo** (Lua). El "background" es
  *hinting*: `hintPage` renderiza la página completa en el caché con un flag
  (`Document:hintPage`, `frontend/document/document.lua`; el `@todo` del
  propio código dice "this should trigger a background operation"). Para
  reflow sí hay hilo real (`hintReflowedPage` con precache en hilo de fondo
  y `waitForContext`). Durante el pinch-zoom renderiza solo la parte visible
  ("prescaled") y al soltar hace hinting de la página completa.
- **Evince**: 1 worker + 4 colas de prioridad (URGENT/HIGH/LOW/NONE) +
  cancelación con `GCancellable` — ver `evince-architecture.md` §1.
- **zathura**: `GThreadPool` con **sort function sobre la cola**
  (`g_thread_pool_set_sort_function(priv->pool, render_thread_sort)`,
  `zathura/render.c:95`): los jobs abortados van primero para sacarlos de la
  cola sin ejecutarlos. Cada request lleva una lista de `render_job_t` con
  `atomic_bool aborted`; `zathura_render_request_abort()` marca todos los
  jobs en vuelo; el job comprueba el flag en puntos de control (antes de
  render, antes de recolor, antes de emitir la señal de completado).
  La prioridad de ordenación adicional es `last_view_time` (recencia de
  visualización).
- **AndroidPdfViewer**: un único `RenderingHandler extends Handler` sobre un
  `HandlerThread` dedicado; los render tasks llegan como mensajes
  (`MSG_RENDER_TASK`); `cacheOrder` (contador monótono por set visible) es
  el comparador de prioridad del `CacheManager` (los sets más nuevos
  desalojan primero del passive cache).
- **pdf.js**: render en el web worker; la cola de prioridad es
  `PDFRenderingQueue::getHighestPriority` (`web/pdf_rendering_queue.js`):
  1. páginas visibles, 2. detail views de visibles, 3. prefetch direccional
  (la siguiente si se hizo scroll abajo, la anterior si arriba), 4. con
  `preRenderExtra`, una página más en la misma dirección. Estados por página:
  INITIAL → RUNNING → PAUSED → FINISHED; una vista PAUSED (render abortado)
  se reanuda, no se re-encola. Cancelación vía `AbortSignal` +
  `RenderingCancelledException`.
- **MuPDF**: `fz_cookie` (`include/mupdf/fitz/device.h`) — `abort`,
  `progress`, `progress_max` — el primitivo de **abort cooperativo** del
  motor: el render comprueba `cookie->abort` entre operaciones.

### 2.5 Prefetch de páginas vecinas

- **Evince**: direccional (DOWN pide las siguientes antes que las
  anteriores) + `MAX_PRELOADED_PAGES = 3` por lado, todo recalculado por
  bytes en cada scroll — `evince-architecture.md` §2.3.
- **pdf.js**: prefetch direccional 1-2 páginas en el sentido del scroll
  (ver §2.4).
- **AndroidPdfViewer**: `PRELOAD_OFFSET = 20` dp alrededor del viewport
  (`util/Constants.java`).
- **KOReader**: hinting de páginas adyacentes tras el gesto (el flujo
  `PageUpdate` → recálculo de páginas → hintPage).
- **PDFLector ya tiene esto**: `prefetch.rs` (actor 1-worker). El informe de
  Evince ya lo anotó como implementado.

### 2.6 Resolución de pantalla vs máxima

- **Evince**: render al `device_scale` actual; el zoom máximo admisible se
  acota por bytes (`max_scale = sqrt(cache_size/(w·dpi·4·h·dpi))`) —
  `evince-architecture.md` §0.
- **AndroidPdfViewer**: el tamaño de render viene del viewport
  (`renderingTask.width/height`); `bestQuality ? ARGB_8888 : RGB_565` —
  los renders baratos (preview/miniatura) usan **16 bits/pixel**.
- **pdf.js**: `maxCanvasPixels` por defecto 32 MP; **CSS-only zooming**
  (si `maxCanvasPixels = 0`): entre re-renders, el canvas se escala con
  transform CSS/GPU — el zoom se ve instantáneo y el re-render real se
  pospone hasta que el canvas se queda pequeño para la escala actual.
- **KOReader**: el zoom forma parte del hash → render exacto a la resolución
  pedida; e-ink BW = 1 byte/pixel en el cálculo del tamaño medio del caché.
- **MuPDF**: `fz_scale` en la matriz del display list — la rasterización se
  hace al tamaño final pedido, no a resolución interna; el store cachea
  recursos (imágenes, glifos) independientemente de la escala.

### 2.7 Placeholder durante el render

- **Evince**: **no dibuja nada** si la página no está (no hay placeholder) —
  `evince-architecture.md` §1.2.
- **AndroidPdfViewer**: usa `RGB_565` para previews baratos (quality
  downgrade en vez de placeholder); el bit `thumbnail` distingue miniaturas
  (caché separada de 8).
- **pdf.js**: el canvas anterior se mantiene y se **CSS-escala** mientras
  llega el nuevo render (placeholder implícito: lo que había, estirado).
- **KOReader**: durante el pinch, render parcial del área visible
  ("prescaled") — el usuario ve la parte que toca, borrosa/actualizándose,
  y al soltar se completa con hinting.

### 2.8 Invalidación al hacer zoom

| Proyecto | Mecanismo de invalidación |
|----------|---------------------------|
| KOReader | Zoom en el **hash de la clave** → clave nueva, re-render; el viejo tile sobrevive en el LRU (multi-resolución). |
| Okular | `TilesManager::setSize(w,h)` → `markDirty()` de todos los tiles → re-render a la nueva resolución; `setRequest` descarta pixmaps de requests obsoletas. |
| AndroidPdfViewer | Re-grid automático al cambiar zoom (más tiles) + `cacheOrder` nuevo → los tiles viejos quedan en passive cache. |
| pdf.js | Comparación de escala en `update()`; si el canvas aún vale, solo CSS-transform; si no, re-render (y detail canvas si supera el máximo). |
| Evince | `check_job_size_and_unref` — mata jobs cuyo bitmap final no encaja con el nuevo size/device_scale. |

### 2.9 Extras relevantes

- **Caché en disco (KOReader)**: `DocCache` con `disk_cache = true` persiste
  los tiles renderizados en `cache/` (zstd via `Persist`, clave md5);
  al reabrir un documento, las páginas ya renderizadas se cargan de disco en
  vez de re-renderizar (`TileCacheItem:totable/dump`,
  `frontend/document/tilecacheitem.lua`; serialización al cerrar en
  `doccache.lua`). Con Syncthing de por medio es un punto a valorar.
- **Presión de memoria (KOReader)**: `DocCache:memoryPressureCheck()` — si la
  RAM se dispara, **descarta la mitad del caché** (guard contra OOM,
  `frontend/cache.lua`); citado en `koptinterface.lua` (nota sobre el issue
  #7627). El presupuesto también se recalcula desde la RAM libre.
- **Store con scavenge (MuPDF)**: `fz_store` cachea recursos pesados
  (imágenes, glifos, display lists) con presupuesto `max_store`; cuando el
  allocator no encuentra memoria, `fz_store_scavenge` evicta bajo presión
  (`include/mupdf/fitz/store.h`).
- **Eviction por distancia al viewport (Okular)**: `searchLowestPriorityPixmap`
  evicta el pixmap **más lejano de la página actual** (no el menos usado);
  los tiles visibles nunca se descartan (`core/document.cpp`).
- **Modo oscuro sin re-render**: Evince blend GPU (`GSK_BLEND_MODE_DIFFERENCE`),
  KOReader `invertRect` sobre el blitbuffer — nunca re-renderizar para
  invertir (ya anotado en `evince-architecture.md` §4.3).

## 3. Matriz de decisiones por proyecto

| Técnica | KOReader | Evince | Okular | zathura | AndroidPdfViewer | pdf.js |
|---------|:---:|:---:|:---:|:---:|:---:|:---:|
| Caché limitada por **bytes** | ✅ (RAM libre %) | ✅ (50 MB) | ✅ (proc/meminfo) | ❌ count | ❌ count (120) | ❌ por página |
| **Tiling** | rect visible | ❌ | ✅ quadtree 2 MP | ❌ | ✅ grid 256 px | ✅ detail canvas |
| **Display list** retenido | kctx | ❌ (re-render) | ❌ | ❌ (recrea cada frame) | ❌ | ✅ operator list |
| 1 hilo render + cola priorizada | ✅ (UI thread) | ✅ 4 colas | ✅ | ✅ pool + sort | ✅ Looper | ✅ worker + queue |
| **Cancelación** de render | implícita (clave) | ✅ GCancellable | ✅ late-pixmap drop | ✅ atomic abort | ✅ mensajes | ✅ AbortSignal |
| Prefetch direccional | hinting | ✅ | ✅ (preload) | ✅ | ✅ offset | ✅ |
| Placeholder / preview | render parcial | ❌ nada | ❌ | ❌ | ✅ RGB_565 | ✅ CSS-zoom |
| Inval. zoom | hash | size-check | markDirty | re-render | re-grid | scale-check |
| Caché en **disco** | ✅ zstd | ❌ | ❌ | ❌ | ❌ | ❌ |
| Modo oscuro sin re-render | ✅ invertRect | ✅ blend | ✅ | ✅ recolor | — | ✅ CSS filter |

## 4. Aplicable a PDFLector (con prioridad)

Estado actual de `pdf_core`: `RenderCache` LRU por bytes con escalera de zoom
(1×/2×/4×) en `cache.rs`; prefetch actor 1-worker en `prefetch.rs`;
`MupdfEngine` → `Document::to_pixmap` en `engine/mupdf.rs`.

### P0 — ya alineado (validar, no cambiar)

1. **Caché LRU por bytes** — correcto y común en los lectores de bajo
   consumo (Evince 50 MB, KOReader % RAM libre). *Mejora barata*: calcular el
   presupuesto desde la RAM libre como KOReader (`calcCacheMemSize`),
   manteniendo el `byte_budget` configurable; añadir un `memoryPressureCheck`
   que descarte la mitad si el RSS sube demasiado (guard OOM de KOReader).
   Medición: `adb shell dumpsys meminfo` antes/después.
2. **Prefetch direccional actor 1-worker** — mismo patrón que Evince (único
   worker, no N hilos) y coherente con el TLS de MuPDF. *Añadir*: orden de
   encolado por dirección de scroll (UP/DOWN) como Evince/pdf.js.
3. **Render a resolución de pantalla** — ya es la regla; mantenerla. La
   escalera 1×/2×/4× encaja con la idea de cache multi-resolución: al hacer
   zoom a un nivel intermedio (p. ej. 1.5×), renderizar al nivel superior y
   **escalar por GPU/CSS el bitmap** (técnica pdf.js: CSS-only zoom) mientras
   llega el render exacto — placeholder de coste cero.

### P1 — adoptar (alto valor / coste bajo)

4. **Display lists en `MupdfEngine`** (P1 alta): renderizar cada página una
   vez a `fz_display_list` y re-ejecutar con `fz_run_display_list` a cada
   escala de la escalera. Es el mecanismo nativo de MuPDF (el propio motor
   documenta "create once, run many times"), hace el re-render por zoom
   mucho más barato que re-parsear, y habilita el render parcial con
   `scissor`. El crate `mupdf` 0.8 expone display lists
   (`Page::to_display_list` / `DisplayList::run`); verificar API exacta en
   la versión embebida. Medición: `pdf_bench` — tiempo de render de una
   página a 1×/2×/4× con y sin display list.
5. **Render parcial con `scissor`** (P1, ligado al 4): ante zoom alto,
   renderizar solo el rectángulo visible de la página al nivel superior y
   blitear (técnica KOReader "excerpt" + MuPDF `scissor` + pdf.js detail
   canvas). Reduce el tiempo de primer pintado en zoom alto sin necesidad
   del quadtree completo de Okular.
6. **Invalidación por zoom ya resuelta, pero documentarla**: la clave
   `PageKey` debe incluir nivel de escala; al cambiar zoom, los bitmaps
   antiguos quedan en el LRU (multi-resolución) en vez de invalidarse
   agresivamente. El `lru` crate ya la maneja si la clave es
   `(page, scale_level)`.
7. **Cancelación de render en el worker de prefetch**: marcar el job como
   cancelado al cambiar de página/zoom y comprobar el flag entre etapas
   (set-up / render / post), como zathura (`atomic_bool aborted` en puntos
   de control) y Evince (`GCancellable`). Con display lists, MuPDF ofrece
   además `fz_cookie.abort` para abortar a medio render. Esto elimina el
   "frames tardíos" cuando el usuario salta de página con N renders en cola.

### P2 — evaluar (coste mayor, decidir con datos)

8. **Tiling real** (Okular quadtree / grid 256 px): solo si el render de
   página completa a 2× en la tablet supera el presupuesto de frame time o
   la página entera no cabe en el presupuesto de bytes (una A4 a 2× ≈ 16 MB
   ARGB → ~9 páginas en 150 MB). Hasta entonces, render por rectángulo
   visible (P1-5) cubre el caso. Decisión diferida con medición de
   `pdf_bench` (p95 frame time en zoom alto).
9. **Caché en disco** (KOReader): los tiles renderizados podrían persistirse
   en el sidecar SQLite/Syncthing-friendly para que reabrir un PDF no
   re-renderice. Compatible con el espíritu "sync-friendly" del proyecto,
   pero añade I/O y gestión de invalidación por mtime; el hash de KOReader
   (file+mtime+opciones) es el modelo. Requiere decisión del autor.
10. **RGB_565 para previews** (AndroidPdfViewer): si se decide placeholder
    de baja calidad durante el zoom (alternativa al CSS-scaling del P0-3).
    En la tablet de tinta de PDFLector (TCL NXTPaper) el ahorro de RAM y
    tiempo es real (mitad de bytes, ~2× más rápido de rasterizar).
11. **`fz_store` / presupuesto de recursos**: el crate `mupdf` ya gestiona
    el store interno con su propio límite; verificar que
    `fz_new_context(max_store)` está expuesto para acotar la RAM del motor,
    complementario al `byte_budget` de `cache.rs`.

### Lo que NO adoptar

- **Caché por count** (zathura, AndroidPdfViewer): el presupuesto por bytes
  es estrictamente mejor para el objetivo <150 MB RSS; el count es una
  simplificación de escritorio.
- **Quadtree completo de Okular en v1**: complejidad alta para un beneficio
  que solo aparece en zoom alto; cubierto por P1-5. Revisitar con datos.
- **Recrear el display list en cada render** (zathura-pdf-mupdf): es el caso
  simple, no el óptimo; nosotros cachearemos el display list (P1-4).
- **Múltiples hilos de render N-por-página**: ningún lector interactivo lo
  hace; MuPDF no escala con hilos por la contención del contexto; mantener
  el actor 1-worker (más 1 para extracción de texto, si hiciera falta).

## 5. Cómo medir antes de adoptar (criterio AGENTS.md §8)

1. `pdf_bench` con y sin display list: render 1×/2×/4× de un PDF de 500
   páginas del corpus; métrica: tiempo medio y p95 por página.
2. Zoom alto (4×) en tablet: frame time p95 en scroll y tiempo al primer
   pintado con render parcial (scissor) vs página completa.
3. RSS en tablet tras abrir/cerrar/reabrir: impacto de caché en disco (P2-9)
   y de `memoryPressureCheck` (P0-1).
4. Overlay de debug existente (`metrics.rs`): hits/misses del LRU, bytes
   residentes, tiempo de render — ya previsto en PLAN.md §3.5.

---

> Fuentes clonadas en `/tmp/pdfresearch/` (rama `main`, `--depth 1` —
> desechables). Fechas de estudio: 2026-08-12. Estrellas: `gh search repos`
> del mismo día. El deep-dive de Evince (mismo patrón de fondo) está en
> `docs/research/evince-architecture.md`.
