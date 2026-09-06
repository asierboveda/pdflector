// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Navegación y apertura de documentos (extraído de `reader.rs`, 2026-09-06): cambio de página (`goto_page`, `next_page`, `prev_page`, `jump_page`), persistencia de posición (`save_state`) y apertura de PDFs (`open_pdf`, `open_pdf_at`).

use super::Reader;
use super::UiMode;
use crate::annotations::ToolKind;
use crate::draw::compose_library_snapshot;
use log::error;
use log::info;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};
use std::path::Path;
use std::time::Instant;

impl Reader {
    /// Cambia a la página `page` (0-based) — modo UNA HOJA: `page` se fija
    /// directamente (no hay scroll que alinear: la columna de páginas se
    /// eliminó). Base compartida de `next_page`/`prev_page`/`jump_page` y del
    /// tap derecho/izquierdo. No hay salto con re-render: las páginas vecinas
    /// salen de la caché (paso instantáneo). Invalida los overlays cacheados
    /// (indicador, sheet, frame de la animación).
    fn goto_page(&mut self, page: u32) {
        let prev = self.page;
        if prev == page {
            return;
        }
        self.page = page;
        self.page_badge = None; // el indicador "N / total" cambia
        self.sheet_bitmap = None; // el indicador del sheet cambia
        info!("page {}", self.page + 1);
        // Cambio de página SIN congelar: si la nueva está en caché (prefetch
        // previo), el blit es inmediato; si no, se muestra la página ANTERIOR
        // (fallback) mientras el worker renderiza la nueva asíncronamente.
        if self.cache.peek(page).is_none() {
            self.fallback_page = Some(prev);
            let pages = {
                let n = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                let lo = page.saturating_sub(1);
                let hi = (page + 1).min(n.saturating_sub(1));
                (lo..=hi)
                    .filter(|&p| self.cache.peek(p).is_none())
                    .collect()
            };
            self.launch_render(pages, self.rendered_zoom, false);
        }
        self.save_state();
        if self.window.is_some() {
            self.blit();
        }
    }

