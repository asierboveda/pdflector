//! Estado de la app y lógica del visor/picker/biblioteca (`struct Reader`).
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): aquí vive TODO
//! el estado de UI (`Reader`), los tipos de lista y los helpers de layout del
//! picker. El input (gestos) está en `input`, el dibujo en `draw`, el JNI en
//! `jni`, la escala inicial en `view` (stub) y el blit rápido en `zoom` (stub).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use android_activity::AndroidApp;
use android_activity::ndk::hardware_buffer_format::HardwareBufferFormat;
use android_activity::ndk::native_window::NativeWindow;
use log::{error, info, warn};
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::store::{AnnotationStore, sidecar_path};
use pdf_core::{Annotation, AnnotationSet, Bitmap, Color, Document, RenderEngine, Stroke};

use crate::annotations::{ActiveStroke, DEFAULT_STROKE_COLOR, STROKE_PALETTE};
use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES, PageCache};
use crate::draw::{
    PageAnnots, PageBlit, blit_stacked, render_library_list, render_picker_list, render_viewer_bar,
};
use crate::input::GestureState;
use crate::jni::{
    android_sdk_int, launch_intent_pdf, query_media_store, read_content_uri_bytes,
    sanitize_pdf_name,
};
use crate::persist;
use crate::view::initial_scale;
use crate::zoom::blit_fast;
use crate::{BACKGROUND, DARK_BG, ERROR_BG, PINCH_MAX, PINCH_MIN};

/// Separación vertical (px) entre páginas de la columna del scroll continuo y
/// margen superior/inferior del documento (el alto total de la columna es
/// `PAGE_GAP + Σ(alto(página) + PAGE_GAP)`).
const PAGE_GAP: i32 = 8;

/// Un PDF externo recibido por "abrir con" (ACTION_VIEW) al lanzar la app.
/// Construido en `jni::launch_intent_pdf`, consumido en `Reader::new`.
pub(crate) struct LaunchPdf {
    /// Nombre mostrable del fichero (log/status).
    pub(crate) name: String,
    /// URI original recibida (log).
    pub(crate) source: String,
    /// Ruta local abrible con `MupdfEngine::open` (ya copiada si content://).
    pub(crate) path: String,
}

/// Modo de UI actual de la app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UiMode {
    /// Visor de página (render + gestos existentes).
    Viewer,
    /// Picker: lista de PDFs de los directorios de la app (fallback interno).
    Picker,
    /// Biblioteca: PDFs del sistema vía MediaStore (carpeta + nombre).
    Library,
}

/// Una entrada de la lista del picker.
pub(crate) struct PdfEntry {
    /// Nombre de fichero (para mostrar y loguear).
    pub(crate) name: String,
    /// Ruta absoluta (se abre con `MupdfEngine::open`).
    pub(crate) path: String,
    /// Tamaño en bytes (se muestra formateado).
    pub(crate) size: u64,
    /// Etiqueta del directorio de origen ("internal" / "external").
    pub(crate) source: &'static str,
}

/// Una entrada de la biblioteca MediaStore (PDF del sistema).
#[derive(Clone)]
pub(crate) struct LibraryEntry {
    /// DISPLAY_NAME (nombre mostrable del fichero).
    pub(crate) name: String,
    /// RELATIVE_PATH (carpeta, p. ej. "Download/" o "Document/Mates/3S/");
    /// vacío si el proveedor no la expone (API < 29) o es la raíz.
    pub(crate) folder: String,
    /// content:// URI (`ContentUris.withAppendedId(files_uri, _ID)`).
    pub(crate) uri: String,
    /// Tamaño en bytes (`_SIZE`, 0 si no disponible).
    pub(crate) size: i64,
}

/// Resultado de una consulta a MediaStore: lista + estado del permiso y del
/// error (para el mensaje de estado de la biblioteca). Construido en
/// `jni::query_media_store`, consumido en `Reader::rescan_library`.
pub(crate) struct LibraryScan {
    pub(crate) entries: Vec<LibraryEntry>,
    /// ¿Concedido el acceso a todos los archivos (API 30+) o no requerido (≤ 12)?
    pub(crate) permission_granted: bool,
    /// Error de consulta mostrable (None si OK).
    pub(crate) error: Option<String>,
}

/// Escanea los directorios de la app buscando `*.pdf` para el picker:
/// `internal_data_path()` y `external_data_path()`, en cada uno la raíz y el
/// subdirectorio `pdfs/`. Ordena por nombre (case-insensitive) y deduplica
/// por ruta.
fn scan_pdfs(app: &AndroidApp) -> Vec<PdfEntry> {
    fn push_dir(dir: &Path, source: &'static str, out: &mut Vec<PdfEntry>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let path = e.path();
            let is_pdf = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));
            if !is_pdf {
                continue;
            }
            out.push(PdfEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                path: path.display().to_string(),
                size: e.metadata().map(|m| m.len()).unwrap_or(0),
                source,
            });
        }
    }
    let mut entries = Vec::new();
    for (base, source) in [
        (app.internal_data_path(), "internal"),
        (app.external_data_path(), "external"),
    ] {
        if let Some(base) = base {
            push_dir(&base, source, &mut entries);
            push_dir(&base.join("pdfs"), source, &mut entries);
        }
    }
    entries.sort_by_key(|a| a.name.to_lowercase());
    let mut seen = HashSet::new();
    entries.retain(|e| seen.insert(e.path.clone()));
    entries
}

/// Alto (px) de cada fila del picker, proporcional a la ventana.
pub(crate) fn picker_row_h(win_h: i32) -> i32 {
    (win_h / 26).max(48)
}

/// Alto (px) de la cabecera del picker (título + botones).
pub(crate) fn picker_header_h(win_h: i32) -> i32 {
    picker_row_h(win_h) * 3 / 2
}

/// Ancho (px) de los botones de la cabecera del picker.
pub(crate) fn picker_btn_w(win_w: i32) -> i32 {
    win_w / 4
}

/// Alto (px) de los botones de la cabecera del picker.
pub(crate) fn picker_btn_h(win_h: i32) -> i32 {
    picker_row_h(win_h) * 4 / 5
}

/// Nº de filas visibles en el picker (depende de si hay mensaje de estado).
pub(crate) fn picker_visible_rows(win_h: i32, has_status: bool) -> usize {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    ((win_h - picker_header_h(win_h) - status_h) / picker_row_h(win_h)).max(0) as usize
}

