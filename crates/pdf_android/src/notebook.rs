// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Cuaderno por documento (PR-02): notas de texto asociadas a cada PDF.
//!
//! ## Formato (`internal/notebooks.json`)
//!
//! ```json
//! { "version": 1,
//!   "documents": [
//!     { "path": "/data/user/0/com.pdflector.app/files/pdfs/doc.pdf",
//!       "notes": [ { "id": 1, "text": "…", "page": 11,
//!                    "created_unix": 1764150000, "edited_unix": 1764150000 } ] } ] }
//! ```
//!
//! - Clave del documento: la ruta local ABSOLUTA del PDF, la MISMA que usan
//!   `library.json`/`state.json` (`persist::BookProgress::path`).
//! - `page` es 0-based y opcional (la página en la que se escribió la nota).
//! - Un fichero único para todos los documentos: pocas notas por libro, y
//!   se escribe completo y atómico (`pdf_core::write_atomic`) en cada cambio.
//! - Instalaciones antiguas sin `notebooks.json` cargan un cuaderno vacío;
//!   un fichero corrupto se aparta a `notebooks.json.corrupt` antes de que
//!   el siguiente guardado lo sustituya (no se pierden notas en silencio).

use std::io;
use std::path::{Path, PathBuf};

use log::warn;
use pdf_core::write_atomic;
use serde::{Deserialize, Serialize};

/// Una nota del cuaderno.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Note {
    /// Identificador único dentro del documento.
    pub(crate) id: u64,
    pub(crate) text: String,
    /// Página (0-based) en la que se escribió; None = nota general.
    #[serde(default)]
    pub(crate) page: Option<u32>,
    pub(crate) created_unix: i64,
    pub(crate) edited_unix: i64,
}

/// Cuaderno de UN documento.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Notebook {
    #[serde(default)]
    pub(crate) notes: Vec<Note>,
}

impl Notebook {
    /// Añade una nota (texto recortado). None si el texto está vacío.
    pub(crate) fn add(&mut self, text: &str, page: Option<u32>, now: i64) -> Option<u64> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let id = self.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        self.notes.push(Note {
            id,
            text: text.to_string(),
            page,
            created_unix: now,
            edited_unix: now,
        });
        Some(id)
    }

    /// Cambia el texto de una nota. false si no existe o el texto está vacío.
    pub(crate) fn edit(&mut self, id: u64, text: &str, now: i64) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        match self.notes.iter_mut().find(|n| n.id == id) {
            Some(n) => {
                n.text = text.to_string();
                n.edited_unix = now;
                true
            }
            None => false,
        }
    }

    /// Borra una nota. false si no existía.
    pub(crate) fn delete(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    pub(crate) fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Notas de la más reciente a la más antigua (creación; empate por id).
    pub(crate) fn newest_first(&self) -> Vec<&Note> {
        let mut out: Vec<&Note> = self.notes.iter().collect();
        out.sort_by(|a, b| {
            b.created_unix
                .cmp(&a.created_unix)
                .then_with(|| b.id.cmp(&a.id))
        });
        out
    }
}

/// Entrada de `notebooks.json`: cuaderno de un documento.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DocNotebook {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) notes: Vec<Note>,
}

/// Contenido completo de `notebooks.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotebookStore {
    #[serde(default = "store_version")]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) documents: Vec<DocNotebook>,
}

fn store_version() -> u32 {
    1
}

impl Default for NotebookStore {
    fn default() -> Self {
        Self {
            version: store_version(),
            documents: Vec::new(),
        }
    }
}

impl NotebookStore {
    /// Cuaderno de `path` (vacío si no tiene notas).
    pub(crate) fn notebook(&self, path: &str) -> Notebook {
        self.documents
            .iter()
            .find(|d| d.path == path)
            .map(|d| Notebook {
                notes: d.notes.clone(),
            })
            .unwrap_or_default()
    }

    /// Sustituye el cuaderno de `path` (un cuaderno vacío borra la entrada).
    pub(crate) fn set(&mut self, path: &str, notebook: &Notebook) {
        if notebook.notes.is_empty() {
            self.documents.retain(|d| d.path != path);
            return;
        }
        match self.documents.iter_mut().find(|d| d.path == path) {
            Some(d) => d.notes = notebook.notes.clone(),
            None => self.documents.push(DocNotebook {
                path: path.to_string(),
                notes: notebook.notes.clone(),
            }),
        }
    }
}

