# Memory — PDFLector

> Registro cronológico de versiones y cambios del proyecto (de más antiguo a más nuevo).
> Documento vivo: cada sesión o cambio relevante añade una entrada fechada.
> Formato de entrada: `AAAA-MM-DD — Título` + lista de ítems (qué cambió, versiones implicadas, mediciones).

## 2026-08-05 — Inicio del proyecto y Primer paso (§2 de PLAN.md)

- Creados `AGENTS.md` (reglas del agente) y `docs/` (vault de Obsidian: `PROYECTO.md`, `PLAN.md`).
- **Skills del agente** instalados en `.agents/skills/` (32 skills, frontmatter válido):
  - `anthropics/skills` (18): incluye `pdf` y `skill-creator`.
  - `obra/superpowers` (14): incluye `test-driven-development`, `systematic-debugging`, `verification-before-completion`.
  - Generado `skills-lock.json`.
- **Toolchain Rust vía rustup** (antes: Rust 1.97.1 de pacman, sin rustup):
  - rustup 1.29.0, toolchain `stable-x86_64-unknown-linux-gnu` (rustc 1.97.1).
  - Target Android añadido: `aarch64-linux-android`.
  - `~/.bashrc` carga `$HOME/.cargo/env` (persistente en shells nuevos).
- **Herramientas de sistema (pacman, sudo)**: `android-tools` (adb 1.0.41) y `jdk17-openjdk` (17.0.19).
- **Android SDK en `~/Android/Sdk`** (sin sdkmanager-managed platform-tools; adb vía pacman):
  - `cmdline-tools` latest (build 11076708), licencias aceptadas.
  - NDK `r28` (extraído en `ndk/android-ndk-r28`).
  - Platform `android-35`, build-tools `35.0.0`.
  - `~/.profile` exporta `ANDROID_HOME`, `ANDROID_NDK_HOME` y PATH del cmdline-tools.
- **Corpus de pruebas** en `corpus/` (dentro del repo, gitignored; 4 PDFs, validados con pypdf):
  - `dense_textbook.pdf` — 93 páginas de texto denso.
  - `scanned_pages.pdf` — 30 páginas a imagen (simula PDF escaneado).
  - `scientific_paper.pdf` — 12 páginas con gráficos vectoriales.
  - `large_document.pdf` — 500 páginas (test de RAM/FPS).
  - Generado con uv + reportlab + pillow (script en `tools/generate_corpus.py`).
- **`PLAN.md` actualizado**: §2.2 y §2.3 marcados según estado real.
- **Pendiente**: tablet Lenovo Idea Tab (opciones de desarrollador + depuración USB) y `adb devices` → verificación 2.3; reiniciar opencode para que el agente cargue los skills del proyecto.

## 2026-08-05 — Workspace compilando: pdfium.rs alineado con pdfium-render 0.8.37

- **`crates/pdf_core/Cargo.toml`**: `pdfium-render` ahora con features `["thread_safe", "sync"]`.
  - Nota técnica: `thread_safe` (ya en `default` de la crate) solo envuelve `FPDF_InitLibrary` en un mutex global; los `unsafe impl Send/Sync for Pdfium` están gated tras la feature `sync`, que es la necesaria para un `static OnceLock<Pdfium>`.
- **`crates/pdf_core/src/engine/pdfium.rs`**: corregida la API de pdfium-render 0.8.37:
  - Campo interno `PdfiumDocument<'static>` → `PdfDocument<'static>` (tipo real de la crate).
  - `page.width()/height()` devuelven `PdfPoints` → se usa `.value` en `page_size()` y `render_page()`.
  - `bitmap.as_bytes().to_vec()` (deprecado) → `bitmap.as_raw_bytes()` (ya devuelve `Vec<u8>`).
  - `render(width, height, None)` acepta i32 directo; `bitmap.width()/height()` → `as u32`.
- **Dos problemas de concurrencia encontrados y resueltos** (los 4 tests colgaban o segfault en paralelo, pasaban con `--test-threads=1`):
  1. Deadlock en init: con el patrón `if PDFIUM.get().is_none() { bind; set }`, varios hilos lanzaban `FPDF_InitLibrary()` a la vez; esa llamada toma un mutex global de la crate **para siempre**, así que los perdedores se bloqueaban en futex. Se reemplazó por `OnceLock::get_or_init` con el `Result` dentro del propio static (inicialización atómica de un solo hilo; `get_or_try_init` no está estabilizado en Rust 1.97).
  2. SIGSEGV/SIGABRT en render concurrente: la feature `thread_safe` no serializa las llamadas nativas de PDFium (solo el init). Se añadió `static PDFIUM_LOCK: Mutex<()>` que protege `open()`, `page_count()`, `page_size()` y `render_page()` (helpers internos `*_unlocked` para evitar reentrancia).
