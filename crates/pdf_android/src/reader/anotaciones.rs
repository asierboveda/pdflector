// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Anotaciones y modo del boli: carga y guardado del sidecar (`load_annotations`, `save_annotations`), modo del lápiz y su persistencia (`set_pen_mode`, `persist_tool_state`) y deshacer/rehacer (`record_annotation_edit`, `undo_annotation`, `redo_annotation`).

use super::Reader;
use crate::annotations::PenMode;
use crate::undo::AnnotationEdit;
use log::error;
use log::info;
use pdf_core::AnnotationSet;
use pdf_core::store::AnnotationStore;
use pdf_core::store::sidecar_path;
use std::path::Path;

impl Reader {
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
    pub(crate) fn load_annotations(&mut self, path: &str) {
        // Complete the previous document's queued snapshot before changing its
        // target sidecar; this only signals the actor and never waits for I/O.
        self.folder_sync.flush();
        let sidecar = sidecar_path(Path::new(path));
        let sidecar_existed = sidecar.is_file();
        let latest_snapshot = self.folder_sync.latest_snapshot(&sidecar);
        let mut set = match AnnotationStore::open(&sidecar) {
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
        if let Some(latest) = latest_snapshot {
            set = latest;
        } else if !sidecar_existed
            && let Some(imported) = self
                .synced_documents
                .iter()
                .find(|document| document.local_path == Path::new(path))
            && let Some(restored) = self.synced_annotations.get(&imported.id)
        {
            set = restored.clone();
            info!(
                "annotations: restored {} from portable folder backup",
                set.len()
            );
        }
        self.annotation_backup_restore_allowed = !sidecar_existed;
        self.annotations = set;
        self.annot_sidecar = Some(sidecar);
    }

    /// Guarda el `AnnotationSet` completo en el sidecar del documento abierto
    /// (`AnnotationStore::save` — reescritura transaccional del set, O(n) con
    /// n = nº de anotaciones; se llama solo en acciones de usuario, nunca por
    /// frame). Best-effort: un fallo solo se loguea, no rompe el dibujo.
    ///
    /// Lo llaman las rutas de anotación (tinta, subrayador, goma). El camino de
    /// guardado es el mismo que usaba el modo dibujo eliminado; el modelo de
    /// anotaciones sigue siendo persistible y exportable.
    pub(crate) fn save_annotations(&mut self) {
        self.annotation_backup_restore_allowed = false;
        let Some(sidecar) = self.annot_sidecar.clone() else {
            return;
        };
        let Some(pdf_path) = self.doc_path.as_ref().map(Path::new) else {
            error!("annotations save skipped: current PDF path unavailable");
            return;
        };
        let publish = self
            .synced_documents
            .iter()
            .any(|document| document.local_path == pdf_path);
        let count = self.annotations.len();
        let job = crate::folder_sync::PendingSave {
            pdf_path: pdf_path.to_path_buf(),
            sidecar,
            annotations: self.annotations.clone(),
            publish,
            persisted: false,
        };
        if let Err(reason) = self.folder_sync.save(job.clone()) {
            error!("annotations save queue rejected snapshot: {reason}");
            let detail = self.defer_annotation_save(job);
            self.status = Some(format!("Annotations pending: {detail}"));
            self.list_dirty = true;
            self.redraw();
        } else {
            info!("annotations queued ({} total) for serialized save", count);
            let label = self
                .synced_documents
                .iter()
                .find(|document| document.local_path == pdf_path)
                .map(|document| document.relative_path.display().to_string());
            self.status = Some(match label {
                Some(path) => format!("Saving annotations: {path}"),
                None => "Saving annotations…".to_string(),
            });
            self.redraw();
        }
    }

    /// Fija la herramienta del lápiz (barra), la persiste y refresca la
    /// barra. Un gesto en curso o una selección de Recorte se cancelan sin
    /// guardar nada a medias (ADR-012).
    pub(crate) fn set_pen_mode(&mut self, mode: PenMode) {
        if self.pen_mode == mode {
            return;
        }
        self.cancel_tool_gesture();
        self.clear_recorte();
        self.pen_mode = mode;
        self.persist_tool_state();
        self.invalidate_toolbar();
    }

    /// Persiste modo, color/grosor del boli, color del resaltador y plegado
    /// de la barra en `tool_state.json`.
    pub(crate) fn persist_tool_state(&self) {
        let state = crate::persist::ToolState {
            ink_color: self.ink_color,
            ink_width: self.ink_width,
            highlight_color: self.highlight_color,
            mode: self.pen_mode,
            toolbar_collapsed: self.toolbar_collapsed,
            toolbar_dock: self.toolbar_dock,
        };
        crate::persist::save_tool_state(self.internal_dir.as_deref(), &state);
    }

    /// Registra una acción del usuario sobre las anotaciones en el historial
    /// de deshacer. Refresca la barra solo si cambia la disponibilidad de
    /// sus botones ↶/↷ (evita re-renderizarla tras cada trazo).
    pub(crate) fn record_annotation_edit(&mut self, edit: AnnotationEdit) {
        let before = (self.undo.can_undo(), self.undo.can_redo());
        self.undo.push(edit);
        if before != (self.undo.can_undo(), self.undo.can_redo()) {
            self.invalidate_toolbar();
        }
    }

    /// Deshace la última acción sobre las anotaciones (botón ↶ de la barra).
    pub(crate) fn undo_annotation(&mut self) {
        self.clear_recorte();
        let page = self.undo.undo(&mut self.annotations);
        self.after_history_step(page);
    }

    /// Rehace la última acción deshecha (botón ↷ de la barra).
    pub(crate) fn redo_annotation(&mut self) {
        self.clear_recorte();
        let page = self.undo.redo(&mut self.annotations);
        self.after_history_step(page);
    }

    /// Tras deshacer/rehacer: recompone la capa Dry, guarda el sidecar,
    /// refresca la barra y lleva la vista a la página afectada si es otra.
    fn after_history_step(&mut self, page: Option<usize>) {
        self.invalidate_toolbar();
        let Some(page) = page else {
            return;
        };
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.invalidate_dry();
        }
        self.save_annotations();
        if page as u32 != self.page {
            self.jump_page(page as i32 - self.page as i32);
        }
        self.mark_repaint();
    }

    /// Olvida el historial de deshacer (cambio de documento o salida del
    /// lector) y cierra el popover de la barra.
    pub(crate) fn reset_undo_history(&mut self) {
        self.clear_recorte();
        self.undo.clear();
        self.erase_edit = AnnotationEdit::default();
        self.close_toolbar_popover();
        self.invalidate_toolbar();
    }
}
