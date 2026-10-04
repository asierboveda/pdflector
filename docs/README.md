# Documentación — PDFLector

> Índice único. Si algo no está listado aquí, no existe como documentación: lo que dejó de servir se borró y vive en el historial de git (`git log -- <ruta>`).

## Reglas

- **`AGENTS.md` manda** sobre todo lo demás: es el contrato operativo para agentes y humanos.
- **GitHub Issues es la única cola de trabajo pendiente.** El repo no mantiene un roadmap ni un backlog paralelo (`AGENTS.md` §3).
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
| [`../ESTADO_ACTUAL.md`](../ESTADO_ACTUAL.md) | Instantánea factual del software que existe, contrastada con el código |
| [`PROYECTO.md`](PROYECTO.md) | Visión, plataforma y alcance |

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
| [`ADR-009`](adr/ADR-009-evaluacion-tinta-causal-front-buffer.md) | Evaluación aislada de tinta causal y front buffer (experimento) |
| [`ADR-010`](adr/ADR-010-integracion-tinta-causal-producto.md) | Integración de la tinta causal en el producto Android — **sustituye ADR-009 para el producto** |

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