- **`crates/pdf_core/src/engine.rs`**: `#[derive(Debug)]` en `Bitmap` (lo exige `unwrap_err()` del test `out_of_range_page_is_an_error`).
- **Verificación Fase 0** (toolchain rustup 1.97.1):
  - `cargo build --workspace` → OK (pdf_core + pdf_app/eframe + pdf_bench).
  - `cargo test -p pdf_core` → 4/4 OK (simple.pdf vía vendor/pdfium/lib/libpdfium.so), estable en paralelo (3 ejecuciones).
  - `cargo clippy --all-targets -- -D warnings` → limpio.
  - `cargo fmt --all` → aplicado, `--check` limpio.

## 2026-08-05 — Fase 0: lanzamiento de pdf_app

- **Build release**: `cargo build -p pdf_app --release` → OK en 57.12 s (caché del workspace; `Finished release profile`).
  - Binario generado: `target/release/pdf_app` (21.702.360 bytes), confirmado con `OK_BINARIO`.
- **Lanzamiento de la app**: `./target/release/pdf_app corpus/scientific_paper.pdf` en background (PID vía fichero, sesión `setsid`, stdin `/dev/null`, stdout+stderr a `/tmp/pdflector_app.log`), con disparador que la cierra a los 6 s.
  - Verificación de liveness con timestamps: **t+2s proceso VIVO → ventana abierta y renderizando**; **t+7s proceso terminado por el disparador (6 s)** → corrió los 6 s completos sin colgarse ni cerrarse sola.
  - `/tmp/pdflector_app.log` = **0 bytes / limpio**: `grep -iE 'bind|libpdfium|panic|error'` sin coincidencias → sin error de bind de libpdfium en runtime.
- **Cierre limpio**: `pgrep -x pdf_app` → no hay proceso colgado tras la prueba.
- **Verificación independiente de render** (sin depender de ver la ventana): `cargo test -p pdf_core -- --nocapture` → **4/4 OK** en 0.16 s, incluyendo `renders_page_1_to_rgba_bitmap_of_expected_dimensions` y `rendered_page_is_not_blank` (abrir + renderizar página 1 a bitmap).
- **Conclusión Fase 0**: criterio de aceptación "abrir PDF y mostrar página 1" verificado → **ABRIR_PDF_Y_MOSTRAR_PAGINA_1 = OK**. (Velocidad/scroll de página queda para Fase 1.)
- Nota operativa: `pkill -f 'target/release/pdf_app'` hace match también con el shell que lanza el comando (la cadena va en su argv); usar el PID guardado en fichero o `pgrep -x pdf_app` para el disparador/verificación.

## 2026-08-05 — Repo: skills del ecosistema fuera de git (corrección del init)

- El commit inicial `2170cc8` incluía 488 archivos de skills del ecosistema (`.agents/skills/` + `.claude/skills/` symlinks, ~11,7 MB) además del código. Patrón incorrecto: lo instalado vía lockfile se excluye de git (análogo a `node_modules` + `package-lock.json`).
- **Decisión**: skills PROPIOS del proyecto → `.opencode/skills/` (versionados); skills del ecosistema → instalados con `npx skills add` en `.agents/skills/` y `.claude/skills/` (en disco, **gitignored**), reproducibles desde `skills-lock.json` (órdenes en PLAN §2.1).
- `.gitignore` actualizado con `/.agents/skills/` y `/.claude/skills/`.
- `git rm -r --cached .agents/skills .claude/skills` (los archivos QUEDAN en disco; el entorno del agente no cambia).
- Documentos alineados: AGENTS.md §11 y §12, PLAN.md §2.1B.
- Commit raíz reescrito con `--amend --no-edit` (sin remoto, histórico único): ahora solo contiene el proyecto. Hash `2170cc8` descartado.
- Verificación: `git ls-files | wc -l` pequeño; `git ls-files | grep -E '^\.(agents|claude)/'` vacío; `.agents/skills/` en disco intacto (~12 MB); `git status` limpio.

