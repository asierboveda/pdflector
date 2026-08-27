# Auditoría del contexto que leen los agentes — 2026-08-23

> Alcance: todos los ficheros del repo que un agente de IA lee como contexto o
> instrucciones: `AGENTS.md`, `docs/PROYECTO.md`, `docs/PLAN.md`, `docs/adr/*`,
> `memory.md`, `README.md`, `CONTRIBUTING.md`, `.opencode/skills/*`, CI y
> templates de PR. Objetivo: detectar contradicciones, desactualizaciones y
> huecos; proponer mejoras y un plan revisado.
> Estado del repo auditado: reconciliado contra `marakihau@1db57a6`
> (commit de la Fase 3.5) y `oyster` (rama de este agente).

## 1. Inventario de lo que lee un agente

| Fichero | Rol | Estado |
|---|---|---|
| `AGENTS.md` | Reglas de comportamiento + decisiones pendientes | **Desactualizado** (§6 contradice ADR-004; no existen las reglas multi-agente) |
| `docs/PROYECTO.md` | Visión, stack, hoja de ruta | **Desactualizado** (arquitectura sin `pdf_android`; UI "por decidir"; Ollama pendiente) |
| `docs/PLAN.md` | Plan por fases, criterios, riesgos | **Desactualizado** (último progreso marcado = 2026-08-13; faltan biblioteca premium 08-18 y Fase 3.5 08-22) |
| `docs/adr/ADR-001` (MuPDF) | Motor PDF (aceptado) | ✅ Coherente |
| `docs/adr/ADR-002` (Evince) | Patrones de caché/render (referencia) | ✅ Aceptado; nota histórica PDFium correcta |
| `docs/adr/ADR-003` (baseline) | Baseline Evince vs PDFium | ✅ Histórico |
| `docs/adr/ADR-004` (UI Slint) | **Decisión UI final: Slint, ACEPTADO 2026-08-13** | ⚠️ Aceptado pero **ignorado por el resto de la doc y por la práctica** (la UI real es `pdf_android`) |
| `memory.md` | Registro cronológico | ⚠️ Entradas desordenadas (2026-08-13 intercaladas entre 2026-08-18); referencia residual a "Lenovo Idea Tab" |
| `.opencode/skills/*` (2) | android-tablet-adb, pdflector-rendimiento | ✅ Bien hechos; falta `exportar-anotaciones` y `syncthing-sync` previstos en PLAN §2.1B |
| `README.md` / `CONTRIBUTING.md` | Puerta de entrada | ⚠️ README sin `pdf_android` ni `docs/adr` |
| CI + PR template | Verificación | ⚠️ `pdf_android` nunca se compila en CI; template sin checklist Android |

## 2. Hallazgos

### 2.1 CRÍTICO — Contradicciones que un agente encuentra al leer

1. **UI final**: `ADR-004` decidió **Slint** (aceptado, verificado en la tablet,
   ítem §5) el 2026-08-13. Pero `AGENTS.md §6` mantiene "UI final Android: Slint
   vs Tauri → Fase 6 (spike de 1-2 días)" como **pendiente**, y `PLAN.md` Fase 6
   y `PROYECTO.md` "Decisiones pendientes" dicen lo mismo. Un agente nuevo lee
   dos verdades excluyentes.
2. **Ollama**: `PLAN.md` Fase 5 (2026-08-13) ya tiene "Decisión: Ollama en el PC
   (localhost)" y el lado app completado; `AGENTS.md §6` y `PROYECTO.md` la
   listan como **pendiente**.
3. **`pdf_android` no existe en la arquitectura**: `AGENTS.md §4`,
   `PROYECTO.md` y `PLAN.md §3.1` describen solo 3 crates (`pdf_core`/`pdf_app`/
   `pdf_bench`). La **app real de la tablet** (`crates/pdf_android`, android-activity,
   biblioteca premium, visor, anotaciones: ~12 600 líneas) no aparece en la doc de
   arquitectura ni en el README. Solo `Cargo.toml` y `memory.md` la mencionan.
   Consecuencia práctica: un agente que lee AGENTS.md §4 cree que la UI Android
   aún no existe.

### 2.2 CRÍTICO — El plan no refleja el estado real del proyecto

- Última marca de progreso en `PLAN.md`: **2026-08-13**.
- Ausente: biblioteca premium (2026-08-18, memory.md), **Fase 3.5** de
  anotaciones (2026-08-22, rama `marakihau`, ya commiteada: selection.rs,
  overlay.rs, barra de herramientas Android, auditoría blit/prefetch con
  ganancias 31-89 %).
