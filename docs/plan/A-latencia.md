# Fase A — Instrumentación y harness TCL

Instrumentación de métricas de rendimiento, tiempos de frame (p95), perfiles de memoria PSS y automatización de mediciones vía `adb` sobre hardware real.

## Auditoría

- `FrameTimer` integrado en `crates/pdf_android/src/gpu/surface.rs:108` con anillo circular prealocado y cálculo de percentiles sin asignación dinámica de memoria.
- `pdf_bench` mide tiempos de rasterización (`render1x`) y resolución espacial de resaltado (`benches/highlight.rs`).
- `tools/adb-bench.sh` automatiza la captura de métricas directamente desde el dispositivo conectado.

## Objetivo

Disponer de un arnés reproducible (`cargo run -p pdf_bench` + `tools/adb-bench.sh`) que reporte percentiles p95 y consumo PSS en hardware real sin manipulación manual.

## Tareas

- [x] A1. **`FrameTimer` en `pdf_android`**: Integrado en `crates/pdf_android/src/gpu/surface.rs:108` y emitido en `gpu/pipeline.rs:1022-1038` (`frame p95=` a logcat cada 120 presents con overhead nanosegundo). Verificado en TCL 9469X el 2026-09-04.
- [x] A2. **Benches de micro-rendimiento en host**: `crates/pdf_bench/benches/highlight.rs` (búsqueda indexada vs. lineal en documentos a 2 columnas y selección marquee).
- [x] A3. **Script de automatización `tools/adb-bench.sh`**: Ejecución en un comando de sweep de páginas, volcado PSS con dumpsys, captura de pantalla y extracción de percentiles desde logcat (verificado 2026-09-04 en TCL 9469X).
- [ ] A4. **Gate de benchmark en CI**: Integrar en `.github/workflows/ci.yml` la ejecución de `cargo test -p pdf_core` y `cargo bench -- --quick` con umbral de fallo ante regresiones de rendimiento. Estado: PENDIENTE (no existe en CI actual).
- [ ] A5. **Baseline completo de interacción en tablet TCL**:
  - Sweep sintético de 5 pasadas: superado (2026-09-04).
  - PSS bajo interacción continua: medido en 234.7 → 287.5 MB tras 15 cambios de página (2026-09-07).
  - 200 trazos simultáneos y 100 gestos de resaltado interactivo: PENDIENTE / BLOQUEADO por requerir stylus físico en el banco de pruebas (`docs/benchmark-results.md:697`).

## Criterio de cierre

- [x] Harness `tools/adb-bench.sh` funcional con reporte automático de métricas.
- [ ] Gate de regresión de bench activo en CI.
- [ ] Medición de 200 trazos y 100 gestos completada en hardware real.

## Referencias

- `crates/pdf_core/src/metrics.rs`
- `crates/pdf_android/src/gpu/surface.rs`
- `crates/pdf_android/src/gpu/pipeline.rs`
- `crates/pdf_bench/benches/highlight.rs`
- `docs/benchmark-results.md`
