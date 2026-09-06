// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Estado de la app y lógica del visor/picker/biblioteca (`struct Reader`).
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13) y de la partición de
//! `reader.rs` (2026-09-06, Tarea 4.4 de la reestructuración): `reader/mod.rs` conserva
//! el MODELO DE DATOS — tipos de lista (`PdfEntry`, `LibraryEntry`, `LibraryScan`,
//! `BookStatus`, `LibSort`, `LibraryViewMode`, …), estado de selección/IA (`SelState`,
//! `SelMenu`, `AiPhase`, `AiPanel`, `ListDrag`, `EmptyStateGeom`), el `struct Reader`
//! con sus campos (los de la biblioteca, `lib_*`, viven en `LibraryState`
//! — Tarea 4.5, ver `library_state.rs`), los helpers
//! libres del modelo (`title_from_name`, `scan_pdfs`, …), dos métodos transversales
//! (`next_ovl_id`, `mark_repaint`), `load_pen_mode` y el `Drop`. La LÓGICA vive en 12
//! submódulos por responsabilidad (ver abajo). El input (gestos) está en `input`, el
//! dibujo en `draw`, el JNI en `jni`, la escala inicial en `view` (stub) y el blit
//! rápido en `zoom` (stub). Los paths `crate::reader::*` que consumen `draw`, `input`,
//! `gpu`, `jni` y `persist` se conservan con re-exports al final de este módulo.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use android_activity::AndroidApp;
use android_activity::ndk::native_window::NativeWindow;
use pdf_core::engine::mupdf::MupdfDocument;
use pdf_core::{AnnotationSet, Bitmap, Color, PageTextCache};

use crate::annotations::PenMode;
use crate::annotations::{ToolGesture, ToolKind};
use crate::cache::PageCache;
use crate::draw::ButtonRect;
use crate::gpu::Gpu;
use crate::input::GestureState;
use crate::persist::{BookProgress, RecentEntry};
use crate::theme;
use crate::thumbs::ThumbCache;

// Partición de `reader.rs` (2026-09-06, Tarea 4.4): submódulos por
// responsabilidad — ver el doc de cada uno para su contenido.
mod anotaciones;
mod geometry;
mod library;
mod library_state;
mod life;
mod navigation;
mod pinch;
mod redraw;
mod seleccion;
mod sheet_chrome;
mod tick;
mod toast_ia;
mod tools;

// Tipos del worker de render y del anclaje del pinch: campos del `struct Reader`.
use pinch::PinchAnchor;
use redraw::{RenderWorker, WorkerMsg};

