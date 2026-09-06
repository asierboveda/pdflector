// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Estado de la BIBLIOTECA (Tarea 4.5 de la reestructuración): agrupa en
//! `LibraryState` los campos `lib_*` que vivían en el `struct Reader`
//! (scrolls en px, filtros y orden de "My Library", registro de progreso,
//! planos cacheados de la zona fija + banda y la transición al abrir un
//! libro). `Reader` lo posee como ÚNICO campo `library: LibraryState`, y los
//! accesos pasan de `self.lib_x` a `self.library.lib_x`.
//!
//! Partición de métodos (ver el reporte de la tarea): aquí viven SOLO los
//! métodos que tocan EXCLUSIVAMENTE campos `lib_*` (`LibraryState::new`,
//! `book_progress_pct`, `entry_passes`). Los métodos de biblioteca que
//! mezclan estado del visor/picker (ventana, listas de `Reader`, IME,
//! persistencia de directorios, `redraw`, …) se quedan en `impl Reader`
//! (reader/library.rs y demás) accediendo a este estado vía `self.library`.

use super::BookStatus;
use super::LibSort;
use super::LibraryEntry;
use crate::persist::{self, BookProgress};
use pdf_core::Bitmap;
use std::path::Path;
use std::time::Instant;

/// Estado de la biblioteca de `Reader`: scrolls (vertical y de las filas
/// horizontales), filtros de búsqueda, sort/filtro de estado, registro de
/// progreso persistido, planos cacheados de la biblioteca (zona fija +
/// banda de contenido) y la transición visual al abrir un libro.
pub(crate) struct LibraryState {
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
    /// Bitmap CACHEADO de la zona FIJA de la biblioteca (cabecera editorial +
    /// campo de búsqueda + panel de chips + franja de estado): alto =
    /// `lib_content_y0`, origen = borde superior de la ventana. Se
    /// re-renderiza SÓLO cuando cambia la estructura (datos, filtros, panel
    /// de búsqueda, estado, tamaño de ventana), NUNCA por frame de scroll
    /// (el blit copia la zona fija + la banda de contenido, ver `lib_band`).
    /// Es el análogo del frame compuesto del visor para la biblioteca.
    pub(crate) lib_header: Option<Bitmap>,
    /// Generación del bitmap `lib_header` (bump en cada re-render o mutación
    /// in-place): clave de la textura GPU dedicada del plano de cabecera.
    /// El present GPU solo re-sube la textura cuando esta versión cambia
    /// (Tarea 2.7: la subida por frame de ~12 MB sería lenta).
    pub(crate) lib_header_ver: u64,
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
    /// Generación del bitmap `lib_band` (bump en cada re-render o mutación
    /// in-place — portadas nuevas pegadas sobre la banda): clave de la
    /// textura GPU dedicada de la banda (Tarea 2.7).
    pub(crate) lib_band_ver: u64,
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
    /// Id de generación del snapshot de `lib_fade` (textura dedicada GPU del
    /// fade; ver `ovl_seq`).
    pub(crate) lib_fade_id: u64,
}

impl LibraryState {
    /// Constructor del estado de biblioteca: los valores por defecto que
    /// `Reader::new` inicializaba inline en los campos `lib_*` pasan aquí.
    /// El registro de progreso (`lib_books`) se lee de `internal/library.json`
    /// exactamente como hacía el Reader (misma persistencia, mismo
    /// comportamiento); los demás campos arrancan "en reposo" (sin scroll,
    /// sin filtros, planos y fade sin renderizar).
    pub(crate) fn new(internal_dir: Option<&Path>) -> Self {
        Self {
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
            lib_sort: LibSort::RecentlyAdded,
            lib_status: None,
            lib_books: persist::load_progress(internal_dir),
            lib_filtered: Vec::new(),
            lib_header: None,
            lib_header_ver: 0,
            lib_band: None,
            lib_band_ver: 0,
            lib_row_dirty: None,
            lib_fade: None,
            lib_fade_id: 0,
        }
    }

    /// Porcentaje leído de un libro (0.0-1.0) según la ruta de su fichero.
    #[allow(dead_code)]
    pub(crate) fn book_progress_pct(&self, path: &str) -> Option<f32> {
        crate::persist::progress_for(&self.lib_books, path).map(|b| b.pct())
    }

    /// ¿La entrada pasa el filtro de BÚSQUEDA activo (carpeta + letra inicial)?
    pub(crate) fn entry_passes(&self, e: &LibraryEntry) -> bool {
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
}
