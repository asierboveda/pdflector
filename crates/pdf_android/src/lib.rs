// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_android — spike de Fase 1: ver un PDF renderizado en la tablet.
//!
//! App Android nativa mínima (backend `native-activity` de android-activity;
//! NativeActivity clásica, el `.so` se carga vía `android.app.lib_name`).
//!
//! - Al arrancar SIN intent (lanzamiento normal desde el launcher) muestra la
//!   **biblioteca MediaStore en REJILLA 3×3**: cada celda muestra la portada
//!   (página 1, render PERZOSO y bajo demanda con placeholder — ver `thumbs`)
//!   y el título debajo; tocar una celda copia el fichero al almacenamiento
//!   interno y lo abre con `pdf_core` (motor MuPDF, ADR-001).
//! - Con "abrir con" (ACTION_VIEW + content://) abre el PDF directamente sin
//!   pasar por la biblioteca (ver `launch_intent_pdf`).
//! - En cada redibujado renderiza la página actual a una escala *cover*
//!   (`max(win_w / page_w, win_h / page_h)`, puntos PDF → px; ver `view`)
//!   multiplicada por el factor de zoom continuo (1.0 = página completa),
//!   centrada sobre un fondo oscuro (letterbox).
//! - Blitea el `Bitmap` RGBA8 de pdf_core al `ANativeWindow` fila a fila,
//!   respetando `buffer.stride` (en píxeles, puede ser > `width`). El formato
//!   del buffer se fuerza a `R8G8B8A8_UNORM` con
//!   `ANativeWindow_setBuffersGeometry`; por defensa se manejan también
//!   `R8G8B8X8_UNORM` (copia directa) y `R5G6B5_UNORM` (conversión 565).
//! - Input multitáctil (`input_events_iter()`, que ya funcionaba para el tap):
//!   - tap en la mitad derecha → página siguiente; izquierda → anterior
//!     (el visor es PÁGINA a PÁGINA: el arrastre para scrollear se eliminó
//!     por decisión del autor, 2026-08-XX; el deslizamiento de un dedo
//!     cancela el tap sin hacer nada);
//!   - tirón hacia abajo desde la mitad superior → **sheet de ajustes**
//!     (panel deslizante desde arriba con Back/Open/Dark/−10/N/+10; swipe up
//!     o tap fuera lo cierra — 2026-08-XX);
//!   - pinch con dos dedos → zoom SOLO por pinch: factor RELATIVO a la
//!     distancia inicial y anclado al centro del pinch (el punto bajo los
//!     dedos no se desplaza); durante el gesto solo se actualiza el factor y
//!     se blitea el bitmap cacheado (sin re-render), y al soltar se
//!     re-renderizan las páginas visibles UNA vez a la resolución final.
//!
//! ## Rediseño de UX: pantalla completa, sheet, rejilla (2026-08-XX)
//!
//! Cambios ESTRUCTURALES del visor y la biblioteca (el estilo lo hará otro
//! agente; aquí solo la estructura funcional; ver `docs/ux-rediseño-estructura.md`):
//!
//! 1. **Visor a pantalla completa**: la barra superior fija (Open/✏️/●/↶/±10/
//!    Dark) se ELIMINÓ; el documento ocupa TODA la pantalla. Los ajustes
//!    viven ahora en un **sheet** deslizante desde arriba (mitad de la
//!    ventana): se revela con un tirón hacia abajo desde la mitad superior
//!    (sigue al dedo; al soltar, animación de ~150 ms hacia el objetivo más
//!    cercano) y se cierra con swipe up o tap fuera. Controles: Back
//!    (biblioteca MediaStore), Open (picker interno), Dark/Light, −10/+10 y
//!    "N / total" (tap = página siguiente). La animación la avanza
//!    `Reader::tick`, que el bucle llama con `poll_events(Some(16 ms))`
//!    SOLO mientras hay trabajo diferido (sheet animándose, portadas
//!    pendientes o long-press del dedo en el documento): en reposo el poll
//!    bloquea sin gastar batería.
//! 2. **Indicador de página "N / total"** como overlay pequeño abajo a la
//!    IZQUIERDA (sin barra); tap en él = página siguiente (decisión
//!    documentada en `input::page_badge_tap`).
//! 3. **Biblioteca en rejilla 3×3** con portada (página 1) + título (1-2
//!    líneas). Portadas PERZOSAS: solo celdas visibles, en lotes de ≤ 3 por
//!    tick (`Reader::pump_thumbs`), placeholder "…" mientras cargan, caché
//!    LRU acotada en `thumbs.rs` (36 entradas / 9 MiB, portadas de 200 px).
//!    La tira de letras A-Z se quitó (no encaja en la rejilla; decisión
//!    documentada).
//! 4. **Lápiz ✏️, subrayado, undo ↶ y color ● eliminados** (minimalista): no
//!    hay gesto de dibujo; se MANTIENEN la carga y el render de anotaciones
//!    ya guardadas (los trazos del usuario no se pierden, solo no se pueden
//!    crear desde la UI por ahora). El estado de dibujo queda en
//!    `annotations.rs` con `#![allow(dead_code)]` (ver su cabecera).
//!
//! ## Página a página + caché de páginas (2026-08-XX)
//!
//! El visor volvió al modo PÁGINA a PÁGINA (decisión del autor): el
//! arrastre vertical continuo (que sustituyó al salto de página el
//! 2026-08-13) se ELIMINÓ; pasar de página es un tap en la mitad derecha
//! (siguiente) o izquierda (anterior).
//!
//! ## Modo UNA HOJA (2026-08-XX) — la columna de páginas se eliminó
//!
//! El visor muestra SOLO la página actual (modo UNA HOJA): se eliminaron
//! toda la geometría de la columna apilada (`page_offsets`/`page_heights`/
//! `doc_height`/`scroll_y`/`layout_dirty`/`pending_page`), `visible_pages()`,
//! `blit_stacked` y `update_page_from_scroll` (ver el diff del commit). El
//! blit dibuja UNA página (fondo + página actual + anotaciones + overlays,
//! `draw::blit_page`) centrada con cover y recortada a los bordes de la
//! ventana; el zoom (pinch) actúa SOLO sobre la página actual (recortada a
//! sus bordes, nunca otra hoja) y mantiene el pan de anclaje. La `PageCache`
//! (LRU) se CONSERVA para que prev/next sea instantáneo (precarga la vecina,
//! `ensure_pages_rendered`), pero las vecinas nunca se dibujan:
//!
//! - `cache.rs` (`PageCache`): LRU en RAM de páginas renderizadas
//!   (página → `Bitmap`), limitada por bytes (48 MiB) y por entradas (5),
//!   coherente con el RSS < 150 MB; evita el re-render al volver atrás.
//!   Los bitmaps se guardan SIEMPRE normales: la inversión de modo oscuro se
//!   aplica al blitear (`draw::blit_page`).
//! - Render (vía caché) de la página actual + 1 vecina por lado (prefetch
//!   simple: el paso de página es instantáneo); el blit dibuja SOLO la
//!   página actual (centrado cover + pan de anclaje del pinch, recorte a la
//!   ventana) con un solo lock+present (`draw::blit_page`, vecino-más-cercano
//!   para el zoom).
//! - El pinch hace zoom (factor relativo + anclado; re-render nítido al
//!   soltar); el tap cambia de página. La página actual alimenta el
//!   indicador "N / total", los saltos ±10 y la persistencia (`persist`), que
//!   sigue guardando la página actual como `page` del estado.
//!
//! ## Sheet de ajustes: animación sin re-blit de la página (2026-08-XX)
//!
//! El sheet (panel deslizante desde arriba) iba LENTO al tirar hacia abajo:
//! la causa era que cada frame de la animación (~150 ms con `poll_events` +
//! timeout, ~10 ticks) y cada Move del arrastre hacían un redraw completo que
//! re-bliteaba la página ENTERA (~25-40 ms/blit en la tablet) + el overlay
//! del sheet — la animación no llegaba a 60 fps. El fix: mientras el sheet
//! está visible (`sheet_progress > 0`) el visor usa un FRAME COMPUESTO
//! (`Reader::page_frame`, un `Bitmap` RGBA8 del tamaño de la ventana con
//! fondo + página + anotaciones + indicador, compuesto UNA vez al empezar a
//! deslizar con `draw::compose_frame`) y cada frame de la animación/arrastre
//! solo copia ese frame al buffer (`draw::blit_composed`, memcpy ~1-2 ms) +
//! el overlay del sheet. La PÁGINA no se re-blitea en cada paso: el sheet se
//! siente inmediato y fluido. El frame se invalida al cambiar página/zoom/
//! modo oscuro/ventana/documento y se libera al cerrar el sheet del todo.
//!
//! ### Nota de rendimiento del zoom (por qué NO `scale_bitmap`)
//!
//! El plan sugería re-render a `scale_level_for_zoom(zoom)` y escalar el
//! bitmap existente con `scale_bitmap` como camino "fast" mientras llega el
//! nítido. El benchmark Fase 1/B3 (docs/benchmark-results.md, tablet TCL)
//! mide que `scale_bitmap` es ~4-5,6× MÁS LENTO que el re-render MuPDF en esta
//! tablet (69-70 ms vs 15-17 ms a 2x; 276-325 ms vs 53-59 ms a 4x): el upscale
//! software por píxel es el camino caro. Por eso el pinch re-renderiza directo
//! a la escala continua (el camino rápido medido) y NO usa la ladder +
//! `scale_bitmap`. El zoom "escalona" visualmente en pasos del 2.5 %
//! (ZOOM_RE_RENDER_EPS) para no re-renderizar en cada Move.
//!
//! ## Refresco de pantalla (debug 2026-08-13)
//!
//! Síntoma reportado: tras un tap para pasar página, logcat mostraba
//! "page 2" + "render page 2" + "blit" pero el screenshot seguía mostrando la
//! página 1 (hash idéntico). Diagnóstico con checksums FNV del bitmap y del
//! buffer: el render de la página 2 producía EXACTAMENTE los mismos píxeles que
//! la página 1 y el blit copiaba fielmente ese bitmap. La causa raíz NO era el
//! refresco sino el corpus: `scientific_paper.pdf` tiene las 12 páginas
//! píxel-idénticas (verificado con `pdftoppm`/md5 en desktop). Con
//! `large_document.pdf` (500 págs.) el refresco funciona: 4 screenshots de
//! páginas 1-4 con hashes distintos, correlacionados con los fnv del logcat.
//!
//! Verificaciones de robustez del blit (todas OK en el código):
//! 1. `unlock_and_post` se llama SIEMPRE: el `NativeWindowBufferLockGuard` de
//!    ndk lo hace en su `Drop`, y el guard vive en el scope del `blit` — todos
//!    los early-returns lo dropean antes de salir.
//! 2. Formato/stride/dimensiones se leen del guard de CADA `lock()`, no una
//!    vez (nunca se cachean).
//! 3. Defensa contra `ANativeWindow` stale: tras `WindowResized`/`RedrawNeeded`
//!    se re-obtiene `app.native_window()` (la glue de NativeActivity devuelve
//!    siempre el handle vigente; en recreaciones de la surface es un window
//!    nuevo que además necesita re-forzar el formato con
//!    `set_buffers_geometry`).
//!
//! El render es síncrono en el hilo del bucle (MuPDF rinde ~18-20 ms/página en
//! la tablet; caché y render en segundo plano son trabajo de Fase 6).
//!
//! ## Biblioteca MediaStore (2026-08-13)
//!
//! El lanzamiento normal (sin intent) abre una biblioteca de los PDFs del
//! SISTEMA consultando `MediaStore.Files` (`content://media/external/file`):
//! proyección `[_ID, DISPLAY_NAME, RELATIVE_PATH, _SIZE]`, selección
//! `mime_type='application/pdf'` y orden `RELATIVE_PATH, DISPLAY_NAME`. Cada
//! fila se convierte a content URI con `ContentUris.withAppendedId(files_uri,
//! _ID)`. Al tocar una fila: `ContentResolver.openInputStream(uri)` → copia a
//! `internal/pdfs/` → `MupdfEngine::open`.
//!
//! Permisos (verificado en la TCL 9469X, Android 15): la lectura de PDFs
//! ajenos vía MediaStore exige en Android 13+ el appop **"All files access"**
//! (`MANAGE_EXTERNAL_STORAGE`), concedido por el usuario en Ajustes; NO existe
//! un `READ_MEDIA_*` para documentos (PDF no es imagen/vídeo/audio). En
//! Android ≤ 12 basta `READ_EXTERNAL_STORAGE` (declarada con `maxSdkVersion=32`).
//! La biblioteca detecta el estado del permiso
//! (`Environment.isExternalStorageManager()`) y muestra un botón **Grant** que
//! abre los Ajustes del permiso; al volver, el `Resume` re-consulta MediaStore.
//! El picker de carpeta interna queda como fallback si MediaStore devuelve
//! vacío (permiso concedido y sin PDFs).
//!
//! ## Rejilla 3×3 y fin del índice de letras (2026-08-XX)
//!
//! La biblioteca pasó de lista de filas a REJILLA de 3 columnas (portada de
//! la página 1 + título; portadas perezosas y caché en `thumbs.rs` — ver
//! "Rediseño de UX" arriba). Con la rejilla se ELIMINÓ la tira de letras
//! A-Z+'#' que filtraba por inicial (decisión documentada en
//! `docs/ux-rediseño-estructura.md`): sus 27 celdas estaban diseñadas para
//! la lista de filas y no encajan en la rejilla; la navegación es por scroll.
//! Se quitaron `normalize_letter`, `lib_strip_*`, `library_filter(_ed)` y
//! `set_library_filter`.
//!
//! La agrupación por carpeta con encabezados colapsables NO se implementa en
//! esta iteración (decisión documentada): MediaStore ya ordena por
//! `relative_path, display_name` (las carpetas quedan agrupadas); un
//! colapso/expansión exigiría una indirección fila-visual→entrada en tap,
//! scroll y clamp (más estado por
//! carpeta) que complica el código por poco: con el índice se encuentra un
//! PDF en ≤ 2 taps.
//!
//! ## Selección de PDF (picker, 2026-08-13)
//!
//! La vía preferida del enunciado (selector del sistema SAF,
//! `ACTION_OPEN_DOCUMENT` + `onActivityResult`) NO es viable limpiamente en
//! este stack, por dos razones verificadas en el código de las crates:
//! 1. android-activity 0.6.1 (backends native-activity y game-activity) NO
//!    expone activity results: el stock `android.app.NativeActivity` no
//!    reenvía `onActivityResult` al native y la crate no añade ningún hook
//!    (grep de `onActivityResult` en su src: sin resultados).
//! 2. cargo-apk 0.10 + ndk-build 0.10 no compilan fuentes Java: subclasear la
//!    Activity para capturar el resultado exigiría inyectar un classes.dex a
//!    mano en el APK (javac+d8+aapt2+resign) — frágil y fuera de "cambios
//!    mínimos".
//!
//! Por eso el picker implementa el fallback del enunciado: una lista de PDFs
//! de los directorios de la app.
//! - Botón **Open** dibujado en la esquina superior izquierda del visor (el
//!   tap en esa zona abre el picker en vez de pasar de página).
//! - El picker lista los `*.pdf` de `internal_data_path()` (raíz + subdir
//!   `pdfs/`) y `external_data_path()` (raíz + subdir `pdfs/`), ordenados por
//!   nombre; arrastre vertical para hacer scroll, tap en una fila para abrir,
//!   botones **Rescan** (releer directorios) y **Back** (volver al PDF abierto,
//!   si lo hay).
//! - El texto se dibuja con `android.graphics.Canvas` vía JNI (fuente del
//!   sistema, antialiasing) sobre un `android.graphics.Bitmap` que se
//!   convierte a nuestro `Bitmap` RGBA8 y se blitea por el mismo camino que la
//!   página (misma resolución de pantalla, sin reescalado).
//! - Cómo añadir PDFs a la tablet (el APK release no es debuggable, así que
//!   `run-as` no funciona; la vía externa es la práctica):
//!   ```text
//!   adb push corpus/mi.pdf /sdcard/Android/data/com.pdflector.app/files/pdfs/
//!   # o en internal (solo APK debug): adb shell run-as com.pdflector.app \
//!   #   sh -c 'cp /data/local/tmp/mi.pdf files/pdfs/mi.pdf'
//!   ```
//!   y pulsar **Rescan** en el picker. El arranque sigue resolviendo el PDF por
//!   defecto como antes (`PDFLECTOR_PDF` → internal/demo.pdf → fallback); si
//!   no hay ninguno, la app arranca directamente en el picker.