## 2026-08-05 — Scaffolding estándar de repo + subida a GitHub

- Añadido scaffolding estándar: `.editorconfig`, `.gitattributes`, `.github/dependabot.yml` (cargo + github-actions semanal), `.github/PULL_REQUEST_TEMPLATE.md`, `.github/ISSUE_TEMPLATE/bug_report.md` + `feature_request.md`, `CONTRIBUTING.md` (apunta a `AGENTS.md`).
- **Licencia SIN decidir**: no se añade `LICENSE` (pendiente ADR-001 tras la Fase 0.5; README ya lo indica). Estado provisional: "All rights reserved" implícito.
- Commit local: "Añade scaffolding estándar de repo (.editorconfig, plantillas GH, dependabot)".
- Repo GitHub `pdflector` (público, decisión 3 del plan): creado vía `gh repo create --source=. --push`; remoto `origin` configurado. URL: https://github.com/asierboveda/pdflector.
- Verificación previa al push: sin secretos en archivos tracked.

## 2026-08-05 — Fase 0.5 Ola 1: backend MuPDF + harness pdf_bench + spike Android PDFium

- **MuPDF backend (A)**: añadido `crates/pdf_core` tras feature `mupdf` (mitenu messense/mupdf 0.8.0, mupdf-sys 0.8.0, AGPL-3.0). Features mínimas: `default-features=false` + `base14-fonts`. Misma API `RenderEngine`/`Document` que PDFium. `MupdfEngine::new()` sin lib path (estático, thread-safe por context clonable por hilo, sin OnceLock). `engine/mupdf.rs`. Tests cruzados en `tests/mupdf_backend.rs` (5/5 OK): page_count==2 en simple.pdf = mismo que PDFium. Build C 1ª vez ~24 s.
- **pdf_bench (B)**: criterion 0.5 (`benches/open_render.rs`, grupos open/render_1x/render_2x) + sesgo binario `src/main.rs` (barrido manual, mediana de 3, sonda VmHWM → PEAK_RSS_KB). Genérico sobre `RenderEngine`. Números PDFium inicial: dense 9,69/35,34 ms, scanned 20,01/66,20 ms, paper 1,72/26,44 ms, large 6,86/35,10 ms. PEAK_RSS_KB=32520.
- **Android PDFium spike (C1)**: `pdf_core --features pdfium` cross-compila a `aarch64-linux-android` en **1 comando / 22 s** (bind dinámico en runtime, `libpdfium.so` ARM aarch64 descargada de bblanchon/pdfium-binaries chromium/7988 a `vendor/pdfium-android-arm64/`). `.cargo/config.toml` con linker `aarch64-linux-android24-clang`. `tools/fetch_pdfium_android.sh` (idempotente). `.gitignore` ampliado a `/vendor`.

## 2026-08-05 — Fase 0.5 Ola 2: spike Android MuPDF + comparativa real

- **Android MuPDF spike (C2)**: `pdf_core --features mupdf` cross-compila a `aarch64-linux-android` en **17 s** con 1 variable de entorno: `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot` (bindgen la necesita porque no usa el wrapper clang del NDK). Sin deps de sistema. Fricción **BAJA**. rlib ARM aarch64 confirmado. Dato clave del ADR.
- **Comparativa (D)**: `pdf_bench` extendido con selector de motor (feature `mupdf = ["pdf_core/mupdf"]`, arg CLI). Resultados finales (release, AMD Ryzen 7 5800H): MuPDF gana en render 2,7-4× (dense 1x 3,53 vs 9,69 ms; large 2x 10,19 vs 35,10 ms) y en **RSS pico -21%** (25572 vs 32520 KB). Único caso donde PDFium no pierde: scanned 2x empata. Tabla en `docs/benchmark-results.md`.

## 2026-08-05 — Fase 0.5 Ola 3: ADR-001 → MuPDF, AGPL-3.0, eliminación de PDFium

