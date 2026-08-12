# Ingeniería inversa de Evince — arquitectura de rendimiento y mapeo a Android

> **Fecha**: 2026-08-10
> **Fuente**: código fuente de Evince (GNOME, rama `main`) — `libview/ev-view.c`,
> `libview/ev-job-scheduler.c`, `libview/ev-jobs.c`, `libview/ev-pixbuf-cache.c`,
> `libview/ev-page-cache.c`, `libview/ev-document-model.c`,
> `backend/pdf/ev-poppler.c`. Todas las afirmaciones citan fichero:línea.
> **Propósito**: extraer los patrones de rendimiento de Evince y traducirlos al
> stack de PDFLector (Fase 0.5 → ADR-001, Fase 1 → métricas en tablet).

---

## 1. Resumen de la arquitectura de Evince (lo relevante para rendimiento)

**Stack**: GTK4 + `libdocument` (interfaz `EvDocument`) + `libview` (widget
`EvView`). Backend PDF: **Poppler** (GPL-2+) renderizando vía Cairo, 100 %
software. La GPU solo compone texturas (GDK GL); los shaders de
`libview/shader/` son transiciones de presentación de diapositivas, **no**
render de páginas.

### 1.1 Pipeline de render

```
EvView (snapshot, ev-view.c:4869)
  └─ calcula páginas visibles por intersección con el viewport   ev-view.c:681
  └─ ev_pixbuf_cache_set_page_range(start, end, scale, rotation) ev-view.c:792
       └─ update_range → crea/cancela/re-prioriza EvJobRenderTexture  ev-pixbuf-cache.c:498
            └─ Scheduler: 1 hilo + 4 colas (URGENT/HIGH/LOW/NONE)     ev-job-scheduler.c:40-52
                 └─ ev_document_render() con mutex GLOBAL doc+fc     ev-jobs.c:826-836
                      └─ pdf_page_render: surface ARGB32 del tamaño destino
                         (escala×página×device_scale) → poppler_page_render  ev-poppler.c:382-424
            └─ surface → GdkTexture (gdk_memory_texture_new)         ev-jobs.c:764-789
  └─ draw_one_page: append_texture con transform (scroll+zoom gratis)  ev-view.c:~4948
```

Hechos clave verificados en código:

1. **Un solo hilo de render global** (`ev_job_scheduler_init`, ev-job-scheduler.c:85)
   consume 4 colas FIFO de prioridad. Dentro del render, **mutex globales** del
   documento y de fontconfig serializan todos los renders (ev-jobs.c:826-836).
   Evince no paraleliza el rasterizado: su fluidez no viene de más hilos, sino de
   composición barata + cancelación.
2. **Render a tamaño destino, nunca a resolución nativa**: `pdf_page_render`
   crea una surface del tamaño pedido (escala × tamaño de página × device scale
   HiDPI, ev-pixbuf-cache.c:243) y hace `poppler_page_render` sobre ella
   (ev-poppler.c:382-424). **No hay tiling ni multirresolución**: página
   completa a la resolución de pantalla, siempre.
3. **Cancelación estricta**: si el job está en cola se elimina; si está
   corriendo, al terminar se comprueba `g_cancellable_is_cancelled` y se descarta
   el resultado sin entregarlo (ev-job-scheduler.c:147-189, ev-jobs.c:656,
   871). Los resultados que llegan tarde se descartan silenciosamente si están
   fuera del rango de interés (ev-pixbuf-cache.c:322-326).
4. **Dibujado con transform**: `draw_one_page` pinta la textura cacheada en el
   rect de página ya transformado por scroll/zoom. Mientras no hay textura →
   spinner (page_ready = FALSE). **Durante scroll/zoom las texturas viejas se
   escalan por la GPU hasta que llega el re-render** (progresivo por escalado,
   no por baja resolución).
5. **Zoom con límite derivado del presupuesto de caché**:
   `max_scale = sqrt(cache_size / (w·dpi·4·h·dpi))` (ev-view.c:7581): el zoom
   máximo se auto-regula para que una página a ese zoom quepa en la caché. Es
   la defensa contra OOM.

### 1.2 Gestión de memoria y caché

- **Caché de píxeles: ventana deslizante, no LRU global** (`ev-pixbuf-cache.c`):
  array de texturas del viewport (prioridad **URGENT**) + hasta
  `MAX_PRELOADED_PAGES = 3` (prioridad **LOW**) a cada lado (ev-pixbuf-cache.c:103).
  Al mover la ventana (`move_one_job`, :374) las texturas que salen del rango se
  **liberan** (:389-393); las que entran al viewport se **re-priorizan en vuelo**
  (`ev_job_scheduler_update_job`, :423-425).
