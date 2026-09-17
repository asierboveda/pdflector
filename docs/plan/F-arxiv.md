# Fase F — Descubrimiento e integración nativa con arXiv

Catálogo integrado de preprints de arXiv y recepción directa de papers mediante intents del sistema operativo Android.

## Auditoría

- `crates/pdf_core/src/arxiv.rs`:
  - `parse_arxiv_id` (línea 50): normaliza identificadores arXiv tanto en formato nuevo (`YYMM.NNNNN[vV]`) como clásico (`arch-ive/YYMMNNN`), filtrando URLs completas (`arxiv.org/abs/...`, `arxiv.org/pdf/...`).
  - `ArxivQuery` (línea 84): serialización de consultas con parámetros de búsqueda, lista de IDs, paginación (`start`, `max_results`) y ordenación.
  - `MIN_QUERY_INTERVAL` (línea 158): control estricto de tasa de peticiones a la API oficial de arXiv (mínimo 3.0 s entre consultas consecutivas) para evitar bloqueos por IP.
  - `ArxivClient` (línea 186): cliente HTTP síncrono con base configurable y timeout controlado.
- `crates/pdf_android/src/discover.rs`:
  - `DiscoverWorker` (línea 238): actor ejecutado en hilo secundario con conexión HTTP dedicada y cola MPSC no bloqueante para la UI.
  - `FeedCache` (línea 118): caché en disco LRU de respuestas XML/Atom (tope de 4 MiB, TTL de 10 min).
  - Descarga atómica (línea 638): los ficheros PDF se reciben en un fichero temporal `.part` y se renombran atómicamente tras validar integridad.
- Interfaz en `crates/pdf_android/`:
  - `draw/discover.rs`: renderizado nativo en GPU del feed, tarjetas de artículo, resúmenes y botones de acción.
  - `reader/discover_categories.rs`: taxonomía completa de arXiv organizada en ~45 categorías (Computer Science, Matemáticas, Física, etc.).
- Handoff en `crates/pdf_android/src/jni.rs`:
  - `parse_arxiv_target` (línea 574): extractor de identificador arXiv desde texto plano compartido o URLs provenientes de navegadores externos y aplicaciones como PaperTok.

## Objetivo

Permitir al usuario explorar novedades académicas por temática y descargar y abrir papers directamente en el visor con un solo toque o mediante compartir enlace desde Android, sin congelar la interfaz ni violar el presupuesto de memoria.

## Tareas

- [x] F1. **Módulo core arXiv**: `pdf_core::arxiv` con cliente HTTP, parser de identificadores y rate limiter de 3 s implementados y testeados.
- [x] F2. **Worker actor y caché en disco**: `DiscoverWorker` en hilo desacoplado del hilo UI, con caché LRU de feeds (4 MiB, TTL 10 min) y descarga atómica `.part`.
- [x] F3. **Interfaz de usuario Discover**: Rejilla/feed interactivo en `draw/discover.rs` con catálogo de ~45 categorías temáticas y estados visuales de carga y error.
- [x] F4. **Handoff nativo por Intent**: Integración en `jni.rs` para capturar eventos `android.intent.action.SEND` y URLs de arXiv.
- [ ] F5. **Medición de fluidez de scroll en tablet TCL**: Medir tiempo de frame en scroll continuo del feed de Discover con `tools/adb-bench.sh`.
- [ ] F6. **Validación de handoff por adb y robustez offline**: Probar inyección de intents mediante `adb shell am start` y verificar que ante corte de red la aplicación muestra un aviso limpio sin jank ni cierres inesperados.

## Criterio de cierre

- [ ] Scroll continuo de la pestaña Discover en la TCL 9469X a 60 fps sostenidos (tiempo de frame p95 < 16.6 ms). Estado: `SIN MEDIR`.
- [ ] Apertura de paper compartido mediante intent en < 2.0 s tras completar descarga. Estado: `SIN MEDIR`.
- [ ] Consumo de memoria PSS en la vista Discover estabilizado por debajo de 180 MB. Estado: `SIN MEDIR`.
- [ ] Cero bloqueos en hilo UI durante descarga de PDFs pesados o fallos de red.

## Referencias

- `crates/pdf_core/src/arxiv.rs`
- `crates/pdf_android/src/discover.rs`
- `crates/pdf_android/src/draw/discover.rs`
- `crates/pdf_android/src/reader/discover_categories.rs`
- `crates/pdf_android/src/jni.rs`
