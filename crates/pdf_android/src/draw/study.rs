// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Panel de Estudio del visor (PR-02): cuaderno del documento (lista de
//! notas), editor de nota y editor de la clave de Gemini. `study_layout` es
//! la fuente ÚNICA de geometría: la usan el render (`render_study_panel`) y
//! el hit-test del tap (`study_hit`), igual que `menus.rs` y el sheet.

use crate::notebook::Note;

use super::ButtonRect;

/// Pantalla del panel de Estudio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudyView {
    /// Lista de notas del documento (más reciente primero).
    Notebook,
    /// Editor de una nota (nueva o existente).
    NoteEditor,
    /// Editor de la clave de Gemini.
    KeyEditor,
}

/// Resultado del hit-test de un tap sobre el panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StudyHit {
    Close,
    NewNote,
    ScrollUp,
    ScrollDown,
    /// Tap en el cuerpo de una nota: saltar a su página.
    OpenNote(u64),
    EditNote(u64),
    DeleteNote(u64),
    /// Campo de texto: reabrir el teclado.
    Field,
    Save,
    Cancel,
    ClearKey,
    /// Dentro del panel sin control (se consume el tap).
    Inside,
}

/// Fila de una nota visible: rect, botones y textos ya envueltos.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NoteRowLayout {
    pub(crate) id: u64,
    pub(crate) rect: ButtonRect,
    pub(crate) edit: ButtonRect,
    pub(crate) delete: ButtonRect,
    /// Línea de metadatos ("Pág. 12 · 2026-11-26").
    pub(crate) meta: String,
    /// Vista previa del texto (como mucho `ROW_PREVIEW_LINES` líneas).
    pub(crate) lines: Vec<String>,
    /// El botón Borrar está pidiendo confirmación.
    pub(crate) confirm_delete: bool,
}

/// Datos que necesita el layout (sin `Reader`: testeable en host).
pub(crate) struct StudyInput<'a> {
    pub(crate) view: StudyView,
    /// Notas del documento, ya ordenadas (más reciente primero).
    pub(crate) notes: &'a [&'a Note],
    /// Primera fila visible de la lista.
    pub(crate) scroll: usize,
    /// Nota con el Borrar pendiente de confirmar.
    pub(crate) confirm_delete: Option<u64>,
    /// Hay una clave guardada (muestra «Borrar clave»).
    pub(crate) has_key: bool,
}

/// Layout del panel en px de ventana.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StudyLayout {
    pub(crate) rect: ButtonRect,
    pub(crate) buttons: Vec<(StudyHit, ButtonRect)>,
    pub(crate) rows: Vec<NoteRowLayout>,
    pub(crate) field: Option<ButtonRect>,
    pub(crate) scroll: usize,
    pub(crate) max_scroll: usize,
    /// Caracteres por línea del campo de texto (envoltorio del borrador).
    pub(crate) field_chars: usize,
}

/// Líneas de vista previa por fila de nota.
pub(crate) const ROW_PREVIEW_LINES: usize = 2;
/// Líneas visibles del campo de texto del editor.
pub(crate) const FIELD_LINES: usize = 4;

/// Layout del panel (fuente única de dibujo e hit-test).
pub(crate) fn study_layout(_win_w: f32, _win_h: f32, _input: &StudyInput) -> StudyLayout {
    StudyLayout {
        rect: (0.0, 0.0, 0.0, 0.0),
        buttons: Vec::new(),
        rows: Vec::new(),
        field: None,
        scroll: 0,
        max_scroll: 0,
        field_chars: 1,
    }
}

/// Hit-test de un tap: None si cae FUERA del panel.
pub(crate) fn study_hit(_layout: &StudyLayout, _x: f32, _y: f32) -> Option<StudyHit> {
    None
}

/// Envoltorio de texto plano por palabras a `max_chars` columnas (respeta
/// los saltos de línea; parte palabras más largas que la línea).
pub(crate) fn wrap_plain(_text: &str, _max_chars: usize) -> Vec<String> {
    Vec::new()
}

