// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Panel de Estudio del visor (PR-02): estado y controlador PURO
//! (`StudyState::on_hit` devuelve efectos; sin `AndroidApp` ni JNI, así se
//! testea en host) + el pegamento con `Reader` (abrir/cerrar, teclado,
//! guardado del cuaderno y de la clave).

use super::{Reader, UiMode};
use crate::android_activity::AndroidApp;
use crate::draw::{StudyHit, StudyView, study_hit, study_layout_for};
use crate::gemini_key::GeminiKey;
use crate::notebook::Notebook;
use log::{error, info};
use pdf_core::Document as _;

/// Contexto del documento abierto que necesita el controlador.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StudyCtx {
    pub(crate) now: i64,
    /// Página actual (0-based): se adjunta a las notas nuevas.
    pub(crate) page: u32,
    pub(crate) page_count: u32,
}

/// Efecto que el `Reader` debe aplicar tras un tap del panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StudyEffect {
    /// Cerrar el panel.
    Close,
    /// Saltar a la página (0-based).
    Jump(u32),
    /// Abrir el teclado con este texto inicial.
    OpenKeyboard(String),
    HideKeyboard,
    /// Persistir el cuaderno del documento.
    SaveNotebook,
    SaveKey(GeminiKey),
    ClearKey,
}

/// Estado del panel de Estudio (Some en `Reader::study` mientras está abierto).
#[derive(Clone, Debug)]
pub(crate) struct StudyState {
    pub(crate) view: StudyView,
    pub(crate) notebook: Notebook,
    pub(crate) scroll: usize,
    /// Texto del editor (lo que llega del IME).
    pub(crate) draft: String,
    /// Nota en edición (None = nota nueva).
    pub(crate) editing: Option<u64>,
    /// Página que se adjunta a la nota nueva.
    pub(crate) draft_page: Option<u32>,
    pub(crate) confirm_delete: Option<u64>,
    /// Error de validación visible en el editor.
    pub(crate) error: Option<&'static str>,
    /// El editor de la clave se abrió directamente desde el sheet: guardar
    /// o cancelar cierran el panel.
    pub(crate) key_only: bool,
}

/// Mensaje del editor de notas cuando se intenta guardar sin texto.
pub(crate) const EMPTY_NOTE_MSG: &str = "Escribe algo antes de guardar.";

impl StudyState {
    /// Panel abierto en la lista de notas del documento.
    pub(crate) fn notebook(notebook: Notebook) -> Self {
        Self {
            view: StudyView::Notebook,
            notebook,
            scroll: 0,
            draft: String::new(),
            editing: None,
            draft_page: None,
            confirm_delete: None,
            error: None,
            key_only: false,
        }
    }

    /// Panel abierto directamente en el editor de la clave (borrador vacío:
    /// la clave guardada nunca se precarga en el campo de texto).
    pub(crate) fn key_editor() -> Self {
        Self {
            view: StudyView::KeyEditor,
            key_only: true,
            ..Self::notebook(Notebook::default())
        }
    }

    /// ¿Está abierto un editor (el teclado escribe en `draft`)?
    pub(crate) fn is_editing(&self) -> bool {
        matches!(self.view, StudyView::NoteEditor | StudyView::KeyEditor)
    }

    /// Nuevo texto del IME. true si cambió (hay que repintar).
    pub(crate) fn set_draft(&mut self, text: &str) -> bool {
        if self.draft == text {
            return false;
        }
        self.draft = text.to_string();
        self.error = None;
        true
    }

    /// Texto que muestra el campo: la clave siempre enmascarada.
    pub(crate) fn field_text(&self) -> String {
        match self.view {
            StudyView::KeyEditor if !self.draft.is_empty() => {
                let n = self.draft.chars().filter(|c| !c.is_whitespace()).count();
                format!("{} ({n} caracteres)", crate::gemini_key::mask(&self.draft))
            }
            _ => self.draft.clone(),
        }
    }

    /// Tap FUERA del panel: la lista se cierra; un editor no descarta lo
    /// escrito (hay que pulsar Cancelar).
    pub(crate) fn on_outside(&mut self) -> Vec<StudyEffect> {
        if self.is_editing() {
            Vec::new()
        } else {
            vec![StudyEffect::Close]
        }
    }

    /// Abre el editor de notas con `text` (nota nueva si `editing` es None).
    fn open_note_editor(
        &mut self,
        editing: Option<u64>,
        page: Option<u32>,
        text: String,
    ) -> Vec<StudyEffect> {
        self.view = StudyView::NoteEditor;
        self.editing = editing;
        self.draft_page = page;
        self.draft = text.clone();
        self.error = None;
        vec![StudyEffect::OpenKeyboard(text)]
    }

