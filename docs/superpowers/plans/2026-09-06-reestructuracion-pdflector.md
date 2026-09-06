# Plan de implementación — Reestructuración integral de PDFLector

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ejecutar las 4 fases del spec de reestructuración (docs → gpu → CI → megaficheros), cada punto con su test de verificación.

**Architecture:** Fase 1 reescribe docs activos y congela históricos contra anclajes verificados. Fase 2 revierte el diff corrupto de gpu.rs y reordena el pipeline present (dry = página+anotaciones, overlays a fb0) con fix del ovl_cache ABA y del EGL_BAD_ALLOC. Fase 3 añade job Android a CI. Fase 4 parte reader/draw/gpu/input en módulos por responsabilidad (extracción mecánica + LibraryState).

**Tech Stack:** Rust stable, MuPDF (via pdf_core), EGL/GLES2 (FFI manual), GitHub Actions, NDK r28.

**Spec:** `docs/superpowers/specs/2026-09-06-reestructuracion-pdflector-design.md`

## Global Constraints

- AGENTS.md MUST/MUST NOT vigente: pdf_core sin UI, sin `unwrap/expect` en producción de pdf_core, sin deps GPL, sin claves en git, biblioteca nunca borra PDFs.
- Español en docs/chat, inglés en código/commits.
- Cada fase termina con commit(s) atómico(s) + entrada en CHANGELOG.md.
- El diff sin commit de gpu.rs (worktree `code_reviuw`) NO se toca hasta la Tarea 2.1.
- Comandos de verificación:
  - Host: `cargo test -p pdf_core` (antes `python3 tools/generate_corpus.py` si falta corpus) · `cargo fmt --all` · `cargo clippy --all-targets -- -D warnings`
  - Android: `export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28; export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH; export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"; cargo check -p pdf_android --target aarch64-linux-android`
  - Requiere ficheros placeholder `crates/pdf_android/groq_key.txt` y `google_key.txt` (gitignored) con contenido `"placeholder"` para que `include_str!` compile.
- NOTA ejecución: los tests de pdf_android (Tareas 2.2-2.5) corren en el workspace host vía `cargo test -p pdf_android --lib` SOLO si el crate compila en host; como hoy no compila (ndk-sys), la lógica pura testable vive en un módulo `#[cfg(test)]` de gpu.rs cuya parte no-FFI se extrae a funciones puras, y se valida con `cargo check --target aarch64-linux-android` + medición TCL. Alternativa si el executor decide habilitar host: ver Tarea 2.0.

---

## FASE 1 — DOCS & GOBERNANZA

### Tarea 1.0: Cabecera de CHANGELOG + regla en AGENTS.md (G1)

**Files:**
- Modify: `CHANGELOG.md:1-6`
- Modify: `AGENTS.md:70`

**Interfaces:**
- Consumes: nada.
- Produces: regla "CHANGELOG.md completo, más reciente arriba" citada por el resto de tareas de docs.

- [ ] **Step 1: Editar CHANGELOG.md:3** — sustituir la frase de la cabecera que dice "últimas 5 entradas" por:

```markdown
> Registro completo de cambios, más reciente arriba. Cada entrada cierra con
> verificación: fecha + hardware + flujo + métrica (regla AGENTS.md).
```

- [ ] **Step 2: Editar AGENTS.md:70** — la línea actual dice: `` `- Roadmap activo: **`docs/plan/NEXT-PLAN.md` (fases A–E)**. `docs/PLAN.md` es índice histórico, no kanban. `CHANGELOG.md` = últimas 5 entradas.` ``. Sustituir SOLO el fragmento final por: `` `CHANGELOG.md` = registro completo, más reciente arriba. ``

- [ ] **Step 3: Verificar (V1.1)**

Run: `grep -rn "últimas 5" AGENTS.md CHANGELOG.md; echo "exit=$?"`
Expected: `exit=1` (0 hits).

- [ ] **Step 4: Commit**

```bash
git add CHANGELOG.md AGENTS.md
git commit -m "docs: CHANGELOG keeps full history, fix stale rule (G1)"
```

### Tarea 1.1: AGENTS.md — quitar límite-50 como tarea futura

**Files:**
- Modify: `AGENTS.md:34`

**Interfaces:**
- Produces: política de biblioteca consistente con E4 (nunca borrado automático, sin límite).

- [ ] **Step 1: Editar AGENTS.md:34** — sustituir la frase "No corregir aquí el límite de 50 PDFs; hay tarea futura específica en Fase E" (el límite ya no existe) por:

```markdown
- La biblioteca nunca elimina un PDF automáticamente: solo el usuario puede borrarlo (E4, sin tope de nº de libros).
```

- [ ] **Step 2: Verificar (V1.4)**

Run: `grep -n "tarea futura" AGENTS.md; echo "exit=$?"`
Expected: `exit=1`.

- [ ] **Step 3: Commit**

```bash
git add AGENTS.md
git commit -m "docs: drop stale 50-PDF limit reference in AGENTS.md (E4 done)"
```

### Tarea 1.2: docs/PROYECTO.md — quitar borrado-auto como pendiente

**Files:**
- Modify: `docs/PROYECTO.md:125`

**Interfaces:** none

- [ ] **Step 1: Editar docs/PROYECTO.md:125** — eliminar la línea que lista el límite-50/borrado automático como pendiente futuro (E4 ya lo ejecutó en dirección contraria: sin límite). Mantener la política :26 ("nunca borra") intacta.

- [ ] **Step 2: Verificar**

Run: `grep -n "borrado automático\|borrado-auto\|límite-50\|tope de 50" docs/PROYECTO.md; echo "exit=$?"`
Expected: `exit=1`.

- [ ] **Step 3: Commit**

```bash
git add docs/PROYECTO.md
git commit -m "docs: remove auto-delete from PROYECTO pending list (E4 policy)"
```