/// Metadatos de la fila: página (1-based) y fecha de creación.
pub(crate) fn note_meta(_note: &Note) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u64, text: &str, page: Option<u32>) -> Note {
        Note {
            id,
            text: text.to_string(),
            page,
            created_unix: 1_795_694_399,
            edited_unix: 1_795_694_399,
        }
    }

    fn center(r: ButtonRect) -> (f32, f32) {
        ((r.0 + r.2) * 0.5, (r.1 + r.3) * 0.5)
    }

    fn inside(outer: ButtonRect, r: ButtonRect) -> bool {
        r.0 >= outer.0 && r.1 >= outer.1 && r.2 <= outer.2 && r.3 <= outer.3
    }

    fn overlap(a: ButtonRect, b: ButtonRect) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    fn input<'a>(view: StudyView, notes: &'a [&'a Note], scroll: usize) -> StudyInput<'a> {
        StudyInput {
            view,
            notes,
            scroll,
            confirm_delete: None,
            has_key: false,
        }
    }

    const SIZES: [(f32, f32); 3] = [(1200.0, 2000.0), (2000.0, 1200.0), (800.0, 1280.0)];

    #[test]
    fn notebook_lists_rows_in_given_order_and_fits_the_window() {
        let owned: Vec<Note> = (1..=3)
            .rev()
            .map(|i| note(i, &format!("nota {i}"), Some(i as u32)))
            .collect();
        let notes: Vec<&Note> = owned.iter().collect();
        for (w, h) in SIZES {
            let l = study_layout(w, h, &input(StudyView::Notebook, &notes, 0));
            assert!(inside((0.0, 0.0, w, h), l.rect), "{:?}", l.rect);
            let ids: Vec<u64> = l.rows.iter().map(|r| r.id).collect();
            assert_eq!(ids, vec![3, 2, 1]);
            assert_eq!(l.max_scroll, 0);
            let tags: Vec<StudyHit> = l.buttons.iter().map(|(t, _)| *t).collect();
            assert!(tags.contains(&StudyHit::Close), "{tags:?}");
            assert!(tags.contains(&StudyHit::NewNote), "{tags:?}");
            assert!(!tags.contains(&StudyHit::ScrollDown), "{tags:?}");
            // Botones y filas dentro del panel y sin solapes.
            let mut all: Vec<ButtonRect> = l.buttons.iter().map(|(_, r)| *r).collect();
            for row in &l.rows {
                assert!(inside(l.rect, row.rect), "{row:?}");
                assert!(inside(row.rect, row.edit) && inside(row.rect, row.delete));
                assert!(!overlap(row.edit, row.delete));
                all.push(row.rect);
            }
            for (i, a) in all.iter().enumerate() {
                assert!(inside(l.rect, *a), "{a:?}");
                for b in all.iter().skip(i + 1) {
                    assert!(!overlap(*a, *b), "{a:?} solapa {b:?}");
                }
            }
        }
    }

    #[test]
    fn long_notebook_scrolls_by_rows() {
        let owned: Vec<Note> = (1..=40).rev().map(|i| note(i, "x", None)).collect();
        let notes: Vec<&Note> = owned.iter().collect();
        let l = study_layout(1200.0, 2000.0, &input(StudyView::Notebook, &notes, 0));
        assert!(l.rows.len() < 40 && !l.rows.is_empty());
        assert_eq!(l.max_scroll, 40 - l.rows.len());
        let tags: Vec<StudyHit> = l.buttons.iter().map(|(t, _)| *t).collect();
        assert!(tags.contains(&StudyHit::ScrollUp) && tags.contains(&StudyHit::ScrollDown));
        let l2 = study_layout(1200.0, 2000.0, &input(StudyView::Notebook, &notes, 5));
        assert_eq!(l2.rows[0].id, 35);
        // Scroll fuera de rango: se recorta al máximo.
        let l3 = study_layout(1200.0, 2000.0, &input(StudyView::Notebook, &notes, 999));
        assert_eq!(l3.scroll, l3.max_scroll);
        assert_eq!(l3.rows.last().map(|r| r.id), Some(1));
    }

    #[test]
    fn hit_test_uses_the_same_rects_as_the_drawing() {
        let owned = [note(7, "con página", Some(4)), note(5, "general", None)];
        let notes: Vec<&Note> = owned.iter().collect();
        let l = study_layout(1200.0, 2000.0, &input(StudyView::Notebook, &notes, 0));
        for (tag, r) in &l.buttons {
            let (x, y) = center(*r);
            assert_eq!(study_hit(&l, x, y), Some(*tag));
        }
        let row = &l.rows[0];
        let (x, y) = center(row.edit);
        assert_eq!(study_hit(&l, x, y), Some(StudyHit::EditNote(7)));
        let (x, y) = center(row.delete);
        assert_eq!(study_hit(&l, x, y), Some(StudyHit::DeleteNote(7)));
        assert_eq!(
            study_hit(&l, row.rect.0 + 12.0, (row.rect.1 + row.rect.3) * 0.5),
            Some(StudyHit::OpenNote(7))
        );
        assert_eq!(
            study_hit(&l, l.rows[1].rect.0 + 12.0, l.rows[1].rect.1 + 12.0),
            Some(StudyHit::OpenNote(5))
        );
        // Fuera del panel: None (cierra); dentro sin control: Inside.
        assert_eq!(study_hit(&l, l.rect.0 - 5.0, l.rect.1 - 5.0), None);
        assert_eq!(
            study_hit(&l, l.rect.0 + 4.0, l.rect.3 - 4.0),
            Some(StudyHit::Inside)
        );
    }

    #[test]
    fn rows_show_page_number_and_preview() {
        let long = "palabra ".repeat(200);
        let owned = [note(1, &long, Some(11)), note(2, "corta", None)];
        let notes: Vec<&Note> = owned.iter().collect();
        let l = study_layout(1200.0, 2000.0, &input(StudyView::Notebook, &notes, 0));
        assert!(l.rows[0].meta.contains("Pág. 12"), "{}", l.rows[0].meta);
        assert!(l.rows[0].meta.contains("2026-11-26"), "{}", l.rows[0].meta);
        assert!(!l.rows[1].meta.contains("Pág."), "{}", l.rows[1].meta);
        assert_eq!(l.rows[0].lines.len(), ROW_PREVIEW_LINES);
        assert!(l.rows[0].lines[1].ends_with('…'), "{:?}", l.rows[0].lines);
        assert_eq!(l.rows[1].lines, vec!["corta".to_string()]);
        let mut confirm = input(StudyView::Notebook, &notes, 0);
        confirm.confirm_delete = Some(2);
        let l = study_layout(1200.0, 2000.0, &confirm);
        assert!(!l.rows[0].confirm_delete && l.rows[1].confirm_delete);
    }

    #[test]
    fn editors_sit_above_the_keyboard_and_offer_save_cancel() {
        for (w, h) in SIZES {
            for view in [StudyView::NoteEditor, StudyView::KeyEditor] {
                for has_key in [false, true] {
                    let mut i = input(view, &[], 0);
                    i.has_key = has_key;
                    let l = study_layout(w, h, &i);
                    assert!(inside((0.0, 0.0, w, h), l.rect));
                    assert!(l.rect.3 <= h * 0.5, "{view:?} {:?} tapa el teclado", l.rect);
                    let field = l.field.expect("campo de texto");
                    assert!(inside(l.rect, field));
                    assert!(l.field_chars >= 10);
                    let tags: Vec<StudyHit> = l.buttons.iter().map(|(t, _)| *t).collect();
                    assert!(tags.contains(&StudyHit::Save), "{tags:?}");
                    assert!(tags.contains(&StudyHit::Cancel), "{tags:?}");
                    let clear = view == StudyView::KeyEditor && has_key;
                    assert_eq!(tags.contains(&StudyHit::ClearKey), clear, "{view:?}");
                    let (x, y) = center(field);
                    assert_eq!(study_hit(&l, x, y), Some(StudyHit::Field));
                    for (i, (_, a)) in l.buttons.iter().enumerate() {
                        assert!(!overlap(*a, field));
                        for (_, b) in l.buttons.iter().skip(i + 1) {
                            assert!(!overlap(*a, *b), "{a:?} {b:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wrap_plain_breaks_words_and_lines() {
        assert_eq!(
            wrap_plain("hola mundo cruel", 10),
            vec!["hola mundo", "cruel"]
        );
        assert_eq!(wrap_plain("a\nb", 10), vec!["a", "b"]);
        assert_eq!(wrap_plain("abcdefghijkl", 5), vec!["abcde", "fghij", "kl"]);
        assert_eq!(wrap_plain("ñandú ñandú", 5), vec!["ñandú", "ñandú"]);
        assert!(wrap_plain("", 5).is_empty());
    }
}
