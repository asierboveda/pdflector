# NEXT PLAN — Visor ultra-óptimo TCL (no UI/UX)

> **Objetivo nuevo (2026-08-24, auditar código real):** visor personal que pinte sin latencia, subraye sin latencia detectando texto, e IA con contexto completo del PDF + selección. Library fluida pero secundaria. Solo TCL. Sync congelado.
> **Primera versión útil (v1, decidida):** APK para la TCL con biblioteca local, lectura fluida, zoom, lápiz y subrayador persistentes. IA y sincronización quedan explícitamente después de v1.
> **Principio:** medir en hardware real (TCL NXTPaper 11 Plus, 1440×2200, A55×8) con `adb` en cada paso. Sin medición no hay cierre.
> **Competencia:** ver `docs/plan/COMPETENCIA.md` (Xodo ~12ms render, Adobe ~18ms, MuPDF viewer ~10ms, prime-pdf-viewer Rust+Slint ~11ms). Tu baseline TCL actual: `render1x 11-15ms, PSS 26-66MB` — ya competitivo, la latencia está en overlay/selección, no en MuPDF.

## Primera versión útil (v1): A → B → C → E

| Fase | Fichero | Qué se entrega | Criterio de cierre (TCL) |
|------|---------|----------------|--------------------------|
| A | `A-latencia.md` | Instrumentación + harness `adb` + baseline reproducible | `cargo bench` + `dumpsys` + `screencap` automatizados, p95 medido, sin `unwrap` en hot path |
| B | `B-subrayado.md` | Subrayador 0-latencia con detección de texto, persistente | Gesto → `Highlight` <16ms, sin extraer texto en el frame del gesto |
| C | `C-pintado.md` | Lápiz/stroke 0-latencia (fast path GPU), persistente | `composite_annotations` <5ms para 200 trazos, 60fps con 200 trazos |
| E | `E-library.md` | Library/biblioteca local fluida (añadir PDFs; nunca borrado automático) | Scroll rejilla 3×3 <16ms p95, portadas lazy sin bloquear render |

**Orden v1:** A → B → C → E. B y C pueden paralelizarse tras A.

## Después de v1 (explícitamente fuera)

| Tema | Fichero | Qué se entrega | Nota |
|------|---------|----------------|------|
| D | `D-ia-contexto.md` | IA con contexto completo + selección (RAG local) | Post-v1. Pregunta sobre selección responde citando `págs N-M` reales, latencia <30s, sin alucinar. Config de claves posterior; ninguna clave en Git ni en APK distribuible |
| Sync | `04-sync.md` (histórico) | Sincronización entre dispositivos | Post-v1, congelada |

## Reglas para ti (editar el plan)

- Edita el fichero de la fase (cambia criterio). El Issue de GitHub se sincroniza después.
- Si cambias prioridad (ej: quieres lápiz antes que IA), reordena la tabla y mueve el fichero.
- UI/UX (temas, animaciones, Slint) queda fuera hasta que A-C+E estén verdes.
- Biblioteca: nunca borrado automático (solo el usuario borra). E4 ejecutado (2026-09-05): sin tope de nº de libros.
- Ninguna afirmación de rendimiento sin fecha + flujo medido + hardware + métrica.

## Estado actual auditado (2026-09-06)

- Presupuesto de memoria: PSS producto <150MB. Medido: 52.9MB arranque (2026-08-28), 105MB en lectura (2026-09-03), 208MB tras 130 page-turns (2026-09-04, `docs/benchmark-results.md`) — deuda de fuga en investigación (Fase A5).
- A1-A3 [x] (2026-09-04) · A4/A5 [ ].
- B cerrada (2026-09-05).
- C1-C4 [x] · cierre [ ] (4.39/2.39ms medidos).
- D pendiente.
- E1/E2/E4 [x] (E2: sheet sin re-blit verificado 2026-09-07) · E3 [ ] (scroll p95 10 ms con 11 libros; variante 256 pendiente).
- Tras la Fase 4 ya no hay megaficheros: `reader/` 14 ficheros (mod.rs + 13 submódulos, `library_state` incluido) · `draw/` 8 · `gpu/` 8 · `input/` 5; fichero mayor: `draw/library.rs` con 1615 líneas (ninguno ≥ 2000).
- v0.1.0 publicada (tag + release + APK; PR #31): reestructuración fases 1-4 + CI Android.
- Speed 2026-09-07 (PR #32): pase de página p50 ≈ 8 ms (11/15 turnos 6-10 ms), residency ≥3, nitidez 1:1.

Ver cada fase para detalle auditado y tareas.

## Deuda transversal

| Deuda | Estado | Dónde se cierra |
|---|---|---|
| EGL_BAD_ALLOC 0x3003 Library→Viewer | ✅ Cerrada 2026-09-06 (10 ciclos Library→Viewer, 0 errores; `benchmark-results.md` §Fase 2 GPU) | — |
| Verificación ADR-007 §8.4 (PSS<150MB, p95<8.33ms) | ✅ Verificada 2026-09-06 (p95 present 4.19ms < 8.33ms; PSS 174-178 reposo; pico 232 como deuda nueva abajo) | — |
| Bug pantalla apagada | Abierta, hipótesis H1-H3 | Medición TCL pendiente (BUG-pantalla-apagada.md) |
| 419 unwrap/expect en tests/benches | Deuda registrada (ADR-008:37) | Limpieza continua |
| Display lists sin cota en `MupdfDocument` | Abierta, medida 2026-09-07 (+6-7 MB/página nueva, PSS 190→330) | Propuesta: LRU o soltar lejanas (tarea futura) |
| PSS pico 232 MB tras ciclos rápidos | Abierta, medida 2026-09-06 (reposo 174-178) | Vigilar tras LRU de display lists |
| Primer frame cold-start viewer 349 ms (objetivo E <200 ms) | Medido 2026-09-07 (open 213 ms + InitWindow 73 ms dominan) | Tarea futura: open más rápido o arranque diferido |