//! ## Selección de texto: long-press + arrastre, copiar y subrayar (2026-08-XX)
//!
//! Selección de texto con long-press (mantener pulsado) + arrastre, con menú
//! flotante Copiar/Subrayar/IA (la Parte 2 —IA— la añadirá otro agente):
//!
//! - **Gesto** (`input.rs`): mantener un dedo QUIETO (sin levantarlo y sin
//!   moverse más de `TAP_SLOP`) sobre el documento durante `LONG_PRESS_MS`
//!   (400 ms) entra en MODO SELECCIÓN: `input::tick_gestures` (resuelto en
//!   `Reader::tick` con el poll con timeout de `needs_tick` mientras el dedo
//!   esté abajo) fija el ancla en el punto del dedo y materializa el rect
//!   como punto (`begin_sel`); al arrastrar (> `SELECT_SLOP`) el rect sigue
//!   al dedo (`update_sel`); al levantar, `end_sel` fija la selección y abre
//!   el menú Copiar/Subrayar/IA (un long-press sin arrastre se descarta). El
//!   tap simple de página es INMEDIATO (sin ventana de doble-tap: un doble-
//!   tap rápido son dos cambios de página) y NO se dispara nunca mientras
//!   haya selección/menú abierto.
//! - **Estado** (`reader.rs`): `Reader::sel` guarda la selección en coords de
//!   PANTALLA (px de ventana, `anchor`/`cur`) — decisión documentada: el
//!   gesto, el render del rect y el menú viven en pantalla; la conversión a
//!   página se hace UNA sola vez al extraer texto (`sel_text`) o subrayar
//!   (`highlight_sel`) con `Reader::screen_to_page`, la INVERSA exacta del
//!   mapeo del blit (misma `scale = cover × zoom` y `dx/dy` que `PageAnnots`).
//! - **Render** (`draw.rs`): el rect de selección se dibuja translúcido con
//!   borde sobre la página, RECORTADO a los bordes de la hoja
//!   (`Reader::sel_screen_rect`); el menú flotante se renderiza con el
//!   Canvas+JNI como overlay cacheado (`Reader::sel_menu`).
//! - **Copiar** (`jni.rs`): `ClipboardManager.setPrimaryClip(
//!   ClipData.newPlainText("text", sel))` con el contexto de la Activity;
//!   aviso breve "copied" en un toast sobre el indicador (`Reader::toast`).
//! - **Subrayar** (`reader.rs`): añade un `Annotation::Highlight` con el rect
//!   de selección en página (amarillo) al `AnnotationSet` y PERSISTE con
//!   `AnnotationStore::save` (sidecar SQLite); el render de highlights ya
//!   existente (`draw::draw_highlight`, relleno translúcido bajo los trazos)
//!   lo muestra al re-redibujar.
//!
//! La extracción de texto (`sel_text`) llama a `doc.text(page)` UNA vez y
//! concatena el texto de los spans cuyo bbox INTERSECTA el rect de selección,
//! ordenados por (y, luego x) — orden de lectura; si no hay texto (PDF
//! escaneado) devuelve cadena vacía y "Copiar" avisa "no text".

