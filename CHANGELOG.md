# CHANGELOG — PDFLector

> Registro completo de cambios, más reciente arriba. Sin historial paralelo.
> Cada entrada cierra con verificación: fecha + hardware + flujo + métrica (regla AGENTS.md).
> Formato: `AAAA-MM-DD — Título`.
>
> Las entradas anteriores a 2026-09-18 son registro histórico: citan ficheros que se
> borraron en la auditoría documental de esa fecha. Se conservan tal cual porque un
> changelog no se reescribe; el contenido está en el historial de git (`git log -- <ruta>`).
> Para el estado vigente, ver `docs/README.md`.

## 2026-09-18 — El repositorio queda en una sola rama: `main`

Limpieza final de ramas. Solo queda `main` (`047b467`), tanto en local como en GitHub.

- **`asierboveda/feat-ui-polish-zones` eliminada** (1 commit huérfano, `2cb6705`, del 2026-08-14, nunca tuvo PR).
  Auditoría del commit antes de borrarlo: su parte visual (acción primaria dorada, más espaciado en
  biblioteca y sheet) está **superada** por el sistema de temas Readest que vive hoy en `main`. Sí
  quedaban dos mejoras funcionales sin rescatar en el panel de IA (**Copiar** y **Regenerar/Reintentar**),
  que se anotan en el issue #36 para cuando se retome la fase D.
- **Ramas de Dependabot**: GitHub las eliminó automáticamente al cerrar sus PRs (los 5 migrados en `047b467`).
- Estado final: `git ls-remote --heads origin` devuelve **una sola referencia**, `refs/heads/main`.

## 2026-09-18 — Fase D (IA) aplazada y errores factuales corregidos en su plan

- **Decisión del dueño: la IA se aplaza** hasta que la estructura del proyecto esté consolidada.
  Registrado en `docs/plan/NEXT-PLAN.md` (nueva sección *Orden de trabajo vigente*, con la fase D
  marcada como aplazada) y en `docs/plan/D-ia-contexto.md` (aviso de cabecera) e issue #36
  retitulado. La IA básica (explicar la selección con Groq/Gemini) ya está en el producto; lo
  aplazado es el salto a contexto global del documento (RAG BM25).
- **Corregido en `D-ia-contexto.md`**: citaba `crates/pdf_android/src/draw/ai_panel.rs`, que **no existe**;
  el panel vive en `draw/overlays.rs` (`render_ai_panel`, `ai_panel_layout`). Sustituido en las dos
  apariciones (componentes y referencias).
- Añadido el **orden de trabajo vigente** al roadmap: cerrar A4/A5/C/E3 y F en hardware real, luego
  la deuda con issue, y D al final y solo cuando el dueño lo pida.

## 2026-09-18 — Dependencias al día: los 5 PRs de Dependabot, migrados

