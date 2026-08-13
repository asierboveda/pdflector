# Ingeniería inversa de Evince — Arquitectura de rendimiento

> Investigación de referencia para PDFLector. Evince (GNOME Document Viewer) es
> el visor de PDF de referencia en escritorio Linux; su código fuente C/GTK es
> una mina de patrones probados para scroll fluido de PDFs grandes con bajo
> consumo de RAM.
>
> **Fuentes** (rama `main`, Evince 49.alpha.1 — instalado en el sistema:
> `evince 48.4-1`):
> - `libview/ev-pixbuf-cache.{c,h}` — caché LRU de páginas
> - `libview/ev-job-scheduler.{c,h}` — pool de hilos y colas de prioridad
> - `libview/ev-jobs.{c,h}` — jerarquía de trabajos asíncronos
> - `libview/ev-view.c` — widget principal, scroll, pintado (`snapshot`)
> - `libdocument/ev-document.c` — mutex globales y API del documento
> - `backend/pdf/ev-poppler.c` — backend PDF real (Poppler + Cairo)
>
> Capturas del código en `/tmp/opencode/evince/` durante esta sesión; volver a
> descargar con `curl` desde `https://gitlab.gnome.org/GNOME/evince/-/raw/main/<ruta>`
> si se necesitan.
>
> **Actualización 2026-08-12 (tras ADR-001)**: este doc se redactó con el
> backend **PDFium** como referencia. Desde ADR-001 (Fase 0.5) el motor es
> **MuPDF** (crate `mupdf` 0.8, AGPL-3.0) y el backend PDFium fue eliminado:
> `engine/pdfium.rs`, `PDFIUM_LOCK`, `pdfium-render` y la feature `pdfium` ya
> no existen en `pdf_core`. Los patrones de Evince aquí analizados son
> agnósticos del motor y siguen vigentes; las menciones a `FPDF_*`,
> `PdfiumDocument` o `PDFIUM_LOCK` son históricas. En el código actual:
> `engine::mupdf::MupdfEngine`/`MupdfDocument` (bitmap RGBA), caché LRU por
> bytes en `cache.rs` y prefetch **actor 1-worker** en `prefetch.rs`
> (`std::thread` + `mpsc`, no tokio; `MupdfDocument` no es Send-sound: el
> contexto MuPDF vive en el TLS del hilo creador).

## 0. Resumen ejecutivo (qué copiar tal cual)

Evince **no** usa tiling, **no** usa GPU para rasterizar y **no** paraleliza por
página dentro del documento. Su fórmula es:

1. **Un único hilo de render** consume una cola priorizada de jobs (URGENT →
   HIGH → LOW → NONE). Un PDF grande con scroll rápido nunca lanza N renders
   concurrentes: hay un cuello de botella serializado deliberado, lo que evita
   que 30 páginas compitan por la CPU y produzcan frames tardíos.
2. **Caché LRU por rango visible** + **prefetch direccional** (1-3 páginas
   anteriores/posteriores, más páginas si caben en el presupuesto de bytes).
3. **Cairo ARGB32 en CPU → upload a GPU como `GdkMemoryTexture`** por frame.
4. **Cancelación cooperativa** con `GCancellable` revisado en cada job:
   cuando el usuario salta a otra página, los jobs en vuelo se matan.
5. **`max_scale` calculado por bytes** (`sqrt(cache_size / (w·dpi·4·h·dpi))`):
   el zoom máximo admisible se acota para que las páginas quepan en el
   presupuesto.

Estos cinco puntos son 1:1 traducibles al stack de PDFLector (MuPDF vía el
crate `mupdf` + hilos de fondo en `std::thread` + composición egui/Android) y
son el corazón del diseño de `pdf_core`.

## 1. Motor de renderizado y pipeline

### 1.1 Backend

```
EvDocument (abstract)              ← API uniforme
  └─ EvDocumentPoppler (backend/pdf/ev-poppler.c)
       └─ poppler-glib (bindings GLib de Poppler)
            └─ libpoppler (C++) + Cairo
                 └─ cairo_image_surface_create (ARGB32, software)
```

`pdf_document_render` (`ev-poppler.c:434`) llama a `pdf_page_render`
(`ev-poppler.c:387`), que crea un `cairo_image_surface_t` del tamaño
final en píxeles (ya con `device_scale` aplicado), escala según
`EvRenderContext`, rota, llama `poppler_page_render(page, cr)` y rellena el
fondo con `cairo_paint` blanco.

