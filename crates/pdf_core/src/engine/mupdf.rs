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
        Self::new().expect("mupdf engine init")
    }
}

impl RenderEngine for MupdfEngine {
    type Document = MupdfDocument;

    fn open(&self, path: &Path) -> Result<Self::Document> {
        let doc = mupdf::Document::open(path)
            .map_err(|e| Error::Engine(format!("open {}: {e}", path.display())))?;
        Ok(MupdfDocument { inner: doc })
    }
}

pub struct MupdfDocument {
    inner: mupdf::Document,
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
        let page = self.load_page(page)?;

        // `to_pixmap` renders the page bounding box through the given matrix;
        // a uniform scale from PDF points to device pixels, no alpha so the
        // samples are 3-component RGB.
        let pixmap = page
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_rgb(),
                false,
                true,
            )
            .map_err(|e| Error::Engine(e.to_string()))?;

        let width = pixmap.width() as usize;
        let height = pixmap.height() as usize;
        let n = pixmap.n() as usize; // components per pixel (3 for RGB)
        let stride = pixmap.stride() as usize;
        let samples = pixmap.samples();

        // MuPDF stores rows bottom-up in PDF terms? No: pixmap samples are
        // top-down row-major. Rows may be padded to `stride` bytes, so copy
        // `width * n` bytes per row and expand RGB to RGBA (alpha = 255).
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
