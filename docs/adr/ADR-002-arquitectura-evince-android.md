# ADR-002 — Arquitectura de Evince: análisis para Android

> **Origen**: ingeniería inversa de Evince (GNOME PDF viewer).
> **Objetivo**: extraer patrones de diseño de rendimiento y mapearlos a Android.
> **Fecha**: 2026-08-10
>
> **Nota de motor (2026-08-12)**: este ADR se redactó tomando **PDFium** como
> implementación de referencia. Desde ADR-001 (Fase 0.5) el motor es **MuPDF**
> (AGPL-3.0): los patrones de Evince aquí analizados son agnósticos del motor,
> pero las traducciones concretas de API PDFium (`FPDF_*`, `FPDFBitmap`) son
> históricas.

---

## 1. Resumen de la arquitectura de Evince

Evince es un visor de PDFs de escritorio (C/GTK) con 20 años de evolución. Su
arquitectura separa limpiamente motor de renderizado, caché, modelo de documento
y vista. Las claves de su fluidez están en tres pilares:

1. **Renderizado asíncrono con prioridades**: nunca se bloquea el hilo de UI.
2. **Caché de texturas limitado por bytes** con prefetch inteligente.
3. **Separación de caché de píxeles y de metadatos** (texto, enlaces, anotaciones).

### 1.1 Capas de Evince

```
┌──────────────────────────────────────────┐
│  shell/ (EvWindow, EvSidebar...)         │  ← UI de aplicación
├──────────────────────────────────────────┤
│  libview/                                │
│  ├─ EvView           (GTK widget)        │  ← widget de dibujado, scroll, input
│  ├─ EvPixbufCache    (caché de texturas) │  ← texturas GPU, límite por bytes
│  ├─ EvPageCache      (caché metadatos)   │  ← texto, enlaces, formularios
│  ├─ EvJobScheduler   (planificador jobs) │  ← cola de prioridad multi-hilo
│  └─ EvDocumentModel  (estado observable)  │  ← página, zoom, rotación, modo oscuro
├──────────────────────────────────────────┤
│  libdocument/                            │
│  ├─ EvDocument       (interfaz doc)      │  ← abrir, n_páginas, render, texto
│  ├─ EvRenderContext  (contexto render)   │  ← página + rotación + escala + target
│  └─ backends/                            │
│      ├─ pdf/poppler (Poppler→Cairo)      │  ← backend principal para PDF
│      └─ ... otros                        │
└──────────────────────────────────────────┘
```

El backend Poppler (`backend/pdf/`) implementa `EvDocument.render()` llamando a
`poppler_page_render()` sobre un `cairo_t`, es decir: **Poppler interpreta el
PDF y Cairo rasteriza**.

### 1.2 Pipeline de renderizado (de PDF a píxel en pantalla)

```
Scroll del usuario
       │
       ▼
EvView.update_range_and_current_page()
  │  calcula start_page / end_page visibles
  │
  ├─► EvPageCache.set_page_range(start, end)
  │     encola EvJobPageData (metadatos, PRIORITY_NONE)
  │
  └─► EvPixbufCache.set_page_range(start, end, selections)
        │
        ├─ 1. update_range(): redimensiona arrays (job_list, prev_job, next_job)
        │     mueve jobs existentes, libera los sobrantes
        │
        ├─ 2. clear_job_sizes(): cancela jobs obsoletos (escala cambiada)
        │
        └─ 3. add_jobs_if_needed():
              ├─ job_list[i]  → EV_JOB_PRIORITY_URGENT  (páginas visibles)
              ├─ next_job[i]  → EV_JOB_PRIORITY_LOW     (prefetch adelante)
              └─ prev_job[i]  → EV_JOB_PRIORITY_LOW     (prefetch atrás)
              * Orden según dirección de scroll detectada *
                    │
                    ▼
              EvJobScheduler.push_job(job, priority)
                    │
                    ▼
              thread_pool → ev_job_render_texture_run()
                │  ev_document_doc_mutex_lock()
                │  ev_page = ev_document_get_page(document, page)
                │  rc = ev_render_context_new(page, rotation, scale)
                │  surface = ev_document_render(document, rc)
                │     └─ poppler_page_render(page, cairo_t)
                │        └─ Cairo rasteriza a superficie ARGB32
                │  texture = gdk_memory_texture_new(surface)  ← GPU texture
                │  ev_document_doc_mutex_unlock()
                │  job->page_ready = TRUE
                │
                ▼
              g_idle_add → emit_finished (en hilo principal)
                │
                ▼
              job_finished_cb()
                │  copy_job_to_job_info(job_render, job_info)
                │  g_signal_emit(JOB_FINISHED, region)
                │
                ▼
              EvView.job_finished_cb()
                gtk_widget_queue_draw(view)  ← redibuja la zona
```

