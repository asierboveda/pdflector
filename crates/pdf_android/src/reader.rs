// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

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
use base64::Engine;
use log::{error, info, warn};
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::store::{AnnotationStore, sidecar_path};
use pdf_core::{
    Annotation, AnnotationSet, Bitmap, Color, Document, Highlight, Rect, RenderEngine, Stroke,
    TextSpan,
};

use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES, PageCache};
use crate::draw::{
    ButtonRect, PageAnnots, PageBlit, ai_panel_layout, blit_composed, blit_page, compose_frame,
    render_ai_panel, render_library_grid, render_page_badge, render_picker_list, render_sel_menu,
    render_sheet, render_toast, sel_menu_layout,
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
use crate::{BACKGROUND, DARK_BG, ERROR_BG, PINCH_MAX, PINCH_MIN, SEL_MIN_PX, TOAST_MS};

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

/// Pad exterior horizontal de la rejilla (px): margen fijo de 16 px, estilo
/// Apple Books (e-reader: portadas con aire a los bordes, sin relleno denso).
pub(crate) fn grid_pad(_win_w: i32) -> f32 {
    16.0
}

/// Separación entre celdas de la rejilla (px).
pub(crate) fn grid_gap() -> f32 {
    14.0
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

/// Alto (px) del área de portada: proporción 2:3 (alto = ancho × 1.5), estilo
/// Apple Books, para TODAS las celdas (rejilla uniforme). La miniatura real
/// (con su propia proporción) se recorta en centro al pegarse (`paste_thumb`,
/// center-crop: escala hasta rellenar y recorta el sobrante, nunca letterbox).
pub(crate) fn grid_cover_h(win_w: i32) -> f32 {
    grid_cover_w(win_w) * 1.5
}

/// Alto (px) de la zona de título de la celda: 1 línea de título (~13 sp,
/// ≈ 16 px) + aire generoso hasta la fila siguiente, estilo Apple Books.
pub(crate) fn grid_title_h(_win_w: i32) -> f32 {
    40.0
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

/// Selección de texto en curso (rectángulo de arrastre del doble-tap): ancla
/// (punto del doble-tap) y punto actual del dedo, ambos en px de VENTANA
/// (pantalla).
///
/// Decisión documentada: la selección se guarda en coords de PANTALLA (no de
/// página) porque el gesto, el render del rect y el menú viven en pantalla y
/// la conversión a página solo se hace UNA vez cuando se necesita
/// (`sel_page_rect`, con `screen_to_page` — la INVERSA exacta del mapeo del
/// blit, misma `scale = cover × zoom` y `dx/dy` que la capa de anotaciones).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SelState {
    /// Punto del doble-tap (px de ventana): esquina fija del rect.
    pub(crate) anchor: (f32, f32),
    /// Posición actual del dedo (px de ventana): esquina móvil del rect.
    pub(crate) cur: (f32, f32),
}

/// Menú flotante de la selección fijada (Copiar / Subrayar / IA): tarjeta
/// pequeña cerca del rect de selección con sus botones (etiqueta + rect en px
/// de ventana — geometría COMPARTIDA por el render y el tap de `input`). Se
/// muestra al soltar el arrastre (`end_sel`); tocar fuera lo cierra y
/// descarta la selección. "IA" es un hueco visual para la Parte 2 (otro
/// agente): se dibuja atenuado y su tap solo avisa.
pub(crate) struct SelMenu {
    /// Esquina superior izquierda del menú en px de ventana.
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// Tamaño del menú en px (el del bitmap cacheado).
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// Bitmap del menú (Canvas+JNI, fondo transparente), cacheado mientras
    /// el menú esté abierto.
    pub(crate) bitmap: Bitmap,
    /// Botones (etiqueta + rect en px de ventana), compartidos con el tap.
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
}

/// Fase del panel de "Preguntar a la IA" (Parte 2): decide el título, el
/// color del cuerpo y qué muestra el panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiPhase {
    /// Consulta en vuelo: el hilo de fondo aún no ha devuelto nada (el panel
    /// muestra "preguntando…" y `tick` sigue sondeando el canal).
    Asking,
    /// Respuesta del modelo lista (texto envuelto en `AiPanel::lines`).
    Answer,
    /// La consulta falló (sin red, key inválida, error HTTP/JSON...): el
    /// panel muestra el error en rojo en el mismo sitio que la respuesta.
    Error,
}

/// Panel flotante de "Preguntar a la IA" (Parte 2): tarjeta tipo `SelMenu`
/// con cabecera (título + botones ✕/▲/▼) y cuerpo de texto envuelto en
/// varias líneas; si el texto desborda el cuerpo, el scroll (▲/▼) muestra
/// solo una ventana de líneas (`scroll..scroll+visible`) — el render
/// (`draw::render_ai_panel`) salta las líneas fuera de la ventana, así que
/// el recorte es gratis. Geometría y bitmaps cacheados mientras esté
/// abierto; el tap vive en `input::ai_panel_tap` (misma geometría
/// compartida que `SelMenu`).
pub(crate) struct AiPanel {
    /// Esquina superior izquierda del panel en px de ventana.
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// Tamaño del panel en px (el del bitmap cacheado).
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// Botones (etiqueta + rect en px de ventana): "×" (cerrar, siempre) y
    /// "▲"/"▼" (scroll, solo si `scrollable`). Compartidos con el tap.
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
    /// Bitmap del panel (cabecera + líneas VISIBLES del cuerpo), cacheado
    /// mientras el panel esté abierto; se re-renderiza al hacer scroll.
    pub(crate) bitmap: Bitmap,
    /// Número total de líneas envueltas del texto actual (`ai_text`).
    pub(crate) lines: usize,
    /// Primera línea visible en el cuerpo (0 = principio).
    pub(crate) scroll: usize,
    /// Máximo de líneas visibles a la vez (alto del cuerpo / alto de línea).
    pub(crate) visible: usize,
    /// ¿El texto desborda el cuerpo? (true → botones ▲/▼ y recorte).
    pub(crate) scrollable: bool,
}

/// Estado de la app, vivo durante todo el bucle de `android_main`.
/// `pub(crate)` por la partición de `lib.rs`: `input` y `draw` leen campos,
/// `lib` llama a los métodos (gestos y listas viven en otros módulos).
pub(crate) struct Reader {
    pub(crate) doc: Option<MupdfDocument>,
    /// Página actual, 0-based: la ÚNICA hoja que se dibuja (modo UNA HOJA,
    /// sin columna de páginas). Alimenta el indicador "N / total", los saltos
    /// ±10 y la persistencia.
    pub(crate) page: u32,
    /// Referencia owned al ANativeWindow (Some entre InitWindow y TerminateWindow).
    window: Option<NativeWindow>,
    /// Bitmap de la LISTA del picker/biblioteca (render de pantalla completa
    /// con Canvas+JNI). Las páginas del visor viven en `cache` (PageCache);
    /// este campo solo lo usan los modos Picker/Library.
    pub(crate) bitmap: Option<Bitmap>,
    /// Caché LRU de páginas renderizadas (página → Bitmap) para el paso de
    /// página INSTANTÁNEO (prev/next): evita re-renderizar al volver atrás y
    /// precarga la vecina (`ensure_pages_rendered`). Guarda SIEMPRE bitmaps
    /// normales; la inversión de modo oscuro se aplica al blitear
    /// (`draw::blit_page`). SOLO se dibuja la página actual (modo UNA HOJA);
    /// las vecinas solo se cachean.
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
    /// Frame de página compuesto (fondo, página actual, capa de anotaciones
    /// e indicador "N / total") en un `Bitmap` RGBA8 del tamaño de la ventana.
    /// Se compone UNA vez al empezar a deslizar el sheet (`sheet_progress > 0`,
    /// en `blit`) y se reutiliza mientras el sheet esté visible: cada frame de
    /// la animación/arrastre copia este bitmap (`draw::blit_composed`, memcpy
    /// ~1-2 ms) + el overlay del sheet, en vez de re-blitear la página
    /// completa en cada paso (~25-40 ms/frame — la CAUSA del lag del sheet;
    /// ver `blit` y `draw::compose_frame`). Se invalida (None) al cambiar
    /// página, zoom, modo oscuro, ventana o documento; se libera al cerrar el
    /// sheet del todo.
    page_frame: Option<Bitmap>,
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
    /// Selección de texto en curso (doble-tap + arrastre) en px de ventana
    /// (ver `SelState`): Some durante el arrastre Y mientras está fijada con
    /// su menú abierto (`sel_menu`); se descarta al tocar fuera del menú o al
    /// ejecutar Copiar/Subrayar. None = sin selección activa
    /// (`has_selection`).
    pub(crate) sel: Option<SelState>,
    /// Menú flotante de la selección fijada (Copiar/Subrayar/IA): bitmap +
    /// posición/geometría en px de ventana (ver `SelMenu`). Some mientras el
    /// menú esté abierto; tocar fuera lo cierra y descarta la selección.
    pub(crate) sel_menu: Option<SelMenu>,
    /// Panel flotante de "Preguntar a la IA" (Parte 2): tarjeta tipo
    /// `SelMenu` con cabecera (título + ✕/▲/▼) y cuerpo de texto envuelto
    /// con scroll (ver `AiPanel`). Some mientras esté abierto (fase
    /// Asking/Answer/Error); se abre al tocar "IA" en el menú de selección
    /// (`ask_ai`) y se cierra con ✕ o tap fuera (`close_ai_panel`).
    pub(crate) ai_panel: Option<AiPanel>,
    /// Texto actual del panel de IA: "preguntando…" mientras la consulta
    /// está en vuelo, la respuesta del modelo o el mensaje de error. Lo
    /// consume `draw::ai_panel_layout` para envolver las líneas.
    pub(crate) ai_text: String,
    /// Fase del panel de IA (`AiPhase`): decide el título, el color del
    /// cuerpo y el flujo del tap. Separada del panel para que el render
    /// (`draw::render_ai_panel`) la lea sin dependencias circulares.
    pub(crate) ai_phase: AiPhase,
    /// Receptor del hilo de fondo de Groq (std::thread + mpsc, el patrón de
    /// `pdf_core::prefetch`): Some mientras una consulta está en vuelo.
    /// `tick` lo sondea con `try_recv` (sin bloquear) y lo libera al llegar
    /// el resultado o al cerrar el panel. None = sin consulta activa.
    ai_rx: Option<std::sync::mpsc::Receiver<pdf_core::ai::Result<String>>>,
    /// Aviso breve ("copied", "highlighted", "no text", ...) sobre el
    /// indicador de página: texto + momento de creación; `tick` lo expira a
    /// los `TOAST_MS` (1,5 s) y el bitmap cacheado se invalida con el texto.
    pub(crate) toast: Option<(String, Instant)>,
    /// Bitmap cacheado del aviso breve (`draw::render_toast`), None sin
    /// aviso o con texto nuevo (se re-renderiza al cambiarlo).
    toast_bitmap: Option<Bitmap>,
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
            page_frame: None,
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
            sel: None,
            sel_menu: None,
            ai_panel: None,
            ai_text: String::new(),
            ai_phase: AiPhase::Asking,
            ai_rx: None,
            toast: None,
            toast_bitmap: None,
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
                            // Modo UNA HOJA: la página restaurada se fija
                            // directamente (no hay scroll que alinear).
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
        self.page_frame = None;
        self.list_dirty = true;
        // Nueva ventana → posible nueva escala cover: las páginas de la caché
        // se reutilizan si el tamaño no cambió; el redraw detecta el cambio de
        // `win_w/h` y limpia la caché si hace falta.
        self.redraw();
    }

    /// `TerminateWindow`: soltar la ventana (drop → `ANativeWindow_release`).
    pub(crate) fn terminate_window(&mut self) {
        self.window = None;
        self.bitmap = None;
        self.page_badge = None;
        self.sheet_bitmap = None;
        self.page_frame = None;
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
            self.list_dirty = true;
            self.page_badge = None;
            self.sheet_bitmap = None;
            self.page_frame = None;
        }
        match self.mode {
            UiMode::Viewer => {
                // Modo UNA HOJA: página actual + 1 vecina por lado (prefetch
                // simple para que prev/next sea instantáneo), vía caché LRU.
                // SOLO se dibuja la página actual (`blit`): las vecinas solo
                // se cachean.
                self.ensure_pages_rendered();
                // Indicador de página (abajo a la izquierda) y sheet de
                // ajustes, cacheados: se re-renderizan solo si cambió la
                // ventana, la página o el modo oscuro (invalidadores en
                // `goto_page`, `toggle_dark`, `set_zoom_*` y el resize de
                // arriba). El sheet se materializa solo cuando empieza a
                // verse y se libera al cerrar del todo.
                if self.doc.is_some() && self.page_badge.is_none() {
                    self.page_badge = render_page_badge(self);
                }
                if self.sheet_progress > 0.0 && self.sheet_bitmap.is_none() {
                    self.sheet_bitmap = render_sheet(self);
                }
                if self.sheet_progress <= 0.0 {
                    self.sheet_bitmap = None;
                    self.page_frame = None; // liberar el frame al cerrar del todo
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
        if self.window.is_some() {
            self.blit();
        }
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
    /// fijo en el borde superior del viewport — modo UNA HOJA, sin scroll).
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

    /// Clamp del pan de anclaje a los bordes de la hoja (modo UNA HOJA): con
    /// zoom-in la página se puede mover HASTA sus bordes pero un borde de la
    /// hoja NUNCA entra dentro de la ventana (nada de fondo visible alrededor
    /// de la página). `page` es el tamaño de la página en pantalla (`dw·zoom`
    /// o `dh·zoom`) y `win` el tamaño de la ventana en ese eje (f32).
    ///
    /// Geometría real del blit (`blit`): la página ocupa en pantalla
    /// `[base + pan, base + pan + page]`, con `base` la posición de origen
    /// del bitmap escalado SIN pan — centrada en X (`centered_base`,
    /// `align_top = false`) y alineada al borde superior en Y (`base = 0`,
    /// `align_top = true`; el anclaje Y real del pinch es "arriba",
    /// confirmado en `blit`: `dy = pan_y`).
    ///
    /// - Si `page >= win` (la página es más grande que la ventana): exige
    ///   cubrirla entera, `base + pan <= 0` y `base + pan + page >= win`, o
    ///   sea `pan ∈ [win − page − base, −base]`. En Y (`base = 0`) queda
    ///   `pan.clamp(win − page, 0)`; en X el rango se desplaza por el
    ///   centrado de `centered_base`: `[(win − page)/2, (page − win)/2]`.
    /// - Si `page < win` (página más pequeña; solo posible con zoom < 1):
    ///   centrada en X (el centrado ya lo hace `centered_base` → pan 0) y
    ///   arriba en Y (pan 0).
    fn clamp_pan(pan: f32, page: f32, win: f32, align_top: bool) -> f32 {
        if page >= win {
            let base = if align_top { 0.0 } else { (win - page) / 2.0 };
            pan.clamp(win - page - base, -base)
        } else {
            0.0
        }
    }

    /// Garantiza en la caché la página actual + 1 vecina por cada lado
    /// (prefetch simple para que prev/next sea INSTANTÁNEO): renderiza solo
    /// los miss, a `cover × rendered_zoom` (la escala de la caché), y
    /// promueve la recencia LRU de las páginas que toca. El render es
    /// síncrono en el hilo del bucle (~18-25 ms/página en la tablet); el
    /// prefetch adelanta la vecina para que el tap de página entre en ella
    /// sin re-render en el momento de volverse visible. En el modo UNA HOJA
    /// SOLO se DIBUJA la página actual (`blit`): las vecinas solo se cachean.
    fn ensure_pages_rendered(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let n = doc.page_count();
        if n == 0 {
            return;
        }
        let lo = self.page.saturating_sub(1);
        let hi = (self.page + 1).min(n - 1);
        for page in lo..=hi {
            if self.cache.get(page).is_some() {
                continue; // hit: sin re-render (volver atrás es instantáneo)
            }
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

    /// Blitea el frame actual al buffer del ANativeWindow con UN solo
    /// lock+present.
    ///
    /// - Visor (modo UNA HOJA): `draw::blit_page` dibuja el fondo y SOLO la
    ///   página actual (centrado cover + pan de anclaje del pinch; nunca otra
    ///   hoja), recortando a la ventana; los overlays (indicador de página +
    ///   sheet de ajustes) van después en el mismo buffer. Zoom RELATIVO
    ///   `zoom / rendered_zoom`: 1:1 nítido para bitmaps recién renderizados
    ///   y escala vecino-más-cercana durante el pinch (sin re-render). Con el
    ///   sheet visible se usa el FRAME COMPUESTO (`page_frame`, ver abajo):
    ///   el frame se compone una vez y cada frame de la animación copia ese
    ///   bitmap (`draw::blit_composed`) + el overlay del sheet — la PÁGINA
    ///   NO se re-blitea en cada paso de la animación (el fix del lag del
    ///   sheet; ver `draw::compose_frame`).
    /// - Picker/Biblioteca: `zoom::blit_fast` con el bitmap de la lista.
    ///
    /// Aquí se decide SOLO el estado que depende del `Reader`: fondo rojo sin
    /// documento, modo oscuro (inversión al blitear) y overlays del visor.
    fn blit(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
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
                // Piezas del blit de la página actual (una sola hoja): bitmap
                // cacheado (`peek`, sin promoción LRU — el blit solo lee; la
                // recencia la fija el render/prefetch), centrado horizontal
                // cover + pan de anclaje (puede ser negativo con zoom > 1: se
                // recortan los bordes de la página) y la capa de anotaciones.
                let blit_zoom = if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
                    self.zoom / self.rendered_zoom
                } else {
                    1.0
                };
                let page_blit: Option<PageBlit> = self.cache.peek(self.page).map(|bmp| {
                    let dx = (((self.win_w as f32 - bmp.width as f32 * blit_zoom) / 2.0)
                        + self.pan_x)
                        .round() as i32;
                    // Una sola hoja: sin columna ni scroll_y → dy = pan.
                    let dy = self.pan_y.round() as i32;
                    PageBlit {
                        bitmap: bmp,
                        dx,
                        dy,
                        zoom: blit_zoom,
                    }
                });
                // Anotaciones de la página, en orden de dibujo (z): trazos y
                // highlights guardados (Stroke/Highlight; TextNote no se
                // dibuja aún). Los highlights se dibujan DEBAJO de los trazos
                // (`draw::draw_annotations`); el trazo/highlight en curso no
                // existe (no hay modo dibujo; el rect de selección en vivo va
                // aparte, ver `sel_rect` abajo).
                let anns: Option<PageAnnots> = page_blit.as_ref().and_then(|pb| {
                    let page_anns = self.annotations.for_page(self.page as usize);
                    let strokes: Vec<&Stroke> = page_anns
                        .iter()
                        .filter_map(|a| match &a.kind {
                            Annotation::Stroke(s) => Some(s),
                            _ => None,
                        })
                        .collect();
                    let highlights: Vec<&Highlight> = page_anns
                        .iter()
                        .filter_map(|a| match &a.kind {
                            Annotation::Highlight(h) => Some(h),
                            _ => None,
                        })
                        .collect();
                    if strokes.is_empty() && highlights.is_empty() {
                        return None;
                    }
                    // scale = cover × zoom: px de ventana por punto PDF (la
                    // misma escala efectiva del blit). Si no se puede saber el
                    // tamaño de la página (render fallido), escala degradada.
                    let scale = match self.doc.as_ref().and_then(|d| d.page_size(self.page).ok()) {
                        Some((pw, ph)) => initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom,
                        None => blit_zoom,
                    };
                    Some(PageAnnots {
                        dx: pb.dx,
                        dy: pb.dy,
                        scale,
                        strokes,
                        highlights,
                    })
                });
                // Rect de selección en vivo/fijado (px de ventana, recortado
                // a los bordes de la PÁGINA por `sel_screen_rect`): se dibuja
                // como capa translúcida sobre la página, antes de los
                // overlays (`draw::blit_page`/`compose_frame`).
                let sel_rect = self.sel_screen_rect();
                // Aviso breve ("copied", ...): bitmap cacheado materializado
                // aquí para que las dos rutas de blit (normal y frame
                // compuesto del sheet) lo usen como overlay.
                if self.toast.is_some() && self.toast_bitmap.is_none() {
                    self.toast_bitmap = render_toast(self);
                }
                let toast_ov: Option<(&Bitmap, i32, i32)> = self.toast_bitmap.as_ref().map(|tb| {
                    let (_, by, _, _) = page_badge_rect(self.win_w, self.win_h);
                    let tx = (self.win_w - tb.width as i32) / 2;
                    let ty = by - tb.height as i32 - 8;
                    (tb, tx, ty)
                });
                // Indicador de página (abajo a la izquierda, siempre): se usa
                // en el blit normal y también dentro del frame compuesto.
                let badge: Option<(&Bitmap, i32, i32)> = self.page_badge.as_ref().map(|b| {
                    let (bx, by, _, _) = page_badge_rect(self.win_w, self.win_h);
                    (b, bx, by)
                });
                // Overlays del visor en el MISMO buffer (un solo lock+present):
                // indicador de página, menú de selección, panel de IA, aviso
                // breve y sheet de ajustes deslizado desde el borde superior
                // (solo si está visible; `progress == 1` = abierto del todo).
                // El menú, el panel y el aviso van SIEMPRE (también con el
                // sheet: se añaden al frame compuesto o como overlays de
                // `blit_composed`).
                let mut overlays: Vec<(&Bitmap, i32, i32)> = Vec::with_capacity(5);
                if let Some((b, bx, by)) = badge {
                    overlays.push((b, bx, by));
                }
                if let Some(menu) = self.sel_menu.as_ref() {
                    overlays.push((&menu.bitmap, menu.x, menu.y));
                }
                if let Some(panel) = self.ai_panel.as_ref() {
                    overlays.push((&panel.bitmap, panel.x, panel.y));
                }
                if let Some((tb, tx, ty)) = toast_ov {
                    overlays.push((tb, tx, ty));
                }
                if self.sheet_progress > 0.0
                    && let Some(s) = self.sheet_bitmap.as_ref()
                {
                    let slide =
                        (sheet_h(self.win_h) as f32 * (1.0 - self.sheet_progress)).round() as i32;
                    overlays.push((s, 0, -slide));
                }
                if self.sheet_progress > 0.0 {
                    // Sheet visible: frame compuesto + overlay del sheet. El
                    // frame (fondo + página + anotaciones + indicador) se
                    // compone UNA vez al empezar a deslizar y se reutiliza
                    // mientras el sheet esté visible: cada frame de la
                    // animación/arrastre copia el frame (memcpy ~1-2 ms) +
                    // el sheet, en vez de re-blitear la página completa
                    // (~25-40 ms/frame — la CAUSA del lag del sheet).
                    if self.page_frame.is_none() {
                        let composed = compose_frame(
                            self.win_w,
                            self.win_h,
                            bg,
                            self.dark,
                            page_blit.as_ref(),
                            anns.as_ref(),
                            sel_rect,
                            badge,
                        );
                        self.page_frame = Some(composed);
                    }
                    if let Some(frame) = self.page_frame.as_ref() {
                        // El frame ya incluye el indicador y el rect de
                        // selección: menú, aviso breve y sheet como overlays.
                        let mut sheet_ov: Vec<(&Bitmap, i32, i32)> = Vec::with_capacity(4);
                        if let Some(menu) = self.sel_menu.as_ref() {
                            sheet_ov.push((&menu.bitmap, menu.x, menu.y));
                        }
                        if let Some(panel) = self.ai_panel.as_ref() {
                            sheet_ov.push((&panel.bitmap, panel.x, panel.y));
                        }
                        if let Some((tb, tx, ty)) = toast_ov {
                            sheet_ov.push((tb, tx, ty));
                        }
                        if let Some(s) = self.sheet_bitmap.as_ref() {
                            let slide = (sheet_h(self.win_h) as f32 * (1.0 - self.sheet_progress))
                                .round() as i32;
                            sheet_ov.push((s, 0, -slide));
                        }
                        blit_composed(window, frame, &sheet_ov);
                    } else {
                        // Defensa: frame no disponible (compose falló) → blit
                        // normal con el sheet como overlay.
                        blit_page(
                            window,
                            bg,
                            self.dark,
                            page_blit.as_ref(),
                            anns.as_ref(),
                            sel_rect,
                            &overlays,
                        );
                    }
                } else {
                    blit_page(
                        window,
                        bg,
                        self.dark,
                        page_blit.as_ref(),
                        anns.as_ref(),
                        sel_rect,
                        &overlays,
                    );
                }
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

    /// Cambia a la página `page` (0-based) — modo UNA HOJA: `page` se fija
    /// directamente (no hay scroll que alinear: la columna de páginas se
    /// eliminó). Base compartida de `next_page`/`prev_page`/`jump_page` y del
    /// tap derecho/izquierdo. No hay salto con re-render: las páginas vecinas
    /// salen de la caché (paso instantáneo). Invalida los overlays cacheados
    /// (indicador, sheet, frame de la animación).
    fn goto_page(&mut self, page: u32) {
        self.page = page;
        self.page_badge = None; // el indicador "N / total" cambia
        self.sheet_bitmap = None; // el indicador del sheet cambia
        self.page_frame = None; // el frame de la animación del sheet cambia
        info!("page {}", self.page + 1);
        self.redraw();
        self.save_state();
    }

    pub(crate) fn next_page(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1);
        if self.page < last {
            self.goto_page(self.page + 1);
        }
    }

    pub(crate) fn prev_page(&mut self) {
        if self.page > 0 {
            self.goto_page(self.page - 1);
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
            self.goto_page(target);
        }
    }

    // ---------------------------------------------------------------------
    // Selección de texto: doble-tap + arrastre, copiar y subrayar (Parte 1)
    // ---------------------------------------------------------------------
    //
    // El gesto vive en `input.rs` (doble-tap sin levantar + arrastre); aquí
    // el estado (`sel`/`sel_menu`), las transformaciones de coords, la
    // extracción de texto y las acciones Copiar/Subrayar. Decisiones
    // documentadas en `SelState` (coords de pantalla) y en el doc de la
    // cabecera de `lib.rs`.

    /// ¿Hay una selección activa (en curso o fijada con su menú)? El tap
    /// simple izq/der de página NO se dispara mientras tanto (ver
    /// `input::sel_menu_tap`/`fire_tap_action`).
    ///
    /// `dead_code` intencional (2026-08-XX): los gestos consultan el estado
    /// directamente (`sel`/`sel_menu`) y esta es la API pública que pide la
    /// Parte 1 para que otros agentes (p. ej. la Parte 2 —IA—) sepan si hay
    /// selección sin tocar el estado interno.
    #[allow(dead_code)]
    pub(crate) fn has_selection(&self) -> bool {
        self.sel.is_some()
    }

    /// Comienza el arrastre de selección: ancla = punto del doble-tap y
    /// punto actual = el mismo (el rect crece con `update_sel`). Solo se
    /// llama tras moverse > `SELECT_SLOP` desde el segundo down
    /// (`input`, `GestureKind::Selecting`). Blit directo (sin re-render):
    /// como en el pinch, la página está cacheada y solo cambia la capa.
    pub(crate) fn begin_sel(&mut self, ax: f32, ay: f32) {
        self.sel = Some(SelState {
            anchor: (ax, ay),
            cur: (ax, ay),
        });
        self.page_frame = None; // el frame compuesto incluiría el rect viejo
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Actualiza el punto actual del arrastre (posición del dedo) y
    /// redibuja el rect (blit directo, página cacheada).
    pub(crate) fn update_sel(&mut self, cx: f32, cy: f32) {
        if let Some(s) = self.sel.as_mut() {
            s.cur = (cx, cy);
        }
        self.page_frame = None;
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Fija la selección al levantar el dedo: si el rect es significativo
    /// (≥ `SEL_MIN_PX` por lado) abre el menú Copiar/Subrayar/IA; un
    /// doble-tap sin arrastre (rect degenerado) se descarta.
    pub(crate) fn end_sel(&mut self) {
        let Some((l, t, r, b)) = self.sel_screen_rect() else {
            self.clear_selection(); // no hubo arrastre
            return;
        };
        if (r - l).abs() < SEL_MIN_PX || (b - t).abs() < SEL_MIN_PX {
            self.clear_selection(); // doble-tap sin arrastre: nada que fijar
            return;
        }
        self.open_sel_menu();
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Descarta la selección y su menú (si los hay) y redibuja solo si había
    /// algo visible que quitar. Es la acción de "tocar fuera del menú" y la
    /// limpieza de cualquier transición (cambio de página/documento, gesto
    /// cancelado, segundo dedo).
    pub(crate) fn clear_selection(&mut self) {
        let had = self.sel.is_some() || self.sel_menu.is_some();
        self.sel = None;
        self.sel_menu = None;
        self.page_frame = None;
        if had && self.window.is_some() {
            self.blit();
        }
    }

    /// Transformación pantalla → página (px de ventana → puntos PDF): la
    /// INVERSA exacta del mapeo del blit (`screen = (dx, dy) + pt × scale`,
    /// con `scale = cover × zoom` y `dx/dy` la esquina del bitmap escalado —
    /// centrado cover + pan de anclaje; ver `blit` y `PageAnnots`). Es la
    /// misma familia de transformación que usan el pinch (`anchor_pan`) y la
    /// capa de anotaciones, así que el rect de selección queda alineado con
    /// lo que se ve. None si la página actual no está disponible.
    fn screen_to_page(&self, sx: f32, sy: f32) -> Option<(f32, f32)> {
        let doc = self.doc.as_ref()?;
        let (pw, ph) = doc.page_size(self.page).ok()?;
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        let scale = cover * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let dx = (Self::centered_base(self.win_w, pw * cover, self.zoom) + self.pan_x).round();
        let dy = self.pan_y.round();
        Some(((sx - dx) / scale, (sy - dy) / scale))
    }

    /// Rectángulo de la página actual en px de ventana (left, top, right,
    /// bottom): la posición del bitmap escalado + su tamaño a la escala
    /// efectiva `cover × zoom` — la MISMA geometría del blit. None si la
    /// página no está disponible. Se usa para RECORTAR el rect de selección
    /// a los bordes de la hoja (nunca a la ventana entera).
    fn page_screen_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.as_ref()?;
        let (pw, ph) = doc.page_size(self.page).ok()?;
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        let scale = cover * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let dx = (Self::centered_base(self.win_w, pw * cover, self.zoom) + self.pan_x).round();
        let dy = self.pan_y.round();
        Some((dx, dy, dx + pw * scale, dy + ph * scale))
    }

    /// Rect normalizado de la selección en px de ventana (left, top, right,
    /// bottom), RECORTADO a los bordes de la página actual: si el dedo
    /// arrastra fuera de la hoja (letterbox/pan), el rect se detiene en el
    /// borde. None sin selección o sin página.
    pub(crate) fn sel_screen_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let s = self.sel?;
        let l = s.anchor.0.min(s.cur.0);
        let r = s.anchor.0.max(s.cur.0);
        let t = s.anchor.1.min(s.cur.1);
        let b = s.anchor.1.max(s.cur.1);
        let (pl, pt, pr, pb) = self.page_screen_rect()?;
        Some((l.max(pl), t.max(pt), r.min(pr), b.min(pb)))
    }

    /// Rect de la selección en coordenadas de PÁGINA (puntos PDF): convierte
    /// las dos esquinas del rect de pantalla con `screen_to_page`. Lo usan
    /// la extracción de texto (`sel_text`) y el subrayado
    /// (`highlight_sel`) — la ÚNICA conversión a página que se hace.
    pub(crate) fn sel_page_rect(&self) -> Option<Rect> {
        let (l, t, r, b) = self.sel_screen_rect()?;
        let a = self.screen_to_page(l, t)?;
        let c = self.screen_to_page(r, b)?;
        // `Rect::new` normaliza extents negativos (defensa; ya ordenado).
        Some(Rect::new(a.0, a.1, c.0 - a.0, c.1 - a.1))
    }

    /// Extrae el texto bajo la selección actual: llama a `doc.text(page)`
    /// UNA sola vez y concatena el texto de los spans cuyo bbox INTERSECTA
    /// el rect de selección en página, en orden de lectura (ordenados por y
    /// y luego x).
    ///
    /// Devuelve cadena vacía si no hay selección, si la página no tiene
    /// texto extraíble (p. ej. PDF ESCANEADO: `spans` vacío) o si el rect no
    /// cubre ningún span — en ese caso "Copiar" avisa "no text" en vez de
    /// copiar basura.
    pub(crate) fn sel_text(&self) -> String {
        let Some(page_rect) = self.sel_page_rect() else {
            return String::new();
        };
        let Some(doc) = self.doc.as_ref() else {
            return String::new();
        };
        let Ok(pt) = doc.text(self.page) else {
            return String::new();
        };
        if pt.spans.is_empty() {
            return String::new(); // PDF escaneado / sin texto extraíble
        }
        // Intersección de bbox en coords de página (el rect ya está en
        // página): `span` corta al rect si sus bordes se solapan.
        let (px, py) = (page_rect.x, page_rect.y);
        let (qx, qy) = (page_rect.x + page_rect.w, page_rect.y + page_rect.h);
        let mut hits: Vec<&TextSpan> = pt
            .spans
            .iter()
            .filter(|s| s.x < qx && s.x + s.w > px && s.y < qy && s.y + s.h > py)
            .collect();
        // Orden de lectura: por y (fila), luego x (izquierda → derecha).
        // MuPDF ya devuelve los spans en orden aproximado de lectura, pero
        // el sort garantiza el orden aunque la selección cruce columnas.
        hits.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        // Los spans son líneas de texto (stext line): se unen con salto de
        // línea para conservar la lectura por filas al copiar.
        hits.iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// PNG base64 de la REGIÓN seleccionada (para "Preguntar a la IA" con
    /// visión: ecuaciones/gráficos, Fase 5): recorta del bitmap cacheado de
    /// la página actual el rect de selección (px de ventana,
    /// `sel_screen_rect`) y lo codifica a PNG en memoria → base64 SIN
    /// prefijo (el contrato de `GroqClient::chat_vision`; el prefijo
    /// `data:image/png;base64,` lo añade pdf_core). None si no hay bitmap,
    /// escala inválida o el crop queda vacío (zoom/pan raro) — el llamador
    /// cae al envío solo-texto.
    ///
    /// Mapeo ventana → píxeles del bitmap: la MISMA geometría del blit
    /// (`blit`): el bitmap se dibuja en `(dx, dy)` escalado por
    /// `blit_zoom = zoom / rendered_zoom`, así que un px de pantalla `s`
    /// cae en el px de bitmap `(s − origen) / blit_zoom`. Se usa floor/ceil
    /// para que el crop cubra al menos la región seleccionada y se recorta
    /// a los bordes del bitmap (clamp) — el rect de selección ya viene
    /// recortado a la hoja por `sel_screen_rect`, pero el pan puede dejar
    /// parte del rect fuera del bitmap.
    ///
    /// Modo oscuro: la caché guarda SIEMPRE bitmaps normales (la inversión
    /// se aplica al blitear, `draw::blit_page`), así que el crop sale con
    /// los colores del DOCUMENTO — decisión documentada: para explicar una
    /// ecuación la información es la misma y el modelo de visión no se
    /// confunde con un fondo negro.
    fn sel_image_png_base64(&self) -> Option<String> {
        let bmp = self.cache.peek(self.page)?;
        let (l, t, r, b) = self.sel_screen_rect()?;
        // Escala de dibujo del bitmap cacheado (relativa a su render): 1:1
        // nítido en reposo (`rendered_zoom == zoom`), vecino-más-cercano del
        // bitmap viejo durante el pinch. Si no es finita (defensa), no hay
        // imagen que mandar.
        let blit_zoom = if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
            self.zoom / self.rendered_zoom
        } else {
            return None;
        };
        if !blit_zoom.is_finite() || blit_zoom <= 0.0 {
            return None;
        }
        // Esquina del bitmap escalado en pantalla (misma aritmética que el
        // blit: centrado horizontal cover + pan de anclaje; Y alineado
        // arriba).
        let dx = (((self.win_w as f32 - bmp.width as f32 * blit_zoom) / 2.0) + self.pan_x).round();
        let dy = self.pan_y.round();
        // Rect de selección en píxeles del bitmap, cubriendo al menos lo
        // seleccionado (floor/ceil) y recortado a los bordes del bitmap.
        let x0 = ((l - dx) / blit_zoom).floor();
        let y0 = ((t - dy) / blit_zoom).floor();
        let x1 = ((r - dx) / blit_zoom).ceil();
        let y1 = ((b - dy) / blit_zoom).ceil();
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return None; // NaN/inf (defensa): sin imagen
        }
        let (x0, y0) = (x0.max(0.0) as u32, y0.max(0.0) as u32);
        let (x1, y1) = (
            (x1 as i64).min(bmp.width as i64) as u32,
            (y1 as i64).min(bmp.height as i64) as u32,
        );
        if x0 >= x1 || y0 >= y1 {
            return None; // crop vacío (rect fuera del bitmap): sin imagen
        }
        let (w, h) = (x1 - x0, y1 - y0);
        // Crop fila a fila del RGBA8 row-major (un rango contiguo por fila:
        // `start = (row × width + x0) × 4`).
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y0..y1 {
            let start = ((row * bmp.width + x0) * 4) as usize;
            rgba.extend_from_slice(&bmp.data[start..start + (w as usize) * 4]);
        }
        // Codificación PNG en memoria (encoder `png`, RGBA8 → PNG) y base64
        // sin prefijo (contrato de `chat_vision`).
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&rgba).ok()?;
        }
        Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }

    /// "Copiar": copia el texto de la selección al portapapeles de Android
    /// (`jni::copy_to_clipboard`) y muestra "copied"; si no hay texto (PDF
    /// escaneado) avisa "no text". En ambos casos cierra el menú y descarta
    /// la selección.
    pub(crate) fn copy_sel(&mut self, app: &AndroidApp) {
        let text = self.sel_text();
        if text.is_empty() {
            self.show_toast("no text");
        } else {
            crate::jni::copy_to_clipboard(app, &text);
            self.show_toast("copied");
        }
        self.clear_selection();
    }

    /// "Subrayar": añade al `AnnotationSet` un `Annotation::Highlight` con
    /// el rect de selección en página (amarillo por defecto) y lo PERSISTE
    /// en el sidecar SQLite (`AnnotationStore::save`). El render de
    /// highlights ya existente (`draw::draw_annotations`, relleno
    /// translúcido bajo los trazos) lo muestra al redibujar; el frame
    /// compuesto del sheet se invalida para que lo recoja. Cierra el menú y
    /// descarta la selección.
    pub(crate) fn highlight_sel(&mut self) {
        let Some(rect) = self.sel_page_rect() else {
            self.clear_selection();
            return;
        };
        let ann = Annotation::Highlight(Highlight {
            // El rect de selección completo como un único rect (el modelo
            // permite varios rects por línea; aquí basta con la caja).
            rects: vec![rect],
            // Amarillo por defecto, alfa ~43 % (translúcido sobre el texto).
            color: Color {
                r: 255,
                g: 235,
                b: 59,
                a: 110,
            },
        });
        if self.annotations.add(self.page as usize, ann).is_some() {
            self.save_annotations();
            self.page_frame = None; // el frame compuesto tendría el highlight viejo
            self.show_toast("highlighted");
        } else {
            self.show_toast("highlight failed");
        }
        self.clear_selection();
    }

    /// Aviso breve sobre el indicador de página ("copied", ...): texto +
    /// timestamp; `tick` lo expira a los `TOAST_MS` y el bitmap cacheado se
    /// invalida al cambiar el texto.
    pub(crate) fn show_toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
        self.toast_bitmap = None;
        self.redraw();
    }

    // ---------------------------------------------------------------------
    // "Preguntar a la IA" (Parte 2): hilo de fondo + Groq + panel flotante
    // ---------------------------------------------------------------------
    //
    // El tap en "IA" del menú de selección (`input::sel_menu_tap`) llama a
    // `ask_ai`: se cierra el menú, se abre el panel en fase Asking
    // ("preguntando…") y se lanza un hilo de fondo (std::thread + mpsc, el
    // mismo patrón de `pdf_core::prefetch`) que construye el `GroqClient`
    // (pdf_core::ai, contrato de la Parte 1) con la key embebida
    // (`GROQ_API_KEY`/`GROQ_MODEL` de `lib.rs`) y llama a `chat(system,
    // prompt)` con el texto seleccionado (o `chat_vision` con el PNG de la
    // selección, Fase 5: ecuaciones/gráficos — ver `ask_ai`) y envía el
    // resultado por el canal. El hilo de UI sondea el canal en `tick`
    // (`try_recv`, sin bloquear) y al llegar el mensaje pasa el panel a
    // Answer (texto envuelto con scroll) o Error (mensaje claro en el mismo
    // panel). Decisiones:
    //
    // - La key va EMBEBIDA en el APK (uso personal, sin telemetría; ver
    //   `lib.rs`). Una consulta no se cancela al cerrar el panel: el hilo
    //   termina solo y el resultado se descarta al soltar el receptor.
    // - El hilo de fondo evita bloquear el hilo de UI durante la red
    //   (AGENTS.md §4.6): la generación en Groq puede tardar varios segundos.
    // - Con selección que tenga IMAGEN (crop del bitmap cacheado,
    //   `sel_image_png_base64`) se llama a `chat_vision` con el modelo de
    //   visión (`GROQ_VISION_MODEL`): la imagen es la fuente principal para
    //   ecuaciones/gráficos y el texto extraído va como prompt (puede ser
    //   "" en PDF escaneado). Sin imagen, se cae al `chat` solo-texto.

    /// "IA": lanza la consulta a Groq en un hilo de fondo y abre el panel en
    /// fase "preguntando…". Si no hay ni texto ni imagen aprovechable avisa
    /// "no text" y no abre el panel (mismo comportamiento que Copiar; con
    /// imagen — PDF escaneado — sí abre: la imagen es la fuente principal).
    /// El texto y la imagen se capturan ANTES de cerrar el menú (`sel_text`
    /// y `sel_image_png_base64`).
    pub(crate) fn ask_ai(&mut self) {
        let text = self.sel_text();
        // Imagen de la selección (ecuaciones/gráficos): PNG base64 del crop
        // del bitmap cacheado. None si el crop no es posible (sin bitmap,
        // rect vacío) — en ese caso se cae al envío solo-texto de siempre.
        let image = self.sel_image_png_base64();
        self.clear_selection(); // el panel sustituye al menú de selección
        if text.is_empty() && image.is_none() {
            self.show_toast("no text");
            return;
        }
        info!(
            "ask_ai: {} chars, image {} B PNG -> Groq (text {}, vision {})",
            text.chars().count(),
            image.as_ref().map_or(0, String::len),
            crate::GROQ_MODEL,
            crate::GROQ_VISION_MODEL
        );
        // Panel en fase Asking ("preguntando…") y hilo de fondo con la
        // llamada HTTP: el UI nunca espera por la red.
        self.ai_text = "preguntando…".to_string();
        self.ai_phase = AiPhase::Asking;
        self.rebuild_ai_panel();
        let (tx, rx) = std::sync::mpsc::channel();
        let prompt = text;
        std::thread::spawn(move || {
            let result = match image {
                // Con imagen (ecuación/gráfico): chat_vision con el modelo
                // de visión; el prompt es el texto extraído (puede ser ""
                // en un PDF escaneado — la imagen es la fuente principal).
                Some(b64) => {
                    let client = pdf_core::ai::GroqClient::with_model(
                        crate::GROQ_API_KEY,
                        crate::GROQ_VISION_MODEL,
                    );
                    let system = "Eres un asistente de estudio. Explica de forma clara y concisa lo que se ve en la imagen (ecuación, gráfico o texto).";
                    client.chat_vision(system, &prompt, &b64)
                }
                // Sin imagen: el chat solo-texto de siempre.
                None => {
                    let client = pdf_core::ai::GroqClient::with_model(
                        crate::GROQ_API_KEY,
                        crate::GROQ_MODEL,
                    );
                    let system = "Eres un asistente de estudio. Explica de forma clara y concisa el texto que te dan.";
                    client.chat(system, &prompt)
                }
            };
            // El error también viaja por el canal (`AiError` implementa
            // Display): el hilo de UI decide si es respuesta o error.
            let _ = tx.send(result);
        });
        self.ai_rx = Some(rx);
        self.redraw();
    }

    /// Aplica el resultado del hilo de Groq al panel (fase Answer/Error) y
    /// libera el receptor (deja de sondear el canal y de pedir ticks).
    fn ai_answer(&mut self, text: String, phase: AiPhase) {
        self.ai_text = text;
        self.ai_phase = phase;
        self.ai_rx = None;
        self.rebuild_ai_panel();
        self.redraw();
    }

    /// (Re)construye el panel de IA con el texto y la fase actuales
    /// (`ai_text`/`ai_phase`): layout (`draw::ai_panel_layout`, incluye el
    /// envoltorio de líneas y los botones) + render del bitmap
    /// (`draw::render_ai_panel`). None si la ventana no está lista (se deja
    /// el panel anterior).
    fn rebuild_ai_panel(&mut self) {
        let Some(layout) = ai_panel_layout(self) else {
            return;
        };
        let Some(bitmap) = render_ai_panel(self) else {
            return;
        };
        let (mx, my, mrx, mry) = layout.rect;
        self.ai_panel = Some(AiPanel {
            x: mx as i32,
            y: my as i32,
            w: (mrx - mx) as i32,
            h: (mry - my) as i32,
            buttons: layout.buttons,
            bitmap,
            lines: layout.lines.len(),
            scroll: layout.scroll,
            visible: layout.visible,
            scrollable: layout.scrollable,
        });
    }

    /// Cierra el panel de IA (✕ o tap fuera): descarta el resultado
    /// pendiente si la consulta aún está en vuelo (el hilo de fondo termina
    /// solo y su mensaje se descarta al soltar el receptor).
    pub(crate) fn close_ai_panel(&mut self) {
        let had = self.ai_panel.is_some();
        self.ai_panel = None;
        self.ai_rx = None;
        self.ai_text = String::new();
        self.ai_phase = AiPhase::Asking;
        if had {
            self.redraw();
        }
    }

    /// Scroll del cuerpo del panel de IA (▲/▼, un paso = una línea): solo
    /// si el texto desborda (`scrollable`); re-renderiza el bitmap con la
    /// nueva ventana de líneas visibles.
    pub(crate) fn ai_scroll(&mut self, delta: i32) {
        let (scrollable, lines, visible, scroll) = match &self.ai_panel {
            Some(p) => (p.scrollable, p.lines, p.visible, p.scroll),
            None => return,
        };
        if !scrollable {
            return;
        }
        let max = lines.saturating_sub(visible);
        let target = (scroll as i32 + delta).clamp(0, max as i32) as usize;
        if target == scroll {
            return;
        }
        if let Some(p) = self.ai_panel.as_mut() {
            p.scroll = target;
        }
        // Re-render con la nueva ventana de líneas visibles (ambos lados se
        // evalúan antes de bindear: el bitmap es owned, el préstamo mutable
        // de `ai_panel` vive solo en el cuerpo).
        if let (Some(bmp), Some(p)) = (render_ai_panel(self), self.ai_panel.as_mut()) {
            p.bitmap = bmp;
        }
        self.redraw();
    }

    /// Abre el menú flotante de la selección fijada: calcula la geometría
    /// (`draw::sel_menu_layout`, cerca del rect, dentro de la ventana),
    /// renderiza el bitmap (Canvas+JNI) y guarda ambos en `sel_menu`.
    fn open_sel_menu(&mut self) {
        let Some(layout) = sel_menu_layout(self) else {
            return;
        };
        let Some(bitmap) = render_sel_menu(self) else {
            return;
        };
        let (mx, my, mrx, mry) = layout.rect;
        self.sel_menu = Some(SelMenu {
            x: mx as i32,
            y: my as i32,
            w: (mrx - mx) as i32,
            h: (mry - my) as i32,
            bitmap,
            buttons: layout.buttons,
        });
    }

    /// ¿Trabajo diferido pendiente en el bucle de eventos? (poll con timeout
    /// de 16 ms → `tick`): animación del sheet, portadas de la biblioteca,
    /// tap de página diferido por la ventana de doble-tap o aviso breve
    /// visible. En reposo el poll bloquea sin gastar batería.
    pub(crate) fn needs_tick(&mut self) -> bool {
        self.sheet_anim
            || self.thumbs_pending()
            || self.toast.is_some()
            || self.gesture.tap_pending()
            // Consulta de Groq en vuelo: `tick` sondea el canal del hilo de
            // fondo (sin esto el poll bloquearía y la respuesta tardaría en
            // aparecer hasta el siguiente evento de input).
            || self.ai_rx.is_some()
    }

    // ---------------------------------------------------------------------
    // Sheet de ajustes (panel deslizante desde arriba, 2026-08-XX)
    // ---------------------------------------------------------------------

    /// ¿Animación del sheet en vuelo? La consulta global de trabajo diferido
    /// es `needs_tick` (incluye esta señal + portadas + tap diferido +
    /// aviso breve); `sheet_animating` ya no se usa desde `lib` (2026-08-XX).
    #[allow(dead_code)]
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
    /// (`dy / alto del sheet`, recortado a [0, 1]) y se redibuja el frame.
    /// El redraw es BARATO con el sheet visible: `blit` usa el frame de
    /// página compuesto (`page_frame`, memcpy) + el overlay del sheet, NO
    /// re-blitea la página completa (ver `blit`) — la animación del sheet
    /// no degrada el frame time.
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
        self.page_frame = None;
    }

    /// Tick del bucle de eventos (timeout ~16 ms): avanza la animación del
    /// sheet, dispara el tap de página diferido por la ventana de doble-tap,
    /// expira el aviso breve (toast) y renderiza un lote de portadas
    /// pendientes de la biblioteca. `lib::android_main` lo invoca en los
    /// eventos Wake/Timeout, que solo ocurren mientras `needs_tick()` (sin
    /// despertar el loop en reposo).
    pub(crate) fn tick(&mut self, app: &AndroidApp) {
        // Tap diferido (ventana de doble-tap): si expiró sin un segundo down,
        // se ejecuta el tap de página (`input::tick_gestures`).
        crate::input::tick_gestures(self, app);
        // Resultado del hilo de Groq (si hay una consulta en vuelo): `try_recv`
        // sondea el canal SIN bloquear; al llegar el mensaje se actualiza el
        // panel (fase Answer/Error) y se libera el receptor. Mientras tanto el
        // poll con timeout se mantiene vivo vía `needs_tick` (ai_rx.is_some).
        if self.ai_rx.is_some() {
            let outcome = {
                let rx = self.ai_rx.as_ref().unwrap();
                match rx.try_recv() {
                    Ok(Ok(answer)) => Some((answer, AiPhase::Answer)),
                    Ok(Err(e)) => Some((format!("Error: {e}"), AiPhase::Error)),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    // El hilo murió sin enviar (defensa): mostrar error en vez
                    // de quedarse en "preguntando…" para siempre.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                        "Error: sin respuesta del servidor".to_string(),
                        AiPhase::Error,
                    )),
                }
            };
            if let Some((text, phase)) = outcome {
                self.ai_answer(text, phase);
            }
        }
        // Aviso breve: expira a los TOAST_MS (libera también su bitmap).
        if let Some((_, at)) = &self.toast
            && at.elapsed() >= TOAST_MS
        {
            self.toast = None;
            self.toast_bitmap = None;
            self.redraw();
        }
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
                self.page_frame = None; // liberar también el frame compuesto
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
    /// completa) y hace un redraw de solo blit — `blit` escala el bitmap
    /// cacheado de la página ACTUAL (la única hoja) con el zoom RELATIVO
    /// `zoom / rendered_zoom` (vecino-más-cercano), SIN re-renderizar MuPDF.
    /// El re-render nítido a la resolución final se hace UNA vez al soltar el
    /// pinch (`set_zoom_sharp`). El redraw normal (render + blit) no se usa
    /// aquí porque `ensure_pages_rendered` re-renderizaría en cada Move.
    ///
    /// El zoom es un factor RELATIVO a la distancia inicial del gesto
    /// (`zoom = z0 × dist / start_dist`, calculado por `input`); aquí solo se
    /// aplica. Además recalcula el pan de ANCLAJE: el punto de documento que
    /// estaba bajo el centro del pinch al iniciar (`begin_pinch`) se mantiene
    /// fijo en pantalla (ver `anchor_pan`) y se CLAMPEA a los bordes de la
    /// hoja (`clamp_pan`): un borde de la página nunca entra dentro de la
    /// ventana. Al hacer zoom-in solo se ve una porción de ESA página,
    /// recortada a sus bordes (nunca otra hoja: el blit solo dibuja la página
    /// actual).
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
                // actual queda en el borde superior del viewport (modo UNA
                // HOJA, sin scroll) → base 0. Ambos se clampean después a los
                // bordes de la hoja (ver `clamp_pan`).
                self.pan_x = Self::clamp_pan(
                    Self::anchor_pan(
                        p.ax,
                        Self::centered_base(self.win_w, dw, p.z0),
                        Self::centered_base(self.win_w, dw, zoom),
                        p.z0,
                        p.pan_x0,
                        zoom,
                    ),
                    dw * zoom,
                    self.win_w as f32,
                    false,
                );
                self.pan_y = Self::clamp_pan(
                    Self::anchor_pan(p.ay, 0.0, 0.0, p.z0, p.pan_y0, zoom),
                    dh * zoom,
                    self.win_h as f32,
                    true,
                );
            }
        }
        self.zoom = zoom;
        self.page_frame = None; // el frame compuesto del sheet tiene el zoom viejo
        // Redraw de solo blit: reutiliza los bitmaps de la caché (escala de la
        // última renderización, `rendered_zoom`); `blit` escala la página
        // actual con el zoom nuevo. El render y el reescalado de ventana los
        // cubre el bucle de eventos (RedrawNeeded/WindowResized) si hicieran
        // falta.
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Zoom FINAL del pinch (sharp): setea el factor (1.0 = página completa),
    /// conserva el pan de anclaje calculado por el último `set_zoom_fast` (el
    /// punto bajo los dedos no salta al re-renderizar) y re-renderiza la
    /// página actual UNA única vez a la escala continua resultante (render
    /// directo a resolución de pantalla — el camino medido más rápido en la
    /// tablet, ver nota de rendimiento en la cabecera de `lib.rs`): la caché
    /// se limpia y el redraw renderiza vía `ensure_pages_rendered`. Persiste
    /// el zoom (solo aquí, al soltar el gesto: `set_zoom_fast` es transitorio
    /// y escribir en cada Move de 60-120 Hz llenaría el disco).
    pub(crate) fn set_zoom_sharp(&mut self, zoom: f32) {
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        // El pan de anclaje YA es el del zoom final (último `set_zoom_fast`);
        // el re-render a la nueva escala (`rendered_zoom = zoom`) mantiene el
        // mismo mapeo documento→pantalla (la escala efectiva `doc·zoom` no
        // cambia), así que el punto bajo los dedos permanece fijo al soltar.
        self.zoom = zoom;
        self.rendered_zoom = zoom;
        // Reclamp del pan al zoom FINAL (por si `set_zoom_sharp` llega sin un
        // `set_zoom_fast` previo, p. ej. pinch sin Moves): `clamp_pan` solo
        // depende de `page = doc·zoom`, así que un pan ya clampeado no cambia
        // y el rango cubre la ventana entera también tras el re-render
        // (`rendered_zoom = zoom` → la escala efectiva `doc·zoom` no cambia).
        let (dw, dh) = self.page_doc_size_px(self.page);
        if dw > 0.0 && dh > 0.0 {
            self.pan_x = Self::clamp_pan(self.pan_x, dw * zoom, self.win_w as f32, false);
            self.pan_y = Self::clamp_pan(self.pan_y, dh * zoom, self.win_h as f32, true);
        }
        self.cache.clear();
        self.page_frame = None; // el frame compuesto del sheet tiene el zoom viejo
        info!("zoom {:.3}", self.zoom);
        self.redraw();
        self.save_state();
    }

    /// Alterna el modo oscuro SIN re-renderizar MuPDF y SIN tocar la caché:
    /// esta guarda SIEMPRE bitmaps normales (de colores) y la inversión
    /// (255 − v, la transformación de `pdf_core::dark::invert_bitmap`) se
    /// aplica en el blit (`draw::blit_page`), solo cuando el modo oscuro está
    /// activo. El fondo letterbox pasa a negro puro (`DARK_BG`). La
    /// preferencia se persiste junto a la posición.
    pub(crate) fn toggle_dark(&mut self) {
        self.dark = !self.dark;
        // La caché guarda SIEMPRE bitmaps normales: la inversión (255 − v) se
        // aplica en el blit (`draw::blit_page`), solo con el modo oscuro
        // activo — no se re-renderiza ni se invierte nada aquí.
        self.page_badge = None; // colores del indicador cambian
        self.sheet_bitmap = None; // colores del sheet cambian (Dark/Light)
        self.page_frame = None; // el frame compuesto cambia de modo
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
    /// Caller actual: `highlight_sel` (subrayar la selección). El camino de
    /// guardado es el mismo que usaba el modo dibujo eliminado; el modelo de
    /// anotaciones sigue siendo persistible y exportable.
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
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                self.pinch = None;
                self.bitmap = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.mode = UiMode::Viewer;
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera (libera también el frame)
                self.clear_selection(); // selección del documento anterior: fuera
                self.close_ai_panel(); // panel de IA del documento anterior: fuera
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
        self.clear_selection(); // selección del visor: fuera (no pinta en biblioteca)
        self.close_ai_panel(); // panel de IA del visor: fuera
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
        self.clear_selection(); // selección del visor: fuera (no pinta en el picker)
        self.close_ai_panel(); // panel de IA del visor: fuera
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
