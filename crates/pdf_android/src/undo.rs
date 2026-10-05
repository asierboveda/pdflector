// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Historial de deshacer/rehacer de las anotaciones del documento abierto.
//!
//! Cada acción del usuario (trazo, subrayado, pasada de goma, mover o
//! escalar con Recorte) se registra como un [`AnnotationEdit`]: las
//! anotaciones que QUITÓ (copia completa), los ids que AÑADIÓ y las que
//! MODIFICÓ en su sitio (versión anterior y posterior, mismo id). Deshacer
//! aplica la edición inversa sobre el `AnnotationSet` (quita lo añadido,
//! re-añade lo quitado, restaura lo modificado) y guarda esa inversa en la
//! pila de rehacer; rehacer hace lo simétrico.
//!
//! `AnnotationSet::add` siempre asigna un id NUEVO, así que una anotación
//! re-añadida cambia de id (y pasa al final del orden z de su página). La
//! inversa registra los ids nuevos y el historial reescribe el id viejo en
//! las demás ediciones de ambas pilas (p. ej. "añadí el trazo" sigue
//! apuntando al trazo después de deshacer una pasada de goma que lo partió).
//! Las modificaciones en su sitio conservan el id. El historial vive solo en
//! memoria, es por documento y está acotado a [`UNDO_LIMIT`] ediciones.

use pdf_core::{Annotated, AnnotationSet};
use std::collections::VecDeque;

/// Máximo de ediciones que se pueden deshacer (la más antigua se descarta).
pub(crate) const UNDO_LIMIT: usize = 100;

/// Una edición atómica sobre el `AnnotationSet`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AnnotationEdit {
    /// Anotaciones eliminadas por la edición (copia con su página y tipo).
    removed: Vec<Annotated>,
    /// (página, id) de las anotaciones añadidas por la edición.
    added: Vec<(usize, u64)>,
    /// (antes, después) de las anotaciones modificadas en su sitio.
    changed: Vec<(Annotated, Annotated)>,
}

impl AnnotationEdit {
    /// Edición que solo añadió `id` en `page`.
    pub(crate) fn added(page: usize, id: u64) -> Self {
        Self {
            added: vec![(page, id)],
            ..Self::default()
        }
    }

    /// Edición que modificó anotaciones en su sitio (mismo id).
    pub(crate) fn changed(before: Vec<Annotated>, after: Vec<Annotated>) -> Self {
        Self {
            changed: before.into_iter().zip(after).collect(),
            ..Self::default()
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty() && self.changed.is_empty()
    }

    /// Apunta una anotación añadida durante la edición.
    pub(crate) fn record_added(&mut self, page: usize, id: u64) {
        self.added.push((page, id));
    }

    /// Apunta una anotación eliminada durante la edición. Si la había añadido
    /// esta misma edición (p. ej. un trozo de trazo que la goma vuelve a
    /// partir en la misma pasada), solo se olvida: no existía antes.
    pub(crate) fn record_removed(&mut self, ann: Annotated) {
        if let Some(pos) = self.added.iter().position(|&(_, id)| id == ann.id) {
            self.added.remove(pos);
        } else {
            self.removed.push(ann);
        }
    }

    /// Página principal de la edición (para llevar la vista hasta ella).
    pub(crate) fn page(&self) -> Option<usize> {
        self.added
            .first()
            .map(|&(page, _)| page)
            .or_else(|| self.removed.first().map(|a| a.page_idx))
            .or_else(|| self.changed.first().map(|(a, _)| a.page_idx))
    }

    /// Revierte la edición sobre `set`. Devuelve la edición que la rehace y
    /// los pares (id viejo, id nuevo) de las anotaciones re-añadidas.
    fn revert(self, set: &mut AnnotationSet) -> (Self, Vec<(u64, u64)>) {
        let mut inverse = Self::default();
        let mut remap = Vec::with_capacity(self.removed.len());
        for (page, id) in self.added {
            let found = set.for_page(page).into_iter().find(|a| a.id == id).cloned();
            if let Some(ann) = found {
                set.remove(id);
                inverse.removed.push(ann);
            }
        }
        for ann in self.removed {
            if let Some(id) = set.add(ann.page_idx, ann.kind) {
                inverse.added.push((ann.page_idx, id));
                remap.push((ann.id, id));
            }
        }
        for (before, after) in self.changed {
            if set.restore(std::slice::from_ref(&before)).is_ok() {
                inverse.changed.push((after, before));
            }
        }
        (inverse, remap)
    }

    /// Sustituye ids re-asignados en todas las referencias de la edición.
    fn remap_ids(&mut self, remap: &[(u64, u64)]) {
        let map = |id: &mut u64| {
            if let Some(&(_, new)) = remap.iter().find(|(old, _)| old == id) {
                *id = new;
            }
        };
        for (_, id) in &mut self.added {
            map(id);
        }
        for ann in &mut self.removed {
            map(&mut ann.id);
        }
        for (before, after) in &mut self.changed {
            map(&mut before.id);
            map(&mut after.id);
        }
    }
}

/// Pilas de deshacer/rehacer.
#[derive(Debug, Default)]
pub(crate) struct UndoHistory {
    undo: VecDeque<AnnotationEdit>,
    redo: Vec<AnnotationEdit>,
}

impl UndoHistory {
    /// Registra una edición nueva del usuario: invalida lo que se podía rehacer.
    pub(crate) fn push(&mut self, edit: AnnotationEdit) {
        if edit.is_empty() {
            return;
        }
        self.redo.clear();
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(edit);
    }

