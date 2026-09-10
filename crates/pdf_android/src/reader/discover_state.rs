// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Estado de la pestaña y pantallas de Descubrir (arXiv Discover).
//!
//! Agrupa en `DiscoverState` la navegación entre pantallas (Feed, Búsqueda, Ficha, Áreas),
//! el estado del worker de fondo, los resultados cacheados y el progreso de descargas.

use std::path::Path;
use std::sync::mpsc::Receiver;

use pdf_core::arxiv::ArxivEntry;

use super::discover_categories::default_categories;
use crate::discover::{DiscoverCmd, DiscoverMsg, DiscoverWorker};
use crate::persist::{DiscoverPrefs, load_discover, save_discover};

/// Pantalla activa dentro del modo Discover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoverScreen {
    /// Feed cronológico según categorías seleccionadas.
    Feed,
    /// Búsqueda por texto o por campos arXiv.
    Search,
    /// Ficha de detalle de un paper antes de descargar.
    Detail,
    /// Selector de áreas / categorías temáticas de interés.
    Areas,
}

/// Pestaña seleccionada en la barra superior de Discover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoverTab {
    Feed,
    Search,
    Areas,
}

/// Fase del estado de red o carga de Discover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoverPhase {
    Idle,
    Loading,
    Error(String),
}

/// Estado global de Discover mantenido en `Reader`.
pub struct DiscoverState {
    /// Pantalla actualmente visible.
    pub screen: DiscoverScreen,
    /// Pestaña activa en la cabecera.
    pub active_tab: DiscoverTab,
    /// Fase de carga del feed o búsqueda.
    pub phase: DiscoverPhase,
    /// Texto de consulta en el buscador.
    pub query: String,
    /// ¿El teclado virtual está abierto para el buscador?
    pub ime_active: bool,
    /// Categorías seleccionadas por el usuario para su feed.
    pub selected_cats: Vec<String>,
    /// Entradas cargadas en el feed principal.
    pub feed_entries: Vec<ArxivEntry>,
    /// ¿Hay más páginas disponibles en el feed?
    pub feed_has_more: bool,
    /// Entradas devueltas por la búsqueda activa.
    pub search_entries: Vec<ArxivEntry>,
    /// ¿Hay más páginas disponibles en la búsqueda?
    pub search_has_more: bool,
    /// Entrada seleccionada mostrada en la pantalla de Ficha (`Detail`).
    pub selected_entry: Option<ArxivEntry>,
    /// Identificador canónico del paper descargándose actualmente.
    pub downloading_id: Option<String>,
    /// Bytes descargados en la descarga en curso.
    pub download_bytes: u64,
    /// Mensaje de aviso o notificación breve (toast/status).
    pub status_msg: Option<String>,
    /// Desplazamiento vertical del scroll en píxeles.
    pub scroll: f32,
    /// Desplazamiento vertical en la pantalla de Ficha.
    pub detail_scroll: f32,
    /// Desplazamiento vertical en la pantalla de Áreas.
    pub areas_scroll: f32,
    /// Worker de fondo en un hilo dedicado con 1 conexión HTTP.
    pub worker: Option<DiscoverWorker>,
    /// Canal de recepción de mensajes desde el worker.
    pub rx: Option<Receiver<DiscoverMsg>>,
    /// ¿Hay una petición o descarga en curso en el worker?
    pub worker_busy: bool,
}

impl DiscoverState {
    pub fn new(internal_dir: Option<&Path>) -> Self {
        let prefs = load_discover(internal_dir);
        let selected_cats = if prefs.cats.is_empty() {
            default_categories()
        } else {
            prefs.cats
        };

        Self {
            screen: DiscoverScreen::Feed,
            active_tab: DiscoverTab::Feed,
            phase: DiscoverPhase::Idle,
            query: String::new(),
            ime_active: false,
            selected_cats,
            feed_entries: Vec::new(),
            feed_has_more: false,
            search_entries: Vec::new(),
            search_has_more: false,
            selected_entry: None,
            downloading_id: None,
            download_bytes: 0,
            status_msg: None,
            scroll: 0.0,
            detail_scroll: 0.0,
            areas_scroll: 0.0,
            worker: None,
            rx: None,
            worker_busy: false,
        }
    }

    /// Guarda las categorías seleccionadas en `discover.json`.
    pub fn persist_prefs(&self, internal_dir: Option<&Path>) {
        let prefs = DiscoverPrefs {
            cats: self.selected_cats.clone(),
        };
        save_discover(internal_dir, &prefs);
    }

    /// Alterna la selección de una categoría en la lista de áreas.
    pub fn toggle_category(&mut self, code: &str, internal_dir: Option<&Path>) {
        if let Some(pos) = self.selected_cats.iter().position(|c| c == code) {
            self.selected_cats.remove(pos);
        } else {
            self.selected_cats.push(code.to_string());
        }
        self.persist_prefs(internal_dir);
    }

    /// Envía un comando al worker si está inicializado.
    pub fn send_cmd(&mut self, cmd: DiscoverCmd) {
        if let Some(w) = self.worker.as_ref() {
            self.worker_busy = true;
            w.send(cmd);
        }
    }

    /// Cancela la descarga u operación en vuelo.
    pub fn cancel(&mut self) {
        if let Some(w) = self.worker.as_ref() {
            w.cancel();
        }
        self.downloading_id = None;
        self.download_bytes = 0;
        self.worker_busy = false;
        self.phase = DiscoverPhase::Idle;
    }

    /// ¿El worker está procesando una consulta o descarga?
    pub fn is_busy(&self) -> bool {
        self.worker_busy || self.downloading_id.is_some() || self.phase == DiscoverPhase::Loading
    }
}
