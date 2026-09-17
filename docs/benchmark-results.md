# Registro de Evidencia de Rendimiento

Este documento es el **registro único y canónico de evidencia empírica** de rendimiento de PDFLector. Funciona bajo el modelo **append-only** (cronológico inverso: mediciones más recientes primero).

## Regla de oro
> **Ninguna afirmación de rendimiento sin fecha + hardware + flujo medido + métrica.**
> Si un dato no ha sido medido en hardware real, se declara explícitamente como `SIN MEDIR`. No se admiten estimaciones teóricas ni extrapolaciones como hechos.

## Cómo añadir una entrada
1. Insertar la nueva medición al principio del registro (debajo de esta cabecera).
2. Indicar fecha ISO (`AAAA-MM-DD`), hardware exacto (modelo, CPU/SoC, RAM, SO), build/commit y condiciones ambientales (pantalla ON/OFF, governor, batería).
3. Describir el flujo medido y el método de captura (`adb-bench.sh`, logcat streaming, criterion, dumpsys).
4. Documentar métricas crudas (p50, p95, RSS/PSS) sin maquillar desviaciones ni calcular fps teóricos inversos.
5. Si una medición contradice un dato previo o una decisión de diseño, documentar la discrepancia con total honestidad.

---

## 2026-09-07 — Batch TCL: Cold start, scroll de biblioteca y PSS bajo interacción

- **Hardware**: TCL NXTPaper 11 Plus (modelo 9469X, MediaTek MT8781 8× Cortex-A55, 8 GB RAM, Android 15, pantalla 1440×2200 portrait @ 320 dpi). Pantalla ON (`svc power stayon true`), verificado retorno a `stayon false` + Dozing al finalizar.
- **Build**: Release `f51d75c` (`pdf_android`, optimizaciones de velocidad de pase A–D).
- **Flujo medido**: Tres escenarios combinados en la tablet: cold start del visor restaurando página, scroll continuo en biblioteca y retención de memoria PSS en ráfaga de pases de página.

### 1. Primer frame cold-start (Viewer)
- **Método**: `am force-stop` + `logcat -c` + marcador COLDMARK + `am start`; medición hasta el primer evento `gl_present|blit`. Restaura libro de prueba *Análisis Funcional* (346 páginas, tipografía densa real) en pág. 61 (SepiaDark).
- **Métrica**: Mediana de 3 ejecuciones: **349 ms** (runs: 338, 355, 349 ms). **NO alcanza el objetivo <200 ms**.
- **Desglose run 1**:
  - Apertura del documento MuPDF (346 pp): 213 ms.
  - Inicialización de ventana nativa (`InitWindow`): 73 ms.
  - Primer `gl_present`: 52 ms (cálculo de presentación 14.1 ms, swap 2.8 ms).
- **Conclusión**: El cuello de botella reside en la apertura del PDF y la inicialización del sistema de ventanas, no en el pipeline de presentación GPU.

### 2. Scroll en biblioteca con 11 libros (E3 parcial)
- **Método**: Interacción continua con swipes en rejilla 3×3 y carousel. Medición de intervalos entre presents y tiempos de ejecución del frame.
- **Rejilla (N=535 presents en 14.1 s, 5+5 swipes)**:
  - Intervalo entre frames: mediana 8.0 ms, **p95 10.0 ms** (máximo intra-swipe 11.0 ms).
  - Coste `gl_present`: mediana 1.22 ms, p95 2.61 ms, máx 5.06 ms.
  - Swap time: mediana 0.83 ms, p95 2.20 ms.
  - Re-renders durante scroll: **0**.
- **Carousel "Seguir leyendo" (N=556 presents en 15.4 s)**:
  - Intervalo entre frames: mediana 8.0 ms, p95 10.0 ms. Coste present: mediana 1.19 ms, p95 2.58 ms.
- **Nota**: Se validó con 11 libros con overflow visual; la variante sintética de 256 portadas quedó pendiente por inviabilidad de carga manual vía UI sin harness específico.

### 3. PSS bajo interacción continua (15 pases de página)
- **Método**: 15 cambios de página consecutivos (págs. 60→75) sobre libro denso de 346 páginas. Muestreo de PSS mediante `dumpsys meminfo` en reposo y tras cada ráfaga.
- **Métricas**:
  - Latencia de turno: 15/15 completados (hits en caché: 5–8 ms; misses con render: 133–149 ms).
  - Evolución PSS total:
    - Base reposo: **234 713 KB** (~229.2 MB).
    - +5 turnos: 242 790 KB.
    - +10 turnos: 248 394 KB.
    - +15 turnos: **287 565 KB** (~280.8 MB).
    - Tras 20 s de quietud: 287 522 KB; tras 40 s: 287 518 KB (memoria asentada, sin fugas continuas).
  - Desglose final: Native Heap 176.0 MB, GL mtrack 68.7 MB, EGL mtrack 28.9 MB, Total RSS 416.6 MB.
  - Salto abrupto de +39.2 MB detectado entre turnos 11 y 15 atribuido a acumulación de display lists en MuPDF.

