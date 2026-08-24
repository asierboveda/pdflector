# Fase 4 — Sincronización

> Milestone: `Fase 4 — Sincronización` · Issue: #28 · Estado: 🟡 Código OK, falta E2E Syncthing

## Objetivo

Anotar en tablet → visible en PC en <1min, sin corrupción en conflicto.

## Criterio de aceptación

- [x] Layout `BibliotecaPDF/ + annotations/<id>.db` (sidecar por PDF)
- [x] `sync.rs` watch_annotations con `notify` debounce 150ms + hot-reload
- [ ] Syncthing instalado en TCL + PC + móvil, carpeta compartida
- [ ] Conflicto simulado (edición simultánea) → `.stversions` sin corrupción

## Tareas

- [x] `crates/pdf_core/src/sync.rs`
- [ ] Infra Syncthing + prueba E2E

## Cómo modificar

- Si quieres sync por otro medio (Nextcloud, Git): cambia este fichero y crea ADR (impacta `store.rs`).

## Referencias

- `crates/pdf_core/src/sync.rs`
