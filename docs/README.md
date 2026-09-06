# Índice de documentación PDFLector

> Convención: los ADR son snapshots inmutables de su fecha (cifras y rutas
> citadas no se reescriben; solo el campo Estado se normaliza). Los históricos
> se congelan con banner y no se editan. Los activos se listan primero.

## Activos
- `AGENTS.md` — reglas operativas para agentes (raíz del repo).
- `docs/PROYECTO.md` — visión, plataforma y alcance.
- `docs/plan/NEXT-PLAN.md` — ÚNICO roadmap editable (fases A–E + deuda transversal).
- `docs/plan/README.md` — mapa de plan/ (vigente e histórico).
- `docs/plan/00-objetivo.md` — norte del producto (métricas y prioridades).
- `docs/plan/A-latencia.md` — fase A (checkboxes con evidencia).
- `docs/plan/B-subrayado.md` — fase B (checkboxes con evidencia).
- `docs/plan/C-pintado.md` — fase C (checkboxes con evidencia).
- `docs/plan/D-ia-contexto.md` — fase D (checkboxes con evidencia).
- `docs/plan/E-library.md` — fase E (checkboxes con evidencia).
- `docs/plan/COMPETENCIA.md` — comparativa con lectores de referencia.
- `docs/plan/BUG-pantalla-apagada.md` — bug abierto (hipótesis H1-H3).
- `docs/benchmark-results.md` — evidencia de mediciones (se añade por fase).
- `CHANGELOG.md` — registro completo, más reciente arriba.
- `docs/log/memory-2026-08.md` — registro histórico mensual (se cierra al mes).
- `README.md`, `CONTRIBUTING.md` — portada y guía de contribución (raíz).
- `docs/legal.md` — cumplimiento AGPL (VIGENTE, no histórico).

## Históricos (congelados, no editar)
- `docs/PLAN.md` — índice histórico de planes 1-6.
- `docs/PLAN-UX-UI-NUEVO.md` — propuesta UX 2026-08 (congelada 2026-09-04).
- `docs/plan/PLAN-IMPLEMENTACION-TECNICA-PRO.md`, `docs/plan/PLAN-UX-UI-PRO.md` — planes Pro (congelados).
- `docs/auditoria-contexto-agentes-2026-08-23.md` — auditoría de contexto (sus skills "exportar-anotaciones" y "syncthing-sync" NUNCA existieron).
- `docs/ux-rediseño-estructura.md`, `docs/api-anotaciones-ui.md` (superseded), `docs/api-anotaciones-fase3.md`.
- `docs/plan/01-lectura-fluida.md`, `docs/plan/02-modo-oscuro.md`, `docs/plan/03-anotaciones.md`, `docs/plan/04-sync.md`, `docs/plan/05-ia.md`, `docs/plan/06-android.md` — planes históricos fase a fase.
- `docs/research/android-native-readers.md` — investigación.
- `docs/research/annotations-selection.md` — investigación.
- `docs/research/comparativa-boli.md` — investigación.
- `docs/research/evince-architecture.md` — investigación.
- `docs/research/library-ui.md` — investigación.
- `docs/research/notas-android-estudio.md` — investigación.
- `docs/research/rendering-cache.md` — investigación.
- `docs/research/separacion-dedo-stylus.md` — investigación.
- `docs/research/tinta-directa-patron.md` — investigación.
- `docs/research/zoom-pan-ux.md` — investigación.
- `docs/investigacion/evince-baseline.md` — investigación.
- `docs/benchmarks/desktop-engine-shootout.md` — comparativa de motores (histórico).

## Superseded
- `docs/adr/ADR-004-ui-android.md` (por ADR-005) · `docs/api-anotaciones-ui.md` (por fase 3 real).

## ADRs (docs/adr/, snapshots inmutables)
- `docs/adr/ADR-001-motor-pdf.md` — MuPDF.
- `docs/adr/ADR-002-arquitectura-evince-android.md` — Evince→Android.
- `docs/adr/ADR-003-baseline-evince-vs-pdfium.md` — baseline.
- `docs/adr/ADR-004-ui-android.md` — UI Android (superseded por ADR-005).
- `docs/adr/ADR-005-ui-android-nativa.md` — UI nativa (supersede 004).
- `docs/adr/ADR-006-motor-stylus-baja-latencia.md` — stylus EGL/GLES2.
- `docs/adr/ADR-007-pipeline-wet-dry-ink.md` — Wet/Dry (supersede parcial del present de 006; estado: Aceptado).
- `docs/adr/ADR-008-integracion-zoom-ux-stylus.md` — zoom+UI/UX.

## Plan y spec de la reestructuración (docs/superpowers/)
- `docs/superpowers/plans/2026-09-06-reestructuracion-pdflector.md` — plan de implementación (Fases 1-4).
- `docs/superpowers/specs/2026-09-06-reestructuracion-pdflector-design.md` — spec de diseño de la reestructuración.