//!
//! ## Partición en módulos (2026-08-13)
//!
//! `lib.rs` (2625 líneas) se ha partido para permitir 3 cambios en paralelo
//! (quitar doble-tap, optimizar zoom, pantalla completa) sin pisarse entre
//! agentes. Cada agente toca SOLO su módulo:
//! - `reader`: `struct Reader`, tipos de UI (`UiMode`, `PdfEntry`,
//!   `LibraryEntry`, `LibraryScan`, `LaunchPdf`) y helpers del picker.
//! - `input`: gestos (`GestureKind`/`GestureState`), `handle_motion` y taps.
//! - `draw`: blit de buffers, Canvas+JNI (`jni_text_bitmap`), listas, botón
//!   "Open" y la capa de anotaciones (polilíneas Bresenham sobre el bitmap).
//! - `jni`: toda la interacción Java (Intent, MediaStore, ContentResolver).
//! - `view`: escala inicial de apertura (stub: contain; futuro: fill+crop).
//! - `zoom`: blit rápido (stub: blit completo; futuro: recorte según zoom).
//! - `persist`: estado del visor en `internal/state.json` (posición + modo
//!   oscuro; ver la sección "Visor: estado persistido y modo oscuro").
//! - `annotations`: estado de dibujo (trazo en curso, paleta de colores) —
//!   CONSERVADO pero SIN uso desde la UI (ver abajo y `annotations.rs`); el
//!   modelo y la persistencia (sidecar SQLite) viven en pdf_core.
//!
//! Aquí queda SOLO `android_main`, el bucle de eventos, las constantes
//! compartidas (`pub(crate)`) y los `mod`.
//!
//! ## Anotaciones a mano (modo dibujo) — ELIMINADO de la UI (2026-08-XX)
//!
//! La barra superior tenía el botón "✏️" (modo dibujo: el arrastre con un
//! dedo creaba un `Stroke` en coordenadas de página, se guardaba en el
//! sidecar SQLite y se dibujaba como capa vectorial Bresenham sobre el
//! bitmap — AGENTS.md §4.3). Con el rediseño minimalista (pantalla completa
//! con sheet) el modo dibujo y sus controles (✏️/●/↶) se ELIMINARON: no hay
//! gesto de dibujo.
//!
//! Se MANTIENEN la carga y el render de anotaciones ya guardadas (el usuario
//! no pierde sus trazos): la capa vectorial sigue en `draw.rs`
//! (`PageAnnots`/`draw_annotations`) y el sidecar se sigue cargando
//! (`Reader::load_annotations`). `annotations.rs` queda con
//! `#![allow(dead_code)]` documentado por si una fase futura reintroduce la
//! creación.
//!
//! ## Visor: estado persistido, modo oscuro y overlays (2026-08-XX)
//!
//! Tres mejoras del visor, coherentes entre sí:
//!
//! 1. **Posición recordada** (`persist.rs`): `internal/state.json` guarda
//!    `{path, page, zoom, dark}` en cada cambio de página, al soltar un
//!    pinch y al alternar el modo oscuro (escritura *eager*, crash-safe). Al
//!    arrancar SIN intent se restaura: si la ruta guardada sigue existiendo,
//!    el PDF se abre directamente en esa página/zoom/modo; si ya no existe (o
//!    no se puede abrir), se BORRA el estado y se abre la biblioteca
//!    MediaStore (política completa en `persist.rs`).
//! 2. **Overlays del visor** (bliteados en el MISMO buffer que las páginas,
//!    tras ellas — no hace falta tocar `zoom.rs`): el indicador de página
//!    "N / total" abajo a la izquierda (`draw::render_page_badge`, tap =
//!    página siguiente) y el sheet de ajustes deslizante desde arriba
//!    (`draw::render_sheet`). Renderizados con el mismo Canvas+JNI de
//!    `draw.rs`; la geometría DEBE coincidir con las zonas de tap de
//!    `input.rs` (helpers compartidos en `reader.rs`).
//! 3. **Modo oscuro**: `toggle_dark` invierte el bitmap YA renderizado con
//!    `pdf_core::dark::invert_bitmap` (sin re-renderizar MuPDF); los renders
//!    nuevos se invierten al generarse, en `render_current_page`. El fondo
//!    letterbox pasa a negro puro (`DARK_BG`). La preferencia se persiste
//!    junto a la posición.

