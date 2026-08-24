# Fase E — Biblioteca fluida (2-3 días, secundaria)

> Dijiste: "no importa que sea hiper óptima, importa más que sea óptimo el lector". Auditado: library es la más pesada (rejilla 3×3, portadas lazy, continue reading, chips).

## Auditoría

- `pdf_android/src/reader.rs` 4.3k líneas, `draw.rs` 3.7k: library tiene `lib_scroll` en píxeles, `lib_band` cache (banda scrolleable), `thumbs.rs` LRU 36 entradas/9MiB/200px, `ThumbCache`, `lib_cont_*` carousel, `lib_org_*` sort/filter.
- Ya tiene `compose_library_snapshot` + `blit_lib_fade` para transición lista→visor (bien).
- Problema: portadas se generan con `openFileDescriptor + /proc/self/fd` en `tick` (≤3/tick) pero bloquean `reader` si hay 256 PDFs (MediaStore scan).
- No crítico para tu objetivo actual.

## Objetivo

Scroll rejilla sin jank, portadas sin bloquear apertura de PDF.

## Tareas

- [ ] E1. **Portadas sin bloquear**: `ThumbCache` ya tiene `THUMB_BYTE_BUDGET`, pero `pump_thumbs` corre en `tick` (16ms). Mover a hilo fondo (actor como `prefetch.rs`, 1 worker con `MupdfEngine` por hilo).
- [ ] E2. **Menu/sheet**: `sheet_progress` 0→0.5 con `compose_frame` cacheado ya es 1-2ms (bien). Solo pulir: asegura `sheet_anim` no re-blitea página (ya hace `blit_composed`).
- [ ] E3. **Medir**: `adb-bench.sh` library 256 PDFs, p95 scroll <16ms

## Criterio de cierre

- [ ] Abrir biblioteca 256 PDFs → primer frame <200ms, scroll p95 <16ms en TCL

## Cómo modificar

- Si quieres simplificar a lista (no rejilla): borra `grid_*` y usa `picker_row_h` (ahorro ~2k líneas).

## Referencias

- `crates/pdf_android/src/thumbs.rs`, `reader.rs: lib_*`, `draw.rs: render_library_grid`