Los 5 PRs (#5, #6, #22, #23, #24) llevaban abiertos desde el 12–19 de agosto contra una base que ya
no existía. Se aplicaron **juntos** en una rama y se verificaron con el compilador: en GitHub los 3
marcados «CLEAN» solo significaban que no había conflicto de **texto**, no que compilaran.

- **Sin cambios de código**: `android_logger` 0.14→0.15, `rusqlite` 0.37→**0.40** (los 7 tests del
  sidecar SQLite pasan) y `reqwest` 0.11→**0.12** (cross-compila a aarch64 con `rustls-tls`).
- **`criterion` 0.5→0.8 (migración)**: `criterion::black_box` quedó deprecado y con el
  `-D warnings` del CI rompía el build (43 avisos). Sustituido por `std::hint::black_box` en los 7
  benches que lo usaban. Verificado **en ejecución**, no solo compilado: `cargo bench -p pdf_bench
  --bench highlight -- --quick` corre y reporta.
- **`eframe` 0.32→0.36 (migración de API)**: egui 0.36 es un rediseño, no un salto.
  - `App::update(ctx, frame)` → `App::ui(ui, frame)`. El cuerpo sigue escrito contra `ctx`; se
    obtiene con `ui.ctx().clone()` (el `Context` es un `Arc`) para no reescribir 2.800 líneas.
  - `egui::SidePanel::{left,right}` → `egui::Panel::{left,right}` (struct unificado);
    `default_width` → `default_size` (el `Panel` mide en el eje del panel). `CentralPanel` y
    `Window::show(ctx, …)` siguen igual.
  - `ctx.style()` → `root.style()`; `ctx.screen_rect()` → `ctx.viewport_rect()`;
    `ScrollSource { drag: false }` → `drag: DragScroll::Never`.
- **Verificación**: fmt limpio; clippy `--all-targets -D warnings` limpio; clippy del target Android
  limpio; `cargo test -p pdf_core` 181/0; `check` aarch64 OK; un bench corriendo; `pdf_app`
  construido, arrancado y comprobado **visualmente** (sidebar, cabecera flotante, página renderizada
  y barra de herramientas dibujan bien tras la migración).

## 2026-09-18 — Limpieza de código: −2.619 líneas, `#[allow(dead_code)]` 44→8, `expect()` en producción 5→0

Limpieza orquestada con 6 agentes en paralelo sobre dominios de ficheros disjuntos.
Objetivo: quitar lo que no sirve, lo que miente y lo que incumple las reglas del propio
proyecto. La limpieza **no cambia comportamiento**: 181 tests verdes (antes 193; la
diferencia son los tests del código eliminado).

- **Compositor CPU de anotaciones eliminado** (−1.028 líneas): `pdf_core/src/overlay.rs` (659 l.)
  y `strokecache.rs` (135 l.) con su test de integración. El producto pinta por GPU (pipeline
  dry/wet, ADR-007) y **ni `pdf_android` ni `pdf_app` importaban ese compositor**: solo lo usaban
  tests y benches. `tests/annotations_pipeline.rs` se adaptó (conserva los dos casos que prueban
  el pipeline vivo: extracción → subrayado → sidecar).
- **Crate `pdf_spike` eliminado** (−689 líneas): experimento ya consumido — la predicción vive en
  `pdf_android`, el present EGL en `gpu/`. Fuera del workspace y del APK.
- **Predicción duplicada eliminada** (−238 líneas): `pdf_android/src/prediction.rs` (predictor
  Hermite, ADR-006) quedó superado por `ink/` (Kalman + spring-mass) y estaba bajo
  `#![allow(dead_code)]` a nivel de módulo entero.
- **Apertura sin copia eliminada** (−195 líneas): `jni.rs` `ContentFd` + `open_content_fd`
  (abrir `content://` con fd nativo sin duplicar el fichero). La biblioteca pasó a **curada con
  copia**, así que ese camino no tenía consumidor.
- **Código muerto y cascadas**: 13 items con cero usos (`render_view_menu`,
  `render_settings_menu`, `grid_visible_rows`, `grid_rows_y0`, `grid_total_rows`, `open_picker`,
  constantes y funciones EGL sin uso…) + los huérfanos que su borrado dejó al descubierto
  (`GRID_COLS`, import `log::error`, `PageCache::{get,len,resident_bytes,promote}`, `glScissor`).
  `#[allow(dead_code)]`: **44 → 8**, cada superviviente con justificación escrita y concreta.
- **Violaciones de regla corregidas**: los 5 `expect()` en producción de `pdf_core`
  (`ai.rs:190,209`, `export.rs:90`, `sync.rs:78,124`) sustituidos por caminos sin pánico que
  preservan el comportamiento en el caso normal. `unwrap/expect` en producción de `pdf_core`: **0**.
- **Comentarios: arqueología eliminada y punteros corregidos** (7 ficheros de `pdf_android`,
  `pdf_app`, `pdf_core`, `pdf_bench`): fuera las frases «extraído de `reader.rs`, 2026-09-06,
  Tarea 4.4» y las menciones a fases numeradas muertas (Fase 3.5, Fase 0.5, Fase 1); corregidos
  los punteros a `draw.rs`/`reader.rs`/`gpu.rs` (hoy directorios) y las citas a símbolos que no
  existen (`tool_overlay`, `raster_tool_layer`, `LIBRARY_MAX`, `budget_scale`).
- **CI**: el job `android` gana un paso de `cargo clippy -p pdf_android --target
  aarch64-linux-android --all-targets -- -D warnings`. El `clippy` del job `check` corre solo los
  `default-members` del workspace, así que la app real (excluida de ellos) **nunca se linteaba**:
  un error real (`clippy::option_map_unit_fn` en `reader/redraw.rs`) llevaba tiempo oculto. Ya
  corregido, y el hueco cerrado.
- **Documentación coherente**: `docs/api-anotaciones-fase3.md` deja de documentar la API borrada;
  `docs/plan/{C-pintado,A-latencia,NEXT-PLAN,DEUDA}.md` actualizados (el criterio vivo de C es
  medir 200 trazos en la tablet, `SIN MEDIR`); `benchmark-results.md` conserva su entrada
  histórica con nota de que el código medido ya no existe; skill de rendimiento con 8 benches
  reales (no 9); `ADR-006` anota que su spike fue retirado; nueva deuda `RND-06` (`crop_margins`
  tiene tests pero no está cableado a la UI).
- **Verificación** (2026-09-18, host AMD Ryzen 7 5800H + NDK r28): `cargo fmt --all --check`
  limpio; `cargo clippy --all-targets -- -D warnings` limpio; `cargo clippy -p pdf_android
  --target aarch64-linux-android --all-targets -- -D warnings` limpio; `cargo test -p pdf_core`
  181/0; `cargo check -p pdf_android --target aarch64-linux-android --all-targets` OK.
  Rust total: 41.688 → 39.069 líneas en 96 ficheros (antes 103). **Sin verificación en la tablet**
  (no conectada): los flujos de UI se prueban en la próxima sesión con hardware.

## 2026-09-18 — Auditoría documental: 63 ficheros a 31, −8.259 líneas

Reconstrucción completa de la documentación, orquestada con 7 agentes de reconocimiento y
6 de escritura sobre dominios disjuntos. El repositorio tenía cinco planes compitiendo,
~3.000 líneas de material muerto y contradicciones de integridad graves.

- **Un solo roadmap**: `docs/plan/NEXT-PLAN.md` (fases A–F). Eliminados los competidores,
  entre ellos `docs/PLAN.md` (el documento **más referenciado del repo**, con 41 menciones y
  citado por **17 ficheros de código**) y los dos planes «Pro».
- **Contradicciones registradas, no enterradas**: ADR-001 se apoya en un benchmark que el
  propio repo contradice (el shootout da PDFium ganador en 14/16 pruebas, discrepancia sin
  explicar); ADR-003 declaraba 9 ms/página frente a los 73,6 ms/página de su baseline.
  Ambos conflictos quedan declarados y la decisión del dueño (MuPDF) se mantiene.
- **Huecos rellenados**: `docs/benchmark-results.md` (la evidencia canónica) no mencionaba
  Evince ni poppler; el trabajo de arXiv/Discover (~2.500 líneas en producción) no aparecía en
  ninguna documentación vigente ni en el CHANGELOG.
- **Reglas corregidas** en `AGENTS.md`: fuera comandos imposibles (`-D clippy::unwrap_used`
  daba 39 errores reales en tests), afirmaciones falsas («la APK compila sin claves», cuando
  `include_str!` las exige) y añadida la sección **Trabajo con agentes** (worktrees, fuente de
  verdad `main`, propiedad de fichero, integración), cuya ausencia ya causó que dos ramas
  arreglaran el mismo bug por separado.
- **GitHub**: 6 milestones renombrados del waterfall a las fases A–F; los 6 issues de fases
  muertas cerrados (por primera vez en la historia del repo) y sustituidos por #33–#40 con
  criterio de cierre medible.
- Verificación: cero referencias a ficheros borrados y cero enlaces internos rotos.

## 2026-09-15 — Fix taps tragados y doble pase de página

- Fix en transiciones de página (`gpu/pipeline.rs`, `gpu/dry_key.rs`): en un fallo de caché
  (miss) en pase de página, el bitmap de fallback se subía a GPU bajo la clave de la página
  nueva; al tener idéntica longitud de bytes, la subida de la página real se saltaba
  considerándose ya cargada, congelando la pantalla en la página anterior (el tap parecía
  tragado) y saltando dos páginas en el siguiente tap.
- Solución: subir la textura bajo la clave de la página residente, resetear la clave de la
  dry key en fallback y limpiar el estado de tap ante slop, palma o segundo dedo (`input/motion.rs`).
- Verificación TCL NXTPaper 9469X (2026-09-15, commit `4906161`): pase de página 1:1 verificado
  sin saltos dobles ni taps perdidos (un tap, una página).

## 2026-09-10 — Acceso completo arXiv + Discover tab + handoff Papertok

Pila completa de descubrimiento y lectura de artículos científicos en la tablet (commits
`d991577`..`a970a43`, PR #31/fase F):

- **Motor arXiv (`crates/pdf_core`)**: parser de identificadores arXiv (nuevo formato y legacy)
  con constructor de consultas (`arxiv.rs`); búsqueda en la API de arXiv con parser Atom en
  streaming (`quick-xml`); descarga de PDFs con limitación de tasa (throttle) y resolución de
  nombres vía cabecera `Content-Disposition`.
- **Almacenamiento y preferencias (`pdf_android`)**: persistencia de metadatos en `papers.json`
  (esquema saneado de título e id) y preferencias de Discover (`discover_prefs.json`).
  Preservación de corchetes en resolución de rutas locales y coincidencia por `arxiv_id`.
- **Worker de fondo (`DiscoverWorker`)**: hilo en background con fetch asíncrono y bomba de
  eventos (`DiscoverPump`) procesada en el tick del bucle principal; caché de feed acotada
  ($\le 4$ MiB) con TTL de 10 minutos.
- **Pestaña Discover (`UiMode::Discover`)**: interfaz completa de descubrimiento con feed
  temático, buscador de artículos, ficha detallada con resumen/autores y selector de 45
  categorías arXiv agrupadas por disciplinas. Pulido tipográfico y tarjetas adaptadas al
  look&feel del lector.
- **Handoff bidireccional y Papertok**: integración mediante esquema URI `pdflector://` y
  filtro de intent `ACTION_SEND` (content URI); preservación del estado guardado ante
  destinos de handoff inválidos.
- **Limpieza de UI e invalidaciones GPU**: eliminación de la sección "Continue Reading" del
  carrusel de biblioteca (`draw/library.rs`); invalidación explícita de la caché de texturas
  GPU y del render worker al cambiar de documento (`reader/life.rs`).
- **Verificación**: tests unitarios de arXiv en verde (`cargo test -p pdf_core --test arxiv`,
  740 líneas de tests pasando), build release aarch64 limpio y probado en tablet TCL NXTPaper 9469X.

## 2026-09-10 — Mejoras del visor en tablet: gestos TwoFingerMode, pin de página y trazo uniforme

Integración de las mejoras de interacción táctil y motor de lápiz de la rama review (commits
`c90e550`, `5d9b8e1`, `3747166`, merge `029e36f`):

- **Protección de página visible (`PageCache`)**: `set_protected` fija la página actual en
  `goto_page`, `open` y `restore`; la evicción LRU la salta y se detiene si solo queda ella.
  `goto_page` descarta fallbacks obsoletos en acierto de caché: volver atrás ya no muestra
  otra página bajo el indicador correcto.
- **Máquina de estados TwoFingerMode (`input/motion.rs`)**: gestos a dos dedos clasificados
  entre `Undecided`, `Pan` y `Zoom` con coherencia de dirección, umbrales proporcionales e
  histéresis. Arrastre paralelo traslada 1:1 (incluso a zoom 1 para el recorte cover) con 0
  zoom fantasma; el pellizco mantiene la escala relativa exacta al punto de inicio.
- **Pan clampado y blit residente**: cálculo de escala `entry_blit_zoom` derivado del propio
  bitmap residente para eliminar saltos (+435 px) al soltar el pellizco; pan horneado en la
  capa dry (`DryKey`) a zoom 1.
- **Trazo de tinta uniforme y Suavizado A (`reader/tools.rs`, `gpu/pipeline.rs`)**: dibujo con
  ancho constante de base en vuelo (sin efecto fuelle ni ensanchamiento al soltar) y discos de
  unión solo en extremos y esquinas reales. Suavizado A con subdivisión Bézier adaptativa a la
  curvatura, punta húmeda curvada hacia el punto Kalman y settle en punto medio ($\\epsilon=0.20$).
- **Subrayado en orden de lectura y borrado rápido (`selection.rs`, `reader/tools.rs`)**:
  el resaltador con lápiz rellena texto en orden de lectura continuo (de pen-down al final de
  la primera línea, líneas intermedias completas, y de inicio a pen-up en la última). El
  borrador sobre un highlight elimina la anotación completa al contacto; borrado de trazos con
  poda por AABB, invalidación por corte y refresco inmediato.
- **Verificación TCL NXTPaper 9469X (2026-09-10, `docs/benchmark-results.md`)**: pase de página
  en 5-9 ms, gestos de pan/zoom fluidos sin pánicos, 171 tests de `pdf_core` pasando limpios.

## 2026-09-07 — Velocidad de pase de página (fases A-D + guards)

- Fix corrección: la página real no se mostraba tras un miss (dry horneada con fallback
  sin re-bake; `invalidate_dry` era dead code). Ahora `poll_render` invalida al aterrizar.
- `save_state` diferido (0 I/O en el tap; flush 2 s + transiciones + Pause). `count_for_page`
  O(1) con TDD. Instrumentación permanente `page_turn <ms>`.
- Prefetch direccional asimétrico (2 por delante, 1 por detrás, preemption por lote).
- Fix raíz: crop centrado a píxeles de ventana en el worker (bitmaps 27.4→12.7 MB;
  `CachedPage` con metadatos; overlays anclados a caja full) + evicción diferida a ticks idle.
- Guards anti-negro (fallback verificado + bitmap degenerado descartado, con warns).
- Medición TCL 9469X (2026-09-07, `docs/benchmark-results.md`): 11/15 turnos a 6-10 ms
  (p50 ≈ 8 ms; antes: 115 ms sistemáticos); misses a 102-168 ms (un render); nitidez 1:1
  verificada visualmente. Deuda: display lists sin cota (+6-7 MB/página nueva en libro
  complejo) y PSS pico post-ciclos.

## 2026-09-06 — Reestructuración Fase 2 completa: pipeline Dry/Overlays + productor único EGL

- GPU: `DryKey` reducida a `{page, zoom_bits, ann_count, dark}` (pan/chrome/sheet/toast ya no
  invalidan la capa base); overlays de UI a fb0 tras componer dry⊕wet; pan aplicado al quad
  (con fix de doble-pan y clear de fb0); `ovl_cache` por id estable con LRU de bytes (fix ABA);
  wet guard con selección visible; logging de ciclo de vida EGL con contadores.
- Fix raíz `EGL_BAD_ALLOC` 0x3003 Library→Viewer: una `ANativeWindow` admite un solo productor
  de BufferQueue; productor único GPU (Library/Picker se presentan por el pipeline GL; surface
  EGL persistente, `ANativeWindow_lock` solo como fallback sin GPU).
- Verificación TCL 9469X (2026-09-06, build `f5381e9`, pantalla ON; `docs/benchmark-results.md`):
  10 ciclos Library→Viewer con 0×0x3003 (antes: 7/10 fallos); pan con stylus p95 4.19 ms sobre
  459 presents con 0 re-renders de la dry (objetivo p95 <16.6 ms); PSS 118 MB arranque,
  174-178 MB reposo — pico 232 MB tras ciclos: deuda registrada.

## 2026-09-06 — Reestructuración Fase 4: splits por responsabilidad + LibraryState + limpieza

- 4.3 `input.rs` → `input/{gestos,motion,dispatch,stylus}` (pure move).
- 4.1 `gpu/mod.rs` → `gpu/{ffi,shaders,surface,textures,pipeline}` (pure move).
- 4.4 `reader.rs` → 12 submódulos por responsabilidad en `reader/` (pure move).
- 4.2 `draw.rs` → 7 submódulos por responsabilidad en `draw/` (pure move).
- 4.5 `LibraryState` extraído de `Reader`: 19 campos `lib_*` (scrolls px, filtros,
  sort, progreso, planos cacheados, fade) viven ahora en
  `reader/library_state.rs`; los accesos usan `self.library.lib_x`.
- 4.6 limpieza integral: 40 imports muertos retirados (39 en `reader/*` vía
  `cargo fix` + 1 manual), 4 `#[allow(dead_code)]` huérfanos quitados (ítems
  usados), doc comments de `lib_scroll` → `library.lib_scroll` (Tarea 4.5),
  AGENTS.md: fila CI con "job Android en CI (sin TCL)", `cargo fmt` en los 6
  ficheros con deriva local. Los `#[allow(dead_code)]` restantes (43) protegen
  API de fases futuras, UI oculta por diseño o ítems superados-documentados —
  inventario por caso en el historial git de la reestructuración (commit `c236c27`).
- Verificación: `cargo check -p pdf_android --target aarch64-linux-android`
  0 errores / 0 warnings; `cargo test -p pdf_core` 161/0; `cargo clippy
  --all-targets -- -D warnings` verde; `cargo fmt --all -- --check` verde;
  `wc -l` de `pdf_android/src` < 2000 en todos los ficheros; 43 allows
  `dead_code` (todos justificados).

## 2026-09-06 — CI Android: job `android` (aarch64-linux-android, NDK r28/API 35)

- Nuevo job paralelo `android` en `.github/workflows/ci.yml`: toolchain rustup
  stable con target `aarch64-linux-android` + NDK r28 (API 35,
  `android-actions/setup-android-ndk`), `cargo check -p pdf_android --target
  aarch64-linux-android` con cache `Swatinem/rust-cache` (workspaces
  ". -> target"). Crea in-situ los placeholders gitignored
  `groq_key.txt`/`google_key.txt` (include_str!) y exporta sysroot/PATH del NDK
  (`ANDROID_NDK_HOME` + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`).
- AGENTS.md: el carril "Compilación cruzada Android" de la tabla de validación
  pasa de "Dev local con NDK, no en CI actual" a "Dev local con NDK + CI
  (job `android`)".
- Verificación: YAML validado localmente (python yaml.safe_load, exit 0);
  el job aún no ha corrido — V3.1 (workflow run real) y V3.2 (prueba de valor
  con rama que rompe) requieren push a remoto, diferidas a decisión del usuario.

## 2026-09-06 — Reestructuración Fase 1 completa: docs & gobernanza

- Lote 1 (1.0–1.4, correcciones puntuales): CHANGELOG completo (G1, era
  "últimas 5" con 19 reales); AGENTS.md sin límite-50 (E4); PROYECTO.md sin
  borrado-auto; 00-objetivo/COMPETENCIA/D-ia corregidos.
- Lote 2 (1.5–1.9, reescrituras mayores): NEXT-PLAN con presupuesto PSS real
  (52.9/105/208MB) + deuda transversal; docs/README.md índice maestro (G3);
  24 históricos congelados con banner; README/CONTRIBUTING/PR-template con
  verificación Android local; ADR-007 con estado normalizado (supersede
  parcial de ADR-006 present).
- Verificación: greps V1.1-V1.4 en 0 hits; índice docs/README.md completo (find
  vs listado, diferencia vacía); ADRs con estado normalizado.

## 2026-09-05 — Eliminada la barra de herramientas del visor

Fuera la píldora Resaltar/Boli/↶/●/━/→ y todo su cableado (`toolbar_tap`,
`set_tool`, `toggle/close_toolbar`, `cycle_ink_*`, `undo_last_annotation`,
`render_toolbar`, campos `toolbar_open/bitmap`, `DryKey.toolbar_open`,
constantes `STROKE_WIDTHS/INK_PALETTE`, `save_tool_state`). El lápiz dibuja
según `pen_mode` persistido (botón lateral alterna Ink↔Highlight); el dedo
navega. Sin regresión compilada (clippy aarch64 limpio); verificación en TCL
pendiente en esta misma sesión.

## 2026-09-04 — Fix freeze visor→biblioteca; retorno documentado como bloqueado

`enter_library`/`open_picker` sueltan la surface EGL (`drop_surface`) y la
biblioteca vuelve a dibujar (0 `ANativeWindow_lock failed` en TCL 9469X).
Corrección al registro del 2026-08-28: el `recreate` de vuelta falla con
`EGL_BAD_ALLOC` (0x3003) tras uso CPU de la ventana — verificado con
`eglGetError`, ni geometría ni reintentos lo resuelven. Retorno
Library→Viewer queda como tarea de arquitectura (single-producer EGL);
mientras tanto, el visor funciona íntegro desde arranque fresco (vía de
validación B4 con stylus).

## 2026-09-03 — Integración zoom + UI/UX sobre main (ADR-008)

Merge `ui_ux` (tokens RICOUI en `pdf_core::theme` + shell inmersivo egui) y port
F3.1/F3.2/F3.3 de `mejora_zoom` (worker persistente, debounce 350 ms, display
lists). F2 GLES3 y F1 desktop descartados (superseded, ver ADR-008). TCL 9469X:
APK release arranca en 361 ms sin FATAL, PSS 105 MB tras abrir 500 págs (+5 MB
vs base). `pdf_core` 151/151; clippy lib limpio.

## 2026-08-28 — Fase 2 (ADR-006): presentación del visor con EGL/GLES2

Cutover del modo Viewer a `eglSwapBuffers` (`gpu.rs`, FFI EGL/GLES2 propio, sin crates
nuevas): página como textura perezosa, tinta como TRIANGLE_STRIP con la misma polilínea
midpoint que se persiste, overlays como quads de bitmaps Canvas+JNI, dark mode como
uniform. Library/Picker siguen en SW; Viewer↔Library hace drop/recreate surface con el
contexto EGL vivo. ~1700 líneas de rasterización muerta eliminadas (draw.rs/reader.rs).
pdf_core intacto (70/70); clippy `-D warnings -D clippy::unwrap_used` verde; PSS TCL
52.9 MB. Medición p50/p95 del present en gesto pendiente (tablet con keyguard; ver
`docs/benchmark-results.md`).

## 2026-08-26 — Tarea 3/3: Library — Contenido SettingsMenu (☰) + SettingsModel

Implementación completa de las 5 secciones del SettingsMenu desplegable (☰) estilo Readest:

- **Dropdown SettingsMenu flotante** (`draw.rs:render_settings_menu`/`draw_settings_menu`): tarjeta anclada al borde derecho de `settings_menu_button_rect` (`dropdown-end`), con sombra multinivel (`shadow-2xl`), esquinas redondeadas 16px, borde `base-300` y fondo `base-100`.
- **Sección LECTURA RECIENTE**: toggle "Mostrar lectura reciente" que sincroniza en tiempo real `recent_shelf_enabled`, mostrando/ocultando el carrusel superior y ajustando el offset vertical de la rejilla.
- **Sección TAMAÑO DE PORTADA**: 3 opciones con checkmark ("Pequeño (Small)", "Mediano (Medium)", "Grande (Large)") que ajustan `cover_size` (0/1/2) aplicando multiplicadores 0.85 / 1.0 / 1.15 tanto en cuadrícula como en filas de lista, recalculando alturas de celda y escalado de portadas.
- **Sección PROGRESO**: toggle "Mostrar porcentaje de lectura" (`cover_progress`) que muestra un badge flotante redondeado (radio 999, fondo `primary`, texto `primary_content`) con el porcentaje leído (p. ej. "67%") en la esquina inferior derecha de cada portada en rejilla y lista. Libros no leídos (0%) se omiten.
- **Sección ADMINISTRAR BIBLIOTECA**: botón "Vaciar biblioteca" con confirmación interactiva en 2 pasos y timeout de 3 segundos ("¿Vaciar? Toca de nuevo para confirmar"). Elimina `library.json` y los PDFs en el almacenamiento interno de la app, invoca `reload_curated_library(app)` mostrando el estado vacío ("Tu biblioteca está vacía") y lanza toast de confirmación "Library cleared".
- **Sección ACERCA DE**: fila informativa no interactiva con nombre de la app (`PDFLector`), versión del paquete (`env!("CARGO_PKG_VERSION")`, v0.1.0) y licencia ("Open source · AGPL-3.0").
- **Persistencia**: campos `cover_size: u8` (default 1) y `cover_progress: bool` (default false) integrados en `ViewerState` con `#[serde(default)]`, persistiendo y recuperando el estado entre sesiones.

## 2026-08-26 — Tarea 2/3: Library — Contenido ViewMenu (⋯) — Grid/List, Columns, Covers, Group/Sort

Implementación completa de las 6 secciones del ViewMenu desplegable (⋯) estilo Readest:

- **Dropdown ViewMenu flotante** (`draw.rs:render_view_menu`/`draw_view_menu`): tarjeta anclada al borde derecho (`dropdown-end`) con sombra multinivel (`shadow-2xl`), esquinas redondeadas 16px, borde `base-300` y fondo `base-100`. Se dibuja directamente sobre la cabecera fija y se compone en blit con alpha blending.
- **Sección ViewMode (Grid ↔ List)**: opciones "Cuadrícula (Grid)" y "Lista (List)" con checkmark en la opción activa; tap cambia `view_mode`, persiste en `state.json` (`save_state`), invalida con `list_dirty = true` y cierra el menú. Modo Lista muestra tarjetas de ancho completo (1 columna) con portada miniatura, título, formato/autor y barra de progreso.
- **Sección Columns (Auto + Stepper)**: botón "Auto" con estado activo/inactivo (`auto_columns`) + stepper "− N +" para columnas 1..4 (deshabilitado visualmente cuando Auto=true; al pulsar − o + desactiva Auto y fija el valor exacto). Rejilla adapta dinámicamente el ancho de celda y portada a `effective_grid_cols()`.
- **Sección Book Covers (Crop ↔ Fit + Hide)**: opciones "Recortar (Crop)", "Completa (Fit)" y "Ocultar portadas". Crop escala llenando marco 2:3; Fit escala manteniendo relación de aspecto con letterbox `base-200`; Hide oculta las portadas mostrando tarjetas compactas solo con metadatos.
- **Sección Destacados (Show recently read)**: toggle "Mostrar lectura reciente" que muestra/oculta el bloque Continue Reading según `recent_shelf_enabled`.
- **Secciones Agrupar por y Ordenar por**: opciones "Ninguno (Libros)" / "Autor" y "Título" / "Autor" / "Fecha añadido" / "Última lectura" / "Progreso" (con checkmark en la opción activa). "Progreso" ordena primero los libros con mayor porcentaje leído (`pct()`). Tap reordena `lib_filtered` y refresca la vista.

## 2026-08-25 — Tarea 1/3: esqueleto del header de biblioteca (⋯ View + ☰ Settings)

Estructura de los menús de cabecera estilo Readest (sin contenido todavía,
Tareas 2-3):

- **Modelo**: enums `LibraryViewMode {Grid,List}`, `LibraryCoverFit {Crop,Fit}`
  y `LibraryGroupBy {None,Author}` + 8 campos en `Reader` con defaults
  (`view_mode`, `cover_fit`, `auto_columns`, `columns=3`, `view_menu_open`,
  `settings_menu_open`, `hide_covers`, `recent_shelf_enabled`) + helper
  `is_grid()`.
- **Geometría**: `view_menu_button_rect`/`settings_menu_button_rect` —
  círculos ~32 dp (≈65 px) con 8 dp de separación, alineados a la izquierda
  del "＋ Añadir" (anclaje a derecha con `grid_pad`), centrados en el Y de la
  fila de botones (`lib_header_buttons_cy`); los dropdowns futuros cuelgan
  del borde derecho (`dropdown-end`).
- **Render** (`render_library_header`): dos círculos con glifos ⋯ y ☰ (14 sp,
  `base_content`); el botón con su dropdown abierto se marca en `primary`
  (highlight).
- **Input** (`library_tap`): tap en ⋯ alterna `view_menu_open` (cierra
  settings); tap en ☰ alterna `settings_menu_open` (cierra view); tap FUERA
  de ambos cierra los dos (sin disparar otra acción); `list_dirty`+`redraw()`
  para re-renderizar la cabecera cacheada.
- **Persistencia** (`ViewerState`): 5 campos nuevos con `#[serde(default)]`
  (view_mode/cover_fit/columns/hide_covers/recent_shelf_enabled; defaults de
  arranque con funciones para columns=3 y shelf=true) — `state.json` viejo
  carga sin romper (verificado: fichero sin campos → defaults) y `save_state`
  los persiste (verificado con `view_mode=List` sobreviviendo a reinicios).

## 2026-08-25 — Control total de anotaciones con el BOLI (sin menús)

Toda la anotación se controla solo con el boli (S-Pen/Saber, TCL 9469X):

- **A) Tocar con el boli SIEMPRE dibuja**: el boli pasa a dibujar (Ink) o
  subrayar (Highlight) según el MODO actual sin depender de la barra de
  herramientas; el dedo sigue navegando (tap página, pinch zoom — nunca
  dibuja).
- **B) Botón UP del boli (el SUPERIOR) = alternar modo** con toast
  ("✏️ Pen" / "🖍️ Highlighter"). Calibrado en fase A: este boli reporta el botón
  superior como `STYLUS_SECONDARY` (0x40) — INVERTIDO al estándar; las
  constantes `PEN_BTN_MODE`/`PEN_BTN_ERASE` concentran la asignación. El
  toggle funciona en CONTACTO; en el AIRE el "stylus-handwriting-event-
  receiver" de Android 15 (window del sistema uid 1000, `INTERCEPTS_STYLUS`)
  consume el hover del stylus — limitación del sistema, documentada en
  `input.rs`/`annotations.rs`.
- **C) Botón DOWN (el INFERIOR) + apoyar = GOMA REAL**: recorte parcial en
  vivo (un trazo se PARTE en trozos por el barrido de la goma; un subrayado
  se divide en rectos) con `pdf_core::{split_stroke,trim_highlight}` (puras,
  con tests) + barrido continuo entre pasadas (`erase_last`). Se guarda una
  sola vez al levantar; no entra en el undo (permanente).
