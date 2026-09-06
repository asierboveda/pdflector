# Fase 2 — Modo oscuro

> **[HISTÓRICO — congelado 2026-09-06]** Documento congelado: su contenido refleja
> el estado de 2026-08. No editar. El roadmap vigente vive en `docs/plan/NEXT-PLAN.md`.

> Milestone: `Fase 2 — Modo oscuro` · Issue: #26 · Estado: 🟡 Lado app OK, falta test de regresión

## Objetivo

UI oscura + páginas invertidas sin re-render, conmutación instantánea.

## Criterio de aceptación

- [x] Toggle Dark/Light en `pdf_app` (eframe storage) y `pdf_android` (sheet)
- [x] `dark::invert_bitmap` solo en blit (caché siempre normal)
- [ ] Test: caché no mezcla bitmaps invertidos/normales tras toggle
- [ ] Persistencia verificada tras reinicio

## Tareas

- [x] `crates/pdf_core/src/dark.rs`
- [x] Integración en `pdf_app` y `pdf_android/draw.rs`
- [ ] Test de regresión cache+dark

## Cómo modificar

- Si quieres inversión por shader GPU en Slint (Fase 6) en vez de CPU: anótalo aquí y crea ADR.

## Referencias

- `crates/pdf_core/src/dark.rs`
