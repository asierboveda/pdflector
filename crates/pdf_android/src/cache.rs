// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Caché LRU de páginas renderizadas (`struct PageCache`, página → `Bitmap`)
//! para el scroll vertical continuo del visor: guarda los bitmaps de pdf_core
//! y los reutiliza al volver atrás o al desplazarse, evitando el re-render de
//! MuPDF (~18-25 ms/página en la tablet) en el camino caliente del blit.
//!
//! ## Política (límites y evicción — documentada)
//!
//! - **Clave**: índice de página (`u32`). Todos los bitmaps residentes están
//!   renderizados a la MISMA escala (`cover × rendered_zoom`, ver `reader`):
//!   al cambiar el zoom, la ventana o el documento se invalida con `clear()`
//!   (una escala distinta es un render distinto; conservar niveles antiguos
//!   solo consumiría presupuesto — misma regla que `pdf_core::cache`).
//! - **Presupuesto de bytes**: 48 MiB (`CACHE_BYTE_BUDGET`). En la tablet
//!   (pantalla ≈ 1200×2000 px en portrait) una página a escala cover ocupa
//!   ≈ 1200 × 2000 × 4 B ≈ 9,2 MiB; el presupuesto admite 5 páginas
//!   (~46 MiB), coherente con el objetivo RSS < 150 MB del proyecto: la caché
//!   queda en ~⅓ del presupuesto total (el resto es el .so, MuPDF y los
//!   buffers de ventana).
//! - **Tope de entradas**: 5 (`CACHE_MAX_ENTRIES`). Suficiente para "volver
//!   atrás instantáneo" (2-3 páginas hacia atrás) y para el prefetch ±1
//!   vecina del viewport, sin dejar crecer la cola LRU.
//! - **Evicción**: least-recently-used. `get` promueve la entrada (recencia
//!   real, evita re-render en el render de cada frame); `insert` expulsa del
//!   frente de la cola LRU hasta cumplir `byte_budget` y `max_entries`. Si una
//!   única página supera todo el presupuesto (zoom alto: una página a 8× puede
//!   pesar cientos de MiB) se expulsa TODO y entra sola — best-effort,
//!   idéntico a `pdf_core::cache`, y el footprint coincide con el del
//!   `bitmap` único que ya alojaba la app antes de la caché.
//! - **Modo oscuro**: la caché guarda SIEMPRE bitmaps normales (de colores).
//!   La inversión (255 − v) se aplica al blitear, por página, solo si el modo
//!   oscuro está activo (ver `draw::blit_page`); nunca se almacena una
//!   variante invertida.
//!
//! `HashMap` + `VecDeque` (cola de recencia, frente = LRU): con ≤ 5 entradas
//! la promoción es O(n) con n ≤ 5 y no hace falta una crate LRU dedicada.

use std::collections::{HashMap, VecDeque};

use pdf_core::Bitmap;

/// Presupuesto máximo de bytes residentes de la caché: ≈ 5 páginas a escala
/// cover de pantalla completa en la tablet (ver cabecera del módulo).
pub(crate) const CACHE_BYTE_BUDGET: usize = 48 * 1024 * 1024;

/// Tope de entradas (páginas) residentes.
pub(crate) const CACHE_MAX_ENTRIES: usize = 5;

/// Bytes que ocupa un `Bitmap` RGBA8 (cifra real del buffer, nunca estimada).
fn bitmap_bytes(bmp: &Bitmap) -> usize {
    bmp.width as usize * bmp.height as usize * 4
}

/// Caché LRU de páginas renderizadas (página → `Bitmap`), limitada por bytes
/// y por nº de entradas.
pub(crate) struct PageCache {
    map: HashMap<u32, Bitmap>,
    /// Cola de recencia: el frente es el least-recently-used (primera víctima).
    lru: VecDeque<u32>,
    /// Bytes totales de los bitmaps residentes.
    bytes: usize,
    byte_budget: usize,
    max_entries: usize,
}

impl PageCache {
    pub(crate) fn new(byte_budget: usize, max_entries: usize) -> Self {
        Self {
            map: HashMap::new(),
            lru: VecDeque::new(),
            bytes: 0,
            byte_budget,
            max_entries,
        }
    }

    /// Lookup que PROMUEVE la entrada (recencia LRU). Lo usa el render
    /// (`ensure_pages_rendered`): un hit evita el re-render y marca la página
    /// como recientemente usada.
    #[allow(dead_code)] // el flujo async usa `peek`; get queda como API
    pub(crate) fn get(&mut self, page: u32) -> Option<&Bitmap> {
        if self.map.contains_key(&page) {
            self.promote(page);
        }
        self.map.get(&page)
    }

    /// Lookup SIN promoción: el blit de cada frame lee las páginas visibles
    /// sin reordenar la recencia (el orden lo fija el render/prefetch).
    pub(crate) fn peek(&self, page: u32) -> Option<&Bitmap> {
        self.map.get(&page)
    }

    /// Inserta (o reemplaza) el bitmap de `page`, expulsando LRU hasta caber
    /// en `byte_budget` y `max_entries`. Una página que supera todo el
    /// presupuesto expulsa el resto y entra sola (best-effort, ver cabecera).
    pub(crate) fn insert(&mut self, page: u32, bitmap: Bitmap) {
        let incoming = bitmap_bytes(&bitmap);
        // Reemplazo de una página ya residente: liberar sus bytes y su hueco
        // en la cola antes de reinsertarla como la más reciente.
        if let Some(old) = self.map.remove(&page) {
            self.bytes -= bitmap_bytes(&old);
            if let Some(pos) = self.lru.iter().position(|&p| p == page) {
                self.lru.remove(pos);
            }
        }
        while self.map.len() >= self.max_entries && !self.lru.is_empty() {
            self.evict_lru();
        }
        while self.bytes + incoming > self.byte_budget && !self.lru.is_empty() {
            self.evict_lru();
        }
        self.bytes += incoming;
        self.map.insert(page, bitmap);
        self.lru.push_back(page);
    }

    /// Descarta todo (cambio de zoom, de ventana o de documento): los bitmaps
    /// viejos son de otra escala y nunca se reutilizarían.
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.lru.clear();
        self.bytes = 0;
    }

    /// Nº de páginas residentes (para el log de debug).
    #[allow(dead_code)] // métrica de debug
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Bytes totales residentes (para el log de debug).
    #[allow(dead_code)] // métrica de debug
    pub(crate) fn resident_bytes(&self) -> usize {
        self.bytes
    }

    #[allow(dead_code)]
    fn promote(&mut self, page: u32) {
        if let Some(pos) = self.lru.iter().position(|&p| p == page) {
            self.lru.remove(pos);
            self.lru.push_back(page);
        }
    }

    fn evict_lru(&mut self) {
        if let Some(victim) = self.lru.pop_front()
            && let Some(bmp) = self.map.remove(&victim)
        {
            self.bytes -= bitmap_bytes(&bmp);
        }
    }
}
