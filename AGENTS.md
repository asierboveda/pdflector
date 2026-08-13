# AGENTS.md — PDFLector

> Reglas de comportamiento para cualquier agente de IA que trabaje en este repositorio.
> Contexto completo del proyecto: `docs/` (vault de Obsidian dentro del propio repo):
> `docs/PROYECTO.md` = visión, `docs/PLAN.md` = plan de implementación por fases.

## 1. Qué es este proyecto

Lector de PDFs rápido y ligero para tablet Android con lápiz (TCL NXTPaper 11 Plus, ~200 €).
Gratis, sin anuncios, sin pagos, sin telemetría. Proyecto personal de **aprendizaje**:
es el primer proyecto real del autor en Rust. Desarrollo inicial en escritorio Linux
(Omarchy/Arch); destino final Android.

## 2. Prioridades (en este orden, siempre)

1. **Fluidez total** — 60 fps mínimo sostenidos en scroll; objetivo 120 fps.
2. **Consumo mínimo de RAM** — < 150 MB RSS en tablet con PDFs de 500+ páginas.
3. Gratis y sin anuncios.
4. **Aprendizaje** — el autor quiere entender Rust y las decisiones técnicas.

Si una propuesta mejora algo a costa de las prioridades 1 o 2, se rechaza o se
plantea explícitamente al autor antes de actuar.

## 3. Cómo debe comportarse el agente

- **Idioma**: responde siempre en **español**. Código, identificadores, ficheros y
  mensajes de commit en inglés.
- **Modo aprendizaje**: al introducir un concepto de Rust nuevo o una crate nueva,
  explica brevemente el porqué (2-4 líneas). No sermonees sobre lo ya establecido.
- **Cambios mínimos**: implementa exactamente lo pedido. Nada de features,
  refactorizaciones ni "mejoras" no solicitadas.
- **Medir antes de optimizar**: ninguna afirmación sobre rendimiento sin datos
  (benchmark, overlay de debug, `/proc`). Toda optimización propuesta debe indicar
  cómo se va a medir.
- **No asumas decisiones pendientes** (ver §6). Si la tarea depende de una decisión
  abierta, pregunta antes de escribir código.
- **Dependencias**: cada crate nueva se justifica en una línea (qué aporta,
  mantenimiento, licencia). Preferir `std` y crates consolidadas. `unsafe` solo si
  es imprescindible, acotado y comentado.
- **Licencias**: solo dependencias compatibles con distribución gratuita
  (MIT/Apache-2.0/BSD/CC0...). **Nada de copyleft (AGPL/GPL/LGPL) sin decisión
  explícita del autor** — está ligada a la elección de motor PDF, ver §6.
- **Git**: nunca `init`/`commit`/`push`/ramas sin que el autor lo pida explícitamente.
- **Una sola carpeta**: TODO lo del proyecto vive dentro de `~/Projects/pdflector/`.
  En `~/Projects/` cada carpeta es un proyecto distinto: prohibido crear archivos
  o carpetas de este proyecto fuera de la suya. Excepciones: herramientas del
  sistema (Android SDK en `~/Android/Sdk`), configuración del propio agente
  (skills globales) y `/tmp` para temporales desechables.
- **Documenta TODO lo que hagas** (ver §12): ninguna decisión, medición o cambio
  queda solo en el chat. Si no está documentado, no está hecho.
- **`memory.md`**: mantén al día el registro cronológico de versiones y cambios
  del proyecto (ver §13).

## 4. Arquitectura (innegociable)

Workspace cargo:

```
crates/pdf_core/    # Lógica pura: documento, render, caché, anotaciones,
                    # persistencia, exportación. SIN UI.
crates/pdf_app/     # UI egui/eframe — SOLO prototipo de escritorio.
crates/pdf_bench/   # Harness de benchmarks (escritorio y Android).
docs/adr/           # Decisiones de arquitectura (ADR-001: motor PDF, ...)
```

1. `pdf_core` **no depende de ninguna UI** (ni de egui). Toda la lógica vive aquí.
2. El motor PDF va detrás de un `trait RenderEngine`, con backends
   intercambiables por feature flags.
3. Las anotaciones son **vectoriales, en coordenadas de página**, dibujadas como
   capa sobre el bitmap cacheado. Nunca rasterizadas dentro del bitmap de página.
4. Render **a resolución de pantalla**, nunca a resolución máxima.
5. Caché LRU de páginas **limitado por bytes**, con prefetch de páginas
   colindantes en hilos de fondo. Prohibido mantener todas las páginas en memoria.
6. El hilo de UI **nunca** se bloquea renderizando (rayon / hilos de fondo).
7. Extracción de texto **perezosa**: solo cuando se necesita.

## 5. Stack vigente

- Rust estable (toolchain vía rustup), cargo workspace.
- Motor PDF: **MuPDF** (crate `mupdf` 0.8 + `mupdf-sys`, AGPL-3.0) detrás del
  `trait RenderEngine` en pdf_core — único backend desde ADR-001
  (el `pdfium-render` de Fase 0 fue eliminado).
- UI prototipo: **egui/eframe** (solo escritorio; no portar a Android).
- Persistencia: SQLite vía `rusqlite` (sidecar por PDF, pensado para Syncthing).
- Benchmarks: `criterion`. Concurrencia: `rayon`.
- Android: despliegue y métricas por `adb` (USB) — Fases 0.5, 1 y 6.
- Sync: Syncthing externo; la app no implementa red, solo formato de ficheros
  sync-friendly.
- Python 3.14 + uv disponibles para tooling auxiliar (generación de corpus, scripts).

