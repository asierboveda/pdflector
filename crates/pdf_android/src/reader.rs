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
use pdf_core::{Annotation, AnnotationSet, Bitmap, Document, RenderEngine, Stroke};

use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES, PageCache};
use crate::draw::{
    PageAnnots, PageBlit, blit_stacked, render_library_grid, render_page_badge, render_picker_list,
    render_sheet,
};
use crate::input::GestureState;
use crate::jni::{
    android_sdk_int, launch_intent_pdf, open_content_fd, query_media_store, read_content_uri_bytes,
    sanitize_pdf_name,
};
use crate::persist;
use crate::thumbs::{THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES, THUMB_W, ThumbCache};
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
    ///
    /// `dead_code` intencional (2026-08-XX): la rejilla 3×3 no muestra el
    /// tamaño (la lista sí lo hacía); la proyección de MediaStore lo sigue
    /// trayendo gratis y una futura vista de detalle puede usarlo.
    #[allow(dead_code)]
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

/// Alto (px) del sheet de ajustes desplegado del todo: la mitad de la
/// ventana (panel deslizante desde el borde superior).
pub(crate) fn sheet_h(win_h: i32) -> i32 {
    win_h / 2
}

/// Pad horizontal del sheet (px).
pub(crate) fn sheet_pad(win_w: i32) -> f32 {
    (win_w / 48).max(12) as f32
}

/// Alto (px) de los botones del sheet.
pub(crate) fn sheet_btn_h(win_h: i32) -> f32 {
    (win_h / 32).max(48) as f32
}

/// Separación vertical entre filas de botones del sheet (px).
pub(crate) fn sheet_gap(win_h: i32) -> f32 {
    (win_h / 64).max(24) as f32
}

/// Y del borde superior de la fila 1 de botones del sheet (Back/Open/Dark):
/// debajo del título "Settings" (ver `render_sheet`).
pub(crate) fn sheet_row1_y(win_h: i32) -> f32 {
    (win_h / 48).max(44) as f32
}

/// Y del borde superior de la fila 2 de botones del sheet (−10/N/+10).
pub(crate) fn sheet_row2_y(win_h: i32) -> f32 {
    sheet_row1_y(win_h) + sheet_btn_h(win_h) + sheet_gap(win_h)
}

/// Ancho (px) de cada botón de fila del sheet (3 por fila, con pads entre
/// ellos).
pub(crate) fn sheet_btn_w(win_w: i32) -> f32 {
    (win_w as f32 - 4.0 * sheet_pad(win_w)) / 3.0
}

/// --- Rejilla 3×3 de la biblioteca (geometría compartida por render y tap) ---
/// Columnas de la rejilla de la biblioteca.
pub(crate) const GRID_COLS: usize = 3;

/// Pad exterior horizontal de la rejilla (px).
pub(crate) fn grid_pad(win_w: i32) -> f32 {
    (win_w / 48).max(12) as f32
}

/// Separación entre celdas de la rejilla (px).
pub(crate) fn grid_gap() -> f32 {
    12.0
}

/// Inset de la portada dentro de la celda (px).
pub(crate) const GRID_CELL_PAD: f32 = 8.0;

/// Ancho (px) de una celda de la rejilla.
pub(crate) fn grid_cell_w(win_w: i32) -> f32 {
    let w = win_w as f32;
    (w - 2.0 * grid_pad(win_w) - 2.0 * grid_gap()) / GRID_COLS as f32
}

/// Ancho (px) del área de portada dentro de la celda.
pub(crate) fn grid_cover_w(win_w: i32) -> f32 {
    grid_cell_w(win_w) - 2.0 * GRID_CELL_PAD
}

/// Alto (px) del área de portada: proporción A4 (1:√2) para TODAS las celdas
/// (rejilla uniforme; la portada real, con su propia proporción, se centra
/// dentro del área).
pub(crate) fn grid_cover_h(win_w: i32) -> f32 {
    grid_cover_w(win_w) * std::f32::consts::SQRT_2
}

/// Alto (px) de la zona de título de la celda (hasta 2 líneas).
pub(crate) fn grid_title_h(win_w: i32) -> f32 {
    let line = (grid_cell_w(win_w) / 14.0).max(16.0);
    2.0 * line + 10.0
}

/// Alto (px) total de una celda de la rejilla.
pub(crate) fn grid_cell_h(win_w: i32) -> f32 {
    grid_cover_h(win_w) + grid_title_h(win_w)
}