- La "Fase 3.5" no existe en el plan; el trabajo de anotaciones se registra a
  medias (PLAN Fase 3 cerrada "solo Stroke", sin Highlight/TextNote UI ni la
  Fase 3.5).
- Fases 4 y 5 marcadas "(lado app) completada", pero **sus criterios de
  aceptación reales quedan sin verificar**: sync completa (falta instalar
  Syncthing en tablet/PC/móvil) y Ollama real (nunca probado contra un modelo).
- `PLAN.md §3.3` (mapa de módulos de pdf_core) no incluye módulos que ya
  existen: `zoom`, `ai`, `overlay`, `selection`, ni los de pdf_android
  (`persist`, `library`).

### 2.3 ALTO — Sin reglas de coordinación multi-agente (causa raíz de la duplicación)

- El repo se trabaja con **un worktree/rama por agente** (devilray, marakihau,
  seahorse, oyster...) y `AGENTS.md` no documenta: qué rama es la fuente de
  verdad, cómo evitar solapes entre agentes, ni el protocolo de integración.
- **Evidencia del coste**: las ramas `marakihau` y `devilray` resolvieron **el
  mismo problema dos veces** — suavizado (Catmull-Rom vs Chaikin) y resaltado
  (gesture+selection.rs vs TextWord+word_boxes) — ambas en
  `pdf_core/src/annotations.rs` y `lib.rs`. El merge no es trivial: hay que
  elegir una implementación de suavizado y decidir si se rescata `TextWord`/
  `widths`. Ese trabajo duplicado no habría ocurrido con una regla simple
  ("antes de tocar X, consulta el/los worktrees activos y el diff de las ramas").

### 2.4 ALTO — La verificación de la app real no está cubierta

- `pdf_android` no compila en host (android-activity) ni **se compila en CI**
  (`ci.yml` solo corre los default-members de Cargo.toml: pdf_core/app/bench).
- La definición de "hecho" (`AGENTS.md §8`) y el PR template no incluyen
  paso Android (build aarch64, prueba manual en tablet, logcat limpio, dumpsys).
- El propio trabajo de la Fase 3.5 quedó con "prueba manual pendiente en la
  tablet" (doc `api-anotaciones-ui.md`) y **no hay mecanismo que la haga
  obligatoria antes de merge**. Riesgo: las rutas de la app real se rompen sin
  que nada lo detecte.

### 2.5 MEDIO — Desactualizaciones menores que restan confianza

- `AGENTS.md §7` no incluye los comandos Android reales (build con
  `BINDGEN_EXTRA_CLANG_ARGS`, `cargo apk build`, `dumpsys`, `screencap`); están
  bien documentados en el skill `pdflector-rendimiento` pero no hay puente
  AGENTS→skill.
- `memory.md`: entrada 2026-08-05 sigue citando "tablet Lenovo Idea Tab"
  cuando la propia corrección del 2026-08-12 (línea ~281) actualizó a
  "TCL NXTPaper 11 Plus"; y hay entradas 2026-08-13 intercaladas entre las del
  2026-08-18 (viola "de más antiguo a más nuevo", regla §13).
- `legal.md` (preparatorio, "cuando haya versión") afirma que LICENSE carece de
  cláusula "or later", pero LICENSE líneas ~579 lo contiene y README declara
  AGPL-3.0-or-later → contradicción.
- `README.md` layout sin `pdf_android` ni `docs/adr/`.
- `PLAN.md §2.1B`: skills `exportar-anotaciones` (Fase 3 ya cerrada) y
  `syncthing-sync` (Fase 4) sin crear; la regla AGENTS.md §11 dice proponer el
  skill cuando el procedimiento se repite (export ya se ha tocado 3 veces:
  Fase 3, dev, marakihau).

### 2.6 Ampliación honesta — el gap estratégico más importante

`pdf_android` (android-activity + canvas + JNI, UI custom) se ha convertido en
**el producto real**: biblioteca premium, visor de una hoja, zoom, anotaciones
(Fase 3.5), optimizaciones medidas. `ADR-004` eligió **Slint** cuando
`pdf_android` apenas era un harness de Fase 1. Hoy, "portar a Slint" significaría
reescribir ~12 000 líneas de UI probada en la tablet, contradiciendo las
prioridades 1-2 (fluidez/RAM: ya medidas y optimizadas en este stack) a cambio
de un framework declarativo. **No hay ningún ADR que documente esta bifurcación**
ni una decisión del autor sobre el destino de `pdf_android` vs Slint. Es la
decisión pendiente nº 1 de cara al plan revisado.