    pub(crate) fn next_page(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1);
        if self.page < last {
            self.goto_page(self.page + 1);
        }
    }

    pub(crate) fn prev_page(&mut self) {
        if self.page > 0 {
            self.goto_page(self.page - 1);
        }
    }

    /// Salto rápido de ±N páginas (botones −10/+10 del sheet de ajustes).
    pub(crate) fn jump_page(&mut self, delta: i32) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1) as i32;
        let target = (self.page as i32 + delta).clamp(0, last) as u32;
        if target != self.page {
            self.goto_page(target);
        }
    }

    /// Persiste la posición actual (ruta, página, zoom) + modo oscuro en
    /// `internal/state.json` (ver `persist`). Escritura *eager*: se llama en
    /// cada cambio de página, al soltar el pinch, al abrir un documento y al
    /// alternar el modo oscuro — un cierre inesperado no pierde la posición.
    ///
    /// Además actualiza el REGISTRO DE PROGRESO por libro
    /// (`internal/library.json`, ver `persist::BookProgress`): página actual,
    /// total de páginas y sello de última lectura. El registro se CREA la
    /// primera vez (added_unix) y se actualiza en cada apertura o cambio de
    /// página — de ahí se derivan "Page X of Y", la barra de progreso, el
    /// estado Reading/Finished y los sorts de "My Library" sin abrir el PDF.
    pub(crate) fn save_state(&mut self) {
        let path = self.doc_path.clone().unwrap_or_default();
        let state = crate::persist::ViewerState {
            path: path.clone(),
            page: self.page,
            zoom: self.zoom,
            dark: self.dark,
            theme: Some(self.theme),
            view_mode: self.view_mode,
            cover_fit: self.cover_fit,
            columns: self.columns,
            hide_covers: self.hide_covers,
            recent_shelf_enabled: self.recent_shelf_enabled,
            cover_size: self.cover_size,
            cover_progress: self.cover_progress,
        };
        crate::persist::save_state(self.internal_dir.as_deref(), &state);
        if self.mode == UiMode::Viewer && !path.is_empty() {
            let pages = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
            let now = crate::persist::unix_now();
            self.library.lib_books = crate::persist::touch_progress(
                &self.library.lib_books,
                &path,
                self.page,
                pages,
                now,
            );
            crate::persist::save_progress(self.internal_dir.as_deref(), &self.library.lib_books);
        }
    }

    /// Abre un PDF por ruta (picker) y pasa al visor con la página 1.
    /// Devuelve false (y deja el estado intacto) si no se pudo abrir.
    pub(crate) fn open_pdf(&mut self, path: &str) -> bool {
        self.open_pdf_at(path, None)
    }

    /// Abre un PDF por ruta y pasa al visor; si `start_page` es Some, salta
    /// a esa página (la posición guardada de "Continue Reading"/la rejilla),
    /// si no a la página 1. Devuelve false (y deja el estado intacto) si no
    /// se pudo abrir.
    pub(crate) fn open_pdf_at(&mut self, path: &str, start_page: Option<u32>) -> bool {
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                return false;
            }
        };
        match engine.open(Path::new(path)) {
            Ok(doc) => {
                let pages = doc.page_count();
                info!("opened: {pages} pages");
                // Página de apertura: la guardada (reanudar lectura) o la 1.
                let page = match start_page {
                    Some(p) => p.min(pages.saturating_sub(1)),
                    None => 0,
                };
                self.doc = Some(doc);
                self.page = page;
                self.zoom = 1.0;
                self.rendered_zoom = 1.0;
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                self.pinch = None;
                self.bitmap = None;
                // Transición al abrir: snapshot de la pantalla de lista
                // (biblioteca: cabecera+banda; picker: bitmap) que el visor
                // funde sobre la página los primeros `LIB_FADE_MS`.
                let snapshot = match self.mode {
                    UiMode::Library => compose_library_snapshot(self),
                    UiMode::Picker => self.bitmap.clone(),
                    UiMode::Viewer => None,
                };
                if let Some(s) = snapshot {
                    self.library.lib_fade = Some((Instant::now(), s));
                    self.library.lib_fade_id = self.next_ovl_id(); // snapshot nuevo
                }
                self.library.lib_header = None; // biblioteca fuera: liberar planos
                self.library.lib_band = None;
                self.library.lib_row_dirty = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.mode = UiMode::Viewer;
                // EGL: venimos de Library/Picker sin surface (ver
                // `enter_library`); recrearla ya para el primer present.
                if let (Some(g), Some(win)) = (self.gpu.as_mut(), self.window.as_ref())
                    && !g.has_surface()
                {
                    g.recreate_surface(win);
                }
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.start_render_worker(path);
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera (libera también el frame)
                self.clear_selection(); // selección del documento anterior: fuera
                self.close_ai_panel(); // panel de IA del documento anterior: fuera
                self.thumbs.clear(); // portadas de otra biblioteca: no sirven
                self.thumb_failed.clear();
                self.list_dirty = true;
                self.list_drag = None;
                // Herramientas de anotación: reseteo a la navegación limpia
                // (sin herramienta activa, sin gesto en curso
                // y SIN histórico de sesión del documento anterior — el undo
                // es por sesión, decisión documentada en `session_ids`).
                self.tool = ToolKind::Navigate;
                self.tool_gesture = None;
                self.session_ids.clear();
                // Fase B1: texto del documento nuevo (el del anterior no
                // sirve). Prefetch de la página visible +-2: el primer
                // resaltado de esas páginas será un HIT (sin stext en el
                // hilo UI). El resto se extrae perezoso con `get_or_extract`
                // (1-2 ms) y queda cacheado para repeticiones y para la IA
                // (Fase D).
                self.text_cache.clear();
                if let Some(doc) = self.doc.as_ref() {
                    let base = page.saturating_sub(2);
                    let pages: Vec<u32> = (base..(page + 3).min(pages)).collect();
                    let _n = self.text_cache.prefetch(doc, &pages);
                }
                // Anotaciones del documento (sidecar; set vacío si no existe
                // o está corrupto — nunca impide abrir el PDF).
                self.load_annotations(path);
                self.redraw();
                // Nuevo documento: actualizar la posición persistida (el
                // modo oscuro es una preferencia global y se conserva).
                self.save_state();
                // Y la lista de RECIENTES de la biblioteca (dedup por ruta,
                // más reciente primero, máx. 10 — persist::push_recent).
                self.touch_recent(path);
                true
            }
            Err(e) => {
                error!("cannot open {path}: {e}");
                false
            }
        }
    }
}
