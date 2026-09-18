# NEXT PLAN — Roadmap Consolidado de PDFLector (Fases A–F)

> **Estado auditado a 2026-09-15 (HEAD `4906161`).**  
> **Fuente de verdad:** Este documento y los ficheros de fase (`A-latencia.md` a `F-arxiv.md`) junto a `DEUDA.md` representan el estado real y las prioridades de desarrollo del proyecto. La cola operativa vive en GitHub Issues.  
> **Principio de ingeniería:** Toda afirmación de rendimiento exige fecha, hardware, flujo medido y métrica. Sin medición en hardware real (TCL NXTPaper 11 Plus, 1440×2200, 8× Cortex-A55) no se declara cerrado ningún criterio.

---

## 1. Tabla de Fases del Roadmap

| Fase | Fichero | Qué se entrega | Estado auditado (2026-09-15) | Criterio de cierre medible |
|---|---|---|---|---|
| **A** | `A-latencia.md` | Instrumentación, harness `adb` y baseline reproducible | A1–A3 HECHOS · A4–A5 PENDIENTES | `FrameTimer` en logcat, sweep automatizado, gate en CI y baseline de interacción en tablet |
| **B** | `B-subrayado.md` | Subrayador sin latencia en orden de lectura sobre texto detectado | CERRADA (2026-09-05) | Gesto → `Highlight` < 16 ms p95, sin I/O en frame de gesto; p95 3.5 ms medido en TCL |
| **C** | `C-pintado.md` | Lápiz a mano alzada fluido (pipeline GPU Dry/Wet FBO) | Pipeline funcional · Cierre en TCL PENDIENTE | 200 trazos en página con pintado vivo < 8 ms p95 en tablet TCL (`SIN MEDIR` en tablet) |
| **D** | `D-ia-contexto.md` | IA con contexto global del PDF (RAG BM25 local + visión) | NO INICIADA (clientes base listos) | 4/5 respuestas citan páginas reales, latencia p50 < 15 s vía API externa |
| **E** | `E-library.md` | Biblioteca fluida en rejilla (portadas asíncronas) | E1/E2/E4 HECHOS · E3 PARCIAL | Scroll p95 < 16 ms (10 ms con 11 libros; 256 libros `SIN MEDIR`); cold-start < 200 ms (medido 349 ms) |
| **F** | `F-arxiv.md` | Catálogo Discover arXiv y apertura directa por Intent | Core + UI HECHOS · Cierre PENDIENTE | Scroll de feed < 16 ms p95 en TCL (`SIN MEDIR`), handoff verificado por adb |

---

## 2. Detalle y Estado Real por Fase

### Fase A: Instrumentación y Harness de Rendimiento
- **Implementado**:
  - **A1**: `FrameTimer` integrado en `crates/pdf_android/src/gpu/surface.rs:108` e impresión periódica en `gpu/pipeline.rs:1022-1038` (`frame p95=X.Xms (N frames)` cada 120 presents con overhead nanosegundo).
  - **A2**: Suites de benchmarking en host: `crates/pdf_bench/benches/highlight.rs` (búsqueda y ordenación espacial).
  - **A3**: Arnés de automatización `tools/adb-bench.sh` (ejecución de sweeps de páginas, captura de PSS vía dumpsys, screencap y recolección de métricas de logcat).
- **Pendiente**:
  - **A4**: Gate de rendimiento en CI (no existe en `.github/workflows/ci.yml`). Debe ejecutar los benches y fallar si hay regresión.
  - **A5**: Medición de interacción masiva en tablet (200 trazos simultáneos y 100 gestos de subrayado interactivo): bloqueado para automatización sintética por falta de lápiz físico en el banco de pruebas (`docs/benchmark-results.md:697`). PSS bajo interacción medido: 234.7 → 287.5 MB en 15 turnos rápidos (2026-09-07).

### Fase B: Subrayado sin Latencia (CERRADA)
- **Implementación**:
  - Cero extracción de texto en el frame del gesto: `PageTextCache` pre-extrae páginas vecinas (±2); en `Down` se ordenan los spans una sola vez (`sort_spans_by_y`); durante `Move` únicamente se actualiza el punto actual (`reader/tools.rs:201-205` `g.set_cur(pt)` sin I/O en `:84-92`).
  - Evolución a orden de lectura: El algoritmo evolucionó de un recorrido lineal a búsqueda indexada por banda Y (`sort_spans_by_y` + `highlight_under_gesture_sorted`), garantizando selección precisa en documentos a 2 columnas.
  - Al soltar (`Up`), la anotación `Highlight` se persiste en SQLite/sidecar sin bloquear la renderización.
- **Evidencia en hardware (2026-09-05, TCL 9469X con stylus USI físico)**:
  - Tiempo de frame sostenido durante el trazo continuo: 1.08–5.33 ms (p50 ~2.8 ms, p95 ~3.5 ms), manteniendo entre 60 y 120 fps estables.

### Fase C: Pintado con Lápiz a Mano Alzada
- **Implementación real**:
  - La arquitectura de pintado superó el esquema de superposición de mapas de bits en CPU (`tool_overlay`/`raster_tool_layer`/`copy_region_blend`) y adoptó el pipeline Dual FBO Wet/Dry sobre GLES2 (ADR-007).
  - **Capa base (Dry)**: Almacenada en un FBO estático gestionado por `DryKey` (`crates/pdf_android/src/gpu/dry_key.rs`, `gpu/pipeline.rs:914`). Solo se re-renderiza cuando cambia la página, el zoom, las anotaciones o el modo oscuro.
  - **Trazo en vuelo (Wet)**: Gestionado por `render_wet` (`pipeline.rs:686`), renderiza la geometría activa por frame directamente en GPU con blending alpha nativo.
  - **Simplificación geométrica**: Al soltar el trazo se aplica Douglas-Peucker iterativo con tolerancia fina de ε = 0,20 pt (`reader/tools.rs:328`), preservando la caligrafía natural sin retrasos.
  - **Composición y pintado**: El producto Android no utiliza composición por CPU en producción, apoyándose enteramente en la composición por hardware en GPU (ADR-007). El compositor CPU experimental y sus cachés asociadas fueron eliminados de `pdf_core` en la limpieza de 2026-09-18 por carecer de consumidores.