    /// Vuelve a la lista de notas (o cierra si el panel era solo la clave).
    fn leave_editor(&mut self) -> Vec<StudyEffect> {
        self.draft.clear();
        self.editing = None;
        self.draft_page = None;
        self.error = None;
        if self.key_only {
            vec![StudyEffect::HideKeyboard, StudyEffect::Close]
        } else {
            self.view = StudyView::Notebook;
            vec![StudyEffect::HideKeyboard]
        }
    }

    /// Tap sobre un control del panel.
    pub(crate) fn on_hit(&mut self, hit: StudyHit, ctx: StudyCtx) -> Vec<StudyEffect> {
        // Cualquier control que no sea el mismo Borrar desarma la confirmación.
        let armed = self.confirm_delete.take();
        match (self.view, hit) {
            (StudyView::Notebook, StudyHit::Close) => vec![StudyEffect::Close],
            (StudyView::Notebook, StudyHit::NewNote) => {
                self.open_note_editor(None, Some(ctx.page), String::new())
            }
            (StudyView::Notebook, StudyHit::ScrollUp) => {
                self.scroll = self.scroll.saturating_sub(1);
                Vec::new()
            }
            (StudyView::Notebook, StudyHit::ScrollDown) => {
                let last = self.notebook.notes.len().saturating_sub(1);
                self.scroll = (self.scroll + 1).min(last);
                Vec::new()
            }
            (StudyView::Notebook, StudyHit::OpenNote(id)) => {
                let Some(note) = self.notebook.get(id) else {
                    return Vec::new();
                };
                match crate::notebook::jump_target(note, ctx.page_count) {
                    Some(p) => vec![StudyEffect::Close, StudyEffect::Jump(p)],
                    None => {
                        let (page, text) = (note.page, note.text.clone());
                        self.open_note_editor(Some(id), page, text)
                    }
                }
            }
            (StudyView::Notebook, StudyHit::EditNote(id)) => {
                let Some(note) = self.notebook.get(id) else {
                    return Vec::new();
                };
                let (page, text) = (note.page, note.text.clone());
                self.open_note_editor(Some(id), page, text)
            }
            (StudyView::Notebook, StudyHit::DeleteNote(id)) => {
                if armed == Some(id) {
                    if self.notebook.delete(id) {
                        let last = self.notebook.notes.len().saturating_sub(1);
                        self.scroll = self.scroll.min(last);
                        return vec![StudyEffect::SaveNotebook];
                    }
                    Vec::new()
                } else {
                    self.confirm_delete = Some(id);
                    Vec::new()
                }
            }
            (StudyView::NoteEditor, StudyHit::Save) => {
                let saved = match self.editing {
                    Some(id) => self.notebook.edit(id, &self.draft, ctx.now),
                    None => self
                        .notebook
                        .add(&self.draft, self.draft_page, ctx.now)
                        .is_some(),
                };
                if !saved {
                    self.error = Some(EMPTY_NOTE_MSG);
                    return Vec::new();
                }
                if self.editing.is_none() {
                    self.scroll = 0; // la nota nueva es la primera de la lista
                }
                let mut fx = self.leave_editor();
                fx.push(StudyEffect::SaveNotebook);
                fx
            }
            (StudyView::KeyEditor, StudyHit::Save) => match GeminiKey::parse(&self.draft) {
                Ok(key) => {
                    let mut fx = self.leave_editor();
                    fx.insert(1, StudyEffect::SaveKey(key));
                    fx
                }
                Err(e) => {
                    self.error = Some(e.message());
                    Vec::new()
                }
            },
            (StudyView::KeyEditor, StudyHit::ClearKey) => {
                let mut fx = self.leave_editor();
                fx.insert(1, StudyEffect::ClearKey);
                fx
            }
            (StudyView::NoteEditor | StudyView::KeyEditor, StudyHit::Cancel) => self.leave_editor(),
            (StudyView::NoteEditor | StudyView::KeyEditor, StudyHit::Field) => {
                vec![StudyEffect::OpenKeyboard(self.draft.clone())]
            }
            _ => Vec::new(),
        }
    }
}

/// Destino del texto que teclea el IME.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImeTarget {
    LibrarySearch,
    DiscoverSearch,
    StudyDraft,
    /// Nadie escucha (visor sin editor abierto): ocultar el teclado.
    None,
}