- **Presupuesto por bytes, no por número de páginas**: `get_preload_size`
  (:443-495) suma el tamaño en bytes de cada página (stride×alto ARGB32) y llena
  preload a izquierda/derecha mientras quepa en `max_size`. Default:
  **50 MB** (`DEFAULT_PIXBUF_CACHE_SIZE`, ev-view.c:104). Si el viewport ya
  consume todo el presupuesto → no hay preload. Un doble límite (bytes + 3
  páginas) evita sorpresas con páginas enormes.
- **Invalidación por cambio de escala**: `check_job_size_and_unref` (:343)
  cancela el job en curso si el zoom ya no corresponde al tamaño pedido.
- **Caché de datos de página** (`ev-page-cache.c`): mappings de texto/links/
  imágenes/forms/annotaciones solo para el rango visible **±1** página
  (`PRE_CACHE_SIZE 1`, :88), prioridad NONE (:336). El texto se extrae de forma
  perezosa, solo cuando la página entra en el rango.
- **Sin caché de disco**: nada persiste entre sesiones; la memoria es solo lo
  visible + preload acotado. RSS contenido por construcción.

### 1.3 Prefetch y render progresivo

- **Prefetch**: páginas N±1..3 en ambas direcciones con el doble límite
  (bytes + nº páginas). Los jobs de preload se re-priorizan a URGENT cuando la
  página entra al viewport.
- **Thumbnails**: el sidebar y el preview de enlaces (ev-view.c:2307, prioridad
  LOW) usan `poppler_page_get_thumbnail` — el **thumbnail embebido del propio
  PDF** si existe (coste ~0), y solo si no existe se renderiza a baja escala
  (`make_thumbnail_for_page`, ev-poppler.c:489-546). Es el único mecanismo de
  "vista rápida a baja resolución" de Evince.
- **Progresivo**: texturas viejas escaladas por la GPU mientras el re-render
  está en cola; spinner solo si no hay textura alguna. No hay rendering por
  bandas ni multirresolución.

### 1.4 GPU

Rasterizado 100 % CPU (cairo/poppler → FreeType). La GPU **solo** compone:
`gdk_memory_texture_new` (ev-jobs.c:764-789) sube el bitmap a textura y GTK la
dibuja transformada cada frame. Esto es deliberado: la página es estática, el
rasterizado caro ocurre una vez por (página, zoom), y el scroll/zoom son
transformaciones de textura casi gratis. HiDPI resuelto con device scale en la
superficie, no re-render (ev-pixbuf-cache.c:243-253).

---

## 2. Mapeo de conceptos a Android (stack PDFLector)

| Concepto Evince | Implementación Evince (C) | Equivalente Android / nuestro stack |
|---|---|---|
| Render job en hilo | EvJobRender + 1 hilo scheduler, 4 colas (ev-job-scheduler.c) | Corrutina por página (Kotlin) o cola priorizada + worker en Rust (`pdf_core`); **1 hilo de render por documento** o mutex: PDFium/MuPDF no son thread-safe por documento — mismo serializado que Evince |
| Render a resolución destino | `pdf_page_render` + device scale (ev-poppler.c:382) | `FPDF_RenderPageBitmap` (PDFium) / `fz_new_draw_device` (MuPDF) sobre bitmap de tamaño `página × zoom × densidad`; nunca a resolución nativa |
| Surface ARGB32 → textura | `gdk_memory_texture_new` (ev-jobs.c:764) | Bitmap ARGB_8888 → `glTexImage2D`/`HardwareBuffer` → textura; composición con transform (scroll/zoom) sin re-render |
| Ventana deslizante por bytes | EvPixbufCache, 50 MB, preload ≤3 (ev-pixbuf-cache.c) | Caché en `pdf_core`: presupuesto por bytes (ajustar a RSS <150 MB: ~40-60 MB de píxeles a medir), eviction = liberar páginas fuera de viewport±N, `onTrimMemory` → vaciar |
| Re-priorización en vuelo | `ev_job_scheduler_update_job` (ev-job-scheduler.c:266) | Subir/cancelar jobs de vecinas que entran al viewport; cancelar jobs obsoletos al cambiar zoom/página |
| Progresivo por escalado | textura vieja escalada por GPU | Durante pinch/scroll mostrar la textura actual escalada; re-render en background al estabilizar el gesto |
| Límite de zoom por presupuesto | `max_scale = sqrt(cache/bytes_página)` (ev-view.c:7581) | Igual: limitar zoom máximo para que la página quepa en el presupuesto (o tiling solo si los datos lo piden) |
| Prefetch N±1..3 | preload byte-limitado (ev-pixbuf-cache.c:443) | Prefetch N±1 en prioridad baja, cancelable; las páginas grandes consumen presupuesto, no entradas |
| Thumbnail embebido | `poppler_page_get_thumbnail` (ev-poppler.c:489) | `FPDFPage_GetThumbnailAsBitmap` (PDFium) para sidebar/previews instantáneos |
| Texto perezoso | EvPageCache ±1, prioridad NONE (ev-page-cache.c:88) | Extracción de texto solo rango visible±1 (ya es requisito en AGENTS.md §4.7) |
| Serialización fontconfig/doc | mutex globales (ev-jobs.c:826) | Mutex por documento en `pdf_core`; un render concurrente por documento |