/// Ruta de `notebooks.json` dentro del directorio interno.
pub(crate) fn notebooks_path(internal_dir: &Path) -> PathBuf {
    internal_dir.join("notebooks.json")
}

/// Carga todos los cuadernos (vacío si no hay fichero o directorio). Un
/// fichero corrupto se copia a `notebooks.json.corrupt` (best-effort) para
/// que el siguiente guardado no destruya notas recuperables a mano.
pub(crate) fn load_notebooks(internal_dir: Option<&Path>) -> NotebookStore {
    let Some(dir) = internal_dir else {
        return NotebookStore::default();
    };
    let path = notebooks_path(dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return NotebookStore::default();
    };
    match serde_json::from_str::<NotebookStore>(&text) {
        Ok(store) => store,
        Err(e) => {
            warn!("notebooks.json corrupt ({e}): set aside as notebooks.json.corrupt");
            let _ = write_atomic(&dir.join("notebooks.json.corrupt"), &text);
            NotebookStore::default()
        }
    }
}

/// Cuaderno del documento `path`.
pub(crate) fn load_notebook_for(internal_dir: Option<&Path>, path: &str) -> Notebook {
    load_notebooks(internal_dir).notebook(path)
}

/// Guarda el cuaderno de `path` (relee el fichero, sustituye la entrada y
/// escribe atómico).
pub(crate) fn save_notebook_for(
    internal_dir: Option<&Path>,
    path: &str,
    notebook: &Notebook,
) -> io::Result<()> {
    let Some(dir) = internal_dir else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "internal_data_path unavailable",
        ));
    };
    let mut store = load_notebooks(Some(dir));
    store.set(path, notebook);
    let text = serde_json::to_string_pretty(&store).map_err(io::Error::other)?;
    write_atomic(&notebooks_path(dir), text)
}

/// Página a la que salta el tap sobre una nota (recortada al documento).
pub(crate) fn jump_target(note: &Note, page_count: u32) -> Option<u32> {
    let page = note.page?;
    if page_count == 0 {
        return None;
    }
    Some(page.min(page_count - 1))
}

