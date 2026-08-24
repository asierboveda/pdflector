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
    Annotated, Annotation, AnnotationSet, Bitmap, Color, Document, Gesture, Highlight,
    PageTextCache, Rect, RenderEngine, Stroke, TextSpan,
};

use crate::annotations::{DEFAULT_INK_COLOR, INK_PALETTE, STROKE_WIDTH_PT, ToolGesture, ToolKind};
use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES, PageCache};
use crate::draw::{
    ButtonRect, PageAnnots, PageBlit, ai_panel_layout, blit_composed, blit_lib_fade, blit_library,
    blit_page, compose_frame, compose_library_snapshot, paste_lib_thumbs, raster_tool_layer,
    render_ai_panel, render_carousel_row, render_library_header, render_library_zone,
    render_org_chip_row, render_page_badge, render_picker_list, render_search_chip_row,
    render_sel_menu, render_sheet, render_toast, render_tool_fab, render_toolbar, sel_menu_layout,
    splice_row, tool_fab_rect, toolbar_rect,
};
use crate::input::GestureState;
use crate::jni::{
    android_sdk_int, launch_intent_pdf, open_content_fd, query_media_store, read_content_uri_bytes,
    sanitize_pdf_name,
};
use crate::persist::{self, BookProgress, RecentEntry};
use crate::thumbs::{THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES, THUMB_W, ThumbCache};
use crate::view::initial_scale;
use crate::zoom::blit_fast;
use crate::{
    BACKGROUND, DARK_BG, ERROR_BG, LIB_FADE_MS, PINCH_MAX, PINCH_MIN, SEL_MIN_PX, TOAST_MS,
};

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

/// Orden de "My Library" (sort, chips discretos de organización).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibSort {
    /// `added_unix` del registro de progreso (más reciente primero; los
    /// nunca abiertos al final).
    RecentlyAdded,
    /// `last_read_unix` (más reciente primero; los nunca abiertos al final).
    RecentlyRead,
    /// Título (nombre de fichero sin extensión), case-insensitive.
    Title,
    /// Autor (primer segmento de RELATIVE_PATH), luego título.
    Author,
}

/// Un libro del carousel destacado "Continue Reading": un reciente abierto
/// no terminado, con su progreso persistido (página, total, %). Construido
/// en `Reader::lib_continue_reading` a partir de `recents.json` +
/// `library.json`; lo consumen el render (`draw`), el tap (`input`) y el
/// pump de portadas (`Reader::pump_thumbs`).
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

/// Alto (px) de la zona de texto de la celda: título (1 línea, ~13 sp) +
/// autor (1 línea, secundaria) + barra de progreso fina + aire generoso.
pub(crate) fn grid_title_h(_win_w: i32) -> f32 {
    62.0
}

/// Alto (px) de una celda de la rejilla.
pub(crate) fn grid_cell_h(win_w: i32) -> f32 {
    grid_cover_h(win_w) + grid_title_h(win_w)
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
//
// Estructura vertical FIJA (no scrollea): cabecera (título + Add book) +
// campo de búsqueda (+ panel de chips letra/carpeta si está abierto) +
// franja de estado (si la hay). El CONTENIDO scrollea en vertical por
// píxeles (`Reader::lib_scroll`): [Continue Reading] + [título "My
// Library" + chips de organización + rejilla] + aire inferior. Los
// scrolls HORIZONTALES: carousel (`lib_carousel_x`), panel de búsqueda
// (`lib_letters_x`/`lib_folders_x`) y organización (`lib_sort_x`/
// `lib_filter_x`).

/// Alto (px) de la CABECERA de la biblioteca: título "Library" grande y
/// negrita + botón "＋ Add book" a la derecha. Más alta que la del picker:
/// es la pieza editorial de la pantalla (mucho espacio negativo).
pub(crate) fn lib_header_h(win_h: i32) -> f32 {
    (win_h as f32 / 12.0).clamp(88.0, 120.0)
}

/// Alto (px) del campo de búsqueda (fila fija bajo la cabecera).
pub(crate) fn lib_search_h() -> f32 {
    46.0
}

/// Alto (px) del panel de búsqueda desplegado (2 filas de chips: letras y
/// carpetas); 0 si el campo de búsqueda está cerrado.
pub(crate) fn lib_search_panel_h(win_h: i32, open: bool) -> f32 {
    if open {
        lib_chip_h(win_h) * 2.0 + 12.0
    } else {
        0.0
    }
}

/// Alto (px) de un chip del panel de búsqueda (letras/carpetas).
pub(crate) fn lib_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 56.0).clamp(32.0, 40.0)
}

/// Y (px) del borde superior de la fila 0 (letras) del panel de búsqueda.
pub(crate) fn lib_search_chips_y0(reader: &Reader) -> f32 {
    lib_header_h(reader.win_h) + lib_search_h() + 6.0
}

/// Y (px) del borde superior de la fila 1 (carpetas) del panel de búsqueda.
pub(crate) fn lib_search_chips_y1(reader: &Reader) -> f32 {
    lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) + 6.0
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

/// Ancho (px) de la portada de una tarjeta de "Continue Reading" (grande,
/// 2:3; algo menor que la de la rejilla para que el carousel respire).
pub(crate) fn lib_cont_cover_w(win_w: i32) -> f32 {
    grid_cover_w(win_w) * 0.72
}

/// Alto (px) de la portada de una tarjeta (proporción 2:3).
pub(crate) fn lib_cont_cover_h(win_w: i32) -> f32 {
    lib_cont_cover_w(win_w) * 1.5
}