### 1.3 El flujo de pintado en `draw_one_page()` (ev-view.c)

```c
draw_one_page(view, page, snapshot, page_area, border, expose_area, &page_ready) {
    texture = ev_pixbuf_cache_get_texture(pixbuf_cache, page);
    if (!texture) return;  // aún no renderizada → placeholder

    // Dibuja textura con recorte al área expuesta
    gtk_snapshot_append_texture(snapshot, texture, &bounds);

    // Capa de selección (si hay texto seleccionado)
    selection = ev_pixbuf_cache_get_selection_texture(pixbuf_cache, page, scale);
    gtk_snapshot_append_texture(snapshot, selection, ...);

    // Highlight de búsqueda
    highlight_find_results(view, snapshot, page);
}
```

Puntos clave:
- Dibuja **texturas ya cacheadas** (camino rápido, sin bloqueo).
- La selección es una textura **independiente** compuesta encima (capa).
- El highlight de búsqueda se pinta como overlay vectorial (sin re-render del PDF).

---

## 2. Áreas de rendimiento analizadas

### 2.1 Motor de renderizado y pipeline

**Qué hace Evince:**
- `EvDocument` es una interfaz GObject con método virtual `render() → cairo_surface_t*`.
- `EvRenderContext` encapsula: `page`, `rotation`, `scale`, `target_width`, `target_height`.
- `EvJob` es una tarea asíncrona ejecutada en thread pool de GLib.
- `EvJobScheduler` gestiona una cola con 4 niveles de prioridad:
  - `URGENT` (visible pages)
  - `HIGH` (thumbnails in current range)
  - `LOW` (prefetch)
  - `NONE` (load, save, print, metadata)

**Traducción a Android:**
| Concepto Evince (C/GTK) | Equivalente Android (Kotlin/Rust) |
|--------------------------|-----------------------------------|
| `EvDocument` interface | `trait RenderEngine` (ya definido en PLAN.md) |
| `EvRenderContext` | `struct RenderRequest { page, rotation, scale, target_size }` |
| `EvJob` → `GThreadPool` | `coroutine + Dispatchers.Default` o `rayon::ThreadPool` |
| `EvJobScheduler` con prioridades | `Channel<RenderRequest>` con `select` sobre prioridades |
| `ev_document_doc_mutex_lock()` | `Mutex<Document>` (Rust) o `synchronized` (Kotlin) |
| `g_idle_add → emit_finished` | `withContext(Dispatchers.Main) { callback }` |

**Patrón a replicar:**
- El backend PDF usa `poppler_page_render()` que rasteriza a Cairo. En Android,
  el motor elegido (MuPDF, ADR-001) hace lo mismo renderizando a un
  `fz_pixmap`/bitmap. La envoltura es análoga.
- El render se hace **a resolución de pantalla** (target_width/height), nunca a
  resolución nativa del PDF. Esto es exactamente lo que ya especifica PLAN.md.

### 2.2 Gestión de memoria y caching

**Qué hace Evince (`EvPixbufCache`):**

```
Estructura de arrays (NO una lista LRU genérica, sino posiciones fijas):

  prev_job[0..preload_cache_size]  ← páginas anteriores (prefetch)
  job_list[0..N]                    ← páginas visibles (cache activa)
  next_job[0..preload_cache_size]  ← páginas siguientes (prefetch)

  DONDE preload_cache_size = min(MAX_PRELOADED_PAGES(3),
      páginas que caben en (max_size - tamaño_visibles))
```

