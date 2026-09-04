// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! MuPDF backend: crate `mupdf` 0.8 (messense/mupdf-rs) over MuPDF C
//! (AGPL-3.0 — chosen in ADR-001, Fase 0.5). Single engine, always compiled.
//!
//! MuPDF is linked statically, so unlike a dynamically-bound backend there is
//! no external library to bind: `mupdf::Context::get()` lazily initializes a
//! global base context and clones a private one per thread, which makes MuPDF
//! thread-safe by construction (no shared mutable native state, so no
//! process-wide lock is needed).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use mupdf::{Colorspace, Matrix, TextBlockContent, TextPageFlags};

use crate::engine::{Bitmap, Document, Error, PageText, RenderEngine, Result, TextSpan};

pub struct MupdfEngine;

impl MupdfEngine {
    /// MuPDF links statically and bootstraps its context on first use, so
    /// there is nothing to bind. `new` exists to mirror the engine creation
    /// pattern (the single place where init errors surface) and to force the
    /// one-time context initialization early.
    pub fn new() -> Result<Self> {
        // Touches the lazily-initialized global base context; panics (not
        // errors) only on catastrophic allocator failure.
        let _ctx = mupdf::Context::get();
        Ok(Self)
    }
}

impl Default for MupdfEngine {
    fn default() -> Self {
        // `new` never fails today (context init panics only on catastrophic
        // allocator failure, handled inside mupdf); `unwrap_or` keeps this
        // panic-free per the no-`expect` production rule.
        Self::new().unwrap_or(Self)
    }
}

impl RenderEngine for MupdfEngine {
    type Document = MupdfDocument;

    fn open(&self, path: &Path) -> Result<Self::Document> {
        let doc = mupdf::Document::open(path)
            .map_err(|e| Error::Engine(format!("open {}: {e}", path.display())))?;
        Ok(MupdfDocument {
            inner: doc,
            display_lists: RefCell::new(HashMap::new()),
        })
    }
}

pub struct MupdfDocument {
    inner: mupdf::Document,
    /// Per-page display lists (F3.3), built lazily on first render of the
    /// page and reused for every subsequent scale change: running
    /// `fz_run_display_list` skips the PDF object parse + command-tree walk
    /// that `Page::to_pixmap` pays on every zoom (2-4× faster, F0 spike).
    /// `RefCell` (not `Mutex`): `&mut self` interior mutability over a
    /// shared `&self` — the render paths are single-threaded per document
    /// instance (the worker owns its document), so there is no concurrent
    /// access to alias. This relies on `MupdfDocument` NOT being `Sync`
    /// (mupdf-rs types are plain FFI pointers, no auto traits), which the
    /// compiler enforces if a future caller shares it across threads.
    display_lists: RefCell<HashMap<u32, mupdf::DisplayList>>,
}

impl MupdfDocument {
    /// Loads `page` (0-based), checking the range before loading.
    fn load_page(&self, page: u32) -> Result<mupdf::Page> {
        let page_count = self.page_count();
        if page >= page_count {
            return Err(Error::PageOutOfRange { page, page_count });
        }
        let page = self
            .inner
            .load_page(page as i32)
            .map_err(|e| Error::Engine(e.to_string()))?;
        Ok(page)
    }

    /// Display list of `page`, building it on first use (F3.3). Retained in
    /// `display_lists` for the document's lifetime; dropped with the map.
    fn display_list_for(&self, page: u32) -> Result<std::cell::Ref<'_, mupdf::DisplayList>> {
        if !self.display_lists.borrow().contains_key(&page) {
            let list = self
                .load_page(page)?
                .to_display_list(true)
                .map_err(|e| Error::Engine(e.to_string()))?;
            self.display_lists.borrow_mut().insert(page, list);
        }
        Ok(std::cell::Ref::map(self.display_lists.borrow(), |m| {
            &m[&page]
        }))
    }

    /// RGBA8 `Bitmap` from a 3-component RGB pixmap (row expansion, alpha =
    /// 255; rows may be padded to `stride`, so copy `width * n` per row).
    fn pixmap_to_bitmap(pixmap: &mupdf::Pixmap) -> Result<Bitmap> {
        let width = pixmap.width() as usize;
        let height = pixmap.height() as usize;
        let n = pixmap.n() as usize; // components per pixel (3 for RGB)
        let stride = pixmap.stride() as usize;
        let samples = pixmap.samples();
        let mut data = Vec::with_capacity(width * height * 4);
        for row in samples.chunks(stride).take(height) {
            for px in row[..width * n].chunks_exact(n) {
                data.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
        Ok(Bitmap {
            width: pixmap.width(),
            height: pixmap.height(),
            data,
        })
    }
}

impl Document for MupdfDocument {
    fn page_count(&self) -> u32 {
        self.inner
            .page_count()
            .map(|n| n.max(0) as u32)
            .unwrap_or(0)
    }

    fn page_size(&self, page: u32) -> Result<(f32, f32)> {
        let bounds = self
            .load_page(page)?
            .bounds()
            .map_err(|e| Error::Engine(e.to_string()))?;
        Ok((bounds.x1 - bounds.x0, bounds.y1 - bounds.y0))
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap> {
        // F3.3: rasterize from the page's display list. The list is built
        // once per page (the expensive parse + command-tree walk) and every
        // later zoom only replays it into a fresh pixmap through
        // `fz_run_display_list` — the 2-4× path measured in the F0 spike
        // (`docs/research/gpu-rendering-pipeline.md`). First render of a
        // page pays the list build; subsequent renders at any scale reuse it.
        let list = self.display_list_for(page)?;
        let pixmap = list
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_rgb(),
                false,
            )
            .map_err(|e| Error::Engine(e.to_string()))?;
        Self::pixmap_to_bitmap(&pixmap)
    }

    fn text(&self, page: u32) -> Result<PageText> {
        let page = self.load_page(page)?;

        // `to_text_page` runs MuPDF's structured-text (stext) extractor with
        // default flags; `to_text` yields the plain text in reading order and
        // `structured` the per-line spans with bounding boxes in page points
        // (y grows downward, same space as `page_size`). Both come from the
        // same stext page, so spans stay consistent with `text`.
        let stext = page
            .to_text_page(TextPageFlags::empty())
            .map_err(|e| Error::Engine(e.to_string()))?;
        let text = stext.to_text().map_err(|e| Error::Engine(e.to_string()))?;
        let spans = stext
            .structured()
            .blocks
            .iter()
            .filter_map(|b| match &b.content {
                TextBlockContent::Text { lines } => Some(lines),
                _ => None,
            })
            .flatten()
            .map(|line| TextSpan {
                text: line.text.clone(),
                x: line.bounds.x0,
                y: line.bounds.y0,
                w: line.bounds.x1 - line.bounds.x0,
                h: line.bounds.y1 - line.bounds.y0,
            })
            .collect();

        Ok(PageText { text, spans })
    }
}