- **D) El modo se persiste** en `tool_state.json` (campo `mode`, cargado con
  fallback `Ink` por retrocompatibilidad — `persist.rs` NO se tocó: el JSON
  se lee/escribe como `Value` conservando ink_color/ink_width).
- **E) Barra intacta y funcional** (Resaltar/Boli/↶/●/━/→); elegir Boli/
  Resaltar en la barra sincroniza el modo persistido del boli.
- **Fixes de la verificación en tablet**: (1) el dedo volvió a pasar página
  (el Down reestructurado hacía pan a todo contacto — ahora solo con
  herramienta activa); (2) el resaltador dibuja un RECT LIBRE (bbox del
  trazo, altura de línea 13 pt) cuando no hay texto bajo el trazo — el boli
  SIEMPRE pinta en modo Highlighter.
- **Pulido (2ª iteración)**: (1) INDICADOR DE MODO en el visor: pill
  minimalista abajo a la derecha con ✏️/🖍️ (`draw::render_mode_badge`, cacheada
  en `Reader::mode_badge`, se refresca al alternar modo) — el usuario SIEMPRE
  ve con qué va a dibujar; (2) el borrado del SUBRAYADO ahora usa barrido
  (muestreo del segmento goma anterior→actual en `trim_highlight`): una goma
  rápida ya no "salta" por encima de una línea de 13 pt sin recortarla.
