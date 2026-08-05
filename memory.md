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
