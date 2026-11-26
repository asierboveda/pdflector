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
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;

use mupdf::{Colorspace, Matrix, TextBlockContent, TextPageFlags};

use crate::engine::display_list_cache::{DisplayListBudget, DisplayListCache, DisplayListStats};
use crate::engine::{
    Bitmap, Document, Error, PageGeometry, PageRect, PageText, PageTransform, RenderEngine, Result,
    TextSpan,
};

pub struct MupdfEngine;

const PAGE_GEOMETRY_CACHE_CAPACITY: usize = 5;

/// Matches MuPDF's `base_font_names` table (`source/pdf/pdf-font.c`), ignoring
/// ASCII spaces and case just like `pdf_clean_font_name` / `strcmp_ignore_space`.
#[cfg(any(target_os = "android", test))]
fn is_mupdf_base14_font(name: &str) -> bool {
    const BASE14_ALIASES: &[&str] = &[
        // Courier
        "Courier",
        "CourierNew",
        "CourierNewPSMT",
        // Courier-Bold
        "Courier-Bold",
        "CourierNew,Bold",
        "Courier,Bold",
        "CourierNewPS-BoldMT",
        "CourierNew-Bold",
        // Courier-Oblique
        "Courier-Oblique",
        "CourierNew,Italic",
        "Courier,Italic",
        "CourierNewPS-ItalicMT",
        "CourierNew-Italic",
        // Courier-BoldOblique
        "Courier-BoldOblique",
        "CourierNew,BoldItalic",
        "Courier,BoldItalic",
        "CourierNewPS-BoldItalicMT",
        "CourierNew-BoldItalic",
        // Helvetica
        "Helvetica",
        "ArialMT",
        "Arial",
        // Helvetica-Bold
        "Helvetica-Bold",
        "Arial-BoldMT",
        "Arial,Bold",
        "Arial-Bold",
        "Helvetica,Bold",
        // Helvetica-Oblique
        "Helvetica-Oblique",
        "Arial-ItalicMT",
        "Arial,Italic",
        "Arial-Italic",
        "Helvetica,Italic",
        "Helvetica-Italic",
        // Helvetica-BoldOblique
        "Helvetica-BoldOblique",
        "Arial-BoldItalicMT",
        "Arial,BoldItalic",
        "Arial-BoldItalic",
        "Helvetica,BoldItalic",
        "Helvetica-BoldItalic",
        // Times-Roman
        "Times-Roman",
        "TimesNewRomanPSMT",
        "TimesNewRoman",
        "TimesNewRomanPS",
        // Times-Bold
        "Times-Bold",
        "TimesNewRomanPS-BoldMT",
        "TimesNewRoman,Bold",
        "TimesNewRomanPS-Bold",
        "TimesNewRoman-Bold",
        // Times-Italic
        "Times-Italic",
        "TimesNewRomanPS-ItalicMT",
        "TimesNewRoman,Italic",
        "TimesNewRomanPS-Italic",
        "TimesNewRoman-Italic",
        // Times-BoldItalic
        "Times-BoldItalic",
        "TimesNewRomanPS-BoldItalicMT",
        "TimesNewRoman,BoldItalic",
        "TimesNewRomanPS-BoldItalic",
        "TimesNewRoman-BoldItalic",
        // Symbol
        "Symbol",
        "Symbol,Italic",
        "Symbol,Bold",
        "Symbol,BoldItalic",
        "SymbolMT",
        "SymbolMT,Italic",
        "SymbolMT,Bold",
        "SymbolMT,BoldItalic",
        // ZapfDingbats
        "ZapfDingbats",
    ];
    BASE14_ALIASES.iter().any(|alias| {
        let mut a = name.bytes().filter(|&b| b != b' ');
        let mut b = alias.bytes().filter(|&b| b != b' ');
        loop {
            match (a.next(), b.next()) {
                (Some(x), Some(y)) if x.eq_ignore_ascii_case(&y) => continue,
                (None, None) => return true,
                _ => return false,
            }
        }
    })
}

/// Keep PDF's standard fonts (and all MuPDF Base14 aliases) on MuPDF's bundled
/// Base14 path. Android's generic Noto substitutes do not preserve custom
/// Type 1 encodings/metrics.
#[cfg(target_os = "android")]
struct PdfAndroidFontLoader;