- Mantengo las decisiones del autor: **MuPDF** motor único (prioridad RAM+fluir), repo licenciado **AGPL-3.0** (LICENSE añadido).
- Creado `docs/adr/ADR-001-motor-pdf.md` con benchmark, justificación, consecuencias.
- **PDFium eliminado**: `crates/pdf_core/src/engine/pdfium.rs`, `tests/basic.rs` (el viejo), dep `pdfium-render`, feature `pdfium`, selector de CLI en pdf_bench. `pdf_app` y `pdf_bench` migrados a MuPDF.
- MuPDF pasa a ser **default** (no opt-in): `pdf_core/Cargo.toml` con `mupdf` como dep directa.
- `.cargo/config.toml` conservado (linker aarch64) con comentario de la env var `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` para MuPDF.
- Docs alineados: README (licencia AGPL-3.0), AGENTS.md §5/§6, PLAN.md §1/§6 + Fase 0.5 ✅.
- Verificación final: `cargo build --workspace` OK, `cargo test -p pdf_core` OK, `cargo clippy --workspace -D warnings` limpio, `cargo fmt` limpio, cross-compile aarch64 OK.
- `vendor/pdfium/` y `vendor/pdfium-android-arm64/` obsoletos (gitignored `/vendor`); se limpiarán tras Fase 6 si procede.
- **Fase 0.5 cerrada**. Próximo: Fase 1 (lectura fluida, scroll virtualizado, caché LRU).

## 2026-08-05 — Fase 0.5 Ola 4: Spike Android en hardware real

- **Hardware real**: activada depuración USB + autorización RSA en el Xiaomi 2412DPC0AG
  (adb autorizado, `NVQWDIOB7T9DVSG6 device`). Specs: arm64-v8a, Android 16 (SDK 36),
  8 cores, MemTotal 7.483.884 kB (~7,5 GB RAM).
- **Cambio mínimo en pdf_bench**: `corpus_dir()` lee la env var `PDFLECTOR_CORPUS_DIR`
  (fallback a `CARGO_MANIFEST_DIR/../../corpus`). fmt/clippy/test OK.
- **Cross-compile aarch64-linux-android release OK**: NDK r28 en PATH +
  `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=$ANDROID_NDK_HOME/.../sysroot`.
  Binario ~5,6 MB subido con `adb push` a `/data/local/tmp/pdflector/pdf_bench`; corpus
  (4 PDFs) en `/data/local/tmp/pdflector/corpus/`.
- **Medición sweep en el móvil** (mediana 3, escala 1x/2x, MuPDF release): render1x
  3,88–15,97 ms (3/4 PDFs superan 120 fps, todos ≥60 fps); render2x 35,79–84,33 ms
  (12–28 fps). PEAK_RSS_KB=31220 (~30,5 MB) < objetivo 150 MB → margen ~5×.
  `large` (500 p) no eleva el RSS (lazy load).
- **Hallazgo**: scanned (raster) es el peor caso (15,97 ms a 1x / 84,33 ms a 2x) →
  candidato a optimización futura del render de bitmaps.
- **Hallazgo**: a 2x se cae a 12–28 fps → futuro: tile/render cache para zoom fluido.
- **Decisión diferida / pendiente**: legal (SPDX headers, AGPL-3.0-or-later, atribución
  MuPDF) sigue SIN aprobar → push a GitHub sigue bloqueado. Commit `30b1b4a` sigue
  local sin push.
- **Próximo**: definir Fase 1 (render en dispositivo Android nativo vía app, no solo
  sweep CLI).

## 2026-08-05 — Fase 1 Ola 5: B1 caché LRU + scroll virtualizado

- **Hecho**: implementado `crates/pdf_core/src/cache.rs` — `RenderCache<E>` LRU
  **limitado por bytes** (crate `lru`); tipos `PageKey` (página + escala),
  `RenderedPage` (bitmap + byte_size real `w*h*4`) y `CacheStats` (hits, misses,
  evictions, current_bytes, entries). API `get_or_render` / `stats` / `clear` /
  `ensure_visible` (+ `resident_pages`); constructores `new(engine, doc, budget)`
  y `open(engine, path, budget)`; escalado por `scale_for_level(level)=2^level`
  (nivel 0 = 1x/72 dpi). Módulo `scroll.rs` — `Viewport` +
  `visible_and_prefetch_pages` **pura** (ventana visible + N colindantes, clampada)
  + `populate_visible`. Utilidad `corpus_dir()` en `pdf_core` (env var
  `PDFLECTOR_CORPUS_DIR` con fallback).
- **Hecho**: 18 tests REALES en pdf_core (5 basic preexistentes + 7 cache + 6
  scroll), todos con MupdfEngine + PDFs reales del corpus. Cero mocks.
  `cargo fmt`/`cargo clippy --all-targets -D warnings` limpio, build release OK.
