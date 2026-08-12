//! MuPDF backend: safe wrapper crate `mupdf` (MuPDF 1.27.2, AGPL-3.0).
//! Phase 0.5 benchmark candidate vs PDFium; see ADR-001 for the decision.

use std::path::Path;

use mupdf::{Colorspace, Document as MupdfDocument, Matrix};

use crate::engine::{Bitmap, Document, Error, RenderEngine, Result};

// Unlike pdfium, mupdf-rs is thread-safe by design: each thread gets its own
// fz_context (thread_local in mupdf_sys, derived from a mutex-protected base
// context), so no serialization lock is needed around native calls.
#[derive(Clone, Copy)]
pub struct MupdfEngine;

impl MupdfEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MupdfEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderEngine for MupdfEngine {
    type Document = MupdfDoc;

    fn open(&self, path: &Path) -> Result<MupdfDoc> {
        let inner = MupdfDocument::open(path)
            .map_err(|e| Error::Engine(format!("open {}: {e}", path.display())))?;
        Ok(MupdfDoc { inner })
    }
}

pub struct MupdfDoc {
    inner: MupdfDocument,
}

impl MupdfDoc {
    fn load_page(&self, page: u32) -> Result<mupdf::Page> {
        let page_count = self.page_count();
        if page >= page_count {
            return Err(Error::PageOutOfRange { page, page_count });
        }
        self.inner
            .load_page(page as i32)
            .map_err(|e| Error::Engine(e.to_string()))
    }
}

impl Document for MupdfDoc {
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
        let mupdf_page = self.load_page(page)?;
        let pixmap = mupdf_page
            .to_pixmap(
                &Matrix::new_scale(scale, scale),
                &Colorspace::device_rgb(),
                true,
                true,
            )
            .map_err(|e| Error::Engine(e.to_string()))?;

        Ok(Bitmap {
            width: pixmap.width(),
            height: pixmap.height(),
            data: pixmap.samples().to_vec(),
        })
    }
}