**Algoritmo de cálculo de presupuesto (`ev_pixbuf_cache_get_preload_size`):**

```c
// 1. Suma el tamaño de todas las páginas visibles
range_size = Σ page_size(i)  para i in [start_page..end_page]

// 2. Intenta añadir prefetch hasta MAX_PRELOADED_PAGES
i = 1;
while ((start_page - i > 0 || end_page + i < n_pages) && preload < MAX) {
    if (end_page + i < n_pages) {
        page_size = bytes_de(end_page + i);
        if (range_size + page_size <= max_size) { preload++; range_size += page_size; }
        else break;
    }
    if (start_page - i > 0) {
        page_size = bytes_de(start_page - i);
        if (range_size + page_size <= max_size) { preload++; range_size += page_size; }
        else break;
    }
    i++;
}
```

**Política de expulsión:**
- Al hacer scroll, `update_range()` redimensiona los arrays.
- Las páginas que salen del rango `[start - preload, end + preload]` se liberan
  inmediatamente (`dispose_cache_job_info` → `g_clear_object(&texture)`).
- No es LRU clásico: es un **sliding window estricto** sobre el rango visible.
- El límite es en bytes, no en número de páginas: `max_size = 50 MB` por defecto.

**Zoom:**
- Al cambiar escala, `clear_job_sizes()` cancela jobs con tamaño obsoleto.
- Si hay textura cacheada pero de tamaño incorrecto → se libera y se re-renderiza.
- No usa tiling: renderiza página completa a la nueva escala.

**Traducción a Android:**
| Evince | Android |
|--------|---------|
| `EvPixbufCache.max_size = 50 MB` | `LruCache<Int, Bitmap>(maxSizeBytes)` o `LinkedHashMap` con `totalBytes` |
| Tres arrays (prev, visible, next) | `ArrayDeque` o 3 listas separadas |
| `GdkTexture` (GPU) | `Bitmap` (CPU) o `HardwareBuffer`/`TextureView` (GPU) |
| `ev_pixbuf_cache_get_page_size()` | `bitmap.byteCount` |
| `dispose_cache_job_info()` al salir del rango | `bitmap.recycle()` |

**Recomendación para Android:**
- En tablet de 200 € (~4 GB RAM), el límite debería ser ~40-60 MB para las
  texturas visibles + prefetch (el resto de RAM es para sistema, app, y proceso
  de render).
- Usar `HardwareBuffer` (Android 10+) o `Bitmap.Config.HARDWARE` (Android 8+)
  para que las texturas vivan en VRAM y no consuman heap Java.
- El sliding window estricto es más eficiente que LRU tradicional porque evita
  el coste de mantener orden de acceso.

### 2.3 Carga y paginación predictiva

**Qué hace Evince:**

**A) Prefetch de texturas (píxeles):**
- `EvPixbufCache` renderiza hasta 3 páginas antes y 3 después del rango visible.
- **Dirección de scroll**: `ev_pixbuf_cache_get_scroll_direction()` detecta si
  el usuario va hacia arriba o hacia abajo y prioriza ese lado:
  ```
  Scroll UP   → primero prev_jobs, luego next_jobs
  Scroll DOWN → primero next_jobs, luego prev_jobs
  ```
- Jobs de prefetch tienen `PRIORITY_LOW`; los visibles `PRIORITY_URGENT`.

**B) Prefetch de metadatos (texto, enlaces, anotaciones):**
- `EvPageCache` precachea `PRE_CACHE_SIZE * 2` páginas (2 hacia adelante, 2 hacia atrás).
- Workflow secuencial: expande desde el rango visible hacia afuera alternando
  adelante/atrás (`while ... { end+i; start-i; i++; }`).
- Los metadatos se cargan con `PRIORITY_NONE` (mínima prioridad).

