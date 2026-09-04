// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Byte-bounded LRU cache of rendered pages (docs/PLAN.md §3.3 `cache`, Fase 1).
//!
//! `RenderCache` wraps a `RenderEngine` document and keeps a least-recently-used
//! map keyed by `(page, scale_level)`, evicting entries until the total bytes of
//! resident bitmaps fit `byte_budget`. Every byte figure comes from real bitmap
//! dimensions (`width * height * 4` for RGBA8), never from estimates. Hits never
//! re-render; the only render path is `get_or_render` on a miss.

use std::path::Path;

use lru::LruCache;

use crate::engine::{Bitmap, Document, Error, RenderEngine, Result};
use crate::scroll::{Viewport, visible_and_prefetch_pages};

/// Identifies one cached bitmap: a document page at a zoom level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageKey {
    pub page_idx: usize,
    pub scale_level: u32,
}

/// A rendered page plus its real memory footprint in bytes.
#[derive(Debug)]
pub struct RenderedPage {
    pub bitmap: Bitmap,
    /// `bitmap.width * bitmap.height * 4` (RGBA8): the true buffer size.
    pub byte_size: usize,
}

/// Cumulative cache counters, exposed for the debug overlay (docs/PLAN.md §3.5).
#[derive(Debug, Default, Clone, Copy)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Sum of `byte_size` over the resident entries.
    pub current_bytes: usize,
    pub entries: usize,
}

/// Zoom ladder used by the cache: level 0 = 1.0 (72 dpi), 1 = 2.0 (144 dpi),
/// 2 = 4.0 (288 dpi), ... — maps a `PageKey::scale_level` to the `f32` scale
/// that `RenderEngine::Document::render_page` expects.
pub fn scale_for_level(level: u32) -> f32 {
    2.0_f32.powi(level as i32)
}

/// Zoom level used by the prefetch/population paths (72 dpi, cheap).
pub const DEFAULT_SCALE_LEVEL: u32 = 0;

/// Least-recently-used cache of rendered pages bounded by `byte_budget`.
///
/// A miss renders with the engine, stores the bitmap and evicts LRU entries
/// until the resident bytes fit the budget. A single page larger than the whole
/// budget evicts everything and is stored alone (the budget may then be
/// exceeded by that one page — documented best-effort behaviour).
pub struct RenderCache<E: RenderEngine> {
    #[allow(dead_code)] // held for B2 reopen/invalidate; read only at construction
    engine: E,
    doc: E::Document,
    map: LruCache<PageKey, RenderedPage>,
    byte_budget: usize,
    current_bytes: usize,
    stats: CacheStats,
}

impl<E: RenderEngine> RenderCache<E> {
    /// Wraps an already-opened document in a byte-bounded LRU cache.
    pub fn new(engine: E, doc: E::Document, byte_budget: usize) -> Self {
        Self {
            engine,
            doc,
            map: LruCache::unbounded(),
            byte_budget,
            current_bytes: 0,
            stats: CacheStats::default(),
        }
    }

    /// Convenience: opens `path` with the engine and wraps the document in a
    /// byte-bounded LRU cache.
    pub fn open(engine: E, path: &Path, byte_budget: usize) -> Result<Self> {
        let doc = engine.open(path)?;
        Ok(Self::new(engine, doc, byte_budget))
    }

    /// Number of pages in the wrapped document.
    pub fn page_count(&self) -> usize {
        self.doc.page_count() as usize
    }

    /// Returns the cached page, rendering it on a miss.
    ///
    /// A hit costs no render and promotes the entry (true LRU recency). A miss
    /// renders, inserts and evicts least-recently-used entries until
    /// `current_bytes + incoming <= byte_budget`.
    pub fn get_or_render(&mut self, page_idx: usize, scale_level: u32) -> Result<&RenderedPage> {
        let key = PageKey {
            page_idx,
            scale_level,
        };

        // `get` both detects residency and promotes recency (no re-render).
        if self.map.get(&key).is_some() {
            self.stats.hits += 1;
        } else {
            self.stats.misses += 1;
            let scale = scale_for_level(scale_level);
            let bitmap = self.doc.render_page(page_idx as u32, scale)?;
            let byte_size = bitmap.width as usize * bitmap.height as usize * 4;
            self.evict_to_fit(byte_size);
            self.map.put(key, RenderedPage { bitmap, byte_size });
            self.current_bytes += byte_size;
        }

        self.stats.current_bytes = self.current_bytes;
        self.stats.entries = self.map.len();
        self.map.get(&key).ok_or_else(|| {
            Error::Engine(format!(
                "cache entry missing after get_or_render (page {page_idx}, level {scale_level})"
            ))
        })
    }

