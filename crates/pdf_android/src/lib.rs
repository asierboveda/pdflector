//! pdf_android — spike de Fase 1: ver un PDF renderizado en la tablet.
//!
//! App Android nativa mínima (backend `native-activity` de android-activity;
//! NativeActivity clásica, el `.so` se carga vía `android.app.lib_name`).
//!
//! - Al arrancar SIN intent (lanzamiento normal desde el launcher) muestra la
//!   **biblioteca MediaStore**: los PDFs del sistema con CARPETA y NOMBRE
//!   (p. ej. "Download/test.pdf" o "Document/Mates/3S/Fisica/TEMA_1.pdf");
//!   tocar una fila copia el fichero al almacenamiento interno y lo abre con
//!   `pdf_core` (motor MuPDF, ADR-001).
//! - Con "abrir con" (ACTION_VIEW + content://) abre el PDF directamente sin
//!   pasar por la biblioteca (ver `launch_intent_pdf`).
//! - En cada redibujado renderiza la página actual a una escala *contain*
//!   (`min(win_w / page_w, win_h / page_h)`, puntos PDF → px) multiplicada por
//!   el factor de zoom continuo (1.0 = página completa), centrada sobre un
//!   fondo oscuro (letterbox).
//! - Blitea el `Bitmap` RGBA8 de pdf_core al `ANativeWindow` fila a fila,
//!   respetando `buffer.stride` (en píxeles, puede ser > `width`). El formato
//!   del buffer se fuerza a `R8G8B8A8_UNORM` con
//!   `ANativeWindow_setBuffersGeometry`; por defensa se manejan también
//!   `R8G8B8X8_UNORM` (copia directa) y `R5G6B5_UNORM` (conversión 565).
//! - Input multitáctil (`input_events_iter()`, que ya funcionaba para el tap):
//!   - arrastre vertical → scroll continuo (el documento es una columna de
//!     páginas apiladas; ver sección "Scroll vertical continuo" abajo);
//!   - barrido horizontal / tap derecha-izquierda → página (fallback);
//!   - pinch con dos dedos → zoom SOLO por pinch: durante el gesto solo se
//!     actualiza el factor y se blitea el bitmap cacheado (sin re-render),
//!     y al soltar se re-renderizan las páginas visibles UNA vez a la
//!     resolución final.
//!
//! ## Scroll vertical continuo + caché de páginas (2026-08-13)
//!
//! El visor sustituyó el salto de página (re-render ~18-25 ms por página) por
//! un scroll vertical continuo: el documento se trata como una columna de
//! páginas apiladas (alto = Σ alto(página) × escala + gap; `reader` mantiene
//! `scroll_y`, el layout de la columna y el rango de páginas visibles).
//!
//! - `cache.rs` (`PageCache`): LRU en RAM de páginas renderizadas
//!   (página → `Bitmap`), limitada por bytes (48 MiB) y por entradas (5),
//!   coherente con el RSS < 150 MB; evita el re-render al volver atrás.
//!   Los bitmaps se guardan SIEMPRE normales: la inversión de modo oscuro se
//!   aplica al blitear (`draw::blit_stacked`), por página.
//! - Render (vía caché) de las páginas visibles + 1 vecina (prefetch simple):
//!   el blit dibuja cada página en su posición de la columna
//!   (offset acumulado − scroll_y), recortando a la ventana con un solo
//!   lock+present (`draw::blit_stacked`, vecino-más-cercano para el zoom).
//! - El pinch sigue igual (zoom continuo; re-render nítido al soltar); el
//!   scroll NO cambia de página. La página visible alimenta el indicador
//!   "N / total", los saltos ±10 y la persistencia (`persist`), que sigue
//!   guardando la página visible como `page` del estado.
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
//! ## Índice de letras de la biblioteca (filtro, 2026-08-XX)
//!
//! La biblioteca añade una tira vertical de letras en el borde derecho
//! (A-Z + '#'), sin IME (el teclado en pantalla es complejo en
//! android-activity nativo): tocar una letra FILTRA la lista a las entradas
//! cuyo DISPLAY_NAME empieza por esa letra normalizada (`reader::normalize_letter`:
//! minúsculas, acentos → letra base — á→a, é→e, í→i, ó→o, ú→u, ü→u, ñ→n, ç→c —
//! y números/símbolos → '#'); tocar de nuevo la letra activa (o un Rescan)
//! quita el filtro. Las celdas sin entradas se atenúan y la activa se
//! resalta; la geometría la comparten `draw::render_library_list` e
//! `input::library_tap` (helpers `reader::lib_strip_*`).
//!
//! La agrupación por carpeta con encabezados colapsables NO se implementa en
//! esta iteración (decisión documentada): MediaStore ya ordena por
//! `relative_path, display_name` (las carpetas quedan agrupadas) y cada fila
//! muestra su carpeta en la segunda línea; un colapso/expansión exigiría una
//! indirección fila-visual→entrada en tap, scroll y clamp (más estado por
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
//!   oscuro; ver la sección "Visor: barra superior, estado y modo oscuro").
//! - `annotations`: estado de dibujo (trazo en curso, paleta de colores); el
//!   modelo y la persistencia (sidecar SQLite) viven en pdf_core.
//!
//! Aquí queda SOLO `android_main`, el bucle de eventos, las constantes
//! compartidas (`pub(crate)`) y los `mod`.
//!
//! ## Anotaciones a mano (modo dibujo, 2026-08-16)
//!
//! La barra superior del visor añade el botón "✏️" que activa/desactiva el
//! **modo dibujo**: con él activo, el arrastre con un dedo crea un `Stroke`
//! (polilínea) en coordenadas de página en vez de hacer scroll; al levantar
//! el dedo, el trazo se añade al `AnnotationSet` y se guarda en el sidecar
//! SQLite del PDF (`store::sidecar_path` → `<pdf-dir>/annotations/<stem>.db`;
//! ver `Reader::load_annotations`). Los trazos se dibujan como capa vectorial
//! sobre el bitmap ya bliteado (Bresenham, grosor `width × scale`), nunca
//! rasterizados en el bitmap cacheado (AGENTS.md §4.3). Detalles en
//! `annotations.rs` (estado de dibujo), `reader.rs` (transformación
//! pantalla→página y persistencia) y `draw.rs` (render de la capa).
//!
//! ## Visor: barra superior, estado persistido y modo oscuro (2026-08-XX)
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
//! 2. **Indicador de página**: barra superior del visor (overlay opaco único
//!    bliteado en (0,0) — no hace falta tocar `zoom.rs`) con Open, modo
//!    dibujo ✏️, color del trazo ●, undo ↶, saltos −10/+10, "N / total" (tap
//!    en el indicador = página siguiente) y el toggle "Dark"/"Light".
//!    Renderizado con el mismo Canvas+JNI de
//!    `draw.rs`; la geometría DEBE coincidir con las zonas de tap de
//!    `input.rs`.
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
mod reader;
mod view;
mod zoom;

