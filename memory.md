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

## 2026-08-10 — Investigación: arquitectura de rendimiento de Evince

- **Qué**: ingeniería inversa del visor Evince (GNOME, rama `main`) a partir de su
  código fuente real para extraer patrones de rendimiento replicables en PDFLector.
- **Entregable**: `docs/investigacion/evince-rendimiento.md` — informe con
  arquitectura verificada fichero:línea, mapeo a Android/PDFLector y buenas
  prácticas. Referenciado desde la Fase 0.5 de `docs/PLAN.md`.
- **Hallazgos clave** (verificados en código, no por rumores):
  - Un solo hilo de render global con 4 colas de prioridad (URGENT/HIGH/LOW/NONE)
    y cancelación estricta de jobs obsoletos; render serializado con mutex
    globales doc+fontconfig (`ev-job-scheduler.c`, `ev-jobs.c`).
  - Render completo de página a resolución de pantalla×zoom×device_scale
    (Poppler/Cairo, ARGB32) — **sin tiling ni multirresolución**.
  - Caché de píxeles = ventana deslizante con doble límite: 50 MB por bytes y
    ≤3 páginas de preload a cada lado; eviction al salir de la ventana;
    re-priorización en vuelo LOW→URGENT (`ev-pixbuf-cache.c`).
  - Zoom máximo auto-limitado por el presupuesto de caché: `max_scale = sqrt(cache/(w·dpi·4·h·dpi))` (ev-view.c:7581).
  - Prefetch N±1 y texto/mappings perezosos solo en rango visible±1; thumbnails
    embebidos del PDF para vistas rápidas (`poppler_page_get_thumbnail`).
  - GPU solo para composición (textura GDK); rasterizado 100 % CPU.
  - Progresivo = texturas viejas escaladas por GPU mientras llega el re-render.
- **Mapeo a PDFLector**: cola priorizada + 1 hilo de render por documento en
  `pdf_core` (PDFium/MuPDF no son thread-safe por documento), bitmap→textura con
  composición por transform, caché por bytes gobernada por el RSS objetivo
  (<150 MB), `FPDFPage_GetThumbnailAsBitmap`, límite de zoom por presupuesto.
- **Licencia**: Poppler es GPL-2+ → descartado para el proyecto; el patrón se
  replica con PDFium (BSD-3) o MuPDF (AGPL) — sin cambio en la decisión pendiente
  de Fase 0.5/ADR-001.
- Nota: Evince NO tiene smooth scrolling con render multibanda; la fluidez viene
  de composición barata + cancelación, no de paralelismo de rasterizado.

## 2026-08-10 — Baseline de rendimiento: Evince/poppler en escritorio

- **Qué**: primeras mediciones de rendimiento del proyecto, con Evince 48.4 /
  Poppler 26.07.0 como baseline (el backend de Evince es poppler+cairo; medido
  con `pdftoppm`, que usa ese mismo pipeline single-thread).
- **Entregable**: `docs/investigacion/evince-baseline.md` + script reproducible
  `tools/medir_baseline_evince.sh` (sin GNU time instalado → wrapper python3 con
  `resource.ru_maxrss`).
- **Hardware**: AMD Ryzen 7 5800H (8C/16T), 13 GiB RAM, Linux 7.1.4, Wayland.
  **Corpus**: `corpus/large_document.pdf` (500 páginas A4).
- **Resultados**:
  - Render 500 pág @72 dpi: 73,6 ms/pág · @144 dpi: 326 ms/pág (el coste
    cuadruplica al cuadruplicar píxeles). Max RSS del proceso de render: 22-28 MB.
  - Primera página (apertura+render) @144 dpi: ~0,36 s · @216 dpi: 0,60 s.
  - RSS del visor GUI con 500 páginas: **~198 MB** (frío y caliente) — supera el
    presupuesto objetivo del proyecto (<150 MB en tablet); Evince no es modelo
    de frugalidad, es tope superior a batir.
- **Implicación**: 326 ms/página @2x ⇒ el frame de scroll no puede contener
  nunca un render; confirma caché + render asíncrono + composición como
  obligatorios (patrón ya analizado en `evince-rendimiento.md`).
- **Uso**: números de referencia para el benchmark de Fase 0.5 (PDFium vs MuPDF
  vs poppler, misma máquina/método) y para fijar presupuesto de caché en Fase 1
  (página @2x ≈ 8 MB RGBA → 50 MB ≈ 6 páginas).

## 2026-08-12 — Fase 0.5 completa: MuPDF gana el benchmark → repo AGPL-3.0 (ADR-001)

- **Paso 1 — Backend MuPDF**: crate `mupdf` 0.8.0 evaluado (MuPDF 1.27.2,
  AGPL-3.0, activo — push del autor 2026-08-11, mismo autor que pdfium-render).
  Backend `MupdfEngine` en `pdf_core` con las mismas traits que PDFium
  (open/page_count/page_size/render_page → Bitmap RGBA), sin mutex global:
  mupdf-rs es thread-safe por diseño (fz_context por hilo). 4 tests equivalentes.
- **Paso 2 — Benchmarks criterion** (`cargo bench -p pdf_bench --features mupdf`,
  corpus 4 PDFs, 1x/2x, p1+central): ambos motores 4-15x más rápidos que el
  baseline poppler/Evince (73,6 ms @1x, 326 ms @2x, Ryzen 7 5800H). Escritorio
  con varianza alta (CPU scaling 89%) → dato direccional.
- **Paso 3 — Spike Android** (Lenovo Idea Tab 9469X, Android 15, serial
  A06B4A8E6774623, NDK r28 API 35): ambos compilan; PDFium necesita libpdfium.so
  arm64 precompilada + dlopen; MuPDF estático. Incidencia resuelta: bindgen de
  mupdf-sys usaba glibc del host → `BINDGEN_EXTRA_CLANG_ARGS` con sysroot NDK en
  `.cargo/config.toml`. **Timings tablet**: MuPDF gana 7/7 renders (13,1 ms/pág
  @2x large vs 29,4 PDFium; escaneado @2x 44 ms ambos — decodificador), open 1,4
  ms (10x), RSS pico 28,2 MB vs 26,6, binario 5,7 MB estático vs 5,1+.so 6,1 MB.
- **Paso 4 — Decisión**: **MuPDF + AGPL-3.0** confirmada por el autor. ADR-001 en
  `docs/adr/ADR-001-motor-pdf-mupdf.md`. Eliminado PDFium sin deuda (pdfium.rs,
  tests basic.rs, fetch_pdfium.sh, vendor/, features pdfium en pdf_bench).
  `pdf_app` migrado a MupdfEngine. Herramientas: NDK r28, target
  aarch64-linux-android, `.cargo/config.toml` (linker/CC/AR/bindgen sysroot).
- **Mediciones**: docs/investigacion/benchmark-motores.md (tablas tablet +
  escritorio + RSS + binarios).
- **Pendientes que abre la fase**: añadir `LICENSE` (AGPL-3.0); acotar el store
  de MuPDF (`set_store_max_size`) cuando la caché de `pdf_core` gobierne la RAM
  (Fase 1); skill `android-tablet-adb` propuesto (Paso 5, pendiente de
  aprobación).
