// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Métodos de `Reader` para la pestaña y el ciclo de vida de Discover (arXiv).
//!
//! Integra el worker de fondo (`DiscoverWorker`), el procesamiento no bloqueante
//! en `tick` (`pump_discover`), y la incorporación de papers descargados a la biblioteca curada.

use std::path::Path;

use super::discover_state::{DiscoverPhase, DiscoverScreen, DiscoverTab};
use super::discover_worker::{DiscoverCmd, DiscoverMsg, DiscoverWorker};
use super::{LibraryEntry, Reader, UiMode};

/// Busca si un paper de arXiv ya está presente en la biblioteca curada.
/// Maneja tanto IDs modernos (2401.12345) como clásicos con barra sustituida por guion bajo (hep-th_9901001).
pub(crate) fn find_arxiv_in_library<'a>(
    library_list: &'a [LibraryEntry],
    arxiv_id: &str,
) -> Option<&'a LibraryEntry> {
    let sanitized = arxiv_id.replace('/', "_");
    library_list
        .iter()
        .find(|b| b.name.contains(arxiv_id) || b.name.contains(&sanitized))
}
use crate::persist::{self, PaperMeta};
use android_activity::AndroidApp;
use log::{error, info, warn};
use pdf_core::arxiv::ArxivEntry;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};

impl Reader {
    /// Asegura que el worker Discover esté iniciado con sus rutas de almacenamiento.
    pub(crate) fn ensure_discover_worker(&mut self, app: &AndroidApp) {
        if self.discover.worker.is_some() {
            return;
        }
        let Some(internal_dir) = app.internal_data_path() else {
            warn!("ensure_discover_worker: internal_data_path no disponible");
            return;
        };
        let cache_dir = internal_dir.join("arxiv_cache");
        let (worker, rx) = DiscoverWorker::spawn(cache_dir, internal_dir);
        self.discover.worker = Some(worker);
        self.discover.rx = Some(rx);
    }

    /// ¿Hay trabajo activo en Discover (consulta o descarga en vuelo)?
    /// Usado por `needs_tick` para mantener el polling activo en el bucle de eventos.
    pub(crate) fn discover_busy(&self) -> bool {
        self.discover.is_busy()
    }

    /// ¿Está el paper de arXiv indicado ya presente en la biblioteca?
    pub(crate) fn is_arxiv_in_library(&self, arxiv_id: &str) -> bool {
        find_arxiv_in_library(&self.library_list, arxiv_id).is_some()
    }

    /// Busca la entrada correspondiente a un paper de arXiv en la biblioteca.
    pub(crate) fn find_arxiv_in_library(&self, arxiv_id: &str) -> Option<LibraryEntry> {
        find_arxiv_in_library(&self.library_list, arxiv_id).cloned()
    }

