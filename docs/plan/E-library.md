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
- [x] E1. **Portadas sin bloquear**: `ThumbWorker` actor en segundo plano (`crates/pdf_android/src/thumbs.rs`) con instancia propia de `MupdfEngine`, canal MPSC (`Sender`/`Receiver`) y preemption de cola. El hilo UI solo hace `try_recv()` no bloqueante en `tick()`. Verificado en hardware TCL: blits fluidos de ~5.5–6.3 ms mientras las portadas cargan progresivamente en segundo plano sin congelar la UI.
- [ ] E2. **Menu/sheet**: `sheet_progress` 0→0.5 con `compose_frame` cacheado ya es 1-2ms (bien). Solo pulir: asegura `sheet_anim` no re-blitea página (ya hace `blit_composed`).
- [ ] E3. **Medir**: `adb-bench.sh` library 256 PDFs, p95 scroll <16ms
- [x] E4. **Eliminar cualquier borrado automático**: eliminada la función `enforce_library_limit`, la constante `LIBRARY_MAX` (límite histórico de 50 libros) y los comandos de borrado de ficheros `fs::remove_file` al añadir libros. La biblioteca nunca borra un PDF automáticamente; solo la acción explícita del usuario desde el menú puede borrar un libro.

## Criterio de cierre

- [ ] Abrir biblioteca 256 PDFs → primer frame <200ms, scroll p95 <16ms en TCL

## Cómo modificar

- Si quieres simplificar a lista (no rejilla): borra `grid_*` y usa `picker_row_h` (ahorro ~2k líneas).

## Referencias

- `crates/pdf_android/src/thumbs.rs`, `reader.rs: lib_*`, `draw.rs: render_library_grid`
