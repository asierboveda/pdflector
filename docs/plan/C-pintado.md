# Fase C — Pintado y trazo a mano alzada sin latencia

Pintado fluido con lápiz óptico o stylus, pipeline de dibujo acelerado en GPU y simplificación geométrica para evitar acumulación de vértices.

## Auditoría

- **Arquitectura GPU real (ADR-007)**:
  - El mecanismo histórico en software sobre CPU (`tool_overlay`, `raster_tool_layer`, `copy_region_blend`) fue completamente sustituido por el pipeline Dual FBO Wet/Dry sobre GLES2/EGL.
  - **Capa Dry (base estática)**: FBO con textura persistente gobernado por `DryKey` (`crates/pdf_android/src/gpu/dry_key.rs`, `gpu/pipeline.rs:914`). Contiene la página decodificada y las anotaciones confirmadas. Solo se reconstruye si cambia de página, zoom, número de anotaciones o modo oscuro. Durante el trazo activo, la capa Dry permanece inmutable (cero parpadeo y cero re-rasterizado).
  - **Capa Wet (trazo en vuelo)**: Gestionada por `render_wet` (`crates/pdf_android/src/gpu/pipeline.rs:686`). Renderiza directamente sobre un FBO transparente la polilínea del trazo activo (`ink_pts`), la predicción de movimiento y el cursor, combinándose con mezcla alpha en el framebuffer de presentación (`fb0`).
- **Simplificación geométrica en captura**:
  - En `crates/pdf_android/src/reader/tools.rs:328`, al levantar el lápiz (`Up`), la polilínea muestreada se compacta con el algoritmo Douglas-Peucker (`pdf_core::simplify_polyline`) con tolerancia ε = 0,20 pt (no 0.8 pt), eliminando puntos colineales redundantes sin alterar la fidelidad caligráfica.
- **Composición y pintado**:
  - El producto Android no utiliza composición por CPU en producción, apoyándose enteramente en la composición por hardware en GPU (ADR-007). El compositor CPU experimental y sus cachés asociadas fueron eliminados de `pdf_core` por carecer de consumidores de producción.

## Objetivo

Dibujar a mano alzada con stylus manteniendo un tiempo de presentación interactivo < 8 ms p95 en la pantalla de la tablet, incluso con páginas que contengan 200 trazos acumulados.

## Tareas

- [x] C1. **Simplificación de trazos**: Integración de `simplify_polyline` con ε = 0,20 pt en el cierre del gesto (`reader/tools.rs:328`).
- [x] C2. **Pipeline Dual FBO Wet/Dry**: Implementado en GLES2 con gestión de claves `DryKey` e invalidación selectiva (`gpu/pipeline.rs`, `gpu/dry_key.rs`).
- [x] C3. **Microbenchmarks históricos en host x86_64**: Medición previa en host sobre un camino CPU hoy eliminado. El único criterio vivo de la fase C es medir 200 trazos con pintado vivo < 8 ms p95 en la tablet TCL, y sigue `SIN MEDIR`.
- [ ] C4. **Medición y cierre en hardware real TCL**: Ejecutar prueba de 200 trazos activos concurrentes en la tablet TCL 9469X midiendo percentiles reales con `FrameTimer`.

## Criterio de cierre

- [ ] 200 trazos en una página en la tablet TCL 9469X: scroll continuo p95 < 16.6 ms y pintado en vivo con stylus p95 < 8 ms. Estado: `SIN MEDIR` en tablet.

## Cómo modificar

- Para ajustar la sensibilidad del trazo, modificar la tolerancia $\varepsilon$ en `crates/pdf_android/src/reader/tools.rs`.
- El grosor base se configura mediante `STROKE_WIDTH_PT` (ver deuda en `docs/plan/DEUDA.md` para presets dinámicos).

## Referencias

- `crates/pdf_android/src/gpu/dry_key.rs`
- `crates/pdf_android/src/gpu/pipeline.rs`
- `crates/pdf_android/src/reader/tools.rs`
- `docs/adr/ADR-007-gpu-pipeline-dry-wet.md`
- `docs/benchmark-results.md`
