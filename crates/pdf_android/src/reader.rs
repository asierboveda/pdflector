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
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use android_activity::ndk::hardware_buffer_format::HardwareBufferFormat;
use android_activity::ndk::native_window::NativeWindow;
use base64::Engine;
use log::{error, info, warn};
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::store::{AnnotationStore, sidecar_path};
use pdf_core::{
    Annotation, AnnotationSet, Bitmap, Color, Document, Gesture, Highlight, PageTextCache, Rect,
    RenderEngine, Stroke, TextSpan,
};

use crate::annotations::{ERASE_HIT_RADIUS_PT, ERASE_HL_PAD_PT, PenMode};
use crate::annotations::{ToolGesture, ToolKind};
use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES, PageCache};
use crate::draw::{
    ButtonRect, ai_panel_layout, blit_library, compose_library_snapshot, paste_lib_thumbs,
    render_ai_panel, render_eraser_cursor, render_library_header, render_library_zone,
    render_mode_badge, render_page_badge, render_picker_list, render_search_chip_row,
    render_sel_menu, render_sheet, render_toast, render_viewer_bottom_chrome,
    render_viewer_top_chrome, sel_menu_layout, splice_row,
};
use crate::gpu::Gpu;
use crate::input::GestureState;
use crate::jni::{
    android_sdk_int, launch_intent_pdf, query_media_store, read_content_uri_bytes,
    sanitize_pdf_name,
};
use crate::persist::{self, BookProgress, RecentEntry};
use crate::theme;
use crate::thumbs::{THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES, ThumbCache};
use crate::view::initial_scale;
use crate::zoom::blit_fast;
use crate::{LIB_FADE_MS, PINCH_MAX, PINCH_MIN, SEL_MIN_PX, TOAST_MS};

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
    /// Picker: lista de PDFs de los directorios de la app (fallback interno)
    /// o selector de "＋ Añadir" (ver `PickerKind`).
    Picker,
    /// Biblioteca CURADA: solo los libros registrados en
    /// `internal/library.json` (`persist::load_progress`); SIN escaneo de
    /// MediaStore. Altas vía el selector de `add_book`.
    Library,
}

/// Qué lista muestra el modo `UiMode::Picker`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerKind {
    /// Fallback interno histórico (PDFs de los directorios de la app);
    /// lista `pdf_list`, tap = abrir.
    Files,
    /// Selector de "＋ Añadir": TODOS los PDFs del sistema vía MediaStore;
    /// lista TEMPORAL `select_list` (nunca `library_list`: la rejilla curada
    /// no cambia hasta confirmar la selección). Tap = copiar a
    /// `internal/pdfs/` + registrar en `library.json` (`add_selected`).
    Select,
}

/// Una fila del selector de añadir (`PickerKind::Select`): una CARPETA del
/// gestor de archivos (para entrar) o un PDF (índice en `select_list`, para
/// curar).
#[derive(Clone, Debug)]
pub(crate) enum PickRow {
    /// Carpeta (nombre visible del nivel actual; al tocarla se entra).
    Folder(String),
    /// PDF: índice en `select_list` (se copia a `internal/pdfs/`).
    File(usize),
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
/// error (para el mensaje de estado). Construido en `jni::query_media_store`,
/// consumido en `Reader::add_book`/`rescan_select` (selector de añadir).
pub(crate) struct LibraryScan {
    pub(crate) entries: Vec<LibraryEntry>,
    /// ¿Concedido el acceso a todos los archivos (API 30+) o no requerido (≤ 12)?
    pub(crate) permission_granted: bool,
    /// Error de consulta mostrable (None si OK).
    pub(crate) error: Option<String>,
}

/// Estado de lectura de un libro, DERIVADO del registro de progreso
/// (`persist::BookProgress`): Unread (nunca abierto: sin registro), Reading
/// (abierto, no terminado) o Finished (última página alcanzada). Es el
/// filtro de estado de "My Library" y el que decide qué entra en
/// "Continue Reading".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BookStatus {
    Unread,
    Reading,
    Finished,
}

/// Orden de "My Library" (sort, chips discretos de organización y menú View).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum LibSort {
    /// `added_unix` del registro de progreso (más reciente primero; los
    /// nunca abiertos al final).
    #[default]
    RecentlyAdded,
    /// `last_read_unix` (más reciente primero; los nunca abiertos al final).
    RecentlyRead,
    /// Título (nombre de fichero sin extensión), case-insensitive.
    Title,
    /// Autor (primer segmento de RELATIVE_PATH), luego título.
    Author,
    /// Porcentaje de progreso leído (pct(), mayor progreso primero).
    Progress,
}

/// Layout de la biblioteca (menú View "⋯", Tarea 2 implementa el contenido):
/// rejilla de portadas o lista de filas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum LibraryViewMode {
    /// Rejilla (por defecto).
    #[default]
    Grid,
    /// Lista (filas compactas).
    List,
}

/// Ajuste de las portadas dentro de sus marcos (menú View "⋯", Tarea 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum LibraryCoverFit {
    /// Recorte central (fill, por defecto).
    #[default]
    Crop,
    /// Portada completa visible (contain).
    Fit,
}

/// Agrupación de la rejilla (menú View "⋯", Tarea 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum LibraryGroupBy {
    /// Sin agrupar (por defecto).
    #[default]
    None,
    /// Agrupar por autor.
    Author,
}

/// Un libro del carousel destacado "Continue Reading": un reciente abierto
/// no terminado, con su progreso persistido (página, total, %). Construido
/// en `Reader::lib_continue_reading` a partir de `recents.json` +
/// `library.json`; lo consumen el render (`draw`), el tap (`input`) y el
/// pump de portadas (`Reader::pump_thumbs`). Desde la biblioteca minimalista
/// (2026-08-25, rejilla + buscador sin sección Continue Reading), solo el
/// pump lee `path`/`name`; el resto de campos y el draw se conservan por si
/// se reintroduce la sección.
#[allow(dead_code)] // sección "Continue Reading" oculta por diseño
pub(crate) struct ContinueBook {
    /// Ruta local absoluta (clave del documento; abre con `open_pdf_at`).
    pub(crate) path: String,
    /// Nombre de fichero (se muestra bajo la portada de la tarjeta).
    pub(crate) name: String,
    /// Autor derivado (primer segmento de carpeta de MediaStore o "PDF").
    pub(crate) author: String,
    /// Página guardada, 0-based (donde se reanuda).
    pub(crate) page: u32,
    /// Total de páginas del documento.
    pub(crate) page_count: u32,
    /// Porcentaje leído (0.0-1.0) para la barra de progreso.
    pub(crate) pct: f32,
}

/// Título de un libro a partir del NOMBRE de fichero (sin extensión).
pub(crate) fn title_from_name(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name.to_string())
}

/// Título de un libro de la biblioteca: nombre del fichero sin extensión.
pub(crate) fn entry_title(e: &LibraryEntry) -> String {
    title_from_name(&e.name)
}

/// Autor de un libro de la biblioteca: primer segmento de RELATIVE_PATH (la
/// carpeta, p. ej. "Download/" → "Download") o "PDF" si no hay carpeta.
/// Deriva el "autor" de una biblioteca personal de PDFs sin metadatos
/// (MuPDF no expone metadatos por página de forma barata): la carpeta del
/// sistema (Descargas/Documentos/…) como colección, no como ruta (nada de
/// rutas completas visibles — AGENTS.md/estética premium).
pub(crate) fn entry_author(e: &LibraryEntry) -> String {
    e.folder
        .split('/')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "PDF".to_string())
}

/// Estado de lectura de un libro con su registro de progreso (None = nunca
/// abierto → Unread).
pub(crate) fn book_status(p: Option<&BookProgress>) -> BookStatus {
    match p {
        None => BookStatus::Unread,
        Some(p) if p.is_finished() => BookStatus::Finished,
        Some(_) => BookStatus::Reading,
    }
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

/// Alto (px) del área de la barra superior flotante de chrome del visor.
pub(crate) fn viewer_top_chrome_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.02).clamp(28.0, 48.0) + 68.0 + 12.0
}

/// Alto (px) del área de la barra inferior flotante de chrome del visor.
pub(crate) fn viewer_bottom_chrome_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.025).clamp(28.0, 48.0) + 76.0 + 16.0
}

/// Alto (px) del sheet de ajustes (B2: ajustado al contenido real ~42% de win_h).
pub(crate) fn sheet_h(win_h: i32) -> i32 {
    (win_h as f32 * 0.42).clamp(650.0, 950.0).round() as i32
}

/// Pad horizontal del sheet (px).
pub(crate) fn sheet_pad(win_w: i32) -> f32 {
    (win_w as f32 * 0.04).clamp(24.0, 56.0)
}

/// Alto (px) de los botones del sheet (S3: alto >= 48 px).
pub(crate) fn sheet_btn_h(_win_h: i32) -> f32 {
    50.0
}

/// Y del borde superior de la fila de temas del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_theme_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.18).clamp(90.0, 160.0)
}

/// Y del borde superior de la fila de navegación del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_nav_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.48).clamp(280.0, 430.0)
}

/// Y del borde superior de la fila de acciones del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_act_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.78).clamp(470.0, 700.0)
}

/// Ancho (px) de cada botón de 3 por fila del sheet.
pub(crate) fn sheet_btn_w(win_w: i32) -> f32 {
    (win_w as f32 - 4.0 * sheet_pad(win_w)) / 3.0
}

/// Ancho (px) de cada botón de 4 por fila (temas) del sheet.
pub(crate) fn sheet_theme_btn_w(win_w: i32) -> f32 {
    (win_w as f32 - 5.0 * sheet_pad(win_w)) / 4.0
}

/// --- Rejilla 3×3 de la biblioteca (geometría compartida por render y tap) ---
/// Columnas de la rejilla de la biblioteca.
pub(crate) const GRID_COLS: usize = 3;

/// Pad exterior horizontal de la rejilla (px): margen con respiro estilo Apple Books.
pub(crate) fn grid_pad(win_w: i32) -> f32 {
    (win_w as f32 * 0.04).clamp(24.0, 60.0)
}

/// Separación entre celdas de la rejilla (px): respiro >= 3% de win_w.
pub(crate) fn grid_gap(win_w: i32) -> f32 {
    (win_w as f32 * 0.035).clamp(24.0, 56.0)
}

/// Inset de la portada dentro de la celda (px).
pub(crate) const GRID_CELL_PAD: f32 = 10.0;

/// Ancho (px) de una celda de la rejilla según el número de columnas.
pub(crate) fn grid_cell_w(win_w: i32, cols: usize) -> f32 {
    let w = win_w as f32;
    let c = cols.max(1) as f32;
    (w - 2.0 * grid_pad(win_w) - (c - 1.0) * grid_gap(win_w)) / c
}

/// Multiplicador de escala según `cover_size`: 0 -> 0.85 (Pequeño), 1 -> 1.0 (Mediano), 2 -> 1.15 (Grande).
pub(crate) fn cover_size_multiplier(cover_size: u8) -> f32 {
    match cover_size {
        0 => 0.85,
        2 => 1.15,
        _ => 1.0,
    }
}

/// Ancho (px) del área de portada dentro de la celda.
pub(crate) fn grid_cover_w(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    (grid_cell_w(win_w, cols) - 2.0 * GRID_CELL_PAD) * cover_size_multiplier(cover_size)
}

/// Alto (px) del área de portada: proporción 2:3 (alto = ancho × 1.5), estilo
/// Apple Books, para TODAS las celdas (rejilla uniforme).
pub(crate) fn grid_cover_h(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    grid_cover_w(win_w, cols, cover_size) * 1.5
}

/// Alto (px) de la zona de texto de la celda: título (14sp) + autor (12sp) + barra progreso + padding.
pub(crate) fn grid_title_h(_win_w: i32) -> f32 {
    72.0
}

/// Alto (px) de una celda de la rejilla.
pub(crate) fn grid_cell_h(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    grid_cover_h(win_w, cols, cover_size) + grid_title_h(win_w)
}

/// Alto (px) de una fila de la biblioteca en modo Lista.
pub(crate) fn list_row_h(_win_h: i32, cover_size: u8) -> f32 {
    (116.0 * cover_size_multiplier(cover_size)).max(96.0)
}

/// Separación vertical (px) entre filas en modo Lista.
pub(crate) fn list_row_gap() -> f32 {
    12.0
}

/// Rectángulo de una fila de la biblioteca en modo Lista.
pub(crate) fn list_row_rect(
    win_w: i32,
    rows_y0: i32,
    idx: usize,
    win_h: i32,
    cover_size: u8,
) -> (f32, f32, f32, f32) {
    let pad = grid_pad(win_w);
    let x = pad;
    let w = win_w as f32 - 2.0 * pad;
    let h = list_row_h(win_h, cover_size);
    let gap = list_row_gap();
    let y = rows_y0 as f32 + idx as f32 * (h + gap);
    (x, y, x + w, y + h)
}

// --- Biblioteca rediseñada: biblioteca PERSONAL premium (2026-08-XX) ---
//
// La biblioteca ya NO es un file manager: es una biblioteca personal de
// libros (estilo Apple Books/Kindle pero propio). Las PORTADAS mandan;
// el header es editorial (título grande + "＋ Add book" + campo de
// búsqueda); "Continue Reading" (carousel horizontal de tarjetas con
// portada grande, título, autor, barra de progreso, "Page X of Y" y acción
// "Read") es el punto de entrada; y "My Library" es la rejilla principal
// de portadas con título/autor/progreso y sus chips discretos de
// organización (sort/filter). Toda la geometría de abajo es COMPARTIDA
// por el render (`draw::render_library_zone` + `render_library_header`), el tap y el arrastre
// (`input`) y el pump de portadas (`Reader::pump_thumbs`).

/// Alto (px) de la CABECERA de la biblioteca: título "Library" grande y
/// negrita + botón "＋ Add book" a la derecha.
pub(crate) fn lib_header_h(win_h: i32) -> f32 {
    (win_h as f32 / 16.0).clamp(115.0, 135.0)
}

/// Centro Y (px) de la FILA DE BOTONES de la cabecera editorial ("＋ Añadir"
/// y los círculos de menú "⋯"/"☰"): la mitad del espacio libre bajo el
/// margen superior de 36 px. Compartido por el render y el tap para que el
/// hit-test y el dibujo coincidan exactamente.
pub(crate) fn lib_header_buttons_cy(win_h: i32) -> f32 {
    let header_h = lib_header_h(win_h);
    let top_pad = 36.0f32;
    top_pad + (header_h - top_pad) / 2.0
}

/// Diámetro (px) del círculo de los botones de menú de la cabecera: un
/// touch target de ~32 dp (≈ 65 px con densidad 2.0 de la TCL) — el tamaño
/// mínimo cómodo para un dedo.
pub(crate) fn header_menu_btn_d(win_w: i32) -> f32 {
    (win_w as f32 / 22.0).clamp(56.0, 72.0)
}

/// Separación (px) entre el círculo de menú y su vecino ("8 dp gap").
pub(crate) fn header_menu_gap() -> f32 {
    16.0
}

/// Ancho (px) del botón "＋ Añadir" de la cabecera (compartido por el
/// render y la geometría de los menús ⋯/☰, que se alinean a su izquierda).
pub(crate) fn lib_add_btn_w(win_w: i32) -> f32 {
    (win_w as f32 * 0.18).clamp(110.0, 160.0)
}

/// Rectángulo (left, top, right, bottom) del botón de menú SETTINGS "☰":
/// círculo de `header_menu_btn_d` alineado a la IZQUIERDA del "＋ Añadir"
/// (que mantiene su anclaje a derecha con `grid_pad`) con `header_menu_gap`
/// de separación, centrado en el Y de la fila de botones. Los futuros
/// dropdowns cuelgan de su BORDE DERECHO (`dropdown-end`) para no cortarse
/// por la izquierda.
pub(crate) fn settings_menu_button_rect(win_w: i32, win_h: i32) -> (f32, f32, f32, f32) {
    let d = header_menu_btn_d(win_w);
    let pad = grid_pad(win_w);
    let add_x = win_w as f32 - pad - lib_add_btn_w(win_w); // borde izq del ＋
    let cy = lib_header_buttons_cy(win_h);
    (
        add_x - header_menu_gap() - d,
        cy - d / 2.0,
        add_x - header_menu_gap(),
        cy + d / 2.0,
    )
}

/// Rectángulo del botón de menú VIEW "⋯": a la IZQUIERDA del de settings,
/// con `header_menu_gap` de separación.
pub(crate) fn view_menu_button_rect(win_w: i32, win_h: i32) -> (f32, f32, f32, f32) {
    let (l, t, _, b) = settings_menu_button_rect(win_w, win_h);
    let d = header_menu_btn_d(win_w);
    (l - d - header_menu_gap(), t, l - header_menu_gap(), b)
}

/// Alto (px) del campo de búsqueda (fila fija bajo la cabecera).
pub(crate) fn lib_search_h() -> f32 {
    48.0
}

/// Alto (px) del panel de búsqueda desplegado (2 filas de chips: letras y
/// carpetas); 0 si el campo de búsqueda está cerrado.
pub(crate) fn lib_search_panel_h(win_h: i32, open: bool) -> f32 {
    if open {
        lib_chip_h(win_h) * 2.0 + 16.0
    } else {
        0.0
    }
}

/// Alto (px) de un chip del panel de búsqueda (letras/carpetas, >= 40 px).
pub(crate) fn lib_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 50.0).clamp(40.0, 46.0)
}

/// Y (px) del borde superior de la fila 0 (letras) del panel de búsqueda.
pub(crate) fn lib_search_chips_y0(reader: &Reader) -> f32 {
    lib_header_h(reader.win_h) + lib_search_h() + 6.0
}

/// Y (px) del borde superior de la fila 1 (carpetas) del panel de búsqueda.
pub(crate) fn lib_search_chips_y1(reader: &Reader) -> f32 {
    lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) + 8.0
}

/// Y (px) del borde superior del contenido scrolleable (cabecera + campo de
/// búsqueda + panel de chips si está abierto + franja de estado si la hay).
pub(crate) fn lib_content_y0(win_h: i32, search_open: bool, has_status: bool) -> i32 {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    (lib_header_h(win_h) + lib_search_h() + lib_search_panel_h(win_h, search_open)) as i32
        + status_h
}

/// Alto (px) de un título de sección ("CONTINUE READING"/"My Library").
pub(crate) fn lib_section_title_h(win_h: i32) -> f32 {
    (win_h as f32 / 64.0).clamp(24.0, 32.0)
}

/// Ancho (px) de la portada de una tarjeta de "Continue Reading" (2:3).
#[allow(dead_code)] // sección "Continue Reading" oculta por diseño (2026-08-25)
pub(crate) fn lib_cont_cover_w(win_h: i32) -> f32 {
    lib_cont_cover_h(win_h) / 1.5
}

/// Alto (px) de la portada de una tarjeta (proporción 2:3).
#[allow(dead_code)] // sección "Continue Reading" oculta por diseño (2026-08-25)
pub(crate) fn lib_cont_cover_h(win_h: i32) -> f32 {
    lib_cont_card_h(win_h) - 32.0
}

/// Alto (px) de la tarjeta horizontal (~15% de win_h).
pub(crate) fn lib_cont_card_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.15).clamp(240.0, 330.0)
}

/// Ancho (px) de la tarjeta horizontal.
pub(crate) fn lib_cont_card_w(win_w: i32, _win_h: i32) -> f32 {
    (win_w as f32 * 0.52).clamp(440.0, 640.0)
}

/// Separación horizontal entre tarjetas del carousel (px).
pub(crate) fn lib_cont_gap() -> f32 {
    18.0
}

/// X (px) en coords de CONTENIDO de la tarjeta `i` del carousel (sin el
/// scroll horizontal aplicado).
pub(crate) fn lib_cont_card_x(win_w: i32, win_h: i32, i: usize) -> f32 {
    grid_pad(win_w) + i as f32 * (lib_cont_card_w(win_w, win_h) + lib_cont_gap())
}

/// Alto (px) del bloque de "Continue Reading" (título de sección + fila de
/// tarjetas) en coords de contenido; 0 si no hay libros en curso.
pub(crate) fn lib_cont_block_h(_win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    if !has_cont {
        0.0
    } else {
        lib_section_title_h(win_h) + lib_cont_card_h(win_h) + 16.0
    }
}

/// --- Organización de "My Library" (sort + filter, chips discretos) ---
/// Alto (px) de un chip de organización (>= 40 px).
pub(crate) fn lib_org_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 50.0).clamp(40.0, 46.0)
}

/// Separación entre las filas de chips de sort y filter (px).
pub(crate) fn lib_org_gap() -> f32 {
    10.0
}

/// Alto (px) del bloque de organización (2 filas: sort + filter).
pub(crate) fn lib_org_block_h(win_h: i32) -> f32 {
    lib_org_chip_h(win_h) * 2.0 + lib_org_gap() + 6.0
}

/// Ancho (px) reservado para la etiqueta discreta de cada fila ("SORT" /
/// "FILTER"), antes de los chips.
pub(crate) fn lib_org_label_w() -> f32 {
    54.0
}

/// Y (px) del borde superior de la fila de organización `row` (0 = sort,
/// 1 = filter) en coords de CONTENIDO (bajo el título de "My Library").
pub(crate) fn lib_org_y(win_w: i32, win_h: i32, has_cont: bool, row: usize) -> f32 {
    lib_grid_y0(win_w, win_h, has_cont) - lib_org_block_h(win_h)
        + row as f32 * (lib_org_chip_h(win_h) + lib_org_gap())
}

/// Y (px) del borde superior de la REJILLA o LISTA en coords de CONTENIDO.
/// Si `has_cont` es true (estantería de recientes activa con libros),
/// deja espacio para el carousel Continue Reading.
pub(crate) fn lib_grid_y0(win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    if has_cont {
        lib_cont_block_h(win_w, win_h, true) + 16.0
    } else {
        8.0
    }
}

/// Ancho (px) de un chip del panel de búsqueda según el nº de caracteres de
/// su etiqueta (los de carpetas llevan la ruta, p. ej. "Download/").
pub(crate) fn lib_chip_w(win_w: i32, chars: usize) -> f32 {
    (10.0 + chars as f32 * 7.0).clamp(40.0, (win_w / 3) as f32)
}

/// Ancho fijo (px) de los chips de letras (etiquetas de 1-3 caracteres).
pub(crate) fn lib_letter_chip_w(win_w: i32) -> f32 {
    (win_w as f32 / 26.0).clamp(40.0, 56.0)
}

