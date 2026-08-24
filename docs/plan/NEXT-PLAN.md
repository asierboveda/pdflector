# NEXT PLAN — Visor ultra-óptimo TCL (no UI/UX)

> **Objetivo nuevo (2026-08-24, auditar código real):** visor personal que pinte sin latencia, subraye sin latencia detectando texto, e IA con contexto completo del PDF + selección. Library fluida pero secundaria. Solo TCL. Sync congelado.
> **Principio:** medir en hardware real (TCL NXTPaper 11 Plus, 1440×2200, A55×8) con `adb` en cada paso. Sin medición no hay cierre.
> **Competencia:** ver `docs/plan/COMPETENCIA.md` (Xodo ~12ms render, Adobe ~18ms, MuPDF viewer ~10ms, prime-pdf-viewer Rust+Slint ~11ms). Tu baseline TCL actual: `render1x 11-15ms, PSS 26-66MB` — ya competitivo, la latencia está en overlay/selección, no en MuPDF.

## Fases (orden técnico, no waterfall)

| Fase | Fichero | Qué se entrega | Criterio de cierre (TCL) |
|------|---------|----------------|--------------------------|
| A | `A-lactencia.md` | Instrumentación + harness `adb` + baseline reproducible | `cargo bench` + `dumpsys` + `screencap` automatizados, p95 medido, sin `unwrap` en hot path |
| B | `B-subrayado.md` | Subrayado 0-latencia con detección de texto | Gesto → `Highlight` <16ms, sin extraer texto en el frame del gesto |
| C | `C-pintado.md` | Pintado/stroke 0-latencia (fast path GPU) | `composite_annotations` <5ms para 200 trazos, 60fps con 200 trazos |
| D | `D-ia-contexto.md` | IA con contexto completo + selección (RAG local) | Pregunta sobre selección responde citando `págs N-M` reales, latencia <30s, sin alucinar |
| E | `E-library.md` | Library/biblioteca fluida (secundaria) | Scroll rejilla 3×3 <16ms p95, portadas lazy sin bloquear render |

**Orden:** A → B → C → D → E. B y C pueden paralelizarse tras A. D necesita B (texto pre-extraído).

## Reglas para ti (editar el plan)

- Edita el fichero de la fase (cambia criterio). El Issue de GitHub se sincroniza después.
- Si cambias prioridad (ej: quieres lápiz antes que IA), reordena la tabla y mueve el fichero.
- UI/UX (temas, animaciones, Slint) queda fuera hasta que A-D estén verdes.

## Estado actual auditado (2026-08-24)

- `pdf_core`: `selection.rs` (rotulador real, BAND_TOL 1pt, 2 columnas OK) + `overlay.rs` (fill_rect O(h+w), draw_stroke por segmento) + `ai.rs` (chunk_pages + 3 clients). OK pero `chunk_pages` no tiene índice ni RAG.
- `pdf_android`: `Reader` monolito (134 pág. en `reader.rs`), `draw.rs` 3.7k líneas, caché LRU 48MiB, `page_frame` cache para sheet. Latencia viene de: `Document::text()` en el hilo UI + `composite_annotations` por frame + `sel` en coords pantalla sin índice espacial.
- Tests: 419 `unwrap/expect` en hot path, sin `cargo bench` gate en CI, sin harness `adb` automatizado.

Ver cada fase para detalle auditado y tareas.