    /// Deshace la última edición; devuelve la página afectada.
    pub(crate) fn undo(&mut self, set: &mut AnnotationSet) -> Option<usize> {
        let edit = self.undo.pop_back()?;
        let (inverse, remap) = edit.revert(set);
        self.remap_ids(&remap);
        let page = inverse.page();
        if !inverse.is_empty() {
            self.redo.push(inverse);
        }
        page
    }

    /// Rehace la última edición deshecha; devuelve la página afectada.
    pub(crate) fn redo(&mut self, set: &mut AnnotationSet) -> Option<usize> {
        let edit = self.redo.pop()?;
        let (inverse, remap) = edit.revert(set);
        self.remap_ids(&remap);
        let page = inverse.page();
        if !inverse.is_empty() {
            self.undo.push_back(inverse);
        }
        page
    }

    /// Propaga ids re-asignados a todas las ediciones pendientes.
    fn remap_ids(&mut self, remap: &[(u64, u64)]) {
        if remap.is_empty() {
            return;
        }
        for edit in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            edit.remap_ids(remap);
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Olvida el historial (cambio de documento).
    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_core::{Annotation, Color, Stroke};

    fn stroke(x: f32) -> Annotation {
        let color = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        Annotation::Stroke(Stroke::new(vec![(x, 0.0), (x, 10.0)], 2.0, color).unwrap())
    }

    fn xs(set: &AnnotationSet, page: usize) -> Vec<f32> {
        set.for_page(page)
            .into_iter()
            .map(|a| match &a.kind {
                Annotation::Stroke(s) => s.points[0].0,
                _ => f32::NAN,
            })
            .collect()
    }

    #[test]
    fn undo_and_redo_an_added_stroke() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        let id = set.add(2, stroke(1.0)).unwrap();
        h.push(AnnotationEdit::added(2, id));

        assert_eq!(h.undo(&mut set), Some(2));
        assert!(set.is_empty());
        assert!(!h.can_undo());
        assert!(h.can_redo());

        assert_eq!(h.redo(&mut set), Some(2));
        assert_eq!(xs(&set, 2), vec![1.0]);
        assert!(h.can_undo());
        assert!(!h.can_redo());

        // La re-añadida tiene id nuevo: deshacer otra vez debe encontrarla.
        h.undo(&mut set);
        assert!(set.is_empty());
    }

    #[test]
    fn undo_of_an_erase_restores_the_original_and_drops_the_pieces() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        let original = set.add(0, stroke(5.0)).unwrap();

        // Pasada de goma: el original se parte en dos trozos y uno de ellos
        // vuelve a partirse en la misma pasada.
        let mut edit = AnnotationEdit::default();
        let ann = set.for_page(0)[0].clone();
        set.remove(original);
        edit.record_removed(ann);
        let a = set.add(0, stroke(1.0)).unwrap();
        edit.record_added(0, a);
        let b = set.add(0, stroke(9.0)).unwrap();
        edit.record_added(0, b);
        let piece = set.for_page(0)[1].clone();
        set.remove(b);
        edit.record_removed(piece);
        let c = set.add(0, stroke(8.0)).unwrap();
        edit.record_added(0, c);
        h.push(edit);
        assert_eq!(xs(&set, 0), vec![1.0, 8.0]);