- **Pulido (3ª iteración, feedback del usuario)**: (1) ICONOS MINIMALISTAS
  lineales (estilo del pack "minimalista" de IconScout, sin emojis): el boli
  es una PLUMA diagonal dibujada con el rasterizador de tinta (cuerpo ancho +
  punta fina) y el resaltador es el símbolo de TEXT0 SUBRAYADO (barra +
  patillas finas); (2) BORRADO CON GOMA REAL MEJORADA: `split_stroke` recorta
  los SEGMENTOS por intersección exacta círculo→segmento (el hueco es
  exactamente el círculo de la goma, no un "mordisco" de vértices) con
  barrido multi-círculo muestreado; (3) CURSOR DE GOMA visible durante el
  borrado (círculo del tamaño real de la goma que sigue al boli) — el usuario
  ve exactamente qué se va a borrar.

## 2026-08-25 — Buscador con TECLADO real (sin chips A-Z) + biblioteca minimalista

El buscador de la biblioteca pasa de chips de letra/carpeta a TEXTO LIBRE con
el teclado del sistema, y la biblioteca queda en solo libros + buscador
(estilo Readest):

- **Teclado del sistema en NativeActivity** (`jni.rs` + `tools/ime/`): el
  backend native-activity de android-activity 0.6 no entrega texto por la vía
  nativa (set_text_input_state es NOP; TextEvent solo en game-activity, que
  exige Activity Java). Vía implementada: helper Java MÍNIMO
  (`tools/ime/ImeHelper.java`) compilado a dex con las tools del SDK
  (`tools/ime/build.sh`, javac + d8 + android.jar), EMBEBIDO en el binario
  (`include_bytes!`) y cargado en runtime con `DexClassLoader` desde
  `files/ime/` (el dex se marca 0444: ART rechaza dexes writable). El helper
  crea un EditText invisible en el hilo UI (`runOnUiThread`) y abre el IME;
  Rust hace polling del texto en `tick` (`needs_tick` + ime_active).