**C) Renderizado progresivo:**
- `CacheJobInfo.page_ready`: indica que la textura está completa.
- `EvJobRenderTexture.page_ready`: flag que permite obtener textura parcial
  (pero el código actual solo la marca tras render completo).
- Durante la carga, la vista pinta un fondo (placeholder CSS `document-page`).

**Traducción a Android:**
| Evince | Android |
|--------|---------|
| `ScrollDirection` | Delta Y del `RecyclerView.OnScrollListener` |
| `add_prev_jobs_if_needed()` / `add_next_jobs_if_needed()` | Lógica en `onScrolled(dy)` |
| `PRIORITY_URGENT` vs `LOW` | `CoroutineDispatcher` con límite de paralelismo o `Channel` con `select` |
| `CacheJobInfo.page_ready` | `Bitmap?` nullable: null = no listo, not null = cacheado |
| `PRE_CACHE_SIZE` metadatos | Cargar texto/enlaces asíncronamente con `Dispatchers.IO` |

**Recomendación para Android:**
- Usar `RecyclerView` con `LinearLayoutManager` en vertical para el scroll de
  páginas (aprovecha el view recycling nativo).
- El prefetch se implementa con `RecyclerView.LayoutManager.collectAdjacentPrefetchPositions()`
  o manualmente en `onScrolled()`.
- Para el caso de scroll continuo con canvas propio (no RecyclerView), implementar
  el mismo algoritmo de sliding window que Evince.

### 2.4 Optimizaciones de GPU y renderizado gráfico

**Qué hace Evince:**
1. **Renderizado software (Cairo) → textura GPU (GdkTexture)**:
   - Poppler rasteriza con Cairo a `cairo_image_surface_t` (ARGB32 en RAM).
   - `gdk_texture_new_for_surface()` envuelve los bytes en `GdkMemoryTexture`.
   - El snapshot de GTK4 sube la textura a la GPU y pinta con OpenGL/Vulkan.

2. **Sin tiling**: renderiza la página completa a la resolución de pantalla.

3. **Composición multi-capa** (en `draw_one_page`):
   ```
   Capa 0: textura de página (PDF renderizado)
   Capa 1: textura de selección (highlight)
   Capa 2: overlay de búsqueda (rectángulos coloreados, vectorial)
   Capa 3: anotaciones (ventanas GTK como overlay)
   ```
   La selección se renderiza **aparte** (no incrustada en la página) →
   cambiar la selección no requiere re-renderizar el PDF.

4. **Modo oscuro**: `ev_document_model_get_inverted_colors()` → se aplica como
   filtro CSS `inverted` sobre toda la página, no re-renderizando el PDF.

5. **Device scale**: `gtk_widget_get_scale_factor()` → se multiplica la escala
   de render para pantallas HiDPI. En Android esto se traduce a `density`.

**Traducción a Android:**

| Evince | Android |
|--------|---------|
| Cairo ARGB32 → GPU | MuPDF `fz_pixmap` → `Bitmap` (RGBA) → subir a `Canvas` o `TextureView` |
| Sin tiling | Ídem: render de página completa (evita overhead de stitching) |
| Composicion multi-capa | `Canvas.drawBitmap()` + `drawRect()` + `drawPath()` encima |
| Selección como textura separada | `Path` con `Paint` de highlight sobre el bitmap base |
| Modo oscuro vía CSS invert | `Paint.setColorFilter(new PorterDuffColorFilter(...))` o `LightingColorFilter` |
| Device scale | `context.resources.displayMetrics.density` |

**Nota sobre GPU en Android:**
- `Canvas` con `Bitmap` usa renderizado software (Skia raster) salvo que uses
  `HardwareBuffer` con `SurfaceControl`/`SurfaceView` (API 29+).
- `TextureView` usa la GPU para la composición pero la renderización la hace
  PDFium en CPU.
- Para **verdadero renderizado GPU de PDFs** haría falta Vulkan/OpenGL, pero
  PDFium es CPU. MuPDF tiene backend OpenGL experimental. No necesario para el
  objetivo de 60 fps en tablet de 200 €.