#[cfg(target_os = "android")]
impl mupdf::FontLoader for PdfAndroidFontLoader {
    fn load_font(&self, name: &str, hints: mupdf::FontHints) -> Option<mupdf::Font> {
        if is_mupdf_base14_font(name) {
            return None;
        }
        mupdf::FontLoader::load_font(&mupdf::AndroidFontLoader, name, hints)
    }

    fn load_cjk_font(
        &self,
        name: &str,
        ordering: mupdf::CjkFontOrdering,
        serif: bool,
    ) -> Option<mupdf::Font> {
        mupdf::FontLoader::load_cjk_font(&mupdf::AndroidFontLoader, name, ordering, serif)
    }

    fn load_fallback_font(
        &self,
        script: u32,
        language: u32,
        hints: mupdf::FontHints,
    ) -> Option<mupdf::Font> {
        mupdf::FontLoader::load_fallback_font(&mupdf::AndroidFontLoader, script, language, hints)
    }
}

fn initialize_font_loader() {
    #[cfg(target_os = "android")]
    {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| mupdf::set_font_loader(PdfAndroidFontLoader));
    }
}

impl MupdfEngine {
    /// MuPDF links statically and bootstraps its context on first use, so
    /// there is nothing to bind. `new` exists to mirror the engine creation
    /// pattern (the single place where init errors surface) and to force the
    /// one-time context initialization early.
    pub fn new() -> Result<Self> {
        initialize_font_loader();
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

impl MupdfEngine {
    /// Opens `path` with an explicit display-list budget (PR-03). `open`
    /// uses `DisplayListBudget::TABLET_DEFAULT`; tests pass tiny budgets.
    pub fn open_with_display_list_budget(
        &self,
        path: &Path,
        budget: DisplayListBudget,
    ) -> Result<MupdfDocument> {
        initialize_font_loader();
        let doc = mupdf::Document::open(path)
            .map_err(|e| Error::Engine(format!("open {}: {e}", path.display())))?;
        Ok(MupdfDocument {
            inner: doc,
            display_lists: RefCell::new(DisplayListCache::new(budget)),
            page_geometries: RefCell::new(VecDeque::new()),
        })
    }
}

impl RenderEngine for MupdfEngine {
    type Document = MupdfDocument;

    fn open(&self, path: &Path) -> Result<Self::Document> {
        self.open_with_display_list_budget(path, DisplayListBudget::TABLET_DEFAULT)
    }
}

pub struct MupdfDocument {
    inner: mupdf::Document,
    /// Per-page display lists (F3.3), built lazily on first render of the
    /// page and reused for every subsequent scale change, bounded by a
    /// `DisplayListBudget` with LRU eviction (PR-03): running
    /// `fz_run_display_list` skips the PDF object parse + command-tree walk
    /// that `Page::to_pixmap` pays on every zoom (2-4× faster, F0 spike).
    /// `RefCell` (not `Mutex`): `&mut self` interior mutability over a
    /// shared `&self` — the render paths are single-threaded per document
    /// instance (the worker owns its document), so there is no concurrent
    /// access to alias. This relies on `MupdfDocument` NOT being `Sync`
    /// (mupdf-rs types are plain FFI pointers, no auto traits), which the
    /// compiler enforces if a future caller shares it across threads.
    display_lists: RefCell<DisplayListCache<mupdf::DisplayList>>,
    /// Tiny immutable page metadata is retained after first access so frame
    /// and gesture paths never ask MuPDF to recalculate the page transform.
    page_geometries: RefCell<VecDeque<(u32, PageGeometry)>>,
}

/// Fixed cost charged to every display list: the `fz_display_list` header,
/// the slack of its doubling node array and the page-level gstate/clip nodes
/// even an empty page records.
const DISPLAY_LIST_BASE_BYTES: usize = 16 * 1024;

/// Display-list bytes charged per byte of (usually Flate-compressed) content
/// stream. Content streams compress ~3-5x and MuPDF records roughly one node
/// (header + packed floats / path data) per drawing operator, i.e. about the
/// size of the decoded operator text: 4x the stored length.
const CONTENT_STREAM_FACTOR: usize = 4;

/// `/Length` of a stream object (0 when missing or malformed).
fn stream_length(stream: &mupdf::pdf::PdfObject) -> usize {
    stream
        .get_dict("Length")
        .ok()
        .flatten()
        .and_then(|length| length.as_int().ok())
        .map_or(0, |length| length.max(0) as usize)
}

/// Estimated resident size of the display list built from `page`, in bytes.
///
/// MuPDF does not expose a display list's size (`fz_display_list` is opaque
/// in mupdf-sys and has no size query), so this is a cheap proxy read from
/// the page dictionary — no stream is decoded:
/// - `DISPLAY_LIST_BASE_BYTES` per list;
/// - `CONTENT_STREAM_FACTOR` × the stored length of the page's content
///   streams and of its Form XObjects (their operators become nodes);
/// - 1 × the stored length of its Image XObjects: the list keeps a reference
///   to each `fz_image`, which pins the compressed image data in memory
///   (decoded tiles live in MuPDF's own store, not in the list).
///
/// Shared resources are charged to every page that uses them, so the
/// estimate errs high. Calibrate against PSS on the device (see the
/// `dlcache` log line of the render worker).
fn estimate_display_list_bytes(page: &mupdf::Page) -> usize {
    let mut bytes = DISPLAY_LIST_BASE_BYTES;
    let Ok(pdf_page) = mupdf::pdf::PdfPage::try_from(page.clone()) else {
        return bytes; // not a PDF page: only the fixed cost is known
    };
    let object = pdf_page.object();

    if let Ok(Some(contents)) = object.get_dict("Contents") {
        if contents.is_array().unwrap_or(false) {
            let count = contents.len().unwrap_or(0);
            for index in 0..count {
                if let Ok(Some(stream)) = contents.get_array(index as i32) {
                    bytes += CONTENT_STREAM_FACTOR * stream_length(&stream);
                }
            }
        } else {
            bytes += CONTENT_STREAM_FACTOR * stream_length(&contents);
        }
    }

    let xobjects = object
        .get_dict_inheritable("Resources")
        .ok()
        .flatten()
        .and_then(|resources| resources.get_dict("XObject").ok().flatten());
    if let Some(xobjects) = xobjects {
        for index in 0..xobjects.dict_len().unwrap_or(0) {
            let Ok(Some(xobject)) = xobjects.get_dict_val(index as i32) else {
                continue;
            };
            let subtype = xobject
                .get_dict("Subtype")
                .ok()
                .flatten()
                .and_then(|name| name.as_name().ok());
            let length = stream_length(&xobject);
            bytes += match subtype.as_deref() {
                Some(b"Image") => length,
                _ => CONTENT_STREAM_FACTOR * length,
            };
        }
    }
    bytes
}

impl MupdfDocument {
    /// Counters of this document's display-list cache (entries, estimated
    /// bytes, peak, hits, misses, evictions). `Display` formats them as one
    /// `dlcache ...` log line.
    pub fn display_list_stats(&self) -> DisplayListStats {
        self.display_lists.borrow().stats()
    }

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
    /// the budgeted LRU `display_lists` (PR-03): the returned `Rc` keeps the
    /// list alive — and unevictable — while the caller renders from it, and
    /// the `RefCell` borrow ends before the render starts.
    fn display_list_for(&self, page: u32) -> Result<Rc<mupdf::DisplayList>> {
        self.display_lists.borrow_mut().get_or_build(page, || {
            let page_obj = self.load_page(page)?;
            let list = page_obj
                .to_display_list(true)
                .map_err(|e| Error::Engine(e.to_string()))?;
            Ok((list, estimate_display_list_bytes(&page_obj)))
        })
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

    fn tiny_budget(max_entries: usize, max_bytes: usize) -> DisplayListBudget {
        DisplayListBudget {
            max_entries,
            max_bytes,
        }
    }

    #[test]
    fn display_lists_stay_within_entry_budget_over_a_long_document() {
        let path = multi_page_pdf(12);
        let doc = MupdfEngine::new()
            .unwrap()
            .open_with_display_list_budget(&path.0, tiny_budget(3, usize::MAX))
            .unwrap();

        for page in 0..doc.page_count() {
            doc.render_page(page, 1.0).unwrap();
            assert!(doc.display_list_stats().entries <= 3);
        }

        let s = doc.display_list_stats();
        assert_eq!(s.entries, 3, "{s}");
        assert_eq!(s.misses, 12, "{s}");
        assert_eq!(s.evictions, 9, "{s}");
        assert!(s.bytes > 0, "{s}");
        assert!(s.peak_bytes >= s.bytes, "{s}");
    }

    #[test]
    fn display_list_is_reused_across_zoom_levels() {
        let path = multi_page_pdf(2);
        let doc = MupdfEngine::new().unwrap().open(&path.0).unwrap();

        doc.render_page(0, 1.0).unwrap();
        doc.render_page(0, 2.0).unwrap();

        let s = doc.display_list_stats();
        assert_eq!((s.misses, s.hits, s.entries), (1, 1, 1), "{s}");
    }

    #[test]
    fn disabled_display_list_budget_renders_identically_and_retains_nothing() {
        let path = single_page_text_pdf(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
            "Hello",
        );
        let engine = MupdfEngine::new().unwrap();
        let cached = engine.open(&path.0).unwrap();
        let uncached = engine
            .open_with_display_list_budget(&path.0, tiny_budget(0, 0))
            .unwrap();

        let a = cached.render_page(0, 1.5).unwrap();
        let b = uncached.render_page(0, 1.5).unwrap();
        assert_eq!(a.data, b.data);
        uncached.render_page(0, 1.5).unwrap();

        let s = uncached.display_list_stats();
        assert_eq!((s.entries, s.bytes, s.misses), (0, 0, 2), "{s}");
        assert_eq!(cached.display_list_stats().entries, 1);
    }

    #[test]
    fn display_list_estimate_grows_with_page_content() {
        let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
        let short = single_page_text_pdf(font, "A");
        let long = single_page_text_pdf(font, &"Lorem ipsum dolor sit amet ".repeat(200));
        let engine = MupdfEngine::new().unwrap();
        let short_doc = engine.open(&short.0).unwrap();
        let long_doc = engine.open(&long.0).unwrap();

        let short_bytes = estimate_display_list_bytes(&short_doc.load_page(0).unwrap());
        let long_bytes = estimate_display_list_bytes(&long_doc.load_page(0).unwrap());
        assert!(short_bytes > 0);
        assert!(
            long_bytes > short_bytes,
            "long {long_bytes} <= short {short_bytes}"
        );
    }

    fn single_page_text_pdf(font_dict: &str, text: &str) -> TempPdf {
        let stream = format!("BT /F1 24 Tf 36 100 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
                .to_string(),
            font_dict.to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        ];

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
            "pdflector-base14-{}-{}.pdf",
            std::process::id(),
            NEXT_TEMP_PDF.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, pdf).unwrap();
        TempPdf(path)
    }

    #[test]
    fn base14_alias_matcher_covers_mupdf_aliases_and_spaces() {
        for name in [
            "Times-Roman",
            "Helvetica-Bold",
            "Courier-Oblique",
            "Symbol",
            "ZapfDingbats",
            "TimesNewRomanPSMT",
            "TimesNewRomanPS-BoldMT",
            "TimesNewRoman,Italic",
            "Times New Roman",
            "Times New Roman, Bold",
            "ArialMT",
            "Arial-BoldMT",
            "Arial,BoldItalic",
            "CourierNewPSMT",
            "Courier New",
            "CourierNew,Bold",
            "SymbolMT",
            "SymbolMT,BoldItalic",
            "timesnewromanpsmt",
            "arialmt",
        ] {
            assert!(
                super::is_mupdf_base14_font(name),
                "expected Base14 alias match for {name:?}"
            );
        }
        for non_base14 in [
            "",
            "Roboto",
            "NotoSans-Regular",
            "LiberationSerif",
            "TimesNewRomanExtra",
            "ArialUnicodeMS",
        ] {
            assert!(
                !super::is_mupdf_base14_font(non_base14),
                "did not expect Base14 match for {non_base14:?}"
            );
        }
    }

    #[test]
    fn base14_alias_custom_encoding_preserves_glyphs_and_text() {
        let aliased = single_page_text_pdf(
            "<< /Type /Font /Subtype /Type1 /BaseFont /TimesNewRomanPSMT /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >> >>",
            "A",
        );
        let reference = single_page_text_pdf(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
            "B",
        );

        let engine = MupdfEngine::new().unwrap();
        let aliased_doc = engine.open(&aliased.0).unwrap();
        let reference_doc = engine.open(&reference.0).unwrap();

        let extracted: String = aliased_doc
            .text(0)
            .unwrap()
            .spans
            .into_iter()
            .map(|span| span.text)
            .collect();
        assert_eq!(extracted.trim(), "B");

        let aliased_bmp = aliased_doc.render_page(0, 1.0).unwrap();
        let reference_bmp = reference_doc.render_page(0, 1.0).unwrap();
        assert_eq!(aliased_bmp.data, reference_bmp.data);
    }
}