### 4. Menú Sheet lateral (E2)
- **Métrica**: Apertura completada en 13 presents (~130 ms, 1.04–4.06 ms c/u) con 1 creación de textura overlay 1440×924. Cierre en 13 presents (1.27–4.27 ms). **0 re-renderizados de página (`render page`), 0 desalojos de caché, 0 recreaciones de textura de página**.

---

## 2026-09-07 — Velocidad de pase de página y residencias de caché

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200). Pantalla ON.
- **Build**: Release `25a8dd7` (prefetch direccional, crop a ventana y desalojo diferido).
- **Flujo medido**: Taps secuenciales de cambio de página con pausas de 2.2 s sobre *dense_textbook.pdf* (93 pág) y *Análisis Funcional* (346 pág, tipografía real compleja).
- **Métricas**:
  - Baseline previo (sin crop a ventana, bitmap 27.4 MB en landscape): 12/12 turnos en fallo de caché a ~115 ms (0% hits por evicción inmediata en cada inserción).
  - Con crop a ventana (12.7 MB/página) y evicción diferida:
    - Serie de 15 turnos en *Análisis Funcional* (portrait): `9, 158, 8, 8, 155, 8, 6, 102, 7, 6, 6, 7, 102, 10, 10 ms`.
    - **11/15 turnos son hits en caché a 6–10 ms** (p50 ≈ 8 ms).
    - 4/15 turnos son misses con render completo a 102–168 ms.
    - Residencia mínima verificada: ≥3 páginas simultáneas en caché.
  - PSS observado durante la sesión: 190–205 MB.
- **Deuda detectada**:
  - Display lists de `MupdfDocument` acumulativas sin cota de evicción (+65 a +79 MB tras 22 turnos, ~6–7 MB/página nueva en documentos complejos).
  - 2 frames negros transitorios registrados en ~40 turnos (mitigados con guardas de fallback y validación de bitmap).

---

## 2026-09-06 — Presentación GPU Dry/Overlays y estabilidad de transiciones EGL

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Mali-G57 MC2, Android 16). Pantalla ON, batería al 7% en carga.
- **Build**: Release `f5381e9` (pipeline EGL productor único, surface persistente).
- **Flujo medido**: 10 ciclos consecutivos de transición Library ↔ Viewer, pan interactivo con stylus USI y medición de PSS en reposo.

### 1. Ciclos de transición Library ↔ Viewer (Estabilidad EGL)
- **Condición previa (build 1143e9e)**: `eglCreateWindowSurface` fallaba con error `0x3003` (`EGL_BAD_ALLOC`) en 7 de cada 10 transiciones debido a alternancia de productores (`ANativeWindow_lock` en CPU para biblioteca vs EGL en GPU para visor).
- **Resultado con productor único GPU (`f5381e9`)**:
  - **0 recreaciones de superficie EGL en 10 ciclos**.
  - **0 errores `EGL_BAD_ALLOC`** (0 fallos de superficie).
  - Coste de presentación de biblioteca por GPU: **4.6–6.7 ms** (frente a 4.2–19.4 ms en software).

### 2. Pan continuo sin re-rasterización (DryKey)
- **Métrica**: 459 frames de presentación durante arrastre continuo con stylus (`input stylus swipe`, tool_type=stylus):
  - **p50: 3.15 ms · p90: 3.64 ms · p95: 4.19 ms · máx: 17.90 ms**.
  - Re-renderizados de capa Dry durante el desplazamiento: **0** (`fbo create = 0`). El pan es pura traslación de quad con swap de buffers.

### 3. Memoria PSS
- **Métricas**:
  - Arranque: **118 MB**.
  - Pico tras 10 ciclos rápidos consecutivos: **232 MB**.
  - Reposo asentado tras interacción: **174–178 MB** (estabilizado, sin crecimiento monótono).

---

