# Plan — PDFLector

> Fuente de verdad editable. Cada fase es un fichero. Estado real = GitHub Milestones + Issues #25-30.
> Edita estos ficheros para cambiar el objetivo. El agente lee `AGENTS.md` + este índice.

## Fases

| Fase | Fichero | Milestone | Issue | Estado |
|------|---------|-----------|-------|--------|
| 0 | `fases/00-andamiaje.md` | — | — | ✅ Cerrada 2026-08-05 |
| 0.5 | `fases/05-motor.md` | — | — | ✅ ADR-001 MuPDF/AGPL |
| 1 | `01-lectura-fluida.md` | Milestone 1 | #25 | 🟡 Lado app OK, falta medición HW |
| 2 | `02-modo-oscuro.md` | Milestone 2 | #26 | 🟡 Lado app OK, falta test |
| 3 | `03-anotaciones.md` | Milestone 3 | #27 | 🟡 Modelo OK, falta estrés 200 trazos |
| 4 | `04-sync.md` | Milestone 4 | #28 | 🟡 Código OK, falta E2E Syncthing |
| 5 | `05-ia.md` | Milestone 5 | #29 | 🟡 Código OK, falta E2E Ollama/Groq |
| 6 | `06-android.md` | Milestone 6 | #30 | 🟡 Spike OK, falta port Slint |

## Cómo modificar el plan

1. Edita el fichero de la fase (cambia objetivo, criterio o tareas).
2. Actualiza el Issue de GitHub correspondiente (`gh issue edit <n> --body "..."` o desde la web).
3. Si cambias arquitectura/decisión → crea `docs/adr/ADR-00X-*.md`.

## Presupuestos globales (AGENTS.md §MUST)

- p95 frame time <16.6ms (60fps), render <25ms en TCL, PSS <150MB con 500 pág.
- Medir antes de afirmar. Toda fase se cierra con log fecha+hardware.

## Decisiones cerradas

Ver `docs/PLAN.md` §1 y `docs/PROYECTO.md`. Motor=MuPDF, lápiz sin presión, export MD+PDF, sync Syncthing, repo público AGPL-3.0-or-later.