**Punto clave: todo el render es CPU-software**. Poppler no usa Skia ni GPU.
La "aceleración hardware" del visor llega después, al subir el bitmap a la
GPU como textura cada frame.

### 1.2 El flujo de dibujado por frame

`ev-view.c:4869 ev_view_snapshot` (GTK4):

```
for página i en [start_page, end_page]:
    page_texture = ev_pixbuf_cache_get_texture (cache, i)   // LRU lookup
    gtk_snapshot_append_texture (snapshot, texture, area)   // GPU compositing
```

Si la página no está lista (`page_texture == NULL`), `draw_one_page` marca
`page_ready = FALSE` y la vista se queda en blanco para esa página hasta que
llegue el `job-finished`. **No hay placeholder, no hay página en baja
resolución**: la página no renderizada no se dibuja.

### 1.3 Hilos

`ev-job-scheduler.c` mantiene **un solo hilo worker**
(`g_thread_new ("EvJobScheduler", ev_job_thread_proxy, NULL)`) que consume
jobs de 4 colas FIFO (`GQueue`):

| Cola                | Uso                                            |
|---------------------|------------------------------------------------|
| `EV_JOB_PRIORITY_URGENT` | Páginas en rango visible (`start_page..end_page`) |
| `EV_JOB_PRIORITY_HIGH`   | Miniaturas en rango visible                     |
| `EV_JOB_PRIORITY_LOW`    | Páginas adyacentes (prefetch)                   |
| `EV_JOB_PRIORITY_NONE`   | Load, save, print, find…                        |

`ev_job_queue_get_next_unlocked` (`ev-job-scheduler.c:69`) siempre barre
URGENT → HIGH → LOW → NONE en busca del primer job disponible. **Evince no
crea un hilo por página**: serializa con un `GMutex` y duerme con
`g_cond_wait` si la cola está vacía.

`ev_job_update_job` (`ev-job-scheduler.c:266`) permite **subir** la
prioridad de un job en cola: si el usuario hace scroll rápido y luego se
detiene, los jobs LOW de las páginas que acaba de ver se mueven a la cola
URGENT.

### 1.4 Cancelación

Cada `EvJob` lleva un `GCancellable` (`ev-jobs.c:124`). `ev_job_cancel`
(`ev-jobs.c:215`) marca `cancelled = TRUE` y dispara la cancelación. Los
puntos críticos donde el job comprueba `g_cancellable_is_cancelled`:

- `ev_job_render_texture_run` (`ev-jobs.c:871`) — justo después de `ev_document_render`.
- `ev_pixbuf_cache_check_job_size_and_unref` (`ev-pixbuf-cache.c:343`) — cuando
  cambia el tamaño (zoom), el job viejo se mata y se sustituye.

`end_job` (`ev-pixbuf-cache.c:164`) llama a `ev_job_cancel` y libera el job
del scheduler. Esto es lo que produce el comportamiento "scroll fluido": al
saltar de página, todos los jobs en vuelo de la página anterior se descartan
y el worker pasa al siguiente URGENT sin desperdiciar CPU en páginas ya
invisibles.

### 1.5 Mutex de documento

Dos `GMutex` globales en `ev-document.c:83-84`:

- `ev_doc_mutex` — protege el árbol del documento (load, páginas,
  anotaciones, links, fuentes). Se toma en TODA llamada al backend
  (`ev-poppler.c:461, 556` para `pdf_page_render` y `get_thumbnail_surface`).
- `ev_fc_mutex` — file cache / font cache de Poppler. Es un mutex separado
  porque su granularidad es distinta.

**Implicación para Android (actualizada tras ADR-001)**: MuPDF también exige
serialización, por otra vía: `mupdf::Context` clona un contexto por hilo (TLS)
y `MupdfDocument` queda ligado al hilo que lo crea (no es Send-sound), así que
el documento no se puede mover entre hilos. `pdf_core` lo resuelve con un único
worker de render (actor 1-worker en `prefetch.rs`, `std::thread` + `mpsc`),
patrón alineado con Evince. En la era PDFium el equivalente era `PDFIUM_LOCK:
Mutex<()>` global (eliminado con el backend).

## 2. Gestión de memoria y caché

### 2.1 Estructura de la caché

`EvPixbufCache` (`ev-pixbuf-cache.c:39`) tiene tres arrays de `CacheJobInfo`:

```
prev_job   [preload_cache_size]   ← páginas anteriores (LOW priority)
job_list   [start_page..end_page] ← rango visible   (URGENT priority)
next_job   [preload_cache_size]   ← páginas siguientes (LOW priority)
```

Cada `CacheJobInfo` lleva:

```c
EvJob *job;                  // job en vuelo o NULL
gboolean page_ready;
GdkTexture *texture;         // bitmap GPU-uploaded
cairo_region_t *region;      // región a redibujar (dirty rect)
int device_scale;            // para invalidar al cambiar DPI
EvRectangle target_points;   // selección/anotación
```

### 2.2 Cálculo del rango y `preload_cache_size`

Presupuesto total: `max_size` en bytes (por defecto **50 MB**,
`ev-view.c:104 DEFAULT_PIXBUF_CACHE_SIZE`).

`ev_pixbuf_cache_get_preload_size` (`ev-pixbuf-cache.c:443`) hace:

```
range_size = Σ tamaños de páginas visibles
while range_size < max_size AND preload_cache_size < MAX_PRELOADED_PAGES (3):
    añade página siguiente i=1 → si cabe, +
    añade página anterior i=1 → si cabe, +
    i++
```

**El rango de páginas y la profundidad de preload se recalculan en cada
scroll** mediante `ev_pixbuf_cache_update_range` (`ev-pixbuf-cache.c:498`).
Este método:

1. Reorganiza los `CacheJobInfo` existentes (los que siguen visibles se
   mueven; los que se salen se descartan).
2. **Libera texturas** de páginas que ya no están en rango mediante
   `g_clear_object (&job_info->texture)`.
3. Reasigna prioridades: lo que pasa a URGENT se mueve de cola
   (`ev_job_scheduler_update_job`).

### 2.3 Scroll direccional (prefetch inteligente)

`ev_pixbuf_cache_get_scroll_direction` (`ev-pixbuf-cache.c:784`) detecta si
el usuario va hacia arriba o hacia abajo comparando rangos:

```c
if (start_page < pixbuf_cache->start_page) return SCROLL_DIRECTION_UP;
if (end_page   > pixbuf_cache->end_page)   return SCROLL_DIRECTION_DOWN;
```

Y en `ev_pixbuf_cache_add_jobs_if_needed` (`ev-pixbuf-cache.c:758`),
**se piden los jobs siguientes ANTES que los anteriores** cuando la
dirección es DOWN, y al revés cuando es UP. Resultado: el primer frame tras
un scroll rápido ya tiene la página siguiente en vuelo, no la anterior.

### 2.4 Invalidación por `device_scale`

Cada `CacheJobInfo` guarda el `device_scale` con el que se renderizó
(`ev-pixbuf-cache.c:652`). Al mover la ventana entre monitores con DPI
distinto (`notify::scale-factor`), `gtk_widget_queue_resize` dispara
`ev_pixbuf_cache_set_page_range` → `check_job_size_and_unref`
(`ev-pixbuf-cache.c:343`), que mata los jobs cuyo bitmap no encaja y
solicita nuevos. El texto se ve nítido inmediatamente después del cambio de
monitor.

### 2.5 Sin tiling

Evince **renderiza siempre la página entera** al `device_scale` actual. No
usa tiles ni teselado. El comentario en `ev-pixbuf-cache.c:339` lo deja
claro: "This checks a job to see if the job would generate the right sized
pixbuf given a scale. If it won't, it removes the job and clears it to
NULL." — un solo bitmap por página.

Para una tablet de 10-11" a ~200 dpi con páginas A4, una página cabe en
~3-5 MB a ARGB32 (sin compresión) → ~10-15 páginas en 50 MB de caché, lo
que encaja perfectamente con `MAX_PRELOADED_PAGES = 3` por lado + rango
visible.

## 3. Carga y paginación predictiva

### 3.1 Flujo al cambiar de página

`ev_view_change_page` → `view_update_range_and_current_page`
(`ev-view.c:760-805`):

```c
priv->start_page = max(0, current_page - (dual_page ? 1 : 0));
priv->end_page   = min(n_pages-1, current_page + (dual_page ? 0 : 1));
ev_page_cache_set_page_range (page_cache, start, end);   // datos (text, links…)
ev_pixbuf_cache_set_page_range (pixbuf_cache, start, end, selections);
if (cache tiene la página actual lista)
    gtk_widget_queue_draw (widget);
```