/// Nº de filas de celdas visibles en la biblioteca (cabecera + franja de
/// estado restan de la ventana; mínimo 1 fila para que siempre haya algo).
pub(crate) fn grid_visible_rows(win_w: i32, win_h: i32, has_status: bool) -> usize {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    let usable = (win_h - picker_header_h(win_h) - status_h) as f32;
    (usable / grid_cell_h(win_w)).floor().max(1.0) as usize
}

/// Y del borde superior de la zona de rejilla (cabecera + franja de estado).
pub(crate) fn grid_rows_y0(win_h: i32, has_status: bool) -> i32 {
    picker_header_h(win_h) + if has_status { picker_row_h(win_h) } else { 0 }
}

/// Rectángulo (left, top, right, bottom) en px de ventana de la celda
/// `(row, col)` de la rejilla (compartido por `draw::render_library_grid` e
/// `input::library_tap`).
pub(crate) fn grid_cell_rect(
    win_w: i32,
    rows_y0: i32,
    row: usize,
    col: usize,
) -> (f32, f32, f32, f32) {
    let x = grid_pad(win_w) + col as f32 * (grid_cell_w(win_w) + grid_gap());
    let y = rows_y0 as f32 + row as f32 * grid_cell_h(win_w);
    (x, y, x + grid_cell_w(win_w), y + grid_cell_h(win_w))
}

/// Tamaño fijo del indicador de página "N / total" (overlay abajo a la
/// izquierda). Ancho ~1/8 de ventana, alto ~1/60 (≈ 150×33 px en la tablet).
pub(crate) fn page_badge_size(win_w: i32, win_h: i32) -> (i32, i32) {
    ((win_w / 8).max(110), (win_h / 60).max(30))
}