## 2026-09-05 — Subrayado y persistencia en hardware real

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200). Stylus físico USI 2.0.
- **Build**: Release `pdf_android`, arquitectura Dual FBO (Wet/Dry sobre GLES2/EGL).
- **Flujo medido**: Subrayado continuo interactivo con lápiz sobre texto real y guardado en SQLite.
- **Métricas**:
  - Presentación GPU durante el trazo (`gl_present`): **1.08–5.33 ms** (p50: 2.8 ms, **p95: 3.5 ms**).
  - Algoritmo de intersección de texto `highlight_under_gesture_sorted`:
    - Host x86 (Ryzen 7 5800H): **9.82 µs** (optimizado frente a 28.07 µs del baseline no ordenado).
    - Tablet TCL (MT8781): **< 0.1 ms**.
  - Persistencia SQLite (`save_annotations`): Ejecución asíncrona en hilo de fondo (**0 ms de bloqueo en hilo UI**).

---

## 2026-09-05 — Composición de anotaciones y StrokeCache

- **Hardware**: AMD Ryzen 7 5800H (8C/16T), Linux release build. Benchmark criterion (`benches/composite.rs`) configurado a resolución nativa de la tablet TCL (1440×2200).
- **Flujo medido**: Fusión de capa de anotaciones sobre bitmap de página completa. Comparación entre rasterización euclidiana directa vs hit en `StrokeCache`.
- **Métricas**:

| Escenario (1440×2200) | Rasterización directa optimizada | Con `StrokeCache` (hit) | Speedup |
|---|---:|---:|---:|
| 10 trazos, 0 resaltados | 531 µs | — | Baseline |
| 50 trazos, 10 resaltados | 1.51 ms | — | — |
| 100 trazos, 100 resaltados | 3.67 ms | — | — |
| **200 trazos**, 0 resaltados | **4.39–4.55 ms** | **2.39 ms** | **2.2×** |

- **Notas de diseño**:
  - La optimización de distancia euclidiana en `draw_segment` descarta el cálculo de `sqrt()` en ~85% de los píxeles (núcleo y fondo mediante radios al cuadrado), bajando el tiempo directo de 5.40 ms a 4.39 ms (-18.5%).
  - `StrokeCache` retiene la capa rasterizada y reduce el coste por frame a **2.39 ms** mediante blit con aritmética entera.

---

## 2026-09-05 — Carga asíncrona de biblioteca (ThumbWorker)

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200).
- **Build**: Release `pdf_android`, actor MPSC `ThumbWorker` con `MupdfEngine` en hilo dedicado.
- **Flujo medido**: Apertura de biblioteca y scroll continuo en rejilla 3×3 con carga progresiva de portadas en segundo plano.
- **Métricas**:
  - Frame time de blit durante scroll (`blit 1440x2200`): **5.56–6.36 ms (p95: ~6.1 ms)**.
  - Bloqueo de E/S síncrona en hilo UI: **0 ms** (`try_recv` no bloqueante sobre canal).
  - Eliminado el límite arbitrario de 50 libros de la política de retención previa.

---

## 2026-09-04 — Barrido inicial TCL y consumo PSS de la aplicación

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, 8 GB RAM, Android 15 / SDK 36, pantalla 1440×2200 @ 320 dpi). Pantalla encendida con `stayon true`.
- **Build**: Release aarch64 (`crates/pdf_bench`), corpus de 4 documentos en `/data/local/tmp/pdflector/corpus`.
- **Flujo medido**: `tools/adb-bench.sh --runs 5` (mediana de páginas 0, central y final en cada ejecución) y arranque de la aplicación para medición de PSS con `dumpsys meminfo`.
- **Métricas de render por página**:

| Ejecución | dense (93p) 1x | scanned (30p) 1x | paper (12p) 1x | large (500p) 1x | RSS pico (KB) |
|:---:|---:|---:|---:|---:|---:|
| Run 1 | 12.61 ms | 35.95 ms | 11.52 ms | 14.87 ms | 27 088 |
| Run 2 | 14.05 ms | 38.50 ms | 11.36 ms | 14.96 ms | 26 984 |
| Run 3 | 13.22 ms | 33.45 ms | 12.29 ms | 14.35 ms | 27 128 |
| Run 4 | 13.14 ms | 33.57 ms | 12.10 ms | 14.63 ms | 27 172 |
| Run 5 | 15.18 ms | 33.46 ms | 11.75 ms | 14.40 ms | 27 452 |

- **Memoria de la app real**:
  - Arranque en biblioteca: **PSS 110 352 KB (~107.8 MB)** (Native Heap 51.9 MB, Graphics 47.3 MB, Code 4.3 MB; RSS 235 189 KB).
  - Tras 130 pases de página por tap (195 presents): **PSS 208 882 KB (~204 MB)**. El incremento se debe a la retención de texturas Wet/Dry y búferes EGL acumulados.

---