## 3. Mejoras propuestas (contexto para agentes)

1. **Actualizar `AGENTS.md`**:
   - §4: añadir `crates/pdf_android` (UI Android real, android-activity) a la
     arquitectura innegociable.
   - §5: registrar Slint como decisión ADR-004 aceptada **o** revisarla (ver
     decisión pendiente 1); listar deps reales.
   - §6: mover Slint y Ollama a "resueltas" o reformular la que siga abierta.
   - §7: añadir comandos Android (build aarch64 + apk, dumpsys, screencap) o
     referenciar el skill.
   - Nueva **regla de coordinación multi-agente**: antes de tocar un crate,
     revisar `git worktree list` + diffs sin commitear de las ramas activas;
     fuente de verdad = `main`; protocolo de integración (quién resuelve el
     merge de su rama).
   - §8 (definición de "hecho"): añadir paso Android (build aarch64 release,
     prueba manual en tablet con lista, logcat sin errores).
2. **Actualizar `PLAN.md`** al estado real (2026-08-23): Fases 0-2 cerradas;
   Fase 3 con módulos (y Fase 3.5); Fase 4/5 lado app cerrado con criterio
   real pendiente (Syncthing instalado / Ollama real); Fase 6 redefinida
   según la decisión de UI; `§3.3` con módulos reales (`zoom`, `ai`,
   `overlay`, `selection`, `persist`).
3. **Decidir y documentar (ADR-005 o revisión de ADR-004)**: destino de
   `pdf_android` vs Slint. Recomendación técnica de esta auditoría:
   consolidar `pdf_android` (producto real, medido, optimizado) y archivar el
   port a Slint salvo decisión explícita del autor; el ADR-004 queda como
   referencia histórica del spike.
4. **CI**: añadir job `aarch64-linux-android` (instalar NDK + `cargo check -p
   pdf_android --target aarch64-linux-android`) para que la app real se
   verifique en cada PR.
5. **PR template**: checklist Android (build, prueba manual, logcat, benchmarks
   si toca render).
6. **memory.md**: reordenar las entradas sueltas de 2026-08-13/18 sin reescribir
   contenido (regla §13) y corregir la referencia a Lenovo en la entrada de
   ítem con nota.
7. **legal.md**: marcar como resuelto/actualizado (LICENSE es AGPL-3.0-or-later
   y el proyecto ya está publicado) o borrar la afirmación contradictoria.
8. **Skills**: crear `exportar-anotaciones` (procedimiento repetido 3×) y
   `syncthing-sync` cuando se haga la instalación real de Fase 4.
9. **README**: añadir `pdf_android` y `docs/adr/` al layout.

## 4. Plan revisado propuesto (hoja de ruta)

| Prioridad | Trabajo | Verificación |
|---|---|---|
| 1 | Decidir UI: consolidar `pdf_android` vs portar a Slint (autor) → ADR-005 | ADR aceptado |
| 2 | Integrar ramas de agentes: merge `marakihau` (Fase 3.5) + resolver solape con `devilray` (elegir 1 suavizado; decidir rescatar TextWord/widths) | Tests + build android |
| 3 | Sincronizar contexto: AGENTS.md + PLAN.md + PROYECTO.md a estado 2026-08-23 | `git diff` docs |
| 4 | Prueba manual completa en tablet (barra de anotaciones, boli, undo, biblioteca sin parpadeo) + frame time p95 real | Planilla manual + logcat + overlay |
| 5 | CI Android (`cargo check aarch64`) + PR template | PR verde |
| 6 | Cerrar Fase 4/5 real: Syncthing instalado (criterio <1 min) y Ollama real probado | Criterios de PLAN |
| 7 | Fase 3 criterio pendiente: test de estrés 200 trazos en la tablet | Frame time p95 sin degradación |
| 8 | Fase 6 real: lápiz físico (ADR-004 §7.3), semana de uso diario | Uso real sin cuelgues |

## 5. Documento de decisión pendiente (para el autor)

1. **UI final**: ¿consolidar `pdf_android` (recomendado por esta auditoría) o
   portar a Slint según ADR-004?
2. ¿Confirmas mover Slint y Ollama de la tabla de "pendientes" de AGENTS.md §6
   a "resueltas con matiz" (Slint → revisar; Ollama → decidida en PLAN)?