---

## 3. Mapeo directo de conceptos Evince → Android/Rust

### 3.1 Equivalencias de componentes

| Componente Evince | Responsabilidad | Equivalente en pdf_core | Implementación |
|-------------------|----------------|------------------------|----------------|
| `EvDocument` | Abrir PDF, page count, render | `trait RenderEngine` | Ya definido en PLAN.md §3.2 |
| `EvRenderContext` | Pagina, rotación, escala, target size | `struct RenderRequest` | A implementar |
| `EvJob` | Tarea asíncrona | `rayon::spawn` + `std::sync::mpsc::channel` | A implementar en `render` module |
| `EvJobScheduler` | Cola con prioridades | `rayon::ThreadPool` dedicado + `crossbeam::deque` | O usar `tokio` si se prefiere async |
| `EvPixbufCache` | Caché de bitmaps por bytes | `struct PageCache` en `cache` module | A implementar |
| `EvPageCache` | Caché de metadatos de página | `struct PageMetaCache` (texto, links, forms) | A implementar |
| `EvDocumentModel` | Estado observable (pagina, zoom, rotación) | `struct DocumentState` con canales de notificación | A implementar |
| `EvView` | Widget de dibujado y scroll | `pdf_app` (egui) ahora; `Canvas`/`TextureView` en Android | Prototipo hecho |
| `height_to_page_cache` | Mapa rápido altura→página | `Vec<f32>` de alturas acumuladas | A implementar |

### 3.2 Equivalencia de patrones de concurrencia

```
Evince:
  hilo UI (GTK main loop)
    → idle_add(callback)  ← se llama al terminar un job

  thread pool (GThreadPool)
    → ev_job_render_texture_run()  ← pesado, bloqueante
    → ev_job_page_data_run()       ← metadatos
```

```
Android (Rust/coroutines):
  hilo UI (Main dispatcher)
    → channel.recv() → actualizar UI  ← recibe resultado de render

  background threads (rayon / Dispatchers.Default)
    → render_page()  ← pesado, bloqueante
    → fetch_metadata()  ← ligero
```

**Patrón de comunicación:**

```rust
// Patrón Evince en Rust:
// 1. UI thread → envía RenderRequest por channel (no bloquea)
// 2. Background thread → ejecuta render
// 3. Background thread → envía resultado por channel de vuelta
// 4. UI thread → en el próximo frame, procesa cola de resultados

use std::sync::mpsc;

struct RenderQueue {
    tx_request: Sender<RenderRequest>,
    rx_result: Receiver<RenderResult>,
}

// En hilo principal (egui frame):
for result in rx_result.try_iter() {
    cache.insert(result.page, result.bitmap);
}
```

### 3.3 Equivalencia del algoritmo de caché

```
Evince (ev-pixbuf-cache.c):

  set_page_range(start, end):
    1. update_range()          → redimensiona arrays
    2. clear_job_sizes(scale)  → cancela jobs con escala obsoleta
    3. add_jobs_if_needed()    → encola nuevos renders

  get_texture(page):
    job_info = find_job_cache(page)
    return job_info.page_ready ? job_info.texture : NULL
```

```rust
// Equivalente Rust:
struct PageCache {
    job_list: Vec<CacheEntry>,      // visible pages
    prev_jobs: VecDeque<CacheEntry>, // preload behind
    next_jobs: VecDeque<CacheEntry>, // preload ahead
    start_page: usize,
    max_bytes: usize,
}

impl PageCache {
    fn set_page_range(&mut self, start: usize, end: usize, scale: f32) {
        // 1. Shift the window
        self.slide_window(start, end);
        // 2. Cancel jobs for wrong scale & enqueue new
        self.cancel_obsolete_jobs(scale);
        self.enqueue_missing_jobs(scale);
    }

    fn get_texture(&self, page: usize) -> Option<&Bitmap> {
        self.find_entry(page).and_then(|e| e.texture.as_ref())
    }

    fn preload_capacity(&self) -> usize {
        let visible_bytes: usize = self.job_list.iter()
            .filter_map(|e| e.texture.as_ref())
            .map(|t| t.byte_count())
            .sum();
        let remaining = self.max_bytes.saturating_sub(visible_bytes);
        // How many preload pages fit in remaining budget
        ...
    }
}
```