## 6. Decisiones PENDIENTES — no asumir, preguntar

> Motor y licencia resueltos por ADR-001 (2026-08-05): **MuPDF, AGPL-3.0**.

| Decisión | Se resuelve en |
|----------|----------------|
| UI final Android: Slint vs Tauri | Fase 6 (spike de 1-2 días) |
| Ollama: PC por red local vs otra opción | Inicio de Fase 5 |
| Formato canónico de anotaciones | Fases 3-4 |

## 7. Comandos

```bash
cargo run -p pdf_app          # lanzar app de escritorio
cargo test -p pdf_core        # tests del núcleo
cargo bench -p pdf_bench      # benchmarks
cargo clippy --all-targets -- -D warnings
cargo fmt --all

# Android (a partir de Fase 1), tablet por USB:
adb install <apk>
adb shell dumpsys meminfo <paquete>
adb logcat
```

## 8. Presupuestos de rendimiento y definición de "hecho"

| Métrica | Objetivo |
|---------|----------|
| Frame time p95 en scroll | < 16,6 ms (60 fps sostenidos) |
| Render de página en tablet | < 25 ms (revisable con datos de Fase 1) |
| RSS en tablet, PDF 500 pág. | < 150 MB |
| 200 trazos de anotación visibles | sin degradar frame time |

Una tarea está **hecha** cuando:

1. Compila sin warnings (`clippy --all-targets -- -D warnings`).
2. `cargo fmt` limpio.
3. Tests de `pdf_core` pasan, con tests nuevos para la lógica nueva.
4. Si se tocó render/caché/zoom: benchmarks ejecutados, sin regresión, y el
   resultado anotado (en el commit, PR o doc de la fase).
5. Si la tarea tenía criterio de aceptación en PLAN.md, está verificado con datos.
6. El trabajo queda documentado según §12.

## 9. Testing

- `pdf_core`: tests unitarios **obligatorios** para toda lógica nueva (caché,
  anotaciones, persistencia, exportación, sync).
- `pdf_bench`: benchmark nuevo cada vez que se toca una ruta crítica de rendimiento.
- UI (egui): prueba manual; no invertir en tests de UI sobre el prototipo.

## 10. Flujo de trabajo por fases

El proyecto avanza por las fases de `PLAN.md` (Fase 0 → 6). No adelantes trabajo
de fases futuras salvo petición explícita del autor. Al cerrar una fase: verifica
su criterio de aceptación con mediciones y marca el hito como cumplido en `PLAN.md`.

## 11. Skills del proyecto

- **Skills PROPIOS del proyecto** → `.opencode/skills/<nombre>/SKILL.md`
  (**versionados**, parte del repo). Formato Agent Skills estándar: `SKILL.md`
  con `name` + `description`. Cárgalos cuando la tarea coincida con su
  descripción; si un procedimiento se repite dos veces (medir, desplegar a la
  tablet, exportar...), propón capturarlo como skill.
- **Skills del ecosistema** (anthropics, obra/superpowers…) se instalan con
  `npx skills add` en `.agents/skills/` y `.claude/skills/` (en disco),
  **gitignored**. En un clone limpio se reproducen desde `skills-lock.json`
  con las órdenes de PLAN.md §2.1.

## 12. Documentación obligatoria — nada queda solo en el chat

**Todo** lo que el agente haga debe quedar registrado en el repo, en el lugar que
corresponda. Si no está documentado, no está hecho.

| Qué se hizo | Dónde se documenta |
|-------------|--------------------|
| Decisión técnica (motor, licencia, formato, UI...) | ADR nuevo en `docs/adr/` + actualizar la tabla de decisiones de `docs/PLAN.md` |
| Fase o hito completado | Marcar en `docs/PLAN.md` con fecha y las mediciones de su criterio de aceptación |
| Medición de rendimiento (benchmark, RSS, frame time) | Anotada con fecha y hardware en el doc de la fase o ADR correspondiente |
| Procedimiento repetible que emerge | Skill nuevo en `.opencode/skills/` (regla §11) |
| Cambio de arquitectura o del plan | Actualizar `AGENTS.md` y/o `docs/PLAN.md` en el mismo cambio |
| Comando nuevo (build, deploy, medición) | §7 de este archivo + skill si aplica |
| Datos de prueba (corpus, fixtures) | `corpus/` (gitignored) + su generador en `tools/` |
| Registro cronológico de versiones y cambios del proyecto | `memory.md` (raíz del repo) |

Al terminar cada sesión de trabajo, deja el rastro: **qué se hizo, qué se decidió
y qué sigue**, en el documento que corresponda. Este es un proyecto de
aprendizaje: la documentación es parte del producto, no un extra.

## 13. Registro de versiones y cambios (`memory.md`)

`memory.md` (raíz del repo) es el registro cronológico del proyecto, **de más
antiguo a más nuevo**. Es un documento vivo, no un log del agente: registra
cambios y versiones reales del proyecto.

Reglas:

1. Al terminar una sesión o un cambio relevante, añade una entrada fechada:
   `## AAAA-MM-DD — Título`.
2. Cada entrada lista: qué cambió, versiones de herramientas/toolchain/crates
   implicadas y, si aplica, mediciones con su hardware (benchmark, RSS, frame time).
3. Las decisiones con impacto de arquitectura van además a su ADR en `docs/adr/`;
   `memory.md` referencia el ADR, no lo sustituye.
4. No reescribas entradas pasadas: añade una nueva. Si hay que corregir algo,
   edita la entrada correspondiente indicando la corrección.