- **UI del buscador**: tocar "Buscar por título o carpeta..." abre el teclado;
  el texto filtra la rejilla por subcadena case-insensitive MIENTRAS se
  escribe; "✕" limpia y cierra el teclado; sin coincidencias → "Sin
  resultados — toca ✕ para limpiar". Eliminados de la UI los chips A-Z y de
  carpetas (código conservado, inactivo).
- **Fix**: abrir un libro curado desde la rejilla (la uri es la RUTA LOCAL;
  antes intentaba copiarla como content:// → "No content provider").

## 2026-08-25 — Biblioteca minimalista estilo Readest (rejilla + buscador)

La biblioteca se simplifica a SOLO libros + buscador, siguiendo el lenguaje
visual de Readest:

- **Se eliminaron de la UI la sección "Continue Reading" (recientes/leyendo
  ahora) y los chips de ORGANIZACIÓN (sort "Recientes/Los leídos/Título/Autor"
  y filtro "Todos/En lectura/Terminados/Por leer")** (`draw.rs`, `reader.rs`, `input.rs`):
  la rejilla de portadas empieza justo bajo el campo de búsqueda
  (`lib_grid_y0` = 8 px), sin títulos de sección intermedios.
- El **buscador** se mantiene como único filtro: campo "Buscar por título o
  carpeta..." + panel de chips de letra A-Z / carpetas (sin teclado, limitación
  de native-activity documentada).
- El orden de la rejilla queda FIJO por "añadido más reciente primero"
  (`LibSort::RecentlyAdded` por defecto); los libros curados aparecen arriba.
- Código del carousel/organización conservado con `#[allow(dead_code)]`
  documentado (no se borra lógica, solo se desactiva de la UI).

## 2026-08-25 — Biblioteca curada: altas por selector + tope fijo LRU (50)

La biblioteca deja de ser un escaneo de MediaStore y pasa a ser CURADA:

- **Arranque sin intent sin escaneo** (`reader.rs`, `lib.rs`): `Reader::new`
  ya no llama a `rescan_library`; muestra SOLO lo registrado en
  `internal/library.json` (`reload_curated_library`). Vacía → empty state
  "Tu biblioteca está vacía + Añadir PDF". `Resume` tampoco re-consulta
  MediaStore. Migración one-shot: instalaciones antiguas con PDFs en
  `internal/pdfs/` y registro vacío se auto-importan (added = ahora).
- **Selector "＋ Añadir" GESTOR DE ARCHIVOS**: el botón de cabecera consulta
  MediaStore en una lista TEMPORAL (`select_list`, nunca `library_list`) y
  abre un navegador por CARPETAS (árbol de `RELATIVE_PATH`): las carpetas
  del nivel actual primero (📁, en `primary`), los PDFs después (📄), con
  barra de breadcrumb "⬆ …" para subir de nivel y "Atrás" para cancelar.
  Se evita así la lista plana inabarcable de MediaStore. Tap en un PDF =
  copiar a `internal/pdfs/` + contar páginas (MuPDF) + registrar progreso
  (tope LRU aplicado).
- **Tope fijo con evicción LRU** (`persist.rs`): `LIBRARY_MAX = 50`;
  `enforce_library_limit(books, max)` (pura, con tests) devuelve las
  expulsadas del menos recientemente leído al menos (nunca leído →
  ordena por `added_unix`). Al añadir el 51º se borra fichero + portada
  cacheada del expulsado y se muestra toast "Biblioteca llena — se eliminó X".
- **Portadas por ruta local**: las entradas curadas usan su ruta local como
  clave de portada; el pump renderiza por ruta (`render_thumb_path`) en vez
  de fd content://. `ThumbCache::remove(key)` nuevo para purgar la portada
  de un libro evictado (sin tocar la política LRU existente).
- **Eliminado `rescan_library`**: la rejilla jamás lista PDFs del sistema por
  sí sola ([7] verificado con 200 PDFs en Download — solo aparecen curados).

## 2026-08-25 — Iteración 3 — cierre de defectos visuales

Cierre de defectos visuales, bugs residuales y pulido perceptual en `crates/pdf_android`:

- **Bugs corregidos**:
  - **[A1] Eliminación del botón fantasma ✏️**: Eliminado el renderizado residual de `tool_fab` y su captura de toques `tool_fab_tap` en el visor, dejando la esquina superior derecha completamente limpia.
  - **[A2] Swatch de tema en Top Bar**: Incorporado padding interno $\ge 18$ px en `viewer_top_chrome_buttons`, garantizando que el swatch circular quede 100% dentro de la tarjeta flotante sin cortar el radio de curvatura.
  - **[A3] Paleta Default-Dark & Sepia-Dark**: Alineados los valores derivados de Readest en `Default-Dark` (`#141414` base-200, `#242424` base-100, `#77BBEE` primary) y `Sepia-Dark` (`#201B15` base-200, `#342E25` base-100, `#48D1CC` primary).
- **Pulido perceptual**:
  - **[B1] Barra de progreso**: Implementado track de ancho completo en `base-300` con fill `primary` redondeado y separación vertical $\ge 8$ px del texto de autor (eliminado el aspecto de enlace subrayado en rejilla y Continue Reading).
  - **[B2] Sheet sin espacio muerto**: Altura de panel ajustada al 42% de `win_h` y distribución vertical uniforme (`space-evenly`) de las 3 secciones (`TEMA`, `LECTURA`, `DOCUMENTO`).
  - **[B3] Contraste de tarjetas en dark mode**: Delta de luminosidad perceptible de +10% en superficies `base-100` frente al fondo `base-200`, reforzado con borde 1px `base-300` y sombra multicapa `0x70000000`.
  - **[B4] Swatches informativos en selector de temas**: Círculos de previsualización que muestran el color real de fondo del tema (`#FFFFFF`, `#F1E8D0`, `#242424`, `#342E25`) con anillo `primary` de 2px en el tema activo.
- **Rendimiento**: Scroll de biblioteca mantenido en 3.9 ms – 6.5 ms por frame blit (muy inferior al límite de 33 ms).
- **Evidencias visuales**: Generadas 6 capturas en `/tmp/`: `pl3-biblio-light.png`, `pl3-biblio-default-dark.png`, `pl3-biblio-sepia-dark.png`, `pl3-viewer-chrome-light.png`, `pl3-sheet-light.png`, `pl3-sheet-dark.png`.

## 2026-08-25 — Iteración 2 — fidelidad visual

Pasada exhaustiva de fidelidad visual y calidad perceptual sobre `crates/pdf_android` adaptando al detalle el lenguaje visual de Readest, con superficies separadas, elevación con sombras multinivel, marcos con contención 2:3 y barras flotantes:

- **Biblioteca (`draw.rs`, `reader.rs`, `lib.rs`)**:
  - **B1 Superficies separadas**: Fondo de biblioteca en `base-200` (`#F2F2F2` en light, `#141414` en dark); tarjetas, barras y filas en `base-100` (`#FFFFFF` en light, `#1C1C1C` en dark) con sombras de elevación y bordes de 1px `base-300`.
  - **B2 Portadas en marco 2:3**: Renderizado `CONTAIN` centrado dentro del marco 2:3 con padding interno de 8px, bordes de papel y sombreado de lomo, evitando el corte/sangre total de celda.
  - **B3 Rejilla con respiro**: Gutter horizontal $\ge 3.5\%$ de `win_w` (50px en 1440w), títulos en 14sp peso 600 `base-content` bajo el marco, autor en 12sp `neutral-content`, y barra de progreso de 4px en `base-300` con fill `primary`.
  - **B4 Continue Reading rediseñado**: Tarjetas horizontales (alto 15% `win_h`, ancho 52% `win_w`) con marco 2:3 a la izquierda, título en 17sp bold `base-content`, autor en 12sp `neutral-content`, progreso separado de 4px, metadatos y botón de píldora `"Continuar"` relleno en `primary` con texto de alto contraste.
  - **B5 Chips de Búsqueda y Organización**: Píldoras con altura $\ge 40$ px (44px), inactivas con fondo `base-100` y borde `base-300`, activas con fondo `primary` y texto en contraste.
- **Visor (`draw.rs`, `reader.rs`, `input.rs`)**:
  - **V1 Barras Flotantes**: Top bar y Bottom bar como tarjetas flotantes con radio de 24px, sombra de elevación multinivel y márgenes $\ge 2\%$ `win_w` respecto a los bordes superior, inferior y laterales.
  - **V2 Botones Táctiles**: Áreas táctiles de todos los controles $\ge 48\times 48$ px con glifos claros e interactivos.
  - **V3 Barra Inferior con Slider**: Tarjeta flotante con texto `"Pág. N de M · P%"` en 13sp `base-content`, pista de 6px radio completo en `base-300`, fill `primary`, y thumb circular de 22px `primary` con borde de 2px `base-100`.
  - **V4 Top Bar**: Píldora `"← Biblioteca"` ($\ge 48$ px alto), título del libro en 16sp peso 600 `base-content`, y swatch circular de 26px del color `primary` activo con anillo interactivo de selección de tema.
- **Sheet de Ajustes (`draw.rs`, `reader.rs`, `input.rs`)**:
  - **S1 Panel Real**: Panel con altura 55-60% de `win_h` (~1276px en 2200h), radio de 24px en el borde inferior y sombra proyectada.
  - **S2 Secciones Estructuradas**: Encabezados de sección en 11sp mayúsculas `neutral-content` (`TEMA`, `LECTURA`, `DOCUMENTO`), fila de 4 swatches circulares de 26px (`Claro`, `Sepia`, `Oscuro`, `Sepia D.`) con anillo activo de 2px `primary`, fila de navegación (`◀ −10`, `Pág. N / Total`, `+10 ▶`), y fila de acciones (`← Biblioteca`, `🔍 Buscar`, `✕ Cerrar`).
  - **S3 Dimensiones de Botones**: Todos los botones del sheet con altura $\ge 48$ px.