use android_activity::{AndroidApp, MainEvent, PollEvent};
use log::info;

mod annotations;
mod cache;
mod draw;
mod input;
mod jni;
mod persist;
pub(crate) mod prediction;
mod reader;
mod thumbs;
mod view;
mod zoom;

use crate::input::handle_input;
use crate::reader::Reader;

/// Radio (px) de movimiento máximo entre Down y Up para considerar el gesto un
/// "tap" (no un swipe). ~20 px a 320 dpi (ViewConfiguration touch slop ≈ 8 dp).
pub(crate) const TAP_SLOP: f32 = 24.0;
/// Umbral de movimiento (px) tras el long-press para EXTENDER el rect de
/// selección: el ancla es el punto del dedo al superar `LONG_PRESS_MS` y el
/// rect (un punto) solo empieza a seguir al dedo si se mueve más de esto
/// (los micro-drags no extienden la selección); un long-press sin arrastre
/// no fija selección.
pub(crate) const SELECT_SLOP: f32 = 8.0;
/// Tamaño mínimo (px) del rect de selección para fijarla y mostrar el menú
/// Copiar/Subrayar/IA: un rect degenerado (long-press sin arrastre) se descarta.
pub(crate) const SEL_MIN_PX: f32 = 2.0;
/// Umbral (px de pantalla) que separa un "toque" (sin gesto) de un gesto de
/// herramienta real (boli/resaltador): si el gesto no recorre al menos esto,
/// `Reader::end_tool_gesture` lo descarta (mismo criterio que el tamaño
/// mínimo de la selección de texto, `SEL_MIN_PX`). Un dedo o el lápiz que
/// toca y suelta sin intención de dibujar no crea una anotación por accidente.
pub(crate) const TOOL_MIN_PX: f32 = 6.0;
/// Tiempo tras el último trazo del stylus durante el cual se ignora el táctil
/// (palm rejection por tiempo): evita pans/zooms accidentales de la palma
/// al apoyar la mano al escribir. 500ms es un compromiso entre reactividad
/// y seguridad (Saber usa ~300-500ms, Samsung Notes ~400ms).
pub(crate) const STYLUS_IGNORE_MS: u64 = 500;
/// Duración del aviso breve ("copied", "highlighted", ...) sobre el indicador
/// de página (`Reader::toast`, expirado en `Reader::tick`).
pub(crate) const TOAST_MS: std::time::Duration = std::time::Duration::from_millis(1500);
/// Duración (s) de la transición al abrir un libro: el snapshot de la
/// biblioteca/picker se funde sobre la página. ~12 frames a 60 fps.
pub(crate) const LIB_FADE_MS: f32 = 0.18;