- **Mediciones**:
  - En host x86_64: mediciones históricas en host sobre el camino CPU hoy eliminado.
  - En hardware real TCL: El cierre formal (200 trazos concurrentes en página con pintado vivo < 8 ms p95) permanece `SIN MEDIR`.

### Fase D: IA con Contexto Global del PDF (NO INICIADA)
- **Componentes existentes**:
  - Segmentación de documentos: `chunk_pages` en `crates/pdf_core/src/ai.rs:155` con word-packing y prefijos `[págs N-M]`.
  - Clientes API: `OllamaClient`, `GroqClient` y `GeminiClient` en `pdf_core::ai`.
  - Panel de IA en Android: Interfaz UI en `crates/pdf_android/src/reader/toast_ia.rs` y `draw/ai_panel.rs`.
  - Visión con contexto: `explain_image` en `reader/toast_ia.rs:99-101` SÍ anexa el texto extraído de la página seleccionada al prompt para enriquecer la imagen PNG enviada a Gemini.
- **Pendiente para ejecución**:
  - D1: Índice local BM25 puro en Rust (sin dependencias pesadas) para recuperar las páginas más relevantes ante una consulta.
  - D2: Construcción de prompt con contexto global estructurado y directiva estricta de citar números de página reales.
  - D3: Estudio de tamaño de ventana de contexto en la tablet con corpus de prueba.
  - D4: Script de validación automatizada `tools/ai-bench.sh`.

### Fase E: Biblioteca y Gestión de Catálogo Local
- **Implementado**:
  - **E1**: `ThumbWorker` en segundo plano (`crates/pdf_android/src/thumbs.rs:197-267`) con actor MPSC e instancia dedicada de `MupdfEngine`. Medido en TCL: blit fluido de 5.56–6.36 ms (p95 6.1 ms) sin congelar la UI.
  - **E2**: Hoja de menú (sheet) animada sin re-renderizar la página PDF de fondo (verificado 2026-09-07 en TCL: apertura y cierre en 13 presents con 0 evicciones de caché).
  - **E4**: Eliminación total de borrado automático de libros; no existen cuotas artificiales de almacenamiento.
  - **Evolución**: Eliminado el carrusel redundante "Continue Reading" (commit `3b726a1`) en favor de una rejilla uniforme y directa.
- **Pendiente**:
  - **E3**: Medición de scroll con 256 libros: En TCL se midió scroll p95 de 10.0 ms con 11 libros (2026-09-07), pero el ensayo a escala con 256 libros está `SIN MEDIR`.
  - Cierre de arranque en frío: Primer frame del visor tras cold-start medido en 349 ms (abrir PDF 213 ms + InitWindow 73 ms), pendiente de optimización frente al objetivo de < 200 ms.

### Fase F: Integración con arXiv y Discover (PRODUCTO EN VIGOR)
- **Implementado**:
  - Módulo core `crates/pdf_core/src/arxiv.rs`: Extracción y normalización de identificadores (`parse_arxiv_id`, :50), serialización de consultas (`ArxivQuery`, :84), cliente de red (`ArxivClient`, :186) y limitador de tasa de 3.0 s (`MIN_QUERY_INTERVAL`, :158).
  - Gestor de descargas `crates/pdf_android/src/discover.rs`: `DiscoverWorker` (:238), caché LRU en disco de feeds (`FeedCache`, 4 MiB, TTL 10 min, :118) y descarga transaccional con renombrado atómico desde `.part` (:638).
  - Interfaz gráfica: Rejilla en GPU `crates/pdf_android/src/draw/discover.rs` y catálogo con ~45 categorías temáticas (`reader/discover_categories.rs`).
  - Integración nativa Android: Recepción de intents en `crates/pdf_android/src/jni.rs:574` (`parse_arxiv_target`) para abrir enlaces compartidos.
- **Pendiente**:
  - Medición de fluidez en tablet (scroll p95 < 16.6 ms). Estado: `SIN MEDIR`.
  - Validación formal de handoff por `adb shell am start`.

---

## 3. Síntesis de Deuda Transversal

La deuda técnica detallada y el backlog de propuestas se gestionan en `docs/plan/DEUDA.md`. Los elementos de mayor criticidad para la estabilidad del producto son:

1. **Display lists sin límite en MuPDF**: Cada página visitada retiene +6–7 MB en memoria nativa, elevando el PSS de 190 a 330 MB. Requiere política LRU.
2. **PSS pico en ráfagas de lectura**: Medido incremento de 234 a 287 MB tras 15 cambios rápidos de página en la TCL 9469X.
3. **Validación en hardware real**: Cierre formal de latencia con 200 trazos (Fase C) y scroll a 256 libros (Fase E).
4. **Decisión arquitectónica de persistencia**: Sidecar JSON lateral frente a guardado incremental directo en el fichero PDF.

---

## 4. Reglas de Mantenimiento del Plan

- Toda modificación en el roadmap debe actualizar este fichero y el documento de fase correspondiente.
- Las tareas individuales acotadas se crean y rastrean en GitHub Issues.
- Cambios en el diseño global o decisiones tecnológicas deben quedar documentados formalmente en `docs/adr/`.