## 2026-09-03 — Verificación en tablet de Display Lists retenidas

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Android 15).
- **Flujo medido**: Barrido `pdf_bench` a escala 2x tras incorporar display lists retenidas en `MupdfDocument`.
- **Métricas**:
  - *large_document.pdf* (2x): 70.35 ms → **68.73 ms**.
  - *dense_textbook.pdf* (2x): 69.54 ms → **66.89 ms**.
  - Conclusión: Variación dentro del margen de ruido térmico; sin regresión observable frente a la ejecución base.

---

## 2026-08-30 — Validación de experiencia física con stylus USI 2.0

- **Hardware**: TCL NXTPaper 11 Plus (9469X, pantalla 1440×2200), Stylus USI 2.0 con botón físico. Build release `pdf_android`.
- **Flujo medido**: Pruebas manuales e instrumentadas de dibujo, borrado por hardware y latencia percibida.
- **Resultados**:
  - **Goma por botón físico (`BTN_STYLUS2` 0x20)**: Transición inmediata a borrado al pulsar (`erase: stroke 10 -> 3 pieces`). Retorno a modo tinta al soltar sin parpadeo.
  - **Shader de tinta GPU**: Corregido `FS_INK_SRC` a premultiplied alpha (`vec4(rgb * a, a)`), eliminando desaturaciones de contraste en fondos claros y oscuros.
  - **Transición sin flashes**: `FS_OVERLAY_SRC` premultiplicado por `uAlpha` eliminó destellos blancos en cambios de estado.
  - **Cero-Pop**: La polilínea simplificada persistida converge con el trazo en vivo (`simplify_polyline` 0.35 pt).
  - **Latencia percibida vs App Nativa (TCL Notes)**:
    - PDFLector presenta a 60 Hz vía `eglSwapBuffers` (tiempo de frame ~1.5–4.0 ms, latencia total del pipeline ~16–30 ms).
    - La app propietaria de TCL recurre a rendering directo en front-buffer (<10 ms), perceptiblemente más inmediata debido a los límites de paso por SurfaceFlinger en apps estándar de Android.

---

## 2026-08-30 — Display Lists vs Re-parse completo en MuPDF

- **Hardware**: AMD Ryzen 7 5800H (8C/16T), Linux release build.
- **Flujo medido**: Renderizado mediante display list retenida en memoria (`fz_run_display_list`) frente a re-parse vectorial completo (`Page::to_pixmap`) por escala.
- **Métricas**:

| Documento | Escala | Re-parse base | Display List | Speedup |
|---|:---:|---:|---:|---:|
| large_document.pdf | 2× | 2.04 ms | 1.13 ms | **1.81×** |
| large_document.pdf | 4× | 5.76 ms | 4.51 ms | **1.28×** |
| dense_textbook.pdf | 2× | 2.33 ms | 1.42 ms | **1.64×** |
| dense_textbook.pdf | 4× | 6.96 ms | 5.15 ms | **1.35×** |
| scientific_paper.pdf | 2× | 3.10 ms | 1.84 ms | **1.68×** |

- **Conclusión**: En la escala habitual de pinch (2x) la display list acelera el render entre 1.6× y 1.8×. A 4x domina el tiempo de rasterizado de píxeles sobre la interpretación del árbol vectorial.

---

## 2026-08-28/29 — Migración a presentación EGL/GLES2 en Viewer

- **Hardware**: TCL NXTPaper 11 Plus (9469X, Mali-G57 MC2, Android 15). APK release aarch64.
- **Flujo medido**: Reemplazo del pipeline `ANativeWindow_lock` + `memcpy` de software por `eglSwapBuffers` con texturas GPU. Trazos generados mediante `input stylus swipe`.
- **Métricas**:
  - `gl_present` con trazo activo (n=1768): **p50: 4.55 ms · p90: 8.64 ms · p95: 10.00 ms** · p99: 13.56 ms · máx: 26.92 ms.
  - Frames aislados > 16.6 ms: 4 de 1768 (0.23%), todos en el rango 16.69–26.92 ms, nunca en ráfagas consecutivas.
  - Pase de página (doc 442 páginas, n=27): **p50: 5.65 ms · p95: 11.64 ms**.
  - PSS durante dibujo continuo: 116–126 MB.
  - PSS en biblioteca (tras liberar textura de página): **71.4 MB**.
  - PSS pico en documento complejo OCR (442 páginas): **152.6 MB** (GL mtrack 49.6 MB, EGL 24.8 MB, Heap nativo 62.8 MB).

---

## 2026-08-27 — Trazo de tinta con History batching y Bézier punto medio