/// Decide a quién va el texto del IME según el modo y el panel de Estudio.
pub(crate) fn ime_target(mode: UiMode, study_editing: bool) -> ImeTarget {
    match mode {
        UiMode::Viewer if study_editing => ImeTarget::StudyDraft,
        UiMode::Viewer => ImeTarget::None,
        UiMode::Discover => ImeTarget::DiscoverSearch,
        UiMode::Library | UiMode::Picker => ImeTarget::LibrarySearch,
    }
}

// ---------------------------------------------------------------------
// Pegamento con `Reader` (no testeable en host: teclado y disco reales)
// ---------------------------------------------------------------------
impl Reader {
    /// Abre el cuaderno del documento actual (sheet → Estudio → Cuaderno).
    pub(crate) fn open_notebook(&mut self) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let nb = crate::notebook::load_notebook_for(self.internal_dir.as_deref(), &path);
        info!("notebook: {} notes", nb.notes.len());
        self.hide_sheet();
        self.study = Some(StudyState::notebook(nb));
        self.refresh_study();
    }

    /// Abre el editor de la clave de Gemini con el teclado (sheet → Estudio).
    pub(crate) fn open_key_editor(&mut self, app: &AndroidApp) {
        self.hide_sheet();
        self.study = Some(StudyState::key_editor());
        crate::jni::ime_attach(app, "");
        self.ime_active = true;
        self.refresh_study();
    }

    /// Cierra el panel de Estudio (el teclado lo oculta `poll_ime_query`
    /// al no tener destino, o `lib_close_ime` al salir del visor).
    pub(crate) fn close_study(&mut self) {
        let had = self.study.is_some();
        self.study = None;
        self.study_bitmap = None;
        if had {
            self.redraw();
        }
    }

    /// Contenido del panel cambiado: re-render en el próximo `blit`.
    fn refresh_study(&mut self) {
        self.study_bitmap = None;
        self.redraw();
    }

    /// Texto nuevo del IME para el editor abierto.
    pub(crate) fn study_set_draft(&mut self, text: &str) {
        if self.study.as_mut().is_some_and(|s| s.set_draft(text)) {
            self.refresh_study();
        }
    }

    /// Tap con el panel de Estudio abierto (misma geometría que el dibujo).
    pub(crate) fn study_tap(&mut self, app: &AndroidApp, x: f32, y: f32) {
        let Some(layout) = study_layout_for(self) else {
            return;
        };
        let ctx = StudyCtx {
            now: crate::persist::unix_now(),
            page: self.page,
            page_count: self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0),
        };
        let Some(st) = self.study.as_mut() else {
            return;
        };
        // El scroll efectivo es el del layout (recortado a las filas que caben).
        st.scroll = layout.scroll;
        let effects = match study_hit(&layout, x, y) {
            Some(hit) => st.on_hit(hit, ctx),
            None => st.on_outside(),
        };
        for fx in effects {
            self.apply_study_effect(app, fx);
        }
        if let Some(st) = self.study.as_mut() {
            st.scroll = st.scroll.min(layout.max_scroll);
        }
        if self.study.is_some() {
            self.refresh_study();
        }
    }

    fn apply_study_effect(&mut self, app: &AndroidApp, fx: StudyEffect) {
        match fx {
            StudyEffect::Close => self.close_study(),
            StudyEffect::Jump(p) => {
                let delta = p as i32 - self.page as i32;
                self.jump_page(delta);
            }
            StudyEffect::OpenKeyboard(text) => {
                crate::jni::ime_attach(app, &text);
                self.ime_active = true;
            }
            StudyEffect::HideKeyboard => {
                if self.ime_active {
                    crate::jni::ime_set_text(app, "");
                    crate::jni::ime_hide(app);
                    self.ime_active = false;
                }
            }
            StudyEffect::SaveNotebook => {
                let (Some(path), Some(st)) = (self.doc_path.as_deref(), self.study.as_ref()) else {
                    return;
                };
                if let Err(e) = crate::notebook::save_notebook_for(
                    self.internal_dir.as_deref(),
                    path,
                    &st.notebook,
                ) {
                    error!("save notebook: {e}");
                    self.show_toast("No se pudo guardar el cuaderno");
                }
            }
            StudyEffect::SaveKey(key) => {
                // Nunca se loguea la clave: solo el resultado.
                match crate::gemini_key::save_key(self.internal_dir.as_deref(), &key) {
                    Ok(()) => {
                        info!("gemini key saved");
                        self.gemini_key = Some(key);
                        self.show_toast("Clave de Gemini guardada");
                    }
                    Err(e) => {
                        error!("save gemini key: {e}");
                        self.show_toast("No se pudo guardar la clave");
                    }
                }
                self.sheet_bitmap = None; // la etiqueta del sheet cambia
            }
            StudyEffect::ClearKey => {
                if let Err(e) = crate::gemini_key::clear_key(self.internal_dir.as_deref()) {
                    error!("clear gemini key: {e}");
                }
                self.gemini_key = None;
                self.sheet_bitmap = None;
                self.show_toast("Clave de Gemini borrada");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const CTX: StudyCtx = StudyCtx {
        now: 1000,
        page: 6,
        page_count: 20,
    };

    fn with_notes() -> (StudyState, u64, u64) {
        let mut nb = Notebook::default();
        let a = nb.add("con página", Some(4), 10).unwrap();
        let b = nb.add("general", None, 20).unwrap();
        (StudyState::notebook(nb), a, b)
    }

    #[test]
    fn new_note_attaches_current_page_and_saves() {
        let mut s = StudyState::notebook(Notebook::default());
        assert!(!s.is_editing());
        assert_eq!(
            s.on_hit(StudyHit::NewNote, CTX),
            vec![StudyEffect::OpenKeyboard(String::new())]
        );
        assert_eq!(s.view, StudyView::NoteEditor);
        assert!(s.is_editing());
        // Guardar vacío: error visible, sigue en el editor.
        assert!(s.on_hit(StudyHit::Save, CTX).is_empty());
        assert_eq!(s.error, Some(EMPTY_NOTE_MSG));
        assert_eq!(s.view, StudyView::NoteEditor);
        assert!(s.set_draft("Repasar la demostración"));
        assert!(!s.set_draft("Repasar la demostración"), "sin cambio");
        assert_eq!(s.error, None, "teclear limpia el error");
        assert_eq!(
            s.on_hit(StudyHit::Save, CTX),
            vec![StudyEffect::HideKeyboard, StudyEffect::SaveNotebook]
        );
        assert_eq!(s.view, StudyView::Notebook);
        let n = &s.notebook.notes[0];
        assert_eq!(n.text, "Repasar la demostración");
        assert_eq!(n.page, Some(6));
        assert_eq!(n.created_unix, 1000);
    }

    #[test]
    fn edit_prefills_text_and_keeps_page() {
        let (mut s, a, _) = with_notes();
        assert_eq!(
            s.on_hit(StudyHit::EditNote(a), CTX),
            vec![StudyEffect::OpenKeyboard("con página".to_string())]
        );
        assert_eq!(s.draft, "con página");
        s.set_draft("con página, revisada");
        let fx = s.on_hit(StudyHit::Save, CTX);
        assert!(fx.contains(&StudyEffect::SaveNotebook), "{fx:?}");
        let n = s.notebook.get(a).unwrap();
        assert_eq!(n.text, "con página, revisada");
        assert_eq!(n.page, Some(4));
        assert_eq!(n.edited_unix, 1000);
        // Cancelar no guarda.
        s.on_hit(StudyHit::EditNote(a), CTX);
        s.set_draft("descartado");
        assert_eq!(
            s.on_hit(StudyHit::Cancel, CTX),
            vec![StudyEffect::HideKeyboard]
        );
        assert_eq!(s.notebook.get(a).unwrap().text, "con página, revisada");
        assert_eq!(s.view, StudyView::Notebook);
    }

    #[test]
    fn delete_needs_a_second_tap_on_the_same_note() {
        let (mut s, a, b) = with_notes();
        assert!(s.on_hit(StudyHit::DeleteNote(a), CTX).is_empty());
        assert_eq!(s.confirm_delete, Some(a));
        // Otro control desarma la confirmación.
        s.on_hit(StudyHit::Inside, CTX);
        assert_eq!(s.confirm_delete, None);
        s.on_hit(StudyHit::DeleteNote(a), CTX);
        assert!(s.on_hit(StudyHit::DeleteNote(b), CTX).is_empty());
        assert_eq!(s.confirm_delete, Some(b), "la confirmación es por nota");
        assert_eq!(
            s.on_hit(StudyHit::DeleteNote(b), CTX),
            vec![StudyEffect::SaveNotebook]
        );
        assert!(s.notebook.get(b).is_none());
        assert!(s.notebook.get(a).is_some());
        assert_eq!(s.confirm_delete, None);
    }

    #[test]
    fn tapping_a_note_with_a_page_jumps_there_and_closes() {
        let (mut s, a, b) = with_notes();
        assert_eq!(
            s.on_hit(StudyHit::OpenNote(a), CTX),
            vec![StudyEffect::Close, StudyEffect::Jump(4)]
        );
        // Página fuera del documento: se recorta a la última.
        let short = StudyCtx {
            page_count: 3,
            ..CTX
        };
        assert_eq!(
            s.on_hit(StudyHit::OpenNote(a), short),
            vec![StudyEffect::Close, StudyEffect::Jump(2)]
        );
        // Nota sin página: se abre para editar.
        assert_eq!(
            s.on_hit(StudyHit::OpenNote(b), CTX),
            vec![StudyEffect::OpenKeyboard("general".to_string())]
        );
        assert_eq!(s.editing, Some(b));
    }

    #[test]
    fn close_scroll_and_outside_in_the_list() {
        let (mut s, _, _) = with_notes();
        s.on_hit(StudyHit::ScrollUp, CTX);
        assert_eq!(s.scroll, 0);
        s.on_hit(StudyHit::ScrollDown, CTX);
        assert_eq!(s.scroll, 1);
        s.on_hit(StudyHit::ScrollDown, CTX);
        assert_eq!(s.scroll, 1, "no más allá de la última nota");
        assert_eq!(s.on_hit(StudyHit::Close, CTX), vec![StudyEffect::Close]);
        assert_eq!(s.on_outside(), vec![StudyEffect::Close]);
        // En un editor el tap fuera NO descarta lo escrito.
        s.on_hit(StudyHit::NewNote, CTX);
        s.set_draft("a medias");
        assert!(s.on_outside().is_empty());
        assert_eq!(
            s.on_hit(StudyHit::Field, CTX),
            vec![StudyEffect::OpenKeyboard("a medias".to_string())]
        );
    }

    #[test]
    fn key_editor_validates_saves_clears_and_masks() {
        let mut s = StudyState::key_editor();
        assert!(s.is_editing());
        assert_eq!(s.draft, "");
        s.set_draft("corta");
        assert!(s.on_hit(StudyHit::Save, CTX).is_empty());
        assert_eq!(
            s.error,
            Some(crate::gemini_key::KeyError::TooShort.message())
        );
        let pasted = "AIzaTESTONLY_not-a-real-key-0123456789";
        s.set_draft(pasted);
        let shown = s.field_text();
        assert!(shown.contains("••••6789"), "{shown}");
        assert!(!shown.contains("AIza"), "{shown}");
        let key = GeminiKey::parse(pasted).unwrap();
        assert_eq!(
            s.on_hit(StudyHit::Save, CTX),
            vec![
                StudyEffect::HideKeyboard,
                StudyEffect::SaveKey(key),
                StudyEffect::Close
            ]
        );
        let mut s = StudyState::key_editor();
        assert_eq!(
            s.on_hit(StudyHit::ClearKey, CTX),
            vec![
                StudyEffect::HideKeyboard,
                StudyEffect::ClearKey,
                StudyEffect::Close
            ]
        );
        let mut s = StudyState::key_editor();
        assert_eq!(
            s.on_hit(StudyHit::Cancel, CTX),
            vec![StudyEffect::HideKeyboard, StudyEffect::Close]
        );
    }

    #[test]
    fn note_field_shows_the_draft_verbatim() {
        let mut s = StudyState::notebook(Notebook::default());
        s.on_hit(StudyHit::NewNote, CTX);
        s.set_draft("x^2 + 1");
        assert_eq!(s.field_text(), "x^2 + 1");
    }

    #[test]
    fn ime_text_goes_to_the_study_editor_only_in_the_viewer() {
        assert_eq!(ime_target(UiMode::Viewer, true), ImeTarget::StudyDraft);
        assert_eq!(ime_target(UiMode::Viewer, false), ImeTarget::None);
        assert_eq!(ime_target(UiMode::Library, false), ImeTarget::LibrarySearch);
        assert_eq!(
            ime_target(UiMode::Discover, false),
            ImeTarget::DiscoverSearch
        );
        assert_eq!(ime_target(UiMode::Library, true), ImeTarget::LibrarySearch);
    }

    #[test]
    fn deactivate_ime_clears_buffer_before_hiding_keyboard() {
        let mut active = true;
        let mut events = Vec::new();

        deactivate_ime(
            &mut active,
            |text| events.push(format!("set_text:{text:?}")),
            || events.push("hide".to_string()),
        );

        assert!(!active);
        assert_eq!(events, vec!["set_text:\"\"", "hide"]);

        // Idempotent when already inactive.
        events.clear();
        deactivate_ime(
            &mut active,
            |text| events.push(format!("set_text:{text:?}")),
            || events.push("hide".to_string()),
        );
        assert!(events.is_empty());
    }
}
