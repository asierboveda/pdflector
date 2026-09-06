# Objetivo — PDFLector

> Edita este fichero para cambiar el objetivo del proyecto. Es lo que el agente y tú usáis como norte.

## Visión (1 línea)

Lector de PDFs rápido y ligero para tablet Android con lápiz (TCL NXTPaper 11 Plus, 200€). Gratis, sin anuncios, sin telemetría. Primer proyecto real del autor en Rust.

## Prioridades (en orden, innegociables)

1. **Fluidez total** — 60fps mínimo sostenido, objetivo 120fps (p95 <16.6ms)
2. **RAM mínima** — <150MB PSS en tablet con PDFs de 500+ pág.
3. Gratis y sin anuncios
4. Aprendizaje — entender Rust y decisiones técnicas

Si una propuesta mejora algo a costa de 1 o 2, se rechaza o se consulta antes de actuar.

## Lo que NO es objetivo

- Presión del lápiz (no necesaria, ver docs/PROYECTO.md "Decisión sobre presión del lápiz" (la decisión real vive allí, no numerada aquí))
- Scroll continuo (descartado 2026-08-13, el visor es paginado de una hoja con tap)
- Servidor propio / telemetría / pagos

## Cómo modificar el objetivo

Edita las prioridades arriba. Si cambias 1 o 2, actualiza también `AGENTS.md` (MUST) y `docs/plan/01-lectura-fluida.md` (criterios).

## Métricas de éxito

- Fases waterfall descartadas: el roadmap vigente es `docs/plan/NEXT-PLAN.md` (A–E), editable.
- Obsidian y Syncthing quedan fuera de v1 (post-v1, ver `docs/plan/NEXT-PLAN.md`).
