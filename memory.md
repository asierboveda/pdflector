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

## 2026-08-12 — Fase 1 Ola 7: Spike en tablet TCL NXTPaper 11 Plus (hardware objetivo final)

- **Hecho**: spike en la tablet TCL NXTPaper 11 Plus — hardware objetivo final
  (modelo 9469X, MediaTek MT8781 8× Cortex-A55 solo eficiencia, sin big cores,
  8 GB RAM, pantalla 1440×2200 @ 320 dpi, Android 15 / SDK 35, ABI arm64-v8a).
  adb autorizada; medición con pantalla ON (KEYCODE_WAKEUP + `svc power stayon
  true` durante la prueba, limpiado después) — evita el pesimismo de
  governor/doze.
- **Hecho**: cross-compile release `aarch64-linux-android` OK (pdf_core
  recompilado incluyendo los módulos nuevos B1/B2: cache/scroll/prefetch). Push
  de binario + corpus a `/data/local/tmp/pdflector/`. 2 corridas estables
  (difieren <5%).
- **Resultados TCL** (mediana de 2 corridas, pantalla ON):

  | PDF | open (ms) | render1x (ms) | render2x (ms) |
  |---|---|---|---|
  | dense (93p) | 0.40 | 14.51 | 44.18 |
  | scanned (30p) | 0.15 | 31.34 | 119.01 |
  | paper (12p) | 0.16 | 11.64 | 38.44 |
  | large (500p) | 0.25 | 15.40 | 44.73 |

  PEAK_RSS_KB = 26688 (~26,7 MB).
- **Análisis de aceptación contra PLAN.md Fase 1**: render <25 ms cumple en 3/4
  PDFs (dense 14.5, paper 11.6, large 15.4 — todos <25 ms; solo scanned 31 ms lo
  excede, PDF raster = worst case esperable). RSS <150 MB cumple con ~6× de
  margen (26,7 MB). 60 fps: dense→69 fps, paper→86 fps, large→65 fps cumplen;
  scanned→32 fps NO (worst case raster).
- **Corrección HONESTA**: NO es correcto decir "TCL más rápida que Xiaomi". La
  comparación correcta, MISMO ESCALA render1x: TCL es ~2,3× MÁS LENTA que el
  Xiaomi phone. Razón: el Xiaomi tiene big cores (Cortex-A78/A715-class),
  mientras el MT8781 de la TCL tiene 8× Cortex-A55 solo eficiencia — tablet
  enfocada a lectura, no a rendimiento. Se documenta honestamente.
- **vs Desktop** (AMD Ryzen 7 5800H, Fase 0.5): el desktop es 3,5-5,3× más
  rápido que la TCL (esperable; ratios calculados de los datos de
  `docs/benchmark-results.md`).
- **Conclusión**: la tablet cumple los objetivos para PDFs vectoriales (la
  mayoría); no para scanned ni zoom 2x — justamente las optimizaciones futuras
  (B3 zoom, tile/render cache).
- **Pendiente (sigue SIN aprobar)**: mejora legal (SPDX, AGPL-3.0-or-later,
  atribución MuPDF). Push a GitHub sigue bloqueado.
- **Próximo**: B3 (zoom con escalado rápido del bitmap + re-render nítido async)
  y resto de Fase 1 (entregables 4-7): modo paginado, harness android-activity,
  overlay debug.

## 2026-08-13 — Revisión integral en paralelo (6 workers deepseek-v4-flash) + correcciones

- **Método**: revisión y corrección de TODO el repo con 6 agentes `deepseek-v4-flash` en
  paralelo (uno por carpeta), coordinados vía Orca orchestration (Run
  `run_08287002a2a6`). Los 6 workers terminaron `succeeded`.
- **crates/pdf_core** (3 bugs reales + 3 tests de regresión):
  - Race en `Prefetcher::cancel_pending()`: no incrementaba el contador `requested`,
    por lo que tras request→cancel→reissue, `await_idle_timeout()` devolvía `true` sin
    renderizar el reissue (el test de regresión falla sin el fix: 43 misses vs 50).
  - Overflow de `usize` en `visible_and_prefetch_pages` (panic en debug, wrap en
    release) → resuelto con `saturating_add`.
  - `resident_pages()` documentaba "distinct" pero devolvía duplicados con varios
    niveles de zoom → deduplicación preservando orden MRU.
  - Documentado el drop intencional de errores de render por página en el worker.