/// Formatea un tamaño de fichero (B/KB/MB) para la lista.
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Trunca un nombre a `max_chars` caracteres añadiendo "…" si hace falta
/// (Canvas no hace ellipsis automática; la anchura por carácter es una
/// estimación).
pub(crate) fn truncate_name(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

/// Normaliza la primera letra de un nombre para el índice de la biblioteca
/// (filtro por letra inicial, sin IME): minúsculas (A→a), diacríticos →
/// letra base sin acento (á→a, é→e, í→i, ó→o, ú→u, ü→u, ñ→n, ç→c) y todo lo
/// no alfabético (números, signos, espacios, nombres que empiezan por "_")
/// → '#'. El bucket resultante es siempre 'a'..='z' o '#', el mismo espacio
/// que pinta la tira de letras (`lib_strip_letter`).
pub(crate) fn normalize_letter(s: &str) -> char {
    let Some(c) = s.chars().next() else {
        return '#';
    };
    let lower = c.to_lowercase().next().unwrap_or('#');
    match lower {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        'a'..='z' => lower,
        _ => '#',
    }
}

/// Ancho (px) de la tira de letras del índice de la biblioteca: `win_w /
/// LIB_STRIP_W_DIV`. La zona de lista mide `win_w − lib_strip_w(win_w)`;
/// la comparten render (`draw::render_library_list`) y tap
/// (`input::library_tap`).
pub(crate) fn lib_strip_w(win_w: i32) -> i32 {
    win_w / crate::LIB_STRIP_W_DIV
}

/// Alto (px) de cada celda de la tira de letras: reparte el espacio entre el
/// borde superior de la lista (`rows_y0` = cabecera + franja de estado) y el
/// borde inferior de la ventana entre las 27 celdas (A-Z + '#').
pub(crate) fn lib_strip_cell_h(win_h: i32, rows_y0: i32) -> f32 {
    ((win_h - rows_y0) as f32 / 27.0).max(1.0)
}

/// Índice de celda de la tira (0..=26) bajo el punto de ventana `y`, o None
/// si cae fuera de la tira (por encima de `rows_y0`).
pub(crate) fn lib_strip_cell(win_h: i32, rows_y0: i32, y: f32) -> Option<usize> {
    if y < rows_y0 as f32 {
        return None;
    }
    let i = ((y - rows_y0 as f32) / lib_strip_cell_h(win_h, rows_y0)) as usize;
    (i < 27).then_some(i)
}

/// Letra de la celda `i` de la tira (0-25 = 'A'..='Z', 26 = '#').
pub(crate) fn lib_strip_letter(i: usize) -> char {
    if i < 26 {
        (b'A' + i as u8) as char
    } else {
        '#'
    }
}

/// Índice de celda de un bucket normalizado ('a'..='z' | '#'): 0-25 o 26.
pub(crate) fn lib_letter_index(letter: char) -> usize {
    match letter {
        'a'..='z' => (letter as u8 - b'a') as usize,
        _ => 26,
    }
}

/// Estado de la app, vivo durante todo el bucle de `android_main`.
/// `pub(crate)` por la partición de `lib.rs`: `input` y `draw` leen campos,
/// `lib` llama a los métodos (gestos y listas viven en otros módulos).
pub(crate) struct Reader {
    pub(crate) doc: Option<MupdfDocument>,
    /// Página visible actual, 0-based (la primera página que toca el borde
    /// superior del viewport en el scroll). Alimenta el indicador "N / total",
    /// los saltos ±10 y la persistencia.
    pub(crate) page: u32,
    /// Referencia owned al ANativeWindow (Some entre InitWindow y TerminateWindow).
    window: Option<NativeWindow>,
    /// Bitmap de la LISTA del picker/biblioteca (render de pantalla completa
    /// con Canvas+JNI). Las páginas del visor viven en `cache` (PageCache);
    /// este campo solo lo usan los modos Picker/Library.
    pub(crate) bitmap: Option<Bitmap>,
    /// Caché LRU de páginas renderizadas (página → Bitmap) para el scroll
    /// vertical continuo: evita re-renderizar al volver atrás. Guarda SIEMPRE
    /// bitmaps normales; la inversión de modo oscuro se aplica al blitear
    /// (`draw::blit_stacked`).
    cache: PageCache,
    /// Zoom con el que están renderizados los bitmaps de la caché (1.0 =
    /// escala *cover* base; el re-render nítido al soltar el pinch pone
    /// `rendered_zoom = self.zoom`). El blit usa el zoom RELATIVO
    /// `zoom / rendered_zoom`: 1:1 nítido para bitmaps recién renderizados,
    /// escala vecino-más-cercano del bitmap viejo durante el pinch.
    rendered_zoom: f32,
    /// Factor de zoom continuo (1.0 = página completa *cover*).
    pub(crate) zoom: f32,
    /// Desplazamiento manual del viewport en px (0 por ahora; el zoom se
    /// centra en la columna y el blit recorta lo que excede la ventana).
    pan_x: i32,
    pan_y: i32,
    /// Desplazamiento del bitmap de la LISTA dentro del buffer (picker/
    /// biblioteca; 0 por ahora).
    offset_x: i32,
    offset_y: i32,
    /// Scroll vertical continuo: offset del viewport en px del documento
    /// (0 = borde superior de la primera página, que empieza tras un gap de
    /// `PAGE_GAP` px). Clamp a `[0, doc_height - win_h]`. Leído por `input`
    /// (base del arrastre) y escrito por `scroll_to`.
    pub(crate) scroll_y: f32,
    /// El layout de la columna (`page_offsets`/`page_heights`/`doc_height`)
    /// está obsoleto: reconstruirlo en el próximo redraw (cambió el zoom, la
    /// ventana o el documento).
    layout_dirty: bool,
    /// Página pendiente de scroll (salto de página con el layout sin
    /// construir, p. ej. al restaurar el estado antes del primer InitWindow).
    pending_page: Option<u32>,
    /// Borde superior de cada página (px, en espacio del documento),
    /// construido por `rebuild_layout` a la escala de la caché.
    page_offsets: Vec<i32>,
    /// Alto renderizado de cada página (px), paralelo a `page_offsets`.
    page_heights: Vec<i32>,
    /// Alto total de la columna (gap superior + Σ(alto + gap)).
    doc_height: i32,
    /// Dimensiones actuales de la ventana (px).
    pub(crate) win_w: i32,
    pub(crate) win_h: i32,
    /// Máquina de gestos (swipe/pinch/tap).
    pub(crate) gesture: GestureState,
    /// Modo de UI actual (visor de página o picker de PDFs).
    pub(crate) mode: UiMode,
    /// PDFs encontrados en los directorios de la app (picker).
    pub(crate) pdf_list: Vec<PdfEntry>,
    /// PDFs del sistema devueltos por MediaStore (biblioteca).
    pub(crate) library_list: Vec<LibraryEntry>,
    /// Filtro activo de la biblioteca por letra inicial normalizada
    /// ('a'..='z' | '#', ver `normalize_letter`); None = mostrar todas.
    pub(crate) library_filter: Option<char>,
    /// Índices (sobre `library_list`) de las entradas que pasan el filtro
    /// activo: la lista visible del modo Library. Se reconstruye al filtrar
    /// (`set_library_filter`) y al re-consultar (`rescan_library`); render y
    /// tap la recorren con `library_entry_at` (O(1)).
    pub(crate) library_filtered: Vec<usize>,
    /// ¿Concedido el acceso a todos los archivos (API 30+) o asumido (≤ 12)?
    pub(crate) permission_granted: bool,
    /// Nivel de API (Build.VERSION.SDK_INT): decide columnas y permisos.
    pub(crate) sdk_int: i32,
    /// ¿Pendiente de volver de Ajustes tras pulsar Grant? (re-consultar en Resume).
    pub(crate) grant_pending: bool,
    /// Desplazamiento del picker/biblioteca en filas (scroll).
    pub(crate) list_scroll: usize,
    /// La lista del picker necesita re-render (rescan, scroll, resize).
    pub(crate) list_dirty: bool,
    /// Mensaje de estado del picker (bajo la cabecera; p. ej. error de open).
    pub(crate) status: Option<String>,
    /// Ruta local del PDF abierto (clave del estado persistido; None si no
    /// hay documento). Se setea en `open_pdf` y en el "abrir con" del arranque.
    pub(crate) doc_path: Option<String>,
    /// Directorio interno de la app (para `state.json`; ver `persist`).
    internal_dir: Option<PathBuf>,
    /// Modo oscuro activo (página invertida + fondo negro).
    pub(crate) dark: bool,
    /// Bitmap de la barra superior del visor (Open / ✏️ / ● / ↶ / −10 /
    /// "N / total" / +10 / Dark), cacheado: se invalida al cambiar ventana,
    /// página o modo.
    viewer_bar: Option<Bitmap>,
    /// Estado del arrastre del picker: (x, y, scroll) en el Down.
    pub(crate) picker_drag: Option<(f32, f32, usize)>,
    /// Anotaciones del documento abierto: se cargan del sidecar SQLite al
    /// abrir (`load_annotations`) y se guardan al añadir/quitar un trazo
    /// (`save_annotations`). El modelo vive en pdf_core (AGENTS.md §4.3).
    pub(crate) annotations: AnnotationSet,
    /// Ruta del sidecar del documento abierto (`store::sidecar_path`:
    /// `<pdf-dir>/annotations/<stem>.db`); None sin documento. El sidecar de
    /// un PDF abierto por content:// (biblioteca o "abrir con") queda junto
    /// a la copia en `internal/pdfs/` → `internal/pdfs/annotations/<stem>.db`
    /// (ver `open_library_entry`/`jni::launch_intent_pdf`).
    annot_sidecar: Option<PathBuf>,
    /// Modo dibujo activo: el arrastre con un dedo crea un trazo en vez de
    /// hacer scroll (toggle con el botón "✏️" de la barra).
    pub(crate) draw_mode: bool,
    /// Color de los trazos nuevos (alternable con el botón "●").
    pub(crate) stroke_color: Color,
    /// Trazo en curso (dedo bajado en modo dibujo), en coordenadas de página;
    /// se dibuja como capa y se añade al `AnnotationSet` al levantar el dedo.
    pub(crate) active_stroke: Option<ActiveStroke>,
}

impl Reader {
    pub(crate) fn new(app: &AndroidApp) -> Self {
        let mut reader = Self {
            doc: None,
            page: 0,
            window: None,
            bitmap: None,
            cache: PageCache::new(CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES),
            rendered_zoom: 1.0,
            zoom: 1.0,
            pan_x: 0,
            pan_y: 0,
            offset_x: 0,
            offset_y: 0,
            scroll_y: 0.0,
            layout_dirty: true,
            pending_page: None,
            page_offsets: Vec::new(),
            page_heights: Vec::new(),
            doc_height: 0,
            win_w: 0,
            win_h: 0,
            gesture: GestureState::new(),
            mode: UiMode::Library,
            pdf_list: Vec::new(),
            library_list: Vec::new(),
            library_filter: None,
            library_filtered: Vec::new(),
            permission_granted: false,
            sdk_int: android_sdk_int(),
            grant_pending: false,
            list_scroll: 0,
            list_dirty: true,
            status: None,
            doc_path: None,
            internal_dir: app.internal_data_path(),
            dark: false,
            viewer_bar: None,
            picker_drag: None,
            annotations: AnnotationSet::new(),
            annot_sidecar: None,
            draw_mode: false,
            stroke_color: DEFAULT_STROKE_COLOR,
            active_stroke: None,
        };
        match launch_intent_pdf(app) {
            // "Abrir con" (ACTION_VIEW): el PDF se abre directamente, sin pasar
            // por la biblioteca. Si falla, se cae al picker interno con el
            // motivo como estado (comportamiento previo al spike de biblioteca).
            Some(lp) => {
                info!("open-with intent: {} ({})", lp.name, lp.source);
                let engine = match MupdfEngine::new() {
                    Ok(e) => e,
                    Err(e) => {
                        // Prácticamente infalible (solo falla ante fallo catastrófico
                        // del allocator); si ocurriera seguimos sin motor.
                        error!("MupdfEngine::new: {e}");
                        MupdfEngine
                    }
                };
                match engine.open(Path::new(&lp.path)) {
                    Ok(doc) => {
                        info!("opened: {} pages", doc.page_count());
                        reader.doc = Some(doc);
                        reader.doc_path = Some(lp.path.clone());
                        reader.mode = UiMode::Viewer;
                        // Anotaciones del documento (sidecar SQLite; set vacío
                        // si no existe o está corrupto).
                        reader.load_annotations(&lp.path);
                        // Registrar también el "abrir con": el próximo arranque
                        // sin intent restaurará este PDF en su última posición.
                        reader.save_state();
                    }
                    Err(e) => {
                        error!("cannot open {}: {e}", lp.path);
                        reader.mode = UiMode::Picker;
                        reader.status = Some(format!("Cannot open {}", lp.name));
                        reader.pdf_list = scan_pdfs(app);
                    }
                }
            }
            // Lanzamiento normal sin intent. Estado persistido (`persist`): si
            // el PDF guardado sigue accesible, se abre directamente en su
            // página/zoom/modo oscuro; si ya no existe (o no se puede abrir),
            // se BORRA el estado y se abre la biblioteca MediaStore
            // (rescan_library cae al picker interno si MediaStore está vacía).
            None => {
                let restored =
                    if let Some(state) = persist::load_state(reader.internal_dir.as_deref()) {
                        // Solo restaurar si el PDF sigue accesible: `open_pdf`
                        // falla si no se puede abrir (corrupto) y deja el
                        // estado intacto.
                        if Path::new(&state.path).exists() && reader.open_pdf(&state.path) {
                            let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                            reader.page = state.page.min(pages.saturating_sub(1));
                            reader.zoom = state.zoom.clamp(PINCH_MIN, PINCH_MAX);
                            reader.rendered_zoom = reader.zoom;
                            reader.dark = state.dark;
                            reader.pending_page = Some(reader.page); // scroll a la página restaurada
                            reader.layout_dirty = true; // layout nuevo (aún sin tamaño de ventana)
                            reader.cache.clear();
                            reader.viewer_bar = None; // indicador de la página restaurada
                            info!(
                                "restored {} @page {} zoom {:.3} dark {}",
                                state.path,
                                reader.page + 1,
                                reader.zoom,
                                reader.dark
                            );
                            reader.save_state();
                            reader.redraw();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                if !restored {
                    // Sin estado (primer arranque) o PDF ya no accesible:
                    // limpiar el estado huérfano y abrir la biblioteca.
                    persist::clear_state(reader.internal_dir.as_deref());
                    reader.rescan_library(app);
                }
            }
        }
        reader
    }

    /// Sustituye el handle de ventana por el actual y re-fuerza el formato del
    /// buffer. `app.native_window()` devuelve siempre el window vigente de la
    /// glue de NativeActivity; tras una recreación de la surface es un
    /// `ANativeWindow` NUEVO que necesita `set_buffers_geometry` otra vez.
    pub(crate) fn set_window(&mut self, window: NativeWindow) {
        if let Err(e) =
            window.set_buffers_geometry(0, 0, Some(HardwareBufferFormat::R8G8B8A8_UNORM))
        {
            warn!("set_buffers_geometry(R8G8B8A8_UNORM): {e}");
        }
        self.window = Some(window);
    }

    /// `InitWindow`: nueva ventana lista. Fuerza buffers RGBA8888 (0,0 =
    /// conservar tamaño base; solo cambia el formato) e invalida la caché.
    pub(crate) fn init_window(&mut self, window: NativeWindow) {
        self.set_window(window);
        self.bitmap = None;
        self.viewer_bar = None;
        self.list_dirty = true;
        // Nueva ventana → posible nueva escala cover: reconstruir el layout.
        // Las páginas de la caché se reutilizan si el tamaño no cambió; el
        // redraw detecta el cambio de `win_w/h` y limpia la caché si hace falta.
        self.layout_dirty = true;
        self.redraw();
    }

    /// `TerminateWindow`: soltar la ventana (drop → `ANativeWindow_release`).
    pub(crate) fn terminate_window(&mut self) {
        self.window = None;
        self.bitmap = None;
        self.viewer_bar = None;
        self.list_dirty = true;
    }

    /// Redibuja: re-render si cambió página, zoom o tamaño de ventana, y blit.
    pub(crate) fn redraw(&mut self) {
        let (w, h) = match self.window.as_ref() {
            Some(win) => (win.width(), win.height()),
            None => return,
        };
        if w <= 0 || h <= 0 {
            return;
        }
        if w != self.win_w || h != self.win_h {
            self.win_w = w;
            self.win_h = h;
            self.bitmap = None; // lista del picker → re-render
            self.cache.clear(); // nueva escala cover → los bitmaps viejos no sirven
            self.layout_dirty = true;
            self.list_dirty = true;
            self.viewer_bar = None;
        }
        match self.mode {
            UiMode::Viewer => {
                // Layout de la columna (reconstrucción O(páginas), solo al
                // cambiar zoom/ventana/documento — NUNCA durante el scroll).
                if self.layout_dirty {
                    self.rebuild_layout();
                    self.layout_dirty = false;
                }
                // Salto de página pendiente (p. ej. restauración del estado):
                // alinear el scroll con el borde superior de la página.
                if let Some(p) = self.pending_page.take() {
                    let n = self.page_offsets.len();
                    if n > 0 {
                        self.scroll_y = self.page_offsets[p.min(n as u32 - 1) as usize] as f32;
                    }
                }
                // Clamp tras layout y salto pendiente (documento más corto que
                // la ventana → scroll 0; última página → sin sobrepasar).
                self.clamp_scroll();
                // Páginas visibles + 1 vecina (prefetch simple), vía caché.
                self.ensure_pages_rendered();
                // Indicador "N / total": la página visible se persiste al
                // cruzar bordes de página durante el scroll.
                if self.update_page_from_scroll() {
                    self.save_state();
                }
                // Barra superior cacheada: se re-renderiza solo si cambió la
                // ventana, la página o el modo oscuro (invalidadores abajo).
                if self.doc.is_some() && self.viewer_bar.is_none() {
                    self.viewer_bar = render_viewer_bar(self);
                }
            }
            UiMode::Picker | UiMode::Library => {
                // Clamp del scroll si la lista menguó (rescan) o cambió la ventana.
                let list_len = if self.mode == UiMode::Picker {
                    self.pdf_list.len()
                } else {
                    self.filtered_library_len()
                };
                let visible = picker_visible_rows(self.win_h, self.status.is_some());
                let max_scroll = list_len.saturating_sub(visible);
                if self.list_scroll > max_scroll {
                    self.list_scroll = max_scroll;
                }
                if self.list_dirty {
                    let bmp = if self.mode == UiMode::Picker {
                        render_picker_list(self)
                    } else {
                        render_library_list(self)
                    };
                    if let Some(bmp) = bmp {
                        self.bitmap = Some(bmp);
                        self.offset_x = 0;
                        self.offset_y = 0;
                        self.list_dirty = false;
                    }
                }
            }
        }
        if let Some(win) = self.window.as_ref() {
            self.blit(win);
        }
    }

    /// Reconstruye el layout de la columna de páginas apiladas del scroll
    /// continuo: `page_offsets[p]` = borde superior de la página p en px de
    /// documento, `page_heights[p]` = su alto renderizado y `doc_height` =
    /// alto total (gap superior + Σ(alto + gap)).
    ///
    /// La escala de cada página es `cover(página) × rendered_zoom` (cover
    /// per-page, `view::initial_scale`: cada página llena la pantalla según su
    /// propia proporción — comportamiento de apertura actual). Se reconstruye
    /// SOLO al cambiar zoom, ventana o documento (`layout_dirty`), NUNCA
    /// durante el scroll: el scroll solo mueve `scroll_y`.
    fn rebuild_layout(&mut self) {
        self.page_offsets.clear();
        self.page_heights.clear();
        let mut y = PAGE_GAP;
        if let Some(doc) = self.doc.as_ref() {
            let n = doc.page_count();
            for p in 0..n {
                self.page_offsets.push(y);
                let h = self.page_height_px(p);
                self.page_heights.push(h);
                y += h + PAGE_GAP;
            }
        }
        self.doc_height = y;
    }

    /// Alto en px de la página `page` a la escala de la caché
    /// (`cover × rendered_zoom`). `page_size` devuelve puntos PDF;
    /// `view::initial_scale` aplica la política cover (llenar la pantalla
    /// recortando el exceso por los bordes).
    fn page_height_px(&self, page: u32) -> i32 {
        let Some(doc) = self.doc.as_ref() else {
            return 0;
        };
        let Ok((pw, ph)) = doc.page_size(page) else {
            return 0;
        };
        let scale = initial_scale(pw, ph, self.win_w, self.win_h) * self.rendered_zoom;
        (ph * scale).round().max(1.0) as i32
    }

    /// Ajusta `scroll_y` al rango válido del documento (`[0, doc_height − win_h]`).
    fn clamp_scroll(&mut self) {
        let max = (self.doc_height - self.win_h).max(0);
        self.scroll_y = self.scroll_y.clamp(0.0, max as f32);
    }

    /// Rango de páginas que intersectan el viewport (`[scroll_y, scroll_y +
    /// win_h)` en px de documento). Devuelve `(first, last)` con `last >= first`.
    /// Con el documento vacío o una ventana degenerada devuelve la página más
    /// cercana al borde superior del viewport (evita un rango vacío).
    fn visible_pages(&self) -> (usize, usize) {
        let n = self.page_offsets.len();
        if n == 0 {
            return (0, 0);
        }
        let top = self.scroll_y as i32;
        let bottom = top + self.win_h;
        let mut first = n;
        let mut last = 0usize;
        for i in 0..n {
            let off = self.page_offsets[i];
            if off + self.page_heights[i] > top && off < bottom {
                if i < first {
                    first = i;
                }
                last = i;
            }
        }
        if last >= first {
            return (first, last);
        }
        let mut best = 0usize;
        let mut best_d = i32::MAX;
        for i in 0..n {
            let d = (self.page_offsets[i] - top).abs();
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        (best, best)
    }

    /// Garantiza en la caché las páginas visibles + 1 vecina por cada lado
    /// (prefetch simple): renderiza solo los miss, a `cover × rendered_zoom`
    /// (la escala de la caché), y promueve la recencia LRU de las páginas que
    /// toca. El render es síncrono en el hilo del bucle (~18-25 ms/página en
    /// la tablet); el prefetch adelanta una página para que el scroll entre en
    /// ella sin re-render en el momento de volverse visible.
    fn ensure_pages_rendered(&mut self) {
        let n = self.page_offsets.len();
        if n == 0 {
            return;
        }
        let (first, last) = self.visible_pages();
        let lo = first.saturating_sub(1);
        let hi = (last + 1).min(n - 1);
        for p in lo..=hi {
            let page = p as u32;
            if self.cache.get(page).is_some() {
                continue; // hit: sin re-render (volver atrás es instantáneo)
            }
            let Some(doc) = self.doc.as_ref() else {
                return;
            };
            let (pw, ph) = match doc.page_size(page) {
                Ok(s) => s,
                Err(e) => {
                    error!("page_size {page}: {e}");
                    continue;
                }
            };
            let scale = initial_scale(pw, ph, self.win_w, self.win_h) * self.rendered_zoom;
            let t0 = Instant::now();
            match doc.render_page(page, scale) {
                Ok(bmp) => {
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    info!(
                        "render page {} @scale {scale:.3} -> {}x{} px: {ms:.2} ms (cache: {} pages / {:.1} MiB)",
                        page + 1,
                        bmp.width,
                        bmp.height,
                        self.cache.len(),
                        self.cache.resident_bytes() as f64 / (1024.0 * 1024.0)
                    );
                    self.cache.insert(page, bmp);
                }
                Err(e) => {
                    error!("render page {page}: {e}");
                }
            }
        }
    }

    /// Actualiza `page` a la primera página visible en el scroll (la que toca
    /// el borde superior del viewport): alimenta el indicador "N / total", los
    /// saltos ±10 y la persistencia. Devuelve true si cambió (→ invalidar la
    /// barra y guardar el estado).
    fn update_page_from_scroll(&mut self) -> bool {
        let n = self.page_offsets.len();
        let (first, _) = self.visible_pages();
        let page = if n == 0 {
            0
        } else {
            (first as u32).min(n as u32 - 1)
        };
        if page != self.page {
            self.page = page;
            self.viewer_bar = None; // el indicador "N / total" cambia
            info!("page {}", self.page + 1);
            true
        } else {
            false
        }
    }

    /// Blitea el frame actual al buffer del ANativeWindow con UN solo
    /// lock+present.
    ///
    /// - Visor (scroll continuo): `draw::blit_stacked` dibuja el fondo y cada
    ///   página visible de la columna en su posición (offset acumulado −
    ///   scroll_y), recortando a la ventana; el overlay de la barra superior
    ///   va después en el mismo buffer. Zoom RELATIVO `zoom / rendered_zoom`
    ///   por página: 1:1 nítido para bitmaps recién renderizados y escala
    ///   vecino-más-cercana durante el pinch (sin re-render).
    /// - Picker/Biblioteca: `zoom::blit_fast` con el bitmap de la lista.
    ///
    /// Aquí se decide SOLO el estado que depende del `Reader`: fondo rojo sin
    /// documento, modo oscuro (inversión al blitear) y overlay del visor.
    fn blit(&self, window: &NativeWindow) {
        let t0 = Instant::now();
        let bg = if self.doc.is_none() {
            ERROR_BG
        } else if self.dark {
            DARK_BG // modo oscuro: fondo negro puro (se funde con la página)
        } else {
            BACKGROUND
        };
        match self.mode {
            UiMode::Viewer => {
                let (first, last) = self.visible_pages();
                let blit_zoom = if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
                    self.zoom / self.rendered_zoom
                } else {
                    1.0
                };
                let mut pages: Vec<PageBlit> = Vec::with_capacity(last - first + 1);
                // Capa de anotaciones: trazos guardados de cada página visible
                // + el trazo en curso (modo dibujo), transformados por la
                // misma (dx, dy, scale) que el blit de la página (la capa se
                // dibuja SOBRE el bitmap, ver `draw::blit_stacked`).
                let mut ann_layers: Vec<PageAnnots> = Vec::new();
                for p in first..=last {
                    // `peek` (sin promoción LRU): el blit solo lee; la
                    // recencia la fija el render/prefetch.
                    let Some(bmp) = self.cache.peek(p as u32) else {
                        continue;
                    };
                    // Centrado horizontal del bitmap en la ventana (puede ser
                    // negativo con zoom > 1: se recortan los bordes; mismo
                    // criterio que el centrado *contain* previo a la caché).
                    let dx =
                        ((self.win_w as f32 - bmp.width as f32 * blit_zoom) / 2.0).round() as i32;
                    // Posición en la columna: offset acumulado − scroll.
                    let dy = self.page_offsets[p] - self.scroll_y as i32;
                    pages.push(PageBlit {
                        page: p as u32,
                        bitmap: bmp,
                        dx,
                        dy,
                        zoom: blit_zoom,
                    });
                    // Trazos guardados de la página, en orden de dibujo (z);
                    // solo Stroke (Highlight/TextNote no se dibujan aún).
                    let strokes: Vec<&Stroke> = self
                        .annotations
                        .for_page(p)
                        .iter()
                        .filter_map(|a| match &a.kind {
                            Annotation::Stroke(s) => Some(s),
                            _ => None,
                        })
                        .collect();
                    // Trazo en curso (dedo bajado en modo dibujo), si es de
                    // esta página: se dibuja encima de los guardados.
                    let active = match &self.active_stroke {
                        Some(act) if act.page == p as u32 => Some(act),
                        _ => None,
                    };
                    if !strokes.is_empty() || active.is_some() {
                        // scale = cover × zoom: px de ventana por punto PDF
                        // (la misma escala efectiva del blit; derivación en
                        // `screen_to_page`). Si no se puede saber el tamaño de
                        // la página (render fallido), escala degradada.
                        let scale = match self.doc.as_ref().and_then(|d| d.page_size(p as u32).ok())
                        {
                            Some((pw, ph)) => {
                                initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom
                            }
                            None => blit_zoom,
                        };
                        ann_layers.push(PageAnnots {
                            page: p as u32,
                            dx,
                            dy,
                            scale,
                            strokes,
                            active,
                        });
                    }
                }
                blit_stacked(
                    window,
                    bg,
                    self.dark,
                    &pages,
                    &ann_layers,
                    self.viewer_bar.as_ref(),
                );
            }
            UiMode::Picker | UiMode::Library => match self.bitmap.as_ref() {
                Some(bmp) => {
                    // Bitmap de la lista a 1:1 (zoom relativo 1.0).
                    blit_fast(
                        window,
                        bmp,
                        1.0,
                        bg,
                        (self.offset_x + self.pan_x, self.offset_y + self.pan_y),
                        None,
                    );
                }
                None => {
                    // Sin lista: solo el fondo (guard hace unlock_and_post al caer).
                    let Ok(mut guard) = window.lock(None) else {
                        warn!("ANativeWindow_lock failed");
                        return;
                    };
                    let bpp = match guard.format().bytes_per_pixel() {
                        Some(b) => b,
                        None => {
                            warn!(
                                "buffer format without bytes_per_pixel: {:?}",
                                guard.format()
                            );
                            return;
                        }
                    };
                    let dst_w = guard.width();
                    let dst_h = guard.height();
                    let dst_stride = guard.stride(); // en píxeles
                    let dst = guard.bits() as *mut u8;
                    crate::draw::fill_buffer(dst, dst_w, dst_h, dst_stride, bpp, bg);
                }
            },
        }
        info!(
            "blit {}x{}: {:.2} ms (lock+copy+unlock_and_post)",
            self.win_w,
            self.win_h,
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// Desplaza el scroll a la página `page` (0-based): la página queda
    /// alineada con el borde superior del viewport. Base compartida de
    /// `next_page`/`prev_page`/`jump_page` y del tap derecho/izquierdo. Ya no
    /// hay salto con re-render: las páginas vecinas salen de la caché.
    fn scroll_to_page(&mut self, page: u32) {
        self.pending_page = Some(page);
        self.redraw();
        self.save_state();
    }

    /// Scroll continuo: fija la posición vertical del viewport (px del
    /// documento) y redibuja. NO cambia de página; el indicador "N / total"
    /// se actualiza al cruzar bordes de página (redraw) y la posición se
    /// persiste solo cuando la página visible cambia.
    pub(crate) fn scroll_to(&mut self, y: f32) {
        let max = (self.doc_height - self.win_h).max(0) as f32;
        let y = y.clamp(0.0, max);
        if (self.scroll_y - y).abs() < 0.5 {
            return;
        }
        self.scroll_y = y;
        self.redraw();
    }

    pub(crate) fn next_page(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1);
        if self.page < last {
            self.scroll_to_page(self.page + 1);
        }
    }

    pub(crate) fn prev_page(&mut self) {
        if self.page > 0 {
            self.scroll_to_page(self.page - 1);
        }
    }

    /// Salto rápido de ±N páginas (botones −10/+10 de la barra superior).
    pub(crate) fn jump_page(&mut self, delta: i32) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1) as i32;
        let target = (self.page as i32 + delta).clamp(0, last) as u32;
        if target != self.page {
            self.scroll_to_page(target);
        }
    }

    /// Zoom DURANTE el pinch (fast): solo actualiza el factor (1.0 = página
    /// completa) y hace un redraw de solo blit — `blit` escala los bitmaps
    /// cacheados de la columna con el zoom RELATIVO `zoom / rendered_zoom`
    /// (vecino-más-cercano), SIN re-renderizar MuPDF. El re-render nítido a la
    /// resolución final se hace UNA vez al soltar el pinch
    /// (`set_zoom_sharp`). El redraw normal (render + blit) no se usa aquí
    /// porque `ensure_pages_rendered` re-renderizaría en cada Move.
    pub(crate) fn set_zoom_fast(&mut self, zoom: f32) {
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        if (self.zoom - zoom).abs() < 1e-4 {
            return;
        }
        self.zoom = zoom;
        // Redraw de solo blit: reutiliza los bitmaps de la caché (escala de la
        // última renderización, `rendered_zoom`) y el layout de la columna;
        // `blit` escala cada página con el zoom nuevo. El render y el
        // reescalado de ventana los cubre el bucle de eventos
        // (RedrawNeeded/WindowResized) si hicieran falta.
        if let Some(win) = self.window.as_ref() {
            self.blit(win);
        }
    }

    /// Zoom FINAL del pinch (sharp): setea el factor (1.0 = página completa),
    /// mantiene el mismo punto del documento en el borde superior del viewport
    /// (el scroll se escala con el factor) y re-renderiza las páginas visibles
    /// UNA única vez a la escala continua resultante (render directo a
    /// resolución de pantalla — el camino medido más rápido en la tablet, ver
    /// nota de rendimiento en la cabecera de `lib.rs`): la caché se limpia y
    /// el layout se reconstruye; el redraw renderiza vía `ensure_pages_rendered`.
    /// Persiste el zoom (solo aquí, al soltar el gesto: `set_zoom_fast` es
    /// transitorio y escribir en cada Move de 60-120 Hz llenaría el disco).
    pub(crate) fn set_zoom_sharp(&mut self, zoom: f32) {
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        // Mantener el MISMO punto del documento en el borde superior del
        // viewport: el scroll se escala con el factor de zoom (las alturas de
        // página cambian con la escala y el layout se reconstruye en el redraw
        // siguiente). La caché se limpia: los bitmaps viejos son de otra escala.
        let factor = if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
            zoom / self.rendered_zoom
        } else {
            1.0
        };
        self.scroll_y *= factor;
        self.zoom = zoom;
        self.rendered_zoom = zoom;
        self.cache.clear();
        self.layout_dirty = true;
        info!("zoom {:.3}", self.zoom);
        self.redraw();
        self.save_state();
    }

    /// Alterna el modo oscuro SIN re-renderizar MuPDF y SIN tocar la caché:
    /// esta guarda SIEMPRE bitmaps normales (de colores) y la inversión
    /// (255 − v, la transformación de `pdf_core::dark::invert_bitmap`) se
    /// aplica en el blit (`draw::blit_stacked`), por página, solo cuando el
    /// modo oscuro está activo. El fondo letterbox pasa a negro puro
    /// (`DARK_BG`). La preferencia se persiste junto a la posición.
    pub(crate) fn toggle_dark(&mut self) {
        self.dark = !self.dark;
        // La caché guarda SIEMPRE bitmaps normales: la inversión (255 − v) se
        // aplica en el blit (`draw::blit_stacked`), por página, solo con el
        // modo oscuro activo — no se re-renderiza ni se invierte nada aquí.
        self.viewer_bar = None; // la etiqueta del botón cambia (Dark/Light)
        info!("dark mode: {}", self.dark);
        self.save_state();
        self.redraw();
    }

    /// Alterna el modo dibujo (botón "✏️" de la barra): con él activo, el
    /// arrastre con un dedo crea un trazo en vez de hacer scroll. El trazo en
    /// curso, si lo hubiera, se descarta (el gesto no puede continuar al
    /// cambiar de modo). El botón de la barra se re-renderiza (cambia de
    /// color cuando el modo está activo).
    pub(crate) fn toggle_draw_mode(&mut self) {
        self.draw_mode = !self.draw_mode;
        self.active_stroke = None;
        self.viewer_bar = None; // la etiqueta "✏️" cambia de color
        info!("draw mode: {}", self.draw_mode);
        self.redraw();
    }

    /// Alterna el color de los trazos nuevos entre la `STROKE_PALETTE`
    /// (botón "●" de la barra: rojo → azul → verde → ...). Solo afecta a los
    /// trazos futuros (los ya guardados conservan su color).
    pub(crate) fn cycle_stroke_color(&mut self) {
        let idx = STROKE_PALETTE
            .iter()
            .position(|c| *c == self.stroke_color)
            .unwrap_or(0);
        self.stroke_color = STROKE_PALETTE[(idx + 1) % STROKE_PALETTE.len()];
        self.viewer_bar = None; // el botón "●" cambia de color
        info!(
            "stroke color: #{:02X}{:02X}{:02X}",
            self.stroke_color.r, self.stroke_color.g, self.stroke_color.b
        );
        self.redraw();
    }

    /// Undo (botón "↶"): quita el último trazo de la página actual (el de
    /// mayor id, que es el último añadido — `AnnotationSet::add` asigna ids
    /// monótonos) y guarda el sidecar. La página actual es `self.page` (la
    /// que toca el borde superior del viewport); el trazo visible más
    /// reciente suele estar en ella.
    pub(crate) fn undo_last_stroke(&mut self) {
        // Copiar el id antes de mutar: `for_page` presta el set inmutablemente
        // y `remove` lo quiere mutable (ids únicos y monótonos → el último de
        // la página es el más reciente).
        let last_id = self
            .annotations
            .for_page(self.page as usize)
            .last()
            .map(|a| a.id);
        let Some(id) = last_id else {
            return;
        };
        if self.annotations.remove(id) {
            info!("undo: removed annotation {id}");
            self.save_annotations();
            self.redraw();
        }
    }

    /// Empieza un trazo en `page` con el punto del Down (coordenadas de
    /// página, ver `screen_to_page`). `input` garantiza que el dedo cae sobre
    /// una página (no en un hueco de la columna) y fuera de la barra superior.
    pub(crate) fn begin_stroke(&mut self, page: u32, pt: (f32, f32)) {
        self.active_stroke = Some(ActiveStroke::new(page, pt, self.stroke_color));
    }

    /// Añade un punto al trazo en curso (Move del dedo) y redibuja la capa de
    /// anotaciones: el trazo crece en pantalla. `redraw()` no re-renderiza
    /// MuPDF (las páginas visibles están en la caché): cuesta el blit de
    /// pantalla (~1-3 ms) + el dibujo del trazo, dentro del presupuesto del
    /// frame a 60-120 Hz de entrada.
    pub(crate) fn extend_stroke(&mut self, page: u32, pt: (f32, f32)) {
        let Some(act) = self.active_stroke.as_mut() else {
            return;
        };
        if act.page != page {
            return; // el trazo pertenece a la página del Down
        }
        act.push(pt);
        self.redraw();
    }

    /// Termina el trazo en curso (Up del dedo): lo convierte en
    /// [`pdf_core::Stroke`], lo añade al `AnnotationSet` y guarda el sidecar.
    /// Devuelve true si se guardó un trazo (los degenerados, < 2 puntos, se
    /// descartan).
    pub(crate) fn finish_stroke(&mut self) -> bool {
        let Some(act) = self.active_stroke.take() else {
            return false;
        };
        let Some(s) = Stroke::new(act.points, act.width, act.color) else {
            return false; // polilínea degenerada: un tap no es un trazo
        };
        match self
            .annotations
            .add(act.page as usize, Annotation::Stroke(s))
        {
            Some(id) => {
                info!("stroke {id} saved (page {})", act.page + 1);
                self.save_annotations();
                self.redraw(); // quitar el trazo activo del frame
                true
            }
            None => false,
        }
    }

    /// Descarta el trazo en curso sin guardar (Cancel del sistema).
    pub(crate) fn cancel_stroke(&mut self) {
        if self.active_stroke.take().is_some() {
            self.redraw();
        }
    }

    /// Carga las anotaciones del PDF `path` desde su sidecar SQLite
    /// (`store::sidecar_path` → `AnnotationStore::open` → `load`).
    ///
    /// Dónde vive el sidecar: `sidecar_path` lo sitúa en `<pdf-dir>/annotations/<stem>.db`
    /// (PLAN §3.5). Para la biblioteca y el "abrir con" el PDF se copia a
    /// `internal/pdfs/` (`open_library_entry`/`jni::launch_intent_pdf`), así
    /// que el sidecar queda en `internal/pdfs/annotations/<stem>.db` junto a
    /// la copia — pensado para Syncthing (un conflicto queda contenido en un
    /// solo fichero). Para el picker, junto al PDF elegido.
    ///
    /// Si el sidecar no existe o está corrupto, el set queda VACÍO (nunca se
    /// impide abrir el PDF ni se rompe la app); `AnnotationStore::open` crea
    /// el directorio y el esquema al primer guardado.
    fn load_annotations(&mut self, path: &str) {
        let sidecar = sidecar_path(Path::new(path));
        let set = match AnnotationStore::open(&sidecar) {
            Ok(store) => match store.load() {
                Ok(set) => {
                    info!(
                        "annotations: {} loaded from {}",
                        set.len(),
                        sidecar.display()
                    );
                    set
                }
                Err(e) => {
                    error!("annotations load {}: {e}", sidecar.display());
                    AnnotationSet::new()
                }
            },
            Err(e) => {
                error!("annotations open {}: {e}", sidecar.display());
                AnnotationSet::new()
            }
        };
        self.annotations = set;
        self.annot_sidecar = Some(sidecar);
    }

    /// Guarda el `AnnotationSet` completo en el sidecar del documento abierto
    /// (`AnnotationStore::save` — reescritura transaccional del set, O(n) con
    /// n = nº de anotaciones; se llama solo en acciones de usuario, nunca por
    /// frame). Best-effort: un fallo solo se loguea, no rompe el dibujo.
    fn save_annotations(&self) {
        let Some(sidecar) = self.annot_sidecar.as_ref() else {
            return;
        };
        match AnnotationStore::open(sidecar) {
            Ok(store) => match store.save(&self.annotations) {
                Ok(()) => info!(
                    "annotations saved ({} total) to {}",
                    self.annotations.len(),
                    sidecar.display()
                ),
                Err(e) => error!("annotations save {}: {e}", sidecar.display()),
            },
            Err(e) => error!("annotations open {}: {e}", sidecar.display()),
        }
    }

    /// Convierte un punto de la ventana (px) a coordenadas de página (puntos
    /// PDF) para `page` — la inversa exacta de la transformación del blit
    /// (`draw::blit_page_scaled`).
    ///
    /// # Fórmula (documentada)
    ///
    /// El blit dibuja la página con su esquina superior izquierda en
    /// `(dx, dy)` de la ventana y una escala `scale` de px de ventana por
    /// punto PDF:
    ///
    /// ```text
    /// scale = cover(page) × zoom
    /// dx    = (win_w − page_w_px) / 2      (centrado horizontal; < 0 a zoom > 1)
    /// dy    = page_offsets[page] − scroll_y (columna del scroll continuo)
    /// screen = page_pt × scale + (dx, dy)
    /// ```
    ///
    /// con `cover(page) = initial_scale(pw, ph, win_w, win_h)` (la política
    /// de apertura, `view.rs`), `page_w_px = pw × scale` y `page_offsets` el
    /// layout de la columna. Como `dx`/`dy`/`scale` son exactamente los que
    /// usa el blit, la inversa
    ///
    /// ```text
    /// page_pt = (screen − (dx, dy)) / scale
    /// ```
    ///
    /// devuelve el punto de página que el usuario ve bajo el dedo, coherente
    /// con lo dibujado en pantalla en cualquier zoom (el factor continuo
    /// `zoom` va dentro de `scale`; durante el pinch sin re-render el blit
    /// escala el bitmap viejo con el zoom relativo, y `scale × blit_zoom`
    /// simplifica a `cover × zoom` igualmente).
    pub(crate) fn screen_to_page(&self, page: u32, sx: f32, sy: f32) -> (f32, f32) {
        let Some(doc) = self.doc.as_ref() else {
            return (sx, sy);
        };
        let Ok((pw, ph)) = doc.page_size(page) else {
            return (sx, sy);
        };
        let scale = initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return (sx, sy);
        }
        let dx = (self.win_w as f32 - pw * scale) / 2.0;
        let dy = self.page_offsets.get(page as usize).copied().unwrap_or(0) as f32 - self.scroll_y;
        ((sx - dx) / scale, (sy - dy) / scale)
    }

    /// Página bajo el punto de ventana `y` (geometría en pantalla, con el
    /// zoom relativo del blit por si `rendered_zoom != zoom`), o `None` si el
    /// punto cae en un hueco de la columna (gap entre páginas) o fuera de
    /// ella. La usa el modo dibujo para saber sobre qué página empieza el
    /// trazo (`input`).
    pub(crate) fn page_at_y(&self, y: f32) -> Option<u32> {
        let blit_zoom = if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
            self.zoom / self.rendered_zoom
        } else {
            1.0
        };
        let (first, last) = self.visible_pages();
        for p in first..=last {
            let top = self.page_offsets[p] - self.scroll_y as i32;
            let h = (self.page_heights[p] as f32 * blit_zoom).round() as i32;
            if y >= top as f32 && y < (top + h) as f32 {
                return Some(p as u32);
            }
        }
        None
    }

    /// Persiste la posición actual (ruta, página, zoom) + modo oscuro en
    /// `internal/state.json` (ver `persist`). Escritura *eager*: se llama en
    /// cada cambio de página, al soltar el pinch, al abrir un documento y al
    /// alternar el modo oscuro — un cierre inesperado no pierde la posición.
    fn save_state(&self) {
        if self.mode != UiMode::Viewer {
            return;
        }
        let Some(path) = self.doc_path.as_ref() else {
            return;
        };
        let state = crate::persist::ViewerState {
            path: path.clone(),
            page: self.page,
            zoom: self.zoom,
            dark: self.dark,
        };
        crate::persist::save_state(self.internal_dir.as_deref(), &state);
    }

    /// Abre un PDF por ruta (picker) y pasa al visor con la página 1.
    /// Devuelve false (y deja el estado intacto) si no se pudo abrir.
    pub(crate) fn open_pdf(&mut self, path: &str) -> bool {
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                return false;
            }
        };
        match engine.open(Path::new(path)) {
            Ok(doc) => {
                info!("opened: {} pages", doc.page_count());
                self.doc = Some(doc);
                self.page = 0;
                self.zoom = 1.0;
                self.rendered_zoom = 1.0;
                self.scroll_y = 0.0; // columna desde el principio
                self.pending_page = None;
                self.pan_x = 0;
                self.pan_y = 0;
                self.bitmap = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.layout_dirty = true;
                self.mode = UiMode::Viewer;
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.viewer_bar = None;
                self.list_dirty = true;
                self.picker_drag = None;
                self.draw_mode = false; // cada documento empieza en modo lectura
                self.active_stroke = None;
                // Anotaciones del documento (sidecar; set vacío si no existe
                // o está corrupto — nunca impide abrir el PDF).
                self.load_annotations(path);
                self.redraw();
                // Nuevo documento: actualizar la posición persistida (el
                // modo oscuro es una preferencia global y se conserva).
                self.save_state();
                true
            }
            Err(e) => {
                error!("cannot open {path}: {e}");
                false
            }
        }
    }

    /// Entra en la biblioteca MediaStore (botón "Open" del visor): re-consulta
    /// y deja de mostrar la página. Si MediaStore está vacía, `rescan_library`
    /// cae al picker interno como fallback.
    pub(crate) fn enter_library(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Library;
        self.list_scroll = 0;
        self.list_dirty = true;
        self.bitmap = None;
        self.viewer_bar = None;
        self.picker_drag = None;
        self.rescan_library(app);
    }

    /// Vuelve del picker al visor sin cambiar el documento (botón Back).
    pub(crate) fn exit_picker(&mut self) {
        self.mode = UiMode::Viewer;
        self.list_dirty = true;
        self.bitmap = None; // lista del picker (las páginas siguen en la caché)
        self.picker_drag = None;
        self.redraw();
    }

    /// Re-consulta MediaStore (botón Rescan / arranque / Resume de la
    /// biblioteca). Actualiza la lista, el estado del permiso y el mensaje;
    /// si el permiso está concedido pero MediaStore no devuelve PDFs, cae al
    /// picker interno como fallback (regla del enunciado).
    pub(crate) fn rescan_library(&mut self, app: &AndroidApp) {
        self.grant_pending = false;
        let scan = query_media_store(app, self.sdk_int);
        self.library_list = scan.entries;
        // Datos nuevos: se quita el filtro (mostrar todas) y se reconstruye
        // la lista filtrada (sin filtrar = todas las entradas).
        self.library_filter = None;
        self.library_filtered = (0..self.library_list.len()).collect();
        self.permission_granted = scan.permission_granted;
        self.list_scroll = 0;
        self.list_dirty = true;
        if !self.permission_granted {
            self.status = Some("All files access not granted — tap Grant".to_string());
        } else if let Some(e) = scan.error {
            self.status = Some(format!("MediaStore error: {e}"));
        } else if self.library_list.is_empty() {
            // Fallback: carpeta interna de la app (permiso OK, sin PDFs en MediaStore).
            self.pdf_list = scan_pdfs(app);
            self.mode = UiMode::Picker;
            self.status = Some(if self.pdf_list.is_empty() {
                "No PDFs in MediaStore or app folder".to_string()
            } else {
                "No PDFs in MediaStore — showing app folder".to_string()
            });
        } else {
            self.status = None;
        }
        info!(
            "library: {} PDFs (all-files-access: {})",
            self.library_list.len(),
            self.permission_granted
        );
        self.redraw();
    }

    /// Aplica o quita el filtro por letra inicial de la biblioteca: con
    /// `Some(l)` solo se muestran las entradas cuyo nombre normalizado
    /// empieza por `l` (bucket 'a'..='z' | '#'); `None` muestra todas. La
    /// lista filtrada (`library_filtered`) se reconstruye aquí y el scroll
    /// vuelve al principio. La tira de letras (`input::library_tap`) la
    /// llama con `Some(letra)` al tocar una celda y con `None` al repetir la
    /// letra activa.
    pub(crate) fn set_library_filter(&mut self, letter: Option<char>) {
        if self.library_filter == letter {
            return;
        }
        self.library_filter = letter;
        self.library_filtered = match letter {
            None => (0..self.library_list.len()).collect(),
            Some(l) => self
                .library_list
                .iter()
                .enumerate()
                .filter(|(_, e)| normalize_letter(&e.name) == l)
                .map(|(i, _)| i)
                .collect(),
        };
        self.list_scroll = 0;
        self.list_dirty = true;
        info!(
            "library filter: {} -> {} entries",
            letter
                .map(|c| c.to_string())
                .unwrap_or_else(|| "all".into()),
            self.library_filtered.len()
        );
        self.redraw();
    }

    /// Longitud de la lista de la biblioteca con el filtro aplicado (lo que
    /// realmente se muestra en el modo Library).
    pub(crate) fn filtered_library_len(&self) -> usize {
        self.library_filtered.len()
    }

    /// Entrada `idx`-ésima de la lista FILTRADA de la biblioteca (None si
    /// fuera de rango). Los índices se resuelven vía `library_filtered`
    /// (indirección O(1)); render y tap no deben tocar `library_list`
    /// directamente en modo Library.
    pub(crate) fn library_entry_at(&self, idx: usize) -> Option<&LibraryEntry> {
        let i = *self.library_filtered.get(idx)?;
        self.library_list.get(i)
    }

    /// Abre un documento de la biblioteca: copia los bytes de su content://
    /// URI a `internal/pdfs/` (ContentResolver.openInputStream) y lo abre con
    /// MuPDF. Devuelve false (estado intacto) si algo falla.
    pub(crate) fn open_library_entry(&mut self, app: &AndroidApp, entry: &LibraryEntry) -> bool {
        let Some(dir) = app.internal_data_path() else {
            error!("open library: internal_data_path unavailable");
            return false;
        };
        let pdfs_dir = dir.join("pdfs");
        if let Err(e) = fs::create_dir_all(&pdfs_dir) {
            error!("open library: create_dir_all {}: {e}", pdfs_dir.display());
            return false;
        }
        let dest = pdfs_dir.join(sanitize_pdf_name(&entry.name));
        match read_content_uri_bytes(app, &entry.uri) {
            Some(bytes) => {
                let n = bytes.len();
                if let Err(e) = fs::write(&dest, &bytes) {
                    error!("open library: write {}: {e}", dest.display());
                    return false;
                }
                info!(
                    "library open: {} ({}) -> {} ({} bytes)",
                    entry.name,
                    entry.folder,
                    dest.display(),
                    n
                );
            }
            None => {
                error!("open library: cannot read {}", entry.uri);
                return false;
            }
        }
        let path = dest.display().to_string();
        self.open_pdf(&path)
    }

    /// Relee los directorios de la app (botón Rescan del picker).
    pub(crate) fn rescan(&mut self, app: &AndroidApp) {
        self.pdf_list = scan_pdfs(app);
        self.list_scroll = 0;
        self.status = None;
        self.list_dirty = true;
        info!("rescan: {} PDFs", self.pdf_list.len());
        self.redraw();
    }
}
