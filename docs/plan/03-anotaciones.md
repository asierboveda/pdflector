# Fase 3 — Anotaciones y exportación

> Milestone: `Fase 3 — Anotaciones y exportación` · Issue: #27 · Estado: 🟡 Modelo OK, falta estrés HW

## Objetivo

Anotaciones vectoriales en márgenes (subrayado, boli, nota) nítidas a cualquier zoom, sin penalizar scroll.

## Criterio de aceptación

- [ ] 200 trazos visibles sin degradar p95 (medir en TCL con bench `annotations.rs` + frame time)
- [ ] Export MD (citas + nº página) abre bien en Obsidian
- [ ] Export PDF con anotaciones /Ink /Highlight /Text legible en lector externo (pypdf validado)
- [ ] Sidecar `annotations/<stem>.db` sync-friendly (un fichero por PDF)

## Tareas

- [x] `annotations.rs` Stroke/Highlight/TextNote + serde
- [x] `store.rs` rusqlite sidecar + `export.rs` MD/PDF
- [x] `pdf_app` capa vectorial + panel anotaciones + `pdf_bench/benches/annotations.rs`
- [x] `pdf_android` Stroke con Bresenham + paleta + undo (luego eliminado en rediseño minimalista, ver 2026-08-18)
- [ ] Medición estrés 200 trazos en TCL (Fase 6)
- [ ] Validación export E2E (Obsidian + lector externo)

## Cómo modificar

- Si quieres recuperar lápiz en `pdf_android` (eliminado en 2026-08-18): re-activa `annotations.rs` (tiene `allow(dead_code)`) y re-añade gesto ✏️.
- Si quieres cambiar formato sidecar: documenta en ADR y migra `store.rs`.

## Referencias

- `crates/pdf_core/src/annotations.rs`, `store.rs`, `export.rs`
- `docs/api-anotaciones-fase3.md`, `docs/api-anotaciones-ui.md`