/// Alto (px) de la zona de texto de la tarjeta (título + autor + barra de
/// progreso + "Page X of Y · Z%" + botón Read).
pub(crate) fn lib_cont_text_h() -> f32 {
    106.0
}

/// Alto (px) de la tarjeta completa (portada + zona de texto).
pub(crate) fn lib_cont_card_h(win_w: i32) -> f32 {
    lib_cont_cover_h(win_w) + lib_cont_text_h()
}

/// Ancho (px) de la tarjeta (portada + padding interior de 10 px por lado).
pub(crate) fn lib_cont_card_w(win_w: i32) -> f32 {
    lib_cont_cover_w(win_w) + 20.0
}

/// Separación horizontal entre tarjetas del carousel (px).
pub(crate) fn lib_cont_gap() -> f32 {
    14.0
}

/// X (px) en coords de CONTENIDO de la tarjeta `i` del carousel (sin el
/// scroll horizontal aplicado).
pub(crate) fn lib_cont_card_x(win_w: i32, i: usize) -> f32 {
    grid_pad(win_w) + i as f32 * (lib_cont_card_w(win_w) + lib_cont_gap())
}

/// Alto (px) del bloque de "Continue Reading" (título de sección + fila de
/// tarjetas) en coords de contenido; 0 si no hay libros en curso (la
/// sección se OCULTA, no se pinta un hueco vacío).
pub(crate) fn lib_cont_block_h(win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    if !has_cont {
        0.0
    } else {
        lib_section_title_h(win_h) + lib_cont_card_h(win_w) + 6.0
    }
}

/// --- Organización de "My Library" (sort + filter, chips discretos) ---
/// Alto (px) de un chip de organización: más pequeño que los del panel de
/// búsqueda — discretos, no dominan sobre las portadas.
pub(crate) fn lib_org_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 62.0).clamp(24.0, 30.0)
}

/// Separación entre las filas de chips de sort y filter (px).
pub(crate) fn lib_org_gap() -> f32 {
    8.0
}

/// Alto (px) del bloque de organización (2 filas: sort + filter).
pub(crate) fn lib_org_block_h(win_h: i32) -> f32 {
    lib_org_chip_h(win_h) * 2.0 + lib_org_gap()
}

/// Ancho (px) reservado para la etiqueta discreta de cada fila ("SORT" /
/// "FILTER"), antes de los chips.
pub(crate) fn lib_org_label_w() -> f32 {
    46.0
}

/// Y (px) del borde superior de la fila de organización `row` (0 = sort,
/// 1 = filter) en coords de CONTENIDO (bajo el título de "My Library").
pub(crate) fn lib_org_y(win_w: i32, win_h: i32, has_cont: bool, row: usize) -> f32 {
    lib_grid_y0(win_w, win_h, has_cont) - lib_org_block_h(win_h)
        + row as f32 * (lib_org_chip_h(win_h) + lib_org_gap())
}

/// Y (px) del borde superior de la REJILLA en coords de CONTENIDO (tras el
/// bloque de Continue Reading, el título de "My Library" y el bloque de
/// organización).
pub(crate) fn lib_grid_y0(win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    lib_cont_block_h(win_w, win_h, has_cont) + lib_section_title_h(win_h) + lib_org_block_h(win_h)
}