    /// Renders (on miss) the visible window plus `prefetch_radius` neighbours
    /// on each side, honouring the byte budget. Synchronous, no threads
    /// (background rendering is Fase 1 B2).
    pub fn ensure_visible(&mut self, vp: &Viewport, prefetch_radius: usize) -> Result<()> {
        let range = visible_and_prefetch_pages(vp, self.page_count(), prefetch_radius);
        for page in range {
            self.get_or_render(page, DEFAULT_SCALE_LEVEL)?;
        }
        Ok(())
    }

    /// Snapshot of the cache counters.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Distinct page indices currently resident, most-recently-used first
    /// (a page cached at several scale levels appears once). Useful for the
    /// B2 prefetch/invalidation logic and for hit-path benches.
    pub fn resident_pages(&self) -> Vec<usize> {
        let mut seen = std::collections::HashSet::new();
        self.map
            .iter()
            .map(|(k, _)| k.page_idx)
            .filter(|page| seen.insert(*page))
            .collect()
    }

    /// Full resident keys `(page, scale_level)`, most-recently-used first.
    /// Used by the prefetcher's thread-safe `resident_pages` snapshot.
    pub fn resident_keys(&self) -> Vec<PageKey> {
        self.map.iter().map(|(k, _)| *k).collect()
    }

    /// Deep copy of the bitmap cached at `(page_idx, scale_level)`, if
    /// resident. A pure peek: does NOT render on a miss and does NOT promote
    /// LRU recency (unlike `get_or_render`) nor touch the hit counter.
    ///
    /// Backs `Prefetcher::get_page` (pdf_app contract): the worker answers
    /// `Some(bitmap)` for a resident page without moving it out of the cache.
    pub fn peek_clone(&self, page_idx: usize, scale_level: u32) -> Option<Bitmap> {
        let key = PageKey {
            page_idx,
            scale_level,
        };
        self.map.peek(&key).map(|page| Bitmap {
            width: page.bitmap.width,
            height: page.bitmap.height,
            data: page.bitmap.data.clone(),
        })
    }

    /// Evicts every resident entry cached at a `scale_level` other than
    /// `keep_level`, keeping `current_bytes` and the eviction counter in sync.
    ///
    /// Why: after a zoom change the old ladder level is dead weight — its
    /// bitmaps can never be reused (each level is a distinct render) and would
    /// only consume byte budget that the new level's crisp re-renders need.
    /// Trimming before populating the new level avoids budget thrashing:
    /// without it, evicting a page only to re-render it at the new level a
    /// moment later would churn the LRU and double-render.
    pub fn trim_to_scale_level(&mut self, keep_level: u32) {
        // lru 0.18 has no retain(); collect stale keys first (iter is
        // borrow-free) and pop them with explicit byte accounting.
        let stale: Vec<PageKey> = self
            .map
            .iter()
            .filter(|(k, _)| k.scale_level != keep_level)
            .map(|(k, _)| *k)
            .collect();
        let mut evicted_bytes = 0usize;
        let mut evicted = 0u64;
        for key in stale {
            if let Some(page) = self.map.pop(&key) {
                evicted_bytes += page.byte_size;
                evicted += 1;
            }
        }
        self.current_bytes -= evicted_bytes;
        self.stats.evictions += evicted;
        self.stats.current_bytes = self.current_bytes;
        self.stats.entries = self.map.len();
    }

    /// Drops every resident page and resets the byte accounting. The engine and
    /// document stay open; counters keep their history.
    pub fn clear(&mut self) {
        self.map.clear();
        self.current_bytes = 0;
        self.stats.current_bytes = 0;
        self.stats.entries = 0;
    }

    /// Evicts least-recently-used entries until `current_bytes + incoming`
    /// fits `byte_budget`. If a single incoming page exceeds the whole budget,
    /// evicts everything and lets it in alone.
    fn evict_to_fit(&mut self, incoming_bytes: usize) {
        while self.current_bytes + incoming_bytes > self.byte_budget && !self.map.is_empty() {
            if let Some((_, victim)) = self.map.pop_lru() {
                self.current_bytes -= victim.byte_size;
                self.stats.evictions += 1;
            }
        }
    }
}