### 3.4 Sección `height_to_page_cache`

Evince mantiene un array `height_to_page[i]` con la altura acumulada hasta la
página `i` (en coordenadas de documento sin escala). Esto permite conversión
O(1) de coordenada Y de scroll → número de página:

```c
// EvView → get_page_y_offset:
height_to_page[page] = Σ height_of_page(j)  for j in 0..page

// Calcular offset de scroll para la página N:
offset = height_to_page[N] * scale + spacing
```

En Android, esto es **esencial** para scroll virtualizado sin RecyclerView (como
en visores PDF tipo Xodo o ReadEra). Recomendación: precalcular tras abrir el
documento y cachear en un `Vec<f32>`.

---

## 4. Buenas prácticas directas para implementación en Android

### 4.1 NUNCA bloquear el hilo de UI

```
// MAL:
override fun onDraw(canvas: Canvas) {
    val bitmap = engine.renderPage(currentPage, scale) // BLOQUEA
    canvas.drawBitmap(bitmap, 0f, 0f, null)
}

// BIEN (patrón Evince):
override fun onDraw(canvas: Canvas) {
    val bitmap = pageCache.get(currentPage) // O(1), no bloquea
    if (bitmap != null) {
        canvas.drawBitmap(bitmap, ...)
    }
    // Si bitmap == null → placeholder gris (el render está en marcha)
}
```

### 4.2 Caché de ventana deslizante, no LRU global

```
Estructura de datos:
  - visible: Array<CacheEntry>   (páginas [start..end])
  - prev: ArrayDeque<CacheEntry> (prefetch antes de start)
  - next: ArrayDeque<CacheEntry> (prefetch después de end)

Al hacer scroll:
  - Mover entradas entre los 3 arrays (O(1) por entrada)
  - Liberar entradas que salen del rango
  - Encolar renders para nuevas posiciones de prev/next

El límite es en BYTES, no en número de páginas:
  maxBytes = 40 * 1024 * 1024  // 40 MB en tablet
```

### 4.3 Renderizado a resolución de pantalla, no a resolución PDF

```kotlin
val targetWidth = (pageWidthPoints * displayDensity * zoomScale).toInt()
val targetHeight = (pageHeightPoints * displayDensity * zoomScale).toInt()
// NUNCA: pageWidthPoints * 2 (super-sample)
```

Esto es lo que hace Evince con `ev_render_context_set_target_size()`. PDFium
recibe exactamente las dimensiones de salida y renderiza a esa resolución.

### 4.4 Prefetch con dirección de scroll

```kotlin
fun onScrolled(dy: Float) {
    val direction = if (dy > 0) ScrollDirection.DOWN else ScrollDirection.UP
    pageCache.scrollDirection = direction

    val newStartPage = layoutManager.findFirstVisibleItemPosition()
    val newEndPage = layoutManager.findLastVisibleItemPosition()
    pageCache.setRange(newStartPage, newEndPage) // esto dispara prefetch
}
```

### 4.5 Separación de caché de píxeles y metadatos

```
PixbufCache (texturas):
  - Limitado en bytes
  - Expira al hacer scroll fuera del rango
  - Dependiente de la escala (invalidado al hacer zoom)

PageMetaCache (texto, enlaces, anotaciones):
  - Carga perezosa y bajo demanda
  - No depende de la escala (coordenadas de documento)
  - Precargado asíncronamente con baja prioridad
```

### 4.6 Composicion de capas para selección y anotaciones

```
En onDraw():
  1. drawBitmap(pageTexture, ...)          ← capa base (PDF)
  2. if (hasSelection) drawSelection(...)  ← capa de highlight
  3. if (hasLinks) drawLinkRects(...)      ← capa de enlaces
  4. drawAnnotations(paths, ...)            ← capa vectorial
```

