// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Persistencia del estado del visor (posición + modo oscuro) en
//! `internal_data_path()/state.json` (JSON con serde/serde_json — ya en el
//! grafo de dependencias vía pdf_core, mismas versiones y ya cross-compiladas
//! a aarch64-linux-android; no añaden unidades de compilación nuevas).
//!
//! ## Formato
//!
//! ```json
//! { "path": "/data/user/0/com.pdflector.app/files/pdfs/doc.pdf",
//!   "page": 3, "zoom": 1.0, "dark": false }
//! ```
//!
//! - `path`: ruta local ABSOLUTA del PDF abierto (clave del documento). Para
//!   la biblioteca y "abrir con" es la copia en `internal/pdfs/` (idempotente
//!   entre sesiones); para el picker, la ruta elegida.
//! - `page`: 0-based. `zoom`: factor continuo (1.0 = escala inicial de
//!   apertura). `dark`: modo oscuro activo.
//!
//! ## Política de guardado y restauración (documentada)
//!
//! - **Eager**: se guarda en cada cambio de página (`next_page`/`prev_page`/
//!   `jump_page`), al soltar un pinch (`set_zoom_sharp` — NO durante el
//!   gesto, para no escribir en cada Move de 60-120 Hz), al alternar el modo
//!   oscuro y al abrir un documento. Un cierre inesperado no pierde posición.
//! - **Restauración**: al arrancar SIN intent, `Reader::new` lee el estado;
//!   si `path` sigue existiendo, abre el PDF directamente en esa
//!   página/zoom/modo. Si el fichero ya no existe (o no se puede abrir),
//!   BORRA el estado y abre la biblioteca MediaStore — el estado huérfano no
//!   se conserva. Sin estado guardado → biblioteca.
//! - La escritura es best-effort: un fallo de disco solo se loguea, no rompe
//!   la app.

use std::fs;
use std::path::{Path, PathBuf};

use log::{error, info, warn};
use pdf_core::Color;
use serde::{Deserialize, Serialize};

use crate::annotations::{DEFAULT_INK_COLOR, STROKE_WIDTH_PT};

/// Máximo de entradas de la lista de recientes (los últimos PDFs abiertos).
pub(crate) const RECENTS_MAX: usize = 10;

// ---------------------------------------------------------------------------
// Progreso por libro (`library.json`): la biblioteca personal premium
// ---------------------------------------------------------------------------
//
// Además del estado del visor (`state.json`) y del histórico de recientes
// (`recents.json`), la biblioteca persiste un REGISTRO DE PROGRESO por libro
// (`internal/library.json`): para cada PDF abierto guarda la página actual,
// el total de páginas, el último momento de lectura y el momento en que se
// añadió a la biblioteca. De ahí se DERIVAN (sin abrir el PDF):
//
// - `page_count` → "Page X of Y" y la barra de progreso (se guarda al abrir
//   el documento; abrir cada PDF solo para contar páginas rompería el frame
//   time y el presupuesto de RAM, AGENTS.md §2/§8).
// - `progress% = (page+1) / page_count`.
// - `status` = Unread (nunca abierto: sin registro) / Reading (abierto, no
//   terminado) / Finished (última página alcanzada).
// - Sort "Recently Added" (added_unix) y "Recently Read" (last_read_unix).
//
// Clave: la ruta local ABSOLUTA del PDF (la misma de `state.json` y
// `recents.json`); para los libros de la biblioteca MediaStore es la copia
// en `internal/pdfs/` (`Reader::entry_path`). El registro se CREA al abrir
// un PDF por primera vez (added_unix = ahora) y se actualiza en cada cambio
// de página / apertura (escritura *eager*, mismo patrón que `state.json`;
// un cierre inesperado no pierde la posición). Un registro huérfano (PDF
// borrado) es inofensivo: los libros se unen por nombre/ruta al listar.