- **Hardware**: TCL NXTPaper 11 Plus (9469X, Android 15). Release aarch64.
- **Flujo medido**: Captura de eventos con `input stylus swipe`. Muestreo de tiempos de dibujo por dirty rect en `draw.rs`.
- **Métricas**:
  - Blit durante el gesto (n=156): **p50: 4.80 ms · p95: 5.78 ms · máx: 7.46 ms**.
  - Frames superiores a 16.6 ms en toda la sesión: **0 de 156**.
  - Tamaño de dirty rects procesados: 13×42 a 53×94 px (solo el incremento diferencial del trazo).
  - PSS a lo largo de la sesión: 110 MB en arranque → 145.6 MB estable tras más de 20 trazos y cambios de página.

---

## 2026-08-24 — PageTextCache en hardware real

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Android 15).
- **Flujo medido**: Extracción de texto de página (`stext`) y consulta en `PageTextCache` (LRU 512 páginas).
- **Métricas**:

| Documento | Extracción fría pág 0 | Extracción fría intermedia | Miss de caché pág 10 | Hit de caché pág 10 | Prefetch 20 págs |
|---|---:|---:|---:|---:|---:|
| dense_textbook (93p) | **9.5 ms** | 2.3 / 0.25 ms | 2.4 ms | **0.000 ms** | 48 ms total |
| scientific_paper (12p) | **17.1 ms** | 0.5 / 0.4 ms | 0.38 ms | **0.000 ms** | — |

- **Conclusión**: La primera extracción de texto requiere entre 9 y 17 ms en la CPU de la tablet. Con `PageTextCache`, los accesos subsiguientes para subrayado toman 0.000 ms en el hilo UI.

---

## 2026-08-24 — Benchmark de resaltado y composición en escritorio

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Linux release build.
- **Flujo medido**: `cargo bench -p pdf_bench` para algoritmos de selección y mezcla.
- **Métricas**:
  - Intersección de resaltado (`highlight_under_gesture`):
    - 10 puntos / 20 líneas: ~1.3 µs.
    - 50 puntos / 100 líneas: ~9.6 µs.
    - 100 puntos / 200 líneas: ~36 µs.
    - Dos columnas (200 líneas): ~20 µs.
  - Fusión de anotaciones (`composite_annotations` a 1440×2200):
    - 10 trazos: 0.64 ms.
    - 50 trazos + 10 resaltados: 2.14 ms.
    - 200 trazos: **6.8 ms**.

---

## 2026-08-24 — Corrección de canal alfa en trazo en tiempo real

- **Hardware**: TCL NXTPaper 11 Plus (9469X).
- **Flujo medido**: Captura visual (screencap) durante el trazo de stylus con la herramienta Boli activa.
- **Diagnóstico previo**: El trazo en curso era invisible durante el arrastre porque `composite_annotations` generaba un mapa con alfa 0, omitiendo el pintado en `copy_region_blend`.
- **Métricas con `composite_annotations_alpha`**:
  - Píxeles visibles a mitad de gesto: de **0 px** a **928 px** (coincidencia geométrica exacta con el dedo/lápiz).
  - Píxeles al soltar el trazo: **2205 px**.
  - Coste de `blit_composed`: 4–8 ms por frame durante la interacción.

---

## 2026-08-22 — Optimización de primitivas de blit y prefetch de páginas

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux, release build.
- **Flujo medido**: Microbenchmarks de primitivas de copia en `pdf_bench` (espejo de `draw.rs` a 2000×1200) y prefetch en ráfaga de navegación.
- **Métricas de Blit (resolución 2000×1200, mediana)**:

| Operación | Baseline previo | Con optimización | Reducción |
|---|---:|---:|:---:|
| `blit/page_1to1_bpp4_dark` | 1.376 ms | **154 µs** | **−89 %** |
| `blit/page_zoom135_bpp4_light` | 1.292 ms | **727 µs** | **−44 %** |
| `blit/page_zoom135_bpp4_dark` | 2.391 ms | **556 µs** | **−77 %** |
| `blit/page_1to1_bpp4_light` | 332 µs | 368 µs | Sin cambio (ruido) |
| `blit/page_1to1_bpp2 (RGB565)` | 1.153 ms | 1.167 ms | Sin cambio |

- **Optimizaciones clave**:
  - Inversión de modo oscuro en bpp 4 convertida de iteración byte a byte a operación XOR en u32 (`val ^ 0x00FF_FFFF`), permitiendo vectorización automática del compilador.
  - Acceso directo en escala por vecino más cercano sin comprobaciones de límites redundantes por píxel en slice.
- **Prefetch preemptivo (`prefetch.rs`)**:
  - En una ráfaga de 10 viewports no solapados de 11 páginas cada uno, las páginas efectivamente procesadas cayeron de 110 a **22 páginas (−80%)** al descartar solicitudes obsoletas en vuelo. Tiempo hasta residencia del viewport final: 35.8 ms.