/// Fecha `AAAA-MM-DD` (UTC) de un timestamp UNIX, para la lista de notas
/// (algoritmo civil-from-days de H. Hinnant; sin dependencias de fecha).
pub(crate) fn format_date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pr02-nb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn add_trims_rejects_empty_and_assigns_unique_ids() {
        let mut nb = Notebook::default();
        let a = nb.add("  primera idea \n", Some(3), 100).unwrap();
        let b = nb.add("segunda", None, 200).unwrap();
        assert_ne!(a, b);
        assert_eq!(nb.add("   ", Some(1), 300), None);
        assert_eq!(nb.notes.len(), 2);
        let n = nb.get(a).unwrap();
        assert_eq!(n.text, "primera idea");
        assert_eq!(n.page, Some(3));
        assert_eq!((n.created_unix, n.edited_unix), (100, 100));
        // Tras borrar, una nota nueva no reutiliza el id de otra viva.
        assert!(nb.delete(a));
        let c = nb.add("tercera", None, 400).unwrap();
        assert_ne!(c, b);
    }

    #[test]
    fn edit_updates_text_and_edited_time_only() {
        let mut nb = Notebook::default();
        let id = nb.add("borrador", Some(7), 100).unwrap();
        assert!(nb.edit(id, " versión final ", 500));
        let n = nb.get(id).unwrap();
        assert_eq!(n.text, "versión final");
        assert_eq!(n.created_unix, 100);
        assert_eq!(n.edited_unix, 500);
        assert_eq!(n.page, Some(7));
        assert!(!nb.edit(id, "  ", 600), "texto vacío: no se aplica");
        assert_eq!(nb.get(id).unwrap().text, "versión final");
        assert!(!nb.edit(999, "x", 600));
    }

    #[test]
    fn delete_removes_only_that_note() {
        let mut nb = Notebook::default();
        let a = nb.add("a", None, 1).unwrap();
        let b = nb.add("b", None, 2).unwrap();
        assert!(nb.delete(a));
        assert!(!nb.delete(a));
        assert!(nb.get(a).is_none());
        assert!(nb.get(b).is_some());
    }

    #[test]
    fn newest_first_orders_by_creation_then_id() {
        let mut nb = Notebook::default();
        let a = nb.add("vieja", None, 100).unwrap();
        let b = nb.add("nueva", None, 300).unwrap();
        let c = nb.add("misma hora", None, 300).unwrap();
        let d = nb.add("media", None, 200).unwrap();
        // Editar no reordena (el orden es por creación).
        nb.edit(a, "vieja editada", 999);
        let ids: Vec<u64> = nb.newest_first().iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![c, b, d, a]);
    }

    #[test]
    fn store_keeps_documents_apart_and_drops_empty_ones() {
        let mut store = NotebookStore::default();
        let mut a = Notebook::default();
        a.add("de A", Some(0), 1);
        let mut b = Notebook::default();
        b.add("de B", None, 2);
        store.set("/x/a.pdf", &a);
        store.set("/x/b.pdf", &b);
        assert_eq!(store.notebook("/x/a.pdf"), a);
        assert_eq!(store.notebook("/x/b.pdf"), b);
        assert_eq!(store.notebook("/x/otro.pdf"), Notebook::default());
        store.set("/x/a.pdf", &Notebook::default());
        assert_eq!(store.documents.len(), 1);
        assert_eq!(store.documents[0].path, "/x/b.pdf");
    }

    #[test]
    fn persistence_round_trip_per_document() {
        let dir = tmp_dir("rt");
        let mut a = Notebook::default();
        let id = a.add("Teorema 2: repasar", Some(11), 1000).unwrap();
        save_notebook_for(Some(&dir), "/x/a.pdf", &a).unwrap();
        let mut b = Notebook::default();
        b.add("Otra", None, 2000);
        save_notebook_for(Some(&dir), "/x/b.pdf", &b).unwrap();
        assert_eq!(load_notebook_for(Some(&dir), "/x/a.pdf"), a);
        assert_eq!(load_notebook_for(Some(&dir), "/x/b.pdf"), b);
        a.edit(id, "Teorema 2: hecho", 3000);
        save_notebook_for(Some(&dir), "/x/a.pdf", &a).unwrap();
        assert_eq!(load_notebook_for(Some(&dir), "/x/a.pdf"), a);
        assert_eq!(load_notebook_for(Some(&dir), "/x/b.pdf"), b);
        let text = std::fs::read_to_string(notebooks_path(&dir)).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_installs_without_notebook_data_load_empty() {
        let dir = tmp_dir("legacy");
        assert_eq!(
            load_notebook_for(Some(&dir), "/x/a.pdf"),
            Notebook::default()
        );
        assert_eq!(load_notebook_for(None, "/x/a.pdf"), Notebook::default());
        assert!(save_notebook_for(None, "/x/a.pdf", &Notebook::default()).is_err());
        // Campos opcionales ausentes (sin página, sin versión).
        std::fs::write(
            notebooks_path(&dir),
            r#"{ "documents": [ { "path": "/x/a.pdf", "notes": [
                 { "id": 4, "text": "sin página", "created_unix": 5, "edited_unix": 5 } ] } ] }"#,
        )
        .unwrap();
        let nb = load_notebook_for(Some(&dir), "/x/a.pdf");
        assert_eq!(nb.notes.len(), 1);
        assert_eq!(nb.notes[0].page, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_set_aside_before_being_replaced() {
        let dir = tmp_dir("corrupt");
        std::fs::write(notebooks_path(&dir), "{ roto").unwrap();
        assert_eq!(
            load_notebook_for(Some(&dir), "/x/a.pdf"),
            Notebook::default()
        );
        let backup = dir.join("notebooks.json.corrupt");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{ roto");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jump_target_clamps_to_the_document() {
        let note = |page| Note {
            id: 1,
            text: "x".into(),
            page,
            created_unix: 0,
            edited_unix: 0,
        };
        assert_eq!(jump_target(&note(Some(4)), 10), Some(4));
        assert_eq!(jump_target(&note(Some(40)), 10), Some(9));
        assert_eq!(jump_target(&note(None), 10), None);
        assert_eq!(jump_target(&note(Some(0)), 0), None);
    }

    #[test]
    fn format_date_is_utc_calendar_day() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(951_782_400), "2000-02-29");
        assert_eq!(format_date(1_795_694_399), "2026-11-26");
    }
}