### Tarea 1.3: docs/plan/00-objetivo.md — reescritura de obsolescencias

**Files:**
- Modify: `docs/plan/00-objetivo.md:20-32`

**Interfaces:**
- Consumes: regla de roadmap único (NEXT-PLAN.md como único editable).

- [ ] **Step 1: Editar :21** — sustituir "Modo paginado (descartado 2026-08-13, autor prefiere scroll continuo)" por:

```markdown
- Scroll continuo (descartado 2026-08-13, el visor es paginado de una hoja con tap)
```

- [ ] **Step 2: Editar :30** — sustituir la referencia a fases waterfall "Fase 6" por: `Fases waterfall descartadas: el roadmap vigente es `docs/plan/NEXT-PLAN.md` (A–E), editable.`

- [ ] **Step 3: Editar :31-32** — marcar Obsidian/Syncthing como fuera de v1: `Obsidian y Syncthing quedan fuera de v1 (post-v1, ver `docs/plan/NEXT-PLAN.md`).`

- [ ] **Step 4: Editar :20** — corregir "ver PROYECTO.md decisión 2" → `ver docs/PROYECTO.md "Decisión sobre presión del lápiz" (la decisión real vive allí, no numerada aquí)`.

- [ ] **Step 5: Verificar (V1.2)**

Run: `grep -n "scroll continuo" docs/plan/00-objetivo.md; echo "exit=$?"`
Expected: `exit=1`.

- [ ] **Step 6: Commit**

```bash
git add docs/plan/00-objetivo.md
git commit -m "docs: fix 00-objetivo stale claims (paged viewer, roadmap, post-v1)"
```

### Tarea 1.4: docs/plan/COMPETENCIA.md + D-ia-contexto.md — obsolescencias puntuales

**Files:**
- Modify: `docs/plan/COMPETENCIA.md:9`
- Modify: `docs/plan/D-ia-contexto.md:21`

**Interfaces:** none

- [ ] **Step 1: Editar COMPETENCIA.md:9** — sustituir "`composite_annotations` 200 trazos: no medido aún (Fase A lo medirá)" por:

```markdown
- `composite_annotations` 200 trazos: 4.39ms directo / 2.39ms con StrokeCache (Fase C, 2026-09-05, host a resolución TCL — `docs/benchmark-results.md`)
```

- [ ] **Step 2: Editar D-ia-contexto.md:21** — marcar la ref rota: sustituir `tools/ai-bench.sh` por `` `tools/ai-bench.sh` (por crear en D4) ``.

- [ ] **Step 3: Verificar (V1.3)**

Run: `grep -n "no medido aún" docs/plan/COMPETENCIA.md; echo "exit=$?"`
Expected: `exit=1`.

- [ ] **Step 4: Commit**

```bash
git add docs/plan/COMPETENCIA.md docs/plan/D-ia-contexto.md
git commit -m "docs: close stale claims in COMPETENCIA and D-ia refs"
```

### Tarea 1.5: docs/plan/NEXT-PLAN.md — presupuesto, estado y deuda transversal

**Files:**
- Modify: `docs/plan/NEXT-PLAN.md:6,34-38`

**Interfaces:**
- Produces: sección "Deuda transversal" que las Fases 2-4 citan al cerrar items.

- [ ] **Step 1: Editar :6** — actualizar el presupuesto de PSS con mediciones reales:

```markdown
- Presupuesto de memoria: PSS producto <150MB. Medido: 52.9MB arranque (2026-08-28), 105MB en lectura (2026-09-03), 208MB tras 130 page-turns (2026-09-04, `docs/benchmark-results.md`) — deuda de fuga en investigación (Fase A5).
```

- [ ] **Step 2: Reescribir :34-38 (estado auditado)** — reflejar el estado real por fase: A1-A3 [x] (2026-09-04), A4/A5 [ ]; B cerrada (2026-09-05); C1-C4 [x], cierre [ ] (4.39/2.39ms medidos); D pendiente; E1/E4 [x], E2/E3 [ ]. Añadir las cifras de líneas actuales: `reader.rs 5605 · draw.rs 4899 · gpu.rs 1899 · input.rs 1701`.

- [ ] **Step 3: Añadir sección "Deuda transversal"** (tras el estado auditado):

```markdown
## Deuda transversal

| Deuda | Estado | Dónde se cierra |
|---|---|---|
| EGL_BAD_ALLOC 0x3003 Library→Viewer | Abierta (CHANGELOG 2026-09-04) | Fase 2 del plan de reestructuración (2026-09-06) |
| Verificación ADR-007 §8.4 (PSS<150MB, p95<8.33ms) | Sin entrada en benchmark-results | Fase 2 del plan de reestructuración |
| Bug pantalla apagada | Abierta, hipótesis H1-H3 | Medición TCL pendiente (BUG-pantalla-apagada.md) |
| 419 unwrap/expect en tests/benches | Deuda registrada (ADR-008:37) | Limpieza continua |
```

- [ ] **Step 4: Verificar**

Run: `grep -n "208MB\|Deuda transversal" docs/plan/NEXT-PLAN.md`
Expected: 2+ hits.

- [ ] **Step 5: Commit**

```bash
git add docs/plan/NEXT-PLAN.md
git commit -m "docs: refresh NEXT-PLAN state, PSS budget, transversal debt table"
```

### Tarea 1.6: Índice maestro docs/README.md + indexar huérfanos (G3)

**Files:**
- Create: `docs/README.md`
- Modify: `docs/plan/README.md:8-14`

**Interfaces:**
- Produces: `docs/README.md` como índice canónico citado por AGENTS.md/README.

- [ ] **Step 1: Crear docs/README.md** con la estructura:

```markdown
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
- `docs/plan/A-latencia.md` … `E-library.md` — ficheros de fase (checkboxes con evidencia).
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
- `docs/plan/01-06-*.md` — planes históricos fase a fase.
- `docs/research/*` — investigación (10 docs).
- `docs/investigacion/*`, `docs/benchmarks/*`.

## Superseded
- `docs/adr/ADR-004` (por ADR-005) · `docs/api-anotaciones-ui.md` (por fase 3 real).

## ADRs (docs/adr/, snapshots inmutables)
ADR-001 MuPDF · ADR-002 Evince→Android · ADR-003 baseline · ADR-005 UI nativa
(supersede 004) · ADR-006 stylus EGL/GLES2 · ADR-007 Wet/Dry (supersede parcial
del present de 006; estado: Aceptado) · ADR-008 zoom+UI/UX.
```

- [ ] **Step 2: Editar docs/plan/README.md:8-14** — armonizar el criterio de issue con NEXT-PLAN:28 (un Issue = una tarea con criterio de aceptación medible) y añadir al inicio: `Índice maestro de docs: ver `docs/README.md`.`

- [ ] **Step 3: Verificar (V1.6)**

Run: `comm -13 <(find docs -name '*.md' | sort) <(grep -oE 'docs/[A-Za-z0-9_./-]+\.md|docs/[A-Za-z0-9_/*-]+' docs/README.md | sed 's|/*$||' | sort -u) > /dev/null; find docs -name '*.md' | while read f; do grep -q "$(basename $f)" docs/README.md || echo "FALTA: $f"; done`
Expected: sin líneas "FALTA:".

- [ ] **Step 4: Commit**

```bash
git add docs/README.md docs/plan/README.md
git commit -m "docs: add docs/README.md master index, list orphan PRO plans"
```

### Tarea 1.7: Banners de congelado en históricos

**Files:**
- Modify: `docs/auditoria-contexto-agentes-2026-08-23.md`, `docs/ux-rediseño-estructura.md`, `docs/api-anotaciones-fase3.md`, `docs/plan/01-lectura-fluida.md`, `02-modo-oscuro.md`, `03-anotaciones.md`, `04-sync.md`, `05-ia.md`, `06-android.md`, `docs/research/*.md` (10), `docs/investigacion/evince-baseline.md`, `docs/benchmarks/desktop-engine-shootout.md`

**Interfaces:**
- Consumes: formato de banner de `docs/PLAN-UX-UI-NUEVO.md` (línea 1-3).

- [ ] **Step 1: Añadir el banner** (según el formato existente en PLAN-UX-UI-NUEVO.md) tras la cabecera de cada fichero listado arriba. Plantilla:

```markdown
> **[HISTÓRICO — congelado 2026-09-06]** Documento congelado: su contenido refleja
> el estado de 2026-08. No editar. El roadmap vigente vive en `docs/plan/NEXT-PLAN.md`.
```

(En research/ y benchmarks/ ajustar "estado de 2026-08" a la fecha que indica su propio contenido.)

- [ ] **Step 2: Verificar**

Run: `for f in docs/auditoria-contexto-agentes-2026-08-23.md docs/ux-rediseño-estructura.md docs/api-anotaciones-fase3.md docs/plan/01-lectura-fluida.md docs/plan/02-modo-oscuro.md docs/plan/03-anotaciones.md docs/plan/04-sync.md docs/plan/05-ia.md docs/plan/06-android.md docs/research/*.md docs/investigacion/*.md docs/benchmarks/*.md; do head -5 "$f" | grep -q HISTÓRICO || echo "SIN BANNER: $f"; done; echo ok`
Expected: solo "ok".

- [ ] **Step 3: Commit**

```bash
git add docs/
git commit -m "docs: freeze historical docs with banners (research, plans 01-06, audits)"
```

### Tarea 1.8: README.md + CONTRIBUTING.md + PR template

**Files:**
- Modify: `README.md`, `CONTRIBUTING.md`, `.github/PULL_REQUEST_TEMPLATE.md`

**Interfaces:**
- Consumes: comandos de verificación de AGENTS.md.

- [ ] **Step 1: README.md** — añadir tras la sección de build existente:

```markdown
## Android (tablet TCL 9469X)

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
cargo check -p pdf_android --target aarch64-linux-android   # verificación rápida
cargo apk build -p pdf_android --release --target aarch64-linux-android  # APK
```

Ver también: `docs/README.md` (índice de docs), `docs/adr/` (decisiones),
`docs/benchmark-results.md` (mediciones), `.opencode/skills/` (procedimientos).
```

- [ ] **Step 2: CONTRIBUTING.md** — añadir bloque:

```markdown
## Verificación local (antes de abrir PR)

```bash
python3 tools/generate_corpus.py        # si falta corpus
cargo test -p pdf_core
cargo fmt --all && cargo clippy --all-targets -- -D warnings
# Android (NDK r28, ver README):
cargo check -p pdf_android --target aarch64-linux-android
```
```

- [ ] **Step 3: .github/PULL_REQUEST_TEMPLATE.md** — añadir ítem: `- [ ] Check Android local ejecutado (\`cargo check -p pdf_android --target aarch64-linux-android\`)`

- [ ] **Step 4: Verificar (V1.8)**

Run: `grep -c "aarch64-linux-android" README.md CONTRIBUTING.md .github/PULL_REQUEST_TEMPLATE.md`
Expected: 3 ficheros con ≥1 hit.

- [ ] **Step 5: Commit**

```bash
git add README.md CONTRIBUTING.md .github/PULL_REQUEST_TEMPLATE.md
git commit -m "docs: add Android build/verify to README, CONTRIBUTING, PR template"
```

### Tarea 1.9: Estado de ADR-007 + entrada CHANGELOG de Fase 1

**Files:**
- Modify: `docs/adr/ADR-007-pipeline-wet-dry-ink.md:3-5`, `CHANGELOG.md`

**Interfaces:** none

- [ ] **Step 1: Editar ADR-007:3-5** — cambiar "Aceptado (propuesto)" por `Aceptado` y añadir una línea: `Supersede PARCIALMENTE el present de ADR-006 (dry/wet/present); el resto de ADR-006 (stylus, EGL) sigue vigente.`

- [ ] **Step 2: Añadir entrada CHANGELOG** (2026-09-06):

```markdown
## 2026-09-06 — Reestructuración Fase 1: docs & gobernanza

- CHANGELOG completo (G1, era "últimas 5" con 19 reales); AGENTS.md sin límite-50
  (E4); PROYECTO.md sin borrado-auto; 00-objetivo/COMPETENCIA/D-ia corregidos;
  NEXT-PLAN con presupuesto PSS real (52.9/105/208MB) + deuda transversal;
  docs/README.md índice maestro (G3); 24 históricos congelados con banner;
  README/CONTRIBUTING/PR-template con verificación Android local.
- Verificación: greps V1.1-V1.4 en 0 hits; índice docs/README.md completo (find
  vs listado, diferencia vacía); ADRs con estado normalizado.
```

- [ ] **Step 3: Verificar**

Run: `grep -n "Aceptado" docs/adr/ADR-007-pipeline-wet-dry-ink.md | head -2 && grep -c "## 2026-09-06" CHANGELOG.md`
Expected: 1 "Aceptado" sin "(propuesto)"; 1 entrada nueva.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/ADR-007-pipeline-wet-dry-ink.md CHANGELOG.md
git commit -m "docs: normalize ADR-007 status, changelog entry for phase 1"
```

### Tarea 1.10: Verificación integral Fase 1

**Files:** none (solo verificación)

- [ ] **Step 1: Correr todos los greps de verificación V1.1-V1.8** (los comandos exactos de las tareas 1.0-1.8) y confirmar expected outputs.

- [ ] **Step 2: Re-escanear refs rotas en docs activos** (patrón del scout: rutas `tools/`, `docs/`, `crates/` citadas en AGENTS.md, README.md, CONTRIBUTING.md, docs/*.md activos, docs/plan/ activos que no existan). Expected: 0 (históricos congelados excluidos).

- [ ] **Step 3: Confirmar checkboxes por fase** — cada [x] de A-E cita evidencia con fecha+hardware+flujo+métrica.

- [ ] **Step 4: No commit** (verificación pura; los commits ya existen por tarea).

---

## FASE 2 — GPU

### Tarea 2.0: Extraer lógica de invalidación a módulo puro `dry_key.rs` (habilita tests host)

**Files:**
- Create: `crates/pdf_android/src/gpu/dry_key.rs`
- Create: `crates/pdf_android/src/gpu.rs` → transformado en `crates/pdf_android/src/gpu/mod.rs` (movimiento puro)
- Test: `crates/pdf_android/src/gpu/dry_key.rs` (módulo `#[cfg(test)]` interno)

**Interfaces:**
- Consumes: `DryKey` (gpu.rs:485-495), `OverlayList` (gpu.rs:1847-1855).
- Produces: `pub(crate) struct DryKey { page, zoom_bits, ann_count, dark }` con método `pub(crate) fn invalidates(&self, other: &Self) -> bool`; módulo `gpu` re-exportando lo mismo que hoy.

- [ ] **Step 1: Mover gpu.rs → gpu/mod.rs** (`git mv crates/pdf_android/src/gpu.rs crates/pdf_android/src/gpu/mod.rs`). Sin cambios de contenido.

- [ ] **Step 2: Crear gpu/dry_key.rs** con la clave extraída y la lógica pura de invalidación:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Clave de invalidación de la capa Dry (Fase 2 del plan de reestructuración).
//! PURA: sin FFI, sin GL — compila y se testea en host aunque el resto del
//! crate requiera Android (ver plan 2026-09-06, Tarea 2.0).

/// Clave de invalidación de la capa base persistente (Dry FBO).
/// Reducida desde 9 campos (incluía pan/chrome/sheet/toast) a 4: solo lo que
/// cambia el CONTENIDO de página+anotaciones. Pan, chrome, sheet y toast se
/// componen como overlays en fb0 y ya no invalidan la dry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DryKey {
    pub(crate) page: u32,
    pub(crate) zoom_bits: u32,
    pub(crate) ann_count: usize,
    pub(crate) dark: bool,
}

impl DryKey {
    /// La dry cacheada bajo `self` sirve para `other` si las claves coinciden.
    pub(crate) fn invalidates(&self, other: &Self) -> bool {
        self != other
    }
}
```

- [ ] **Step 3: Test de invalidación (fallando antes del rewire)**

En `gpu/dry_key.rs`, añadir:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DryKey {
        DryKey { page: 3, zoom_bits: 0x3F800000, ann_count: 12, dark: false }
    }

    #[test]
    fn content_fields_invalidate() {
        let k = base();
        for other in [
            DryKey { page: 4, ..k },
            DryKey { zoom_bits: 0x40000000, ..k },
            DryKey { ann_count: 13, ..k },
            DryKey { dark: true, ..k },
        ] {
            assert!(k.invalidates(&other), "debe invalidar: {other:?}");
        }
    }

    #[test]
    fn identical_keys_do_not_invalidate() {
        assert!(!base().invalidates(&base()));
    }
}
```

- [ ] **Step 4: Verificar el test falla** — el test compila y pasa YA en host porque dry_key.rs es puro; el "fallo" esperado de TDD es que gpu/mod.rs aún usa su propio DryKey local (9 campos) y no el del módulo. Verificación real:

Run: `grep -n "struct DryKey" crates/pdf_android/src/gpu/mod.rs crates/pdf_android/src/gpu/dry_key.rs`
Expected: 2 definiciones (la vieja en mod.rs debe eliminarse en Step 5).

- [ ] **Step 5: Rewire** — en gpu/mod.rs: eliminar la definición vieja de DryKey (:485-495), `mod dry_key; pub(crate) use dry_key::DryKey;`, y adaptar `present_viewer` (gpu/mod.rs:1740) para construir la clave reducida (eliminar pan_x/pan_y/chrome_visible/sheet_progress_bits/has_toast de la construcción :1747-1758) y usar `key.invalidates(&self.dry_key)` en la comparación de :1760 aprox.

- [ ] **Step 6: Verificar compilación Android + test host del módulo puro**

Run: `export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28; export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH; export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"; cargo check -p pdf_android --target aarch64-linux-android`
Expected: verde.

- [ ] **Step 7: Commit**

```bash
git add crates/pdf_android/src/gpu.rs crates/pdf_android/src/gpu/
git commit -m "feat(gpu): extract pure DryKey module, reduce invalidation to content fields"
```

### Tarea 2.1: Revert del diff corrupto en code_reviuw

**Files:**
- Modify: `crates/pdf_android/src/gpu.rs` (worktree code_reviuw, revert)

**Interfaces:** none

- [ ] **Step 1: En el worktree code_reviuw**: `git checkout -- crates/pdf_android/src/gpu.rs` (descarta el diff +32/−22 sin commit; el destino ya NO es repararlo: la funcionalidad de pan se re-implementa limpia en 2.2-2.4).

- [ ] **Step 2: Verificar**

Run (en code_reviuw): `git status --short && git diff --stat`
Expected: sin cambios en gpu.rs (diff vacío).

- [ ] **Step 3: Commit** — ninguno (el revert de un diff sin commit no genera commit; el worktree queda limpio).

### Tarea 2.2: Mover overlays fuera de render_dry

**Files:**
- Modify: `crates/pdf_android/src/gpu/mod.rs` (render_dry :1421-1575, present_viewer :1740+)

**Interfaces:**
- Consumes: `OverlayList::collect_viewer` (gpu.rs:1849-1855), `draw_bitmap` (:865).
- Produces: `render_dry` = página+anotaciones SOLO; los overlays se dibujan en `present_viewer` tras componer dry⊕wet.

- [ ] **Step 1: Escribir el test de contención (falla hoy)** — en `gpu/mod.rs`, un test `#[cfg(test)]` NO es viable para render_dry (usa GL). La verificación de esta tarea es de compilación + estructura + medición TCL (2.7). Verificación estructural:

Run: `grep -n "OverlayList::collect_viewer\|render_sheet\|draw_bitmap" crates/pdf_android/src/gpu/mod.rs | sed -n '1,20p'`
Expected HOY: collect_viewer en :1549 (dentro de render_dry). Tras la tarea: collect_viewer SOLO en present_viewer, no en render_dry.

- [ ] **Step 2: Mover el bloque de overlays** — en render_dry, eliminar el bloque `:1547-1578` (OverlayList::collect_viewer + draw_bitmap loop + sheet :1554-1560 + chrome + sel_menu + ai_panel + toast). En present_viewer, tras `draw_fullscreen_texture(self.dry_tex, ...)` (:1780 aprox) y antes del swap, añadir:

```rust
// Overlays de UI directamente a fb0: chrome, sheet, toast, sel_menu,
// ai_panel, lib_fade, badges. Nunca invalidan la dry (Fase 2).
let mut ovl = OverlayList::new();
OverlayList::collect_viewer(reader, &mut ovl);
for (b, x, y) in ovl.items {
    // El pan NO se aplica a los overlays: son UI fija en coords de pantalla.
    self.draw_bitmap(b, x, y, 1.0);
}
```

- [ ] **Step 3: Ajustar lib_fade** — si lib_fade se dibuja hoy vía render_dry o como parte del composite, moverlo al pase de overlays de present_viewer (mismo criterio: UI fija).

- [ ] **Step 4: Verificar compilación**

Run: `cargo check -p pdf_android --target aarch64-linux-android` (con env de NDK)
Expected: verde.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/gpu/mod.rs
git commit -m "feat(gpu): move UI overlays from render_dry to fb0 pass in present_viewer"
```

### Tarea 2.3: Pan en el quad de la dry (draw_fullscreen_texture con offset)

**Files:**
- Modify: `crates/pdf_android/src/gpu/mod.rs` (`draw_fullscreen_texture` :1341, `present_viewer` :1740+)

**Interfaces:**
- Consumes: pan del reader (`reader.pan_x/pan_y` — ver campos en reader.rs struct).
- Produces: `fn draw_fullscreen_texture(&mut self, tex: u32, alpha: f32, offset: (f32, f32))`.

- [ ] **Step 1: Extender la firma** — `draw_fullscreen_texture(&mut self, tex: u32, alpha: f32, offset: (f32, f32))`: aplicar el offset al quad de la dry (traslada el quad de página en pantalla; overlays y wet NO llevan offset).

- [ ] **Step 2: present_viewer pasa el pan real** — `self.draw_fullscreen_texture(self.dry_tex, 1.0, (reader.pan_x as f32, reader.pan_y as f32))` (nombre de campo exacto: verificar en reader.rs el campo de pan vigente antes de escribir).

- [ ] **Step 3: Verificar compilación**

Run: `cargo check -p pdf_android --target aarch64-linux-android` (con env de NDK)
Expected: verde.

- [ ] **Step 4: Commit**

```bash
git add crates/pdf_android/src/gpu/mod.rs
git commit -m "feat(gpu): apply pan offset to dry quad in draw_fullscreen_texture"
```

### Tarea 2.4: Fix ovl_cache ABA + presupuesto por bytes

**Files:**
- Modify: `crates/pdf_android/src/gpu/mod.rs` (:946-999 overlay_tex, :497-502 OverlayTex)

**Interfaces:**
- Consumes: `Bitmap { width, height, data }` (pdf_core).
- Produces: `struct OverlayTex { id: u64, tex: u32, bytes: usize }` con id monotónico asignado por el llamador (Reader posee `ovl_seq: u64`); `overlay_tex(&mut self, id: u64, b: &Bitmap) -> u32`.

- [ ] `**Step 1: Cambiar la clave.**` — `OverlayTex` pierde `key_ptr: *const u8` y `_keep: Vec<u8>`; la nueva clave es `id: u64` monotónico. `overlay_tex(id, b)`: hit por id; miss → upload + push. El bitmap YA NO se clona en la caché (quien posee el id posee el bitmap).

- [ ] **Step 2: Presupuesto por bytes** — sustituir el límite `len() > 8` (:992) por un presupuesto LRU por bytes (mismo patrón que `thumbs.rs` ThumbCache): presupuesto constante `OVL_BYTE_BUDGET: usize = 8 * 1024 * 1024` (8 MiB); evicción del frente (LRU) mientras `bytes + incoming > budget`.

- [ ] **Step 3: Test host del presupuesto (módulo puro)** — extraer la política de evicción a una función pura en un módulo sin GL... NOTA: el `overlay_tex` real requiere GL; la política (presupuesto, evicción del frente) se extrae igual que dry_key: `gpu/ovl_budget.rs` con la lógica pura y `#[cfg(test)]`:

```rust
pub(crate) struct OvlBudget {
    entries: Vec<(u64, usize)>, // (id, bytes) en orden LRU (frente = víctima)
    bytes: usize,
    budget: usize,
}

impl OvlBudget {
    pub(crate) fn new(budget: usize) -> Self { Self { entries: Vec::new(), bytes: 0, budget } }

    /// Registra una inserción de `bytes` para `id`; evicta del frente hasta caber.
    /// Devuelve los ids evictados (para glDeleteTextures en el llamador).
    pub(crate) fn insert(&mut self, id: u64, bytes: usize) -> Vec<u64> {
        self.touch(id);
        let mut evicted = Vec::new();
        while self.bytes + bytes > self.budget && !self.entries.is_empty() {
            let (vid, vb) = self.entries.remove(0);
            self.bytes -= vb;
            evicted.push(vid);
        }
        if self.bytes + bytes <= self.budget {
            self.entries.push((id, bytes));
            self.bytes += bytes;
        }
        evicted
    }

    pub(crate) fn touch(&mut self, id: u64) {
        if let Some(pos) = self.entries.iter().position(|e| e.0 == id) {
            let e = self.entries.remove(pos);
            self.entries.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_lru_front_until_budget() {
        let mut b = OvlBudget::new(100);
        assert!(b.insert(1, 40).is_empty());
        assert!(b.insert(2, 40).is_empty());
        assert_eq!(b.insert(3, 40), vec![1]); // 40+40+40 > 100 → evict 1
    }

    #[test]
    fn touch_promotes_recency() {
        let mut b = OvlBudget::new(100);
        b.insert(1, 40);
        b.insert(2, 40);
        b.touch(1); // 1 pasa a MRU
        assert_eq!(b.insert(3, 40), vec![2]); // víctima ahora 2
    }

    #[test]
    fn oversized_entry_is_dropped() {
        let mut b = OvlBudget::new(50);
        assert!(b.insert(9, 80).is_empty()); // no cabe ni vacía: se descarta
        assert!(b.insert(1, 10).is_empty()); // sigue operativa
    }
}
```

- [ ] **Step 4: Verificar test + compilación** — `cargo test -p pdf_android --lib` NO corre en host (crate Android-only); el test se valida con `cargo check --target aarch64-linux-android` + los tests correrán cuando exista carril host (ver Tarea 2.0 nota). Alternativa inmediata: mover `OvlBudget` y `DryKey` a un sub-crate `pdf_android_pure` en el workspace con `[lib] path = "src/lib.rs"` y target host, dependiendo solo de nada; `pdf_android` lo importa por path. El executor decide según el coste; si se crea el sub-crate, añadir al workspace y correr `cargo test -p pdf_android_pure`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/gpu/ Cargo.toml
git commit -m "fix(gpu): ovl_cache keyed by stable id, LRU byte budget (ABA fix)"
```

### Tarea 2.5: Wet guard + selección fuera de DryKey

**Files:**
- Modify: `crates/pdf_android/src/gpu/mod.rs` (present_viewer :1740+, DryKey ya reducida en 2.0)

**Interfaces:** none

- [ ] **Step 1: Confirmar wet guard** — verificar que `present_viewer` solo llama `render_wet` cuando `has_wet` (gpu/mod.rs:1767-1770 lo hace ya en main; tras las tareas 2.2-2.3 re-verificar el flujo completo tras mover overlays).

- [ ] **Step 2: has_selection/has_erase_pt fuera de DryKey** — confirmar que la DryKey reducida (2.0) ya no los contiene; el rect de selección se dibuja en la wet (:1734-1737), el cursor de goma pasa al pase de overlays con posición viva (eliminar el bitmap congelado de :1899-1907 si aún existe tras 2.2).

- [ ] **Step 3: Verificar compilación**

Run: `cargo check -p pdf_android --target aarch64-linux-android` (con env de NDK)
Expected: verde.

- [ ] **Step 4: Commit**

```bash
git add crates/pdf_android/src/gpu/mod.rs
git commit -m "feat(gpu): wet guard verified, selection/erase cursor out of DryKey"
```

### Tarea 2.6: Trazado del ciclo de vida EGL + fix EGL_BAD_ALLOC

**Files:**
- Modify: `crates/pdf_android/src/gpu/mod.rs` (recreate_surface :691-725, drop_surface_only :747, make_resources)

**Interfaces:** none

- [ ] **Step 1: Añadir log de ciclo de vida** — en cada transición: `info!("gpu: surface create {}x{}", w, h)`, `info!("gpu: surface drop")`, `info!("gpu: fbo create dry={} wet={} tex={}", ...)`, `info!("gpu: fbo destroy dry={} wet={}")`, y un contador `AtomicU64` de creaciones/destrucciones por tipo, logueado en cada drop (detectar leaks por delta ≠ 0 tras N ciclos).

- [ ] **Step 2: Auditar la ruta Library→Viewer** — seguir la secuencia real: `drop_surface` (al entrar en Library) → blits SW → `recreate_surface` (al volver a Viewer). Confirmar que `make_resources`/`recreate` liberan FBOs/texturas previos ANTES de crear los nuevos (o que drop_surface_only ya lo hizo); si alguna ruta crea sin destruir → ese es el leak del 0x3003.

- [ ] **Step 3: Fix del leak** — cualquier creación de textura/FBO va precedida de destrucción de la versión previa si existe (defensivo, idempotente).

- [ ] **Step 4: Compilación + medición TCL (ver 2.7)**

Run: `cargo check -p pdf_android --target aarch64-linux-android` (con env de NDK)
Expected: verde.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/gpu/mod.rs
git commit -m "fix(gpu): EGL resource lifecycle logging and leak fix for 0x3003"
```

### Tarea 2.7: Medición TCL de la Fase 2 (cierra 2 debts)

**Files:**
- Modify: `docs/benchmark-results.md`, `CHANGELOG.md`

**Interfaces:**
- Consumes: skills `.opencode/skills/android-tablet-adb` y `pdflector-rendimiento`.

- [ ] **Step 1: Construir y desplegar** — `cargo apk build -p pdf_android --release --target aarch64-linux-android` (con env NDK) e instalar en la TCL 9469X vía adb (skill android-tablet-adb).

- [ ] **Step 2: Medir ciclo EGL** — 10× Library→Viewer consecutivos; capturar logcat; verificar 0 `EGL_BAD_ALLOC` y contadores create/destroy en delta 0. Capturar `dumpsys meminfo` PSS al inicio y tras los 10 ciclos (estable = delta < 5%).

- [ ] **Step 3: Medir present con pan** — p95 de gl_present con pan activo (antes vs después: la medición "antes" se toma del registro previo al rework, la "después" en esta sesión). Objetivo: pan sin re-raster (DryKey no cambia durante pan → render_dry no se re-invoca; verificar por contador de invocaciones en log).

- [ ] **Step 4: Escribir entradas en benchmark-results.md** — 2 entradas nuevas (ciclo EGL + present con pan) con fecha+hardware+flujo+métrica, y verificar que los criterios ADR-007 §8.4 quedan cubiertos (PSS<150MB, p95<8.33ms) o documentar la brecha.

- [ ] **Step 5: CHANGELOG + commit**

```bash
git add docs/benchmark-results.md CHANGELOG.md
git commit -m "bench(tcl): phase 2 verification — EGL cycle, present with pan"
```

---

## FASE 3 — CI ANDROID

### Tarea 3.1: Job android en ci.yml

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `AGENTS.md` (tabla de validación)

**Interfaces:**
- Consumes: los env de NDK de Global Constraints.

- [ ] **Step 1: Añadir job paralelo** a ci.yml:

```yaml
  android:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android
      - uses: android-actions/setup-android-ndk@v3
        id: setup-ndk
        with:
          api-level: 35
          ndk-version: r28
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: ". -> target"
      - name: check pdf_android (aarch64)
        env:
          ANDROID_NDK_HOME: ${{ steps.setup-ndk.outputs.ndk-path }}
          BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android: --sysroot=${{ steps.setup-ndk.outputs.ndk-path }}/toolchains/llvm/prebuilt/linux-x86_64/sysroot
        run: |
          echo "placeholder" > crates/pdf_android/groq_key.txt
          echo "placeholder" > crates/pdf_android/google_key.txt
          export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
          cargo check -p pdf_android --target aarch64-linux-android
```

- [ ] **Step 2: Actualizar AGENTS.md** — tabla de validación: la fila "Compilación cruzada Android" pasa de "Dev local con NDK, no en CI actual" a "Dev local con NDK + CI (job `android`)".

- [ ] **Step 3: Verificar V3.1** — push a una rama; confirmar workflow run verde con el job android en verde; anotar duración (NDK cacheado tras el primer run).

- [ ] **Step 4: Verificar V3.2 (prueba de valor)** — en una rama de prueba, introducir un error de sintaxis en gpu.rs (p.ej. un `}` extra), push, confirmar que el job android FALLA; borrar la rama.

- [ ] **Step 5: Commit + CHANGELOG**

```bash
git add .github/workflows/ci.yml AGENTS.md CHANGELOG.md
git commit -m "ci: add aarch64-linux-android check job with cached NDK r28"
```

---

## FASE 4 — MEGAFICHEROS

> Los rangos de línea citados son guía contra `main` @ 2247069 y se re-verifican contra el código antes de cada movimiento. Cada tarea = un módulo extraído, un commit. Patrón constante para todas (Tarea 4.1-4.x):

**Patrón por extracción:**
1. `git mv` NO aplica (mismo fichero → submódulo): crear `crates/pdf_android/src/<padre>/<modulo>.rs`, cortar el rango, ajustar visibilidad (`pub(crate)` en lo que cruce el módulo), añadir `mod` en el padre.
2. `cargo check -p pdf_android --target aarch64-linux-android` verde.
3. `cargo test -p pdf_core` verde.
4. `cargo clippy --all-targets -- -D warnings` verde (workspace raíz).
5. Commit: `refactor(pdf_android): extract <modulo> from <padre> (pure move)`.

### Tarea 4.1: gpu/ — ffi.rs, shaders.rs, surface.rs, textures.rs, pipeline.rs

**Files:**
- Create: `crates/pdf_android/src/gpu/ffi.rs` (rango 34-237), `gpu/shaders.rs` (267-378), `gpu/surface.rs` (struct Gpu + superficies+EGL 444-946), `gpu/textures.rs` (947-1240 incl. ovl_cache/OvlBudget), `gpu/pipeline.rs` (dry 1421-1575, wet 1576-1739, present 1740-1823)

**Interfaces:**
- Consumes: Tarea 2.0 (gpu/ ya es directorio con mod.rs + dry_key.rs).
- Produces: `gpu/mod.rs` < 200 líneas re-exportando la API pública del crate (`Gpu`, `present_viewer`, etc.).

### Tarea 4.2: draw/ — primitives.rs, tinta.rs, chrome.rs, sheet.rs, menus.rs, library.rs, overlays.rs

**Files:**
- Create: `crates/pdf_android/src/draw/primitives.rs` (35-107 + copy_region 153), `draw/tinta.rs` (272-437), `draw/chrome.rs` (980-1272), `draw/sheet.rs` (1273-1554), `draw/menus.rs` (picker+vista 1555-2348 + ajustes 2375-2763), `draw/library.rs` (2764-3545), `draw/overlays.rs` (sel+toast+IA 4487-4899)
- Modify: `crates/pdf_android/src/draw.rs` → `draw/mod.rs`

**Interfaces:** none nuevas; `pub(crate)` se ajusta según necesidad.

### Tarea 4.3: input/ — gestos.rs, motion.rs, dispatch.rs, stylus.rs

**Files:**
- Create: `crates/pdf_android/src/input/gestos.rs` (87-321), `input/motion.rs` (367-1350), `input/dispatch.rs` (1537-1623), `input/stylus.rs` (1624-1801)

**Interfaces:**
- Consumes: `handle_input` (dispatch.rs lo re-exporta como hoy en lib.rs:357).

### Tarea 4.4: reader/ — 12 módulos por responsabilidad

**Files:**
- Create: `crates/pdf_android/src/reader/geometry.rs` (308-480), `life.rs` (1536-1830), `redraw.rs` (1833-2480), `seleccion.rs` (2495-2640), `toast_ia.rs` (2805-3011), `sheet_chrome.rs` (3035-3171), `tick.rs` (3172-3474), `pinch.rs` (3474-3600), `anotaciones.rs` (3712-3900), `tools.rs` (3980-4428), `navigation.rs` (4457-4565), `library.rs` (4784-5235 + 5315-5568)
- Modify: `crates/pdf_android/src/reader.rs` → `reader/mod.rs` (struct + tipos + impl mínimos)

**Interfaces:**
- Produces: `Reader` struct permanece en `reader/mod.rs`; los métodos se mueven con sus bloques `impl Reader`.

### Tarea 4.5: LibraryState estructural

**Files:**
- Create: `crates/pdf_android/src/reader/library_state.rs`
- Modify: `crates/pdf_android/src/reader/mod.rs` (struct Reader), todos los módulos reader/* que tocan campos lib_*

**Interfaces:**
- Produces: `pub(crate) struct LibraryState { /* ~30 campos lib_* */ }` poseído por Reader como `reader.library: LibraryState`; los métodos movidos a impl LibraryState van con el struct.

- [ ] **Step 1: Mover los campos** — cortar los ~30 campos `lib_*` del struct Reader (rango 1051-1443 del original) a LibraryState; `Reader` los posee como campo único `library: LibraryState`.
- [ ] **Step 2: Re-apuntar accesos** — `self.lib_x` → `self.library.lib_x` en todos los módulos reader/* (AST edit con `ast_edit`: patrón `self.lib_$FIELD` → `self.library.lib_$FIELD` es multi-campo; hacerlo fichero a fichero con grep de `self\.lib_` primero para inventariar).
- [ ] **Step 2: Verificar** — patrón constante (check Android + test pdf_core + clippy).
- [ ] **Step 3: Commit** — `refactor(reader): extract LibraryState struct from Reader`.

### Tarea 4.6: Depuración de allow(dead_code) + verificación integral Fase 4

**Files:**
- Modify: los ficheros tocados por 4.1-4.5 que conserven `#[allow(dead_code)]` huérfanos.