`ev_pixbuf_cache_set_page_range` (`ev-pixbuf-cache.c:805`) encadena:

1. `update_range` — mueve/dispone `CacheJobInfo`, recalcula `preload_cache_size`.
2. `clear_job_sizes` — invalida jobs cuyo tamaño no encaje (zoom/DPI).
3. `set_selection_list` — actualiza la selección visible.
4. `add_jobs_if_needed` — empuja jobs URGENT y LOW al scheduler.

### 3.2 Notificación de job terminado

`job_finished_cb` (`ev-pixbuf-cache.c:315`) emite la señal
`job-finished` del pixbuf cache. `ev-view.c:8348` la conecta a
`job_finished_cb` del view, que llama a `gtk_widget_queue_draw` (un
invalidate del área sucia) → el próximo frame pinta la textura nueva.

**La señal es el único camino UI ↔ worker**. No hay polling, no hay locks
en el hot path del dibujado.

### 3.3 Sin renderizado progresivo

Evince **no muestra baja resolución mientras se renderiza la alta**. Si la
textura no está, no se dibuja nada (`draw_one_page` retorna con
`page_ready = FALSE`). El thumbnail de Evince es una cosa separada y a
otro tamaño; no se usa como placeholder.

Para tablets con pluma y PDFs grandes, mostrar la página en blanco durante
~30 ms mientras se renderiza a 2x DPI es aceptable si la caché ya contiene
la página a 1x (que cabe en 1 MB y se puede mantener más tiempo). Esto es
una optimización que PDFLector puede considerar aparte, pero no la copia de
Evince.

## 4. Optimizaciones de GPU y renderizado gráfico

### 4.1 Pipeline CPU → GPU

```
Poppler dibuja en:   cairo_image_surface_t (CAIRO_FORMAT_ARGB32, CPU RAM)
                     ↓  cairo_surface_destroy
Job completa:        gdk_memory_texture_new(bytes, stride)   // shared bytes
                     ↓
Snapshot GTK4:       gtk_snapshot_append_texture (snapshot, texture, area)
                     ↓  GSK render node
GPU compositor:      vulkan/GL delega a GSK — Evince no elige el backend
```

`gdk_memory_texture_new` (`ev-jobs.c:780`, `ev-pixbuf-cache.c:271`) **no
copia los píxeles**: `GBytes` apunta al buffer del `cairo_surface_t`
(`cairo_image_surface_get_data`). Cuando la textura se destruye, libera el
surface. Una sola asignación de memoria por página renderizada.

### 4.2 Composición GPU

`gtk_snapshot_append_texture` (`ev-view.c:7250`) delega en GSK (GNOME Scene
Graph Kit), que sí usa GPU vía Vulkan/GL. Pero Evince **no escribe shaders
ni hace custom GL**: confía 100 % en el compositor del toolkit para blit
texturas 2D.

### 4.3 Renderizado "inverted" para modo oscuro

`draw_surface` (`ev-view.c:7235`):

```c
if (inverted) {
    gtk_snapshot_push_blend (snapshot, GSK_BLEND_MODE_DIFFERENCE);
    gtk_snapshot_append_color (snapshot, &WHITE, area);
    gtk_snapshot_pop (snapshot);
}
gtk_snapshot_append_texture (snapshot, texture, area);
```

**No se re-renderiza la página** para invertir colores — se hace una pasada
de blend en GPU (un quad blanco con `GL_ONE_MINUS_DST_COLOR`). Coste cero
en CPU; coste trivial en GPU. Esto es directamente traducible a Android con
un `Paint` con `ColorFilter` o un shader GLSL en Skia.

### 4.4 Doble buffer del compositor

GTK4/GSK mantienen un framebuffer doble/triple implícito; Evince no se
preocupa por vsync: el toolkit lo gestiona. **En Android el símil es
`Choreographer` + `SurfaceView` (doble buffer) o `GLSurfaceView`
(triple buffer)**; `pdf_app` debe entregar frames antes del vsync, no hacer
su propio timing.

## 5. Mapeo C/C++ → stack Android (PDFLector)

