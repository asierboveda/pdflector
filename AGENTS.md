# AGENTS.md — PDFLector

> TL;DR para agentes. Fuente única de reglas. Detalle en `docs/PROYECTO.md` y `docs/PLAN.md`.
> Stack: Rust stable + MuPDF (AGPL-3.0) + egui (desktop) / Slint (Android futuro).

## MUST

1. **Fluidez > RAM > resto** — p95 <16.6ms (60fps), render <25ms en TCL, RSS <150MB PSS. Medir antes de afirmar.
2. **pdf_core sin UI** — nunca depende de egui/Slint. Lógica pura. Motor tras `trait RenderEngine`. Mutations vía feature flags.
3. **Caché LRU por bytes, prefetch ±1, hilo UI nunca bloquea** (rayon/worker). Anotaciones vectoriales en coords de página, capa sobre bitmap.
4. **Español en chat, inglés en código/commits.** Explica crates/conceptos Rust nuevos en 2-4 líneas.
5. **Cambios mínimos** — implementa solo lo pedido. Nada de refactors/features no solicitados.
6. **Documenta** — decisión → `docs/adr/`, medición → `docs/benchmark-results.md`, procedimiento repetible → `.opencode/skills/`.

## MUST NOT

- No `unwrap/expect` en `pdf_core` (usa `Result`). `unsafe` solo acotado y comentado.
- No dependencias GPL/AGPL sin aprobación. Preferir MIT/Apache-2.0. Justifica cada crate nueva (1 línea).
- No render a resolución máxima, no guardar todas las páginas en memoria.
- No cerrar fase/issue sin medición con fecha+hardware.

## Arquitectura

```
crates/pdf_core    # motor MuPDF, cache, prefetch, zoom, annotations, store, export, sync, ai
crates/pdf_app     # egui desktop (prototipo)
crates/pdf_bench   # criterion + sweep binario
crates/pdf_android # cdylib android-activity (fuera de default-members)
docs/adr/          # ADR-001 MuPDF/AGPL, ADR-004 Slint
```

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

- Fuente de verdad: **GitHub Issues + Milestones (Fase 1-6)**. `PLAN.md` es índice, no kanban. `CHANGELOG.md` = últimas 5 entradas.
- Una sola carpeta: todo en `~/Projects/pdflector/`. Excepciones: `~/Android/Sdk`, `/tmp`.
- Issues: un Issue = una tarea con criterio de aceptación medible. Se cierra con log/screenshot.
- Skills propios → `.opencode/skills/<nombre>/SKILL.md` (versionado). Ecosistema → `.agents/skills/` (gitignored, de `skills-lock.json`).