/// Registro de progreso de un libro (una entrada de `library.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct BookProgress {
    /// Ruta local absoluta del PDF (clave del documento; coincide con
    /// `state.json`/`recents.json`).
    pub(crate) path: String,
    /// Página actual, 0-based.
    pub(crate) page: u32,
    /// Total de páginas del documento (guardado al abrir).
    pub(crate) page_count: u32,
    /// Unix timestamp (segundos) de la última lectura.
    pub(crate) last_read_unix: i64,
    /// Unix timestamp (segundos) de cuando se añadió a la biblioteca.
    pub(crate) added_unix: i64,
}

impl BookProgress {
    /// Porcentaje leído (0.0-1.0): `(page+1) / page_count`; 0 si el total
    /// no se conoce (defensa). Lo consumen la barra de progreso y el meta
    /// "Page X of Y · Z%" de las tarjetas/celdas.
    pub(crate) fn pct(&self) -> f32 {
        if self.page_count == 0 {
            0.0
        } else {
            (self.page as f32 + 1.0) / self.page_count as f32
        }
    }

    /// ¿Terminado? Se alcanzó la última página (`page+1 >= page_count`): el
    /// libro pasa a `Finished` y deja de aparecer en "Continue Reading".
    pub(crate) fn is_finished(&self) -> bool {
        self.page_count > 0 && self.page + 1 >= self.page_count
    }
}

/// Ruta del fichero de progreso por libro dentro del directorio interno.
pub(crate) fn library_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("library.json")
}

/// Timestamp UNIX actual (segundos) para los sellos de `library.json`.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Carga el registro de progreso; vacío si no existe, no se puede leer o
/// está corrupto (best-effort, como el resto de la persistencia).
pub(crate) fn load_progress(internal_dir: Option<&Path>) -> Vec<BookProgress> {
    let Some(dir) = internal_dir else {
        return Vec::new();
    };
    let text = match fs::read_to_string(library_path(dir)) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<BookProgress>>(&text) {
        Ok(list) => {
            info!("library progress loaded: {} books", list.len());
            list
        }
        Err(e) => {
            warn!("library.json corrupt ({e}): ignoring");
            Vec::new()
        }
    }
}

/// Escribe el registro de progreso (best-effort: un fallo de escritura solo
/// se loguea, igual que `save_state`).
pub(crate) fn save_progress(internal_dir: Option<&Path>, books: &[BookProgress]) {
    let Some(dir) = internal_dir else {
        return;
    };
    let path = library_path(dir);
    let Ok(text) = serde_json::to_string_pretty(books) else {
        return;
    };
    if let Err(e) = fs::write(&path, text) {
        error!("save progress {}: {e}", path.display());
    }
}

/// Busca el registro de progreso de `path` (clave del documento).
pub(crate) fn progress_for<'a>(books: &'a [BookProgress], path: &str) -> Option<&'a BookProgress> {
    books.iter().find(|b| b.path == path)
}

/// Actualiza (o crea) el registro de progreso de `path` con la página
/// actual y el total, sellando `last_read_unix = now`; la primera vez fija
/// también `added_unix = now`. Devuelve la lista NUEVA (el guardado en disco
/// lo hace el llamador). Función PURA (sin reloj ni fs): recibe `now` para
/// poder testearla.
pub(crate) fn touch_progress(
    books: &[BookProgress],
    path: &str,
    page: u32,
    page_count: u32,
    now: i64,
) -> Vec<BookProgress> {
    let mut out = books.to_vec();
    if let Some(b) = out.iter_mut().find(|b| b.path == path) {
        b.page = page;
        b.page_count = page_count;
        b.last_read_unix = now;
    } else {
        out.push(BookProgress {
            path: path.to_string(),
            page,
            page_count,
            last_read_unix: now,
            added_unix: now,
        });
    }
    out
}

fn default_ink_width() -> f32 {
    STROKE_WIDTH_PT
}
fn default_ink_color() -> Color {
    DEFAULT_INK_COLOR
}