/// Rectángulo (left, top, right, bottom) en px de ventana del indicador de
/// página (compartido por el blit y el tap de `input`).
pub(crate) fn page_badge_rect(win_w: i32, win_h: i32) -> (i32, i32, i32, i32) {
    let (bw, bh) = page_badge_size(win_w, win_h);
    let pad = (win_w / 96).max(8);
    (pad, win_h - bh - pad, pad + bw, win_h - pad)
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

/// Anclaje del pinch en curso: estado que `begin_pinch` captura al caer el
/// segundo dedo y que `set_zoom_fast` usa para recalcular el pan que deja
/// fijo el punto de documento bajo el centro del pinch (ver `anchor_pan`).
#[derive(Clone, Copy, Debug)]
struct PinchAnchor {
    /// Centro del pinch en px de ventana (punto de pantalla que permanece
    /// fijo bajo los dedos durante el gesto).
    ax: f32,
    ay: f32,
    /// Zoom al iniciar el gesto (base del factor relativo y del anclaje).
    z0: f32,
    /// Pan de partida (del gesto anterior, 0 si no lo hay): el anclaje es
    /// continuo con el estado previo (a `z == z0` el pan no cambia).
    pan_x0: f32,
    pan_y0: f32,
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
    /// Desplazamiento de anclaje del pinch (px, f32): el punto de pantalla
    /// bajo el CENTRO del pinch permanece fijo mientras se hace zoom
    /// (`begin_pinch` fija el ancla; `set_zoom_fast` recalcula `pan_x/pan_y`
    /// con la fórmula de anclaje, ver `anchor_pan`). Se suma al centrado
    /// base del blit (`dx/dy`); persiste entre gestos y páginas (el zoom
    /// también): pasar de página conserva la misma región de lectura.
    /// 0 = sin desplazamiento.
    pan_x: f32,
    pan_y: f32,
    /// Anclaje del pinch en curso: centro del pinch en px de ventana
    /// (ax, ay), zoom al iniciar el gesto (z0) y pan de partida (pan_x0,
    /// pan_y0). Se fija en `begin_pinch` (PointerDown del segundo dedo), se
    /// consume en cada `set_zoom_fast` y queda sin usar al soltar el gesto
    /// (`set_zoom_sharp` conserva el pan ya calculado). None = sin pinch.
    pinch: Option<PinchAnchor>,
    /// Desplazamiento del bitmap de la LISTA dentro del buffer (picker/
    /// biblioteca; 0 por ahora).
    offset_x: i32,
    offset_y: i32,
    /// Posición vertical del viewport en px del documento, SIEMPRE alineada
    /// con el borde superior de la página actual (`scroll_y ==
    /// page_offsets[page]`; el modo página a página no tiene scroll libre).
    /// La fija `scroll_to_page` (vía `pending_page` + `rebuild_layout`) y la
    /// leen `blit`/`visible_pages`. Antes era el scroll continuo del arrastre
    /// (eliminado por decisión del autor); se conserva el campo porque toda
    /// la geometría de la columna lo usa.
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
    /// Máquina de gestos (tap/pinch).
    pub(crate) gesture: GestureState,
    /// Modo de UI actual (visor de página o picker de PDFs).
    pub(crate) mode: UiMode,
    /// PDFs encontrados en los directorios de la app (picker).
    pub(crate) pdf_list: Vec<PdfEntry>,
    /// PDFs del sistema devueltos por MediaStore (biblioteca, rejilla 3×3).
    pub(crate) library_list: Vec<LibraryEntry>,
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
    /// ¿Objetivo del sheet de ajustes? (true = abierto). La animación real
    /// vive en `sheet_progress`; `sheet_anim` marca que está en vuelo.
    pub(crate) sheet_open: bool,
    /// Progreso de apertura del sheet de ajustes: 0.0 = oculto, 1.0 = abierto
    /// del todo (alto `win_h / SHEET_H_DIV`). Durante el arrastre sigue al
    /// dedo (`drag_sheet`); al soltar, `tick` lo anima hacia el objetivo
    /// (`sheet_open`). Con `progress > 0` el sheet se dibuja deslizado desde
    /// el borde superior sobre el documento.
    pub(crate) sheet_progress: f32,
    /// ¿Animación del sheet en vuelo? Avanza en `Reader::tick`, que el bucle
    /// de eventos llama con `poll_events(Some(16 ms))` mientras
    /// `sheet_animating()` sea true (ver `lib::android_main`).
    sheet_anim: bool,
    /// Bitmap del sheet de ajustes (render Canvas+JNI, alto `win_h / 2`),
    /// cacheado: se invalida al cambiar ventana, página o modo oscuro y se
    /// LIBERA al cerrar del todo (`progress == 0`).
    sheet_bitmap: Option<Bitmap>,
    /// Bitmap del indicador de página "N / total" (overlay abajo a la
    /// izquierda, tap = página siguiente), cacheado: se invalida al cambiar
    /// ventana, página o modo oscuro.
    page_badge: Option<Bitmap>,
    /// Caché LRU de portadas de la biblioteca (content:// URI → portada de la
    /// página 1, `THUMB_W` px de ancho). Se limpia al abrir un PDF: las
    /// portadas y la `PageCache` del visor no compiten por el mismo
    /// presupuesto (estados mutuamente exclusivos: biblioteca vs visor).
    pub(crate) thumbs: ThumbCache,
    /// URIs cuya portada falló al renderizar (PDF corrupto, fd no abrible,
    /// página 1 vacía): no se reintentan — evita un bucle de timeout del
    /// bucle de eventos (`thumbs_pending` las excluye).
    thumb_failed: HashSet<String>,
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
            pan_x: 0.0,
            pan_y: 0.0,
            pinch: None,
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
            permission_granted: false,
            sdk_int: android_sdk_int(),
            grant_pending: false,
            list_scroll: 0,
            list_dirty: true,
            status: None,
            doc_path: None,
            internal_dir: app.internal_data_path(),
            dark: false,
            sheet_open: false,
            sheet_progress: 0.0,
            sheet_anim: false,
            sheet_bitmap: None,
            page_badge: None,
            thumbs: ThumbCache::new(THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES),
            thumb_failed: HashSet::new(),
            picker_drag: None,
            annotations: AnnotationSet::new(),
            annot_sidecar: None,
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
                            reader.page_badge = None; // indicador de la página restaurada
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
        self.page_badge = None;
        self.sheet_bitmap = None;
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
        self.page_badge = None;
        self.sheet_bitmap = None;
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
            self.page_badge = None;
            self.sheet_bitmap = None;
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
                // Indicador de página (abajo a la izquierda) y sheet de
                // ajustes, cacheados: se re-renderizan solo si cambió la
                // ventana, la página o el modo oscuro (invalidadores arriba,
                // en `update_page_from_scroll` y en `toggle_dark`). El sheet
                // se materializa solo cuando empieza a verse y se libera al
                // cerrar del todo.
                if self.doc.is_some() && self.page_badge.is_none() {
                    self.page_badge = render_page_badge(self);
                }
                if self.sheet_progress > 0.0 && self.sheet_bitmap.is_none() {
                    self.sheet_bitmap = render_sheet(self);
                }
                if self.sheet_progress <= 0.0 {
                    self.sheet_bitmap = None;
                }
            }
            UiMode::Picker | UiMode::Library => {
                // Clamp del scroll si la lista menguó (rescan) o cambió la ventana.
                // Picker: filas de `picker_row_h`; biblioteca: filas de celdas
                // de la rejilla 3×3 (`grid_cell_h`).
                let list_len = if self.mode == UiMode::Picker {
                    self.pdf_list.len()
                } else {
                    self.grid_total_rows()
                };
                let visible = if self.mode == UiMode::Picker {
                    picker_visible_rows(self.win_h, self.status.is_some())
                } else {
                    grid_visible_rows(self.win_w, self.win_h, self.status.is_some())
                };
                let max_scroll = list_len.saturating_sub(visible);
                if self.list_scroll > max_scroll {
                    self.list_scroll = max_scroll;
                }
                if self.list_dirty {
                    let bmp = if self.mode == UiMode::Picker {
                        render_picker_list(self)
                    } else {
                        render_library_grid(self)
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

    /// Tamaño de la página `page` en px de ventana a zoom 1 (cover × puntos
    /// PDF): las dimensiones que el usuario ve con el factor 1.0, base del
    /// centrado y del anclaje del pinch. Equivale a `bitmap_cached.width /
    /// rendered_zoom` (los bitmaps se renderizan a cover × rendered_zoom);
    /// se calcula de la página para no depender de un hit de caché.
    fn page_doc_size_px(&self, page: u32) -> (f32, f32) {
        let Some(doc) = self.doc.as_ref() else {
            return (0.0, 0.0);
        };
        let Ok((pw, ph)) = doc.page_size(page) else {
            return (0.0, 0.0);
        };
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        (pw * cover, ph * cover)
    }

    /// Esquina superior izquierda del bitmap escalado para centrado
    /// horizontal: `base(z) = (win − doc·z) / 2` (px de zoom 1), la misma
    /// fórmula que `blit` usa para `dx` sin pan. Lineal en `z`; en el
    /// anclaje Y la base es 0 (el borde superior de la página actual está
    /// fijo en el borde superior del viewport: `scroll_y = page_offsets[page]`).
    fn centered_base(win: i32, doc: f32, z: f32) -> f32 {
        (win as f32 - doc * z) / 2.0
    }

    /// Fórmula de anclaje del pinch: el pan (px) que, a zoom `z`, deja fijo
    /// en pantalla el punto de documento que estaba bajo el ancla al iniciar
    /// el gesto. `base0`/`base` son la posición de origen del bitmap escalado
    /// al zoom de partida (`z0`) y al zoom actual (`z`) — centrado horizontal
    /// `centered_base` o 0 en Y — y `pan0` el pan de partida.
    ///
    /// Derivación del anclaje. El mapeo pantalla de un punto de documento `q`
    /// (px a zoom 1) es `screen(q, z) = base(z) + pan(z) + q·z`, con
    /// `base(z)` la posición de origen del bitmap escalado:
    ///
    /// - el punto bajo el ancla al iniciar el gesto es
    ///   `q = (ancla − base(z0) − pan0) / z0` y no cambia durante el gesto;
    /// - imponiendo `screen(q, z) == ancla` queda la fórmula:
    ///
    /// ```text
    /// pan(z) = ancla − base(z) − q·z,   con q = (ancla − base(z0) − pan0) / z0
    /// ```
    ///
    /// Propiedades: a `z == z0` devuelve `pan0` (continuidad con el pan del
    /// gesto anterior); como `base(z)` es lineal en `z`, el pan es una función
    /// lineal del zoom (sin saltos entre Moves); al soltar el pinch
    /// (`set_zoom_sharp` re-renderiza a `rendered_zoom = zoom` y `blit_zoom`
    /// vuelve a 1.0) la escala efectiva `doc·z` no cambia, así que el mismo
    /// pan mantiene el anclaje sin que la página salte.
    fn anchor_pan(anchor: f32, base0: f32, base: f32, z0: f32, pan0: f32, z: f32) -> f32 {
        let q = (anchor - base0 - pan0) / z0;
        anchor - base - q * z
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
    /// saltos ±10 y la persistencia. En el modo página a página coincide con
    /// `self.page` (scroll_y = su borde superior); sirve de red de seguridad
    /// si el clamp del final del documento (zoom < 1) desvía el scroll.
    /// Devuelve true si cambió (→ invalidar la barra y guardar el estado).
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
            self.page_badge = None; // el indicador "N / total" cambia
            self.sheet_bitmap = None; // el indicador del sheet cambia
            info!("page {}", self.page + 1);
            true
        } else {
            false
        }
    }

    /// Blitea el frame actual al buffer del ANativeWindow con UN solo
    /// lock+present.
    ///
    /// - Visor (página a página): `draw::blit_stacked` dibuja el fondo y cada
    ///   página visible de la columna en su posición (offset acumulado −
    ///   scroll_y, con scroll_y siempre alineado al borde superior de la
    ///   página actual; pan de anclaje del pinch añadido), recortando a la
    ///   ventana; los overlays (indicador de página + sheet de ajustes)
    ///   van después en el mismo buffer. Zoom RELATIVO `zoom / rendered_zoom`
    ///   por página: 1:1 nítido para bitmaps recién renderizados y escala
    ///   vecino-más-cercana durante el pinch (sin re-render).
    /// - Picker/Biblioteca: `zoom::blit_fast` con el bitmap de la lista.
    ///
    /// Aquí se decide SOLO el estado que depende del `Reader`: fondo rojo sin
    /// documento, modo oscuro (inversión al blitear) y overlays del visor.
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
                    // criterio que el centrado *contain* previo a la caché),
                    // + el pan de anclaje del pinch (0 fuera de un gesto).
                    let dx = (((self.win_w as f32 - bmp.width as f32 * blit_zoom) / 2.0)
                        + self.pan_x)
                        .round() as i32;
                    // Posición en la columna: offset acumulado − scroll (en el
                    // modo página a página, scroll_y = borde superior de la
                    // página actual → dy = 0 + pan), + pan de anclaje.
                    let dy =
                        (self.page_offsets[p] as f32 - self.scroll_y + self.pan_y).round() as i32;
                    pages.push(PageBlit {
                        page: p as u32,
                        bitmap: bmp,
                        dx,
                        dy,
                        zoom: blit_zoom,
                    });
                    // Trazos guardados de la página, en orden de dibujo (z);
                    // solo Stroke (Highlight/TextNote no se dibujan aún). El
                    // trazo en curso (modo dibujo) se eliminó con la barra
                    // superior (2026-08-XX): no hay nada que añadir encima.
                    let strokes: Vec<&Stroke> = self
                        .annotations
                        .for_page(p)
                        .iter()
                        .filter_map(|a| match &a.kind {
                            Annotation::Stroke(s) => Some(s),
                            _ => None,
                        })
                        .collect();
                    if !strokes.is_empty() {
                        // scale = cover × zoom: px de ventana por punto PDF
                        // (la misma escala efectiva del blit). Si no se puede
                        // saber el tamaño de la página (render fallido),
                        // escala degradada.
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
                        });
                    }
                }
                // Overlays del visor en el MISMO buffer (un solo lock+present):
                // indicador de página abajo a la izquierda (siempre) y sheet de
                // ajustes deslizado desde el borde superior (solo si está
                // visible; `progress == 1` = abierto del todo).
                let mut overlays: Vec<(&Bitmap, i32, i32)> = Vec::with_capacity(2);
                if let Some(b) = self.page_badge.as_ref() {
                    let (bx, by, _, _) = page_badge_rect(self.win_w, self.win_h);
                    overlays.push((b, bx, by));
                }
                if self.sheet_progress > 0.0
                    && let Some(s) = self.sheet_bitmap.as_ref()
                {
                    let slide =
                        (sheet_h(self.win_h) as f32 * (1.0 - self.sheet_progress)).round() as i32;
                    overlays.push((s, 0, -slide));
                }
                blit_stacked(window, bg, self.dark, &pages, &ann_layers, &overlays);
            }
            UiMode::Picker | UiMode::Library => match self.bitmap.as_ref() {
                Some(bmp) => {
                    // Bitmap de la lista a 1:1 (zoom relativo 1.0).
                    blit_fast(
                        window,
                        bmp,
                        1.0,
                        bg,
                        (
                            self.offset_x + self.pan_x.round() as i32,
                            self.offset_y + self.pan_y.round() as i32,
                        ),
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
    /// alineada con el borde superior del viewport (`scroll_y =
    /// page_offsets[page]`, vía `pending_page` + `rebuild_layout`). Base
    /// compartida de `next_page`/`prev_page`/`jump_page` y del tap
    /// derecho/izquierdo. No hay salto con re-render: las páginas vecinas
    /// salen de la caché (paso instantáneo).
    fn scroll_to_page(&mut self, page: u32) {
        self.pending_page = Some(page);
        self.redraw();
        self.save_state();
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

    /// Salto rápido de ±N páginas (botones −10/+10 del sheet de ajustes).
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

    // ---------------------------------------------------------------------
    // Sheet de ajustes (panel deslizante desde arriba, 2026-08-XX)
    // ---------------------------------------------------------------------

    /// ¿Animación del sheet en vuelo? El bucle de eventos usa esta señal
    /// para mantener `poll_events(Some(16 ms))` y avanzar `tick` mientras
    /// tanto (sin ella el loop bloquea y la animación no progresa).
    pub(crate) fn sheet_animating(&self) -> bool {
        self.sheet_anim
    }

    /// Comienza el arrastre del sheet (dedo deslizándose): deja de animar y
    /// deja que el dedo controle `sheet_progress` directamente.
    pub(crate) fn begin_sheet_drag(&mut self) {
        self.sheet_anim = false;
    }

    /// Arrastre del sheet: `dy` = desplazamiento vertical del dedo desde el
    /// Down (px, positivo = hacia abajo). El progreso sigue al dedo
    /// (`dy / alto del sheet`, recortado a [0, 1]) y se redibuja el frame
    /// (solo blit: las páginas visibles ya están en la caché).
    pub(crate) fn drag_sheet(&mut self, dy: f32) {
        let h = sheet_h(self.win_h).max(1) as f32;
        self.sheet_progress = (dy / h).clamp(0.0, 1.0);
        if self.sheet_progress > 0.0 && self.sheet_bitmap.is_none() {
            // El sheet se empieza a ver: materializar su bitmap cacheado.
            self.sheet_bitmap = render_sheet(self);
        }
        self.redraw();
    }

    /// Fin del arrastre: anima el sheet hasta el objetivo más cercano
    /// (abierto si `progress >= 0.5`, cerrado si no).
    pub(crate) fn end_sheet_drag(&mut self) {
        self.sheet_open = self.sheet_progress >= 0.5;
        self.sheet_anim = true;
    }

    /// Cierra el sheet CON animación (tap fuera del panel): no toca el
    /// progreso actual; `tick` lo anima hasta 0 (el tap en el documento no
    /// cambia de página a propósito: cerrar el panel no debe avanzar).
    pub(crate) fn hide_sheet(&mut self) {
        if self.sheet_progress > 0.0 {
            self.sheet_open = false;
            self.sheet_anim = true;
        }
    }

    /// Oculta el sheet INMEDIATAMENTE (sin animación): al entrar en la
    /// biblioteca o al abrir otro documento el estado del visor se reinicia.
    fn sheet_hide_now(&mut self) {
        self.sheet_open = false;
        self.sheet_progress = 0.0;
        self.sheet_anim = false;
        self.sheet_bitmap = None;
    }

    /// Tick del bucle de eventos (timeout ~16 ms): avanza la animación del
    /// sheet y renderiza un lote de portadas pendientes de la biblioteca.
    /// `lib::android_main` lo invoca en los eventos Wake/Timeout, que solo
    /// ocurren mientras `sheet_animating()` o `thumbs_pending()` (sin
    /// despertar el loop en reposo).
    pub(crate) fn tick(&mut self, app: &AndroidApp) {
        if self.sheet_anim {
            let target = if self.sheet_open { 1.0 } else { 0.0 };
            // Ease exponencial: ~10 ticks (≈ 150 ms) para recorrer el 95 %.
            self.sheet_progress += (target - self.sheet_progress) * 0.3;
            if (target - self.sheet_progress).abs() < 0.01 {
                self.sheet_progress = target;
                self.sheet_anim = false;
            }
            if self.sheet_progress <= 0.0 {
                self.sheet_bitmap = None; // liberar el bitmap al cerrar del todo
            }
            self.redraw();
        }
        if self.mode == UiMode::Library && self.pump_thumbs(app) {
            self.list_dirty = true;
            self.redraw();
        }
    }

    // ---------------------------------------------------------------------
    // Portadas de la biblioteca (perezosas, bajo demanda — ver `thumbs`)
    // ---------------------------------------------------------------------

    /// ¿Hay portadas pendientes entre las celdas VISIBLES de la biblioteca?
    /// El bucle de eventos la usa para mantener el poll con timeout mientras
    /// `pump_thumbs` tiene trabajo (sin batería extra cuando no hay nada).
    pub(crate) fn thumbs_pending(&mut self) -> bool {
        if self.mode != UiMode::Library || self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        let visible = grid_visible_rows(self.win_w, self.win_h, self.status.is_some());
        for row in self.list_scroll..self.list_scroll + visible {
            for col in 0..GRID_COLS {
                // Clonar la URI: `thumbs.get` promueve la recencia (necesita
                // &mut self) y no puede convivir con el préstamo de
                // `grid_entry_at`.
                let Some(uri) = self.grid_entry_at(row, col).map(|e| e.uri.clone()) else {
                    continue;
                };
                if self.thumbs.get(&uri).is_none() && !self.thumb_failed.contains(&uri) {
                    return true;
                }
            }
        }
        false
    }

    /// Renderiza bajo demanda un lote de portadas de las celdas VISIBLES
    /// (máx. 3 por tick, ~1-3 ms cada una): solo las que no están en caché ni
    /// fallaron. Devuelve true si entró alguna portada nueva (→ re-render de
    /// la rejilla). Nunca renderiza las 256 de golpe: por frame se procesa
    /// un lote pequeño y el resto queda pendiente para los ticks siguientes.
    fn pump_thumbs(&mut self, app: &AndroidApp) -> bool {
        if self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        let visible = grid_visible_rows(self.win_w, self.win_h, self.status.is_some());
        let mut budget = 3usize;
        let mut changed = false;
        for row in self.list_scroll..self.list_scroll + visible {
            for col in 0..GRID_COLS {
                if budget == 0 {
                    return changed;
                }
                // Clonar uri+name antes de mutar la caché (préstamos).
                let Some((name, uri)) = self
                    .grid_entry_at(row, col)
                    .map(|e| (e.name.clone(), e.uri.clone()))
                else {
                    continue;
                };
                if self.thumbs.get(&uri).is_some() || self.thumb_failed.contains(&uri) {
                    continue;
                }
                match self.render_thumb(app, &uri) {
                    Some(bmp) => {
                        self.thumbs.insert(uri.clone(), bmp);
                        info!(
                            "thumb {} cached ({} entries / {:.1} MiB)",
                            name,
                            self.thumbs.len(),
                            self.thumbs.resident_bytes() as f64 / (1024.0 * 1024.0)
                        );
                        changed = true;
                    }
                    None => {
                        self.thumb_failed.insert(uri.clone());
                        warn!("thumb failed: {uri}");
                    }
                }
                budget -= 1;
            }
        }
        changed
    }

    /// Renderiza la portada (página 1) de un PDF de la biblioteca: abre la
    /// content:// URI por fd NATIVO (`/proc/self/fd/N`, sin copiar ni leer
    /// el fichero entero — ver `jni::ContentFd`) con MuPDF a `THUMB_W` px de
    /// ancho. `None` si falla (PDF corrupto, fd no abrible, página 1 vacía).
    fn render_thumb(&self, app: &AndroidApp, uri: &str) -> Option<Bitmap> {
        let fd = open_content_fd(app, uri)?;
        let path = fd.proc_path();
        let result: pdf_core::Result<Bitmap> = (|| {
            let engine = MupdfEngine::new()?;
            let doc = engine.open(Path::new(&path))?;
            let (pw, _ph) = doc.page_size(0)?;
            if !pw.is_finite() || pw <= 0.0 {
                return Err(pdf_core::Error::InvalidArgument(
                    "page 1 width invalid".into(),
                ));
            }
            doc.render_page(0, THUMB_W as f32 / pw)
        })();
        fd.close();
        match result {
            Ok(bmp) if bmp.width > 0 && bmp.height > 0 => Some(bmp),
            _ => None,
        }
    }

    /// Fija el anclaje del pinch en curso: el centro del pinch en px de
    /// ventana + el zoom y el pan de partida. `input` lo llama al caer el
    /// segundo dedo (PointerDown); `set_zoom_fast` recalcula después el pan
    /// para que el punto de documento bajo este ancla permanezca fijo.
    pub(crate) fn begin_pinch(&mut self, ax: f32, ay: f32) {
        self.pinch = Some(PinchAnchor {
            ax,
            ay,
            z0: self.zoom,
            pan_x0: self.pan_x,
            pan_y0: self.pan_y,
        });
    }

    /// Zoom DURANTE el pinch (fast): solo actualiza el factor (1.0 = página
    /// completa) y hace un redraw de solo blit — `blit` escala los bitmaps
    /// cacheados de la columna con el zoom RELATIVO `zoom / rendered_zoom`
    /// (vecino-más-cercano), SIN re-renderizar MuPDF. El re-render nítido a la
    /// resolución final se hace UNA vez al soltar el pinch
    /// (`set_zoom_sharp`). El redraw normal (render + blit) no se usa aquí
    /// porque `ensure_pages_rendered` re-renderizaría en cada Move.
    ///
    /// El zoom es un factor RELATIVO a la distancia inicial del gesto
    /// (`zoom = z0 × dist / start_dist`, calculado por `input`); aquí solo se
    /// aplica. Además recalcula el pan de ANCLAJE: el punto de documento que
    /// estaba bajo el centro del pinch al iniciar (`begin_pinch`) se mantiene
    /// fijo en pantalla (ver `anchor_pan`).
    pub(crate) fn set_zoom_fast(&mut self, zoom: f32) {
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        if (self.zoom - zoom).abs() < 1e-4 {
            return;
        }
        if let Some(p) = self.pinch {
            let (dw, dh) = self.page_doc_size_px(self.page);
            if dw > 0.0 && dh > 0.0 {
                // X: el bitmap escalado se centra en la ventana → base
                // dependiente del zoom. Y: el borde superior de la página
                // actual queda en el borde superior del viewport (scroll_y =
                // page_offsets[page]) → base 0.
                self.pan_x = Self::anchor_pan(
                    p.ax,
                    Self::centered_base(self.win_w, dw, p.z0),
                    Self::centered_base(self.win_w, dw, zoom),
                    p.z0,
                    p.pan_x0,
                    zoom,
                );
                self.pan_y = Self::anchor_pan(p.ay, 0.0, 0.0, p.z0, p.pan_y0, zoom);
            }
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
    /// conserva el pan de anclaje calculado por el último `set_zoom_fast` (el
    /// punto bajo los dedos no salta al re-renderizar) y re-renderiza las
    /// páginas visibles UNA única vez a la escala continua resultante (render
    /// directo a resolución de pantalla — el camino medido más rápido en la
    /// tablet, ver nota de rendimiento en la cabecera de `lib.rs`): la caché
    /// se limpia y el layout se reconstruye; el redraw renderiza vía
    /// `ensure_pages_rendered`. Persiste el zoom (solo aquí, al soltar el
    /// gesto: `set_zoom_fast` es transitorio y escribir en cada Move de
    /// 60-120 Hz llenaría el disco).
    pub(crate) fn set_zoom_sharp(&mut self, zoom: f32) {
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        // El pan de anclaje YA es el del zoom final (último `set_zoom_fast`);
        // el re-render a la nueva escala (`rendered_zoom = zoom`) mantiene el
        // mismo mapeo documento→pantalla (la escala efectiva `doc·zoom` no
        // cambia), así que el punto bajo los dedos permanece fijo al soltar.
        // `scroll_y` no se toca: en el modo página a página está siempre
        // alineado con el borde superior de la página actual.
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
        self.page_badge = None; // colores del indicador cambian
        self.sheet_bitmap = None; // colores del sheet cambian (Dark/Light)
        info!("dark mode: {}", self.dark);
        self.save_state();
        self.redraw();
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
    ///
    /// `dead_code` intencional (2026-08-XX): la UI de dibujo se quitó (no se
    /// pueden CREAR trazos desde el visor), pero el camino de guardado se
    /// conserva intacto para que el modelo de anotaciones siga siendo
    /// persistible cuando una fase futura reintroduzca la creación (el
    /// usuario no pierde la capacidad de exportar/sincronizar trazos).
    #[allow(dead_code)]
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
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                self.pinch = None;
                self.bitmap = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.layout_dirty = true;
                self.mode = UiMode::Viewer;
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera
                self.thumbs.clear(); // portadas de otra biblioteca: no sirven
                self.thumb_failed.clear();
                self.list_dirty = true;
                self.picker_drag = None;
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

    /// Entra en la biblioteca MediaStore (botón "Back" del sheet del visor):
    /// re-consulta y deja de mostrar la página. Si MediaStore está vacía,
    /// `rescan_library` cae al picker interno como fallback.
    pub(crate) fn enter_library(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Library;
        self.list_scroll = 0;
        self.list_dirty = true;
        self.bitmap = None;
        self.sheet_hide_now(); // fuera del visor: el sheet no pinta en biblioteca
        self.picker_drag = None;
        self.rescan_library(app);
    }

    /// Abre el picker interno (botón "Open" del sheet del visor): PDFs de
    /// los directorios de la app (el fallback cuando MediaStore no sirve).
    pub(crate) fn open_picker(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Picker;
        self.pdf_list = scan_pdfs(app);
        self.list_scroll = 0;
        self.status = None;
        self.list_dirty = true;
        self.bitmap = None;
        self.sheet_hide_now();
        self.picker_drag = None;
        self.redraw();
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
        // Datos nuevos: el scroll vuelve al principio y la lista visible son
        // TODAS las entradas (la rejilla 3×3 no tiene filtro por letra; ver
        // la nota en la cabecera del módulo).
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

    /// Nº de filas de celdas de la rejilla de la biblioteca (3 columnas).
    pub(crate) fn grid_total_rows(&self) -> usize {
        self.library_list.len().div_ceil(GRID_COLS)
    }

    /// Entrada de la rejilla en la fila `row` (0-based) y columna `col`
    /// (0..GRID_COLS) — resolución directa sobre `library_list` (sin filtro:
    /// la rejilla muestra TODAS las entradas en orden de MediaStore). None si
    /// la celda está fuera de rango (fila incompleta de la última fila).
    pub(crate) fn grid_entry_at(&self, row: usize, col: usize) -> Option<&LibraryEntry> {
        self.library_list.get(row * GRID_COLS + col)
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