| Concepto Evince                  | Stack Android / PDFLector (desde ADR-001: MuPDF)         |
|----------------------------------|-----------------------------------------------------------|
| `EvDocument` (trait C)           | `pdf_core::Document` (Rust trait sobre `MupdfDocument`)  |
| `EvDocumentPoppler`              | `pdf_core::engine::mupdf::MupdfEngine` (motor único, ADR-001) |
| `EvPixbufCache`                  | `pdf_core::cache::PageCache` (LRU bytes, mutex `tokio::sync::Mutex`) |
| `EvJob` + `EvJobScheduler`       | `tokio` con `Semaphore::MAX_PERMIT = 1` para la cola de render |
| `EvJobPriority` (URGENT/HIGH/LOW)| Prioridades de `tokio` o dos colas `VecDeque<RenderJob>` |
| `GCancellable`                   | `tokio_util::sync::CancellationToken` o `Drop` del job    |
| `ev_doc_mutex`                   | Contexto MuPDF por hilo (TLS): documento ligado al hilo creador → actor 1-worker (`prefetch.rs`) |
| `cairo_image_surface_t`          | `pdf_core::Bitmap` (RGBA, CPU)                            |
| `GdkMemoryTexture` (upload GPU)  | `android.graphics.Bitmap` + `SurfaceTexture` o `Bitmap` directo a `Canvas` |
| `gtk_snapshot_append_texture`    | `Canvas::drawBitmap()` o GLES `glTexImage2D`             |
| `MAX_PRELOADED_PAGES = 3`        | Constante configurable por bytes disponibles              |
| `DEFAULT_PIXBUF_CACHE_SIZE` 50MB | `PageCache::max_bytes(RSS_DISPONIBLE * 0.4)`              |
| `device_scale` por `CacheJobInfo`| `DisplayMetrics.densityDpi` + invalidar al rotar/config cambiar |
| `gtk_widget_queue_draw`          | `View.invalidate()` en Android                             |
| `notify::scale-factor`           | `Configuration` change → `Activity.onConfigurationChanged` |

### Buenas prácticas directas para `pdf_core`

1. **Un solo worker de render, no N hilos paralelos por página**. Para
   PDFLector esto se traduce a un `tokio` task pool con `MAX_CONCURRENT_RENDERS = 1`
   (o 2 en tablets grandes). PDFium + Cairo/Poppler no escalan bien con N
   hilos compitiendo por la CPU; serializar es más predecible.
2. **Cola de prioridad explícita**. `URGENT` (rango visible) se sirve antes
   que `LOW` (prefetch). Si el usuario salta de página, los URGENT nuevos
   deben poder **mover** jobs LOW ya en vuelo (cancelar y reintentar) — esto
   ya está implementado en `ev_job_scheduler_update_job`.
3. **Caché LRU por rango visible**, no por página individual. Las páginas
   visibles se quedan aunque no se hayan usado recientemente; las lejanas
   se evictan por bytes, no por count.
4. **`preload_cache_size` calculado por bytes**. No hardcodear "N páginas
   antes/después": el coste de una página depende de su resolución efectiva
   (que depende del zoom y del DPI).
5. **Cancelación cooperativa en cada paso del render**. MuPDF
   (`fz_run_page`) no es interrumpible nativamente, pero el job
   wrapper sí: entre setup y post-process, comprobar `cancel.is_cancelled()`.
6. **Prefetch direccional**: detectar la dirección del scroll (UP/DOWN) y
   encolar primero las páginas hacia donde va el usuario.
7. **Compartir memoria CPU↔GPU**: un `Bitmap` ARGB8888 puede subirse a
   `SurfaceTexture` sin copia (`Bitmap.toString → GL_TEXTURE_2D via
   EGL_KHR_image`). En Skia/Android el símil es `Bitmap` con
   `Config.HARDWARE` y `Canvas` directo sin reasignar.
8. **Re-render al cambiar DPI, no al cambiar zoom**. Si dos zooms producen
   bitmaps del mismo tamaño final en píxeles (p. ej. zoom 0.5× a 2× DPI =
   1× a 1× DPI), reusar el bitmap. Esto es lo que hace
   `check_job_size_and_unref` comparando `(width * device_scale)` final.
9. **Modo oscuro por blend de GPU**, no por re-render. Una capa blanca con
   `PorterDuff.Mode.DIFFERENCE` invierte la página renderizada a coste
   despreciable.
10. **Serialización del documento**. MuPDF liga el documento al TLS del
    hilo creador: mantener un único worker de render (actor 1-worker en
    `prefetch.rs`) y no mover `MupdfDocument` entre hilos. En la era PDFium el
    equivalente era `PDFIUM_LOCK` (eliminado con el backend).