// ---------------------------------------------------------------------------
// Configuración de "Preguntar a la IA" (Fase 5): Groq (texto) + Gemini (imagen)
// ---------------------------------------------------------------------------
//
// Las API keys van EMBEBIDAS en el APK: es una app de USO PERSONAL (sin
// telemetría ni servidores propios) y la consulta va directa del dispositivo
// al proveedor por HTTPS. Obtenlas gratis en https://console.groq.com/keys
// (plan free) y en https://aistudio.google.com/apikey. PRIVACIDAD: las keys
// viajan en el binario — no publiques el APK ni el repo con keys reales; los
// placeholders de abajo compilan y muestran el error "no hay red / key
// inválida" en el panel.
//
// HÍBRIDO (2026-08-XX): TEXTO → Groq con llama-3.3-70b-versatile (buen
// equilibrio de calidad/velocidad: el usuario del panel es "explícame este
// párrafo", así que la latencia importa); IMAGEN → Gemini con
// gemini-flash-latest (el modelo de visión de Groq fue RETIRADO y devuelve
// 403 — ver `GEMINI_MODEL`).
pub(crate) const GROQ_API_KEY: &str = include_str!("../groq_key.txt");
pub(crate) const GROQ_MODEL: &str = "llama-3.3-70b-versatile";
pub(crate) const GOOGLE_API_KEY: &str = include_str!("../google_key.txt");
/// Modelo de GEMINI para "Preguntar a la IA" cuando la selección es una
/// ecuación/gráfico (Fase 5): en ese caso se manda el PNG del crop de la
/// selección a `GeminiClient::explain_image` (pdf_core::ai), que usa ESTE
/// modelo (multimodal, texto + imagen). Coincide con el default de pdf_core;
/// se declara aquí para que la llamada de `Reader::ask_ai` no lleve el
/// literal suelto (mismo patrón que `GROQ_MODEL`). El modelo de visión de
/// Groq (`llama-3.2-90b-vision-preview`) se retiró del servicio y devuelve
/// 403, de ahí el cambio a Gemini.
pub(crate) const GEMINI_MODEL: &str = "gemini-flash-latest";
/// Límites del factor de zoom continuo (1.0 = página completa a pantalla).
/// `PINCH_MIN = 1.0`: SIN zoom hacia fuera — la página no se puede ver más
/// pequeña que a pantalla completa (cover); el pan queda limitado a los
/// bordes de la hoja (ver `Reader::clamp_pan`).
pub(crate) const PINCH_MIN: f32 = 1.0;
pub(crate) const PINCH_MAX: f32 = 8.0;