    /// Sondea el canal del worker Discover sin bloquear (`try_recv`).
    /// Si una descarga finaliza, añade la entrada a la biblioteca curada y a `papers.json`.
    pub(crate) fn pump_discover(&mut self, app: &AndroidApp) {
        self.ensure_discover_worker(app);

        loop {
            let msg = {
                let Some(rx) = self.discover.rx.as_ref() else {
                    return;
                };
                match rx.try_recv() {
                    Ok(m) => m,
                    Err(_) => break,
                }
            };
            match msg {
                DiscoverMsg::FeedLoaded { entries, has_more } => {
                    info!(
                        "Discover: feed cargado con {} entradas (has_more: {has_more})",
                        entries.len()
                    );
                    self.discover.feed_entries = entries;
                    self.discover.feed_has_more = has_more;
                    self.discover.phase = DiscoverPhase::Idle;
                    self.discover.worker_busy = false;
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::SearchLoaded {
                    query,
                    entries,
                    has_more,
                } => {
                    info!(
                        "Discover: búsqueda '{query}' cargada con {} entradas (has_more: {has_more})",
                        entries.len()
                    );
                    self.discover.search_entries = entries;
                    self.discover.search_has_more = has_more;
                    self.discover.phase = DiscoverPhase::Idle;
                    self.discover.worker_busy = false;
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::MoreLoaded { entries, has_more } => {
                    info!(
                        "Discover: página adicional cargada con {} entradas",
                        entries.len()
                    );
                    match self.discover.screen {
                        DiscoverScreen::Feed => {
                            self.discover.feed_entries.extend(entries);
                            self.discover.feed_has_more = has_more;
                        }
                        DiscoverScreen::Search => {
                            self.discover.search_entries.extend(entries);
                            self.discover.search_has_more = has_more;
                        }
                        _ => {}
                    }
                    self.discover.phase = DiscoverPhase::Idle;
                    self.discover.worker_busy = false;
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::QueryFailed { error } => {
                    warn!("Discover: consulta fallida: {error}");
                    self.discover.phase = DiscoverPhase::Error(error);
                    self.discover.worker_busy = false;
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::DownloadProgress { id, bytes } => {
                    if self.discover.downloading_id.as_deref() == Some(&id) {
                        self.discover.download_bytes = bytes;
                    }
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::DownloadFinished { id, path, entry } => {
                    info!("Discover: descarga completada: {id} -> {path}");
                    self.discover.downloading_id = None;
                    self.discover.download_bytes = 0;
                    self.discover.worker_busy = false;
                    self.library_add_entry(app, &path, entry.as_ref());
                    self.show_toast(&format!("Descargado: {id}"));
                    self.discover.dirty = true;
                    self.redraw();
                }
                DiscoverMsg::DownloadFailed { id, error } => {
                    warn!("Discover: error en descarga {id}: {error}");
                    self.discover.downloading_id = None;
                    self.discover.download_bytes = 0;
                    self.discover.worker_busy = false;
                    if error != "Descarga cancelada" {
                        self.show_toast(&format!("Fallo descarga {id}: {error}"));
                    }
                    self.discover.dirty = true;
                    self.redraw();
                }
            }
        }
    }

    /// Añade un fichero PDF descargado a la biblioteca curada y persiste sus metadatos en `papers.json`.
    pub(crate) fn library_add_entry(
        &mut self,
        app: &AndroidApp,
        path: &str,
        meta: Option<&ArxivEntry>,
    ) {
        let dest = Path::new(path);
        if !dest.exists() {
            error!("library_add_entry: path no existe: {path}");
            return;
        }

        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("library_add_entry MupdfEngine::new: {e}");
                return;
            }
        };

        let page_count = match engine.open(dest) {
            Ok(doc) => doc.page_count(),
            Err(e) => {
                error!("library_add_entry open {}: {e}", dest.display());
                return;
            }
        };

        let path_str = dest.display().to_string();
        let now = persist::unix_now();

        // 1. Registrar en `library.json` (progreso curado)
        let books = persist::touch_progress(&self.library.lib_books, &path_str, 0, page_count, now);
        self.library.lib_books = books;
        persist::save_progress(self.internal_dir.as_deref(), &self.library.lib_books);

        // 2. Si hay metadatos de arXiv, persistir en `papers.json`
        if let Some(entry) = meta {
            let paper = PaperMeta {
                path: path_str.clone(),
                arxiv_id: entry.id.clone(),
                title: entry.title.clone(),
                authors: entry.authors.clone(),
                updated: entry.updated.clone(),
            };
            let papers = persist::load_papers(self.internal_dir.as_deref());
            let updated_papers = persist::touch_paper(&papers, paper);
            persist::save_papers(self.internal_dir.as_deref(), &updated_papers);
        }

        info!("library_add_entry: libro añadido con éxito ({path_str}, {page_count} págs)");
        self.reload_curated_library(app);
    }

    /// Cambia al modo Discover y asegura la carga inicial del feed.
    pub(crate) fn enter_discover(&mut self, app: &AndroidApp) {
        self.flush_state();
        self.lib_close_ime(app);
        self.mode = UiMode::Discover;
        self.ensure_discover_worker(app);
        self.discover.dirty = true;

        if self.discover.feed_entries.is_empty() && self.discover.phase == DiscoverPhase::Idle {
            self.discover_refresh_feed();
        }
        self.redraw();
    }

    /// Vuelve al modo Biblioteca desde Discover.
    pub(crate) fn exit_discover(&mut self, app: &AndroidApp) {
        self.lib_close_ime(app);
        self.mode = UiMode::Library;
        self.list_dirty = true;
        self.redraw();
    }

    /// Abre el teclado virtual sobre el buscador de Discover.
    pub(crate) fn discover_open_keyboard(&mut self, app: &AndroidApp) {
        crate::jni::ime_attach(app, &self.discover.query);
        self.ime_active = true;
    }

    /// Desplazamiento de scroll actual de Discover según la pantalla activa.
    pub(crate) fn discover_scroll(&self) -> f32 {
        match self.discover.screen {
            DiscoverScreen::Detail => self.discover.detail_scroll,
            DiscoverScreen::Areas => self.discover.areas_scroll,
            DiscoverScreen::Feed | DiscoverScreen::Search => self.discover.scroll,
        }
    }

    /// Fija el desplazamiento de scroll de Discover según la pantalla activa.
    pub(crate) fn set_discover_scroll(&mut self, s: f32) {
        match self.discover.screen {
            DiscoverScreen::Detail => self.discover.detail_scroll = s,
            DiscoverScreen::Areas => self.discover.areas_scroll = s,
            DiscoverScreen::Feed | DiscoverScreen::Search => self.discover.scroll = s,
        }
    }

    /// Alto total del contenido de Discover según la pantalla activa.
    pub(crate) fn discover_content_h(&self) -> f32 {
        match self.discover.screen {
            DiscoverScreen::Detail => {
                if let Some(entry) = &self.discover.selected_entry {
                    crate::reader::disc_detail_layout(self.win_w, entry).1
                } else {
                    400.0
                }
            }
            DiscoverScreen::Areas => {
                crate::reader::discover_categories::ARXIV_CATEGORIES.len() as f32
                    * crate::reader::disc_cat_row_h()
                    + 80.0
            }
            DiscoverScreen::Feed => {
                16.0 + self.discover.feed_entries.len() as f32
                    * (crate::reader::disc_card_h() + crate::reader::disc_card_gap())
                    + 120.0
            }
            DiscoverScreen::Search => {
                16.0 + self.discover.search_entries.len() as f32
                    * (crate::reader::disc_card_h() + crate::reader::disc_card_gap())
                    + 120.0
            }
        }
    }

    /// Desplazamiento máximo de scroll vertical permitido en Discover.
    pub(crate) fn discover_max_scroll(&self) -> f32 {
        let content_y0 = crate::reader::disc_content_y0(
            self.win_h,
            self.discover.screen,
            self.status.is_some() || self.discover.status_msg.is_some(),
        );
        let viewport = (self.win_h - content_y0).max(0) as f32;
        (self.discover_content_h() - viewport).max(0.0)
    }

    /// ¿La banda cacheada actual de Discover cubre el rango visible de scroll?
    pub(crate) fn disc_band_covers(&self) -> bool {
        let Some((_, origin)) = self.discover.band else {
            return false;
        };
        let scroll = self.discover_scroll() as i32;
        let content_y0 = crate::reader::disc_content_y0(
            self.win_h,
            self.discover.screen,
            self.status.is_some() || self.discover.status_msg.is_some(),
        );
        let viewport = (self.win_h - content_y0).max(0);
        let margin = crate::reader::disc_card_h() as i32;
        scroll >= origin && (scroll + viewport) <= (origin + viewport + 2 * margin)
    }
    pub(crate) fn discover_refresh_feed(&mut self) {
        self.discover.phase = DiscoverPhase::Loading;
        let cats = self.discover.selected_cats.clone();
        self.discover.send_cmd(DiscoverCmd::Feed { cats });
        self.discover.dirty = true;
        self.redraw();
    }

    /// Lanza una búsqueda con el texto actual.
    pub(crate) fn discover_search(&mut self, query: &str) {
        let q = query.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.discover.query = q.clone();
        self.discover.phase = DiscoverPhase::Loading;
        self.discover.screen = DiscoverScreen::Search;
        self.discover.active_tab = DiscoverTab::Search;
        self.discover.send_cmd(DiscoverCmd::Search(q));
        self.discover.dirty = true;
        self.redraw();
    }

    /// Solicita más resultados (paginación) para la pantalla activa.
    pub(crate) fn discover_more(&mut self) {
        self.discover.phase = DiscoverPhase::Loading;
        self.discover.send_cmd(DiscoverCmd::More);
        self.redraw();
        self.discover.dirty = true;
    }

    /// Inicia la descarga de un paper por su identificador y metadatos opcionales.
    pub(crate) fn discover_download(&mut self, id: &str, entry: Option<ArxivEntry>) {
        self.discover.downloading_id = Some(id.to_string());
        self.discover.download_bytes = 0;
        self.discover.send_cmd(DiscoverCmd::Download {
            id: id.to_string(),
            entry,
        });
        self.discover.dirty = true;
        self.redraw();
    }
}
