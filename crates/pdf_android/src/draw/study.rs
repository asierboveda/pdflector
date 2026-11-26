// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Panel de Estudio del visor (PR-02): cuaderno del documento (lista de
//! notas), editor de nota y editor de la clave de Gemini. `study_layout` es
//! la fuente ÚNICA de geometría: la usan el render (`render_study_panel`) y
//! el hit-test del tap (`study_hit`), igual que `menus.rs` y el sheet.

use crate::notebook::{Note, format_date};

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

/// Tamaño del texto de las notas (mismo cuerpo que el panel de IA).
pub(crate) const STUDY_TEXT_SIZE: f32 = 18.0;
/// Interlineado del texto de las notas.
pub(crate) const STUDY_LINE_H: f32 = 26.0;
/// Alto de la cabecera del panel (título + botones).
pub(crate) const STUDY_HEADER_H: f32 = 56.0;
/// Margen interior del panel.
pub(crate) const STUDY_PAD: f32 = 20.0;

const MARGIN: f32 = 16.0;
const BTN_H: f32 = 40.0;
const GAP: f32 = 8.0;
const ROW_H: f32 = 100.0;
const ROW_GAP: f32 = 8.0;
const ROW_BTN_W: f32 = 104.0;
const EMPTY_H: f32 = 72.0;
const HINT_H: f32 = 40.0;

/// Caracteres que caben en `w` px con el cuerpo de las notas (estimación
/// de ancho medio de glifo, la misma del panel de IA).
fn chars_for(w: f32) -> usize {
    ((w / (STUDY_TEXT_SIZE * 0.52)).floor() as usize).max(8)
}

/// Layout del panel (fuente única de dibujo e hit-test).
pub(crate) fn study_layout(win_w: f32, win_h: f32, input: &StudyInput) -> StudyLayout {
    let panel_w = (win_w - 2.0 * MARGIN).clamp(200.0, 1080.0);
    let x = (win_w - panel_w) / 2.0;
    match input.view {
        StudyView::Notebook => notebook_layout(win_h, x, panel_w, input),
        StudyView::NoteEditor | StudyView::KeyEditor => editor_layout(win_h, x, panel_w, input),
    }
}

fn notebook_layout(win_h: f32, x: f32, panel_w: f32, input: &StudyInput) -> StudyLayout {
    let n = input.notes.len();
    let max_body = (win_h * 0.78 - STUDY_HEADER_H - 2.0 * STUDY_PAD).max(ROW_H);
    let visible = (((max_body + ROW_GAP) / (ROW_H + ROW_GAP)).floor() as usize).max(1);
    let max_scroll = n.saturating_sub(visible);
    let scroll = input.scroll.min(max_scroll);
    let shown = n.saturating_sub(scroll).min(visible);
    let body_h = if shown == 0 {
        EMPTY_H
    } else {
        shown as f32 * (ROW_H + ROW_GAP) - ROW_GAP
    };
    let panel_h = STUDY_HEADER_H + body_h + 2.0 * STUDY_PAD;
    let y = ((win_h - panel_h) / 2.0).max(8.0);
    let right = x + panel_w;

    // Cabecera, alineada a la derecha: [+ Nota][▲][▼][×].
    let mut buttons: Vec<(StudyHit, ButtonRect)> = Vec::with_capacity(4);
    let bt = y + (STUDY_HEADER_H - BTN_H) / 2.0;
    let bb = bt + BTN_H;
    let mut bx = right - STUDY_PAD - BTN_H;
    buttons.push((StudyHit::Close, (bx, bt, bx + BTN_H, bb)));
    if max_scroll > 0 {
        bx -= BTN_H + GAP;
        buttons.push((StudyHit::ScrollDown, (bx, bt, bx + BTN_H, bb)));
        bx -= BTN_H + GAP;
        buttons.push((StudyHit::ScrollUp, (bx, bt, bx + BTN_H, bb)));
    }
    let new_w = 132.0;
    bx -= new_w + GAP;
    buttons.push((StudyHit::NewNote, (bx, bt, bx + new_w, bb)));

    let body_top = y + STUDY_HEADER_H + STUDY_PAD;
    let rows = input
        .notes
        .iter()
        .skip(scroll)
        .take(shown)
        .enumerate()
        .map(|(i, note)| {
            let top = body_top + i as f32 * (ROW_H + ROW_GAP);
            let rect = (x + STUDY_PAD, top, right - STUDY_PAD, top + ROW_H);
            let bt = top + (ROW_H - BTN_H) / 2.0;
            let del_r = rect.2 - 12.0;
            let delete = (del_r - ROW_BTN_W, bt, del_r, bt + BTN_H);
            let edit = (delete.0 - GAP - ROW_BTN_W, bt, delete.0 - GAP, bt + BTN_H);
            let text_w = edit.0 - 16.0 - (rect.0 + 16.0);
            NoteRowLayout {
                id: note.id,
                rect,
                edit,
                delete,
                meta: note_meta(note),
                lines: preview_lines(&note.text, chars_for(text_w)),
                confirm_delete: input.confirm_delete == Some(note.id),
            }
        })
        .collect();

    StudyLayout {
        rect: (x, y, right, y + panel_h),
        buttons,
        rows,
        field: None,
        scroll,
        max_scroll,
        field_chars: 0,
    }
}