**Notas de licencia** (relevante para ADR-001): Poppler es GPL-2+ — no es
candidato para este proyecto. El patrón de Evince se replica con **PDFium
(BSD-3)** o **MuPDF (AGPL)**, ambos con rasterizador software al mismo estilo
(decisión pendiente, Fase 0.5). Nada de lo aprendido depende del motor.

---

## 3. Buenas prácticas directas para PDFLector

1. **Render fuera del hilo de UI, siempre**: rasterizar (PDFium/MuPDF) es lo
   caro; el hilo de UI solo compone texturas. Evince lo garantiza por diseño
   (hilo scheduler + jobs).
2. **Un solo render concurrente por documento** (serializado con mutex):
   PDFium no es thread-safe por documento; Evince también serializa. El
   paralelismo se gana con cancelación y composición barata, no con más hilos.
3. **Render a resolución de pantalla** (zoom × densidad), nunca a la resolución
   nativa de la página (Evince/Poppler hacen exactamente esto).
4. **Bitmap → textura una vez**; scroll y zoom = transformar la textura en GPU.
   Re-render solo al estabilizar el gesto (el "progresivo por escalado" de
   Evince). Objetivo 60-120 fps: el frame time de scroll no debe incluir ningún
   rasterizado.
5. **Caché de ventana deslizante por bytes** (no LRU global): viewport URGENT +
   ≤3 vecinas LOW, con presupuesto en bytes (~40-60 MB de píxeles para cumplir
   <150 MB RSS; calibrar con `dumpsys meminfo` en Fase 1). Liberar al salir de
   la ventana; vaciar en `onTrimMemory`.
6. **Cola priorizada + cancelación**: cambiar de página/zoom cancela los jobs
   obsoletos; resultados fuera del rango visible se descartan sin entregar
   (Evince: ev-pixbuf-cache.c:322, ev-job-scheduler.c:147).
7. **Prefetch N±1 cancelable** en prioridad baja, limitado por bytes; si el
   viewport consume todo el presupuesto, no prefetch (comportamiento de Evince).
8. **Límite de zoom derivado del presupuesto de caché** para blindar contra
   OOM (mismo cálculo que ev-view.c:7581).
9. **Thumbnails embebidos del PDF** para sidebar y vista rápida de enlaces —
   coste ~0 frente a re-render a baja escala.
10. **Texto perezoso**: extraer solo el rango visible±1; no cargar el texto de
    todo el documento al abrir.
11. **Medir antes de optimizar**: frame time p95 (objetivo <16,6 ms), RSS en
    tablet, y el benchmark PDFium vs MuPDF de Fase 0.5 alimentando el ADR-001.
    Ninguna decisión de caché/tiling sin esos datos (AGENTS.md §3).
12. **No introducir tiling** sin datos: Evince no lo usa; si el benchmark
    muestra que el re-render de zoom extremo es aceptable a resolución
    pantalla×zoom, no se necesita. Revisitar solo si los datos de Fase 1 lo
    exigen.

---

## 4. Qué replica PDFLector y qué mejora (resumen ejecutivo)

**Se replica tal cual**: render en hilo de fondo a resolución de pantalla;
ventana deslizante de caché con doble límite (bytes + nº páginas); cancelación
de jobs; re-priorización; límite de zoom por presupuesto; thumbnails embebidos;
texto perezoso; serialización por documento.

**PDFLector mejora respecto a Evince**: composición por GPU explícita
(HardwareBuffer/OpenGL en lugar de snapshot de GTK), presupuesto de caché
gobernado por el RSS objetivo (<150 MB, Evince usa un 50 MB fijo), y estructura
`trait RenderEngine` que permite medir ambos motores en Fase 0.5 con los mismos
patrones. La arquitectura de Evince valida las decisiones ya tomadas en
AGENTS.md §4 (render a resolución de pantalla, caché LRU por bytes, prefetch
en background, hilo de UI nunca bloqueado).