Nunca rasterizar la selección dentro del bitmap de página. Si la selección
cambia, solo se invalida la región afectada.

### 4.7 Altura acumulada precacheada (O(1) scroll → page)

```rust
struct HeightCache {
    accumulated: Vec<f32>,  // altura hasta página i (sin escala)
    uniform: bool,
}

impl HeightCache {
    fn page_at_y(&self, y: f32, scale: f32, spacing: f32) -> usize {
        let doc_y = y / scale;
        // Binary search on accumulated
        self.accumulated.binary_search_by(|&h| {
            h.partial_cmp(&doc_y).unwrap()
        }).unwrap_or_else(|i| i.saturating_sub(1))
    }
}
```

### 4.8 Job scheduler con prioridades

```kotlin
sealed class RenderPriority {
    object Urgent : RenderPriority()    // visible pages
    object High : RenderPriority()      // near-visible
    object Low : RenderPriority()       // prefetch
}

class RenderScheduler(private val renderer: RenderEngine) {
    private val urgentChannel = Channel<RenderRequest>(Channel.UNLIMITED)
    private val lowChannel = Channel<RenderRequest>(Channel.UNLIMITED)

    suspend fun start() = coroutineScope {
        launch(Dispatchers.Default) {
            // Worker: urgent first, then low
            while (isActive) {
                val request = urgentChannel.tryReceive().getOrNull()
                    ?: lowChannel.receive()
                val bitmap = renderer.renderPage(request)
                _results.emit(RenderResult(request.page, bitmap))
            }
        }
    }
}
```

---

## 5. Checklist de implementación (resumen)

| # | Patrón Evince | Android |
|---|---------------|---------|
| 1 | `EvDocument.render()` → `cairo_surface_t*` | MuPDF `fz_run_page()` → `fz_pixmap` → `Bitmap` |
| 2 | `EvJobScheduler` 4 prioridades | `CoroutineScope` + 2 canales (urgent, low) |
| 3 | `EvPixbufCache` sliding window + límite bytes | `LruCache`/`LinkedHashMap` + `totalBytes` ≤ 40 MB |
| 4 | `height_to_page_cache` O(1) | `FloatArray` de alturas acumuladas |
| 5 | `ScrollDirection` prefetch | `onScrolled(dy)` + prefetch priorizado |
| 6 | Sin tiling, página completa | Render a target_width/height (resolución pantalla) |
| 7 | `EvPageCache` separado de `EvPixbufCache` | Texto/enlaces cargados aparte, baja prioridad |
| 8 | Selección/anotaciones capa separada | `Canvas` overlay, sin re-render del PDF |
| 9 | Modo oscuro vía filtro CSS | `PorterDuffColorFilter` o `LightingColorFilter` |
| 10 | `g_idle_add` → hilo principal | `withContext(Dispatchers.Main)` o `Handler.post` |

## 6. Lo que no se replica de Evince

- **Tiling de zoom**: Evince no lo usa. Para Android tampoco es necesario:
  renderizar la página completa a la nueva escala es suficientemente rápido
  (< 25 ms objetivo).
- **Backend Poppler**: usaremos MuPDF (ADR-001); el backend PDFium se eliminó en la Fase 0.5.
- **Navegación por enlaces Synctex**: no aplica al caso de uso.
- **Widgets GTK de anotaciones**: en Android serán vistas nativas o canvas overlay.

## 7. Referencias

- Evince source: https://github.com/GNOME/evince (mirror)
- `libview/ev-pixbuf-cache.h` / `.c` — caché de texturas con límite de bytes
- `libview/ev-page-cache.h` / `.c` — caché de metadatos de página
- `libview/ev-jobs.h` / `.c` — sistema de trabajos asíncronos
- `libview/ev-job-scheduler.h` — planificador con prioridades
- `libview/ev-view.c` — widget principal, scroll, pintado (931 KB)
- `libdocument/ev-document.h` — interfaz de documento
- `libdocument/ev-render-context.h` — contexto de renderizado
