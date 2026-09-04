# Fase A — Instrumentación y harness TCL (1-2 días, bloqueante)

> Sin esto todo lo demás es humo. Competencia mide p95; tú aún no.

## Auditoría (código real 2026-08-24)

- `metrics.rs` FrameTimer existe pero solo en `pdf_app`, no en `pdf_android`.
- `pdf_bench` mide `render1x 11-15ms` desktop/TCL, pero no mide `highlight_under_gesture` ni `composite_annotations`.
- `adb` no está conectado (hoy 0 devices). `dumpsys` y `screencap` son manuales.

## Objetivo

Harness reproducible: `cargo run -p pdf_bench` + `cargo apk run` + `adb` que mida p95 y PSS sin tocar la tablet a mano.

## Tareas (editar aquí)

- [x] A1. `FrameTimer` en `pdf_android` (gpu.rs `present_viewer`: anillo + `frame p95=` cada 120 presents). Verificado 2026-09-04 en TCL 9469X (195 presents por page-turn por tap; logcat `frame p95=497.0ms (240 frames)` — intervalo de tap, no frame rate; p95 real con gesto continuo en B/C).
- [x] A2. Bench `crates/pdf_bench/benches/highlight.rs` (pts×líneas + 2 columnas + marquee) + `benches/composite.rs` (10/50/200 trazos a 1440×2200). Corren en host (`--quick` OK); composite 200 trazos ≈5.36ms host x86 (TCL pendiente de bench cruzado en A4).
- [x] A3. Script `tools/adb-bench.sh`: 1 comando (sweep×5 + dumpsys + screencap + logcat p95 + JSON). Verificado 2026-09-04 en TCL (genera `bench-results-TCL-*.json` con sweep+PSS+p95). Robusto a app sin proceso (`trap` + `|| true`).
- [ ] A4. CI: `cargo test -p pdf_core` + `cargo bench -- --quick` con threshold `composite <5ms` (fail si regresa). Pendiente: fijar threshold tras medir composite en TCL.
- [ ] A5. Baseline TCL: sweep 5 runs ✅ (2026-09-04, tabla en `docs/benchmark-results.md`); pendiente 200 trazos + highlight 100 gestos en TCL (harness B/C) y PSS bajo interacción (208MB tras 130 page-turns, ver nota).

## Referencias

- `crates/pdf_core/src/metrics.rs`, `overlay.rs`, `selection.rs`
- `docs/benchmark-results.md` (tu tabla actual)
- Competencia: `ArtifexSoftware/mupdf-android-viewer` (usa `fz_store` + `fz_cookie` para no bloquear UI)
