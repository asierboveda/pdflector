# Fase 1 — Lectura fluida

> Milestone: `Fase 1 — Lectura fluida` · Issue: #25 · Estado: 🟡 Lado app OK, falta verificación HW
> Edita este fichero para cambiar el objetivo. Sincroniza con `gh issue edit 25`.

## Objetivo

Scroll continuo virtualizado sin tirones en la TCL NXTPaper 11 Plus (9469X, MT8781, 8GB, 1440×2200).

## Criterio de aceptación (medible)

- [ ] `render1x <25ms` en 3/4 PDFs del corpus (sweep `pdf_bench` release, pantalla ON, N≥5, mediana)
- [ ] `PSS <150MB` con `large_document.pdf` (500 pág.)
- [ ] `p95 frame time <16.6ms` en `pdf_app` overlay (scroll 500 pág.)
- [ ] `scanned_pages.pdf` documentado como worst case (31ms esperado, no bloquea)

Actual: 2026-08-12 sweep TCL: dense 14.5ms, paper 11.6ms, large 15.4ms, scanned 31ms, PSS 26.7MB → 3/4 OK.

## Tareas

- [x] `cache.rs` LRU por bytes + `scroll.rs` visible_and_prefetch
- [x] `prefetch.rs` actor 1-worker (MuPDF no Send)
- [x] `zoom.rs` scale_bitmap + scale_level_for_zoom + trim_to_scale_level
- [x] `pdf_android` PageCache LRU 48MiB + blit_page + cover
- [ ] Medición final HW: `cargo run -p pdf_bench` en TCL (ver `.opencode/skills/pdflector-rendimiento/SKILL.md`)
- [ ] Overlay p95 en TCL (no solo desktop)

## Notas para modificar

- Si quieres descartar modo paginado: ya descartado (ver memory 2026-08-13).
- Si quieres bajar presupuesto a 120fps: cambia `p95<8.3ms` y re-mide.

## Referencias

- `crates/pdf_core/src/cache.rs`, `scroll.rs`, `prefetch.rs`, `zoom.rs`
- `docs/benchmark-results.md` (histórico de mediciones)
- `.opencode/skills/pdflector-rendimiento/SKILL.md` (procedimiento de medición)