- **Global (G1-G3)**:
  - **G1**: Sombras multinivel visibles en temas claros y oscuros (alpha $\ge 0x60$ + borde `base-300`).
  - **G2**: Tipografía $\ge 12$ sp en todo el texto informativo (únicamente labels en mayúsculas a 11sp).
  - **G3**: Contraste garantizado con texto informativo en `base-content`.
- **Rendimiento**: Frame time en scroll de biblioteca entre 3.8 ms y 6.1 ms (muy inferior a 33 ms).
- **Evidencias visuales**: 6 capturas generadas en `/tmp/`: `pl2-biblio-light.png`, `pl2-biblio-dark.png`, `pl2-viewer-chrome-light.png`, `pl2-viewer-chrome-dark.png`, `pl2-sheet-light.png`, `pl2-biblio-sepia.png`.

## 2026-08-24 — Restyling visual de PDFLector (Android) al look&feel de Readest

Reemplazo completo del sistema visual en `crates/pdf_android` adoptando los principios de diseño, paletas y jerarquía tipográfica de Readest (https://github.com/readest/readest).

- **Sistema de Temas (`lib.rs` `mod theme`)**: Implementados 4 temas completos derivados de la fórmula Readest (`base-100`, `base-200`, `base-300`, `base-content`, `neutral`, `neutral-content`, `primary`, `primary-content`): `Default-Light`, `Sepia-Light`, `Default-Dark`, `Sepia-Dark`. Los 4 temas son ciclables de forma interactiva y persistentes en `ViewerState`. Se eliminaron todos los literales de color hardcodeados fuera de `mod theme` (0 ocurrencias de `0x[0-9A-Fa-f]{8}` fuera de `mod theme`).
- **Chrome del Visor y Navegación (`reader.rs`, `draw.rs`, `input.rs`)**:
  - Taps por tercios horizontales: tercio izquierdo = página anterior, tercio derecho = página siguiente, tercio central = alternar chrome.
  - Barra superior fina: botón estilo píldora `← Biblioteca`, título del documento truncado con elipsis y botón de tema actual (`Default-Light`, `Sepia-Light`, etc.).
  - Barra inferior de progreso: pista estilizada de 3px con relleno de color `primary` y etiqueta `"Página N de M · P %"` en tipografía 12sp `neutral-content`. Auto-ocultado automático tras ≤2.5s de inactividad.
  - Indicador badge mínimo (`N / M`) visible únicamente cuando el chrome está oculto.
- **Sheet de Ajustes (`draw.rs` `render_sheet`)**: Rediseñado con tokens Readest, esquinas redondeadas (16px), título de sección en 11sp mayúsculas (`FONT_LABEL_CAPS`) y control de tema activo destacado en color `primary`.
- **Biblioteca y Componentes (`draw.rs`)**: Biblioteca personal (cabecera, barra de búsqueda, sección "Leyendo ahora", carrusel, chips de ordenación y filtrado, rejilla "Tu Colección", empty state), toasts, FAB y menús adaptados a los tokens y jerarquía tipográfica Readest (`FONT_DISPLAY` 24sp, `FONT_TITLE` 17sp, `FONT_BODY` 14sp, `FONT_CAPTION` 12sp, `FONT_LABEL_CAPS` 11sp).
- **Archivos editados**: `crates/pdf_android/src/{lib.rs, draw.rs, input.rs, reader.rs, persist.rs}`.
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` (0 warnings), `cargo clippy -p pdf_android --target aarch64-linux-android --release -- -D warnings` (limpio), `cargo fmt --all -- --check` (limpio), `cargo check -p pdf_core -p pdf_app -p pdf_bench` (limpio). APK instalado y probado en TCL 9469X con 5 capturas verificadas en `/tmp/`.

## 2026-08-22 — Fase 3.5: anotaciones con lápiz (resaltado + boli) y auditoría del pipeline de render en tablet

Trabajo completo en el worktree de la rama `marakihau` (sin commits hasta esta sesión, en la que el autor pidió commitear). Ver `docs/api-anotaciones-fase3.md` (API de motor) y `docs/benchmark-results.md` (auditoría completa).

- **`pdf_core` — API de anotaciones (lógica pura, sin UI, en coords de página)**:
  - `selection.rs` (nuevo): resaltador con detección de texto — `highlight_under_gesture(spans, gesture, color)` selecciona las líneas (`TextSpan`) cuyo bbox intersecta el gesto (puntos o marquee) y recorta el rect al tramo horizontal del trazo.
  - `overlay.rs` (nuevo): composición de la capa de anotaciones sobre la página.
  - `annotations.rs`: `smooth_polyline` (Catmull-Rom, función pura, pasa por los vértices, preserve endpoints) + tests.
  - `prefetch.rs` + tests ampliados (`prefetch.rs`, `annotations_pipeline.rs`).
- **`pdf_android` — UI de anotación y rendimiento de biblioteca**:
  - Barra de herramientas flotante (píldora centrada arriba): Resaltar / Boli / ↶ deshacer / ● color / → volver a navegación; botón flotante ✎/✕; ocultar la barra vuelve a modo NAVEGACIÓN (decisión documentada).
  - Gestos de herramienta: un dedo o lápiz dibuja en vez de navegar (`begin/update/end_tool_gesture`), umbral `TOOL_MIN_PX = 6` descarta toques sin intención; segundo dedo cancela el gesto; long-press de selección de texto desactivado con herramienta activa.
  - Rendimiento de la biblioteca: scroll por bandas cacheadas (`lib_header` + `lib_band`, blit por filas ~1-3 ms) en vez de re-renderizar toda la pantalla por Canvas+JNI (~20-60 ms → el lag/parpadeo reportado); transición `lib_fade` (0,18 s) al abrir libro; `TOOL_MIN_PX`.
- **Benchmarks (`pdf_bench`)**: benches nuevos `blit.rs` (espejo de las primitivas CPU de `draw.rs`; pdf_android no compila en host, hay que mantenerlo en sync) y `prefetch.rs`. Auditoría completa en `docs/benchmark-results.md`.
- **Mediciones (2026-08-22)**: hardware AMD Ryzen 7 5800H (16 hilos), Rust 1.97.1, release, carga ambiental ALTA (load avg ~5-7 → varianza ±10-20 % en rutas sin tocar). Corpus regenerado el mismo día 18:24 (absolutos NO comparables con 2026-08-05/13; comparaciones dentro del mismo día). Baseline del inventario documentado (open 38-67 µs, render 1x 4,1-6,8 ms, cache_scroll 95,9-131,9 ms…); las rutas optimizadas ganan **31-89 %** (blit/prefetch) por encima del ruido. Sweep de humo: PEAK_RSS 30 072 KB.
- **Verificación**: `cargo test -p pdf_core` todo en verde (44+21+8+7+9+7+5+5+2+4 tests según binario) y `cargo check -p pdf_core` OK. `pdf_android` NO compila en host (android-activity) → **pendiente: prueba manual en la tablet** (barra de herramientas, gestos de boli/resaltado, undo, scroll de biblioteca sin parpadeo) y medición de frame time en dispositivo.

## 2026-08-18 — Biblioteca personal premium: Continue Reading + My Library + progreso por libro + rect de selección rápido

Rediseño de la pantalla **Library** (`UiMode::Library`) como biblioteca PERSONAL (estilo Apple Books/Kindle pero propio): las PORTADAS mandan; se eliminó el aspecto de file manager (sin rutas completas visibles, sin iconos PDF genéricos: placeholder elegante con el título mientras carga la portada). Solo se tocó `crates/pdf_android/` (lib/draw/input/reader/persist); pdf_core/pdf_app/pdf_bench intactos. Sin commits (regla AGENTS.md).

- **Modelo de datos de progreso por libro (`persist.rs`)**: `internal/library.json` = `Vec<BookProgress>` con `{path, page, page_count, last_read_unix, added_unix}` (clave = ruta local absoluta, la misma de `state.json`/`recents.json`). `Reader::save_state` (eager, cada cambio de página/apertura/zoom/dark) lo actualiza con `touch_progress`/`save_progress`/`load_progress`/`progress_for` (funciones puras con tests `#[cfg(test)]`; no ejecutables en host: `ndk-sys` solo compila para Android — `cargo check --tests --target aarch64-linux-android` valida la compilación). Derivados SIN abrir el PDF: `title` = nombre sin extensión (`title_from_name`), `author` = primer segmento de RELATIVE_PATH o "PDF" (`entry_author`), `progress% = (page+1)/page_count` (`pct`), `status` = Unread/Reading/Finished (`book_status`, `is_finished` = última página alcanzada). `open_pdf_at(path, start_page)` reanuda en la página guardada (Continue Reading y rejilla); `open_library_entry` y el carousel pasan la página del registro.
- **Cabecera editorial**: título "Library" grande (hasta 44 px, negrita), botón **"＋ Add book"** (rescan MediaStore + toast "Add PDFs to Downloads, then open with PDFLector" dibujado como overlay con un segundo lock+present vía `draw::blit_overlay`, solo ~1,5 s), y **campo de búsqueda** (píldora) que al tocarla abre el panel de chips letra A-Z/# + carpeta (la búsqueda SIN teclado de siempre, ahora tras un campo; ✕ limpia filtros y cierra). El botón "Search" del sheet abre la biblioteca con el panel desplegado.
- **Continue Reading** (sección destacada, oculta si no hay libros): carousel horizontal de tarjetas (fondo `LIB_CARD_BG`, portada 2:3 grande con sombra, título, autor, barra de progreso dorada, "Page X of Y · Z%", botón "Read"); tocar la tarjeta abre el libro en su página guardada. Construido desde `recents.json` + `library.json` (`lib_continue_reading`); respeta el filtro de letra y, trivialmente, el de estado (All/Reading muestran la sección; Finished/Unread la ocultan).
- **My Library** (rejilla principal 3×3): portada 2:3 dominante + título (primario) + autor (muted) + barra fina de progreso si el libro está empezado. **Organización** discreta bajo el título: chips de SORT (Recently Added / Recently Read / Title / Author) y FILTER (All / Reading / Finished / Unread), que reordenan/filtran `lib_filtered` (`refresh_lib_filtered` con sort precomputado). **Empty state** si no hay PDFs: ilustración de libro (rects Canvas) + "Your library is empty" + "Add your first PDF to start reading." + botón "Add PDF" (o "Grant access" si falta el permiso MANAGE_EXTERNAL_STORAGE; `rescan_library` ya no cae al picker cuando no hay PDFs en ningún sitio).
- **Constantes de tema nuevas (`lib.rs` `mod theme`)**: `LIB_ACCENT`/`LIB_ACCENT_DARK` (dorado + texto oscuro), `LIB_CARD_BG`/`LIB_CARD_BORDER`, `LIB_SEARCH_BG`/`LIB_SEARCH_BORDER`, `LIB_PROGRESS_TRACK`. Reutiliza `LIB_COVER_SHADOW`/`LIB_COVER_PLACEHOLDER`/`LIB_TEXT_*`/`ACCENT_AMBER_*` existentes.
- **Rect de selección MÁS RÁPIDO** (petición del autor; medible con el log de `Reader::blit` durante el arrastre): `draw::fill_rect_bordered` usa ahora un camino bpp 4 con alfa-blend por LUT (`fill_rect_lut`, 2 tablas de 256 construidas una vez por llamada — sin división ni llamada por píxel, relleno por filas, borde como 4 rectas) en vez del `stamp` por píxel con división por 255; y `draw::blit_page_scaled` tiene un camino 1:1 (zoom == 1.0) que copia la página FILA a FILA (memcpy) sin la tabla x ni el bucle de vecino-más-cercano — el arrastre de selección re-blitea la página en cada Move. Sin regresión visual (el blend LUT se desvía ±1 por canal en overlays translúcidos).
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` 0 warnings (NDK r28 + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`), `cargo clippy -p pdf_android --target aarch64-linux-android --release` limpio, `cargo fmt --all -- --check` limpio, `cargo check -p pdf_android --target aarch64-linux-android --release --tests` OK, host check de pdf_core/pdf_app/pdf_bench OK. Pendiente: medición en tablet del frame time del rect de selección y del render de la biblioteca (hook: log `blit ... ms` de `Reader::blit`).

