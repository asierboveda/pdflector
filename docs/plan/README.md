# Plan — PDFLector

> **Plan vigente:** `NEXT-PLAN.md` (A-E, visor ultra-óptimo, aprobado 2026-08-24). Los ficheros `01-06` son histórico del plan waterfall obsoleto.
> **Fuente de verdad:** `NEXT-PLAN.md` + `A/B/C/D/E-*.md`. Edita esos ficheros para cambiar el objetivo.

## Vigente (editable ahora)

| Fase | Fichero | Issue | Objetivo |
|------|---------|-------|----------|
| A | `A-latencia.md` | — | Harness `adb` + p95 |
| B | `B-subrayado.md` | — | Subrayado 0-latencia |
| C | `C-pintado.md` | — | Pintado 200 trazos 60fps |
| D | `D-ia-contexto.md` | — | IA con RAG BM25 + visión |
| E | `E-library.md` | — | Library fluida (secundaria) |

## Histórico (no editar, solo referencia)

| Fase | Fichero | Estado |
|------|---------|--------|
| 1-6 | `01-lectura-fluida.md` ... `06-android.md` | Waterfall 17 semanas, descartado 2026-08-24 |

## Cómo modificar el plan vigente

1. Edita `NEXT-PLAN.md` (tabla de fases) o el fichero `A/B/C/D/E-*.md` (criterio/tareas).
2. Si cambias arquitectura → crea `docs/adr/ADR-00X-*.md`.
3. Harness TCL en `.opencode/skills/pdflector-rendimiento/SKILL.md`.

## Competencia

Ver `COMPETENCIA.md` (Xodo ~12ms, MuPDF ~10ms, prime-pdf-viewer Rust+Slint).