---

## 2026-08-13 — Zoom en tablet TCL: Escala software vs Re-renderizado

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla encendida, 33 °C). Release aarch64.
- **Flujo medido**: Comparación entre escalado por software (`scale_bitmap`, nearest-neighbor en CPU) y re-renderizado vectorial nativo con MuPDF.
- **Métricas (mediana de 3 ejecuciones)**:

| Escenario | Escalado software (`scale_bitmap`) | Re-renderizado vectorial (MuPDF) | Ratio de penalización SW |
|---|---:|---:|:---:|
| large_document pág 0 → 2× | 69.4–70.2 ms | **14.9–16.6 ms** | ~4.5× más lento |
| large_document pág 0 → 4× | 275.8–281.5 ms | **53.2–56.1 ms** | ~5.2× más lento |
| dense_textbook pág 0 → 2× | 69.9 ms | **16.1–16.9 ms** | ~4.3× más lento |
| dense_textbook pág 0 → 4× | 321.4–325.1 ms | **57.3–59.4 ms** | ~5.6× más lento |

- **Decisión arquitectónica**: El reescalado software en CPU sin extensiones SIMD/NEON es prohibitivo para interactividad en la tablet. El zoom inmediato debe realizarse mediante texturas en GPU, seguido de un re-renderizado asíncrono nítido en segundo plano.

---

## 2026-08-13 — Zoom en escritorio: Escala software vs Re-renderizado

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux.
- **Flujo medido**: `crates/pdf_bench/benches/zoom.rs` sobre *large_document.pdf* (página 0).
- **Métricas**:
  - `scale_bitmap` a 2×: 55.9 ms.
  - `scale_bitmap` a 4×: 214.5–219.9 ms.
  - Re-render nativo MuPDF a nivel 1 (2×): **3.3–3.4 ms**.
  - Re-render nativo MuPDF a nivel 2 (4×): **11.9–12.5 ms**.
  - Conclusión idéntica a la tablet: el re-render vectorial es entre 16× y 18× más rápido en CPU que el remuestreo naïve por píxeles.

---

## 2026-08-12 — Sweep inicial en tablet TCL NXTPaper 11 Plus

- **Hardware**: TCL NXTPaper 11 Plus (modelo 9469X, MediaTek MT8781 8× Cortex-A55, 8 GB RAM, Android 15, pantalla 1440×2200 @ 320 dpi). Pantalla encendida con `stayon true`.
- **Build**: Binario `pdf_bench` release compilado para `aarch64-linux-android` (NDK r28 + sysroot). Motor MuPDF.
- **Flujo medido**: Barrido de apertura y renderizado completo a 1x (72 dpi) y 2x (144 dpi) sobre los 4 documentos del corpus.
- **Métricas**:

| Documento (páginas) | open (ms) | render 1x (ms) | render 2x (ms) |
|---|---:|---:|---:|
| dense_textbook (93p) | 0.40 | 14.51 | 44.18 |
| scanned_pages (30p) | 0.15 | 31.34 | 119.01 |
| scientific_paper (12p) | 0.16 | 11.64 | 38.44 |
| large_document (500p) | 0.25 | 15.40 | 44.73 |

- **Memoria**: RSS pico de **26 688 KB (~26.7 MB)**.
- **Ausencia de shootout en tablet**: En la tablet TCL **únicamente se ejecutó MuPDF**. No existe un benchmark comparativo de motores en Android (`android-tablet-shootout.md` nunca existió ni fue medido).

---

## 2026-08-10 — Baseline de referencia: Evince (poppler) en escritorio

Esta medición establece el comportamiento de referencia de un visor estándar de escritorio basado en Poppler + Cairo, sirviendo como evidencia empírica para la toma de decisiones en ADR-003.

- **Hardware**: AMD Ryzen 7 5800H (8C/16T, hasta 4.47 GHz), 13 GiB RAM, Linux 7.1.4-arch1-1 (Wayland/Hyprland).
- **Software**: Evince 48.4, Poppler 26.07.0.
- **Corpus**: `corpus/large_document.pdf` (500 páginas A4, 543 kB, texto vectorial puro).
- **Método de captura**: `tools/bench-evince/bench_evince.sh` utilizando `pdftoppm` (comparte exactamente el pipeline monohilo de renderizado de Evince: poppler + cairo, página completa a la escala solicitada).

### 1. Renderizado monohilo de página completa (Poppler)