## 2026-08-18 — Restyling visual de la UI de Android: paleta warm-neutral, card de ajustes y portadas en rejilla

- **Paleta de color (`lib.rs` `mod theme`)**: migrada a tonos cálidos/neutros premium (`DARK_BAR_BG` = `0xFF0B0D12`, `DARK_BAR_BORDER` = `0xFF232B3A`, `DARK_BTN_BG` = `0xFF161B26`, `DARK_BTN_BORDER` = `0xFF2A3444`, `DARK_BTN_TEXT` = `0xFFE6EAF0`, `ACCENT` warm gold `0xFFC8A96A`/`0xFFD9BD8B`, `DARK_BADGE_BG` = `0xDD0B0D12` semitransparente, `LIB_BG` = `0xFF0B0D12`, `LIB_ROW_EVEN/ODD` = `0xFF10141C`/`0xFF141922`, `LIB_TEXT_PRIMARY/SECONDARY/MUTED`).
- **Sheet de ajustes (`draw.rs` `render_sheet`)**: panel card deslizante desde el borde superior con esquinas inferiores redondeadas (16px), 1px de borde `bar_border`, etiqueta "SETTINGS" en mayúsculas (11sp `LIB_TEXT_SECONDARY`), y botones estilo píldora (radio capsule $H/2$, 1px borde, `DARK_BTN_BG`/`DARK_BTN_TEXT`; toggle de modo oscuro con relleno `ACCENT` warm gold y texto oscuro en estado activo).
- **Biblioteca en rejilla (`draw.rs` `render_library_grid` / `paste_thumb`)**: portadas en celdas de 3 columnas con esquinas redondeadas 12px, 1px de borde `LIB_ROW_BORDER`, escaladas con *scale-to-fill* (sin letterbox), y título de 14sp a 1 sola línea con puntos suspensivos abajo (`LIB_TEXT_SECONDARY`).
- **Indicador de página (`draw.rs` `render_page_badge`)**: badge estilo píldora en la esquina inferior izquierda con fondo oscuro semitransparente (`DARK_BADGE_BG` `0xDD0B0D12`), 1px de borde y texto 12sp (`DARK_BADGE_TEXT`).
- **Archivos editados**: ÚNICAMENTE `crates/pdf_android/src/lib.rs` y `crates/pdf_android/src/draw.rs`.
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` (0 warnings), `cargo fmt --all -- --check` (limpio), `cargo clippy --all-targets -- -D warnings` (limpio).

## 2026-08-18 — Rediseño de UX: pantalla completa, sheet de ajustes, rejilla 3×3

- **Visor a pantalla completa**: se ELIMINÓ la barra superior fija del visor
  (Open/✏️/●/↶/−10/+10/Dark → `render_viewer_bar`, `viewer_bar_tap`, campo
  `viewer_bar`, constantes de layout). El documento ocupa toda la pantalla.
- **Sheet de ajustes** deslizante desde arriba (mitad de la ventana): se
  revela con un tirón hacia abajo desde la mitad superior (sigue al dedo, sin
  chocar con tap de página ni pinch), se cierra con swipe up o tap fuera.
  Controles: Back (biblioteca), Open (picker interno), Dark/Light, −10/+10,
  "N / total" (tap = página siguiente). Animación ~150 ms por `Reader::tick`
  con `poll_events(Some(16 ms))` solo mientras hay trabajo diferido.
- **Indicador de página** "N / total" como overlay abajo a la IZQUIERDA
  (sin barra); tap en él = página siguiente (decisión documentada).
- **Biblioteca en rejilla 3×3** con portada (página 1) + título (1-2 líneas):
  portadas PERZOSAS (solo celdas visibles, ≤ 3/tick, placeholder "…"),
  render sin copiar el PDF (`openFileDescriptor` + `/proc/self/fd/N`), caché
  LRU nueva `thumbs.rs` (36 entradas / 9 MiB / 200 px). Se quitó el índice de
  letras A-Z (no encaja en la rejilla; decisión documentada).
- **Lápiz ✏️ / subrayado / undo ↶ / color ● eliminados** (minimalista): sin
  gesto de dibujo; se MANTIENEN carga y render de anotaciones guardadas
  (`annotations.rs` queda con `#![allow(dead_code)]` documentado).
- **Solo se tocó pdf_android** (draw/input/reader/lib/jni/annotations +
  thumbs nuevo); zoom.rs/view.rs/persist.rs/cache.rs y pdf_core/pdf_app/
  pdf_bench intactos. Doc de estructura de rediseño UX.
- **Verificación**: build release aarch64-linux-android sin warnings (NDK r28
  + BINDGEN_EXTRA_CLANG_ARGS), clippy cross limpio, `cargo fmt --check` OK,
  host check de pdf_core/pdf_app/pdf_bench OK. Sin commits (regla AGENTS.md).

## 2026-08-13 — Visor a UNA SOLA HOJA + sheet fluido + estudio legal

- **Modo una sola hoja** (el autor no quería la columna de páginas apiladas): eliminada toda la
  geometría de columna (page_offsets/page_heights/doc_height/scroll_y/layout_dirty/pending_page,
  PAGE_GAP, visible_pages, rebuild_layout, clamp_scroll, update_page_from_scroll, blit_stacked) →
  sustituida por `draw::blit_page`: el visor blitea SOLO la página actual (centrada cover + pan
  anclado, recortada a sus bordes). Se conserva PageCache LRU con prefetch ±1 (no se dibuja) y el
  zoom relativo/anclado.
- **Sheet fluido**: la causa del lag era el re-blit de la página completa (~25-40 ms) por frame de la
  animación/arrastre. Fix: `draw::compose_frame` compone el frame de página UNA vez
  (fondo+página+anotaciones+indicador) y durante el deslizamiento `draw::blit_composed` solo hace
  memcpy (~1-2 ms) + overlay del sheet — la página ya no se re-blitea por paso.
- **Estudio legal preparatorio** en `docs/legal.md` (NO decisión, solo análisis para el futuro):
  punto de partida (LICENSE=AGPL-3.0-only efectivo; MuPDF AGPL-3.0-or-later), las 3 piezas
  (SPDX/REUSE, variante -or-later recomendada, NOTICE de atribución MuPDF/Artifex + terceros),
  cumplimiento AGPL al distribuir, y checklist de 8 pasos para ejecutar cuando haya versión.
- Sin commits (regla AGENTS.md: pendiente de que el autor lo pida).