11. **No hacer tiling** en v1. Para una tablet de 11" a 200 dpi con A4, una
    página son 1654×2339 px ≈ 15 MB ARGB32; cabe en caché. Tiling añadiría
    complejidad sin ganancia clara en este hardware. Reevaluar si el
    presupuesto de RAM baja de 30 MB.
12. **Sin placeholder de baja resolución**. Si la página no está, no se
    dibuja (pantalla en blanco de esa zona). Es coherente con la UX de
    "scroll sin saltos" y con el hecho de que a 2× DPI una página cabe en
    <30 ms.

## 6. Lo que NO copiar de Evince

- **`g_thread_new` con un solo worker**: Evince usa un hilo de render para
  todo. En Android esto es un anti-patrón si tenemos CPU de sobra; PDFium
  escala mejor con 2 workers que con 1 serializado, hasta ~3-4 workers
  donde la contención del mutex global empieza a notarse. Medir con
  `pdf_bench`. Con **MuPDF**, en cambio, `pdf_core` usa un único worker
  (contexto TLS del hilo creador; ver la nota del encabezado) — la
  recomendación de 2+ workers era específica de PDFium.
- **`G_DEFINE_TYPE` / GObject**: Evince arrastra toda la maquinaria de
  tipos GLib. En Rust ya tenemos `trait` + enums; el `RenderEngine` trait
  de AGENTS.md §4 es el equivalente moderno.
- **`MAX_PRELOADED_PAGES = 3` hardcoded**: el número correcto depende del
  dispositivo (RAM, DPI). Calcular desde `max_bytes` dinámicamente (que es
  lo que ya hace `ev_pixbuf_cache_get_preload_size`).
- **`g_idle_add` para `emit_finished`**: en Android, `Handler(Looper.getMainLooper()).post { }`
  o `Dispatchers.Main` de Kotlin Coroutines. Mismo concepto, mejor
  ergonomía.

## 7. Mediciones pendientes (referencia para Fase 0.5 / 1)

> **Estado 2026-08-12**: la mayoría de estos puntos ya se midieron en la tablet
> TCL NXTPaper 11 Plus (render 1x 11,6-31,3 ms, RSS pico 26,7 MB; ver
> `docs/benchmark-results.md`). Queda pendiente el frame time p95 de scroll del
> harness real (android-activity, Fase 1).

Cuando llegue el momento de medir, Evince sugiere estos puntos:

- **frame time p95 en scroll** (`adb shell dumpsys gfxinfo <pkg>` →
  `Profile data in ms:`). Evince no publica métricas; asumir el objetivo
  de AGENTS.md §8: < 16,6 ms.
- **tiempo de primera página lista** desde apertura del documento. En
  Evince es ~50-100 ms con PDF pequeño (sin poppler-glib, esto incluye
  `poppler_document_new_from_gfile` + primera llamada a `poppler_page_render`).
- **rss en tablet, PDF 500 pág.**: con `max_size = 50 MB` + 5 páginas
  visibles + 3 prefetch por lado = ~60 MB de texturas. Más overhead de
  PDFium (~20 MB), Cairo, Skia. Objetivo < 150 MB → alcanzable con
  `max_size = 40-50 MB`.
- **tiempo de scroll entre dos páginas lejanas** (sin caché previa): mide
  cancelación + nuevo render URGENT.

## 8. Glosario rápido

- **`GdkMemoryTexture`**: textura GPU cuyo backing store es un buffer de
  memoria CPU compartida. Evince la usa para evitar copiar el bitmap.
- **`GtkSnapshot`**: API de GTK4 para construir el árbol de render
  inmutable de un frame (tipo React pero para píxeles).
- **`GSK`**: Scene Graph Kit — compositor GPU de GNOME (Vulkan/GL).
- **`device_scale`**: factor de escala para HiDPI. En Evince se lee con
  `gtk_widget_get_scale_factor(view)`; en Android, `densityDpi / 160`.
- **`EV_JOB_RUN_THREAD` vs `EV_JOB_RUN_MAIN_LOOP`**: en Evince algunos
  jobs (ej. búsqueda de texto) corren en el hilo principal porque son
  baratos; otros (render) en el worker. En Android el símil es
  `Dispatchers.Default` para CPU-bound, `Dispatchers.Main` para UI.

---

> Próximo paso concreto: aplicar estas decisiones a `pdf_core::cache` y
> `pdf_core::engine::pdfium` en la Fase 0.5/1, ajustando los benchmarks de
> `pdf_bench` para incluir los puntos de medición de §7.