| Escala | Píxeles por página (A4) | Tiempo total 500 págs (3 repeticiones) | Tiempo medio por página | RSS máximo del proceso |
|---|:---:|:---:|---:|---:|
| 72 dpi (1×) | 595 × 842 | 36.72 / 36.84 / 37.84 s | **73.6 ms** | 22.5 MB |
| 144 dpi (2×) | 1190 × 1684 | 164.36 / 162.06 s | **326.0 ms** | 28.0 MB |

### 2. Apertura inicial y render de primera página (incluye parseo del documento)

| Escala | Tiempo (3 repeticiones) | Mediana |
|---|:---:|---:|
| 144 dpi (2×) | 0.42 / 0.36 / 0.35 s | **~0.36 s** |
| 216 dpi (3×) | 0.60 / 0.60 / 0.60 s | **0.60 s** |

### 3. Consumo RSS del visor gráfico Evince 48.4 (500 páginas)

| Estado | Consumo RSS |
|---|---:|
| Ventana abierta en página 1 tras 8 s (arranque en frío) | **197 920 kB** (~193.3 MB) |
| Reapertura con caché de disco caliente (8 s) | **197 952 kB** (~193.3 MB) |

### Deducciones clave para el proyecto
1. **La escala cuadruplica el coste de rasterizado**: Pasar de 1× (72 dpi) a 2× (144 dpi) eleva el renderizado de 73.6 ms a 326.0 ms por página. A resolución nativa de lectura, el renderizado en vivo durante un frame de scroll provocaría caídas intolerables de fluidez (~3 fps).
2. **Evince mantiene memoria acotada en render**: Su proceso libera buffers tras pintar (RSS máx 28 MB), pero el visor gráfico completo supera los 197 MB de RSS debido al entorno GTK/GL y la estructura interna del documento.

---

## 2026-08-05 — Comparativa de motores en escritorio (PDFium vs MuPDF) y discrepancia metodológica

- **Hardware**: AMD Ryzen 7 5800H (8C/16T, 3.2–4.4 GHz), 16 GB RAM, Arch Linux (kernel 6.16). Compilación release en Rust 1.97.1.
- **Corpus**: 4 documentos estándar (`paper_12p`, `scanned_30p`, `dense_93p`, `large_500p`).

### La discrepancia metodológica no resuelta
Existen dos conjuntos de mediciones independientes realizados en la misma máquina host durante la Fase 0.5 que arrojaron resultados diametralmente opuestos:

1. **Sweep simple de `pdf_bench` (mediana de 3 corridas en páginas 0, central y final)**:
   - Reportó a **MuPDF como ganador indiscutible**: entre 2.7× y 4× más rápido en renderizado y un 21% menos de consumo de memoria pico.
   - Estos datos fueron la base tomada en cuenta para la redacción y aprobación de **ADR-001**.
2. **Benchmark exhaustivo Criterion (`crates/pdf_bench/benches/engine_shootout.rs`)**:
   - Con un tamaño de muestra N=100 tras 3 segundos de calentamiento por caso, **PDFium resultó ganador en 14 de las 16 pruebas de renderizado (87.5%)**, mostrando ser típicamente 2× a 4× más veloz.
   - PDFium también superó a MuPDF en 3 de las 4 pruebas de apertura de archivo.

Esta divergencia nunca fue aclarada formalmente en su momento (atribuible a diferencias entre corridas monohilo aisladas sin warm-up frente a la saturación de bucle cerrado con optimización de caché de Criterion, y a variaciones en flags de compilación de las bibliotecas estáticas C).

**Decisión del dueño del proyecto**: A pesar de la ventaja demostrada por PDFium en el benchmark Criterion en escritorio, **se mantiene la decisión de ADR-001 en favor de MuPDF**. Los motivos determinantes son:
- Distribución autocontenida: MuPDF se compila como biblioteca estática unificada dentro del binario Rust (6.48 MB totales), eliminando la dependencia de binarios dinámicos pesados y fragmentados como `libpdfium.so` (12.53 MB en disco).
- Ergonomía de integración de tipos C bajo `mupdf-sys` y previsibilidad de licencias y compilación cruzada hacia Android NDK.
- En la tablet TCL solo se desplegó y midió MuPDF; no se llegó a realizar una comparativa en hardware móvil.

A continuación se transcriben los datos brutos de ambas mediciones:

### A. Resultados del Sweep Simple (`pdf_bench`) — Base de ADR-001