/// Chips del panel de BÚSQUEDA de la biblioteca, fila `row` (0 = letras
/// A-Z/#, 1 = carpetas): etiqueta + rect en px de VENTANA (con el scroll
/// horizontal de la fila ya aplicado) + si el chip está ACTIVO. Geometría
/// COMPARTIDA por `draw::render_library_zone` + `render_library_header` e `input::library_tap`. Es la
/// búsqueda SIN teclado presentada como un campo de búsqueda: el teclado
/// del sistema no entrega texto al backend native-activity de
/// android-activity (ver la cabecera del módulo), así que el filtro es por
/// inicial (A-Z/#) y por carpeta, con [All] al frente de cada fila.
pub(crate) fn lib_chips(reader: &Reader, row: usize) -> Vec<(String, ButtonRect, bool)> {
    let win_w = reader.win_w;
    let gap = grid_gap(win_w);
    let x0 = grid_pad(win_w);
    let y = if row == 0 {
        lib_search_chips_y0(reader)
    } else {
        lib_search_chips_y1(reader)
    };
    let scroll = if row == 0 {
        reader.lib_letters_x
    } else {
        reader.lib_folders_x
    };
    let mut out = Vec::new();
    let mut x = x0;
    let mut push =
        |label: String, chars: usize, active: bool, out: &mut Vec<(String, ButtonRect, bool)>| {
            let w = if row == 0 {
                lib_letter_chip_w(win_w)
            } else {
                lib_chip_w(win_w, chars)
            };
            let l = x - scroll;
            let r = l + w;
            out.push((label, (l, y, r, y + lib_chip_h(reader.win_h)), active));
            x += w + gap;
        };
    if row == 0 {
        push("All".to_string(), 3, reader.lib_letter.is_none(), &mut out);
        for c in 'A'..='Z' {
            push(c.to_string(), 1, reader.lib_letter == Some(c), &mut out);
        }
        push("#".to_string(), 1, reader.lib_letter == Some('#'), &mut out);
    } else {
        push("All".to_string(), 3, reader.lib_folder.is_none(), &mut out);
        for f in reader.lib_folders() {
            let active = reader.lib_folder.as_deref() == Some(f.as_str());
            let n = f.chars().count();
            push(f, n, active, &mut out);
        }
    }
    out
}

/// Ancho total (px) de la fila de chips `row` (para el clamp del scroll
/// horizontal).
pub(crate) fn lib_chips_row_w(reader: &Reader, row: usize) -> f32 {
    let chips = lib_chips(reader, row);
    match chips.last() {
        Some((_, (_, _, r, _), _)) => *r + grid_pad(reader.win_w),
        None => grid_pad(reader.win_w),
    }
}

/// Chips de ORGANIZACIÓN de "My Library", fila `row` (0 = sort, 1 = filter):
/// etiqueta + rect en px de VENTANA (con el scroll vertical de la página y
/// el horizontal de la fila aplicados) + si el chip está ACTIVO. Geometría
/// COMPARTIDA por `draw::render_library_zone` + `render_library_header` e `input::library_tap`. Los
/// chips empiezan tras la etiqueta discreta de la fila ("SORT"/"FILTER",
/// `lib_org_label_w`, que dibuja el render y no es tappable).
pub(crate) fn lib_org_chips(reader: &Reader, row: usize) -> Vec<(String, ButtonRect, bool)> {
    let win_w = reader.win_w;
    let gap = 8.0;
    let x0 = grid_pad(win_w) + lib_org_label_w();
    let scroll = if row == 0 {
        reader.lib_sort_x
    } else {
        reader.lib_filter_x
    };
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let y0 =
        content_y0 - reader.lib_scroll + lib_org_y(win_w, reader.win_h, reader.lib_has_cont(), row);
    let chip_h = lib_org_chip_h(reader.win_h);
    let mut out = Vec::new();
    let mut x = x0;
    for (label, active) in lib_org_row(reader, row) {
        let w = (28.0 + label.chars().count() as f32 * 8.5).clamp(56.0, (win_w / 4) as f32);
        let l = x - scroll;
        out.push((label.to_string(), (l, y0, l + w, y0 + chip_h), active));
        x += w + gap;
    }
    out
}

/// Etiquetas + estado activo de la fila de organización `row` (0 = sort,
/// 1 = filter).
fn lib_org_row(reader: &Reader, row: usize) -> Vec<(&'static str, bool)> {
    if row == 0 {
        vec![
            ("Recientes", reader.lib_sort == LibSort::RecentlyAdded),
            ("Leídos", reader.lib_sort == LibSort::RecentlyRead),
            ("Título", reader.lib_sort == LibSort::Title),
            ("Autor", reader.lib_sort == LibSort::Author),
        ]
    } else {
        vec![
            ("Todos", reader.lib_status.is_none()),
            ("En lectura", reader.lib_status == Some(BookStatus::Reading)),
            (
                "Terminados",
                reader.lib_status == Some(BookStatus::Finished),
            ),
            ("Por leer", reader.lib_status == Some(BookStatus::Unread)),
        ]
    }
}

/// Ancho total (px) de la fila de organización `row` (para el clamp del
/// scroll horizontal).
fn lib_org_row_w(reader: &Reader, row: usize) -> f32 {
    match lib_org_chips(reader, row).last() {
        Some((_, (_, _, r, _), _)) => *r + grid_pad(reader.win_w),
        None => grid_pad(reader.win_w),
    }
}

/// Geometría del EMPTY STATE de la biblioteca (sin PDFs): ilustración de un
/// libro + título + subtítulo + botón ("Add PDF" o "Grant access"). La
/// comparten el render (`draw::render_library_zone` + `render_library_header`) y el tap
/// (`input::library_tap`). `None` si la biblioteca tiene libros (no aplica).
pub(crate) struct EmptyStateGeom {
    /// Rect de la ilustración (portada del libro) en px de VENTANA.
    pub(crate) book: (f32, f32, f32, f32),
    /// Baseline (px de ventana) del título "Your library is empty".
    pub(crate) title_y: f32,
    /// Baseline (px de ventana) del subtítulo.
    pub(crate) subtitle_y: f32,
    /// Rect (px de ventana) del botón.
    pub(crate) button: (f32, f32, f32, f32),
}

pub(crate) fn lib_empty_state_geom(reader: &Reader) -> Option<EmptyStateGeom> {
    if !reader.library_list.is_empty() {
        return None;
    }
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let ctop = content_y0 - reader.lib_scroll;
    let h = reader.win_h as f32;
    let cy = ctop + (h - ctop) * 0.40;
    let (bw, bh) = (96.0f32, 128.0f32);
    let bx = (reader.win_w as f32 - bw) / 2.0;
    let by = cy;
    let title_y = by + bh + 34.0;
    let subtitle_y = title_y + 26.0;
    let (bw2, bh2) = (172.0f32, 44.0f32);
    let bx2 = (reader.win_w as f32 - bw2) / 2.0;
    let by2 = subtitle_y + 20.0;
    Some(EmptyStateGeom {
        book: (bx, by, bx + bw, by + bh),
        title_y,
        subtitle_y,
        button: (bx2, by2, bx2 + bw2, by2 + bh2),
    })
}

/// Nº de filas de celdas visibles en la biblioteca (cabecera + franja de
/// estado restan de la ventana; mínimo 1 fila para que siempre haya algo).
#[allow(dead_code)] // geometría pre-rediseño; la biblioteca usa `lib_visible_grid_rows` (px)
pub(crate) fn grid_visible_rows(win_w: i32, win_h: i32, has_status: bool) -> usize {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    let usable = (win_h - picker_header_h(win_h) - status_h) as f32;
    (usable / grid_cell_h(win_w, GRID_COLS, 1)).floor().max(1.0) as usize
}

/// Y del borde superior de la zona de rejilla (cabecera + franja de estado).
#[allow(dead_code)] // geometría pre-rediseño; la biblioteca usa `lib_content_y0` (px)
pub(crate) fn grid_rows_y0(win_h: i32, has_status: bool) -> i32 {
    picker_header_h(win_h) + if has_status { picker_row_h(win_h) } else { 0 }
}

