# PLAN DE IMPLEMENTACIÓN — PDFLector (histórico)

> **HISTÓRICO — no editar.** El roadmap vigente es **`docs/plan/NEXT-PLAN.md` (fases A–E)**.
> Ver también `docs/plan/README.md`. Este fichero se conserva solo como referencia de las fases 1–6.

## Objetivo (histórico)

Ver `docs/plan/00-objetivo.md` — fluidez 60fps + RAM <150MB PSS en TCL NXTPaper 11 Plus.

## Decisiones cerradas (históricas)

| # | Decisión | Resultado |
|---|----------|-----------|
| 1 | Motor PDF | MuPDF (ADR-001) — AGPL-3.0 |
| 2 | Presión lápiz | No necesaria |
| 3 | Distribución | GitHub público, AGPL-3.0-or-later |
| 4 | Exportar notas | Markdown + PDF con anotaciones |
| 5 | Modo oscuro | UI + páginas invertidas |
| 6 | Sync | Sincronización congelada (fuera de v1; ver `docs/plan/NEXT-PLAN.md`) |
| 7 | Plataforma final | `pdf_android` nativa (ADR-005; ADR-004/Slint superseded) |

## Fases 1–6 (históricas, no editables)

| Fase | Fichero | Estado |
|------|---------|--------|
| 0 Andamiaje | `docs/plan/00-objetivo.md` (histórico) | ✅ 2026-08-05 |
| 0.5 Motor | `docs/benchmark-results.md` + ADR-001 (histórico) | ✅ ADR-001 |
| 1 Lectura fluida | `docs/plan/01-lectura-fluida.md` | 🟡 histórico |
| 2 Modo oscuro | `docs/plan/02-modo-oscuro.md` | 🟡 histórico |
| 3 Anotaciones | `docs/plan/03-anotaciones.md` | 🟡 histórico |
| 4 Sync | `docs/plan/04-sync.md` | 🟡 histórico, fuera de v1 |
| 5 IA | `docs/plan/05-ia.md` | 🟡 histórico, fuera de v1 |
| 6 Android | `docs/plan/06-android.md` | 🟡 histórico (Slint descartado por ADR-005) |

Para cambiar el plan, edita `docs/plan/NEXT-PLAN.md` o el fichero `A/B/C/D/E-*.md`.

## Arquitectura y medición (histórico)

Ver `AGENTS.md` (reglas vigentes) y `.opencode/skills/pdflector-rendimiento/SKILL.md` (procedimiento de medición).
Métrica de producto: PSS (`dumpsys`); RSS/VmHWM solo diagnóstico de host.
