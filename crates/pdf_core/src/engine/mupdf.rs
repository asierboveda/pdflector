// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! MuPDF backend: crate `mupdf` 0.8 (messense/mupdf-rs) over MuPDF C
//! (AGPL-3.0 — chosen in ADR-001). Single engine, always compiled.
//!
//! MuPDF is linked statically, so unlike a dynamically-bound backend there is
//! no external library to bind: `mupdf::Context::get()` lazily initializes a
//! global base context and clones a private one per thread, which makes MuPDF
//! thread-safe by construction (no shared mutable native state, so no
//! process-wide lock is needed).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::Path;

use mupdf::{Colorspace, Matrix, TextBlockContent, TextPageFlags};

use crate::engine::{
    Bitmap, Document, Error, PageGeometry, PageRect, PageText, PageTransform, RenderEngine, Result,
    TextSpan,
};

pub struct MupdfEngine;

const PAGE_GEOMETRY_CACHE_CAPACITY: usize = 5;

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
            page_geometries: RefCell::new(VecDeque::new()),
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
    /// Tiny immutable page metadata is retained after first access so frame
    /// and gesture paths never ask MuPDF to recalculate the page transform.
    page_geometries: RefCell<VecDeque<(u32, PageGeometry)>>,
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

    /// Reads and promotes a geometry entry in the five-page LRU. Metadata is
    /// small, but an unbounded per-document map grows with every page visited.
    fn cached_page_geometry(&self, page: u32) -> Option<PageGeometry> {
        let mut cache = self.page_geometries.borrow_mut();
        let index = cache
            .iter()
            .position(|(cached_page, _)| *cached_page == page)?;
        let (_, geometry) = cache.remove(index)?;
        cache.push_front((page, geometry));
        Some(geometry)
    }

    fn cache_page_geometry(&self, page: u32, geometry: PageGeometry) {
        let mut cache = self.page_geometries.borrow_mut();
        if let Some(index) = cache
            .iter()
            .position(|(cached_page, _)| *cached_page == page)
        {
            cache.remove(index);
        }
        cache.push_front((page, geometry));
        while cache.len() > PAGE_GEOMETRY_CACHE_CAPACITY {
            cache.pop_back();
        }
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

    /// Reads an inherited PDF page box without applying page rotation. The
    /// mupdf-rs `PdfPage::crop_box()` helper combines the rotated bounds with
    /// the crop-box offset, which does not describe the source box on rotated
    /// pages; the page CTM handles rotation separately.
    fn pdf_box(page: &mupdf::pdf::PdfPage, key: &str) -> Result<Option<PageRect>> {
        let object = page.object();
        let Some(array) = object
            .get_dict_inheritable(key)
            .map_err(|e| Error::Engine(e.to_string()))?
        else {
            return Ok(None);
        };
        let mut coordinates = [0.0f32; 4];
        for (index, coordinate) in coordinates.iter_mut().enumerate() {
            let value = array
                .get_array(index as i32)
                .map_err(|e| Error::Engine(e.to_string()))?
                .ok_or_else(|| Error::Engine(format!("invalid {key}: missing coordinate")))?;
            *coordinate = value.as_float().map_err(|e| Error::Engine(e.to_string()))?;
        }
        let [x0, y0, x1, y1] = coordinates;
        Ok(Some(PageRect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
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
        let bounds = self.page_geometry(page)?.visible_bounds;
        Ok((bounds.width(), bounds.height()))
    }

    fn page_geometry(&self, page: u32) -> Result<PageGeometry> {
        if let Some(geometry) = self.cached_page_geometry(page) {
            return Ok(geometry);
        }

        let page_obj = self.load_page(page)?;
        let bounds = page_obj
            .bounds()
            .map_err(|e| Error::Engine(e.to_string()))?;
        let pdf_page =
            mupdf::pdf::PdfPage::try_from(page_obj).map_err(|e| Error::Engine(e.to_string()))?;
        let media = Self::pdf_box(&pdf_page, "MediaBox")?
            .ok_or_else(|| Error::Engine("page has no valid MediaBox".to_string()))?;
        let crop = Self::pdf_box(&pdf_page, "CropBox")?.unwrap_or(media);
        let ctm = pdf_page.ctm().map_err(|e| Error::Engine(e.to_string()))?;
        let geometry = PageGeometry {
            visible_bounds: PageRect {
                x0: bounds.x0,
                y0: bounds.y0,
                x1: bounds.x1,
                y1: bounds.y1,
            },
            media_box: PageRect {
                x0: media.x0,
                y0: media.y0,
                x1: media.x1,
                y1: media.y1,
            },
            crop_box: crop,
            rotation: pdf_page
                .rotation()
                .map_err(|e| Error::Engine(e.to_string()))?,
            pdf_to_page: PageTransform {
                a: ctm.a,
                b: ctm.b,
                c: ctm.c,
                d: ctm.d,
                e: ctm.e,
                f: ctm.f,
            },
        };
        self.cache_page_geometry(page, geometry);
        Ok(geometry)
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap> {
        // F3.3: rasterize from the page's display list. The list is built
        // once per page (the expensive parse + command-tree walk) and every
        // later zoom only replays it into a fresh pixmap through
        // `fz_run_display_list` — 1.6-1.8× faster at pinch scale 2×
        // (`docs/benchmark-results.md`, 2026-08-30). First render of a page
        // pays the list build; subsequent renders at any scale reuse it.
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

#[cfg(test)]
mod geometry_cache_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_PDF: AtomicU64 = AtomicU64::new(0);

    struct TempPdf(std::path::PathBuf);

    impl Drop for TempPdf {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn multi_page_pdf(page_count: u32) -> TempPdf {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
                (0..page_count)
                    .map(|page| format!("{} 0 R", page + 3))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ];
        for page in 0..page_count {
            let stream_id = page_count + page + 3;
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << >> /Contents {stream_id} 0 R >>"
            ));
        }
        for _ in 0..page_count {
            objects.push("<< /Length 0 >>\nstream\n\nendstream".to_string());
        }

        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );

        let path = std::env::temp_dir().join(format!(
            "pdflector-geometry-cache-{}-{}.pdf",
            std::process::id(),
            NEXT_TEMP_PDF.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, pdf).unwrap();
        TempPdf(path)
    }

    #[test]
    fn geometry_metadata_cache_stays_within_five_pages() {
        let path = multi_page_pdf(8);
        let doc = MupdfEngine::new().unwrap().open(&path.0).unwrap();

        for page in 0..doc.page_count() {
            doc.page_geometry(page).unwrap();
        }

        assert!(
            doc.page_geometries.borrow().len() <= 5,
            "metadata cache retained {} pages after visiting 8",
            doc.page_geometries.borrow().len()
        );
    }

    #[test]
    fn geometry_metadata_cache_promotes_hits_before_evicting() {
        let path = multi_page_pdf(7);
        let doc = MupdfEngine::new().unwrap().open(&path.0).unwrap();

        for page in 0..5 {
            doc.page_geometry(page).unwrap();
        }
        doc.page_geometry(0).unwrap();
        doc.page_geometry(5).unwrap();

        let cached_pages = doc
            .page_geometries
            .borrow()
            .iter()
            .map(|(page, _)| *page)
            .collect::<Vec<_>>();
        assert_eq!(cached_pages, vec![5, 0, 4, 3, 2]);
    }
}
