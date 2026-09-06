// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Anotaciones y modo del boli (extraído de `reader.rs`, 2026-09-06): carga y guardado del sidecar (`load_annotations`, `save_annotations`) y la persistencia del modo de tinta (`toggle_pen_mode`, `persist_pen_mode`).

use super::Reader;
use crate::annotations::PenMode;
use log::error;
use log::info;
use pdf_core::store::AnnotationStore;
use pdf_core::store::sidecar_path;
use pdf_core::{
    Annotation, AnnotationSet, Bitmap, Color, Document, Gesture, Highlight, PageTextCache, Rect,
    RenderEngine, Stroke, TextSpan,
};
use std::fs;
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
    pub(crate) fn save_annotations(&self) {
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
}