/// Estado de herramientas (color y grosor del boli), global y persistido
/// en `tool_state.json` (no por documento). Se carga al arrancar y se guarda
/// al ciclar color/grosor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct ToolState {
    #[serde(default = "default_ink_color")]
    pub(crate) ink_color: Color,
    #[serde(default = "default_ink_width")]
    pub(crate) ink_width: f32,
}
impl Default for ToolState {
    fn default() -> Self {
        Self {
            ink_color: DEFAULT_INK_COLOR,
            ink_width: STROKE_WIDTH_PT,
        }
    }
}
pub(crate) fn tool_state_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("tool_state.json")
}
pub(crate) fn load_tool_state(internal_dir: Option<&Path>) -> ToolState {
    let Some(dir) = internal_dir else {
        return ToolState::default();
    };
    let text = match fs::read_to_string(tool_state_path(dir)) {
        Ok(t) => t,
        Err(_) => return ToolState::default(),
    };
    serde_json::from_str::<ToolState>(&text).unwrap_or_default()
}
pub(crate) fn save_tool_state(internal_dir: Option<&Path>, state: &ToolState) {
    let Some(dir) = internal_dir else {
        return;
    };
    let path = tool_state_path(dir);
    let Ok(text) = serde_json::to_string_pretty(state) else {
        return;
    };
    if let Err(e) = fs::write(&path, text) {
        error!("save tool_state {}: {e}", path.display());
    }
}

use crate::theme::AppTheme;

/// Estado persistido del visor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct ViewerState {
    /// Ruta local absoluta del PDF abierto (clave del documento).
    pub(crate) path: String,
    /// Página actual, 0-based.
    pub(crate) page: u32,
    /// Factor de zoom continuo (1.0 = escala inicial de apertura).
    pub(crate) zoom: f32,
    /// Modo oscuro activo (página invertida + fondo negro).
    pub(crate) dark: bool,
    /// Tema visual activo (Readest).
    #[serde(default)]
    pub(crate) theme: Option<AppTheme>,
}

/// Ruta del fichero de estado dentro del directorio interno de la app.
pub(crate) fn state_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("state.json")
}

/// Una entrada de la lista de recientes: PDF abierto antes (ruta local
/// ABSOLUTA — la clave del documento — y nombre mostrable).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct RecentEntry {
    /// Ruta local absoluta del PDF (se abre con `MupdfEngine::open`).
    pub(crate) path: String,
    /// Nombre de fichero (se muestra bajo la portada del carousel).
    pub(crate) name: String,
}

/// Ruta del fichero de recientes dentro del directorio interno de la app.
/// SEPARADO de `state.json` a propósito: `state.json` es el estado del visor
/// y se BORRA cuando su PDF ya no es accesible (`clear_state`); los recientes
/// son un histórico que debe sobrevivir a eso.
pub(crate) fn recents_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("recents.json")
}

/// Carga la lista de recientes; vacía si no existe, no se puede leer o está
/// corrupta (best-effort, como el resto de la persistencia).
pub(crate) fn load_recents(internal_dir: Option<&Path>) -> Vec<RecentEntry> {
    let Some(dir) = internal_dir else {
        return Vec::new();
    };
    let text = match fs::read_to_string(recents_path(dir)) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<RecentEntry>>(&text) {
        Ok(list) => {
            info!("recents loaded: {} entries", list.len());
            list
        }
        Err(e) => {
            warn!("recents.json corrupt ({e}): ignoring");
            Vec::new()
        }
    }
}

/// Escribe la lista de recientes (best-effort: un fallo de escritura solo se
/// loguea, igual que `save_state`).
pub(crate) fn save_recents(internal_dir: Option<&Path>, recents: &[RecentEntry]) {
    let Some(dir) = internal_dir else {
        return;
    };
    let path = recents_path(dir);
    let Ok(text) = serde_json::to_string_pretty(recents) else {
        return;
    };
    if let Err(e) = fs::write(&path, text) {
        error!("save recents {}: {e}", path.display());
    }
}

