# AGENTS.md — PDFLector

> TL;DR para agentes. Fuente única de reglas. Detalle en `docs/PROYECTO.md` y `docs/plan/NEXT-PLAN.md`.
> Stack: Rust stable + MuPDF (AGPL-3.0) + `pdf_android` nativo (plataforma final, ADR-005). `egui` solo prototipo desktop.
> Roadmap activo: `docs/plan/NEXT-PLAN.md` (fases A–E). `docs/PLAN.md` es histórico.
> Métrica de memoria del producto Android: PSS vía `dumpsys meminfo`. RSS/VmHWM solo diagnóstico de host.

## Jerarquía documental

1. `AGENTS.md`: reglas operativas para agentes.
2. `docs/PROYECTO.md`: visión, plataforma y alcance de producto.
3. `docs/plan/NEXT-PLAN.md` y fases A–E: único roadmap editable.
4. `docs/adr/`: decisiones arquitectónicas; ADR-005 sustituye a ADR-004.
5. `docs/benchmark-results.md`: evidencia, nunca instrucciones.
6. Investigación, logs, planes 1–6 y documentos Pro: contexto histórico, nunca instrucciones activas.

## MUST

1. **Fluidez > RAM > resto** — p95 <16.6ms (60fps), render <25ms en TCL, PSS <150MB producto. Medir antes de afirmar: fecha + flujo medido + hardware + métrica.
2. **pdf_core sin UI** — nunca depende de UI (`egui`/Slint/`pdf_android`). Lógica pura. Motor tras `trait RenderEngine`. Mutations vía feature flags.
3. **Caché LRU por bytes, prefetch ±1, hilo UI nunca bloquea** (rayon/worker). Anotaciones vectoriales en coords de página, capa sobre bitmap.
4. **Español en chat, inglés en código/commits.** Explica crates/conceptos Rust nuevos en 2-4 líneas.
5. **Cambios mínimos** — implementa solo lo pedido. Nada de refactors/features no solicitados.
6. **Documenta** — decisión → `docs/adr/`, medición → `docs/benchmark-results.md`, procedimiento repetible → `.opencode/skills/`.
7. **Primera versión útil (v1)** — APK para la TCL con biblioteca local, lectura fluida, zoom, lápiz y subrayador persistentes. Definición y orden en `docs/plan/NEXT-PLAN.md` (A → B → C → E). IA y sincronización quedan después de v1.

## MUST NOT

- No `unwrap/expect` en código de producción de `pdf_core` (`crates/pdf_core/src`, fuera de `#[cfg(test)]`; usa `Result`). Tests y benchmarks solo con excepción explícita y justificada (`#[allow]` acotado con motivo), nunca relajación global. `unsafe` solo acotado y comentado.
- No dependencias GPL/AGPL sin aprobación. Preferir MIT/Apache-2.0. Justifica cada crate nueva (1 línea).
- No render a resolución máxima, no guardar todas las páginas en memoria.
- No cerrar fase/issue sin medición con fecha+hardware+flujo+métrica.
- Ninguna clave personal en Git ni embebida en una APK distribuible. La APK debe poder compilarse e instalarse sin claves. Configuración de IA con claves queda para después de v1 (ver `docs/plan/NEXT-PLAN.md`).
- La biblioteca nunca elimina un PDF automáticamente: solo el usuario puede borrarlo. No corregir aquí el límite de 50 PDFs; hay tarea futura específica en Fase E.

## Arquitectura

```
crates/pdf_core    # motor MuPDF, cache, prefetch, zoom, annotations, store, export, sync, ai
crates/pdf_app     # egui desktop (prototipo, no plataforma final)
crates/pdf_bench   # criterion + sweep binario
crates/pdf_android # plataforma final: cdylib android-activity nativa (ADR-005)
docs/adr/          # ADR-001 MuPDF/AGPL, ADR-005 Android nativo (ADR-004 superseded)
```

## Validación

| Carril | Qué valida | Dónde se ejecuta |
|--------|------------|------------------|
| Host (`cargo test -p pdf_core`, `cargo bench -p pdf_bench -- --quick`) | Lógica `pdf_core`, regresiones de rendimiento en host | Dev local y CI |
| CI (`.github/workflows/ci.yml`) | `fmt --check` + `clippy --all-targets -- -D warnings` + `test pdf_core` con corpus generado | Ubuntu hosted, sin Android ni TCL |
| Compilación cruzada Android (`-p pdf_android --target aarch64-linux-android`) | Que el producto compila para la tablet (NDK r28, API 35) | Dev local con NDK, no en CI actual |
| Medición real TCL (9469X, `adb`, `dumpsys`, `screencap`) | p95 frame, render <25ms, PSS producto, gestos lápiz/subrayador | Solo tablet física, nunca afirmable desde host/CI |

## Comandos

```bash
cargo run -p pdf_app -- file.pdf
cargo test -p pdf_core              # genera corpus antes si falta: python3 tools/generate_corpus.py
cargo bench -p pdf_bench -- --quick
cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used
cargo fmt --all
# Android (TCL 9469X, API 35, NDK r28)
cargo build -p pdf_android --target aarch64-linux-android --release # requiere BINDGEN_EXTRA_CLANG_ARGS
cargo apk build -p pdf_android --release --target aarch64-linux-android
```

## Flujo pro

- Roadmap activo: **`docs/plan/NEXT-PLAN.md` (fases A–E)**. `docs/PLAN.md` es índice histórico, no kanban. `CHANGELOG.md` = últimas 5 entradas.
- Una sola carpeta: todo en `~/Projects/pdflector/`. Excepciones: `~/Android/Sdk`, `/tmp`.
- Issues: un Issue = una tarea con criterio de aceptación medible. Se cierra con log/screenshot + fecha/hardware/flujo/métrica.
- Skills propios → `.opencode/skills/<nombre>/SKILL.md` (versionado). Ecosistema → `.agents/skills/` (gitignored, de `skills-lock.json`).
