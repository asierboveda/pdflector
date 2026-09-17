# Documentación — PDFLector

> Índice único. Si algo no está listado aquí, no existe como documentación: lo que dejó de servir se borró y vive en el historial de git (`git log -- <ruta>`).

## Reglas

- **`AGENTS.md` manda** sobre todo lo demás: es el contrato operativo para agentes y humanos.
- **El repo es la verdad de plan y diseño.** GitHub Issues es solo la cola de trabajo.
- **Nada de documentos zombie.** Un documento que no sirve para una decisión futura se borra, no se congela con un banner.
- **Toda afirmación de rendimiento exige fecha + hardware + flujo medido + métrica.** Si no hay medición, se escribe `SIN MEDIR`.

## Mapa

### Operación

| Documento | Contenido |
|---|---|
| [`../AGENTS.md`](../AGENTS.md) | Reglas operativas: arquitectura, comandos, definición de hecho, protocolo de agentes en paralelo |
| [`../README.md`](../README.md) | Portada pública del repo (inglés) |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | Cómo contribuir y verificar |
| [`legal.md`](legal.md) | Estado de cumplimiento de licencias (AGPL-3.0-or-later + MuPDF) |

### Producto

| Documento | Contenido |
|---|---|
| [`PROYECTO.md`](PROYECTO.md) | Visión, plataforma y alcance |

### Plan

| Documento | Contenido |
|---|---|
| [`plan/NEXT-PLAN.md`](plan/NEXT-PLAN.md) | **Único roadmap editable**: fases A–F |
| [`plan/00-objetivo.md`](plan/00-objetivo.md) | Norte del producto: prioridades y métricas |
| [`plan/DEUDA.md`](plan/DEUDA.md) | Deuda técnica medida y backlog de arquitectura |
| [`plan/A-latencia.md`](plan/A-latencia.md) | Fase A — instrumentación y harness |
| [`plan/B-subrayado.md`](plan/B-subrayado.md) | Fase B — subrayado sin latencia (cerrada) |
| [`plan/C-pintado.md`](plan/C-pintado.md) | Fase C — pintado a mano alzada (GPU dry/wet) |
| [`plan/D-ia-contexto.md`](plan/D-ia-contexto.md) | Fase D — IA con contexto del documento |
| [`plan/E-library.md`](plan/E-library.md) | Fase E — biblioteca |
| [`plan/F-arxiv.md`](plan/F-arxiv.md) | Fase F — arXiv / Discover |
| [`plan/README.md`](plan/README.md) | Mapa del directorio de plan |

### Decisiones (ADR)

Snapshots inmutables con `Estado` y `Fecha`. No se reescriben; se sustituyen con otro ADR.

| ADR | Decisión |
|---|---|
| [`ADR-001`](adr/ADR-001-motor-pdf.md) | Motor PDF: MuPDF (AGPL-3.0). Decisión mantenida por el dueño; su justificación está disputada por una medición del propio repo, y eso queda registrado en la evidencia |
| [`ADR-002`](adr/ADR-002-arquitectura-evince-android.md) | Arquitectura de Evince: análisis de referencia (no es una decisión) |
| [`ADR-003`](adr/ADR-003-baseline-evince-vs-pdfium.md) | Baseline Evince vs PDFium (evidencia histórica) |
| [`ADR-004`](adr/ADR-004-ui-android.md) | UI Android: Slint — **sustituido por ADR-005** |
| [`ADR-005`](adr/ADR-005-ui-android-nativa.md) | UI Android: stack nativo propio (`pdf_android`) |
| [`ADR-006`](adr/ADR-006-motor-stylus-baja-latencia.md) | Motor de stylus de baja latencia (EGL/GLES2) |
| [`ADR-007`](adr/ADR-007-pipeline-wet-dry-ink.md) | Pipeline de tinta en dos capas (wet/dry) |
| [`ADR-008`](adr/ADR-008-integracion-zoom-ux-stylus.md) | Integración de zoom + UI/UX sobre el stack de stylus |

### Evidencia y registro

| Documento | Contenido |
|---|---|
| [`benchmark-results.md`](benchmark-results.md) | **Registro único de evidencia**, append-only por fecha. Incluye el baseline de Evince, la comparativa PDFium vs MuPDF y sus discrepancias |
| [`api-anotaciones-fase3.md`](api-anotaciones-fase3.md) | API del motor de anotaciones (`pdf_core`): firmas y semántica |
| [`../CHANGELOG.md`](../CHANGELOG.md) | Registro completo de cambios, más reciente arriba |

### Procedimientos

| Documento | Contenido |
|---|---|
| [`../.opencode/skills/pdflector-rendimiento/SKILL.md`](../.opencode/skills/pdflector-rendimiento/SKILL.md) | Despliegue en la tablet y medición de rendimiento (harness `adb`) |
