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
use serde::{Deserialize, Serialize};

/// Máximo de entradas de la lista de recientes (los últimos PDFs abiertos).
pub(crate) const RECENTS_MAX: usize = 10;

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