/// Alto total (px) del contenido scrolleable de la biblioteca (Continue
/// Reading + título de My Library + organización + rejilla + aire inferior).
pub(crate) fn lib_content_h(win_w: i32, win_h: i32, has_cont: bool, grid_rows: usize) -> f32 {
    lib_grid_y0(win_w, win_h, has_cont) + grid_rows as f32 * grid_cell_h(win_w) + 28.0
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
    let gap = grid_gap();
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
        let w = (10.0 + label.chars().count() as f32 * 6.4).clamp(44.0, (win_w / 4) as f32);
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
            ("Recently Added", reader.lib_sort == LibSort::RecentlyAdded),
            ("Recently Read", reader.lib_sort == LibSort::RecentlyRead),
            ("Title", reader.lib_sort == LibSort::Title),
            ("Author", reader.lib_sort == LibSort::Author),
        ]
    } else {
        vec![
            ("All", reader.lib_status.is_none()),
            ("Reading", reader.lib_status == Some(BookStatus::Reading)),
            ("Finished", reader.lib_status == Some(BookStatus::Finished)),
            ("Unread", reader.lib_status == Some(BookStatus::Unread)),
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
    (usable / grid_cell_h(win_w)).floor().max(1.0) as usize
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
    /// sin filtro de letra. Es la búsqueda por NOMBRE SIN teclado, presentada
    /// como el panel del campo de búsqueda (ver `lib_chips`).
    pub(crate) lib_letter: Option<char>,
    /// Filtro de carpeta activo (RELATIVE_PATH, p. ej. "Download/"); None =
    /// sin filtro de carpeta. Es la búsqueda por CARPETA SIN teclado.
    pub(crate) lib_folder: Option<String>,
    /// ¿El campo de búsqueda está desplegado? (true → panel de chips de
    /// letra/carpeta visible bajo el campo; el contenido baja).
    pub(crate) lib_search_open: bool,
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
    /// Bitmap CACHEADO de la zona FIJA de la biblioteca (cabecera editorial +
    /// campo de búsqueda + panel de chips + franja de estado): alto =
    /// `lib_content_y0`, origen = borde superior de la ventana. Se
    /// re-renderiza SÓLO cuando cambia la estructura (datos, filtros, panel
    /// de búsqueda, estado, tamaño de ventana), NUNCA por frame de scroll
    /// (el blit copia la zona fija + la banda de contenido, ver `lib_band`).
    /// Es el análogo de `page_frame` del visor para la biblioteca.
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
    toast_bitmap: Option<Bitmap>,
    /// Herramienta de anotación activa en el visor (Fase 3.5): Navegar
    /// (gestos normales) / Resaltar / Boli. Con una herramienta distinta de
    /// Navegar el arrastre de UN dedo (o el lápiz de la tablet) dibuja en
    /// vez de navegar; el tap simple no cambia de página (`input`), y la
    /// selección de texto (long-press) queda desactivada mientras esté
    /// activa (`input::tick_gestures`).
    pub(crate) tool: ToolKind,
    /// ¿La barra de herramientas del visor está visible? La muestra/oculta el
    /// botón flotante "✎" (esquina superior derecha); el botón "→" de la
    /// barra la cierra. Ocultar la barra **vuelve a modo navegación**
    /// (decisión documentada: una herramienta activa sin barra visible
    /// dejaría el visor en un modo invisible difícil de revertir).
    pub(crate) toolbar_open: bool,
    pub(crate) status_bar_top: i32,
    /// Bitmap cacheado de la barra de herramientas (píldora con los botones
    /// Resaltar/Boli/↶/●/→, `draw::render_toolbar`). Se invalida al alternar
    /// herramienta/color, al cambiar el modo oscuro o al redimensionar.
    toolbar_bitmap: Option<Bitmap>,
    /// Bitmap cacheado del botón flotante de toggle de la barra ("✎"/"✕",
    /// `draw::render_tool_fab`). Se invalida igual que la barra.
    tool_fab: Option<Bitmap>,
    /// Color actual de la tinta del boli (cicla con el botón "●" de la
    /// barra; arranca en `DEFAULT_INK_COLOR`).
    pub(crate) ink_color: Color,
    /// Gesto de herramienta EN CURSO (dedo/lápiz bajado con una herramienta
    /// activa): puntos y ancla en coordenadas de PÁGINA (ver `ToolGesture`).
    /// `Some` mientras el dedo está abajo; se convierte en una anotación
    /// guardada al levantar (`end_tool_gesture`) o se descarta al cancelar.
    /// Mientras es `Some`, `blit` usa el frame compuesto + la capa temporal
    /// del trazo (sin re-blitear la página por Move — requisito 5).
    pub(crate) tool_gesture: Option<ToolGesture>,
    /// ids de las anotaciones CREADAS EN ESTA SESIÓN (dedo/lápiz, en orden
    /// de creación): el botón "↶" de la barra deshace la última. Solo
    /// anotaciones nuevas de la sesión (no las cargadas del sidecar): el
    /// undo no toca trabajo de otras sesiones (decisión documentada).
    pub(crate) session_ids: Vec<u64>,
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
            lib_scroll: 0.0,
            lib_carousel_x: 0.0,
            lib_folders_x: 0.0,
            lib_letters_x: 0.0,
            lib_sort_x: 0.0,
            lib_filter_x: 0.0,
            lib_letter: None,
            lib_folder: None,
            lib_search_open: false,
            lib_sort: LibSort::RecentlyAdded,
            lib_status: None,
            lib_books: persist::load_progress(app.internal_data_path().as_deref()),
            lib_filtered: Vec::new(),
            recents: persist::load_recents(app.internal_data_path().as_deref()),
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
            toolbar_open: false,
            toolbar_bitmap: None,
            tool_fab: None,
            ink_color: DEFAULT_INK_COLOR,
            tool_gesture: None,
            session_ids: Vec::new(),
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
        self.lib_header = None;
        self.lib_band = None;
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
            self.lib_header = None; // zona fija de la biblioteca: tamaño nuevo
            self.lib_band = None; // banda de contenido: tamaño nuevo
            self.cache.clear(); // nueva escala cover → los bitmaps viejos no sirven
            self.list_dirty = true;
            self.page_badge = None;
            self.sheet_bitmap = None;
            self.page_frame = None;
            self.toolbar_bitmap = None; // la barra reescala con la ventana
            self.tool_fab = None;
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
            UiMode::Picker => {
                // Clamp del scroll si la lista menguó (rescan) o cambió la
                // ventana (picker: filas de `picker_row_h`).
                let visible = picker_visible_rows(self.win_h, self.status.is_some());
                let max_scroll = self.pdf_list.len().saturating_sub(visible);
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
        let viewport = (self.win_h - content_y0).max(0) as i32;
        let content_h = self.lib_content_h() as i32;
        let margin = grid_cell_h(self.win_w) as i32;
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
                let viewport = (self.win_h - content_y0).max(0) as i32;
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
            1 => {
                // Carousel de Continue Reading → banda, bajo su título.
                let row = render_carousel_row(self);
                let x = self.lib_carousel_x as i32;
                let y = lib_section_title_h(self.win_h) as i32;
                if let (Some(row), Some((band, origin))) = (row, self.lib_band.as_mut()) {
                    splice_row(band, &row, -x, y - *origin);
                }
            }
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
            4 | 5 => {
                // Chips de organización (sort/filter) → banda.
                let row = render_org_chip_row(self, (zone - 4) as usize);
                let x = if zone == 4 {
                    self.lib_sort_x as i32
                } else {
                    self.lib_filter_x as i32
                };
                let y = lib_org_y(
                    self.win_w,
                    self.win_h,
                    self.lib_has_cont(),
                    (zone - 4) as usize,
                ) as i32;
                if let (Some(row), Some((band, origin))) = (row, self.lib_band.as_mut()) {
                    splice_row(band, &row, -x, y - *origin);
                }
            }
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

    /// Remienda las filas horizontales de la BANDA (carousel + chips de
    /// sort/filter) sobre la banda actual.
    fn splice_band_rows(&mut self) {
        let has_cont = self.lib_has_cont();
        let cont_row = if has_cont {
            render_carousel_row(self)
        } else {
            None
        };
        let sort_row = render_org_chip_row(self, 0);
        let filter_row = render_org_chip_row(self, 1);
        let carousel_x = self.lib_carousel_x as i32;
        let sort_x = self.lib_sort_x as i32;
        let filter_x = self.lib_filter_x as i32;
        let cont_y = lib_section_title_h(self.win_h) as i32;
        let sort_y = lib_org_y(self.win_w, self.win_h, has_cont, 0) as i32;
        let filter_y = lib_org_y(self.win_w, self.win_h, has_cont, 1) as i32;
        if let Some((band, origin)) = self.lib_band.as_mut() {
            if let Some(row) = cont_row {
                splice_row(band, &row, -carousel_x, cont_y - *origin);
            }
            if let Some(row) = sort_row {
                splice_row(band, &row, -sort_x, sort_y - *origin);
            }
            if let Some(row) = filter_row {
                splice_row(band, &row, -filter_x, filter_y - *origin);
            }
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
                // botón flotante de herramientas + barra de herramientas,
                // indicador de página, menú de selección, panel de IA, aviso
                // breve y sheet de ajustes deslizado desde el borde superior
                // (solo si está visible; `progress == 1` = abierto del todo).
                // El menú, el panel y el aviso van SIEMPRE (también con el
                // sheet: se añaden al frame compuesto o como overlays de
                // `blit_composed`).
                let mut overlays: Vec<(&Bitmap, i32, i32)> = Vec::with_capacity(7);
                // Barra de herramientas (píldora) + botón flotante "✎": se
                // cachean como bitmaps (Canvas+JNI) y se invalidan al
                // alternar tool/color/modo oscuro o al redimensionar.
                if self.toolbar_open && self.toolbar_bitmap.is_none() {
                    self.toolbar_bitmap = render_toolbar(self);
                }
                if self.tool_fab.is_none() {
                    self.tool_fab = render_tool_fab(self);
                }
                if let Some(tb) = self.toolbar_bitmap.as_ref() {
                    let (tx, ty, _, _) = toolbar_rect(self.win_w, self.win_h);
                    overlays.push((tb, tx as i32, ty as i32));
                }
                if let Some(fb) = self.tool_fab.as_ref() {
                    let (fx, fy, _, _) = tool_fab_rect(self.win_w, self.win_h);
                    overlays.push((fb, fx as i32, fy as i32));
                }
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
                // Capa temporal del gesto de herramienta EN CURSO (trazo del
                // boli / rect del resaltador): rasterizada por Move en un
                // bitmap del bbox del trazo (pdf_core::overlay) y copiada con
                // alfa-blend sobre el frame en `blit_composed` — el visor NO
                // re-blitea la página por evento de movimiento (req. 5).
                let tool_layer: Option<(Bitmap, i32, i32)> = if self.tool_gesture.is_some() {
                    self.tool_overlay()
                } else {
                    None
                };
                let use_frame = self.sheet_progress > 0.0 || self.tool_gesture.is_some();
                if use_frame {
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
                        // selección: la capa temporal del trazo en curso
                        // (blend), la barra de herramientas, el menú, el
                        // aviso breve y el sheet como overlays opacos.
                        let mut sheet_ov: Vec<(&Bitmap, i32, i32)> = Vec::with_capacity(6);
                        if let Some(tb) = self.toolbar_bitmap.as_ref() {
                            let (tx, ty, _, _) = toolbar_rect(self.win_w, self.win_h);
                            sheet_ov.push((tb, tx as i32, ty as i32));
                        }
                        if let Some(fb) = self.tool_fab.as_ref() {
                            let (fx, fy, _, _) = tool_fab_rect(self.win_w, self.win_h);
                            sheet_ov.push((fb, fx as i32, fy as i32));
                        }
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
                        blit_composed(
                            window,
                            frame,
                            &sheet_ov,
                            tool_layer.as_ref().map(|(b, x, y)| (b, *x, *y)),
                        );
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
                // Transición al abrir un libro: fundir el snapshot de la
                // biblioteca/picker sobre la página durante `LIB_FADE_MS`
                // (segundo present TRANSITORIO ~12 frames, alfa decreciente;
                // la biblioteca ya no se pinta, solo se funde).
                if let Some((started, snap)) = &self.lib_fade {
                    let t = started.elapsed().as_secs_f32();
                    let alpha = (1.0 - t / LIB_FADE_MS).clamp(0.0, 1.0);
                    if alpha > 0.0 {
                        blit_lib_fade(window, snap, (alpha * 255.0).round() as u8);
                    }
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
                blit_library(window, header, band, content_y0, self.lib_scroll, toast_ov);
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
    pub(crate) fn needs_tick(&mut self) -> bool {
        self.sheet_anim
            || self.thumbs_pending()
            || self.toast.is_some()
            || self.gesture.press_pending()
            // Transición al abrir un libro: el tick expira el fade.
            || self.lib_fade.is_some()
            // Consulta de IA en vuelo: `tick` sondea el canal del hilo de
            // fondo (sin esto el poll bloquearía y la respuesta tardaría en
            // aparecer hasta el siguiente evento de input).
            || self.ai_rx.is_some()
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
    /// sheet, detecta el long-press del dedo en el documento (entra en modo
    /// selección), expira el aviso breve (toast) y renderiza un lote de
    /// portadas pendientes de la biblioteca. `lib::android_main` lo invoca en
    /// los eventos Wake/Timeout, que solo ocurren mientras `needs_tick()` (sin
    /// despertar el loop en reposo).
    pub(crate) fn tick(&mut self, app: &AndroidApp) {
        // Long-press: si el dedo lleva quieto > `LONG_PRESS_MS` en el área de
        // página (sin sheet), `input::tick_gestures` entra en modo selección.
        crate::input::tick_gestures(self, app);
        // Resultado del hilo de IA (si hay una consulta en vuelo): `try_recv`
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
                self.page_frame = None; // liberar también el frame compuesto
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
        // Rejilla (clave = content:// URI), solo filas visibles.
        let (row0, rows) = self.lib_visible_grid_rows();
        for row in row0..row0 + rows {
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
        let row0 = ((content_y0 - grid_y0_screen) / grid_cell_h(self.win_w)).max(0.0) as usize;
        let below = ((self.win_h as f32 - grid_y0_screen) / grid_cell_h(self.win_w))
            .ceil()
            .max(0.0) as usize;
        (row0, below + 1)
    }

    /// Renderiza bajo demanda un lote de portadas de la biblioteca (máx. 3 por
    /// tick, ~1-3 ms cada una): primero el carousel de "Continue Reading"
    /// (clave = ruta local) y después las celdas VISIBLES de la rejilla
    /// (clave = content:// URI). Solo las que no están en caché ni fallaron.
    /// Devuelve true si entró alguna portada nueva (→ re-render de la
    /// biblioteca).
    fn pump_thumbs(&mut self, app: &AndroidApp) -> bool {
        if self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        let mut budget = 3usize;
        let mut changed = false;
        // Carousel de Continue Reading primero (portadas de rutas LOCALES).
        if self.lib_cont_visible() {
            // Clonar rutas + nombres: `thumbs.insert` (mutable) no convive
            // con el préstamo de `lib_continue_reading()` (inmutable).
            let cont: Vec<(String, String)> = self
                .lib_continue_reading()
                .iter()
                .map(|b| (b.path.clone(), b.name.clone()))
                .collect();
            for (path, name) in cont {
                if budget == 0 {
                    return changed;
                }
                if self.thumbs.get(&path).is_some() || self.thumb_failed.contains(&path) {
                    continue;
                }
                match self.render_thumb_path(&path) {
                    Some(bmp) => {
                        self.thumbs.insert(path.clone(), bmp);
                        info!("continue thumb cached: {name}");
                        changed = true;
                    }
                    None => {
                        self.thumb_failed.insert(path.clone());
                        warn!("continue thumb failed: {path}");
                    }
                }
                budget -= 1;
            }
        }
        // Rejilla (clave = content:// URI).
        let (row0, rows) = self.lib_visible_grid_rows();
        for row in row0..row0 + rows {
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

    /// Renderiza la portada (página 1) de un PDF LOCAL por ruta (los
    /// recientes de la biblioteca): abre con `MupdfEngine::open` directo, sin
    /// pasar por un fd content:// (a diferencia de `render_thumb`).
    fn render_thumb_path(&self, path: &str) -> Option<Bitmap> {
        let result: pdf_core::Result<Bitmap> = (|| {
            let engine = MupdfEngine::new()?;
            let doc = engine.open(Path::new(path))?;
            let (pw, _ph) = doc.page_size(0)?;
            if !pw.is_finite() || pw <= 0.0 {
                return Err(pdf_core::Error::InvalidArgument(
                    "page 1 width invalid".into(),
                ));
            }
            doc.render_page(0, THUMB_W as f32 / pw)
        })();
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
            self.page_frame = None; // el frame compuesto del sheet tiene el zoom viejo
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
        self.rendered_zoom = zoom;
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
        self.toolbar_bitmap = None; // los colores de la barra cambian
        self.tool_fab = None; // y los del botón flotante
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

    // ---------------------------------------------------------------------
    // Barra de herramientas de anotación (Fase 3.5: resaltador + boli)
    // ---------------------------------------------------------------------
    //
    // La barra (píldora arriba, `draw::toolbar_rect`) la muestra/oculta el
    // botón flotante "✎" y tiene 5 botones (Resaltar/Boli/↶/●/→, geometría
    // `draw::toolbar_buttons`). El arrastre con una herramienta activa crea
    // un `ToolGesture` (puntos en coords de página) que al levantar se
    // convierte en `Highlight` (resaltador, vía `pdf_core::selection`) o en
    // `Stroke` suavizado (boli, vía `pdf_core::smooth_polyline`).

    /// Activa una herramienta (`ToolKind`). La barra permanece visible para
    /// poder cambiar de herramienta o volver a navegación; el chip activo se
    /// dibuja con el acento dorado (el `toolbar_bitmap` se invalida).
    pub(crate) fn set_tool(&mut self, tool: ToolKind) {
        self.tool = tool;
        self.toolbar_bitmap = None; // los estados activos de los botones cambian
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Muestra/oculta la barra de herramientas (botón flotante "✎"/"✕").
    /// Ocultarla vuelve a modo NAVEGACIÓN (decisión documentada en
    /// `toolbar_open`): nunca queda una herramienta activa sin barra visible.
    pub(crate) fn toggle_toolbar(&mut self) {
        self.toolbar_open = !self.toolbar_open;
        if !self.toolbar_open {
            self.tool = ToolKind::Navigate;
            self.cancel_tool_gesture();
        }
        self.toolbar_bitmap = None;
        self.tool_fab = None; // el icono cambia ("✎" ↔ "✕")
        self.redraw();
    }

    /// Botón "→" de la barra: vuelve a modo navegación y cierra la barra.
    pub(crate) fn close_toolbar(&mut self) {
        self.tool = ToolKind::Navigate;
        self.toolbar_open = false;
        self.toolbar_bitmap = None;
        self.tool_fab = None;
        self.cancel_tool_gesture();
        self.redraw();
    }

    /// Botón "●" de la barra: cicla el color del boli por `INK_PALETTE`.
    /// El botón de la barra se dibuja con el color actual, así que se
    /// invalida su bitmap para que el círculo cambie.
    pub(crate) fn cycle_ink_color(&mut self) {
        let i = INK_PALETTE
            .iter()
            .position(|c| *c == self.ink_color)
            .unwrap_or(0);
        self.ink_color = INK_PALETTE[(i + 1) % INK_PALETTE.len()];
        self.toolbar_bitmap = None; // el "●" muestra el nuevo color
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Botón "↶" de la barra: deshace la ÚLTIMA anotación creada en esta
    /// sesión (la pila `session_ids`; nunca toca anotaciones cargadas del
    /// sidecar ni de sesiones anteriores). La borra del set y persiste.
    pub(crate) fn undo_last_annotation(&mut self) {
        let Some(id) = self.session_ids.pop() else {
            return;
        };
        if !self.annotations.remove(id) {
            return;
        }
        self.save_annotations();
        self.page_frame = None; // el frame tendría la anotación borrada
        self.show_toast("undo");
    }

    /// Gesto de herramienta: el Down convierte el punto de pantalla a
    /// coordenadas de página y crea el `ToolGesture` en la página actual.
    /// El blit pasa a usar el frame compuesto + capa temporal (sin
    /// re-renderizar ni re-blitear la página) mientras el gesto dure.
    pub(crate) fn begin_tool_gesture(&mut self, sx: f32, sy: f32) {
        if self.tool == ToolKind::Navigate {
            return;
        }
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        self.tool_gesture = Some(ToolGesture::new(self.page, self.tool, pt));
        self.page_frame = None; // recomponer el frame SIN el trazo (la capa temporal va aparte)
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Gesto de herramienta: cada Move añade el punto (boli) o actualiza el
    /// rect (resaltador) y re-blitea con el frame compuesto + la capa
    /// temporal del trazo — la página NO se re-blitea por evento (req. 5).
    pub(crate) fn update_tool_gesture(&mut self, sx: f32, sy: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        match self.tool {
            ToolKind::Ink => {
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.push(pt);
                }
            }
            ToolKind::Highlight => {
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.set_cur(pt);
                }
            }
            ToolKind::Navigate => {}
        }
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Gesto de herramienta: al levantar el dedo convierte el gesto en una
    /// anotación GUARDADA (persistida en el sidecar):
    ///
    /// - **Boli**: `smooth_polyline` (Catmull-Rom, el suavizado del motor)
    ///   sobre los puntos capturados → `Stroke` con `STROKE_WIDTH_PT` y el
    ///   color actual. Un gesto sin arrastre (un toque) se descarta.
    /// - **Resaltador**: `pdf_core::highlight_under_gesture` selecciona las
    ///   líneas de texto bajo el trazo (extracción perezosa, solo ahora) y
    ///   crea el `Highlight` alineado al texto; "no text" si no hay líneas.
    ///
    /// El id nuevo se apunta en `session_ids` para el undo.
    pub(crate) fn end_tool_gesture(&mut self) {
        let Some(g) = self.tool_gesture.take() else {
            return;
        };
        self.page_frame = None; // la anotación guardada se dibuja vía el set
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
            if self.window.is_some() {
                self.blit();
            }
            return;
        }
        match g.tool {
            ToolKind::Ink => {
                // Suavizado Catmull-Rom del motor: 6 subdivisiones por
                // segmento (suficiente para que el trazo no se vea
                // poligonal; la serialización guarda solo los puntos
                // suavizados). Fase C: antes de suavizar se simplifica con
                // Douglas-Peucker (epsilon 1.5 pt) — un trazo de 100+
                // puntos del dedo baja a ~15-20 sin perder forma, y el
                // rasterizado/guardado pagan menos.
                let simplified = pdf_core::simplify_polyline(&g.points, 1.5);
                let pts = pdf_core::smooth_polyline(&simplified, 6);
                if let Some(s) = Stroke::new(pts, STROKE_WIDTH_PT, self.ink_color)
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
                let spans = self
                    .doc
                    .as_ref()
                    .and_then(|d| self.text_cache.get_or_extract(d, g.page).ok())
                    .map(|t| t.spans.clone())
                    .unwrap_or_default();
                let gesture = Gesture::Points(g.points);
                if let Some(hl) =
                    pdf_core::highlight_under_gesture(&spans, &gesture, pdf_core::HIGHLIGHT_COLOR)
                {
                    if let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Highlight(hl))
                    {
                        self.session_ids.push(id);
                        self.save_annotations();
                        self.show_toast("highlighted");
                    }
                } else {
                    // Sin líneas bajo el trazo (zona en blanco, PDF escaneado
                    // o trazo demasiado corto): aviso, no se crea nada.
                    self.show_toast("no text");
                }
            }
            ToolKind::Navigate => {}
        }
    }

    /// Gesto de herramienta cancelado (segundo dedo, Cancel del sistema,
    /// ocultar la barra): descarta el trazo en curso sin crear anotación.
    pub(crate) fn cancel_tool_gesture(&mut self) {
        if self.tool_gesture.take().is_some() {
            self.page_frame = None;
            if self.window.is_some() {
                self.blit();
            }
        }
    }

    /// ¿El punto de pantalla cae en el "chrome" de las herramientas (el
    /// botón flotante o la barra abierta)? El Down en esa zona NO inicia un
    /// gesto de herramienta (es un tap de UI): lo decide `input`.
    pub(crate) fn chrome_hit(&self, x: f32, y: f32) -> bool {
        let (l, t, r, b) = tool_fab_rect(self.win_w, self.win_h);
        if x >= l && x < r && y >= t && y < b {
            return true;
        }
        if self.toolbar_open {
            let (l, t, r, b) = toolbar_rect(self.win_w, self.win_h);
            if x >= l && x < r && y >= t && y < b {
                return true;
            }
        }
        false
    }

    /// Capa temporal del gesto de herramienta en curso: rasteriza el trazo
    /// del boli o el rect del resaltador en un bitmap del tamaño de su bbox
    /// de pantalla (`draw::raster_tool_layer`, pdf_core::overlay) con la
    /// misma transformación del blit (`scale = cover × zoom`, esquina con
    /// pan). Coste ∝ bbox del trazo — el presupuesto del requisito 5.
    fn tool_overlay(&self) -> Option<(Bitmap, i32, i32)> {
        let g = self.tool_gesture.as_ref()?;
        let (pw, ph) = self.doc.as_ref()?.page_size(g.page).ok()?;
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        let scale = cover * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let dx = (Self::centered_base(self.win_w, pw * cover, self.zoom) + self.pan_x).round();
        let dy = self.pan_y.round();
        let xform = pdf_core::ViewTransform {
            zoom: scale,
            offset_x: dx,
            offset_y: dy,
        };
        // Anotación TEMPORAL (no está en el set): con los puntos en curso.
        let kind = match g.tool {
            ToolKind::Ink => {
                // Un solo punto aún no es un trazo: duplicarlo desplazado
                // dibuja un punto de tinta (el `Stroke::new` exige ≥ 2).
                let pts = if g.points.len() >= 2 {
                    g.points.clone()
                } else {
                    vec![g.anchor, (g.anchor.0 + 0.01, g.anchor.1)]
                };
                let s = Stroke::new(pts, STROKE_WIDTH_PT, self.ink_color)?;
                Annotation::Stroke(s)
            }
            ToolKind::Highlight => {
                let cur = g.points.last().copied().unwrap_or(g.anchor);
                Annotation::Highlight(Highlight {
                    rects: vec![Rect::new(
                        g.anchor.0,
                        g.anchor.1,
                        cur.0 - g.anchor.0,
                        cur.1 - g.anchor.1,
                    )],
                    color: pdf_core::HIGHLIGHT_COLOR,
                })
            }
            ToolKind::Navigate => return None,
        };
        // Padding para que la media brocha del trazo (y su AA de 1 px) no se
        // recorte en el borde del bitmap temporal.
        let pad = if g.tool == ToolKind::Ink {
            (STROKE_WIDTH_PT * scale * 0.5).ceil() + 1.0
        } else {
            0.0
        };
        let ann = Annotated {
            id: 0,
            page_idx: g.page as usize,
            kind,
        };
        raster_tool_layer(self.win_w, self.win_h, xform, &ann, pad)
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
    fn save_state(&mut self) {
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
        // Progreso por libro (eager, igual que state.json; un cierre
        // inesperado no pierde la página del libro).
        let pages = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
        let now = crate::persist::unix_now();
        self.lib_books =
            crate::persist::touch_progress(&self.lib_books, path, self.page, pages, now);
        crate::persist::save_progress(self.internal_dir.as_deref(), &self.lib_books);
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
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera (libera también el frame)
                self.clear_selection(); // selección del documento anterior: fuera
                self.close_ai_panel(); // panel de IA del documento anterior: fuera
                self.thumbs.clear(); // portadas de otra biblioteca: no sirven
                self.thumb_failed.clear();
                self.list_dirty = true;
                self.list_drag = None;
                // Herramientas de anotación: reseteo a la navegación limpia
                // (barra cerrada, sin herraienta activa, sin gesto en curso
                // y SIN histórico de sesión del documento anterior — el undo
                // es por sesión, decisión documentada en `session_ids`).
                self.tool = ToolKind::Navigate;
                self.toolbar_open = false;
                self.toolbar_bitmap = None;
                self.tool_fab = None;
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

    /// Entra en la biblioteca MediaStore (botón "← Library" del sheet del
    /// visor): re-consulta y deja de mostrar la página. El campo de búsqueda
    /// arranca CERRADO. Si MediaStore está vacía y la carpeta interna de la
    /// app tampoco tiene PDFs, se queda en la biblioteca mostrando el EMPTY
    /// STATE (ver `rescan_library`).
    pub(crate) fn enter_library(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Library;
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
        self.toolbar_open = false;
        self.toolbar_bitmap = None;
        self.tool_fab = None;
        self.tool_gesture = None;
        self.session_ids.clear();
        self.rescan_library(app);
    }

    /// Abre el picker interno (PDFs de los directorios de la app; el fallback
    /// cuando MediaStore no sirve). Con el sheet rediseñado (2026-08-XX) ya no
    /// hay botón "Open" en el visor: el picker queda accesible solo por el
    /// fallback de `rescan_library` (MediaStore vacía). Se conserva el método
    /// por si una fase futura reintroduce la entrada.
    #[allow(dead_code)]
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

    /// Re-consulta MediaStore (botón "＋ Add book" / arranque / Resume de la
    /// biblioteca). Actualiza la lista, el estado del permiso y el mensaje.
    /// Si el permiso está concedido pero no hay PDFs en MediaStore ni en la
    /// carpeta interna de la app, se queda en la biblioteca mostrando el
    /// EMPTY STATE ("Your library is empty" + botón Add PDF); si la carpeta
    /// interna sí tiene PDFs (no visibles en MediaStore), cae al picker como
    /// fallback para no perder el acceso.
    pub(crate) fn rescan_library(&mut self, app: &AndroidApp) {
        self.grant_pending = false;
        let scan = query_media_store(app, self.sdk_int);
        self.library_list = scan.entries;
        // Datos nuevos: el scroll vuelve al principio (vertical y horizontales)
        // y se recalcula la lista filtrada (los filtros activos se CONSERVAN:
        // si el usuario está viendo "Download/", el rescan no le cambia el
        // filtro, solo refresca los datos).
        self.refresh_lib_filtered();
        self.permission_granted = scan.permission_granted;
        self.list_scroll = 0;
        self.lib_scroll = 0.0;
        self.lib_carousel_x = 0.0;
        self.lib_folders_x = 0.0;
        self.lib_letters_x = 0.0;
        self.lib_sort_x = 0.0;
        self.lib_filter_x = 0.0;
        self.list_dirty = true;
        if !self.permission_granted {
            self.status = Some("All files access not granted — tap Grant".to_string());
        } else if let Some(e) = scan.error {
            self.status = Some(format!("MediaStore error: {e}"));
        } else if self.library_list.is_empty() {
            // Sin PDFs en MediaStore. ¿Y en la carpeta interna de la app?
            // Si tampoco hay, EMPTY STATE en la biblioteca (botón "Add PDF");
            // si la hay, picker como fallback para no perder el acceso.
            self.pdf_list = scan_pdfs(app);
            if self.pdf_list.is_empty() {
                self.mode = UiMode::Library;
                self.status = None;
            } else {
                self.mode = UiMode::Picker;
                self.lib_header = None; // biblioteca fuera: liberar planos
                self.lib_band = None;
                self.lib_row_dirty = None;
                self.status = Some("No PDFs in MediaStore — showing app folder".to_string());
            }
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

    /// Nº de filas de celdas de la rejilla de la biblioteca (3 columnas) con
    /// el filtro actual aplicado (`lib_filtered`).
    pub(crate) fn grid_total_rows(&self) -> usize {
        self.lib_filtered.len().div_ceil(GRID_COLS)
    }

    /// Entrada de la rejilla en la fila `row` (0-based) y columna `col`
    /// (0..GRID_COLS) — resolución sobre la lista FILTRADA (`lib_filtered`,
    /// que sin filtros equivale a TODAS las entradas en orden de MediaStore).
    /// None si la celda está fuera de rango (fila incompleta de la última).
    pub(crate) fn grid_entry_at(&self, row: usize, col: usize) -> Option<&LibraryEntry> {
        let idx = row.checked_mul(GRID_COLS)?.checked_add(col)?;
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
        self.lib_filtered = idxs;
    }

    /// Aplica un cambio de filtro/sort: recalcula la lista, clampa el scroll
    /// y re-renderiza.
    fn apply_filter(&mut self) {
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
    pub(crate) fn lib_has_cont(&self) -> bool {
        if let Some(s) = self.lib_status
            && s != BookStatus::Reading
        {
            return false;
        }
        self.lib_recents().iter().any(|r| {
            persist::progress_for(&self.lib_books, &r.path).is_some_and(|p| !p.is_finished())
        })
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
        lib_content_h(
            self.win_w,
            self.win_h,
            self.lib_has_cont(),
            self.grid_total_rows(),
        )
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
        let last_right = lib_cont_card_x(self.win_w, n - 1) + lib_cont_card_w(self.win_w);
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

    /// "＋ Add book" (cabecera) / "Add PDF" (empty state): re-consulta
    /// MediaStore y avisa con un toast cómo añadir PDFs a la biblioteca
    /// (descargarlos a Descargas y abrirlos con PDFLector).
    pub(crate) fn add_book(&mut self, app: &AndroidApp) {
        self.rescan_library(app);
        self.show_toast("Add PDFs to Downloads, then open with PDFLector");
    }

    /// Toggle del campo de búsqueda: abre/cierra el panel de chips de letra
    /// y carpeta (el contenido baja/sube con `lib_search_panel_h`).
    pub(crate) fn lib_toggle_search(&mut self) {
        self.lib_search_open = !self.lib_search_open;
        let max_v = self.lib_max_scroll();
        if self.lib_scroll > max_v {
            self.lib_scroll = max_v;
        }
        self.list_dirty = true;
        self.redraw();
    }

    /// "✕" del campo de búsqueda: limpia los filtros activos (letra y
    /// carpeta), cierra el panel y re-aplica (recalcula `lib_filtered`).
    pub(crate) fn lib_clear_search(&mut self) {
        self.lib_letter = None;
        self.lib_folder = None;
        self.lib_search_open = false;
        self.apply_filter();
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
        // Reanudar en la página guardada si el libro ya se empezó
        // (registro de progreso de `library.json`); si no, página 1.
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
