# PLAN DE IMPLEMENTACIÓN — PDFLector (índice)

> Este fichero es solo índice. El plan editable vive en `docs/plan/` (un fichero por fase).
> Fuente de verdad de estado: GitHub Milestones + Issues #25-30. Ver `docs/plan/README.md`.

## Objetivo

Ver `docs/plan/00-objetivo.md` — fluidez 60fps + RAM <150MB PSS en TCL NXTPaper 11 Plus.

## Decisiones cerradas

| # | Decisión | Resultado |
|---|----------|-----------|
| 1 | Motor PDF | MuPDF (ADR-001) — AGPL-3.0 |
| 2 | Presión lápiz | No necesaria |
| 3 | Distribución | GitHub público, AGPL-3.0-or-later |
| 4 | Exportar notas | Markdown + PDF con anotaciones |
| 5 | Modo oscuro | UI + páginas invertidas |
| 6 | Sync | Syncthing (sidecar por PDF) |

## Fases (editable en `docs/plan/`)

| Fase | Fichero | Estado |
|------|---------|--------|
| 0 Andamiaje | `docs/plan/fases/00-andamiaje.md` (histórico) | ✅ 2026-08-05 |
| 0.5 Motor | `docs/plan/fases/05-motor.md` (histórico) | ✅ ADR-001 |
| 1 Lectura fluida | `docs/plan/01-lectura-fluida.md` | 🟡 Issue #25 |
| 2 Modo oscuro | `docs/plan/02-modo-oscuro.md` | 🟡 Issue #26 |
| 3 Anotaciones | `docs/plan/03-anotaciones.md` | 🟡 Issue #27 |
| 4 Sync | `docs/plan/04-sync.md` | 🟡 Issue #28 |
| 5 IA | `docs/plan/05-ia.md` | 🟡 Issue #29 |
| 6 Android | `docs/plan/06-android.md` | 🟡 Issue #30 |

Edita el fichero de la fase y su Issue en GitHub para cambiar el plan.

## Arquitectura y medición

Ver `AGENTS.md` (TL;DR) y `.opencode/skills/pdflector-rendimiento/SKILL.md` (procedimiento de medición).