/// Constantes de color, temas y tipografía para la interfaz (0xAARRGGBB para Canvas JNI).
pub(crate) mod theme {
    use serde::{Deserialize, Serialize};

    /// Color transparente para fondos de bitmap sin opacidad.
    pub(crate) const TRANSPARENT: u32 = 0x00000000;

    /// Fondo de error cuando no se pudo abrir el PDF (rojo oscuro opaco).
    pub(crate) const ERROR_BG_RGBA: [u8; 4] = [0x5A, 0x12, 0x12, 0xFF];

    /// Color del relleno del rect de selección (azul accent, alfa ~30 %: 77/255).
    pub(crate) const SEL_FILL_RGBA: [u8; 4] = [0x4D, 0xA3, 0xFF, 0x4D];
    /// Color del borde del rect de selección (1-2 px, alfa completo).
    pub(crate) const SEL_BORDER_RGBA: [u8; 4] = [0x4D, 0xA3, 0xFF, 0xFF];

    /// Jerarquía tipográfica única (Readest Design System).
    pub(crate) const FONT_DISPLAY: f32 = 24.0;
    pub(crate) const FONT_TITLE: f32 = 17.0;
    pub(crate) const FONT_BODY: f32 = 14.0;
    pub(crate) const FONT_CAPTION: f32 = 12.0;
    pub(crate) const FONT_LABEL_CAPS: f32 = 11.0;