## 2026-08-13 — Diagnóstico: "Activity no resuelve" en release APK (NO reproducible)

- **Síntoma reportado**: tras las olas Groq/Gemini/zoom, `am start -n com.pdflector.app/android.app.NativeActivity` daba "Error type 3: Activity class does not exist", `cmd package resolve-activity --brief com.pdflector.app` daba "No activity found", y ActivityTaskManager devolvía -92, pese a un AndroidManifest.xml correcto (aapt2 dump xmltree) y el .so en lib/arm64-v8a.
- **Bisección realizada (worktree 681b618, cargo-apk 0.10, NDK r28, aapt v1)**: el ÚNICO diff de manifest entre el commit conocido-bueno 681b618 y HEAD es el permiso INTERNET (`uses_permission android.permission.INTERNET`). Todo lo demás idéntico: hasCode=false, debuggable=false, exported=true, meta-data android.app.lib_name="pdf_android", intent-filters VIEW/application/pdf + MAIN/LAUNCHER.
- **Verificado en la tablet TCL 9469X (Android 15), USB**:
  - APK 681b618: `adb install -r` + `am start` → PID 8523, logcat "opened: 12 pages / restored / Resume / InitWindow".
  - APK HEAD con INTERNET (recompilado desde cero del Cargo.toml actual): → PID 8279, "opened: 328 pages / restored".
  - APK HEAD sin INTERNET (el instalado en target/release, byte-idéntico al que reportaba el fallo): uninstall+install limpio → PID 8116, arranca.
  - `cmd package resolve-activity` (paquete, MAIN/LAUNCHER y VIEW con content-URI) resuelve siempre; `monkey -p com.pdflector.app 1` → PID 8687.
- **Conclusión**: el fallo reportado NO se reproduce con el APK actual ni con el commit anterior; la activity se resuelve y arranca en todas las configuraciones probadas (con/sin INTERNET, install limpio). Sin cambio de metadatos necesario: `crates/pdf_android/Cargo.toml` intacto. El -92 (START_APP_STILL_STARTING) es un código de arranque diferido, no de actividad inexistente.
- **Verificación final**: `cargo build -p pdf_android --target aarch64-linux-android --release` 0 warnings, `cargo fmt --all -- --check` limpio. APK HEAD reinstalado en la tablet y arrancando (PID 8758). Sin commits (regla AGENTS.md).

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
- **Análisis de aceptación contra plan inicial (Fase 1)**: render <25 ms cumple en 3/4
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

## 2026-08-05 — Fase 0.5 Ola 3: ADR-001 → MuPDF, AGPL-3.0, eliminación de PDFium

- Mantengo las decisiones del autor: **MuPDF** motor único (prioridad RAM+fluir), repo licenciado **AGPL-3.0** (LICENSE añadido).
- Creado `docs/adr/ADR-001-motor-pdf.md` con benchmark, justificación, consecuencias.
- **PDFium eliminado**: `crates/pdf_core/src/engine/pdfium.rs`, `tests/basic.rs` (el viejo), dep `pdfium-render`, feature `pdfium`, selector de CLI en pdf_bench. `pdf_app` y `pdf_bench` migrados a MuPDF.
- MuPDF pasa a ser **default** (no opt-in): `pdf_core/Cargo.toml` con `mupdf` como dep directa.
- `.cargo/config.toml` conservado (linker aarch64) con comentario de la env var `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android` para MuPDF.
- Docs alineados: README (licencia AGPL-3.0), AGENTS.md §5/§6, plan inicial §1/§6 + Fase 0.5 ✅.
- Verificación final: `cargo build --workspace` OK, `cargo test -p pdf_core` OK, `cargo clippy --workspace -D warnings` limpio, `cargo fmt` limpio, cross-compile aarch64 OK.
- `vendor/pdfium/` y `vendor/pdfium-android-arm64/` obsoletos (gitignored `/vendor`); se limpiarán tras Fase 6 si procede.
- **Fase 0.5 cerrada**. Próximo: Fase 1 (lectura fluida, scroll virtualizado, caché LRU).

## 2026-08-05 — Fase 0.5 Ola 2: spike Android MuPDF + comparativa real

- **Android MuPDF spike (C2)**: `pdf_core --features mupdf` cross-compila a `aarch64-linux-android` en **17 s** con 1 variable de entorno: `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot` (bindgen la necesita porque no usa el wrapper clang del NDK). Sin deps de sistema. Fricción **BAJA**. rlib ARM aarch64 confirmado. Dato clave del ADR.
- **Comparativa (D)**: `pdf_bench` extendido con selector de motor (feature `mupdf = ["pdf_core/mupdf"]`, arg CLI). Resultados finales (release, AMD Ryzen 7 5800H): MuPDF gana en render 2,7-4× (dense 1x 3,53 vs 9,69 ms; large 2x 10,19 vs 35,10 ms) y en **RSS pico -21%** (25572 vs 32520 KB). Único caso donde PDFium no pierde: scanned 2x empata. Tabla en `docs/benchmark-results.md`.

## 2026-08-05 — Fase 0.5 Ola 1: backend MuPDF + harness pdf_bench + spike Android PDFium

- **MuPDF backend (A)**: añadido `crates/pdf_core` tras feature `mupdf` (mitenu messense/mupdf 0.8.0, mupdf-sys 0.8.0, AGPL-3.0). Features mínimas: `default-features=false` + `base14-fonts`. Misma API `RenderEngine`/`Document` que PDFium. `MupdfEngine::new()` sin lib path (estático, thread-safe por context clonable por hilo, sin OnceLock). `engine/mupdf.rs`. Tests cruzados en `tests/mupdf_backend.rs` (5/5 OK): page_count==2 en simple.pdf = mismo que PDFium. Build C 1ª vez ~24 s.
- **pdf_bench (B)**: criterion 0.5 (`benches/open_render.rs`, grupos open/render_1x/render_2x) + sesgo binario `src/main.rs` (barrido manual, mediana de 3, sonda VmHWM → PEAK_RSS_KB). Genérico sobre `RenderEngine`. Números PDFium inicial: dense 9,69/35,34 ms, scanned 20,01/66,20 ms, paper 1,72/26,44 ms, large 6,86/35,10 ms. PEAK_RSS_KB=32520.
- **Android PDFium spike (C1)**: `pdf_core --features pdfium` cross-compila a `aarch64-linux-android` en **1 comando / 22 s** (bind dinámico en runtime, `libpdfium.so` ARM aarch64 descargada de bblanchon/pdfium-binaries chromium/7988 a `vendor/pdfium-android-arm64/`). `.cargo/config.toml` con linker `aarch64-linux-android24-clang`. `tools/fetch_pdfium_android.sh` (idempotente). `.gitignore` ampliado a `/vendor`.

## 2026-08-05 — Scaffolding estándar de repo + subida a GitHub

- Añadido scaffolding estándar: `.editorconfig`, `.gitattributes`, `.github/dependabot.yml` (cargo + github-actions semanal), `.github/PULL_REQUEST_TEMPLATE.md`, `.github/ISSUE_TEMPLATE/bug_report.md` + `feature_request.md`, `CONTRIBUTING.md` (apunta a `AGENTS.md`).
- **Licencia SIN decidir**: no se añade `LICENSE` (pendiente ADR-001 tras la Fase 0.5; README ya lo indica). Estado provisional: "All rights reserved" implícito.
- Commit local: "Añade scaffolding estándar de repo (.editorconfig, plantillas GH, dependabot)".
- Repo GitHub `pdflector` (público, decisión 3 del plan): creado vía `gh repo create --source=. --push`; remoto `origin` configurado. URL: https://github.com/asierboveda/pdflector.
- Verificación previa al push: sin secretos en archivos tracked.

## 2026-08-05 — Repo: skills del ecosistema fuera de git (corrección del init)

- El commit inicial `2170cc8` incluía 488 archivos de skills del ecosistema (`.agents/skills/` + `.claude/skills/` symlinks, ~11,7 MB) además del código. Patrón incorrecto: lo instalado vía lockfile se excluye de git (análogo a `node_modules` + `package-lock.json`).
- **Decisión**: skills PROPIOS del proyecto → `.opencode/skills/` (versionados); skills del ecosistema → instalados con `npx skills add` en `.agents/skills/` y `.claude/skills/` (en disco, **gitignored**), reproducibles desde `skills-lock.json` (órdenes en PLAN §2.1).
- `.gitignore` actualizado con `/.agents/skills/` y `/.claude/skills/`.
- `git rm -r --cached .agents/skills .claude/skills` (los archivos QUEDAN en disco; el entorno del agente no cambia).
- Documentos alineados: AGENTS.md §11 y §12, plan inicial §2.1B.
- Commit raíz reescrito con `--amend --no-edit` (sin remoto, histórico único): ahora solo contiene el proyecto. Hash `2170cc8` descartado.
- Verificación: `git ls-files | wc -l` pequeño; `git ls-files | grep -E '^\.(agents|claude)/'` vacío; `.agents/skills/` en disco intacto (~12 MB); `git status` limpio.

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

## 2026-08-05 — Inicio del proyecto y primer paso

- Creados `AGENTS.md` (reglas del agente) y `docs/` (vault: `PROYECTO.md`, plan inicial).
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
- **Plan inicial**: §2.2 y §2.3 marcados según estado real.
- **Pendiente**: tablet Lenovo Idea Tab (opciones de desarrollador + depuración USB) y `adb devices` → verificación 2.3; reiniciar opencode para que el agente cargue los skills del proyecto.