// API geométrica compartida del crate (paths `crate::reader::*` que consumen
// `draw`/`input`; el resto de la geometría sigue siendo `reader::geometry`).
pub(crate) use geometry::{
    GRID_CELL_PAD, cover_size_multiplier, grid_cell_h, grid_cell_rect, grid_cell_w, grid_cover_h,
    grid_cover_w, grid_gap, grid_pad, header_menu_btn_d, human_size, lib_add_btn_w, lib_chip_h,
    lib_chips, lib_cont_block_h, lib_cont_card_h, lib_cont_card_w, lib_cont_card_x,
    lib_cont_cover_h, lib_cont_cover_w, lib_cont_gap, lib_content_y0, lib_empty_state_geom,
    lib_grid_y0, lib_header_h, lib_org_block_h, lib_org_chip_h, lib_org_chips, lib_search_chips_y0,
    lib_search_h, lib_search_panel_h, lib_section_title_h, list_row_gap, list_row_h, list_row_rect,
    page_badge_rect, page_badge_size, picker_btn_h, picker_btn_w, picker_header_h, picker_row_h,
    settings_menu_button_rect, sheet_act_y, sheet_btn_h, sheet_btn_w, sheet_h, sheet_nav_y,
    sheet_pad, sheet_theme_btn_w, sheet_theme_y, truncate_name, view_menu_button_rect,
    viewer_bottom_chrome_h, viewer_top_chrome_h,
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
// sección "Continue Reading" oculta por diseño
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
    /// picker, píxeles (`library.lib_scroll`) en la biblioteca.
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
    /// `library.lib_scroll` en píxeles — ver abajo).
    pub(crate) list_scroll: usize,
    /// Estado de la BIBLIOTECA (Tarea 4.5 de la reestructuración): los campos
    /// `lib_*` (scrolls px, filtros, sort, registro de progreso, planos
    /// cacheados y fade de apertura) viven en `LibraryState` — ver
    /// `library_state.rs`; aquí se poseen como un único campo y los accesos
    /// usan `self.library.lib_x`.
    pub(crate) library: library_state::LibraryState,
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
    /// Contador monótono de generaciones de los bitmaps de overlay (chrome,
    /// toast, sheet, badges, menús, cursor de goma, fade): cada re-render de
    /// un overlay consume un id nuevo (`<overlay>_id = next_ovl_id()`). La
    /// caché de texturas GPU (`Gpu::ovl_cache`) se clavea por ese id y NUNCA
    /// por el puntero de `Bitmap::data`: con el puntero, cuando el allocator
    /// reusa la dirección de un bitmap ya liberado (ABA) el hit devolvía la
    /// textura del contenido ANTERIOR. Quien posee el id posee el bitmap: la
    /// caché ya no clona los pixels (Tarea 2.4).
    pub(crate) ovl_seq: u64,
    /// Bitmap renderizado de la barra superior de chrome del visor.
    pub(crate) chrome_top_bitmap: Option<Bitmap>,
    /// Id de generación de `chrome_top_bitmap` (caché GPU; ver `ovl_seq`).
    pub(crate) chrome_top_id: u64,
    /// Bitmap renderizado de la barra inferior de chrome del visor.
    pub(crate) chrome_bottom_bitmap: Option<Bitmap>,
    /// Id de generación de `chrome_bottom_bitmap` (caché GPU; ver `ovl_seq`).
    pub(crate) chrome_bottom_id: u64,
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
    /// Id de generación de `sheet_bitmap` (caché GPU; ver `ovl_seq`).
    pub(crate) sheet_id: u64,
    /// Bitmap del indicador de página "N / total" (overlay abajo a la
    /// izquierda, tap = página siguiente), cacheado: se invalida al cambiar
    /// ventana, página o modo oscuro.
    pub(crate) page_badge: Option<Bitmap>,
    /// Id de generación de `page_badge` (caché GPU; ver `ovl_seq`).
    pub(crate) page_badge_id: u64,
    /// Bitmap del indicador de MODO del boli (overlay abajo a la derecha,
    /// ✏️/🖍️): se invalida al alternar modo o cambiar ventana — el usuario
    /// siempre ve en qué modo va a dibujar el boli.
    pub(crate) mode_badge: Option<Bitmap>,
    /// Id de generación de `mode_badge` (caché GPU; ver `ovl_seq`).
    pub(crate) mode_badge_id: u64,
    /// Posición de pantalla de la GOMA durante el borrado (None = sin gesto
    /// de borrado): dibuja el cursor circular (`eraser_cursor`) para que el
    /// usuario vea exactamente qué área se va a borrar.
    pub(crate) erase_pt: Option<(f32, f32)>,
    /// Radio del cursor de la goma en PÍXELES (radio en puntos × escala
    /// efectiva; fijo durante el gesto — el zoom no cambia mientras se borra).
    pub(crate) erase_r_px: f32,
    /// Bitmap cacheado del cursor circular de la goma (se regenera por gesto).
    pub(crate) eraser_cursor: Option<Bitmap>,
    /// Id de generación de `eraser_cursor` (caché GPU; ver `ovl_seq`).
    pub(crate) eraser_cursor_id: u64,
    /// Caché LRU de portadas de la biblioteca (content:// URI → portada de la
    /// página 1, `THUMB_W` px de ancho). Se limpia al abrir un PDF: las
    /// portadas y la `PageCache` del visor no compiten por el mismo
    /// presupuesto (estados mutuamente exclusivos: biblioteca vs visor).
    pub(crate) thumbs: ThumbCache,
    /// URIs cuya portada falló al renderizar (PDF corrupto, fd no abrible,
    /// página 1 vacía): no se reintentan — evita un bucle de timeout del
    /// bucle de eventos (`thumbs_pending` las excluye).
    thumb_failed: HashSet<String>,
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
    /// Id de generación del bitmap de `sel_menu` (caché GPU; ver `ovl_seq`).
    pub(crate) sel_menu_id: u64,
    /// Panel flotante de "Preguntar a la IA" (Parte 2): tarjeta tipo
    /// `SelMenu` con cabecera (título + ✕/▲/▼) y cuerpo de texto envuelto
    /// con scroll (ver `AiPanel`). Some mientras esté abierto (fase
    /// Asking/Answer/Error); se abre al tocar "IA" en el menú de selección
    /// (`ask_ai`) y se cierra con ✕ o tap fuera (`close_ai_panel`).
    pub(crate) ai_panel: Option<AiPanel>,
    /// Id de generación del bitmap de `ai_panel` (caché GPU; ver `ovl_seq`).
    pub(crate) ai_panel_id: u64,
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
    /// Id de generación de `toast_bitmap` (caché GPU; ver `ovl_seq`).
    pub(crate) toast_id: u64,
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

impl Reader {
    /// Siguiente id de generación de un bitmap de overlay: cada re-render
    /// consume un id nuevo (monótono, nunca se reusa) que la caché de
    /// texturas GPU usa de clave. Ver `ovl_seq`.
    fn next_ovl_id(&mut self) -> u64 {
        self.ovl_seq += 1;
        self.ovl_seq
    }

    /// Marca repintado pendiente (coalescing por vsync).
    fn mark_repaint(&mut self) {
        self.repaint = true;
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // Parada limpia del worker actor (Stop + join): el hilo muere con
        // su documento, sin filtrar el PDF abierto.
        self.stop_render_worker();
    }
}