    /// Temas disponibles en PDFLector (reglas y paletas derivadas exactas de Readest).
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
    pub(crate) enum AppTheme {
        #[default]
        DefaultLight,
        SepiaLight,
        DefaultDark,
        SepiaDark,
    }

    impl AppTheme {
        /// Cicla al siguiente tema: Default-Light → Sepia-Light → Default-Dark → Sepia-Dark → Default-Light...
        pub(crate) fn next(self) -> Self {
            match self {
                Self::DefaultLight => Self::SepiaLight,
                Self::SepiaLight => Self::DefaultDark,
                Self::DefaultDark => Self::SepiaDark,
                Self::SepiaDark => Self::DefaultLight,
            }
        }

        /// ¿Modo oscuro activo?
        pub(crate) fn is_dark(self) -> bool {
            matches!(self, Self::DefaultDark | Self::SepiaDark)
        }

        /// Obtiene la paleta completa calculada para este tema según las reglas Readest.
        pub(crate) fn palette(self) -> ThemePalette {
            match self {
                Self::DefaultLight => ThemePalette {
                    name: "Default-Light",
                    is_dark: false,
                    base_100: 0xFFFFFFFF,
                    base_200: 0xFFF2F2F2,
                    base_300: 0xFFE0E0E0,
                    base_content: 0xFF171717,
                    neutral: 0xFFD9D9D9,
                    neutral_content: 0xFF737373,
                    primary: 0xFF0066CC,
                    primary_content: 0xFFFFFFFF,
                },
                Self::SepiaLight => ThemePalette {
                    name: "Sepia-Light",
                    is_dark: false,
                    base_100: 0xFFF1E8D0,
                    base_200: 0xFFE6DCBF,
                    base_300: 0xFFD4C8A5,
                    base_content: 0xFF5B4636,
                    neutral: 0xFFC9BC96,
                    neutral_content: 0xFF8A705B,
                    primary: 0xFF008B8B,
                    primary_content: 0xFFFFFFFF,
                },
                Self::DefaultDark => ThemePalette {
                    name: "Default-Dark",
                    is_dark: true,
                    base_100: 0xFF242424,
                    // Fondo de biblioteca (delta de luminosidad +10% más profundo en dark para contraste de tarjetas)
                    base_200: 0xFF141414,
                    base_300: 0xFF3D3D3D,
                    base_content: 0xFFE0E0E0,
                    neutral: 0xFF474747,
                    neutral_content: 0xFF9E9E9E,
                    primary: 0xFF77BBEE,
                    primary_content: 0xFF111111,
                },
                Self::SepiaDark => ThemePalette {
                    name: "Sepia-Dark",
                    is_dark: true,
                    base_100: 0xFF342E25,
                    // Fondo de biblioteca (delta de luminosidad +10% más profundo en dark para contraste de tarjetas)
                    base_200: 0xFF201B15,
                    base_300: 0xFF4D4437,
                    base_content: 0xFFFFD595,
                    neutral: 0xFF615747,
                    neutral_content: 0xFFC4A572,
                    primary: 0xFF48D1CC,
                    primary_content: 0xFF1A1610,
                },
            }
        }
    }

    /// Paleta de color derivada de Readest para renderizado UI.
    #[allow(dead_code)]
    #[derive(Copy, Clone, Debug)]
    pub(crate) struct ThemePalette {
        pub(crate) name: &'static str,
        pub(crate) is_dark: bool,
        pub(crate) base_100: u32,
        pub(crate) base_200: u32,
        pub(crate) base_300: u32,
        pub(crate) base_content: u32,
        pub(crate) neutral: u32,
        pub(crate) neutral_content: u32,
        pub(crate) primary: u32,
        pub(crate) primary_content: u32,
    }