/// Rectángulo (left, top, right, bottom) en px de ventana de la celda
/// `(row, col)` de la rejilla (compartido por `draw::render_library_zone` + `render_library_header` e
/// `input::library_tap`).
pub(crate) fn grid_cell_rect(
    win_w: i32,
    rows_y0: i32,
    row: usize,
    col: usize,
    cols: usize,
    cover_size: u8,
) -> (f32, f32, f32, f32) {
    let x = grid_pad(win_w) + col as f32 * (grid_cell_w(win_w, cols) + grid_gap(win_w));
    let y = rows_y0 as f32 + row as f32 * grid_cell_h(win_w, cols, cover_size);
    (
        x,
        y,
        x + grid_cell_w(win_w, cols),
        y + grid_cell_h(win_w, cols, cover_size),
    )
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

/// Selección de texto en curso (rectángulo de arrastre del long-press): ancla
/// (punto del long-press) y punto actual del dedo, ambos en px de VENTANA
/// (pantalla).
///
/// Decisión documentada: la selección se guarda en coords de PANTALLA (no de
/// página) porque el gesto, el render del rect y el menú viven en pantalla y
/// la conversión a página solo se hace UNA vez cuando se necesita
/// (`sel_page_rect`, con `screen_to_page` — la INVERSA exacta del mapeo del
/// blit, misma `scale = cover × zoom` y `dx/dy` que la capa de anotaciones).
#[derive(Clone, Copy, Debug)]
pub(crate) struct SelState {
    /// Punto del long-press (px de ventana): esquina fija del rect.
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

/// Estado del arrastre de las listas (picker interno y biblioteca) en el
/// Down: punto de partida + scrolls de partida + zona de la biblioteca.
/// El picker solo scrollea en vertical por filas; la biblioteca scrollea en
/// vertical por píxeles y, según la zona donde cayó el dedo, en horizontal
/// (carousel de recientes o filas de chips).
pub(crate) struct ListDrag {
    /// X del Down (px de ventana).
    pub(crate) sx: f32,
    /// Y del Down (px de ventana).
    pub(crate) sy: f32,
    /// Scroll vertical de partida: fila (`list_scroll` como f32) en el
    /// picker, píxeles (`lib_scroll`) en la biblioteca.
    pub(crate) v0: f32,
    /// Scroll horizontal de partida (px): carousel o fila de chips en la
    /// biblioteca; 0 en el picker.
    pub(crate) h0: f32,
    /// Zona de la biblioteca donde cayó el Down: 0 = contenido (scroll
    /// vertical), 1 = carousel de recientes (scroll horizontal), 2 = chips de
    /// carpetas, 3 = chips de letras. 0 en el picker.
    pub(crate) zone: u8,
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
    pub(crate) cache: PageCache,
    /// Zoom con el que están renderizados los bitmaps de la caché (1.0 =
    /// escala *cover* base; el re-render nítido al soltar el pinch pone
    /// `rendered_zoom = self.zoom`). El blit usa el zoom RELATIVO
    /// `zoom / rendered_zoom`: 1:1 nítido para bitmaps recién renderizados,
    /// escala vecino-más-cercano del bitmap viejo durante el pinch.
    pub(crate) rendered_zoom: f32,
    /// Factor de zoom continuo (1.0 = página completa *cover*).
    pub(crate) zoom: f32,
    /// Desplazamiento de anclaje del pinch (px, f32): el punto de pantalla
    /// bajo el CENTRO del pinch permanece fijo mientras se hace zoom
    /// (`begin_pinch` fija el ancla; `set_zoom_fast` recalcula `pan_x/pan_y`
    /// con la fórmula de anclaje, ver `anchor_pan`). Se suma al centrado
    /// base del blit (`dx/dy`); persiste entre gestos y páginas (el zoom
    /// también): pasar de página conserva la misma región de lectura.
    /// 0 = sin desplazamiento.
    pub(crate) pan_x: f32,
    pub(crate) pan_y: f32,
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
    /// Dimensiones actuales de la ventana (px).
    pub(crate) win_w: i32,
    pub(crate) win_h: i32,
    /// Máquina de gestos (tap/pinch).
    pub(crate) gesture: GestureState,
    /// Modo de UI actual (visor de página o picker de PDFs).
    pub(crate) mode: UiMode,
    /// PDFs encontrados en los directorios de la app (picker fallback).
    pub(crate) pdf_list: Vec<PdfEntry>,
    /// Qué variante del picker está activa (fallback o selector de añadir).
    pub(crate) picker_kind: PickerKind,
    /// Lista TEMPORAL del selector de "＋ Añadir" (todos los PDFs de
    /// MediaStore). NUNCA es `library_list`: la biblioteca curada solo
    /// cambia al confirmar una selección (`Reader::add_selected`).
    pub(crate) select_list: Vec<LibraryEntry>,
    /// Ruta de CARPETAS abierta en el gestor de archivos del selector de
    /// añadir (segmentos de RELATIVE_PATH; vacío = raíz). Al entrar en una
    /// carpeta solo se ven sus PDFs y subcarpetas (`picker_rows`), evitando
    /// la lista plana inabarcable de MediaStore.
    pub(crate) sel_dir: Vec<String>,
    /// Biblioteca CURADA mostrada en la rejilla: una entrada por registro de
    /// `internal/library.json` cuyo PDF sigue en disco (uri = RUTA LOCAL,
    /// folder = "PDF"). La construye `reload_curated_library`; jamás la
    /// escribe un escaneo del sistema.
    pub(crate) library_list: Vec<LibraryEntry>,
    /// ¿Concedido el acceso a todos los archivos (API 30+) o asumido (≤ 12)?
    pub(crate) permission_granted: bool,
    /// Nivel de API (Build.VERSION.SDK_INT): decide columnas y permisos.
    pub(crate) sdk_int: i32,
    /// ¿Pendiente de volver de Ajustes tras pulsar Grant? (re-consultar en Resume).
    pub(crate) grant_pending: bool,
    /// Desplazamiento del picker en filas (scroll; la BIBLIOTECA usa ahora
    /// `lib_scroll` en píxeles — ver abajo).
    pub(crate) list_scroll: usize,
    /// Scroll VERTICAL del contenido de la biblioteca en PÍXELES (recientes +
    /// título de archivos + rejilla; la cabecera, la barra de filtros y el
    /// estado son fijos). Sustituye al scroll en filas de `list_scroll` para
    /// el modo Library (las secciones tienen alturas distintas, así que el
    /// scroll en filas ya no vale).
    pub(crate) lib_scroll: f32,
    /// Scroll horizontal (px) del carousel de RECIENTES (Continue Reading).
    pub(crate) lib_carousel_x: f32,
    /// Scroll horizontal (px) de la fila de chips de carpetas (búsqueda).
    pub(crate) lib_folders_x: f32,
    /// Scroll horizontal (px) de la fila de chips de letras (búsqueda).
    pub(crate) lib_letters_x: f32,
    /// Scroll horizontal (px) de la fila de chips de SORT (organización).
    pub(crate) lib_sort_x: f32,
    /// Scroll horizontal (px) de la fila de chips de FILTER (organización).
    pub(crate) lib_filter_x: f32,
    /// Filtro de letra inicial activo ('A'..='Z', '#' = dígito/otro); None =
    /// sin filtro de letra. Quedó sin UI desde el buscador con TECLADO
    /// (2026-08-25): siempre None, el código se conserva por compatibilidad.
    pub(crate) lib_letter: Option<char>,
    /// Filtro de carpeta activo (RELATIVE_PATH, p. ej. "Download/"); None =
    /// sin filtro de carpeta. Quedó sin UI desde el buscador con TECLADO:
    /// siempre None, el código se conserva por compatibilidad.
    pub(crate) lib_folder: Option<String>,
    /// ¿El campo de búsqueda está desplegado? (true → panel de chips de
    /// letra/carpeta visible bajo el campo). Siempre false desde el buscador
    /// con TECLADO (2026-08-25): el panel de chips A-Z/carpetas se eliminó
    /// de la UI; el código se conserva por compatibilidad.
    pub(crate) lib_search_open: bool,
    /// Texto del BUSCADOR CON TECLADO: filtro por subcadena (case-
    /// insensitive) sobre el TÍTULO del libro, según lo que el usuario teclea
    /// en el IME (`jni::ime_*`). Vacío = sin filtro.
    pub(crate) lib_query: String,
    /// ¿El teclado del buscador está abierto? true → `tick` hace polling del
    /// texto del EditText invisible (`jni::ime_text`) y re-filtra la rejilla.
    pub(crate) ime_active: bool,
    /// Layout de la biblioteca (rejilla/listas): el menú View "⋯" lo
    /// alterna (Tarea 2); se persiste en `state.json` (`ViewerState`).
    pub(crate) view_mode: LibraryViewMode,
    /// Ajuste de las portadas en sus marcos (Crop/Fit): menú View "⋯";
    /// persistido.
    pub(crate) cover_fit: LibraryCoverFit,
    /// ¿Columnas automáticas (por ancho de ventana)? menú View "⋯" (Tarea 2).
    pub(crate) auto_columns: bool,
    /// Nº de columnas fijas de la rejilla (si `auto_columns` es false);
    /// persistido.
    pub(crate) columns: u32,
    /// ¿El dropdown del menú View "⋯" está abierto? Abrir uno cierra el
    /// otro (mutuamente excluyentes).
    pub(crate) view_menu_open: bool,
    /// ¿El dropdown del menú Settings "☰" está abierto?
    pub(crate) settings_menu_open: bool,
    /// ¿Ocultar portadas (solo títulos)? menú Settings "☰"; persistido.
    pub(crate) hide_covers: bool,
    /// ¿Mostrar la estantería de recientes? menú Settings "☰"; persistido.
    pub(crate) recent_shelf_enabled: bool,
    /// Tamaño de portadas (0: Pequeño, 1: Mediano, 2: Grande); menú Settings "☰"; persistido.
    pub(crate) cover_size: u8,
    /// ¿Mostrar badge de porcentaje leído sobre las portadas? menú Settings "☰"; persistido.
    pub(crate) cover_progress: bool,
    /// Timeout para confirmar el vaciado de la biblioteca (3 segundos).
    pub(crate) clear_confirm_until: Option<std::time::Instant>,
    /// Agrupación de la biblioteca (None = libros sueltos, Author = por autor).
    pub(crate) group_by: LibraryGroupBy,
    /// Orden de "My Library" (chips de sort: Recently Added / Recently Read /
    /// Title / Author).
    pub(crate) lib_sort: LibSort,
    /// Filtro de ESTADO de "My Library" (None = All); también decide si
    /// "Continue Reading" se muestra (solo All/Reading).
    pub(crate) lib_status: Option<BookStatus>,
    /// Registro de PROGRESO por libro (persistido en `internal/library.json`;
    /// ver `persist::BookProgress`): path → {page, page_count, last_read,
    /// added}. Alimenta "Continue Reading", las barras de progreso de la
    /// rejilla, el sort y el filtro de estado. Se actualiza al abrir/cambiar
    /// de página (en `save_state`).
    pub(crate) lib_books: Vec<BookProgress>,
    /// Índices de `library_list` que pasan el filtro actual (cache del
    /// filtrado): la rejilla y las portadas resuelven sobre esta lista.
    pub(crate) lib_filtered: Vec<usize>,
    /// Lista de recientes persistida (los últimos ~10 PDFs abiertos, más
    /// reciente primero; `persist::load_recents`/`touch_recent`).
    pub(crate) recents: Vec<RecentEntry>,
    /// La lista del picker necesita re-render (rescan, scroll, resize).
    pub(crate) list_dirty: bool,
    /// Mensaje de estado del picker (bajo la cabecera; p. ej. error de open).
    pub(crate) status: Option<String>,
    /// Ruta local del PDF abierto (clave del estado persistido; None si no
    /// hay documento). Se setea en `open_pdf` y en el "abrir con" del arranque.
    pub(crate) doc_path: Option<String>,
    /// Directorio interno de la app (para `state.json`; ver `persist`).
    internal_dir: Option<PathBuf>,
    /// Tema activo de la interfaz (DefaultLight, SepiaLight, DefaultDark, SepiaDark).
    pub(crate) theme: theme::AppTheme,
    /// Modo oscuro activo (página invertida + fondo oscuro).
    pub(crate) dark: bool,
    /// ¿Chrome del visor visible? (barra superior fina + barra inferior de progreso).
    pub(crate) chrome_visible: bool,
    /// Momento de expiración para auto-ocultar el chrome del visor (≤ 2.5 s).
    pub(crate) chrome_hide_at: Option<Instant>,
    /// Bitmap renderizado de la barra superior de chrome del visor.
    pub(crate) chrome_top_bitmap: Option<Bitmap>,
    /// Bitmap renderizado de la barra inferior de chrome del visor.
    pub(crate) chrome_bottom_bitmap: Option<Bitmap>,
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
    pub(crate) sheet_bitmap: Option<Bitmap>,
    /// Bitmap del indicador de página "N / total" (overlay abajo a la
    /// izquierda, tap = página siguiente), cacheado: se invalida al cambiar
    /// ventana, página o modo oscuro.
    pub(crate) page_badge: Option<Bitmap>,
    /// Bitmap del indicador de MODO del boli (overlay abajo a la derecha,
    /// ✏️/🖍️): se invalida al alternar modo o cambiar ventana — el usuario
    /// siempre ve en qué modo va a dibujar el boli.
    pub(crate) mode_badge: Option<Bitmap>,
    /// Posición de pantalla de la GOMA durante el borrado (None = sin gesto
    /// de borrado): dibuja el cursor circular (`eraser_cursor`) para que el
    /// usuario vea exactamente qué área se va a borrar.
    pub(crate) erase_pt: Option<(f32, f32)>,
    /// Radio del cursor de la goma en PÍXELES (radio en puntos × escala
    /// efectiva; fijo durante el gesto — el zoom no cambia mientras se borra).
    pub(crate) erase_r_px: f32,
    /// Bitmap cacheado del cursor circular de la goma (se regenera por gesto).
    pub(crate) eraser_cursor: Option<Bitmap>,
    /// Caché LRU de portadas de la biblioteca (content:// URI → portada de la
    /// página 1, `THUMB_W` px de ancho). Se limpia al abrir un PDF: las
    /// portadas y la `PageCache` del visor no compiten por el mismo
    /// presupuesto (estados mutuamente exclusivos: biblioteca vs visor).
    pub(crate) thumbs: ThumbCache,
    /// URIs cuya portada falló al renderizar (PDF corrupto, fd no abrible,
    /// página 1 vacía): no se reintentan — evita un bucle de timeout del
    /// bucle de eventos (`thumbs_pending` las excluye).
    thumb_failed: HashSet<String>,
    /// Bitmap CACHEADO de la zona FIJA de la biblioteca (cabecera editorial +
    /// campo de búsqueda + panel de chips + franja de estado): alto =
    /// `lib_content_y0`, origen = borde superior de la ventana. Se
    /// re-renderiza SÓLO cuando cambia la estructura (datos, filtros, panel
    /// de búsqueda, estado, tamaño de ventana), NUNCA por frame de scroll
    /// (el blit copia la zona fija + la banda de contenido, ver `lib_band`).
    /// Es el análogo del frame compuesto del visor para la biblioteca.
    pub(crate) lib_header: Option<Bitmap>,
    /// Bitmap CACHEADO del contenido scrolleable de la biblioteca (Continue
    /// Reading + My Library + rejilla o empty state): una BANDA de alto =
    /// viewport de contenido + margen de prefetch (1 celda arriba/abajo),
    /// origen en coordenadas de CONTENIDO (`.1` = contenido-y del borde
    /// superior de la banda). El scroll vertical solo cambia DE DÓNDE se
    /// copia la banda al buffer (memcpy por fila, ~1-3 ms), en vez de
    /// re-renderizar toda la pantalla por Canvas+JNI en cada frame
    /// (~20-60 ms → el lag/parpadeo del scroll que se reportó). La banda se
    /// re-renderiza cuando el scroll sale de su rango o cambia el contenido
    /// (datos, filtros, sort, search, thumbs nuevos, ventana).
    pub(crate) lib_band: Option<(Bitmap, i32)>,
    /// Zona cuya fila HORIZONTAL necesita re-render (1 = carousel de Continue
    /// Reading, 2 = chips de letras, 3 = chips de carpetas, 4 = chips de
    /// SORT, 5 = chips de FILTER): el input la fija al arrastrar una fila en
    /// horizontal y `redraw` re-renderiza SOLO esa fila (bitmap pequeño,
    /// Canvas+JNI barato) y la remienda sobre su contenedor (cabecera o
    /// banda). None = sin fila pendiente.
    pub(crate) lib_row_dirty: Option<u8>,
    /// Transición visual al ABRIR un libro (desde la biblioteca o el picker):
    /// snapshot de la pantalla de lista capturado justo antes del cambio de
    /// modo + momento de la captura. Durante `LIB_FADE_MS` el visor lo funde
    /// sobre la página (alfa decreciente) — una transición breve y barata
    /// (blend RGB por filas, ~1-5 ms/frame en la tablet; ~12 frames). Se
    /// libera al terminar; None = sin transición.
    pub(crate) lib_fade: Option<(Instant, Bitmap)>,
    /// Estado del arrastre de las listas (picker y biblioteca): punto del
    /// Down + scrolls de partida + zona de la biblioteca (qué arrastra en
    /// horizontal). Ver `ListDrag`.
    pub(crate) list_drag: Option<ListDrag>,
    /// Anotaciones del documento abierto: se cargan del sidecar SQLite al
    /// abrir (`load_annotations`) y se guardan al añadir/quitar un trazo
    /// (`save_annotations`). El modelo vive en pdf_core (AGENTS.md §4.3).
    pub(crate) annotations: AnnotationSet,
    /// Ruta del sidecar del documento abierto (`store::sidecar_path`:
    /// `<pdf-dir>/annotations/<stem>.db`); None sin documento. El sidecar de
    /// un PDF abierto por content:// (biblioteca o "abrir con") queda junto
    /// a la copia en `internal/pdfs/` → `internal/pdfs/annotations/<stem>.db`
    /// (ver `open_library_entry`/`jni::launch_intent_pdf`).
    /// Caché de texto por página (Fase B1): el resaltador y la selección
    /// leen `get_or_extract` en vez de `doc.text()` (que re-parsea stext en
    /// el hilo UI). Prefetcheada al abrir el PDF (página visible ±2) y
    /// limpiada al cambiar de documento.
    text_cache: PageTextCache,
    annot_sidecar: Option<PathBuf>,
    /// Selección de texto en curso (long-press + arrastre) en px de ventana
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
    /// Receptor del hilo de fondo de IA (std::thread + mpsc, el patrón de
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
    pub(crate) toast_bitmap: Option<Bitmap>,
    /// Herramienta de anotación activa en el visor (Fase 3.5): Navegar
    /// (gestos normales) / Resaltar / Boli. Con una herramienta distinta de
    /// Navegar el arrastre de UN dedo (o el lápiz de la tablet) dibuja en
    /// vez de navegar; el tap simple no cambia de página (`input`), y la
    /// selección de texto (long-press) queda desactivada mientras esté
    /// activa (`input::tick_gestures`).
    pub(crate) tool: ToolKind,
    /// Modo del BOLI persistido (`PenMode`): el boli dibuja (Ink) o subraya
    /// (Highlight) SIEMPRE que toca el PDF, sin depender de la barra de
    /// herramientas; el botón UP del boli lo alterna (`toggle_pen_mode`) y se
    /// guarda en `tool_state.json`. La barra (Fase 3.5) sigue existiendo y
    /// `set_tool` sincroniza este modo para que ambas entradas coincidan.
    pub(crate) pen_mode: PenMode,
    /// ¿El gesto de BORRADO en curso ha eliminado alguna anotación? Se guarda
    /// `store.save` UNA vez al levantar (o cancelar) si cambió algo.
    erase_dirty: bool,
    /// Última posición de la GOMA en coords de página (para el barrido
    /// continuo del borrado: un punto entre dos pasadas consecutivas también
    /// se borra). None = sin barrido previo (primer Move del gesto).
    erase_last: Option<(f32, f32)>,
    pub(crate) status_bar_top: i32,
    /// Color actual de la tinta del boli (arranca en `DEFAULT_INK_COLOR`).
    pub(crate) ink_color: Color,
    /// Grosor actual del boli en pt (arranca en `STROKE_WIDTH_PT`). Cada
    /// trazo guarda su grosor.
    pub(crate) ink_width: f32,
    /// Gesto de herramienta EN CURSO (dedo/lápiz bajado con una herramienta
    /// activa): puntos y ancla en coordenadas de PÁGINA (ver `ToolGesture`).
    /// `Some` mientras el dedo está abajo; se convierte en una anotación
    /// guardada al levantar (`end_tool_gesture`) o se descarta al cancelar.
    /// Mientras es `Some`, `blit` usa el frame compuesto + la capa temporal
    /// del trazo (sin re-blitear la página por Move — requisito 5).
    pub(crate) tool_gesture: Option<ToolGesture>,
    /// ids de las anotaciones CREADAS EN ESTA SESIÓN (dedo/lápiz, en orden
    /// de creación). Solo anotaciones nuevas (no las cargadas del sidecar).
    pub(crate) session_ids: Vec<u64>,
    /// Contexto GPU del visor (Fase 2, ADR-006): EGL/GLES2. Some entre
    /// InitWindow y TerminateWindow (y solo si la creación EGL tuvo éxito —
    /// sin fallback al blit SW: si EGL falla, el visor no pinta).
    gpu: Option<Gpu>,
    /// Repintado pendiente (coalescing por vsync): sigue vivo para Library/
    /// Picker (SW) y para pedir frames GPU (el bucle llama `blit` una vez
    /// por iteración tras `take_repaint()`).
    repaint: bool,
    /// Probe de telemetría (solo logcat): mantenido para comparar el coste
    /// del frame completo GPU con el dirty rect de la Fase 1 (ink_dirty).
    take_repaint_probe: Option<(i32, i32, i32, i32)>,
    /// Fase 1 USI: ancla temporal del gesto (event_time del Down, base
    /// System.nanoTime) — los t_ms de las muestras se re-escalan contra ella.
    /// La fija `input` antes de `begin_tool_gesture`.
    pub(crate) pending_t0_ns: Option<u64>,
    /// Presión normalizada del último evento (0.5 si el driver no la da).
    pub(crate) pending_pressure: Option<f32>,
    /// Ancla temporal del gesto en curso (ns, System.nanoTime del Down del
    /// boli); la lee `feed_stylus_history` para re-escalar los timestamps.
    pub(crate) gesture_t0_ns: u64,
    /// Último instante en que el STYLUS tocó la pantalla (para palm rejection
    /// por tiempo: tras escribir, se ignora el táctil del dedo/palma durante
    /// ~500ms para evitar pans/zooms accidentales al apoyar la mano).
    last_stylus_time: Option<std::time::Instant>,
    /// Render ASÍNCRONO en vuelo (zoom sharp y cambio de página sin congelar
    /// el hilo UI): worker con su PROPIO documento (MuPDF no es Send — patrón
    /// de `prefetch.rs`). Cada lote lleva un `render_seq`; al recibir, si el
    /// seq no es el actual (el usuario hizo otro zoom/página), se descarta.
    render_rx: Option<std::sync::mpsc::Receiver<WorkerMsg>>,
    render_seq: u64,
    /// Actor persistente de render (F3.1): UN hilo con su propio documento
    /// para toda la vida del documento abierto. `None` hasta `open_pdf_at`.
    render_worker: Option<RenderWorker>,
    /// Último Move del pinch en curso (F3.2): `tick` dispara el render
    /// nítido tras 350 ms de quietud; `set_zoom_sharp` lo limpia al soltar.
    last_pinch_move: Option<Instant>,
    /// Página ANTERIOR dibujable mientras llega el render de la nueva (si
    /// está en caché): evita el parpadeo en blanco al pasar página.
    pub(crate) fallback_page: Option<u32>,
    /// Worker actor para render de portadas en segundo plano (Fase E1).
    thumb_worker: Option<crate::thumbs::ThumbWorker>,
    thumb_rx: Option<std::sync::mpsc::Receiver<crate::thumbs::ThumbMsg>>,
}
/// Mensaje del worker de render asíncrono: un bitmap listo a la escala
/// pedida (`target_zoom` = factor de zoom con el que se renderizó, la "escala
/// efectiva" = cover × target_zoom).
struct WorkerMsg {
    seq: u64,
    page: u32,
    bitmap: Bitmap,
    target_zoom: f32,
}
/// Petición de render al worker actor: páginas a la escala pedida, ventana
/// congelada del cover y canal de respuesta propio por lote. Port de F3.1
/// (`mejora_zoom`): el hilo drena comandos entre páginas (preemption por
/// `seq`) en `render_worker_req`.
struct WorkerReq {
    seq: u64,
    pages: Vec<u32>,
    target_zoom: f32,
    clamp_level: bool,
    win_w: i32,
    win_h: i32,
    reply: std::sync::mpsc::Sender<WorkerMsg>,
}

/// Comandos del worker actor: render de la última petición o parada limpia.
enum WorkerCmd {
    Render(WorkerReq),
    Stop,
}

/// Controlador del worker actor de render (F3.1): canal de comandos + join
/// handle. El hilo retiene su propio `MupdfDocument` (MuPDF no es Send) y
/// muere solo con `Stop`. Sustituye al hilo-por-zoom anterior, que disparaba
/// el contador de hilos y reabría el PDF en cada gesto.
struct RenderWorker {
    tx: std::sync::mpsc::Sender<WorkerCmd>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Ejecuta `req` en el worker: renderiza las páginas a la escala pedida y
/// envía cada bitmap por `req.reply`. ENTRE páginas drena el canal: una
/// petición con `seq` mayor sustituye a la actual y `Stop` sale del actor.
/// Errores por página: best-effort (drop silencioso; el UI muestra fallback).
fn render_worker_req(
    doc: &MupdfDocument,
    rx: &std::sync::mpsc::Receiver<WorkerCmd>,
    req: WorkerReq,
) {
    let target = if req.clamp_level {
        let level = pdf_core::scale_level_for_zoom(req.target_zoom).min(1);
        2f32.powi(level as i32)
    } else {
        req.target_zoom
    };
    for page in req.pages {
        // Preemption: un seq posterior ya lanzado anula las páginas restantes.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WorkerCmd::Stop => return,
                WorkerCmd::Render(newer) => {
                    if newer.seq > req.seq {
                        render_worker_req(doc, rx, newer);
                        return;
                    }
                    // seq <= req.seq: lote viejo encolado, ignorar.
                }
            }
        }
        if let Ok((pw, ph)) = doc.page_size(page) {
            let cover = initial_scale(pw, ph, req.win_w, req.win_h);
            // Presupuesto: el bitmap debe caber en la caché (misma regla que
            // `Reader::budget_scale` — duplicada aquí porque el worker no
            // tiene acceso a `self`).
            let mut scale = cover * target;
            let px_pdf = pw as f64 * ph as f64;
            let max_px = crate::cache::CACHE_BYTE_BUDGET as f64 / 4.0;
            while scale > 0.001 && px_pdf * scale as f64 * scale as f64 > max_px {
                scale *= 0.5;
            }
            let target_eff = if cover > 0.0 { scale / cover } else { 1.0 };
            if let Ok(bmp) = doc.render_page(page, scale) {
                let _ = req.reply.send(WorkerMsg {
                    seq: req.seq,
                    page,
                    bitmap: bmp,
                    target_zoom: target_eff,
                });
            }
        }
    }
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
            win_w: 0,
            win_h: 0,
            gesture: GestureState::new(),
            mode: UiMode::Library,
            pdf_list: Vec::new(),
            picker_kind: PickerKind::Files,
            select_list: Vec::new(),
            sel_dir: Vec::new(),
            library_list: Vec::new(),
            permission_granted: false,
            sdk_int: android_sdk_int(),
            grant_pending: false,
            list_scroll: 0,
            lib_scroll: 0.0,
            lib_carousel_x: 0.0,
            lib_folders_x: 0.0,
            lib_letters_x: 0.0,
            lib_sort_x: 0.0,
            lib_filter_x: 0.0,
            lib_letter: None,
            lib_folder: None,
            lib_search_open: false,
            lib_query: String::new(),
            ime_active: false,
            view_mode: LibraryViewMode::Grid,
            cover_fit: LibraryCoverFit::Crop,
            auto_columns: true,
            columns: 3,
            view_menu_open: false,
            settings_menu_open: false,
            hide_covers: false,
            recent_shelf_enabled: true,
            cover_size: 1,
            cover_progress: false,
            clear_confirm_until: None,
            group_by: LibraryGroupBy::None,
            lib_sort: LibSort::RecentlyAdded,
            lib_status: None,
            lib_books: persist::load_progress(app.internal_data_path().as_deref()),
            lib_filtered: Vec::new(),
            recents: persist::load_recents(app.internal_data_path().as_deref()),
            list_dirty: true,
            status: None,
            doc_path: None,
            internal_dir: app.internal_data_path(),
            theme: theme::AppTheme::DefaultLight,
            dark: false,
            chrome_visible: false,
            chrome_hide_at: None,
            chrome_top_bitmap: None,
            chrome_bottom_bitmap: None,
            sheet_open: false,
            sheet_progress: 0.0,
            sheet_anim: false,
            sheet_bitmap: None,
            page_badge: None,
            mode_badge: None,
            erase_pt: None,
            erase_r_px: 0.0,
            eraser_cursor: None,
            thumbs: ThumbCache::new(THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES),
            thumb_failed: HashSet::new(),
            lib_header: None,
            lib_band: None,
            lib_row_dirty: None,
            lib_fade: None,
            list_drag: None,
            annotations: AnnotationSet::new(),
            annot_sidecar: None,
            text_cache: PageTextCache::default(),
            status_bar_top: 0, // se fija en runtime (content_rect top)
            sel: None,
            sel_menu: None,
            ai_panel: None,
            ai_text: String::new(),
            ai_phase: AiPhase::Asking,
            ai_rx: None,
            toast: None,
            toast_bitmap: None,
            tool: ToolKind::Navigate,
            erase_dirty: false,
            erase_last: None,
            ink_color: {
                let ts = persist::load_tool_state(app.internal_data_path().as_deref());
                ts.ink_color
            },
            ink_width: {
                let ts = persist::load_tool_state(app.internal_data_path().as_deref());
                ts.ink_width
            },
            pen_mode: load_pen_mode(app.internal_data_path().as_deref()),
            tool_gesture: None,
            session_ids: Vec::new(),
            repaint: false,
            take_repaint_probe: None,
            gpu: None,
            pending_t0_ns: None,
            pending_pressure: None,
            gesture_t0_ns: 0,
            last_stylus_time: None,
            render_rx: None,
            render_seq: 0,
            render_worker: None,
            last_pinch_move: None,
            fallback_page: None,
            thumb_worker: None,
            thumb_rx: None,
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
                        // Y añadirlo a los RECIENTES de la biblioteca (el
                        // "abrir con" no pasa por `open_pdf`).
                        reader.touch_recent(&lp.path);
                        reader.start_render_worker(&lp.path);
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
            // página/zoom/tema; si ya no existe (o no se puede abrir),
            // se BORRA el estado y se muestra la BIBLIOTECA CURADA
            // (`internal/library.json`) — SIN escanear MediaStore.
            None => {
                let restored =
                    if let Some(state) = persist::load_state(reader.internal_dir.as_deref()) {
                        if let Some(th) = state.theme {
                            reader.theme = th;
                            reader.dark = th.is_dark();
                        } else {
                            reader.dark = state.dark;
                            reader.theme = if state.dark {
                                theme::AppTheme::DefaultDark
                            } else {
                                theme::AppTheme::DefaultLight
                            };
                        }
                        // Preferencias de la BIBLIOTECA (menús ⋯/☰, Tarea 1:
                        // esqueleto): se restauran aunque el PDF guardado ya
                        // no exista (caen a la biblioteca con su layout).
                        reader.view_mode = state.view_mode;
                        reader.cover_fit = state.cover_fit;
                        reader.columns = state.columns;
                        reader.hide_covers = state.hide_covers;
                        reader.recent_shelf_enabled = state.recent_shelf_enabled;
                        reader.cover_size = state.cover_size;
                        reader.cover_progress = state.cover_progress;
                        // Solo restaurar si el PDF sigue accesible: `open_pdf`
                        // falla si no se puede abrir (corrupto) y deja el
                        // estado intacto.
                        if Path::new(&state.path).exists() && reader.open_pdf(&state.path) {
                            let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                            reader.page = state.page.min(pages.saturating_sub(1));
                            reader.zoom = state.zoom.clamp(PINCH_MIN, PINCH_MAX);
                            reader.rendered_zoom = reader.zoom;
                            // Modo UNA HOJA: la página restaurada se fija
                            // directamente (no hay scroll que alinear).
                            reader.cache.clear();
                            reader.page_badge = None; // indicador de la página restaurada
                            info!(
                                "restored {} @page {} zoom {:.3} theme {:?}",
                                state.path,
                                reader.page + 1,
                                reader.zoom,
                                reader.theme
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
                    // limpiar el estado huérfano y mostrar la BIBLIOTECA
                    // CURADA. Sin intent NO hay escaneo de MediaStore: la
                    // rejilla sale de `library.json` (con migración one-shot
                    // de los PDFs que instalaciones antiguas dejaran en
                    // `internal/pdfs/`); vacía → empty state con "Añadir PDF".
                    persist::clear_state(reader.internal_dir.as_deref());
                    reader.reload_curated_library(app);
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
        // El pipeline del visor es GPU (EGL): el contexto se crea UNA vez con
        // la primera ventana de Viewer y sobrevive a surfaces nuevas
        // (recreate_surface). En modos SW (Library/Picker) no se toca.
        if self.mode == UiMode::Viewer {
            match self.gpu.as_mut() {
                Some(g) => {
                    g.recreate_surface(&window);
                }
                None => {
                    // SAFETY: EGL/GLES sobre una NativeWindow válida de
                    // android_activity; fallo → Viewer cae al camino SW.
                    let gpu = unsafe { Gpu::new(&window) };
                    if gpu.is_none() {
                        warn!("gpu: EGL init failed — Viewer en SW");
                    }
                    self.gpu = gpu;
                }
            }
        }
        self.window = Some(window);
    }

    /// `InitWindow`: nueva ventana lista. Fuerza buffers RGBA8888 (0,0 =
    /// conservar tamaño base; solo cambia el formato) e invalida la caché.
    pub(crate) fn init_window(&mut self, window: NativeWindow) {
        self.set_window(window);
        self.bitmap = None;
        self.lib_header = None;
        self.lib_band = None;
        self.page_badge = None;
        self.mode_badge = None;
        self.sheet_bitmap = None;
        self.list_dirty = true;
        // Nueva ventana → posible nueva escala cover: las páginas de la caché
        // se reutilizan si el tamaño no cambió; el redraw detecta el cambio de
        // `win_w/h` y limpia la caché si hace falta.
        self.redraw();
    }

    /// `TerminateWindow`: soltar la ventana (drop → `ANativeWindow_release`).
    pub(crate) fn terminate_window(&mut self) {
        if let Some(g) = self.gpu.as_mut() {
            g.drop_surface();
        }
        self.window = None;
        self.bitmap = None;
        self.page_badge = None;
        self.mode_badge = None;
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
            self.lib_header = None; // zona fija de la biblioteca: tamaño nuevo
            self.lib_band = None; // banda de contenido: tamaño nuevo
            self.cache.clear(); // nueva escala cover → los bitmaps viejos no sirven
            self.list_dirty = true;
            self.page_badge = None;
            self.chrome_top_bitmap = None;
            self.chrome_bottom_bitmap = None;
            self.sheet_bitmap = None;
        }
        match self.mode {
            UiMode::Viewer => {
                // Modo UNA HOJA: la página actual + vecinas se garantizan de
                // forma ASÍNCRONA (worker, patrón de `goto_page`): si alguna
                // falta y no hay ya un lote en vuelo, se lanza — el render
                // síncrono aquí congelaba el UI cada `RedrawNeeded` del
                // sistema (medido: 3 páginas × 40-120 ms, repetido).
                let needs = {
                    let n = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                    let lo = self.page.saturating_sub(1);
                    let hi = (self.page + 1).min(n.saturating_sub(1));
                    (lo..=hi)
                        .filter(|&p| self.cache.peek(p).is_none())
                        .collect::<Vec<u32>>()
                };
                if !needs.is_empty() && self.render_rx.is_none() {
                    self.launch_render(needs, self.rendered_zoom, false);
                }
                // Chrome del visor y sheet de ajustes
                if self.chrome_visible {
                    if self.chrome_top_bitmap.is_none() {
                        self.chrome_top_bitmap = render_viewer_top_chrome(self);
                    }
                    if self.chrome_bottom_bitmap.is_none() {
                        self.chrome_bottom_bitmap = render_viewer_bottom_chrome(self);
                    }
                }
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
            UiMode::Picker => {
                // Clamp del scroll si la lista menguó (rescan/cancel) o cambió
                // la ventana (picker: filas de `picker_row_h`).
                let max_scroll = self.picker_len().saturating_sub(self.picker_visible());
                if self.list_scroll > max_scroll {
                    self.list_scroll = max_scroll;
                }
                if self.list_dirty
                    && let Some(bmp) = render_picker_list(self)
                {
                    self.bitmap = Some(bmp);
                    self.offset_x = 0;
                    self.offset_y = 0;
                    self.list_dirty = false;
                }
            }
            UiMode::Library => {
                // Clamp del scroll VERTICAL (px) si el contenido menguó
                // (filtro, rescan, ventana): cabecera + campo de búsqueda +
                // franja de estado son fijas; el contenido (Continue Reading
                // + My Library) scrollea bajo ellas. Los scrolls
                // HORIZONTALES (carousel, panel de búsqueda y organización)
                // se clampean igual contra su ancho total.
                let max_v = self.lib_max_scroll();
                if self.lib_scroll > max_v {
                    self.lib_scroll = max_v;
                }
                self.lib_carousel_x = self.lib_carousel_x.min(self.lib_cont_max_x());
                self.lib_letters_x = self.lib_letters_x.min(self.lib_chips_max_x(0));
                self.lib_folders_x = self.lib_folders_x.min(self.lib_chips_max_x(1));
                self.lib_sort_x = self.lib_sort_x.min(self.lib_org_max_x(0));
                self.lib_filter_x = self.lib_filter_x.min(self.lib_org_max_x(1));

                if self.list_dirty {
                    // Cambio ESTRUCTURAL (datos/filtros/sort/search/estado/
                    // ventana/entrada): re-renderizar cabecera + banda de
                    // contenido + filas horizontales + portadas. Es el
                    // render CARO (Canvas+JNI), pagado una vez por cambio,
                    // nunca por frame de scroll.
                    self.rebuild_library();
                } else if let Some(zone) = self.lib_row_dirty {
                    // Solo una fila HORIZONTAL se arrastró (carousel o
                    // chips): re-renderizar ESA fila y remendarla sobre su
                    // contenedor — barato (área pequeña), sin tocar el resto.
                    self.rebuild_library_row(zone);
                } else if !self.lib_band_covers() {
                    // El scroll salió de la banda actual: re-bandear
                    // (render de la banda en la nueva posición; cabecera
                    // intacta).
                    self.rebuild_library_band();
                }
            }
        }
        if self.window.is_some() {
            self.blit();
        }
    }

    // ---------------------------------------------------------------------
    // Biblioteca: render CACHEADO en dos planos (zona fija + banda de
    // contenido). El scroll vertical por frame es un memcpy (blit_library),
    // no un re-render Canvas+JNI (~20-60 ms): el mismo patrón que
    // compose_frame/blit_composed del visor aplicado a la biblioteca
    // (2026-08-22, fix del lag/parpadeo del scroll reportado).
    // ---------------------------------------------------------------------

    /// Rebuild COMPLETO de la biblioteca: cabecera (zona fija) + banda de
    /// contenido + filas horizontales + portadas. Se llama solo cuando
    /// cambia la ESTRUCTURA (datos, filtros, sort, panel de búsqueda,
    /// status, ventana, entrada), nunca por frame de scroll.
    fn rebuild_library(&mut self) {
        self.list_dirty = false;
        self.lib_row_dirty = None;
        // 1) Zona fija (cabecera editorial + campo de búsqueda + panel +
        //    franja de estado).
        self.lib_header = render_library_header(self);
        // 2) Banda de contenido en la posición actual del scroll.
        self.rebuild_library_band();
        // 3) Filas horizontales dentro de sus contenedores (carousel,
        //    chips del panel de búsqueda y de organización).
        self.splice_library_rows();
    }

    /// Re-renderiza SOLO la banda de contenido (sin tocar la cabecera): se
    /// llama al entrar/salir de una banda (scroll lejos del rango actual) o
    /// al rebuild completo. El render es Canvas+JNI UNA vez por banda; el
    /// scroll dentro de la banda es memcpy.
    fn rebuild_library_band(&mut self) {
        let content_y0 = lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some());
        let viewport = (self.win_h - content_y0).max(0);
        let content_h = self.lib_content_h() as i32;
        let margin = if self.is_grid() {
            let cols = self.effective_grid_cols();
            grid_cell_h(self.win_w, cols, self.cover_size) as i32
        } else {
            list_row_h(self.win_h, self.cover_size) as i32
        };
        let band_h = (viewport + 2 * margin).min(content_h.max(viewport));
        let band_origin = ((self.lib_scroll as i32) - margin)
            .max(0)
            .min((content_h - band_h).max(0));
        if let Some(bmp) = render_library_zone(self, band_origin, band_h) {
            let mut band = bmp;
            paste_lib_thumbs(self, &mut band, band_origin);
            self.lib_band = Some((band, band_origin));
            self.splice_band_rows();
        } else {
            self.lib_band = None;
        }
    }

    /// ¿La banda actual cubre la ventana de contenido con el scroll actual?
    /// false → hay que re-bandear (render de la banda en la nueva posición).
    fn lib_band_covers(&self) -> bool {
        match &self.lib_band {
            None => false,
            Some((bmp, origin)) => {
                let content_y0 =
                    lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some());
                let viewport = (self.win_h - content_y0).max(0);
                let s = self.lib_scroll as i32;
                s >= *origin && s + viewport <= *origin + bmp.height as i32
            }
        }
    }

    /// Re-renderiza SOLO la fila horizontal `zone` (pequeña, Canvas+JNI
    /// barato) y la remienda sobre su contenedor: el arrastre horizontal del
    /// carousel o de chips no re-renderiza la pantalla completa.
    fn rebuild_library_row(&mut self, zone: u8) {
        self.lib_row_dirty = None;
        match zone {
            2 | 3 => {
                // Chips del panel de búsqueda → cabecera (zona fija).
                let row = render_search_chip_row(self, (zone - 2) as usize);
                let x = if zone == 2 {
                    self.lib_letters_x as i32
                } else {
                    self.lib_folders_x as i32
                };
                let y = if zone == 2 {
                    lib_search_chips_y0(self)
                } else {
                    lib_search_chips_y1(self)
                };
                if let (Some(row), Some(h)) = (row, self.lib_header.as_mut()) {
                    splice_row(h, &row, -x, y as i32);
                }
            }
            // Zonas 1 (carousel), 4 y 5 (sort/filter) ya no existen en la
            // biblioteca minimalista: sin filas que remendar.
            _ => {}
        }
    }

    /// Remienda todas las filas horizontales sobre sus contenedores
    /// (cabecera: chips de búsqueda; banda: carousel + chips de organización).
    /// Se llama tras un rebuild completo (los contenedores acaban de
    /// renderizarse SIN las filas, que se leen de `lib_*_x`).
    fn splice_library_rows(&mut self) {
        let letters_row = if self.lib_search_open {
            render_search_chip_row(self, 0)
        } else {
            None
        };
        let folders_row = if self.lib_search_open {
            render_search_chip_row(self, 1)
        } else {
            None
        };
        let lx = self.lib_letters_x as i32;
        let fx = self.lib_folders_x as i32;
        let cy0 = lib_search_chips_y0(self) as i32;
        let cy1 = lib_search_chips_y1(self) as i32;
        if let Some(header) = self.lib_header.as_mut() {
            if let Some(row) = letters_row {
                splice_row(header, &row, -lx, cy0);
            }
            if let Some(row) = folders_row {
                splice_row(header, &row, -fx, cy1);
            }
        }
        self.splice_band_rows();
    }

    /// Remienda las filas horizontales de la BANDA sobre la banda actual.
    /// Biblioteca MINIMALISTA (estilo Readest): NO hay carousel de Continue
    /// Reading ni chips de sort/filter, así que no se remienda ninguna fila
    /// en la banda (solo los chips del panel de BÚSQUEDA, que viven en la
    /// cabecera fija).
    fn splice_band_rows(&mut self) {
        // Sin filas horizontales en la banda (carousel/organización ocultos).
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

    /// Escala límite por PRESUPUESTO de píxeles: el bitmap de la página
    /// (`pw×ph` pt) a esa escala no debe superar la caché (~48 MiB). Reduce la
    /// escala pedida por mitades hasta caber; nunca devuelve 0. Evita el
    /// render de cientos de MB al abrir con un zoom alto guardado (la
    /// "pillada") tanto en el worker como en el render síncrono del arranque.
    #[allow(dead_code)] // regla documentada; el worker la duplica
    fn budget_scale(&self, pw: f32, ph: f32, scale: f32) -> f32 {
        let max_px = crate::cache::CACHE_BYTE_BUDGET as f64 / 4.0;
        let mut s = scale.max(0.001);
        let px_pdf = pw as f64 * ph as f64;
        while px_pdf * s as f64 * s as f64 > max_px {
            s *= 0.5;
            if s <= 0.01 {
                break;
            }
        }
        s
    }

    /// Esquina superior izquierda del bitmap escalado para centrado
    /// horizontal: `base(z) = (win − doc·z) / 2` (px de zoom 1), la misma
    /// fórmula que `blit` usa para `dx` sin pan. Lineal en `z`; en el
    /// anclaje Y la base es 0 (el borde superior de la página actual está
    /// fijo en el borde superior del viewport — modo UNA HOJA, sin scroll).
    pub(crate) fn centered_base(win: i32, doc: f32, z: f32) -> f32 {
        (win as f32 - doc * z) / 2.0
    }

    /// Tamaño de página en puntos PDF (`None` si no hay documento o falla).
    pub(crate) fn page_size_pt(&self, page: u32) -> Option<(f32, f32)> {
        self.doc.as_ref()?.page_size(page).ok()
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
    #[allow(dead_code)] // superado por el render async (worker)
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
        // Orden: vecinas PRIMERO y la página actual ÚLTIMA. Con zoom alto cada
        // página (~36 MiB) supera el presupuesto de la caché (48 MiB), de modo
        // que renderizar la actual en medio hacía que la última vecina la
        // EVICTARA y el blit no encontrara bitmap → pantalla en blanco (fondo
        // puro) al soltar el pinch. Renderizarla última garantiza que sobreviva
        // a la evicción (las vecinas son prefetch best-effort y se re-renderizan
        // al navegar).
        let mut order: Vec<u32> = (lo..=hi).filter(|&p| p != self.page).collect();
        order.push(self.page);
        for page in order {
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
            // Presupuesto (mismo límite que el worker): evita el render de
            // cientos de MB al abrir con zoom alto guardado.
            let scale = self.budget_scale(pw, ph, scale);
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
    ///   sheet visible solo se copia el overlay del sheet sobre el frame GPU:
    ///   el frame se compone una vez y cada frame de la animación copia ese
    ///   bitmap (`draw::blit_composed`) + el overlay del sheet — la PÁGINA
    ///   NO se re-blitea en cada paso de la animación (el fix del lag del
    ///   sheet; ver `draw::compose_frame`).
    /// - Picker/Biblioteca: `zoom::blit_fast` con el bitmap de la lista.
    ///
    /// Aquí se decide SOLO el estado que depende del `Reader`: fondo rojo sin
    /// documento, modo oscuro (inversión al blitear) y overlays del visor.
    /// Blit del frame actual al ANativeWindow (lock+copy+unlock_and_post).
    /// Con el boli activo usa dirty rect + coalescing por vsync (el bucle
    /// principal lo llama una vez por iteración tras `take_repaint`).
    pub(crate) fn blit(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let t0 = Instant::now();
        let p = self.theme.palette();
        let bg = if self.doc.is_none() {
            theme::ERROR_BG_RGBA
        } else {
            p.rgba_bg()
        };
        // EGL ↔ ANativeWindow: excluyentes. Al salir del visor la surface
        // se suelta (ver `enter_library`) para que el lock SW funcione.
        // NOTA (2026-09-04, TCL): recrear EGL tras uso CPU falla con
        // EGL_BAD_ALLOC (la ventana no re-acepta EGL; ver `recreate_surface`
        // y el retorno de `open_pdf_at`, una vez por transición).
        if self.mode != UiMode::Viewer
            && let Some(g) = self.gpu.as_mut()
        {
            g.drop_surface();
        }
        match self.mode {
            UiMode::Viewer => {
                // FASE 2 (ADR-006): presentación por GPU (EGL/GLES2). La
                // página es una textura (subida SOLO al cambiar página o
                // re-render nítido), la tinta es geometría (strips con AA),
                // los overlays son quads de los bitmaps Canvas+JNI ya
                // generados y el present es `eglSwapBuffers` (spike 1: p50
                // 0.17 ms). Sin dirty rect CPU: frame completo por vsync.
                //
                // Materialización de overlays (misma que el blit SW): los
                // bitmaps se generan aquí si faltan y `present_viewer` los
                // sube como texturas cacheadas por puntero.
                if self.toast.is_some() && self.toast_bitmap.is_none() {
                    self.toast_bitmap = render_toast(self);
                }
                if !self.chrome_visible && self.mode_badge.is_none() {
                    self.mode_badge = render_mode_badge(self);
                }
                if self.erase_pt.is_some() && self.eraser_cursor.is_none() && self.erase_r_px > 4.0
                {
                    self.eraser_cursor = render_eraser_cursor(self, self.erase_r_px as i32);
                }
                // Present GPU: se toma el Gpu del Option (take) para poder
                // pasar `&self`Reader sin conflicto de préstamos — el
                // present solo LEE el Reader.
                if let Some(mut g) = self.gpu.take() {
                    g.present_viewer(self);
                    self.gpu = Some(g);
                }
            }
            UiMode::Library => {
                // Zona fija (`lib_header`) + banda de contenido (`lib_band`)
                // CACHEADAS: el frame por scroll es memcpy de los dos
                // rectángulos (misma idea que compose_frame/blit_composed
                // del visor), NO un re-render Canvas+JNI por frame. El
                // scroll solo cambia de dónde se copia la banda (`.1` =
                // contenido-y de su borde superior).
                let content_y0 =
                    lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some());
                let header = self.lib_header.as_ref();
                let band = self.lib_band.as_ref().map(|(b, o)| (b, *o));
                // Aviso breve (toast) integrado en el MISMO lock+present
                // que la biblioteca (antes: un segundo present por frame
                // durante ~1,5 s — innecesario).
                if self.toast.is_some() && self.toast_bitmap.is_none() {
                    self.toast_bitmap = render_toast(self);
                }
                let toast_ov: Option<(&Bitmap, i32, i32)> = self.toast_bitmap.as_ref().map(|tb| {
                    let tx = (self.win_w - tb.width as i32) / 2;
                    let ty = self.win_h - tb.height as i32 - 16;
                    (tb, tx, ty)
                });
                blit_library(
                    window,
                    p.rgba_lib_bg(),
                    header,
                    band,
                    self.lib_scroll as i32,
                    content_y0,
                    toast_ov,
                );
            }
            UiMode::Picker => match self.bitmap.as_ref() {
                Some(bmp) => blit_fast(window, bmp, 1.0, bg, (self.offset_x, self.offset_y), None),
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
        // Log solo en las rutas SW (Library/Picker): el visor GPU tiene su
        // propio `gl_present` (comparable con el spike). El probe de tinta
        // mantiene el nombre de evento para comparar con la Fase 1.
        if self.mode != UiMode::Viewer {
            info!(
                "blit {}x{}: {:.2} ms (lock+copy+unlock_and_post)",
                self.win_w,
                self.win_h,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some((x0, y0, x1, y1)) = self.take_repaint_probe {
            self.take_repaint_probe = None;
            info!(
                "ink_dirty {}x{} px ({}x{} @ {},{}): {:.2} ms",
                x1 - x0,
                y1 - y0,
                x1 - x0,
                y1 - y0,
                x0,
                y0,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Cambia a la página `page` (0-based) — modo UNA HOJA: `page` se fija
    /// directamente (no hay scroll que alinear: la columna de páginas se
    /// eliminó). Base compartida de `next_page`/`prev_page`/`jump_page` y del
    /// tap derecho/izquierdo. No hay salto con re-render: las páginas vecinas
    /// salen de la caché (paso instantáneo). Invalida los overlays cacheados
    /// (indicador, sheet, frame de la animación).
    fn goto_page(&mut self, page: u32) {
        let prev = self.page;
        if prev == page {
            return;
        }
        self.page = page;
        self.page_badge = None; // el indicador "N / total" cambia
        self.sheet_bitmap = None; // el indicador del sheet cambia
        info!("page {}", self.page + 1);
        // Cambio de página SIN congelar: si la nueva está en caché (prefetch
        // previo), el blit es inmediato; si no, se muestra la página ANTERIOR
        // (fallback) mientras el worker renderiza la nueva asíncronamente.
        if self.cache.peek(page).is_none() {
            self.fallback_page = Some(prev);
            let pages = {
                let n = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                let lo = page.saturating_sub(1);
                let hi = (page + 1).min(n.saturating_sub(1));
                (lo..=hi)
                    .filter(|&p| self.cache.peek(p).is_none())
                    .collect()
            };
            self.launch_render(pages, self.rendered_zoom, false);
        }
        self.save_state();
        if self.window.is_some() {
            self.blit();
        }
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
    // Selección de texto: long-press + arrastre, copiar y subrayar (Parte 1)
    // ---------------------------------------------------------------------
    //
    // El gesto vive en `input.rs` (long-press + arrastre); aquí
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

    /// Comienza el modo selección: ancla = punto del LONG-PRESS y punto
    /// actual = el mismo (el rect, un PUNTO aún sin arrastrar, crece con
    /// `update_sel`). Solo se llama al superar `LONG_PRESS_MS` con el dedo
    /// quieto (`input::tick_gestures`, `GestureKind::Selecting`). Blit
    /// directo (sin re-render):
    /// como en el pinch, la página está cacheada y solo cambia la capa.
    pub(crate) fn begin_sel(&mut self, ax: f32, ay: f32) {
        self.sel = Some(SelState {
            anchor: (ax, ay),
            cur: (ax, ay),
        });
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
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Fija la selección al levantar el dedo: si el rect es significativo
    /// (≥ `SEL_MIN_PX` por lado) abre el menú Copiar/Subrayar/IA; un
    /// long-press sin arrastre (rect degenerado, el punto) se descarta.
    pub(crate) fn end_sel(&mut self) {
        let Some((l, t, r, b)) = self.sel_screen_rect() else {
            self.clear_selection(); // no hubo arrastre
            return;
        };
        if (r - l).abs() < SEL_MIN_PX || (b - t).abs() < SEL_MIN_PX {
            self.clear_selection(); // long-press sin arrastre: nada que fijar
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
    pub(crate) fn sel_text(&mut self) -> String {
        let Some(page_rect) = self.sel_page_rect() else {
            return String::new();
        };
        let Some(doc) = self.doc.as_ref() else {
            return String::new();
        };
        let Ok(pt) = self.text_cache.get_or_extract(doc, self.page) else {
            return String::new();
        };
        let pt = pt.as_ref();
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
    /// prefijo (el contrato de `GeminiClient::explain_image`). None si no
    /// hay bitmap, escala inválida o el crop queda vacío (zoom/pan raro) —
    /// el llamador cae al envío solo-texto.
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
        // sin prefijo (contrato de `GeminiClient::explain_image`).
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
        if let Some(id) = self.annotations.add(self.page as usize, ann) {
            // El id devuelto va a la pila de la sesión: el "↶" de la barra
            // de herramientas deshace también los subrayados hechos con la
            // selección de texto (misma sesión).
            self.session_ids.push(id);
            self.save_annotations();
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
    // "Preguntar a la IA" (Parte 2): hilo de fondo + IA híbrida + panel
    // ---------------------------------------------------------------------
    //
    // El tap en "IA" del menú de selección (`input::sel_menu_tap`) llama a
    // `ask_ai`: se cierra el menú, se abre el panel en fase Asking
    // ("preguntando…") y se lanza un hilo de fondo (std::thread + mpsc, el
    // mismo patrón de `pdf_core::prefetch`) que llama a la IA y envía el
    // resultado por el canal. El hilo de UI sondea el canal en `tick`
    // (`try_recv`, sin bloquear) y al llegar el mensaje pasa el panel a
    // Answer (texto envuelto con scroll) o Error (mensaje claro en el mismo
    // panel). Decisiones:
    //
    // - HÍBRIDO (2026-08-XX): con IMAGEN de la selección (crop del bitmap
    //   cacheado, `sel_image_png_base64`) se llama a
    //   `GeminiClient::explain_image` (pdf_core::ai) con `GEMINI_MODEL`: el
    //   modelo de visión de Groq fue RETIRADO (403), así que la imagen va a
    //   Gemini; la imagen es la fuente principal para ecuaciones/gráficos y
    //   el texto extraído va como contexto adicional en el prompt (puede ser
    //   "" en un PDF escaneado). Sin imagen, se cae a `GroqClient::chat`
    //   solo-texto con `GROQ_MODEL` (el flujo de siempre).
    // - Reintento: si Gemini falla (p. ej. 503/busy), se intenta UNA vez más
    //   tras ~1 s DENTRO del hilo de fondo (nunca bloquea el hilo de UI); si
    //   sigue fallando, el panel muestra un error claro ("modelo ocupado,
    //   reintenta") con el detalle del error original.
    // - La key va EMBEBIDA en el APK (uso personal, sin telemetría; ver
    //   `lib.rs`). Una consulta no se cancela al cerrar el panel: el hilo
    //   termina solo y el resultado se descarta al soltar el receptor.
    // - El hilo de fondo evita bloquear el hilo de UI durante la red
    //   (AGENTS.md §4.6): la generación puede tardar varios segundos.

    /// "IA": lanza la consulta a la IA (Gemini con imagen / Groq solo-texto)
    /// en un hilo de fondo y abre el panel en fase "preguntando…". Si no hay
    /// ni texto ni imagen aprovechable avisa "no text" y no abre el panel
    /// (mismo comportamiento que Copiar; con imagen — PDF escaneado — sí
    /// abre: la imagen es la fuente principal). El texto y la imagen se
    /// capturan ANTES de cerrar el menú (`sel_text` y
    /// `sel_image_png_base64`).
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
            "ask_ai: {} chars, image {} B PNG -> text Groq ({}), image Gemini ({})",
            text.chars().count(),
            image.as_ref().map_or(0, String::len),
            crate::GROQ_MODEL,
            crate::GEMINI_MODEL
        );
        // Panel en fase Asking ("preguntando…") y hilo de fondo con la
        // llamada HTTP: el UI nunca espera por la red.
        self.ai_text = "preguntando…".to_string();
        self.ai_phase = AiPhase::Asking;
        self.rebuild_ai_panel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = match image {
                // Con imagen (ecuación/gráfico): Gemini con `explain_image`;
                // el prompt es la instrucción + el texto extraído como
                // contexto adicional (puede ser "" en un PDF escaneado — la
                // imagen es la fuente principal). El modelo de visión de
                // Groq está retirado (403), por eso la imagen va a Gemini.
                Some(b64) => {
                    let client = pdf_core::ai::GeminiClient::with_model(
                        crate::GOOGLE_API_KEY,
                        crate::GEMINI_MODEL,
                    );
                    let mut prompt =
                        "Explica de forma clara y concisa lo que se ve en la imagen (ecuación, gráfico o texto)."
                            .to_string();
                    if !text.is_empty() {
                        prompt.push_str("\n\nTexto extraído de la página (contexto adicional):\n");
                        prompt.push_str(&text);
                    }
                    match client.explain_image(&prompt, &b64) {
                        Ok(answer) => Ok(answer),
                        Err(first_err) => {
                            // Reintento ÚNICO tras ~1 s: Gemini devuelve
                            // 503/busy en picos de carga y el segundo intento
                            // suele pasar. El sleep va en el hilo de fondo
                            // (el hilo de UI nunca se bloquea).
                            std::thread::sleep(std::time::Duration::from_secs(1));
                            match client.explain_image(&prompt, &b64) {
                                Ok(answer) => Ok(answer),
                                Err(second_err) => Err(pdf_core::AiError::Http {
                                    status: 503,
                                    body: format!(
                                        "modelo ocupado, reintenta — error original: {first_err}; tras reintento: {second_err}"
                                    ),
                                }),
                            }
                        }
                    }
                }
                // Sin imagen: el chat solo-texto de Groq de siempre.
                None => {
                    let client = pdf_core::ai::GroqClient::with_model(
                        crate::GROQ_API_KEY,
                        crate::GROQ_MODEL,
                    );
                    let system = "Eres un asistente de estudio. Explica de forma clara y concisa el texto que te dan.";
                    client.chat(system, &text)
                }
            };
            // El error también viaja por el canal (`AiError` implementa
            // Display): el hilo de UI decide si es respuesta o error.
            let _ = tx.send(result);
        });
        self.ai_rx = Some(rx);
        self.redraw();
    }

    /// Aplica el resultado del hilo de IA al panel (fase Answer/Error) y
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
    /// long-press del dedo en el documento (modo selección) o aviso breve
    /// visible. En reposo el poll bloquea sin gastar batería.
    #[allow(dead_code)] // el bucle usa `has_window()` (timeout fijo)
    pub(crate) fn needs_tick(&mut self) -> bool {
        self.repaint
            // Buscador con teclado: mientras el IME esté abierto, `tick`
            // hace polling del texto tecleado (re-filtra la rejilla en vivo).
            || self.ime_active
            || self.tool_gesture.is_some()
            || self.sheet_anim
            || (self.chrome_visible && self.chrome_hide_at.is_some())
            || self.thumbs_pending()
            || self.toast.is_some()
            || self.gesture.press_pending()
            // Transición al abrir un libro: el tick expira el fade.
            || self.lib_fade.is_some()
            // Consulta de IA en vuelo: `tick` sondea el canal del hilo de
            // fondo (sin esto el poll bloquearía y la respuesta tardaría en
            // aparecer hasta el siguiente evento de input).
            || self.ai_rx.is_some()
            // Render asíncrono en vuelo (zoom sharp / cambio de página):
            // sondear hasta que el worker termine.
            || self.render_rx.is_some()
            // Debounce del pinch (F3.2): los dedos llevan quietos < 350 ms
            // o el render nítido aún no llegó — `tick` decide el disparo.
            || self.last_pinch_move.is_some()
    }

    // ---------------------------------------------------------------------
    // Sheet de ajustes (panel deslizante desde arriba, 2026-08-XX)
    // ---------------------------------------------------------------------

    /// ¿Animación del sheet en vuelo? La consulta global de trabajo diferido
    /// es `needs_tick` (incluye esta señal + portadas + long-press + aviso
    /// breve); `sheet_animating` ya no se usa desde `lib` (2026-08-XX).
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
    /// copiar el overlay del sheet sobre el frame, NO
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
    /// Abre/cierra el sheet CON animación (tap en la barra superior del
    /// chrome): `tick` anima el progreso y `blit` materializa el bitmap al
    /// hacerse visible. Sustituye al antiguo gesto pull-down (eliminado).
    pub(crate) fn toggle_sheet(&mut self) {
        if self.sheet_progress > 0.0 || self.sheet_open {
            self.hide_sheet();
        } else {
            self.sheet_open = true;
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

    /// Muestra el chrome del visor (barra superior e inferior) y programa el auto-hide a 4.5s.
    pub(crate) fn show_chrome(&mut self) {
        self.chrome_visible = true;
        self.chrome_hide_at = Some(Instant::now() + Duration::from_millis(4500));
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.redraw();
    }

    /// Oculta el chrome del visor.
    pub(crate) fn hide_chrome(&mut self) {
        self.chrome_visible = false;
        self.chrome_hide_at = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.redraw();
    }

    /// Alterna la visibilidad del chrome del visor.
    pub(crate) fn toggle_chrome(&mut self) {
        if self.chrome_visible {
            self.hide_chrome();
        } else {
            self.show_chrome();
        }
    }

    /// Resetea el temporizador de auto-ocultado del chrome (al tocar un control).
    pub(crate) fn touch_chrome(&mut self) {
        if self.chrome_visible {
            self.chrome_hide_at = Some(Instant::now() + Duration::from_millis(4500));
        }
    }

    /// Tick del bucle de eventos (timeout ~16 ms): avanza la animación del
    /// sheet, detecta el long-press del dedo en el documento (entra en modo
    /// selección), expira el aviso breve (toast) y renderiza un lote de
    /// portadas pendientes de la biblioteca. `lib::android_main` lo invoca en
    /// los eventos Wake/Timeout, que solo ocurren mientras `needs_tick()` (sin
    /// despertar el loop en reposo).
    pub(crate) fn tick(&mut self, app: &AndroidApp) {
        // Buscador con teclado: recoger lo tecleado y re-filtrar la rejilla
        // (el IME escribe en un EditText invisible; ver `jni::ime_text`).
        self.poll_ime_query(app);
        // Auto-ocultar chrome del visor tras ~2.5 s
        if self.chrome_visible
            && let Some(hide_at) = self.chrome_hide_at
            && Instant::now() >= hide_at
        {
            self.hide_chrome();
        }
        // Long-press: si el dedo lleva quieto > `LONG_PRESS_MS` en el área de
        // página (sin sheet), `input::tick_gestures` entra en modo selección.
        crate::input::tick_gestures(self, app);
        // Render ASÍNCRONO (zoom sharp / cambio de página): aplica los
        // bitmaps que ya llegaron — el UI nunca se congela esperándolos.
        self.poll_render();
        // Debounce del pinch (F3.2): 350 ms de quietud con los dedos en
        // pantalla y el bitmap a otro zoom (> 5%) → render nítido SIN
        // esperar a soltar. Con el actor persistente el lote en vuelo no
        // bloquea (preemption por seq); `render_in_flight_for` evita
        // duplicar el render al mismo nivel.
        if let Some(t) = self.last_pinch_move
            && t.elapsed() >= Duration::from_millis(350)
            && (self.rendered_zoom - self.zoom).abs() / self.zoom.max(1e-4) > 0.05
            && !self.render_in_flight_for(self.zoom)
        {
            self.launch_render(vec![self.page], self.zoom, false);
        }
        // Resultado del hilo de IA (si hay una consulta en vuelo): `try_recv`
        // sondea el canal SIN bloquear; al llegar el mensaje se actualiza el
        // panel (fase Answer/Error) y se libera el receptor. Mientras tanto el
        // poll con timeout se mantiene vivo vía `needs_tick` (ai_rx.is_some).
        if let Some(rx) = self.ai_rx.as_ref() {
            let outcome = {
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
        // Transición al abrir un libro: expirada → se libera (el visor ya
        // muestra solo la página). Durante la transición, cada tick redibuja
        // con un alfa decreciente (el fade se anima en `blit`).
        if let Some((started, _)) = self.lib_fade {
            if started.elapsed().as_secs_f32() >= LIB_FADE_MS {
                self.lib_fade = None;
                self.redraw();
            } else {
                self.redraw(); // un frame más de la transición
            }
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
            }
            self.redraw();
        }
        if self.mode == UiMode::Library && self.pump_thumbs(app) {
            if let Some((mut band, origin)) = self.lib_band.take() {
                // Pegar las portadas nuevas sobre la banda EXISTENTE (memcpy
                // por celda): sin re-render del canvas (antes un rebuild
                // completo de la pantalla por cada lote de portadas).
                paste_lib_thumbs(self, &mut band, origin);
                self.lib_band = Some((band, origin));
                self.splice_band_rows();
                self.redraw();
            } else {
                // Sin banda aún: el primer rebuild la crea con las portadas.
                self.list_dirty = true;
                self.redraw();
            }
        }
    }

    // ---------------------------------------------------------------------
    // Portadas de la biblioteca (perezosas, bajo demanda — ver `thumbs`)
    // ---------------------------------------------------------------------

    /// ¿Hay portadas pendientes entre las celdas VISIBLES de la biblioteca
    /// (carousel de "Continue Reading" + rejilla)? El bucle de eventos la
    /// usa para mantener el poll con timeout mientras `pump_thumbs` tiene
    /// trabajo.
    pub(crate) fn thumbs_pending(&mut self) -> bool {
        if self.mode != UiMode::Library || self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        // Carousel de Continue Reading (clave = ruta local), solo si está
        // visible.
        if self.lib_cont_visible() {
            // Clonar las rutas: `thumbs.get` (mutable) no convive con el
            // préstamo de `lib_continue_reading()` (inmutable).
            let paths: Vec<String> = self
                .lib_continue_reading()
                .iter()
                .map(|b| b.path.clone())
                .collect();
            for path in paths {
                if self.thumbs.get(&path).is_none() && !self.thumb_failed.contains(&path) {
                    return true;
                }
            }
        }
        // Portadas de la rejilla / lista (clave = content:// URI), solo filas visibles.
        if !self.hide_covers {
            if self.is_grid() {
                let cols = self.effective_grid_cols();
                let (row0, rows) = self.lib_visible_grid_rows();
                for row in row0..row0 + rows {
                    for col in 0..cols {
                        let Some(uri) = self.grid_entry_at(row, col).map(|e| e.uri.clone()) else {
                            continue;
                        };
                        if self.thumbs.get(&uri).is_none() && !self.thumb_failed.contains(&uri) {
                            return true;
                        }
                    }
                }
            } else {
                let (idx0, count) = self.lib_visible_list_rows();
                for idx in idx0..idx0 + count {
                    let Some(uri) = self.list_entry_at(idx).map(|e| e.uri.clone()) else {
                        continue;
                    };
                    if self.thumbs.get(&uri).is_none() && !self.thumb_failed.contains(&uri) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// ¿La fila del carousel de "Continue Reading" está dentro de la ventana
    /// (con el scroll vertical actual)? Solo entonces se renderizan sus
    /// portadas.
    fn lib_cont_visible(&self) -> bool {
        if !self.lib_has_cont() {
            return false;
        }
        let content_y0 =
            lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some()) as f32;
        let block_h = lib_cont_block_h(self.win_w, self.win_h, true);
        let top = content_y0 - self.lib_scroll;
        let bottom = top + block_h;
        bottom > content_y0 && top < self.win_h as f32
    }

    /// Rango de filas de la rejilla VISIBLES (o a punto de serlo) con el
    /// scroll vertical actual: (primera fila, nº de filas con 1 de margen de
    /// prefetch por abajo). Coords compartidas con el render y el tap.
    pub(crate) fn lib_visible_grid_rows(&self) -> (usize, usize) {
        let content_y0 =
            lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some()) as f32;
        let grid_y0_screen =
            content_y0 + lib_grid_y0(self.win_w, self.win_h, self.lib_has_cont()) - self.lib_scroll;
        if grid_y0_screen >= self.win_h as f32 {
            return (0, 0); // la rejilla está por debajo de la ventana
        }
        let cols = self.effective_grid_cols();
        let ch = grid_cell_h(self.win_w, cols, self.cover_size);
        let row0 = ((content_y0 - grid_y0_screen) / ch).max(0.0) as usize;
        let below = ((self.win_h as f32 - grid_y0_screen) / ch).ceil().max(0.0) as usize;
        (row0, below + 1)
    }

    /// Rango de filas de la lista VISIBLES con el scroll vertical actual.
    pub(crate) fn lib_visible_list_rows(&self) -> (usize, usize) {
        let content_y0 =
            lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some()) as f32;
        let grid_y0_screen =
            content_y0 + lib_grid_y0(self.win_w, self.win_h, self.lib_has_cont()) - self.lib_scroll;
        if grid_y0_screen >= self.win_h as f32 {
            return (0, 0);
        }
        let rh = list_row_h(self.win_h, self.cover_size) + list_row_gap();
        let row0 = ((content_y0 - grid_y0_screen) / rh).max(0.0) as usize;
        let below = ((self.win_h as f32 - grid_y0_screen) / rh).ceil().max(0.0) as usize;
        (row0, below + 1)
    }

    fn ensure_thumb_worker(&mut self) {
        if self.thumb_worker.is_none() {
            let (worker, rx) = crate::thumbs::ThumbWorker::spawn();
            self.thumb_worker = Some(worker);
            self.thumb_rx = Some(rx);
        }
    }

    /// Recibe las portadas terminadas por el worker de fondo (`try_recv`) y
    /// encola peticiones para las celdas visibles que aún no están en caché.
    /// Cero I/O síncrono en el hilo UI (Fase E1).
    fn pump_thumbs(&mut self, _app: &AndroidApp) -> bool {
        if self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        self.ensure_thumb_worker();

        let mut changed = false;

        // 1. Drenar portadas listas desde el canal MPSC en segundo plano
        if let Some(rx) = self.thumb_rx.as_ref() {
            while let Ok(msg) = rx.try_recv() {
                match msg.bitmap {
                    Some(bmp) => {
                        info!("thumb cached in background: {}", msg.key);
                        self.thumbs.insert(msg.key, bmp);
                        changed = true;
                    }
                    None => {
                        warn!("thumb failed in background: {}", msg.key);
                        self.thumb_failed.insert(msg.key);
                    }
                }
            }
        }

        // 2. Recolectar celdas visibles que aún no están en caché ni fallaron
        let mut needed = Vec::new();

        if self.lib_cont_visible() {
            let cont_paths: Vec<String> = self
                .lib_continue_reading()
                .iter()
                .map(|b| b.path.clone())
                .collect();
            for path in cont_paths {
                if self.thumbs.peek(&path).is_none()
                    && !self.thumb_failed.contains(&path)
                    && !needed.contains(&path)
                {
                    needed.push(path);
                }
            }
        }

        if !self.hide_covers {
            if self.is_grid() {
                let cols = self.effective_grid_cols();
                let (row0, rows) = self.lib_visible_grid_rows();
                for row in row0..row0 + rows {
                    for col in 0..cols {
                        if let Some(uri) = self.grid_entry_at(row, col).map(|e| e.uri.clone())
                            && self.thumbs.peek(&uri).is_none()
                            && !self.thumb_failed.contains(&uri)
                            && !needed.contains(&uri)
                        {
                            needed.push(uri);
                        }
                    }
                }
            } else {
                let (idx0, count) = self.lib_visible_list_rows();
                for idx in idx0..idx0 + count {
                    if let Some(uri) = self.list_entry_at(idx).map(|e| e.uri.clone())
                        && self.thumbs.peek(&uri).is_none()
                        && !self.thumb_failed.contains(&uri)
                        && !needed.contains(&uri)
                    {
                        needed.push(uri);
                    }
                }
            }
        }

        if !needed.is_empty()
            && let Some(w) = self.thumb_worker.as_ref()
        {
            w.request(needed);
        }

        changed
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
        // F3.2: todo Move del pinch actualiza el instante del debounce.
        self.last_pinch_move = Some(Instant::now());
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        if (self.zoom - zoom).abs() < 1e-4 {
            return;
        }
        // De vuelta a zoom 1.0 (PINCH_MIN): la página vuelve a su posición
        // natural (centrada en X, alineada arriba en Y). BUG: sin este
        // reset, un pan residual de un zoom previo dejaba la vista
        // "colgada" en un offset (p. ej. el tercio inferior de la página,
        // pan_y ≈ −400 px con la página cover más alta que la ventana) sin
        // forma de corregirlo — la app NO tiene gesto de pan (el arrastre
        // se eliminó), así que el único "home" posible es el centrado. El
        // clamp de X ya fuerza ~0 (la página a cover casi iguala la
        // ventana), pero en Y el rango de `clamp_pan` admite offsets
        // grandes; por eso el reset es explícito en ambos ejes.
        if zoom <= PINCH_MIN + 1e-4 {
            self.pan_x = 0.0;
            self.pan_y = 0.0;
            self.zoom = zoom;
            if self.window.is_some() {
                self.blit();
            }
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
        // Redraw de solo blit: reutiliza los bitmaps de la caché (escala de la
        // última renderización, `rendered_zoom`); `blit` escala la página
        // actual con el zoom nuevo. El render y el reescalado de ventana los
        // cubre el bucle de eventos (RedrawNeeded/WindowResized) si hicieran
        // falta.
        // EARLY SHARP: si el vecino-más-cercano ya se ve borroso
        // (blit_zoom > 1.6), lanzar en el worker un render de la página a un
        // nivel 2^ceil(log2 zoom) ≤ 2× (clamp pequeño y rápido): al llegar,
        // `poll_render` fija `rendered_zoom` y el pinch sigue pero con el
        // bitmap NUEVO (nitidez progresiva sin esperar al soltar y SIN
        // renders gigantes por cada Move). Solo si no hay otro lote en vuelo.
        let blit_zoom = self.zoom / self.rendered_zoom.max(1e-4);
        if blit_zoom > 1.6 && !self.render_in_flight_for(self.zoom) {
            self.launch_render(vec![self.page], self.zoom, true);
        }
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
        // Fin del gesto (F3.2): el debounce deja de aplicar — aquí mismo se
        // lanza el render final nítido.
        self.last_pinch_move = None;
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        // Sin cambio REAL de zoom (p. ej. dos dedos tocando sin Moves, o un
        // pinch-in que se quedó en el mínimo): no re-renderizar — evita
        // limpiar la caché y pagar el render (~20-40 ms) por un no-op.
        if (self.zoom - zoom).abs() < 1e-4 && (self.rendered_zoom - zoom).abs() < 1e-4 {
            return;
        }
        let (dw, dh) = self.page_doc_size_px(self.page);
        if dw > 0.0 && dh > 0.0 {
            if zoom <= PINCH_MIN + 1e-4 {
                // De vuelta a zoom 1.0: posición natural (ver `set_zoom_fast`).
                self.pan_x = 0.0;
                self.pan_y = 0.0;
            } else {
                // Transición fast→sharp SIN salto: durante el pinch el bitmap
                // VIEJO se dibuja con tamaño `round(bmp.width × zoom/rendered_zoom)`
                // px (vecino-más-cercano) y tras el re-render el NUEVO a
                // `round(dw × zoom)` px 1:1 — la diferencia (≤ 1 px, por el
                // redondeo de píxeles del render) desplazaría el borde
                // izquierdo de la página al soltar. Corregimos el pan para
                // que el borde DIBUJADO quede en el mismo píxel: en `blit`,
                // `dx = round((win − w)/2 + pan)`, así que para que el nuevo
                // dx iguale al dibujado en fast basta
                // `pan_nuevo = dx_fast − (win − w_nuevo)/2` (la corrección es
                // solo en X: en Y el borde superior es `dy = round(pan_y)`,
                // independiente del tamaño del bitmap).
                let old_blit = self.zoom / self.rendered_zoom.max(1e-4);
                let old_w = match self.cache.peek(self.page) {
                    Some(b) => b.width as f32 * old_blit,
                    None => dw * zoom, // sin bitmap (defensa): sin corrección
                };
                let new_w = (dw as f64 * zoom as f64).round() as f32;
                let dx_fast = ((self.win_w as f32 - old_w) / 2.0 + self.pan_x).round();
                self.pan_x = dx_fast - (self.win_w as f32 - new_w) / 2.0;
            }
            // El pan de anclaje YA es el del zoom final (último
            // `set_zoom_fast`); el re-render a la nueva escala
            // (`rendered_zoom = zoom`) mantiene el mismo mapeo
            // documento→pantalla (la escala efectiva `doc·zoom` no cambia),
            // así que el punto bajo los dedos permanece fijo al soltar.
            // Reclamp del pan al zoom FINAL (por si `set_zoom_sharp` llega
            // sin un `set_zoom_fast` previo, p. ej. pinch sin Moves):
            // `clamp_pan` solo depende de `page = doc·zoom`, así que un pan
            // ya clampeado no cambia y el rango cubre la ventana entera
            // también tras el re-render (`rendered_zoom = zoom` → la escala
            // efectiva `doc·zoom` no cambia).
            self.pan_x = Self::clamp_pan(self.pan_x, dw * zoom, self.win_w as f32, false);
            self.pan_y = Self::clamp_pan(self.pan_y, dh * zoom, self.win_h as f32, true);
        }
        self.zoom = zoom;
        // SHARP ASÍNCRONO: NO se limpia la caché ni se re-renderiza en el
        // hilo UI (antes: `cache.clear() + redraw()` congelaba 20-400 ms). El
        // bitmap VIEJO sigue en caché y el blit lo escala a `zoom/rendered_zoom`
        // (preview vecino-más-cercano); el worker renderiza SOLO la página
        // actual (un render por lote: las vecinas se renderizan al navegar,
        // evita 3 renders gigantes de golpe) y, al llegar, `poll_render` fija
        // `rendered_zoom` y repintea (1:1 nítido). El presupuesto de píxeles
        // (launch_render) acota el bitmap para no petar la RAM.
        self.launch_render(vec![self.page], zoom, false);
        info!("zoom {:.3}", self.zoom);
        self.save_state();
        self.mark_repaint();
    }

    /// Establece un tema específico. Invalida cachés y persiste estado.
    pub(crate) fn set_theme(&mut self, theme: crate::theme::AppTheme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.dark = self.theme.is_dark();
        self.page_badge = None;
        self.sheet_bitmap = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.lib_header = None;
        self.lib_band = None;
        self.toast_bitmap = None;
        self.sel_menu = None;
        info!("theme set to {:?} (dark: {})", self.theme, self.dark);
        self.save_state();
        self.redraw();
    }

    /// Cicla el tema activo (DefaultLight → SepiaLight → DefaultDark → SepiaDark → DefaultLight).
    /// Invalida las caches de bitmaps y persiste el nuevo tema.
    pub(crate) fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
        self.dark = self.theme.is_dark();
        self.page_badge = None;
        self.sheet_bitmap = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.lib_header = None;
        self.lib_band = None;
        self.toast_bitmap = None;
        self.sel_menu = None;
        info!("theme cycled to {:?} (dark: {})", self.theme, self.dark);
        self.save_state();
        self.redraw();
    }

    /// Alterna el modo oscuro (cicla al tema siguiente).
    #[allow(dead_code)]
    pub(crate) fn toggle_dark(&mut self) {
        self.cycle_theme();
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
        let Some(sidecar) = self.annot_sidecar.clone() else {
            return;
        };
        let annotations = self.annotations.clone();
        let len = annotations.len();
        // Guardado en hilo de fondo: con 276+ trazos, la serialización JSON +
        // SQLite bloqueaba el hilo UI ~50-200ms al soltar, causando
        // "parpadeo"/ANR al escribir encima de tinta existente. El hilo de
        // fondo evita el bloqueo; best-effort (un fallo solo se loguea).
        std::thread::spawn(move || match AnnotationStore::open(&sidecar) {
            Ok(store) => match store.save(&annotations) {
                Ok(()) => info!("annotations saved ({} total) to {}", len, sidecar.display()),
                Err(e) => error!("annotations save {}: {e}", sidecar.display()),
            },
            Err(e) => error!("annotations open {}: {e}", sidecar.display()),
        });
    }

    /// [B] Botón UP del boli: alterna el modo (Ink ↔ Highlight), muestra el
    /// toast con el modo NUEVO y lo persiste en `tool_state.json`. Lo llama
    /// `input` desde `MotionAction::ButtonPress` (funciona con el boli en el
    /// aire Y en contacto; algunos bolis no emiten ButtonPress — ver la
    /// fuente de verdad doble en `input.rs`).
    pub(crate) fn toggle_pen_mode(&mut self) {
        self.pen_mode = match self.pen_mode {
            PenMode::Ink => PenMode::Highlight,
            PenMode::Highlight => PenMode::Ink,
        };
        self.persist_pen_mode();
        self.mode_badge = None; // el indicador de esquina muestra el modo nuevo
        self.show_toast(self.pen_mode.label());
    }

    /// Persiste el modo del boli en `tool_state.json` (campo "mode"). NO
    /// toca `persist.rs` (fuera de alcance de esta tarea): lee el JSON
    /// completo como `Value` (respetando lo que escribe `persist` —
    /// ink_color/ink_width) y solo conserva/añade "mode".
    fn persist_pen_mode(&self) {
        let Some(dir) = self.internal_dir.as_deref() else {
            return;
        };
        let path = dir.join("tool_state.json");
        let mut v = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        v["mode"] = serde_json::json!(match self.pen_mode {
            PenMode::Ink => "Ink",
            PenMode::Highlight => "Highlight",
        });
        if let Ok(text) = serde_json::to_string_pretty(&v)
            && let Err(e) = fs::write(&path, text)
        {
            error!("persist pen_mode {}: {e}", path.display());
        }
    }

    /// Marca repintado pendiente (coalescing por vsync).
    fn mark_repaint(&mut self) {
        self.repaint = true;
    }

    /// Lanza el render ASÍNCRONO de `pages` a la escala `target_zoom`
    /// (factor de zoom — el worker calcula `cover × target_zoom` con su
    /// propio documento). `clamp_level` limita el render a un nivel 2^x
    /// cercano (early sharp durante el pinch: evita renders gigantes por
    /// cada Move; el sharp final usa `clamp_level=false` para nitidez
    /// máxima). Reemplaza cualquier lote anterior (seq++).
    ///
    /// **Presupuesto de píxeles**: el worker NUNCA produce un bitmap mayor
    /// que la caché (`CACHE_BYTE_BUDGET`, 48 MiB) — un render de la página a
    /// zoom alto (p. ej. 8800×11640 px = 400 MB) petaba la RAM de la tablet
    /// ("se queda pillada") y expulsaba toda la caché. La escala se reduce
    /// por mitades hasta caber; el `target_zoom` enviado refleja el zoom
    /// EFECTIVO (escala/cover) para que el blit quede 1:1.
    fn launch_render(&mut self, pages: Vec<u32>, target_zoom: f32, clamp_level: bool) {
        // Actor persistente (F3.1): sin worker (documento aún sin abrir del
        // todo) no hay a quién pedir — los llamantes previos al open no
        // lanzaban nada útil tampoco (el hilo efímero fallaba al abrir).
        let Some(worker) = self.render_worker.as_ref() else {
            return;
        };
        self.render_seq += 1;
        let seq = self.render_seq;
        let (tx, rx) = std::sync::mpsc::channel::<WorkerMsg>();
        self.render_rx = Some(rx);
        let (win_w, win_h) = (self.win_w, self.win_h);
        let _ = worker.tx.send(WorkerCmd::Render(WorkerReq {
            seq,
            pages,
            target_zoom,
            clamp_level,
            win_w,
            win_h,
            reply: tx,
        }));
    }

    /// ¿Hay un lote en vuelo para este `zoom`? (ventana ±50%: el sharp final
    /// y el early-sharp del pinch comparten objetivo aproximado).
    fn render_in_flight_for(&self, zoom: f32) -> bool {
        self.render_rx.is_some() && (self.rendered_zoom - zoom).abs() / zoom.max(1e-4) < 0.5
    }

    /// Arranca el worker actor de render con `path` como documento. Llamado
    /// UNA vez por documento (`open_pdf_at` y el intent "abrir con"): el
    /// hilo abre SU PROPIO `MupdfDocument` (MuPDF no es Send) y lo retiene
    /// hasta `Stop`. Un worker anterior se detiene antes.
    fn start_render_worker(&mut self, path: &str) {
        self.stop_render_worker();
        let (tx, rx) = std::sync::mpsc::channel::<WorkerCmd>();
        let path = path.to_string();
        let handle = std::thread::Builder::new()
            .name("render-worker".into())
            .spawn(move || {
                let doc = match MupdfEngine::new().and_then(|e| e.open(std::path::Path::new(&path)))
                {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("render-worker: open {path}: {e}");
                        return;
                    }
                };
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        WorkerCmd::Stop => break,
                        WorkerCmd::Render(req) => {
                            render_worker_req(&doc, &rx, req);
                        }
                    }
                }
            })
            .ok();
        self.render_worker = Some(RenderWorker { tx, handle });
    }

    /// Detiene el worker actor (si existe): envía `Stop` y hace `join()`.
    /// Llamado al cambiar de documento y desde `Drop for Reader`.
    fn stop_render_worker(&mut self) {
        if let Some(w) = self.render_worker.take() {
            let _ = w.tx.send(WorkerCmd::Stop);
            if let Some(handle) = w.handle {
                let _ = handle.join();
            }
        }
        // Los resultados en vuelo de lotes antiguos caducan solos:
        // `poll_render` ya descarta por `seq != render_seq`.
    }

    /// Sondeo del worker de render (desde `tick`): aplica los bitmaps
    /// recibidos a la caché y, cuando llega la página actual, fija
    /// `rendered_zoom` al nivel del lote y repintea (el blit pasa de preview
    /// escalado a 1:1 nítido). Los lotes obsoletos (seq viejo) se descartan.
    /// Al desconectarse el worker (terminado), libera el canal.
    fn poll_render(&mut self) {
        loop {
            let Some(rx) = self.render_rx.as_ref() else {
                return;
            };
            match rx.try_recv() {
                Ok(msg) => {
                    if msg.seq != self.render_seq {
                        continue; // lote obsoleto: descartar
                    }
                    self.cache.insert(msg.page, msg.bitmap);
                    if msg.page == self.page {
                        self.rendered_zoom = msg.target_zoom;
                        self.fallback_page = None;
                        self.mark_repaint();
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.render_rx = None;
                    break;
                }
            }
        }
    }

    /// Pan con DEDO (herramienta activa, modo mano): devuelve el pan de
    /// partida al bajar el dedo (mismo formato que `set_pan`).
    pub(crate) fn begin_pan(&self) -> (f32, f32) {
        (self.pan_x, self.pan_y)
    }

    /// Fija el pan (px) del documento y marca repintado — el blit por vsync
    /// aplicará el desplazamiento del frame al `ANativeWindow`.
    pub(crate) fn set_pan(&mut self, x: f32, y: f32) {
        self.pan_x = x;
        self.pan_y = y;
        self.mark_repaint();
    }

    /// ¿Debe ignorarse el táctil del dedo/palma por haber escrito hace poco con el stylus?
    pub(crate) fn should_ignore_touch(&self) -> bool {
        if let Some(t) = self.last_stylus_time {
            t.elapsed().as_millis() < crate::STYLUS_IGNORE_MS as u128
        } else {
            false
        }
    }

    /// ¿Hay un blit pendiente por coalescer? (el bucle principal lo llama
    /// una vez por iteración tras procesar eventos).
    pub(crate) fn take_repaint(&mut self) -> bool {
        let r = self.repaint;
        self.repaint = false;
        r
    }

    /// ¿El bucle debe correr con timeout (~16 ms) para redibujar? (gesto en
    /// curso u otro trabajo diferido).
    #[allow(dead_code)] // apoyo: el flujo real usa `needs_tick(&mut self)`
    pub(crate) fn needs_repaint(&self) -> bool {
        self.repaint
            || self.tool_gesture.is_some()
            || self.sheet_anim
            || self.ai_rx.is_some()
            || self.lib_fade.is_some()
    }

    /// ¿Tenemos ventana (ANativeWindow activo)? El bucle principal usa un
    /// poll con timeout (16 ms) mientras haya ventana — el poll bloqueante
    /// puede perder los toques del visor (ver `lib.rs`).
    pub(crate) fn has_window(&self) -> bool {
        self.window.is_some()
    }

    /// Gesto de herramienta: el Down convierte el punto de pantalla a
    /// coordenadas de página y crea el `ToolGesture` en la página actual.
    /// El blit pasa a usar el frame compuesto + capa temporal (sin
    /// re-renderizar ni re-blitear la página) mientras el gesto dure.
    pub(crate) fn begin_tool_gesture(&mut self, sx: f32, sy: f32, tool: ToolKind) -> bool {
        if tool == ToolKind::Navigate {
            return false;
        }
        // B3: el resaltador pre-ordena los spans en el Down (una vez por
        // gesto) para el preview por present y el cálculo al soltar.
        let want_hl = tool == ToolKind::Highlight;
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return false;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        // Fase 1 USI: ancla temporal (event_time del Down, la fija `input`)
        // y presión inicial. Sin ancla (driver sin timestamps) → t0=0 y las
        // muestras degradan a la ventana sin Δt real (el predictor usa el
        // clamp de dt mínimo); sin presión → 0.5 (w_base neutro).
        let t0 = self.pending_t0_ns.take().unwrap_or(0);
        let pressure = self.pending_pressure.take().unwrap_or(0.5);
        self.tool_gesture = Some(ToolGesture::new(
            self.page,
            tool,
            pt,
            0.0,
            pressure,
            self.ink_width,
        ));
        // El Down ES t=0: el campo se fija tras crear el gesto.
        if let Some(g) = self.tool_gesture.as_mut() {
            g.times_ms[0] = 0.0;
        }
        if want_hl {
            let cached = self.text_cache.get(self.page);
            if let Some(pt) = cached {
                let mut v = pt.spans.clone();
                pdf_core::sort_spans_by_y(&mut v);
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.hl_spans = v;
                }
            }
        }
        self.gesture_t0_ns = t0;
        // FASE 2: la tinta es geometría GPU por frame — sin frame base que
        // clonar ni stamping. El siguiente blit pinta página + gesto.
        if self.window.is_some() {
            self.mark_repaint();
        }
        true
    }

    /// Gesto de herramienta: cada Move añade el punto (boli) o actualiza el
    /// rect (resaltador) y MARCA repintar — el blit real ocurre UNA vez por
    /// vsync en el bucle principal (coalescing de eventos, como Saber/Flutter):
    /// si bliteáramos por Move a 120 Hz, el BufferQueue de SurfaceFlinger
    /// (pantalla 60 Hz) bloquearía cada `unlock_and_post` ~16 ms (backpressure)
    /// → jitter/lag. Con un blit por vsync y dirty rect, el coste por frame es
    /// <1 ms y la latencia es ≤1 frame (16 ms).
    pub(crate) fn update_tool_gesture(&mut self, sx: f32, sy: f32, t_ms: f32, pressure: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        // La herramienta del gesto EN CURSO (la puso `input` según el modo
        // del boli); `self.tool` (barra) es irrelevante aquí.
        let Some(tool) = self.tool_gesture.as_ref().map(|g| g.tool) else {
            return;
        };
        // Actualizar tiempo de stylus para palm rejection por tiempo
        self.last_stylus_time = Some(std::time::Instant::now());
        match tool {
            ToolKind::Ink => {
                // Pipeline de modelado físico (google/ink-stroke-modeler):
                // 1. Sanitiza el evento (240 Hz, noise gate 0.2 pt).
                // 2. Simulación masa-resorte críticamente amortiguada (ζ = 1.0).
                // 3. Estimación y proyección cinemática Kalman a 25–30 ms.
                let Some(g) = self.tool_gesture.as_mut() else {
                    return;
                };
                let t_ns = self
                    .gesture_t0_ns
                    .saturating_add((t_ms as f64 * 1e6) as u64);
                let model_res = g.modeler.update(pt.0, pt.1, t_ns, pressure);
                g.predicted_pt = model_res.predicted_pt;
                let confirmed_pt = model_res.confirmed_pt;

                let Some(&last) = g.points.last() else {
                    return;
                };
                let n0 = g.points.len();
                g.push_with_pressure(confirmed_pt, t_ms, model_res.pressure);
                if g.points.len() == n0 {
                    return;
                }
                let mid = (
                    (last.0 + confirmed_pt.0) / 2.0,
                    (last.1 + confirmed_pt.1) / 2.0,
                );
                let prev_mid = g.prev_mid;
                // Muestrear en página la MISMA curva midpoint que dibuja el
                // present (una sola fuente de verdad para la polilínea).
                let a = prev_mid.unwrap_or(last);
                let steps = 6usize;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let om = 1.0 - t;
                    let q = if prev_mid.is_some() {
                        (
                            om * om * a.0 + 2.0 * om * t * last.0 + t * t * mid.0,
                            om * om * a.1 + 2.0 * om * t * last.1 + t * t * mid.1,
                        )
                    } else {
                        (a.0 + t * (mid.0 - a.0), a.1 + t * (mid.1 - a.1))
                    };
                    g.ink_pts.push(q);
                }
                g.prev_mid = Some(mid);
            }
            ToolKind::Highlight => {
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.set_cur(pt);
                }
            }
            ToolKind::Navigate => {}
        }
        self.mark_repaint();
    }

    /// Gesto de herramienta: al levantar el dedo convierte el gesto en una
    /// anotación GUARDADA (persistida en el sidecar):
    ///
    /// - **Boli**: la polilínea MUESTREADA de la curva midpoint (`ink_pts`,
    ///   lo estampado en vivo) simplificada con Douglas-Peucker fino →
    ///   `Stroke` con el grosor/color actuales. Cero pop: el frame no se
    ///   re-pinta. Un gesto sin arrastre (un toque) se descarta.
    /// - **Resaltador**: `pdf_core::highlight_under_gesture` selecciona las
    ///   líneas de texto bajo el trazo (extracción perezosa, solo ahora) y
    ///   crea el `Highlight` alineado al texto; "no text" si no hay líneas.
    ///
    /// El id nuevo se apunta en `session_ids` (historial de sesión).
    /// `(sx, sy)` = posición del Up (remate M_last→P_up en el boli; las
    /// muestras de history ya se estamparon por el drain previo, así que el
    /// hueco que cierra es solo el último tramo hasta el punto de soltar).
    pub(crate) fn end_tool_gesture(&mut self, _sx: f32, _sy: f32) {
        let Some(g) = self.tool_gesture.take() else {
            return;
        };
        // Actualizar tiempo para palm rejection: tras soltar, se ignora el táctil un margen
        if g.tool != ToolKind::Navigate {
            self.last_stylus_time = Some(std::time::Instant::now());
        }
        // Gesto degenerado (un toque sin arrastre): descartar silenciosamente.
        // El umbral está en px de PANTALLA (TOOL_MIN_PX, el recorrido mínimo
        // del dedo/lápiz); el bbox del gesto en página se convierte con la
        // escala efectiva del blit (cover × zoom).
        let scale = self
            .doc
            .as_ref()
            .and_then(|d| d.page_size(g.page).ok())
            .map(|(pw, ph)| initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom)
            .unwrap_or(1.0);
        let min_d_pt = crate::TOOL_MIN_PX / scale;
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &(x, y) in &g.points {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        if max_x - min_x < min_d_pt && max_y - min_y < min_d_pt {
            // Un toque sin arrastre: sin gesto, sin anotación. La tinta que
            // el present dibujó era solo geometría del frame — muere sola.
            if self.window.is_some() {
                self.mark_repaint();
            }
            return;
        }
        let mut g = g;
        match g.tool {
            ToolKind::Ink => {
                // REMATE M_last→P_up: asienta la masa virtual con StrokeEndPredictor
                // y tapering suave de presión sin "cero-pop".
                let end_res = g.modeler.end_stroke();
                let end_pt = end_res.confirmed_pt;
                if let Some(m_last) = g.prev_mid.or_else(|| g.ink_pts.last().copied())
                    && m_last != end_pt
                {
                    g.ink_pts.push(end_pt);
                }
                // CERO POP: lo estampado en vivo ES el trazo final. Se
                // persiste la polilínea MUESTREADA de la curva midpoint
                // (`ink_pts`) simplificada con Douglas-Peucker fino
                // (ε 0.35 pt ≈ 0.7 px a 2 px/pt: replay < 1 px del vivo —
                // invisible). Sin Catmull-Rom ni re-rasterizado: la tinta del
                // frame no se toca.
                let sampled = if g.ink_pts.len() >= 40 {
                    pdf_core::simplify_polyline(&g.ink_pts, 0.35)
                } else {
                    g.ink_pts.clone()
                };
                if let Some(s) = Stroke::new(sampled, self.ink_width, self.ink_color)
                    && let Some(id) = self.annotations.add(g.page as usize, Annotation::Stroke(s))
                {
                    self.session_ids.push(id);
                    self.save_annotations();
                    self.show_toast("ink");
                }
            }
            ToolKind::Highlight => {
                // El resaltador usa TODO el trazo (los puntos del gesto), no
                // solo ancla→cursor: un trazo curvo selecciona las líneas
                // bajo su bbox completo.
                // B3: camino indexado sobre los spans del Down (sin I/O en
                // el gesto); fallback a la vía clásica si no estaban cacheados.
                let gesture = Gesture::Points(g.points.clone());
                let hl = if g.hl_spans.is_empty() {
                    let spans = self
                        .doc
                        .as_ref()
                        .and_then(|d| self.text_cache.get_or_extract(d, g.page).ok())
                        .map(|t| t.spans.clone())
                        .unwrap_or_default();
                    pdf_core::highlight_under_gesture(&spans, &gesture, pdf_core::HIGHLIGHT_COLOR)
                } else {
                    pdf_core::highlight_under_gesture_sorted(
                        &g.hl_spans,
                        &gesture,
                        pdf_core::HIGHLIGHT_COLOR,
                    )
                };
                if let Some(hl) = hl {
                    if let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Highlight(hl))
                    {
                        self.session_ids.push(id);
                        self.save_annotations();
                        self.show_toast("highlighted");
                    }
                } else {
                    // Sin líneas de texto bajo el trazo (zona en blanco o PDF
                    // escaneado): el resaltador dibuja un RECT LIBRE con el
                    // bbox del trazo (altura de línea ~13 pt) — el boli
                    // SIEMPRE pinta algo en modo Highlighter.
                    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
                    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for &(x, y) in &g.points {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                    let line_h = 13.0f32;
                    let cy = (min_y + max_y) / 2.0 - line_h / 2.0;
                    let rect = pdf_core::Rect::new(min_x, cy, (max_x - min_x).max(1.0), line_h);
                    let hl = pdf_core::Highlight {
                        rects: vec![rect],
                        color: pdf_core::HIGHLIGHT_COLOR,
                    };
                    if let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Highlight(hl))
                    {
                        self.session_ids.push(id);
                        self.save_annotations();
                        self.show_toast("highlighted");
                    }
                }
            }
            ToolKind::Navigate => {}
        }
        // FASE 2: la tinta es geometría GPU — el próximo present pinta la
        // página con el Stroke ya persistido (sin frame base que restaurar).
        if self.window.is_some() {
            self.mark_repaint();
        }
    }

    /// Gesto de herramienta cancelado (segundo dedo, Cancel del sistema,
    /// ocultar la barra): descarta el trazo en curso sin crear anotación.
    pub(crate) fn cancel_tool_gesture(&mut self) {
        if self.tool_gesture.take().is_some() && self.window.is_some() {
            // El present GPU ya no dibuja el gesto: un frame normal.
            self.mark_repaint();
        }
    }

    // ---------------------------------------------------------------------
    // Borrado con el boli (botón DOWN mantenido + tocar el PDF): control
    // total SIN menús ([C] de la tarea). El erase nunca coexiste con un
    // gesto de tinta (`input` solo lo inicia si `tool_gesture` está libre) y
    // NO entra en el undo de sesión (`session_ids` intacto; decisión:
    // permanente).
    // ---------------------------------------------------------------------

    /// Comienza un gesto de borrado (Down del boli con el botón DOWN
    /// pulsado). Devuelve false si no se pudo iniciar (ya hay tinta en curso
    /// o el punto no cae en la página) — en ese caso `input` no entra en el
    /// modo Erase.
    pub(crate) fn begin_erase_gesture(&mut self, sx: f32, sy: f32) -> bool {
        // El modo ERASE nunca coexiste con `tool_gesture` de dibujo: si hay
        // tinta en curso, este Down no inicia borrado (el trazo sigue como
        // estaba).
        if self.tool_gesture.is_some() {
            return false;
        }
        if self.screen_to_page(sx, sy).is_none() {
            return false;
        }
        self.last_stylus_time = Some(std::time::Instant::now());
        self.erase_dirty = false;
        self.erase_last = None;
        self.erase_pt = Some((sx, sy));
        self.erase_r_px = self.eraser_radius_px();
        self.eraser_cursor = None;
        true
    }

    /// Radio del cursor de la goma en px (radio en puntos × escala efectiva).
    fn eraser_radius_px(&self) -> f32 {
        let scale = self
            .doc
            .as_ref()
            .and_then(|d| d.page_size(self.page).ok())
            .map(|(pw, ph)| initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom)
            .unwrap_or(1.0);
        ERASE_HIT_RADIUS_PT * scale
    }

    /// Arrastre de borrado: hit-test del punto (en coords de página) contra
    /// TODAS las anotaciones de la página actual — incluidas las de
    /// SESIONES ANTERIORES (el sidecar se carga en `load_annotations`). Cada
    /// anotación cruzada se elimina EN VIVO (`AnnotationSet::remove`): el
    /// frame de la página se invalida y el siguiente blit la recompone sin
    /// ella — desaparece bajo el boli.
    pub(crate) fn update_erase_gesture(&mut self, sx: f32, sy: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        // Cursor de la goma sigue al boli (el radio se calculó al empezar).
        self.erase_pt = Some((sx, sy));
        // Snapshot SOLO de IDS (sin clonar el `Annotation` completo): cada
        // id se resuelve contra el estado VIGENTE dentro del bucle (el set
        // puede mutar en iteraciones anteriores). Con history a 240 Hz (hasta
        // 16 muestras/evento) esto elimina el coste dominante del borrado:
        // antes se clonaba el set entero POR MUESTRA; ahora solo un Vec de
        // u64 por llamada.
        let snapshot: Vec<u64> = self
            .annotations
            .for_page(self.page as usize)
            .iter()
            .map(|a| a.id)
            .collect();
        let mut changed = false;
        for id in snapshot {
            // El kind se lee POR REFERENCIA en cada iteración (el set puede
            // haber mutado en las anteriores): CERO clones por muestra —
            // split_stroke/trim_highlight trabajan sobre &Stroke/&Highlight y
            // solo remove/add tocan el set (después del hit-test). Con 276
            // trazos y el boli tocando 0-2 por muestra, este bucle es memcpy
            // de ids + hit-tests, nada más.
            let Some(ann) = self
                .annotations
                .for_page(self.page as usize)
                .into_iter()
                .find(|a| a.id == id)
            else {
                continue; // anotación ya eliminada/repicada por el barrido
            };
            match &ann.kind {
                pdf_core::Annotation::Stroke(s) => {
                    // GOMA REAL sobre trazo: se recorta (parte la línea en
                    // trozos), no se elimina entera.
                    if let Some(parts) = pdf_core::annotations::split_stroke(
                        s,
                        pt,
                        ERASE_HIT_RADIUS_PT,
                        self.erase_last,
                    ) {
                        self.annotations.remove(id);
                        let mut kept = 0;
                        for part in parts {
                            if self
                                .annotations
                                .add(self.page as usize, pdf_core::Annotation::Stroke(part))
                                .is_some()
                            {
                                kept += 1;
                            }
                        }
                        info!("erase: stroke {id} -> {kept} piece(s)");
                        changed = true;
                    }
                }
                pdf_core::Annotation::Highlight(h) => {
                    // GOMA REAL sobre subrayado: se parte en rectos (con
                    // barrido: una goma rápida no salta la línea).
                    if let Some(rects) = pdf_core::annotations::trim_highlight(
                        h,
                        pt,
                        ERASE_HL_PAD_PT,
                        self.erase_last,
                    ) {
                        let color = h.color; // Copy: último uso del borrow del set
                        self.annotations.remove(id);
                        if !rects.is_empty() {
                            self.annotations.add(
                                self.page as usize,
                                pdf_core::Annotation::Highlight(pdf_core::Highlight {
                                    rects,
                                    color,
                                }),
                            );
                        }
                        info!("erase: highlight {id} trimmed");
                        changed = true;
                    }
                }
                pdf_core::Annotation::TextNote(_) => {}
            }
        }
        self.erase_last = Some(pt);
        if changed {
            self.erase_dirty = true;
            self.mark_repaint();
        }
    }

    /// Fin del borrado (Up o Cancel del sistema): persiste UNA sola vez si
    /// algo se eliminó (`store.save`, hilo de fondo).
    pub(crate) fn end_erase_gesture(&mut self) {
        self.last_stylus_time = Some(std::time::Instant::now());
        self.erase_pt = None;
        self.eraser_cursor = None;
        if self.erase_dirty {
            self.erase_dirty = false;
            self.save_annotations();
        }
    }

    /// Persiste la posición actual (ruta, página, zoom) + modo oscuro en
    /// `internal/state.json` (ver `persist`). Escritura *eager*: se llama en
    /// cada cambio de página, al soltar el pinch, al abrir un documento y al
    /// alternar el modo oscuro — un cierre inesperado no pierde la posición.
    ///
    /// Además actualiza el REGISTRO DE PROGRESO por libro
    /// (`internal/library.json`, ver `persist::BookProgress`): página actual,
    /// total de páginas y sello de última lectura. El registro se CREA la
    /// primera vez (added_unix) y se actualiza en cada apertura o cambio de
    /// página — de ahí se derivan "Page X of Y", la barra de progreso, el
    /// estado Reading/Finished y los sorts de "My Library" sin abrir el PDF.
    pub(crate) fn save_state(&mut self) {
        let path = self.doc_path.clone().unwrap_or_default();
        let state = crate::persist::ViewerState {
            path: path.clone(),
            page: self.page,
            zoom: self.zoom,
            dark: self.dark,
            theme: Some(self.theme),
            view_mode: self.view_mode,
            cover_fit: self.cover_fit,
            columns: self.columns,
            hide_covers: self.hide_covers,
            recent_shelf_enabled: self.recent_shelf_enabled,
            cover_size: self.cover_size,
            cover_progress: self.cover_progress,
        };
        crate::persist::save_state(self.internal_dir.as_deref(), &state);
        if self.mode == UiMode::Viewer && !path.is_empty() {
            let pages = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
            let now = crate::persist::unix_now();
            self.lib_books =
                crate::persist::touch_progress(&self.lib_books, &path, self.page, pages, now);
            crate::persist::save_progress(self.internal_dir.as_deref(), &self.lib_books);
        }
    }

    /// Abre un PDF por ruta (picker) y pasa al visor con la página 1.
    /// Devuelve false (y deja el estado intacto) si no se pudo abrir.
    pub(crate) fn open_pdf(&mut self, path: &str) -> bool {
        self.open_pdf_at(path, None)
    }

    /// Abre un PDF por ruta y pasa al visor; si `start_page` es Some, salta
    /// a esa página (la posición guardada de "Continue Reading"/la rejilla),
    /// si no a la página 1. Devuelve false (y deja el estado intacto) si no
    /// se pudo abrir.
    pub(crate) fn open_pdf_at(&mut self, path: &str, start_page: Option<u32>) -> bool {
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                return false;
            }
        };
        match engine.open(Path::new(path)) {
            Ok(doc) => {
                let pages = doc.page_count();
                info!("opened: {pages} pages");
                // Página de apertura: la guardada (reanudar lectura) o la 1.
                let page = match start_page {
                    Some(p) => p.min(pages.saturating_sub(1)),
                    None => 0,
                };
                self.doc = Some(doc);
                self.page = page;
                self.zoom = 1.0;
                self.rendered_zoom = 1.0;
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                self.pinch = None;
                self.bitmap = None;
                // Transición al abrir: snapshot de la pantalla de lista
                // (biblioteca: cabecera+banda; picker: bitmap) que el visor
                // funde sobre la página los primeros `LIB_FADE_MS`.
                let snapshot = match self.mode {
                    UiMode::Library => compose_library_snapshot(self),
                    UiMode::Picker => self.bitmap.clone(),
                    UiMode::Viewer => None,
                };
                if let Some(s) = snapshot {
                    self.lib_fade = Some((Instant::now(), s));
                }
                self.lib_header = None; // biblioteca fuera: liberar planos
                self.lib_band = None;
                self.lib_row_dirty = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.mode = UiMode::Viewer;
                // EGL: venimos de Library/Picker sin surface (ver
                // `enter_library`); recrearla ya para el primer present.
                if let (Some(g), Some(win)) = (self.gpu.as_mut(), self.window.as_ref())
                    && !g.has_surface()
                {
                    g.recreate_surface(win);
                }
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.start_render_worker(path);
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera (libera también el frame)
                self.clear_selection(); // selección del documento anterior: fuera
                self.close_ai_panel(); // panel de IA del documento anterior: fuera
                self.thumbs.clear(); // portadas de otra biblioteca: no sirven
                self.thumb_failed.clear();
                self.list_dirty = true;
                self.list_drag = None;
                // Herramientas de anotación: reseteo a la navegación limpia
                // (sin herramienta activa, sin gesto en curso
                // y SIN histórico de sesión del documento anterior — el undo
                // es por sesión, decisión documentada en `session_ids`).
                self.tool = ToolKind::Navigate;
                self.tool_gesture = None;
                self.session_ids.clear();
                // Fase B1: texto del documento nuevo (el del anterior no
                // sirve). Prefetch de la página visible +-2: el primer
                // resaltado de esas páginas será un HIT (sin stext en el
                // hilo UI). El resto se extrae perezoso con `get_or_extract`
                // (1-2 ms) y queda cacheado para repeticiones y para la IA
                // (Fase D).
                self.text_cache.clear();
                if let Some(doc) = self.doc.as_ref() {
                    let base = page.saturating_sub(2);
                    let pages: Vec<u32> = (base..(page + 3).min(pages)).collect();
                    let _n = self.text_cache.prefetch(doc, &pages);
                }
                // Anotaciones del documento (sidecar; set vacío si no existe
                // o está corrupto — nunca impide abrir el PDF).
                self.load_annotations(path);
                self.redraw();
                // Nuevo documento: actualizar la posición persistida (el
                // modo oscuro es una preferencia global y se conserva).
                self.save_state();
                // Y la lista de RECIENTES de la biblioteca (dedup por ruta,
                // más reciente primero, máx. 10 — persist::push_recent).
                self.touch_recent(path);
                true
            }
            Err(e) => {
                error!("cannot open {path}: {e}");
                false
            }
        }
    }

    /// Entra en la biblioteca (botón "← Library" del sheet del visor):
    /// reconstruye la biblioteca CURADA desde `internal/library.json` y deja
    /// de mostrar la página. El campo de búsqueda arranca CERRADO. Vacía →
    /// EMPTY STATE ("Tu biblioteca está vacía" + botón "Añadir PDF").
    pub(crate) fn enter_library(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Library;
        // EGL: la surface del visor hace fallar `ANativeWindow_lock` de la
        // biblioteca; soltarla aquí (el contexto y los FBOs se recrean al
        // volver al visor; ver defensa en `blit`).
        if let Some(g) = self.gpu.as_mut() {
            g.drop_surface();
        }
        self.list_scroll = 0;
        self.lib_search_open = false;
        self.list_dirty = true;
        self.bitmap = None; // lista del picker (no se usa en la biblioteca)
        self.lib_header = None; // zona fija: se re-renderiza en el rebuild
        self.lib_band = None; // banda de contenido: idem
        self.lib_row_dirty = None;
        // La caché de páginas del visor (48 MiB) no sirve en la biblioteca:
        // liberarla aquí evita RSS doble (páginas + zona fija + banda +
        // portadas) y se re-renderiza al volver a un PDF.
        self.cache.clear();
        // Re-cargar los registros persistidos (recents + progreso): la
        // biblioteca debe reflejar cualquier lectura hecha en otra sesión o
        // proceso (Continue Reading / barras de progreso / sort-filtros).
        self.recents = persist::load_recents(self.internal_dir.as_deref());
        self.lib_books = persist::load_progress(self.internal_dir.as_deref());
        self.sheet_hide_now(); // fuera del visor: el sheet no pinta en biblioteca
        self.clear_selection(); // selección del visor: fuera (no pinta en biblioteca)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        // Herramientas del visor: fuera (no pinta en biblioteca).
        self.tool = ToolKind::Navigate;
        self.tool_gesture = None;
        self.session_ids.clear();
        self.lib_close_ime(app);
        self.reload_curated_library(app);
    }

    /// Abre el picker interno (PDFs de los directorios de la app; el fallback
    /// histórico). Con la biblioteca curada no hay ruta de UI hacia él (las
    /// altas van por `add_book`); se conserva el método por si una fase
    /// futura reintroduce la entrada.
    #[allow(dead_code)]
    pub(crate) fn open_picker(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Picker;
        // EGL: igual que en `enter_library` (el picker también blitea por
        // `ANativeWindow_lock`).
        if let Some(g) = self.gpu.as_mut() {
            g.drop_surface();
        }
        self.pdf_list = scan_pdfs(app);
        self.list_scroll = 0;
        self.status = None;
        self.list_dirty = true;
        self.bitmap = None;
        self.sheet_hide_now();
        self.clear_selection(); // selección del visor: fuera (no pinta en el picker)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        self.redraw();
    }

    /// Vuelve del picker al visor sin cambiar el documento (botón Back).
    pub(crate) fn exit_picker(&mut self) {
        self.mode = UiMode::Viewer;
        self.list_dirty = true;
        self.bitmap = None; // lista del picker (las páginas siguen en la caché)
        self.lib_header = None; // biblioteca fuera: liberar planos cedeados
        self.lib_band = None;
        self.lib_row_dirty = None;
        self.list_drag = None;
        self.redraw();
    }

    /// Reconstruye la BIBLIOTECA CURADA desde `internal/library.json` — SIN
    /// consultar MediaStore: una entrada `LibraryEntry` por registro cuyo PDF
    /// sigue existiendo en disco (`uri` = RUTA LOCAL, `folder` = "PDF"; las
    /// portadas y aperturas van por ruta). Antes de listar ejecuta la
    /// MIGRACIÓN one-shot de instalaciones antiguas (`migrate_internal_pdfs`).
    /// Vacía → empty state con "Añadir PDF".
    pub(crate) fn reload_curated_library(&mut self, _app: &AndroidApp) {
        self.mode = UiMode::Library;
        self.picker_kind = PickerKind::Files; // el selector temporal queda fuera
        // Re-leer los registros persistidos: la biblioteca debe reflejar
        // cualquier alta/lectura hecha en otro punto del flujo.
        self.lib_books = persist::load_progress(self.internal_dir.as_deref());
        self.migrate_internal_pdfs();
        // Solo los registros cuyo fichero SIGUE existiendo: un registro
        // huérfano (borrado a mano o evictado) no pinta ninguna celda.
        let mut entries = Vec::new();
        for b in &self.lib_books {
            let p = Path::new(&b.path);
            if !p.is_file() {
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            entries.push(LibraryEntry {
                name,
                folder: "PDF".to_string(),
                uri: b.path.clone(),
                size: 0,
            });
        }
        info!(
            "curated library: {} of {} records with file on disk",
            entries.len(),
            self.lib_books.len()
        );
        self.library_list = entries;
        // La rejilla curada NO requiere permiso de almacenamiento (lee solo
        // ficheros propios); quien lo necesita es el SELECTOR de añadir, y
        // ese lo re-comprueba `query_media_store` al invocarse. Con true, el
        // empty state ofrece "Añadir PDF" en vez de "Conceder acceso".
        self.permission_granted = true;
        // Datos nuevos: scroll al origen (vertical y horizontales) y lista
        // filtrada recalculada; el sort activo ordena por added/read.
        self.list_scroll = 0;
        self.lib_scroll = 0.0;
        self.lib_carousel_x = 0.0;
        self.lib_folders_x = 0.0;
        self.lib_letters_x = 0.0;
        self.lib_sort_x = 0.0;
        self.lib_filter_x = 0.0;
        self.refresh_lib_filtered();
        self.list_dirty = true;
        self.bitmap = None;
        self.lib_header = None; // zona fija: se re-renderiza en el rebuild
        self.lib_band = None;
        self.lib_row_dirty = None;
        self.redraw();
    }

    /// Porcentaje leído de un libro (0.0-1.0) según la ruta de su fichero.
    #[allow(dead_code)]
    pub(crate) fn book_progress_pct(&self, path: &str) -> Option<f32> {
        crate::persist::progress_for(&self.lib_books, path).map(|b| b.pct())
    }

    /// Vacía la biblioteca curada (elimina library.json y los PDFs internos).
    pub(crate) fn clear_library(&mut self, app: &AndroidApp) {
        if let Some(dir) = self.internal_dir.as_deref() {
            let lib_file = crate::persist::library_path(dir);
            if lib_file.exists()
                && let Err(e) = fs::remove_file(&lib_file)
            {
                log::warn!("failed to remove library.json: {e}");
            }
            let pdf_dir = dir.join("pdfs");
            if let Ok(entries) = fs::read_dir(&pdf_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }
        self.lib_books.clear();
        self.library_list.clear();
        self.lib_filtered.clear();
        self.reload_curated_library(app);
        self.settings_menu_open = false;
        self.show_toast("Library cleared");
        self.list_dirty = true;
        self.redraw();
    }

    /// MIGRACIÓN one-shot de instalaciones antiguas: versiones previas
    /// copiaban los PDFs abiertos a `internal/pdfs/` sin registrarlos en
    /// `library.json`. Si el registro está VACÍO y la carpeta NO, se importa
    /// cada PDF como libro (added = ahora). Idempotente: tras guardar, el
    /// registro deja de estar vacío y no vuelve a ejecutarse.
    fn migrate_internal_pdfs(&mut self) {
        if !self.lib_books.is_empty() {
            return;
        }
        let Some(dir) = self.internal_dir.as_deref() else {
            return;
        };
        let pdfs_dir = dir.join("pdfs");
        let Ok(rd) = fs::read_dir(&pdfs_dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")))
            .collect();
        if paths.is_empty() {
            return;
        }
        paths.sort();
        let now = persist::unix_now();
        let books: Vec<BookProgress> = paths
            .iter()
            .map(|p| BookProgress {
                path: p.display().to_string(),
                page: 0,
                // Contar páginas exigiría abrir CADA PDF durante el arranque
                // (launch time); el total real queda sellado en la primera
                // apertura (`save_state` → `touch_progress`).
                page_count: 0,
                last_read_unix: now,
                added_unix: now,
            })
            .collect();
        info!(
            "migration: imported {} PDFs from {} into library.json",
            books.len(),
            pdfs_dir.display()
        );
        self.lib_books = books;
        persist::save_progress(self.internal_dir.as_deref(), &self.lib_books);
    }

    /// Nº de filas de la lista del picker según su variante (el fallback lee
    /// `pdf_list`; el selector de añadir, la TEMPORAL `select_list`). Lo
    /// consumen el clamp de scroll y el tap.
    pub(crate) fn picker_len(&self) -> usize {
        match self.picker_kind {
            PickerKind::Files => self.pdf_list.len(),
            // El selector de añadir navega por CARPETAS: las filas visibles
            // son la vista actual del gestor (carpetas + PDFs), no la lista
            // plana de MediaStore.
            PickerKind::Select => self.picker_rows().len(),
        }
    }

    /// Construye la vista ACTUAL del gestor de archivos del selector de
    /// añadir: carpetas primero (únicas, del nivel actual) y luego los PDFs
    /// directamente contenidos en `sel_dir`. Orden alfabético
    /// (case-insensitive).
    pub(crate) fn picker_rows(&self) -> Vec<PickRow> {
        if self.picker_kind != PickerKind::Select {
            return Vec::new();
        }
        let cur = self.sel_dir.join("/");
        let mut folders: Vec<String> = Vec::new();
        let mut files: Vec<usize> = Vec::new();
        for (idx, e) in self.select_list.iter().enumerate() {
            let f = e.folder.trim_end_matches('/');
            if self.sel_dir.is_empty() {
                // Raíz: carpeta = primer segmento; PDF = sin carpeta.
                if f.is_empty() {
                    files.push(idx);
                } else if let Some(seg) = f.split('/').next()
                    && !seg.is_empty()
                    && !folders.iter().any(|x| x == seg)
                {
                    folders.push(seg.to_string());
                }
                continue;
            }
            // Nivel: PDF directo (== cur) o subcarpeta (siguiente segmento).
            if f == cur {
                files.push(idx);
            } else if let Some(rest) = f.strip_prefix(&format!("{cur}/"))
                && let Some(seg) = rest.split('/').next()
                && !seg.is_empty()
                && !folders.iter().any(|x| x == seg)
            {
                folders.push(seg.to_string());
            }
        }
        folders.sort_by_key(|f| f.to_lowercase());
        files.sort_by_key(|&i| self.select_list[i].name.to_lowercase());
        let mut rows = Vec::with_capacity(folders.len() + files.len());
        rows.extend(folders.into_iter().map(PickRow::Folder));
        rows.extend(files.into_iter().map(PickRow::File));
        rows
    }

    /// Entra en la carpeta `name` del gestor (push al breadcrumb).
    pub(crate) fn picker_sel_enter(&mut self, name: &str) {
        self.sel_dir.push(name.to_string());
        self.list_scroll = 0;
        self.list_dirty = true;
        self.redraw();
    }

    /// Sube un nivel del gestor de archivos; en la raíz no hace nada.
    pub(crate) fn picker_sel_up(&mut self) {
        if self.sel_dir.pop().is_some() {
            self.list_scroll = 0;
            self.list_dirty = true;
            self.redraw();
        }
    }

    /// ¿El selector de añadir muestra la barra de breadcrumb (dentro de una
    /// carpeta)? Añade una fila fija entre la cabecera y la lista.
    pub(crate) fn picker_has_crumb(&self) -> bool {
        self.picker_kind == PickerKind::Select && !self.sel_dir.is_empty()
    }

    /// Nº real de filas visibles del picker (resta la barra de breadcrumb
    /// del selector de añadir cuando está visible).
    pub(crate) fn picker_visible(&self) -> usize {
        let crumbs = if self.picker_has_crumb() { 1 } else { 0 };
        picker_visible_rows(self.win_h, self.status.is_some()).saturating_sub(crumbs)
    }

    /// Nº de columnas efectivas de la rejilla (Auto -> 3, Manual -> columns clamp 1..4).
    pub(crate) fn effective_grid_cols(&self) -> usize {
        if self.auto_columns {
            3
        } else {
            self.columns.clamp(1, 4) as usize
        }
    }

    /// Nº de filas de celdas de la rejilla de la biblioteca con
    /// el filtro actual aplicado (`lib_filtered`).
    #[allow(dead_code)]
    pub(crate) fn grid_total_rows(&self) -> usize {
        let cols = self.effective_grid_cols();
        self.lib_filtered.len().div_ceil(cols)
    }

    /// Entrada de la rejilla en la fila `row` (0-based) y columna `col`
    /// (0..cols) — resolución sobre la lista FILTRADA (`lib_filtered`).
    /// None si la celda está fuera de rango.
    pub(crate) fn grid_entry_at(&self, row: usize, col: usize) -> Option<&LibraryEntry> {
        let cols = self.effective_grid_cols();
        let idx = row.checked_mul(cols)?.checked_add(col)?;
        self.lib_filtered
            .get(idx)
            .and_then(|&i| self.library_list.get(i))
    }

    /// Entrada de la lista en el índice `idx` de la lista FILTRADA.
    pub(crate) fn list_entry_at(&self, idx: usize) -> Option<&LibraryEntry> {
        self.lib_filtered
            .get(idx)
            .and_then(|&i| self.library_list.get(i))
    }

    // ---------------------------------------------------------------------
    // Biblioteca rediseñada: filtros SIN teclado + recientes (2026-08-XX)
    // ---------------------------------------------------------------------
    //
    // Búsqueda: el enunciado pedía un campo de texto con el teclado del
    // sistema vía JNI (InputMethodManager + InputConnection). VERIFICADO en el
    // código de android-activity 0.6.1 (el backend `native-activity` de este
    // proyecto): `NativeActivity::set_text_input_state` es un NOP
    // ("Unsupported") y `InputEvent::TextEvent` SOLO lo produce el backend
    // game-activity (GameTextInput, que exige una Activity Java compilada —
    // y cargo-apk/ndk-build no compilan fuentes Java, ver cabecera de lib.rs).
    // Sin un `onCreateInputConnection` que entregue `commitText`, el teclado
    // blando NO puede mandar texto a una NativeActivity. Por eso el filtro es
    // SIN teclado: letra inicial (A-Z / #) + carpeta, vía los chips de
    // `lib_chips` (ver el worker_done de la sesión).

    /// ¿La entrada pasa el filtro de BÚSQUEDA activo (carpeta + letra inicial)?
    fn entry_passes(&self, e: &LibraryEntry) -> bool {
        // Buscador CON TECLADO: subcadena case-insensitive sobre el título.
        if !self.lib_query.is_empty() {
            let q = self.lib_query.to_lowercase();
            if !e.name.to_lowercase().contains(&q) {
                return false;
            }
        }
        // Filtros legacy por letra/carpeta (sin UI desde 2026-08-25).
        if let Some(f) = &self.lib_folder
            && !e.folder.eq_ignore_ascii_case(f)
        {
            return false;
        }
        if let Some(l) = self.lib_letter {
            let first = e
                .name
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or('#');
            let ok = if l == '#' {
                !first.is_ascii_alphabetic()
            } else {
                first == l
            };
            if !ok {
                return false;
            }
        }
        true
    }

    /// Ruta local del PDF de la biblioteca (la copia en `internal/pdfs/`): la
    /// clave del registro de progreso (`library.json`), de `recents.json` y
    /// de `state.json`. Debe coincidir con la que usa `open_library_entry`.
    pub(crate) fn entry_path(&self, e: &LibraryEntry) -> String {
        let dir = self.internal_dir.as_deref().unwrap_or(Path::new(""));
        dir.join("pdfs")
            .join(sanitize_pdf_name(&e.name))
            .display()
            .to_string()
    }

    /// Clave de orden "recientemente añadido" de la entrada `i` de
    /// `library_list` (added_unix; sin registro → i64::MIN, al final).
    fn sort_added_key(&self, i: usize) -> i64 {
        let e = &self.library_list[i];
        persist::progress_for(&self.lib_books, &self.entry_path(e))
            .map(|p| p.added_unix)
            .unwrap_or(i64::MIN)
    }

    /// Clave de orden "recientemente leído" (last_read_unix; sin registro →
    /// i64::MIN, al final).
    fn sort_read_key(&self, i: usize) -> i64 {
        let e = &self.library_list[i];
        persist::progress_for(&self.lib_books, &self.entry_path(e))
            .map(|p| p.last_read_unix)
            .unwrap_or(i64::MIN)
    }

    /// Reconstruye la caché `lib_filtered` (índices de `library_list`):
    /// filtra por BÚSQUEDA (carpeta + letra) y por ESTADO
    /// (Reading/Finished/Unread), y ORDENA por el sort activo (`lib_sort`).
    /// Se llama al cambiar filtro, sort o al re-consultar MediaStore; sin
    /// filtros equivale a todas las entradas en orden de MediaStore (por
    /// carpeta, luego nombre).
    fn refresh_lib_filtered(&mut self) {
        let mut idxs: Vec<usize> = self
            .library_list
            .iter()
            .enumerate()
            .filter(|(_, e)| self.entry_passes(e))
            .map(|(i, _)| i)
            .collect();
        // Filtro de ESTADO (derivado del registro de progreso).
        if let Some(s) = self.lib_status {
            idxs.retain(|&i| {
                let e = &self.library_list[i];
                book_status(persist::progress_for(&self.lib_books, &self.entry_path(e))) == s
            });
        }
        match self.lib_sort {
            LibSort::Title => idxs.sort_by(|&a, &b| {
                self.library_list[a]
                    .name
                    .to_lowercase()
                    .cmp(&self.library_list[b].name.to_lowercase())
            }),
            LibSort::Author => idxs.sort_by(|&a, &b| {
                entry_author(&self.library_list[a])
                    .to_lowercase()
                    .cmp(&entry_author(&self.library_list[b]).to_lowercase())
                    .then_with(|| {
                        self.library_list[a]
                            .name
                            .to_lowercase()
                            .cmp(&self.library_list[b].name.to_lowercase())
                    })
            }),
            LibSort::Progress => {
                let mut keyed: Vec<(usize, f32)> = idxs
                    .iter()
                    .map(|&i| {
                        let e = &self.library_list[i];
                        let pct = persist::progress_for(&self.lib_books, &self.entry_path(e))
                            .map(|p| p.pct())
                            .unwrap_or(0.0);
                        (i, pct)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| {
                            self.library_list[a.0]
                                .name
                                .to_lowercase()
                                .cmp(&self.library_list[b.0].name.to_lowercase())
                        })
                });
                idxs = keyed.into_iter().map(|(i, _)| i).collect();
            }
            LibSort::RecentlyAdded | LibSort::RecentlyRead => {
                // Precomputar la clave (added/read) una vez por entrada: evita
                // reconstruir la ruta local en cada comparación del sort.
                let mut keyed: Vec<(usize, i64)> = idxs
                    .iter()
                    .map(|&i| {
                        let k = if self.lib_sort == LibSort::RecentlyAdded {
                            self.sort_added_key(i)
                        } else {
                            self.sort_read_key(i)
                        };
                        (i, k)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    b.1.cmp(&a.1).then_with(|| {
                        self.library_list[a.0]
                            .name
                            .to_lowercase()
                            .cmp(&self.library_list[b.0].name.to_lowercase())
                    })
                });
                idxs = keyed.into_iter().map(|(i, _)| i).collect();
            }
        }
        if self.group_by == LibraryGroupBy::Author && self.lib_sort != LibSort::Author {
            idxs.sort_by(|&a, &b| {
                entry_author(&self.library_list[a])
                    .to_lowercase()
                    .cmp(&entry_author(&self.library_list[b]).to_lowercase())
            });
        }
        self.lib_filtered = idxs;
    }

    /// Aplica un cambio de filtro/sort: recalcula la lista, clampa el scroll
    /// y re-renderiza.
    pub(crate) fn apply_filter(&mut self) {
        self.refresh_lib_filtered();
        let max_v = self.lib_max_scroll();
        if self.lib_scroll > max_v {
            self.lib_scroll = max_v;
        }
        self.list_dirty = true;
        self.redraw();
    }

    /// Fija el filtro de letra inicial del panel de búsqueda (None = todas;
    /// chip "All"). Al elegir, el panel se cierra: el campo de búsqueda
    /// muestra el resumen del filtro activo (ver `draw::search_summary`).
    pub(crate) fn lib_set_letter(&mut self, letter: Option<char>) {
        if self.lib_letter != letter {
            self.lib_letter = letter;
            self.lib_search_open = false;
            self.apply_filter();
        }
    }

    /// Fija el filtro de carpeta del panel de búsqueda (None = todas; chip
    /// "All"). Al elegir, el panel se cierra (el campo muestra el resumen).
    pub(crate) fn lib_set_folder(&mut self, folder: Option<String>) {
        if self.lib_folder != folder {
            self.lib_folder = folder;
            self.lib_search_open = false;
            self.apply_filter();
        }
    }

    /// Fija el ORDEN de "My Library" (chips de sort: Recently Added /
    /// Recently Read / Title / Author).
    pub(crate) fn lib_set_sort(&mut self, sort: LibSort) {
        if self.lib_sort != sort {
            self.lib_sort = sort;
            self.apply_filter();
        }
    }

    /// Fija el filtro de ESTADO de "My Library" (None = All; chips de
    /// filter: Reading / Finished / Unread). También decide si "Continue
    /// Reading" se muestra (solo All/Reading, ver `lib_continue_reading`).
    pub(crate) fn lib_set_status(&mut self, status: Option<BookStatus>) {
        if self.lib_status != status {
            self.lib_status = status;
            self.apply_filter();
        }
    }

    /// Recientes que pasan el filtro de LETRA (la carpeta no aplica a los
    /// recientes: son rutas locales, sin RELATIVE_PATH).
    pub(crate) fn lib_recents(&self) -> Vec<&RecentEntry> {
        let Some(l) = self.lib_letter else {
            return self.recents.iter().collect();
        };
        self.recents
            .iter()
            .filter(|r| {
                let first = r
                    .name
                    .chars()
                    .next()
                    .map(|c| c.to_ascii_uppercase())
                    .unwrap_or('#');
                if l == '#' {
                    !first.is_ascii_alphabetic()
                } else {
                    first == l
                }
            })
            .collect()
    }

    /// Carpetas distintas de la biblioteca (orden de MediaStore, dedup
    /// case-insensitive) para la fila de chips de carpetas del panel de
    /// búsqueda.
    pub(crate) fn lib_folders(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.library_list {
            if e.folder.is_empty() {
                continue;
            }
            if !out.iter().any(|f| f.eq_ignore_ascii_case(&e.folder)) {
                out.push(e.folder.clone());
            }
        }
        out
    }

    /// Libros de "Continue Reading" (carousel destacado): los RECIENTES
    /// abiertos no terminados, con su progreso persistido (page, page_count,
    /// %). Respeta el filtro de letra (búsqueda) y el de estado de forma
    /// trivial: con All o Reading la sección se muestra (los libros ya son
    /// Reading por construcción); con Finished/Unread se oculta entera (el
    /// filtro de estado solo tiene sentido para la rejilla). Orden:
    /// recencia (recents.json, más reciente primero).
    pub(crate) fn lib_continue_reading(&self) -> Vec<ContinueBook> {
        if let Some(s) = self.lib_status
            && s != BookStatus::Reading
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for r in self.lib_recents() {
            let Some(p) = persist::progress_for(&self.lib_books, &r.path) else {
                continue; // abierto antes de existir el registro: sin datos
            };
            if p.is_finished() {
                continue;
            }
            out.push(ContinueBook {
                path: r.path.clone(),
                name: r.name.clone(),
                author: self.author_for_name(&r.name),
                page: p.page,
                page_count: p.page_count,
                pct: p.pct(),
            });
        }
        out
    }

    /// ¿Hay libros de "Continue Reading"? (la sección se OCULTA si no).
    /// Versión BARATA sin construir la lista (la llaman las funciones de
    /// geometría por celda, p. ej. `lib_grid_cell_rect`): con `any` suele
    /// parar en el primer reciente (el más reciente es Reading casi siempre),
    /// sin alocar ni mirar el autor.
    ///
    /// Biblioteca MINIMALISTA (estilo Readest): la sección "Continue
    /// Reading"/Recientes está OCULTA por diseño — la biblioteca es solo
    /// rejilla + buscador. Devuelve SIEMPRE `false`; el resto del código
    /// (geometría, tap, drag, pump de portadas) sigue referenciándola, así
    /// que todo colapsa a alto 0 / sin datos sin tocar nada más.
    pub(crate) fn lib_has_cont(&self) -> bool {
        if !self.recent_shelf_enabled {
            return false;
        }
        if let Some(s) = self.lib_status
            && s != BookStatus::Reading
        {
            return false;
        }
        self.recents.iter().any(|r| {
            persist::progress_for(&self.lib_books, &r.path)
                .map(|p| !p.is_finished())
                .unwrap_or(true)
        })
    }

    /// ¿La biblioteca se muestra en REJILLA (vs lista)?
    pub(crate) fn is_grid(&self) -> bool {
        self.view_mode == LibraryViewMode::Grid
    }

    /// Autor de un libro por NOMBRE de fichero: busca la entrada de
    /// MediaStore con el mismo nombre (la copia en `internal/pdfs/` conserva
    /// el DISPLAY_NAME) y deriva el autor de su carpeta; si no está en
    /// MediaStore (picker / "abrir con") → "PDF".
    fn author_for_name(&self, name: &str) -> String {
        self.library_list
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .map(entry_author)
            .unwrap_or_else(|| "PDF".to_string())
    }

    /// Alto total (px) del contenido scrolleable de la biblioteca.
    pub(crate) fn lib_content_h(&self) -> f32 {
        let win_w = self.win_w;
        let win_h = self.win_h;
        let has_cont = self.lib_has_cont();
        let grid_y0 = lib_grid_y0(win_w, win_h, has_cont);
        let count = self.lib_filtered.len();
        if self.is_grid() {
            let cols = self.effective_grid_cols();
            let rows = count.div_ceil(cols);
            let gap = grid_gap(win_w);
            grid_y0 + rows as f32 * (grid_cell_h(win_w, cols, self.cover_size) + gap) + 40.0
        } else {
            let gap = list_row_gap();
            grid_y0 + count as f32 * (list_row_h(win_h, self.cover_size) + gap) + 40.0
        }
    }

    /// Scroll vertical máximo (px) del contenido de la biblioteca.
    pub(crate) fn lib_max_scroll(&self) -> f32 {
        let viewport = (self.win_h
            - lib_content_y0(self.win_h, self.lib_search_open, self.status.is_some()))
            as f32;
        (self.lib_content_h() - viewport).max(0.0)
    }

    /// Scroll horizontal máximo (px) del carousel de "Continue Reading".
    pub(crate) fn lib_cont_max_x(&self) -> f32 {
        let n = self.lib_continue_reading().len();
        if n == 0 {
            return 0.0;
        }
        let last_right = lib_cont_card_x(self.win_w, self.win_h, n - 1)
            + lib_cont_card_w(self.win_w, self.win_h);
        (last_right + grid_pad(self.win_w) - self.win_w as f32).max(0.0)
    }

    /// Scroll horizontal máximo (px) de la fila de chips `row` del panel de
    /// búsqueda (0 = letras, 1 = carpetas).
    pub(crate) fn lib_chips_max_x(&self, row: usize) -> f32 {
        (lib_chips_row_w(self, row) - self.win_w as f32).max(0.0)
    }

    /// Scroll horizontal máximo (px) de la fila de organización `row`
    /// (0 = sort, 1 = filter).
    pub(crate) fn lib_org_max_x(&self, row: usize) -> f32 {
        (lib_org_row_w(self, row) - self.win_w as f32).max(0.0)
    }

    /// Registra un PDF abierto en la lista de recientes (persistida en
    /// `internal/recents.json`; ver `persist`): dedup por ruta, más reciente
    /// primero, máx. `RECENTS_MAX`. Se llama desde `open_pdf` y desde el
    /// "abrir con" del arranque (que no pasa por `open_pdf`).
    fn touch_recent(&mut self, path: &str) {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.recents = persist::push_recent(&self.recents, path.to_string(), name);
        persist::save_recents(self.internal_dir.as_deref(), &self.recents);
    }

    /// Entra en la biblioteca con el campo de búsqueda ABIERTO y los
    /// filtros limpios: es el botón "Search" del sheet — sin teclado, la
    /// búsqueda ES el panel de chips (letra/carpeta) del campo de búsqueda,
    /// así que "buscar" equivale a saltar a la biblioteca con el panel
    /// desplegado.
    pub(crate) fn enter_library_search(&mut self, app: &AndroidApp) {
        self.lib_letter = None;
        self.lib_folder = None;
        self.enter_library(app);
        self.lib_search_open = true;
        self.list_dirty = true;
        self.redraw();
    }

    /// "＋ Añadir" (cabecera) / "Añadir PDF" (empty state): consulta
    /// MediaStore en una LISTA TEMPORAL (`select_list`; NUNCA
    /// `library_list`) y abre el selector (`UiMode::Picker` +
    /// `PickerKind::Select`, título "Selecciona PDF") con TODOS los PDFs del
    /// sistema para elegir cuál curar. La biblioteca no cambia hasta que el
    /// usuario toca un PDF (`add_selected`).
    pub(crate) fn add_book(&mut self, app: &AndroidApp) {
        self.lib_close_ime(app);
        let scan = query_media_store(app, self.sdk_int);
        self.permission_granted = scan.permission_granted;
        self.select_list = scan.entries;
        self.sel_dir = Vec::new();
        self.picker_kind = PickerKind::Select;
        self.mode = UiMode::Picker;
        self.list_scroll = 0;
        self.status = if !self.permission_granted {
            Some("All files access not granted — grant it in system Settings".to_string())
        } else if let Some(e) = scan.error {
            Some(format!("MediaStore error: {e}"))
        } else if self.select_list.is_empty() {
            Some("No PDFs found on the device".to_string())
        } else {
            None
        };
        info!("add picker: {} PDFs in MediaStore", self.select_list.len());
        self.sheet_hide_now();
        self.clear_selection(); // selección del visor: fuera (no pinta aquí)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        self.bitmap = None;
        self.lib_header = None; // biblioteca fuera: liberar planos cedeados
        self.lib_band = None;
        self.lib_row_dirty = None;
        self.list_dirty = true;
        self.redraw();
    }

    /// "Reescanear" del selector de añadir: re-consulta MediaStore y refresca
    /// SOLO la lista temporal (`select_list`); la biblioteca curada queda
    /// intacta.
    pub(crate) fn rescan_select(&mut self, app: &AndroidApp) {
        let scan = query_media_store(app, self.sdk_int);
        self.permission_granted = scan.permission_granted;
        self.select_list = scan.entries;
        self.sel_dir = Vec::new();
        self.list_scroll = 0;
        self.status = if !self.permission_granted {
            Some("All files access not granted — grant it in system Settings".to_string())
        } else if let Some(e) = scan.error {
            Some(format!("MediaStore error: {e}"))
        } else if self.select_list.is_empty() {
            Some("No PDFs found on the device".to_string())
        } else {
            None
        };
        info!("rescan select: {} PDFs", self.select_list.len());
        self.list_dirty = true;
        self.redraw();
    }

    /// "Atrás" del selector de añadir: descarta la lista temporal y vuelve a
    /// la biblioteca curada SIN ningún cambio.
    pub(crate) fn cancel_add(&mut self, app: &AndroidApp) {
        self.select_list = Vec::new();
        self.reload_curated_library(app);
    }

    /// Confirmación del selector: copia el PDF elegido a `internal/pdfs/`
    /// (nombre saneado), cuenta páginas abriéndolo con MuPDF, crea su
    /// registro de progreso (`touch_progress`), aplica el TOPE `LIBRARY_MAX`
    /// con evicción LRU (`enforce_library_limit`: borra fichero + portada
    /// cacheada de cada expulsado), guarda `library.json` y reconstruye la
    /// biblioteca curada. Toast si hubo expulsión.
    pub(crate) fn add_selected(&mut self, app: &AndroidApp, index: usize) {
        let Some(entry) = self.select_list.get(index).cloned() else {
            return;
        };
        let Some(dir) = app.internal_data_path() else {
            error!("add selected: internal_data_path unavailable");
            return;
        };
        let pdfs_dir = dir.join("pdfs");
        if let Err(e) = fs::create_dir_all(&pdfs_dir) {
            error!("add selected: create_dir_all {}: {e}", pdfs_dir.display());
            return;
        }
        let dest = pdfs_dir.join(sanitize_pdf_name(&entry.name));
        match read_content_uri_bytes(app, &entry.uri) {
            Some(bytes) => {
                if let Err(e) = fs::write(&dest, &bytes) {
                    error!("add selected: write {}: {e}", dest.display());
                    self.status = Some(format!("Cannot copy {}", entry.name));
                    self.list_dirty = true;
                    self.redraw();
                    return;
                }
            }
            None => {
                error!("add selected: cannot read {}", entry.uri);
                self.status = Some(format!("Cannot read {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        }
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                self.status = Some(format!("Cannot open {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        };
        let page_count = match engine.open(&dest) {
            Ok(doc) => doc.page_count(),
            Err(e) => {
                error!("add selected: cannot open {}: {e}", dest.display());
                self.status = Some(format!("Invalid PDF {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        };
        let path = dest.display().to_string();
        let now = persist::unix_now();
        let books = persist::touch_progress(&self.lib_books, &path, 0, page_count, now);
        // E4: Política estricta anti-borrado automático. La biblioteca NUNCA
        // elimina un PDF automáticamente. Solo la acción explícita del
        // usuario desde el menú puede borrar un libro.
        self.lib_books = books;
        persist::save_progress(self.internal_dir.as_deref(), &self.lib_books);
        info!(
            "added {path} ({page_count} pages); library {} books (0 auto-evicted)",
            self.lib_books.len()
        );
        self.select_list = Vec::new();
        self.reload_curated_library(app);
    }

    /// "Buscar...": abre el TECLADO del sistema sobre el EditText invisible
    /// (`jni::ime_attach`; ver `tools/ime/ImeHelper.java`) y activa el polling
    /// de `tick`. El texto tecleado filtra la rejilla por subcadena.
    pub(crate) fn lib_open_keyboard(&mut self, app: &AndroidApp) {
        crate::jni::ime_attach(app, &self.lib_query);
        self.ime_active = true;
    }

    /// "✕" del campo de búsqueda: limpia el texto tecleado, cierra el
    /// teclado y re-aplica (recalcula `lib_filtered`; vuelve a verse toda la
    /// biblioteca).
    pub(crate) fn lib_clear_search(&mut self, app: &AndroidApp) {
        self.lib_query.clear();
        crate::jni::ime_set_text(app, "");
        self.ime_active = false;
        crate::jni::ime_hide(app);
        self.apply_filter();
    }

    /// Cierra el teclado del buscador si está abierto (al entrar al visor,
    /// al abrir el selector de añadir, etc.). No toca el texto del filtro.
    pub(crate) fn lib_close_ime(&mut self, app: &AndroidApp) {
        if self.ime_active {
            self.ime_active = false;
            crate::jni::ime_hide(app);
        }
    }

    /// Polling del texto tecleado (llamado desde `tick`): si cambió, se
    /// re-filtra la rejilla (busca mientras se escribe, sin botón).
    fn poll_ime_query(&mut self, app: &AndroidApp) {
        if !self.ime_active {
            return;
        }
        let Some(t) = crate::jni::ime_text(app) else {
            return;
        };
        if t != self.lib_query {
            self.lib_query = t;
            self.refresh_lib_filtered();
            let max_v = self.lib_max_scroll();
            if self.lib_scroll > max_v {
                self.lib_scroll = max_v;
            }
            self.list_dirty = true;
            self.redraw();
        }
    }

    /// Abre un documento de la biblioteca. Entrada CURADA: `uri` es la RUTA
    /// LOCAL del fichero ya copiado en `internal/pdfs/` → abrir directo sin
    /// copiar (`open_pdf_at`, reanuda en la página guardada). Entrada
    /// clásica de MediaStore (content://) → copia los bytes a `internal/
    /// pdfs/` y abre con MuPDF. Devuelve false (estado intacto) si algo falla.
    pub(crate) fn open_library_entry(&mut self, app: &AndroidApp, entry: &LibraryEntry) -> bool {
        // Biblioteca CURADA: el fichero ya está en `internal/pdfs/`.
        if Path::new(&entry.uri).is_file() {
            self.lib_close_ime(app);
            let start = crate::persist::progress_for(&self.lib_books, &entry.uri).map(|p| p.page);
            return self.open_pdf_at(&entry.uri, start);
        }
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
        // Reanudar en la página guardada si el libro ya se empezó
        // (registro de progreso de `library.json`); si no, página 1.
        self.lib_close_ime(app);
        let start = crate::persist::progress_for(&self.lib_books, &path).map(|p| p.page);
        self.open_pdf_at(&path, start)
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

/// Lee el modo del boli persistido en `tool_state.json` (campo "mode":
/// "Ink" | "Highlight"). RETROCOMPATIBLE: un fichero viejo sin el campo (o
/// con un valor desconocido) carga como `Ink`. Best-effort, como el resto de
/// la persistencia. NO toca `persist.rs` (fuera de alcance de esta tarea):
/// el JSON completo se lee como `Value`; al guardar (`persist_pen_mode`)
/// solo se conserva/añade "mode", respetando lo que escribe `persist`
/// (ink_color/ink_width).
fn load_pen_mode(internal_dir: Option<&Path>) -> PenMode {
    let Some(dir) = internal_dir else {
        return PenMode::Ink;
    };
    let path = dir.join("tool_state.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return PenMode::Ink;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return PenMode::Ink;
    };
    match v.get("mode").and_then(|m| m.as_str()) {
        Some("Highlight") => PenMode::Highlight,
        _ => PenMode::Ink,
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // Parada limpia del worker actor (Stop + join): el hilo muere con
        // su documento, sin filtrar el PDF abierto.
        self.stop_render_worker();
    }
}