fn editor_layout(win_h: f32, x: f32, panel_w: f32, input: &StudyInput) -> StudyLayout {
    // Arriba de la pantalla: el teclado del sistema ocupa la mitad inferior.
    let y = (win_h * 0.05).clamp(24.0, 96.0);
    let right = x + panel_w;
    let field_top = y + STUDY_HEADER_H + STUDY_PAD;
    let field_h = FIELD_LINES as f32 * STUDY_LINE_H + 24.0;
    let field = (
        x + STUDY_PAD,
        field_top,
        right - STUDY_PAD,
        field_top + field_h,
    );
    let bt = field.3 + HINT_H;
    let bb = bt + BTN_H + 4.0;
    let mut buttons: Vec<(StudyHit, ButtonRect)> = Vec::with_capacity(3);
    let w = 148.0;
    let save_l = right - STUDY_PAD - w;
    buttons.push((StudyHit::Save, (save_l, bt, save_l + w, bb)));
    let cancel_l = save_l - GAP - w;
    buttons.push((StudyHit::Cancel, (cancel_l, bt, cancel_l + w, bb)));
    if input.view == StudyView::KeyEditor && input.has_key {
        let l = x + STUDY_PAD;
        buttons.push((StudyHit::ClearKey, (l, bt, l + 176.0, bb)));
    }
    StudyLayout {
        rect: (x, y, right, bb + STUDY_PAD),
        buttons,
        rows: Vec::new(),
        field: Some(field),
        scroll: 0,
        max_scroll: 0,
        field_chars: chars_for(field.2 - field.0 - 24.0),
    }
}

fn contains((l, t, r, b): ButtonRect, x: f32, y: f32) -> bool {
    x >= l && x < r && y >= t && y < b
}

/// Hit-test de un tap: None si cae FUERA del panel.
pub(crate) fn study_hit(layout: &StudyLayout, x: f32, y: f32) -> Option<StudyHit> {
    if !contains(layout.rect, x, y) {
        return None;
    }
    if let Some((hit, _)) = layout.buttons.iter().find(|(_, r)| contains(*r, x, y)) {
        return Some(*hit);
    }
    for row in &layout.rows {
        if contains(row.edit, x, y) {
            return Some(StudyHit::EditNote(row.id));
        }
        if contains(row.delete, x, y) {
            return Some(StudyHit::DeleteNote(row.id));
        }
        if contains(row.rect, x, y) {
            return Some(StudyHit::OpenNote(row.id));
        }
    }
    if layout.field.is_some_and(|f| contains(f, x, y)) {
        return Some(StudyHit::Field);
    }
    Some(StudyHit::Inside)
}

/// Envoltorio de texto plano por palabras a `max_chars` columnas (respeta
/// los saltos de línea; parte palabras más largas que la línea).
pub(crate) fn wrap_plain(text: &str, max_chars: usize) -> Vec<String> {
    let max = max_chars.max(1);
    let mut out = Vec::new();
    if text.is_empty() {
        return out;
    }
    for para in text.split('\n') {
        let mut line = String::new();
        let mut len = 0usize;
        for word in para.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            // Palabra más larga que la línea: trocearla.
            while word.len() > max {
                if len > 0 {
                    out.push(std::mem::take(&mut line));
                    len = 0;
                }
                let rest = word.split_off(max);
                out.push(word.into_iter().collect());
                word = rest;
            }
            if word.is_empty() {
                continue;
            }
            let need = if len == 0 {
                word.len()
            } else {
                len + 1 + word.len()
            };
            if need > max {
                out.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.extend(word.iter());
            len += word.len();
        }
        out.push(line);
    }
    out
}

/// Vista previa de una nota: `ROW_PREVIEW_LINES` líneas, con "…" si sigue.
fn preview_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = wrap_plain(text, max_chars);
    if lines.len() > ROW_PREVIEW_LINES {
        lines.truncate(ROW_PREVIEW_LINES);
        if let Some(last) = lines.last_mut() {
            let keep: String = last.chars().take(max_chars.saturating_sub(1)).collect();
            *last = format!("{}…", keep.trim_end());
        }
    }
    lines
}

/// Metadatos de la fila: página (1-based) y fecha de creación.
pub(crate) fn note_meta(note: &Note) -> String {
    let date = format_date(note.created_unix);
    let mut meta = match note.page {
        Some(p) => format!("Pág. {} · {date}", p + 1),
        None => format!("Nota general · {date}"),
    };
    if note.edited_unix != note.created_unix {
        meta.push_str(" · editada");
    }
    meta
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