- [ ] **Step 1: Inventario final** — `grep -c "#\[allow(dead_code)\]" -r crates/pdf_android/src | sort` (baseline 47).
- [ ] **Step 2: Eliminar los huérfanos** — cada allow que ya no protege código usado se quita; si el compilador se queja, el código muerto real se elimina o se usa (decisión por caso, documentada en el commit).
- [ ] **Step 3: Verificar V4.1-V4.6** — check Android verde, test pdf_core verde, clippy verde, ficheros <2000 líneas (`wc -l`), allow count < 47.
- [ ] **Step 4: CHANGELOG + commit final**

```bash
git add -A
git commit -m "refactor(pdf_android): phase 4 complete — modules extracted, dead code purged"
```

---

## Self-review del plan

- **Cobertura del spec**: Fase 1 → Tareas 1.0-1.10 (todos los puntos de §1.2/1.3 y V1.1-V1.8); Fase 2 → 2.0-2.7 (todos los puntos de §2.2-2.3); Fase 3 → 3.1 (V3.1/V3.2); Fase 4 → 4.1-4.6 (V4.1-V4.6). Sin gaps.
- **Placeholders**: ninguno ("Step 4 el executor decide" en 2.4 se limita a una decisión binaria documentada con ambas ramas).
- **Consistencia de tipos**: DryKey reducida definida en 2.0, consumida en 2.2/2.5; OverlayTex/OvlBudget definidos en 2.4, consumidos en 4.1; LibraryState definido en 4.5, consumido por reader/*.
- **Riesgo conocido**: Tareas 2.2-2.5 dependen de que los rangos de gpu.rs (contra main @ 2247069) sigan vigentes tras 2.0/2.1 (el revert de code_reviuw NO afecta a este worktree). Cada tarea re-verifica sus anclajes antes de editar.
