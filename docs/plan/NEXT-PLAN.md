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
- E1/E4 [x] (E4 ejecutado: sin tope de libros) · E2/E3 [ ].
- Cifras de líneas actuales: `reader.rs` 5605 · `draw.rs` 4899 · `gpu.rs` 1899 · `input.rs` 1701.

Ver cada fase para detalle auditado y tareas.

## Deuda transversal

| Deuda | Estado | Dónde se cierra |
|---|---|---|
| EGL_BAD_ALLOC 0x3003 Library→Viewer | Abierta (CHANGELOG 2026-09-04) | Fase 2 del plan de reestructuración (2026-09-06) |
| Verificación ADR-007 §8.4 (PSS<150MB, p95<8.33ms) | Sin entrada en benchmark-results | Fase 2 del plan de reestructuración |
| Bug pantalla apagada | Abierta, hipótesis H1-H3 | Medición TCL pendiente (BUG-pantalla-apagada.md) |
| 419 unwrap/expect en tests/benches | Deuda registrada (ADR-008:37) | Limpieza continua |