- **Hecho**: bench `crates/pdf_bench/benches/cache_scroll.rs` con 3 escenarios
  (naive, caché 8 MB 1ª pasada, pass2 sobre residentes; VMHWM en proceso hijo).
  **Reducción 5× RAM**: naive 107412 KB (~105 MB) → caché 8 MB 21104 KB
  (~20,6 MB). Pass2 hits sobre residentes 0,35 ms. Detalle en
  `docs/benchmark-results.md`.
- **Hecho**: dep `lru = "0.18"` añadida (licencia MIT, verificada con `cargo info
  lru`, compatible con AGPL-3.0 del repo).
- **Hallazgo**: en 8 MB caben **4 páginas** de large_document a 1x (cada una
  ~2 MB). Invariante `current_bytes <= byte_budget` siempre cumplida en 30
  iteraciones con budget 4 MB.
- **Hallazgo de honestidad**: "pass2 todo hits en 50 páginas es matemáticamente
  imposible con caché byte-limitado menor que el barrido; se mide el hit path
  sobre las páginas residentes" (así se documenta también en el bench).
- **Pendiente (sigue SIN aprobar)**: mejora legal (SPDX headers, AGPL-3.0-or-later,
  atribución MuPDF). Push sigue bloqueado. Tres commits locales sin push:
  `30b1b4a`, `bd0aeea` y el de B1.
- **Próximo**: B2 (prefetch en hilos de fondo con cola prioritaria) y resto de
  Fase 1 (entregables 3-7): zoom, modo paginado, harness android-activity,
  overlay debug.

## 2026-08-05 — Fase 1 Ola 6: B2 prefetch hilos de fondo (actor model)

- **Hecho**: implementado `crates/pdf_core/src/prefetch.rs` (218 líneas) —
  arquitectura **actor model con 1 worker thread**. Razón crítica:
  `MupdfDocument` NO es Send-sound (raw `*mut fz_document` atado al TLS context
  del hilo creador — verificado por `cargo check` en sonda y mupdf 0.8).
  `Prefetcher::open(engine, path, budget)` crea el documento DENTRO del hilo
  worker; nunca `unsafe impl Send/Sync`. Single worker (pool múltiple inseguro
  con MuPDF TLS).
- **Hecho**: API pública `open`, `request(vp,total,radius,scale_level)` (no
  bloqueante — solo envía por canal mpsc; visibles PRIMERO, prefetch vecinos
  después; Request nuevo reemplaza wishlist), `cancel_pending`,
  `stats_snapshot()`, `resident_pages()`, `await_idle_timeout(timeout)`.
- **Hecho**: cambios mínimos en cache.rs (+`pub fn resident_keys()` 3 líneas,
  para soporte de `resident_pages` thread-safe vía round-trip por canal). Sin
  deps nuevas (stdlib: mpsc, thread, atomic, HashSet).
- **Hecho**: 6 tests REALES en tests/prefetch.rs (198 líneas) con MupdfEngine +
  large_document.pdf. Verifican: ① prefetch puebla cache en background (7 misses
  exactos); ② prioridad visibles-vs-prefetch observable vía resident_pages con
  budget pequeño forzado; ③ request() < 200ms (no bloquea); ④ Drop no cuelga
  (worker termina limpio); ⑤ cancel + reissue solo re-renderiza las nuevas
  páginas (misses delta exacto); ⑥ await_idle realmente espera (13 misses
  exactos).
- **Hallazgo honesto**: `await_idle_timeout` inicialmente roto (retornaba
  premature ~2ms sin esperar renders); arreglado con `Arc<AtomicU64>
  requested/completed` (poll 2ms hasta `completed >= requested_snapshot`). Race
  residual documentado: no usar request() concurrente durante await_idle.
- **Hecho**: 24 tests en pdf_core pasando (basic 5 + cache 7 + scroll 6 +
  prefetch 6), cero mocks, cero tests inventados. fmt + clippy
  `--all-targets -- -D warnings` limpios. Bench cache_scroll no-regresión (mismos
  números que B1: naive 105MB, cache 21MB).
- **Pendiente (sigue SIN aprobar)**: mejora legal (SPDX, AGPL-3.0-or-later,
  atribución MuPDF). Push a GitHub sigue bloqueado.
- **Próximo**: B3 (zoom con escalado rápido del bitmap + re-render nítido async)
  y resto de Fase 1 (entregables 4-7): modo paginado, harness android-activity,
  overlay debug.