- **crates/pdf_app**: render movido a worker de hilo de fondo (patrón actor de
  `pdf_core::prefetch`; `mupdf::Document` no es `Send`), polling no-bloqueante, estado
  UI consistente al cambiar de PDF, `MupdfEngine::new` propaga error, escala a
  resolución de pantalla (`scale_for_level` × `pixels_per_point`).
- **crates/pdf_bench**: registrado el bench `render_perf` en `Cargo.toml`
  (`harness=false`; antes autodescubierto ejecutaba 0 benchmarks, no medía nada),
  eliminadas 4 copias de resolución de corpus en favor de `pdf_core::corpus_dir()`
  (soporta `PDFLECTOR_CORPUS_DIR`), eliminado `_startup_marker` muerto.
- **tools/**: bug real en `generate_corpus.py` — `scanned_pages.pdf` embebía 30× la
  misma imagen (`drawImage` deduplica por *filename*) → `ImageReader(img)` (dedup por
  contenido, sin PNG temporal); `invariant=1` en los 4 constructores → corpus
  byte-reproducible. En `bench_evince.sh`: `set -e` ya no aborta si pypdf falla
  (`pages=0`), y el `pkill -f` global (mataba Evince del usuario) se sustituyó por
  `kill -- -$pid` con `setsid`.
- **docs/**: 9 ficheros corregidos — menciones obsoletas a PDFium
  (ADR-001 = MuPDF/AGPL-3.0), tablet actualizada a TCL NXTPaper 11 Plus con mediciones
  reales, refs rotas reparadas, aritmética 13/16→14/16 corregida y relato falso del
  cierre de ADR-001 en tablet corregido.
- **raíz + .github/**: `actions/checkout@v7` existe (verificado con `git ls-remote`);
  README corregido (el setup genera corpus antes de `cargo test -p pdf_core`; eliminada
  la referencia a `tools/fetch_pdfium.sh`).
- **Coordinador**: AGENTS.md §1 y `.opencode/skills/android-tablet-adb/SKILL.md`
  actualizados de "Lenovo Idea Tab" a "TCL NXTPaper 11 Plus" (9469X) — hardware real
  desde la Ola 7.
- **Verificación final**: `cargo fmt --all -- --check` limpio; `cargo clippy
  --all-targets -- -D warnings` limpio; `cargo test -p pdf_core` **27/27 OK** (corpus
  regenerado). Sin commits (regla AGENTS.md).
- **Pendientes de decisión (no tocados)**:
  - `show_extras=true` en `mupdf.rs` rasteriza las anotaciones del PDF dentro del
    bitmap (contradice AGENTS.md §4.3; ligado a Fases 3-4).
  - El worker de prefetch no expone su muerte al cliente (`send` falla en silencio).
  - `expect` en `cache.rs` es invariante demostrable (se dejó).
  - `bench_evince.sh`: `ready_clients` matchea por clase y `/tmp/bench-evince.log` es
    ruta fija (observaciones menores).
  - Legal (SPDX, AGPL-3.0-or-later, atribución MuPDF) sigue sin aprobar → push a
    GitHub sigue bloqueado.

## 2026-08-13 — Fase 1 B3: zoom (escalado rápido + re-render nítido + invalidación selectiva)

- **Método**: 3 workers `deepseek-v4-flash` coordinados vía Orca (Run `run_33e07b6b498d`):
  B3-core (pdf_core) → luego, en paralelo, B3-app (pdf_app) y B3-bench (pdf_bench).
  Todos `succeeded`.
- **pdf_core** (nuevo módulo `src/zoom.rs` + cambios en cache/engine/lib):
  - `scale_bitmap(&Bitmap, w, h) -> Result<Bitmap>`: escalador bilinear software
    (std puro, clamp a bordes, determinista, sin deps).
  - `scale_level_for_zoom(zoom) -> u32`: `max(0, ceil(log2(zoom)))` — el re-render
    nítido nunca es un upscale; zoom<=0/NaN clampa a nivel 0.
  - `RenderCache::trim_to_scale_level(keep_level)`: invalida los demás niveles con
    contabilidad correcta de `current_bytes`/`evictions` (evita thrashing al zoom).
  - Nuevo `Error::InvalidArgument(String)`. 16 tests nuevos (unit + integración con
    engine fake).
- **pdf_app**: zoom continuo (1.0, clamp 0.25–8.0) por ctrl+rueda/pinch (`zoom_delta`)
  y botones ±; fast path por GPU (textura existente reescalada) + sharp path async
  (`scale_level_for_zoom` × ppp) con un solo receiver `pending` que descarta renders
  obsoletos; hilo UI siempre en `try_recv`/`request_repaint_after`.
- **pdf_bench**: `benches/zoom.rs` (registrado en Cargo.toml, harness=false) con 3
  grupos: `scale_bitmap` (fast), `rerender` (sharp), `trim_to_scale_level`.
- **Medición (desktop AMD Ryzen 7 5800H, criterion --quick, 2026-08-13)** — ver
  `docs/benchmark-results.md`:
  - `scale_bitmap` a página completa: 31 ms (z1.5) / 55,9 ms (z2) / 215 ms (z4).
  - re-render MuPDF: 3,4 ms (nivel 1/×2) / 12 ms (nivel 2/×4).
  - `trim_to_scale_level`: 6,4 ms.
  - **Hallazgo honesto**: el escalado software es ~16–18× más lento que el re-render
    nativo → el camino "inmediato" del zoom en la UI es GPU (egui), no `scale_bitmap`;
    `scale_bitmap` queda como utilidad pura/testeable para contextos headless (harness
    Android). El re-render nítido es barato (3,4–12 ms, dentro de 60 fps).
- **Verificación final**: `cargo fmt --all -- --check` limpio; `cargo clippy
  --all-targets -- -D warnings` limpio; `cargo test -p pdf_core` **43/43 OK**.
  Sin commits (regla AGENTS.md).
- **Próximo (Fase 1)**: modo paginado, harness android-activity, overlay de debug.
  Sigue pendiente la decisión legal (SPDX/AGPL-3.0-or-later/atribución MuPDF) →
  push a GitHub bloqueado.

## 2026-08-13 — Fase 1 B3 Ola 8: zoom medido en la tablet TCL (fast vs sharp)

- **Método**: 1 worker `deepseek-v4-flash` (Run `run_70ee5801a6ca`). Añadió
  `run_zoom_section` al sweep de `pdf_bench/src/main.rs` (tras el sweep y tras el
  print de PEAK_RSS_KB, para no contaminar el RSS), cross-compiló a
  `aarch64-linux-android` (NDK r28) y midió en la tablet TCL 9469X con pantalla ON
  (2 corridas, batería 66% cargando, 33 °C).
- **Zoom en tablet (mediana de 3, 2 corridas)**: `scale_bitmap` (fast) vs re-render
  nítido (sharp) — ver `docs/benchmark-results.md`:
  - 2x: 69,4–70,2 ms vs 14,9–16,6 ms (~4,5× más lento el fast).
  - 4x: 275,8–325,1 ms vs 53,2–59,4 ms (~5,2–5,6× más lento).
  - **Conclusión**: el escalado software naïve (sin SIMD/NEON) NO es un fast path:
    supera de largo el presupuesto de 16,6 ms y es más caro que re-renderizar. El
    camino inmediato correcto es el reescalado de textura por GPU (ya implementado
    en pdf_app). `scale_bitmap` queda para headless y necesita optimización si se
    quiere usar en UI.
- **Comparación Ola 7 vs Ola 8 (sweep)**: render1x/render2x más altos en algunos
  PDFs, PERO el path de render (`mupdf.rs`) no cambió → NO es regresión de código.
  Dos causas: (1) **confound del corpus** — el fix de `tools/generate_corpus.py`
  hizo que scanned_pages.pdf embeba 30 imágenes DISTINTAS (antes 30 refs a la
  misma), así que render 3 páginas decodifica 3 imágenes distintas → explica el
  +render de scanned y el RSS +~5 MB (31,9 vs 26,7 MB; 3 pixmaps ~2,2 MB c/u en
  caché vs 1); (2) **varianza termal/governor** en dense/paper (saltos no
  reproducibles entre corridas). Para comparación limpia: fijar governor y N≥5
  corridas.
- **Verificación**: host build/clippy/fmt limpios; cross-compile Android release
  OK (19,3 s). Sin commits (regla AGENTS.md).

## 2026-08-13 — Primera versión completa: lectura fluida + anotaciones + export + sync + IA

> Decisión del autor: objetivo = primera versión usable, bajo coste y muy veloz; **se
> descarta "modo paginado"**. El coordinador (deepseek-v4-pro) dividió en olas paralelas
> de workers `deepseek-v4-flash` (Run v1: `run_1ba8feb32901`, v2: `run_86302b5a1f5f`,
> v3: `run_25e783f6a06f`, v4: `run_3ad2046840e2`, v5: `run_38b6e2764ffc`). Todo verificado
> con tests, clippy -D warnings y fmt; **93 tests** en pdf_core al cierre.

- **Scroll virtualizado en pdf_app (Fase 1)**: el hilo UI traduce viewport →
  `Prefetcher::request` → `get_page(page,level)` (sondeo try_recv) → textura, pintando
  solo páginas visibles (±1) y soltando texturas fuera de ventana; byte budget 32 MB,
  prefetch con radio adaptativo (un radio fijo evictaba las visibles del LRU). RSS plano
  durante scroll.
- **pdf_core nuevos módulos**: `zoom` (scale_bitmap + scale_level_for_zoom + trim),
  `dark::invert_bitmap`, `metrics::{FrameTimer,read_rss_kb}` (p95 ring 600),
  `Prefetcher::get_page` + `RenderCache::peek_clone` (Bitmap ahora Clone),
  `annotations` (Stroke/Highlight/TextNote vectoriales en coords de página, AnnotationSet,
  serde), `store` (SQLite sidecar `annotations/<stem>.db` vía rusqlite bundled),
  `export` (Markdown con citas+nº página, y PDF con anotaciones estándar /Ink /Highlight
  /Text vía API de MuPDF — verificado con pypdf), `sync` (layout + `watch_annotations` con
  notify, debounce 150 ms), `ai` (chunk_pages + OllamaClient HTTP crudo por std::net).
- **Extracción de texto perezosa**: `Document::text(page) -> PageText{text, spans}` con
  bbox por línea (mupdf 0.8 stext) — base de subrayado y de los chunks de IA.
- **pdf_app features**: modo oscuro (inversión solo al subir textura, caché SIEMPRE normal;
  persistencia en eframe storage), overlay de debug (p95 frame time < 16,6 ms, RSS, cache),
  capa vectorial de dibujo ✏️ (Stroke; transformación cursor→página = (pos-rect.min)/zoom,
  verificada a ±2 px con inyección XTEST), panel chat IA 💬 (hilo de fondo, llama3.2 en
  localhost:11434, error claro si Ollama no responde), persistencia/export/sync integrados
  (load sidecar al abrir, save al commitear, Export MD/PDF en hilo de fondo, watcher de
  sidecar para hot-reload de Syncthing).
- **Dependencias nuevas (justificadas)**: serde+serde_json (modelo/persistencia/export),
  rusqlite bundled (SQLite sidecar, sin lib de sistema → cross-compila a Android),
  notify 8 (detección de cambios en disco), eframe feature `persistence` (preferencia dark).
- **Decisiones tomadas por el coordinador (autorizadas por el autor)**: Ollama en el PC
  (localhost, la app solo hace HTTP); formato canónico de anotaciones = tabla SQLite
  (id, page_idx, kind, payload JSON) + serde; sidecar `annotations/<stem>.db`.
- **Pendiente (NO tocado)**: legal — SPDX headers, AGPL-3.0-or-later y atribución MuPDF
  (bloquea push a GitHub); y lo que requiere la tablet (harness android-activity, Fase 6
  Android UI Slint/Tauri, Syncthing en la tablet, medición final). Sin commits (regla
  AGENTS.md).

## 2026-08-13 — App Android nativa (pdf_android): PDF renderizado en la tablet

- **Gate cross-compile**: pdf_core y pdf_bench con TODAS las deps nuevas (serde, rusqlite
  bundled, notify) cross-compilan limpios a `aarch64-linux-android` (NDK r28): rusqlite
  (libsqlite3-sys) usa el clang del NDK vía `cc`; notify compila con backend inotify sobre
  Android. No hizo falta gatear nada por feature.
- **Crate nuevo `crates/pdf_android`** (cdylib, `android-activity` 0.6 native-activity +
  `ndk` 0.9 + pdf_core): `android_main` abre un PDF, renderiza la página con pdf_core a
  escala contain y la blitea al ANativeWindow fila a fila respetando `stride`, formato
  forzado R8G8B8A8_UNORM (defensa RGB565); tap derecha/izquierda para pasar página.
  Añadido a members del workspace pero fuera de `default-members` (es Android-only).
- **Empaquetado + despliegue**: cargo-apk v0.10.0, package `com.pdflector.app`, APK debug
  (debuggable) en `target/debug/apk/pdf_android.apk`. SELinux bloquea leer /data/local/tmp
  a un untrusted_app → la app lee `internal_data_path()/demo.pdf` y el PDF se inyecta con
  `adb shell run-as com.pdflector.app`.
- **VERIFICADO en la tablet TCL 9469X**: el PDF (12 pág.) se ve renderizado — pantalla con
  página blanca + texto sobre letterbox gris, 0 píxeles rojos (screenshot analizado por
  píxeles: media 236,236,236, ~91% blanco), taps avanzan página, logcat "opened 12 pages".
- **RSS (build DEBUG con debuginfo)**: TOTAL PSS ~85 MB, TOTAL RSS ~205 MB (`dumpsys
  meminfo`). El objetivo <150 MB RSS aplica a RELEASE: queda medir el APK release.
- **Próximo**: build release + medición RSS/frame-time en la tablet; spike Slint vs Tauri
  (Fase 6) para la UI final; y la decisión legal sigue bloqueando el push.

## 2026-08-13 — Release en tablet + spike Slint/Tauri + skill de medición

- **Release medido en la TCL 9469X** (APK release, LTO+strip): render **18,0–18,2 ms/pág**
  (<25 ms ✓), blit ~3,8 ms, **TOTAL PSS ~66 MB** (<150 MB ✓). El `dumpsys meminfo` TOTAL RSS
  da 188 MB pero inflado por librerías compartidas del runtime (Code 86 MB RSS / solo 2,4 MB
  PSS) → el objetivo <150 MB debe leerse como **PSS**, no RSS bruto (documentado en
  `benchmark-results.md` pendiente de añadir).
- **Input en tablet**: con `android-activity::input_events_iter()` el tap SÍ llega a Rust
  (logcat `page 2` + re-render de la página); queda verificar el refresco visual de la
  superficie tras avanzar (los screenshots post-tap salieron idénticos: posible stale de
  `screencap` o de la superficie — pendiente de debug).
- **ADR-004 (docs/adr/ADR-004-ui-android.md)**: spike Slint 1.17.1 vs Tauri v2 → **Slint
  recomendado** (APK 6,4 MB, ~62 MB PSS, build 1m20s, sin Node) porque Tauri v2 rompe el
  presupuesto de RAM (WebView) y añade latencia IPC. HALLAZGO BLOQUEANTE: en esta tablet el
  input por el looper de android-activity (`InputAvailable`) no llega a Slint (reproducible
  con cargo-apk/xbuild/android-activity puro) — mitigación conocida: usar el camino directo
  `input_events_iter()` (que pdf_android demuestra que funciona). ADR queda en estado
  Propuesto hasta validar input con el lápiz/dedo.
- **Skill nuevo**: `.opencode/skills/pdflector-rendimiento/SKILL.md` unifica el procedimiento
  de medición (desktop/bench, cross-compile, app en tablet, dumpsys/screencap).
- **Próximo**: resolver el input/refresco en la tablet, validar el ADR-004 con el lápiz, y la
  decisión legal (SPDX/AGPL-or-later/atribución MuPDF) sigue bloqueando el push.

## 2026-08-13 — Tablet: gestos + zoom verificados; ADR-004 (Slint) ACEPTADO

- **“Bug” de refresco = falso positivo**: scientific_paper.pdf tiene las 12 páginas
  PIXEL-IDÉNTICAS (md5 idéntico con pdftoppm; el generador del corpus no varía el
  contenido por página). Con large_document.pdf (500 pág.) los screenshots de páginas
  distintas difieren y correlacionan con logcat → el refresco de la superficie SIEMPRE
  funcionó. (Nota de corpus: scientific_paper.pdf es malo para probar paso de página.)
- **pdf_android: gestos implementados y verificados en release** (TCL 9469X): swipe 4
  direcciones (umbral 25% del eje dominante) → página; pinch 2 dedos → zoom continuo
  0.25–8x con re-render MuPDF directo (NO scale_bitmap, 4–5,6× más lento); doble-tap →
  zoom 1x↔2x; tap derecha/izquierda → página. Defensa extra: re-obtener native_window()
  en WindowResized/RedrawNeeded. Verificado: render ~20 ms, blit ~3,8 ms, PSS 66 MB,
  doble-tap zoom 2.0/1.0 en logcat con screenshots distintos (E1≠E2, 793.764 px de
  diff). El pinch físico (2 dedos) no es inyectable por adb (SELinux /dev/input) →
  pendiente de confirmación manual con el dedo.
- **ADR-004 (Slint) → ACEPTADO**: el “bloqueo de input” era un artefacto del test (la
  app pura no dibujaba al ANativeWindow → ventana sin touchableRegion → el sistema no
  entrega toques). InputAvailable + TouchArea.clicked llegan por el looper estándar;
  input_events_iter es solo el drenaje posterior. Riesgo nuevo documentado (7.2): Slint
  1.17.1 no repinta tras cambios de propiedad en esta tablet (upstream #8692/#12687/#12688).
  Pendientes Fase 6: lápiz real, linker API 26, mediciones finales.
- **Estado**: primera versión funcional en escritorio (completa) y en tablet (render +
  gestos + zoom). Próximo natural: port completo a Slint (Fase 6) y validación con lápiz.

## 2026-08-13 — Tablet: selector de PDF + corpus scientific_paper arreglado

- **Selector de archivo en pdf_android (fallback in-app)**: SAF (ACTION_OPEN_DOCUMENT) NO es
  viable en este stack (android-activity 0.6.1 no expone onActivityResult en ningún backend;
  cargo-apk/ndk-build no compilan Java → subclasear la Activity exigiría inyectar dex a mano).
  Implementado en su lugar: botón “Open” → lista de `*.pdf` de internal/external (+ `pdfs/`)
  dibujada con android.graphics.Canvas vía JNI (jni 0.22) → tap abre con MupdfEngine::open,
  con Rescan/Back/scroll y franja de error. Verificado en TCL 9469X release: logcat
  “picker: 5 PDFs found” / “opened: 93 pages” (dense_textbook) / “cannot open” con PDF
  corrupto; 7 screenshots analizados por píxeles. Cómo añadir PDFs: `adb push` + `run-as` a
  internal, o copiar a `Android/data/com.pdflector.app/files/pdfs/`.
- **Corpus**: `tools/generate_corpus.py` arreglado para que scientific_paper.pdf tenga 12
  páginas DISTINTAS (secciones realistas + figuras vectoriales por página, RNG local seed
  42+page, reproducible byte a byte). Antes eran 12 páginas pixel-idénticas. Los 4 PDFs
  siguen generándose (93/30/12/500 pág.).
- **Pendiente (deferido por el autor)**: lápiz real y legal (SPDX/AGPL-or-later/atribución).

## 2026-08-13 — pdf_android abre PDFs externos vía "abrir con" (ACTION_VIEW)

- **"Abrir con" desde Descargas/gestor de archivos**: intent-filter `ACTION_VIEW` +
  `application/pdf` declarado por metadatos TOML de cargo-apk 0.10 (sin manifest propio);
  en android_main se lee `Activity.getIntent().getData()` por JNI (jni 0.22): `content://` →
  `ContentResolver.openInputStream` → copia a `internal/pdfs/` → `MupdfEngine::open` (nombre
  vía OpenableColumns.DISPLAY_NAME). La app queda registrada como visor PDF del sistema.
- **Verificado en TCL 9469X (release)**: content:// con grant abre el PDF externo (logcat
  "opened: 12/15 pages" + screenshots), incluido flujo real desde Files by Google; `file://`
  falla por Scoped Storage (Permission denied, esperable en Android 15) y cae al picker.
- **Pendiente**: decidir si se quiere soporte `file://` (permisos de almacenamiento) — no
  recomendable; content:// es el estándar. Y sigue diferido lápiz + legal.

## 2026-08-13 — Biblioteca MediaStore en el arranque (carpeta + nombre, tipo Evince)

- **Arranque normal = biblioteca**: al lanzar sin intent, pdf_android consulta `MediaStore.Files`
  (JNI, jni 0.22) con proyección `[_ID, DISPLAY_NAME, RELATIVE_PATH, _SIZE]`, selección
  `mime_type='application/pdf'` y orden `RELATIVE_PATH, DISPLAY_NAME`; cada fila → content URI
  (`ContentUris.withAppendedId(files_uri, _ID)`). La UI reutiliza el dibujo Canvas+JNI del
  picker: filas con NOMBRE (línea 1) + CARPETA (línea 2, más pequeña) + tamaño; scroll por
  arrastre, botones Rescan/Grant/Back. Tocar una fila → `openInputStream(uri)` → copia a
  `internal/pdfs/` → `MupdfEngine::open`. El picker de carpeta interna queda como fallback si
  MediaStore devuelve vacío.
- **Permiso (verificado en TCL 9469X, Android 15)**: en Android 13+ la lectura de PDFs ajenos
  exige el appop **"All files access"** (`MANAGE_EXTERNAL_STORAGE`, concedido en Ajustes;
  no existe `READ_MEDIA_*` para documentos). `READ_EXTERNAL_STORAGE` (maxSdkVersion=32) se
  declara solo para Android ≤ 12. La app detecta el estado con
  `Environment.isExternalStorageManager()`; sin permiso muestra botón **Grant** que abre
  `Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION`; al volver, el `Resume` re-consulta.
  Testing: `adb shell appops set com.pdflector.app MANAGE_EXTERNAL_STORAGE allow`.
- **Hallazgo del corpus**: los PDFs metidos con `adb push` quedan `is_pending=1` en MediaStore
  y son invisibles para otras apps (la query solo devuelve los committed: 3 de 256). Se
  "commitean" con `adb shell content call --uri content://media/none --method scan_volume
  --arg external_primary`. Comportamiento correcto de Android, no bug de la app.
- **Verificado en TCL 9469X (release, APK firmado debug keystore)**: lanzamiento normal →
  biblioteca con los 256 PDFs reales (screenshot analizado por píxeles: filas con texto, 0 rojo);
  tap en fila → "library open: … (…) -> files/pdfs/… (N bytes)" + "opened: 328 pages" + página
  renderizada; scroll y doble-tap-zoom/sin regresión en gestos; "abrir con" content:// sigue
  abriendo directo (sin biblioteca). Sin permiso → prompt Grant → Ajustes → conceder → Resume
  re-consulta → 256 PDFs.
- **Nota rendimiento**: primer render de un PDF 56 MB/442 pág. tarda 369 ms (carga de fuentes
  del primer open; el render normal sigue ~18-25 ms/pág). La query de 256 filas tarda <1 s en
  total (incluye el render JNI del primer frame).

## 2026-08-13 — Tablet: pinch rápido + pantalla completa + sin doble-tap (pdf_android modular)

- **Partición previa** (enabler): `crates/pdf_android/src/lib.rs` (2625 l.) partido en 6
  módulos (lib, reader, input, draw, jni, view, zoom) sin cambiar comportamiento; 3 cambios
  posteriores en paralelo sobre ficheros disjuntos.
- **Quitar doble-tap**: eliminado el path de doble-tap (GestureState.last_tap, DOUBLE_TAP_*,
  toggle_zoom, resets) — el zoom es SOLO pinch con los dedos. Tap simple intacto.
- **Pinch rápido (optimización)**: `zoom::blit_fast` escala el bitmap por vecino-más-cercano
  (aritmética entera, tabla x precalculada, sin f32 por píxel, ~memcpy de pantalla) SIN
  re-render de MuPDF durante el Move; al soltar, `set_zoom_sharp` re-renderiza UNA vez.
  `Reader.rendered_zoom` + blit con zoom RELATIVO (`zoom/rendered_zoom`) para no doblar el zoom
  tras el re-render nítido (bug de integración detectado y corregido por el coordinador).
- **Pantalla completa**: `view::initial_scale` de “contain” (letterbox) a **“cover”**
  (`max(win_w/page_w, win_h/page_h)`), rellenando toda la pantalla y recortando márgenes;
  bonus `view::crop_margins(bitmap)` (bbox del contenido no-blanco, sin caller aún).
- **Verificado en TCL 9469X (release)**: biblioteca con 256 PDFs, tap → “opened: 65 pages”
  (exámenes anmi 2022-25.pdf), render a escala cover 2.613 → 1556×2200 (llena la pantalla,
  recorta ancho), screenshot sin barras de letterbox (0 px gris en bordes, media 249 blanco).
  El pinch (2 dedos) no es inyectable por adb → queda confirmarlo a mano.
- **Nota build**: `cargo apk build` necesita `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=
  --sysroot=$ANDROID_NDK_HOME/.../sysroot` + bin del NDK en PATH (ya en el skill
  pdflector-rendimiento); el error `pthreadtypes-arch.h: regparm` era eso (host glibc vs sysroot).

## 2026-08-13 — Ronda de features (desktop + bench + tablet) en olas paralelas

- **Desktop (pdf_app)**: Highlight (drag sobre texto → rects por línea vía Document::text+spans,
  amarillo semitransparente), TextNote (clic → input flotante, marcador+tooltip), panel de
  anotaciones (lista página+tipo+resumen, clic salta/centra, botones ✕ borrar y ✎ editar nota),
  recientes (últimos 5 PDFs en storage eframe). Todo persistido en el sidecar SQLite.
- **Bench (pdf_bench)**: `benches/annotations.rs` (add/for_page/serialize/store_roundtrip) para
  validar que el modelo de anotaciones escala (criterio Fase 3: 200+ trazos sin degradar).
- **Tablet (pdf_android)**:
  - Persistencia de posición (state.json: ruta+page+zoom+dark, restaurada al abrir), indicador
    "N / total" con saltos ±10 y tap=next, modo oscuro (inversión inline en el blit, sin
    re-render, fondo negro), barra superior con Open/−10/+10/Dark.
  - **Scroll vertical continuo + caché**: PageCache LRU (48 MiB / 5 páginas), documento como
    columna de páginas apiladas, viewport visible_pages(), blit_stacked con un solo lock+present
    y vecino-más-cercano; arrastre vertical=scroll, swipe horizontal/tap=salto de página
    instantáneo (caché), pinch fast/sharp conservado.
  - **Anotaciones a mano (Stroke)**: botón ✏️, arrastre crea Stroke en coords de página
    (transformación inversa del blit), render Bresenham+brocha con recorte Liang-Barsky, sidecar
    SQLite (carga al abrir, guarda al soltar), undo (↶) y paleta de 3 colores (●).
  - **Buscador en biblioteca**: índice vertical de letras A-Z+'#' (filtra por inicial normalizada:
    acentos→base, números/símbolos→'#'); sin IME. La agrupación colapsable por carpeta se descartó
    (MediaStore ya ordena por relative_path+display_name y cada fila muestra su carpeta).
- **Verificación**: build release aarch64, clippy y fmt limpios en cada agente; host clippy/fmt
  verdes. Sin commits (regla AGENTS.md).