use crate::input::handle_input;
use crate::reader::{Reader, UiMode};

/// Color de fondo letterbox (gris oscuro, opaco).
pub(crate) const BACKGROUND: [u8; 4] = [0x26, 0x26, 0x26, 0xFF];
/// Fondo letterbox en modo oscuro (negro puro: la página invertida es negra
/// y el fondo se funde con ella al hacer zoom-out).
pub(crate) const DARK_BG: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
/// Fondo cuando no se pudo abrir el PDF (rojo apagado, visible en pantalla).
pub(crate) const ERROR_BG: [u8; 4] = [0x5A, 0x12, 0x12, 0xFF];

/// Fracción del ancho (eje x) o del alto (eje y) de la ventana que debe
/// recorrer un swipe para cambiar de página. 25 % ≈ 360 px en x / 550 px en y
/// en la tablet: claramente por encima del tap slop, sin exigir un barrido
/// completo (el ">40 %" del enunciado era un ejemplo).
pub(crate) const SWIPE_FRACTION: f32 = 0.25;
/// Radio (px) de movimiento máximo entre Down y Up para considerar el gesto un
/// "tap" (no un swipe). ~20 px a 320 dpi (ViewConfiguration touch slop ≈ 8 dp).
pub(crate) const TAP_SLOP: f32 = 24.0;
/// Límites del factor de zoom continuo (1.0 = página completa).
pub(crate) const PINCH_MIN: f32 = 0.25;
pub(crate) const PINCH_MAX: f32 = 8.0;
/// Alto de la barra superior del visor (fracción de la ventana): contiene
/// Open, ✏️ (modo dibujo), ● (color del trazo), ↶ (undo), saltos −10/+10, el
/// indicador "N / total" y el toggle de modo oscuro.
pub(crate) const VIEWER_BAR_H_DIV: i32 = 16;
/// Región del botón "Open" en la barra superior (izquierda).
pub(crate) const OPEN_BTN_W_DIV: i32 = 10;
/// Región del botón "✏️" (modo dibujo) en la barra superior.
pub(crate) const PENCIL_BTN_W_DIV: i32 = 10;
/// Región del botón "●" (alternar color del trazo) en la barra superior.
pub(crate) const COLOR_BTN_W_DIV: i32 = 10;
/// Región del botón "↶" (undo: quitar el último trazo) en la barra superior.
pub(crate) const UNDO_BTN_W_DIV: i32 = 10;
/// Región del botón "Dark" en la barra superior (derecha).
pub(crate) const DARK_BTN_W_DIV: i32 = 10;
/// Región de los botones de salto −10/+10 (a cada lado del indicador).
pub(crate) const JUMP_BTN_W_DIV: i32 = 14;
/// Ancho de la tira de letras (índice A-Z + '#') de la biblioteca, como
/// fracción del ancho de la ventana (el resto es la zona de lista).
pub(crate) const LIB_STRIP_W_DIV: i32 = 20;

#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("pdf_android"),
    );

    let mut reader = Reader::new(&app);
    let mut running = true;

    while running {
        app.poll_events(None, |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                info!("InitWindow");
                if let Some(win) = app.native_window() {
                    reader.init_window(win);
                }
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                info!("TerminateWindow");
                reader.terminate_window();
            }
            PollEvent::Main(MainEvent::WindowResized { .. }) => {
                info!("WindowResized");
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
                // Al volver de Ajustes tras conceder "All files access",
                // re-consultar MediaStore sin esperar a un Rescan manual.
                if reader.mode == UiMode::Library && reader.grant_pending {
                    reader.rescan_library(&app);
                }
            }
            PollEvent::Main(MainEvent::Destroy) => {
                info!("Destroy: saliendo del bucle");
                running = false;
            }
            PollEvent::Wake | PollEvent::Timeout | PollEvent::Main(_) => {}
            _ => {} // PollEvent es #[non_exhaustive]: cubrir futuras variantes
        });
    }

    info!("android_main: fin");
}
