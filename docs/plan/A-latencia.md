# Fase A — Instrumentación y harness TCL (1-2 días, bloqueante)

> Sin esto todo lo demás es humo. Competencia mide p95; tú aún no.

## Auditoría (código real 2026-08-24)

- `metrics.rs` FrameTimer existe pero solo en `pdf_app`, no en `pdf_android`.
- `pdf_bench` mide `render1x 11-15ms` desktop/TCL, pero no mide `highlight_under_gesture` ni `composite_annotations`.
- `adb` no está conectado (hoy 0 devices). `dumpsys` y `screencap` son manuales.

## Objetivo

Harness reproducible: `cargo run -p pdf_bench` + `cargo apk run` + `adb` que mida p95 y PSS sin tocar la tablet a mano.

## Tareas (editar aquí)

- [ ] A1. Añadir `FrameTimer` a `pdf_android` (overlay debug opcional, logcat `frame p95=XXms`)
- [ ] A2. Bench `crates/pdf_bench/benches/highlight.rs` (gesto 100 puntos × 200 líneas) + `benches/composite.rs` (200 trazos)
- [ ] A3. Script `tools/adb-bench.sh` (inspirado en `prime-pdf-viewer` y `mupdf-android-viewer`): `adb shell dumpsys meminfo`, `screencap`, `logcat | grep frame`, con `svc power stayon true` (ver skill pdflector-rendimiento)
- [ ] A4. CI: `cargo test -p pdf_core` + `cargo bench -- --quick` con threshold `composite <5ms` (fail si regresa)
- [ ] A5. Medir baseline TCL (cuando conectes tablet): 5 runs `dense/paper/large` + 200 trazos + highlight 100 gestos → tabla en `docs/benchmark-results.md`

## Criterio de cierre

- [ ] `tools/adb-bench.sh` corre con 1 comando y deja `bench-results-TCL-$(date).json`
- [ ] p95 paint <16.6ms y `composite_annotations` <5ms medidos en TCL

## Referencias

- `crates/pdf_core/src/metrics.rs`, `overlay.rs`, `selection.rs`
- `docs/benchmark-results.md` (tu tabla actual)
- Competencia: `ArtifexSoftware/mupdf-android-viewer` (usa `fz_store` + `fz_cookie` para no bloquear UI)
