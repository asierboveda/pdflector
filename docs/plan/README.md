# Plan — PDFLector

Índice y mapa del plan de desarrollo de PDFLector.

## Fuente de verdad híbrida

1. **Repositorio (`docs/plan/`)**: Fuente de verdad de diseño, arquitectura, prioridades y roadmap general.
2. **GitHub Issues**: Cola operativa de trabajo. Cada issue representa una tarea concreta, acotada y cerrable con criterios de aceptación medibles.

## Mapa del plan

El plan consta de 10 documentos canónicos:

| Fichero | Rol | Estado |
|---|---|---|
| `00-objetivo.md` | Visión, prioridades innegociables y DoD por escenario | Vigente |
| `NEXT-PLAN.md` | Roadmap consolidado (fases A–F y síntesis de deuda) | Vigente |
| `A-latencia.md` | Fase A: Instrumentación, harness de medición y baselines | Parcial (A1–A3 hechos, A4–A5 pendientes) |
| `B-subrayado.md` | Fase B: Subrayador sin latencia en orden de lectura | Cerrada (2026-09-05) |
| `C-pintado.md` | Fase C: Lápiz y trazo fluido (pipeline GPU Dry/Wet) | Activa (pipeline funcional, cierre en tablet pendiente) |
| `D-ia-contexto.md` | Fase D: IA con contexto global del PDF (RAG BM25 + visión) | Pendiente de inicio |
| `E-library.md` | Fase E: Biblioteca fluida (rejilla, portadas en fondo) | Activa (E1/E2/E4 hechos, E3 parcial) |
| `F-arxiv.md` | Fase F: Catálogo arXiv y pestaña Discover integrada | En vigor (módulos core y UI funcionales) |
| `DEUDA.md` | Inventario consolidado de deuda técnica y backlog técnico | Vigente (items medidos e ideas sin medir) |

## Cómo modificar el plan

1. **Cambio de prioridades o requisitos globales**: Edita `00-objetivo.md`.
2. **Evolución del roadmap**: Actualiza `NEXT-PLAN.md` y el fichero específico de la fase (`A`–`F`).
3. **Registro de deuda o mejoras técnicas transversales**: Añade o actualiza la entrada en `DEUDA.md` indicando claramente si está `medido` o es una `idea sin medir`.
4. **Cambios arquitectónicos**: Cuando un cambio altera el diseño estructural o descarta componentes, documenta la decisión formal en `docs/adr/ADR-00X-*.md`.