    #[allow(dead_code)]
    impl ThemePalette {
        pub(crate) fn bg(&self) -> u32 {
            self.base_100
        }
        pub(crate) fn lib_bg(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn card_bg(&self) -> u32 {
            self.base_100
        }
        pub(crate) fn card_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn btn_bg(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn btn_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn btn_text(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn text_primary(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn text_secondary(&self) -> u32 {
            self.neutral_content
        }
        pub(crate) fn text_muted(&self) -> u32 {
            self.neutral_content
        }
        pub(crate) fn accent(&self) -> u32 {
            self.primary
        }
        pub(crate) fn accent_text(&self) -> u32 {
            self.primary_content
        }
        pub(crate) fn progress_track(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn progress_fill(&self) -> u32 {
            self.primary
        }
        pub(crate) fn cover_shadow(&self) -> u32 {
            if self.is_dark { 0x70000000 } else { 0x34000000 }
        }
        pub(crate) fn cover_placeholder(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn sel_overlay(&self) -> u32 {
            if self.is_dark { 0x5577BBEE } else { 0x440066CC }
        }
        pub(crate) fn popup_bg(&self) -> u32 {
            if self.is_dark { 0xF2222222 } else { 0xF2FFFFFF }
        }
        pub(crate) fn popup_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn badge_bg(&self) -> u32 {
            if self.is_dark { 0xDD222222 } else { 0xDDFFFFFF }
        }
        pub(crate) fn badge_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn badge_text(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn status_bg(&self) -> u32 {
            if self.is_dark { 0xFF3A1A1A } else { 0xFFFFEAEA }
        }
        pub(crate) fn status_border(&self) -> u32 {
            if self.is_dark { 0xFF5A2A2A } else { 0xFFFFCCCC }
        }
        pub(crate) fn status_text(&self) -> u32 {
            if self.is_dark { 0xFFFF9999 } else { 0xFFCC0000 }
        }
        pub(crate) fn rgba_bg(&self) -> [u8; 4] {
            let a = ((self.base_100 >> 24) & 0xFF) as u8;
            let r = ((self.base_100 >> 16) & 0xFF) as u8;
            let g = ((self.base_100 >> 8) & 0xFF) as u8;
            let b = (self.base_100 & 0xFF) as u8;
            [r, g, b, a]
        }
        pub(crate) fn rgba_lib_bg(&self) -> [u8; 4] {
            let a = ((self.base_200 >> 24) & 0xFF) as u8;
            let r = ((self.base_200 >> 16) & 0xFF) as u8;
            let g = ((self.base_200 >> 8) & 0xFF) as u8;
            let b = (self.base_200 & 0xFF) as u8;
            [r, g, b, a]
        }
    }
}

#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("pdf_android"),
    );

    let mut reader = Reader::new(&app);
    let mut running = true;
    crate::jni::enter_immersive(&app);
    crate::jni::keep_screen_on(&app);

    while running {
        // Timeout del poll SIEMPRE con ventana (16 ms → tick por vsync):
        // con `poll_events(None)` (reposo) el looper de android-activity
        // puede quedarse dormido y PERDER los toques del visor (bug medido
        // en la TCL: el keyevent despertaba pero el touch no — la app
        // "se quedaba pillada"). El tick es ligero (~µs) y el coste de
        // batería de 60 wakeups/s es despreciable frente a la robustez.
        // Sin ventana (o sin activity), el poll sigue bloqueante (guard).
        let timeout = if reader.has_window() {
            Some(std::time::Duration::from_millis(16))
        } else {
            None
        };
        app.poll_events(timeout, |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                info!("InitWindow");
                reader.status_bar_top = app.content_rect().top;
                info!("status_bar_top: {}", reader.status_bar_top);
                if let Some(win) = app.native_window() {
                    reader.init_window(win);
                }
                // NO re-aplicar keep_screen_on aquí: getWindow().addFlags
                // desde el hilo android_main lanza "Only the original thread
                // that created a view hierarchy can touch its views" (la
                // ventana la crea el UI thread Java); la llamada del arranque
                // (before del bucle, en onCreate aún sin jerarquía) aplica el
                // flag al objeto Window y persiste (verificado: dumpsys
                // window → fl=KEEP_SCREEN_ON).
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                info!("TerminateWindow");
                reader.terminate_window();
            }
            PollEvent::Main(MainEvent::WindowResized { .. }) => {
                info!("WindowResized");
                reader.status_bar_top = app.content_rect().top;
                // Re-obtener el handle: tras resize/recreate puede estar stale
                // (ver nota de refresco en la cabecera del módulo).
                if let Some(win) = app.native_window() {
                    reader.set_window(win);
                }
                reader.redraw();
            }
            PollEvent::Main(MainEvent::RedrawNeeded { .. }) => {
                if let Some(win) = app.native_window() {
                    reader.set_window(win);
                }
                reader.redraw();
            }
            PollEvent::Main(MainEvent::InputAvailable) => {
                handle_input(&app, &mut reader);
            }
            PollEvent::Main(MainEvent::Resume { .. }) => {
                info!("Resume");
                // Biblioteca CURADA: el Resume NO re-consulta MediaStore (la
                // rejilla sale de `internal/library.json`); solo se limpia la
                // marca "pendiente de conceder permiso" — el selector de
                // añadir re-comprueba el permiso por sí mismo al invocarse.
                reader.grant_pending = false;
            }
            PollEvent::Main(MainEvent::Destroy) => {
                info!("Destroy: saliendo del bucle");
                running = false;
            }
            PollEvent::Wake | PollEvent::Timeout | PollEvent::Main(_) => {
                // Trabajo diferido: animación del sheet + lote de portadas.
                // (Wake/Tiemout solo llegan con `poll_events(Some(..))`, es
                // decir, mientras `sheet_animating()` o `thumbs_pending()`.)
                reader.tick(&app);
            }
            _ => {} // PollEvent es #[non_exhaustive]: cubrir futuras variantes
        });
        // Coalescing por vsync (Fase C, comparativa saber-notes): UN blit por
        // iteración si `update_tool_gesture` marcó repintar. Los Moves del
        // boli llegan a 120 Hz pero la pantalla solo presenta a 60 Hz —
        // blitear por evento bloquearía el BufferQueue (~16 ms por
        // unlock_and_post) y encadenaría el lag. Con coalescing: latencia ≤1
        // frame y coste por frame = dirty rect (< 1 ms).
        if reader.take_repaint() {
            reader.blit();
        }
    }

    info!("android_main: fin");
}