/// Añade (o reordena) un PDF abierto al frente de la lista de recientes:
/// deduplica por ruta (un PDF ya presente pasa al frente sin duplicarse) y
/// recorta a `RECENTS_MAX` (los más recientes primero). Devuelve la lista
/// nueva; el guardado en disco lo hace el llamador (`Reader::touch_recent`).
pub(crate) fn push_recent(recents: &[RecentEntry], path: String, name: String) -> Vec<RecentEntry> {
    let mut out: Vec<RecentEntry> = recents.iter().filter(|r| r.path != path).cloned().collect();
    out.insert(0, RecentEntry { path, name });
    out.truncate(RECENTS_MAX);
    out
}

/// Carga el estado guardado; `None` si no existe, no se puede leer o está
/// corrupto (se trata como "sin estado" → biblioteca; un JSON corrupto se
/// sobrescribe en el siguiente guardado).
pub(crate) fn load_state(internal_dir: Option<&Path>) -> Option<ViewerState> {
    let dir = internal_dir?;
    let text = fs::read_to_string(state_path(dir)).ok()?;
    match serde_json::from_str::<ViewerState>(&text) {
        Ok(state) => {
            info!(
                "state loaded: page {} zoom {:.3} dark {}",
                state.page + 1,
                state.zoom,
                state.dark
            );
            Some(state)
        }
        Err(e) => {
            warn!("state.json corrupt ({e}): ignoring");
            None
        }
    }
}

/// Escribe el estado (best-effort: un fallo de escritura solo se loguea).
pub(crate) fn save_state(internal_dir: Option<&Path>, state: &ViewerState) {
    let Some(dir) = internal_dir else {
        return;
    };
    let path = state_path(dir);
    let Ok(text) = serde_json::to_string_pretty(state) else {
        return;
    };
    if let Err(e) = fs::write(&path, text) {
        error!("save state {}: {e}", path.display());
    }
}

/// Borra el estado guardado (PDF ya no accesible / no se pudo abrir).
pub(crate) fn clear_state(internal_dir: Option<&Path>) {
    let Some(dir) = internal_dir else {
        return;
    };
    let path = state_path(dir);
    if path.exists()
        && let Err(e) = fs::remove_file(&path)
    {
        error!("clear state {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_creates_record_with_added_and_last_read() {
        let books = touch_progress(&[], "/x/a.pdf", 3, 100, 1000);
        assert_eq!(books.len(), 1);
        let b = progress_for(&books, "/x/a.pdf").unwrap();
        assert_eq!(b.page, 3);
        assert_eq!(b.page_count, 100);
        assert_eq!(b.last_read_unix, 1000);
        assert_eq!(b.added_unix, 1000);
    }

    #[test]
    fn touch_updates_existing_without_resetting_added() {
        let books = touch_progress(&[], "/x/a.pdf", 0, 100, 1000);
        let books = touch_progress(&books, "/x/a.pdf", 50, 100, 2000);
        assert_eq!(books.len(), 1);
        let b = progress_for(&books, "/x/a.pdf").unwrap();
        assert_eq!(b.page, 50);
        assert_eq!(b.last_read_unix, 2000);
        assert_eq!(b.added_unix, 1000); // no se resetea
    }

    #[test]
    fn touch_keeps_other_books() {
        let books = touch_progress(&[], "/x/a.pdf", 0, 10, 1);
        let books = touch_progress(&books, "/x/b.pdf", 5, 20, 2);
        assert_eq!(books.len(), 2);
        assert!(progress_for(&books, "/x/a.pdf").is_some());
    }

    #[test]
    fn pct_and_finished_derivation() {
        let b = BookProgress {
            path: "/x/a.pdf".into(),
            page: 49,
            page_count: 100,
            last_read_unix: 1,
            added_unix: 1,
        };
        assert!((b.pct() - 0.5).abs() < 1e-6);
        assert!(!b.is_finished());
        let done = BookProgress {
            page: 99,
            ..b.clone()
        };
        assert!(done.is_finished());
        let empty = BookProgress {
            page_count: 0,
            ..b.clone()
        };
        assert_eq!(empty.pct(), 0.0);
        assert!(!empty.is_finished());
    }
}