| Documento (páginas) | Motor | Apertura (ms) | Render 1× (ms) | Render 2× (ms) | RSS pico (KB) |
|---|---|---:|---:|---:|---:|
| dense_textbook (93p) | PDFium | 0.17 | 9.69 | 35.34 | 32 520 |
| dense_textbook (93p) | MuPDF | **0.11** | **3.53** | **8.51** | **25 572** |
| scanned_pages (30p) | PDFium | 0.09 | 20.01 | 66.20 | 32 520 |
| scanned_pages (30p) | MuPDF | **0.07** | **8.93** | **35.38** | **25 572** |
| scientific_paper (12p) | PDFium | 0.08 | **1.72** | 26.44 | 32 520 |
| scientific_paper (12p) | MuPDF | **0.07** | 2.18 | **6.95** | **25 572** |
| large_document (500p) | PDFium | 0.21 | 6.86 | 35.10 | 32 520 |
| large_document (500p) | MuPDF | **0.09** | **3.98** | **10.19** | **25 572** |

### B. Resultados del Shootout Criterion (N=100)

#### Apertura de documento (mediana en microsegundos)
| Documento | Páginas | PDFium (µs) | MuPDF (µs) | Ratio (PDFium / MuPDF) |
|---|---:|---:|---:|:---:|
| scientific_paper | 12 | 54.98 | 55.67 | 0.99× (Empate) |
| scanned_pages | 30 | **56.70** | 70.26 | **1.24× (PDFium)** |
| dense_textbook | 93 | **82.37** | 245.80 | **2.98× (PDFium)** |
| large_document | 500 | **186.03** | 384.32 | **2.07× (PDFium)** |

#### Renderizado de página completa (mediana en milisegundos)
| Documento | Página | Escala | PDFium (ms) | MuPDF (ms) | Ratio y Ganador |
|---|---|:---:|---:|---:|:---:|
| paper_12p | p1 | 1× | **0.502** | 1.752 | **3.49× (PDFium)** |
| paper_12p | p_mitad | 1× | **0.556** | 5.806 | **10.45× (PDFium)** |
| paper_12p | p1 | 2× | **10.477** | 23.087 | **2.20× (PDFium)** |
| paper_12p | p_mitad | 2× | **10.448** | 27.967 | **2.68× (PDFium)** |
| scanned_30p | p1 | 1× | **3.304** | 11.918 | **3.61× (PDFium)** |
| scanned_30p | p_mitad | 1× | **6.299** | 7.299 | **1.16× (PDFium)** |
| scanned_30p | p1 | 2× | **14.457** | 58.994 | **4.08× (PDFium)** |
| scanned_30p | p_mitad | 2× | **18.051** | 28.591 | **1.58× (PDFium)** |
| dense_93p | p1 | 1× | 2.614 | **1.725** | **0.66× (MuPDF)** |
| dense_93p | p_mitad | 1× | **3.459** | 4.521 | **1.31× (PDFium)** |
| dense_93p | p1 | 2× | **20.533** | 54.127 | **2.64× (PDFium)** |
| dense_93p | p_mitad | 2× | **13.450** | 47.979 | **3.57× (PDFium)** |
| large_500p | p1 | 1× | **2.699** | 9.950 | **3.69× (PDFium)** |
| large_500p | p_mitad | 1× | **2.917** | 5.373 | **1.84× (PDFium)** |
| large_500p | p1 | 2× | 33.263 | **30.487** | **0.92× (MuPDF)** |
| large_500p | p_mitad | 2× | **13.749** | 18.160 | **1.32× (PDFium)** |

#### Tamaño de artefacto resultante
| Motor | Binario ejecutable | Bibliotecas dinámicas externas | Espacio total en disco |
|---|---:|---:|---:|
| PDFium | 4.86 MB | `libpdfium.so` 7.67 MB | **12.53 MB** |
| MuPDF | **6.48 MB** | 0 MB (enlace estático) | **6.48 MB** |

---

## 2026-08-05 — Eficiencia de Caché LRU vs Retención Ingenua

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux release build.
- **Flujo medido**: `crates/pdf_bench/benches/cache_scroll.rs` sobre 50 páginas de `large_document.pdf` a escala 1× (72 dpi) con MuPDF. Comparación entre retención arbitraria de bitmaps y ventana acotada a 8 MB.
- **Métricas**:

| Escenario | Tiempo total (ms) | Pico de memoria VMHWM (KB) |
|---|---:|---:|
| `naive_hold_50p_1x` (retener 50 páginas) | 108.02 ms | 107 412 KB (~105 MB) |
| `cache_8mb_firstpass_50p_1x` (primera pasada) | 74.78 ms | **21 104 KB (~20.6 MB)** |
| `cache_8mb_pass2_50p_1x` (hit sobre residentes) | **0.35 ms** | 21 184 KB (~20.7 MB) |

- **Conclusión**: La caché LRU limitada a 8 MB reduce la memoria RAM pico en un factor de 5× (de 105 MB a 20.6 MB). El camino de acierto en caché insume 0.35 ms frente a los 74 ms del recorrido con renderizado.