        h.undo(&mut set);
        assert_eq!(xs(&set, 0), vec![5.0]);

        h.redo(&mut set);
        assert_eq!(xs(&set, 0), vec![1.0, 8.0]);
    }

    #[test]
    fn undoing_an_erase_then_the_stroke_removes_the_stroke_and_redo_replays_both() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        let id = set.add(0, stroke(5.0)).unwrap();
        h.push(AnnotationEdit::added(0, id));

        let mut erase = AnnotationEdit::default();
        let ann = set.for_page(0)[0].clone();
        set.remove(id);
        erase.record_removed(ann);
        for x in [1.0, 9.0] {
            let piece = set.add(0, stroke(x)).unwrap();
            erase.record_added(0, piece);
        }
        h.push(erase);

        h.undo(&mut set); // devuelve el trazo entero (id nuevo)
        assert_eq!(xs(&set, 0), vec![5.0]);
        h.undo(&mut set); // debe quitar ese trazo pese al cambio de id
        assert!(set.is_empty());

        h.redo(&mut set);
        assert_eq!(xs(&set, 0), vec![5.0]);
        h.redo(&mut set);
        assert_eq!(xs(&set, 0), vec![1.0, 9.0]);
        h.undo(&mut set);
        h.undo(&mut set);
        assert!(set.is_empty());
    }

    #[test]
    fn undo_and_redo_of_an_in_place_move_keep_the_id() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        let id = set.add(0, stroke(5.0)).unwrap();
        let t = pdf_core::UniformTransform {
            dx: 10.0,
            ..pdf_core::UniformTransform::IDENTITY
        };
        let before = set.transform_strokes(0, &[id], &t).unwrap();
        let after = vec![set.find(id).unwrap().clone()];
        h.push(AnnotationEdit::changed(before, after));
        assert_eq!(xs(&set, 0), vec![15.0]);

        assert_eq!(h.undo(&mut set), Some(0));
        assert_eq!(xs(&set, 0), vec![5.0]);
        assert_eq!(set.ids(), vec![id]);
        h.redo(&mut set);
        assert_eq!(xs(&set, 0), vec![15.0]);
        assert_eq!(set.ids(), vec![id]);
    }

    #[test]
    fn a_move_survives_an_erase_undo_that_reassigns_the_id() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        let id = set.add(0, stroke(5.0)).unwrap();
        h.push(AnnotationEdit::added(0, id));
        let t = pdf_core::UniformTransform {
            dx: 10.0,
            ..pdf_core::UniformTransform::IDENTITY
        };
        let before = set.transform_strokes(0, &[id], &t).unwrap();
        h.push(AnnotationEdit::changed(
            before,
            vec![set.find(id).unwrap().clone()],
        ));
        let mut erase = AnnotationEdit::default();
        erase.record_removed(set.find(id).unwrap().clone());
        set.remove(id);
        h.push(erase);

        h.undo(&mut set); // re-añade el trazo movido con id nuevo
        assert_eq!(xs(&set, 0), vec![15.0]);
        h.undo(&mut set); // deshace el movimiento sobre el id nuevo
        assert_eq!(xs(&set, 0), vec![5.0]);
        h.undo(&mut set); // quita el trazo
        assert!(set.is_empty());
    }

    #[test]
    fn a_new_edit_clears_redo_and_history_is_bounded() {
        let mut set = AnnotationSet::new();
        let mut h = UndoHistory::default();
        for i in 0..(UNDO_LIMIT + 5) {
            let id = set.add(0, stroke(i as f32)).unwrap();
            h.push(AnnotationEdit::added(0, id));
        }
        for _ in 0..UNDO_LIMIT {
            assert!(h.undo(&mut set).is_some());
        }
        assert!(!h.can_undo());
        assert_eq!(set.len(), 5);

        let id = set.add(0, stroke(-1.0)).unwrap();
        h.push(AnnotationEdit::added(0, id));
        assert!(!h.can_redo());
    }

    #[test]
    fn empty_edits_are_ignored() {
        let mut h = UndoHistory::default();
        h.push(AnnotationEdit::default());
        assert!(!h.can_undo());
    }
}
