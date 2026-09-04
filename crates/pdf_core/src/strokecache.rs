// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Fase C2: Caché de mapa de bits de la capa de tinta (`StrokeCache`).
//!
//! Evita re-rasterizar N trazos por frame cuando la página no ha sufrido
//! cambios en sus anotaciones de tipo trazo (`Stroke`). Combina la
//! composición de rectángulos de resaltado (`Highlight`) con el blit
//! source-over directo de la capa de tinta cacheada.

use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;

use crate::annotations::{Annotated, Annotation};
use crate::engine::Bitmap;
use crate::overlay::{
    ViewTransform, blit_stroke_layer, composite_highlights, composite_strokes_alpha,
};

/// Capacidad por defecto: 4 páginas (~50 MB a 1440×2200).
const DEFAULT_CAPACITY: usize = 4;

/// Clave de la capa de trazos rasterizada.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StrokeKey {
    pub page_idx: usize,
    pub zoom_bits: u32,
    pub width: u32,
    pub height: u32,
}

impl StrokeKey {
    pub fn new(page_idx: usize, zoom: f32, width: u32, height: u32) -> Self {
        Self {
            page_idx,
            zoom_bits: zoom.to_bits(),
            width,
            height,
        }
    }
}

/// Caché LRU de la capa de trazos rasterizada por página y escala.
pub struct StrokeCache {
    lru: LruCache<StrokeKey, Arc<Bitmap>>,
}

impl Default for StrokeCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl StrokeCache {
    /// Crea una nueva caché con capacidad para `capacity` páginas.
    pub fn new(capacity: usize) -> Self {
        Self {
            lru: LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap_or(NonZeroUsize::MIN)),
        }
    }

    /// Obtiene la capa de trazos rasterizada o la genera en caso de miss.
    /// Si `anns` no contiene ningún trazo (`Annotation::Stroke`), devuelve
    /// `None` sin alocar memoria.
    pub fn get_or_render(
        &mut self,
        key: StrokeKey,
        anns: &[&Annotated],
        xform: &ViewTransform,
    ) -> Option<Arc<Bitmap>> {
        let has_strokes = anns.iter().any(|a| matches!(a.kind, Annotation::Stroke(_)));
        if !has_strokes {
            return None;
        }

        if let Some(cached) = self.lru.get(&key) {
            return Some(cached.clone());
        }

        let (w, h) = (key.width, key.height);
        let len = (w as usize) * (h as usize) * 4;
        let mut data = vec![0u8; len];

        composite_strokes_alpha(&mut data, w, h, anns, xform);

        let bitmap = Arc::new(Bitmap {
            width: w,
            height: h,
            data,
        });

        self.lru.put(key, bitmap.clone());
        Some(bitmap)
    }

    /// Invalida las entradas cacheadas correspondientes a `page_idx`.
    pub fn invalidate_page(&mut self, page_idx: usize) {
        let to_remove: Vec<StrokeKey> = self
            .lru
            .iter()
            .filter(|(k, _)| k.page_idx == page_idx)
            .map(|(k, _)| *k)
            .collect();
        for k in to_remove {
            self.lru.pop(&k);
        }
    }

    /// Vacía todas las capas cacheadas.
    pub fn clear(&mut self) {
        self.lru.clear();
    }

    /// Compone las anotaciones de la página sobre `buf` usando la caché para
    /// los trazos. Primero rasteriza los resaltados (`Highlight`) y luego
    /// funde la capa de trazos cacheada.
    pub fn composite(
        &mut self,
        buf: &mut [u8],
        width: u32,
        height: u32,
        page_idx: usize,
        anns: &[&Annotated],
        xform: &ViewTransform,
    ) {
        composite_highlights(buf, width, height, anns, xform);

        let key = StrokeKey::new(page_idx, xform.zoom, width, height);
        if let Some(layer) = self.get_or_render(key, anns, xform) {
            blit_stroke_layer(buf, width, height, &layer);
        }
    }
}
