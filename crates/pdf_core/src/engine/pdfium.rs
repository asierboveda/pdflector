//! PDFium backend: crate `pdfium-render` over the prebuilt `libpdfium.so`
//! fetched by tools/fetch_pdfium.sh (see ADR-001 for the engine decision).

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::{PdfBitmapFormat, Pdfium};

use crate::engine::{Bitmap, Document, Error, RenderEngine, Result};

// Pdfium documents borrow the library handle, so owning both engine and
// documents would need self-referential structs. Binding the library once
// into a static sidesteps that; sharing the handle is safe.
static PDFIUM: OnceLock<std::result::Result<Pdfium, String>> = OnceLock::new();

// Pdfium's native library is single-threaded. The crate's `thread_safe` feature
// only wraps FPDF_InitLibrary in a process-wide mutex (taken forever); every
// other native call is forwarded without locking, so concurrent renders from
// several threads segfault. We serialize all native access ourselves.
static PDFIUM_LOCK: Mutex<()> = Mutex::new(());

pub struct PdfiumEngine;

impl PdfiumEngine {
    /// Binds to `libpdfium.so` at `lib_path`. Idempotent: later calls are no-ops.
    /// `get_or_init` serializes the one-time init: with the crate's `sync`
    /// feature the first `FPDF_InitLibrary()` takes a process-wide mutex that
    /// it never releases, so racing callers would deadlock (see pdfium.rs in
    /// the crate). Only one thread ever runs the closure; the rest wait on it.
    pub fn new(lib_path: &Path) -> Result<Self> {
        let result = PDFIUM.get_or_init(|| {
            Pdfium::bind_to_library(lib_path)
                .map(Pdfium::new)
                .map_err(|e| format!("bind {}: {e}", lib_path.display()))
        });
        match result {
            Ok(_) => Ok(Self),
            Err(e) => Err(Error::Engine(e.clone())),
        }
    }
}

impl RenderEngine for PdfiumEngine {
    type Document = PdfiumDocument;

    fn open(&self, path: &Path) -> Result<PdfiumDocument> {
        let _guard = PDFIUM_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pdfium = PDFIUM
            .get()
            .ok_or(Error::NotInitialized)?
            .as_ref()
            .map_err(|e| Error::Engine(e.clone()))?;
        let inner = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| Error::Engine(format!("open {}: {e}", path.display())))?;
        Ok(PdfiumDocument { inner })
    }
}

pub struct PdfiumDocument {
    inner: pdfium_render::prelude::PdfDocument<'static>,
}

impl PdfiumDocument {
    /// Unlocked helper; callers must hold `PDFIUM_LOCK`.
    fn page_unlocked(&self, page: u32) -> Result<pdfium_render::prelude::PdfPage<'_>> {
        let page_count = self.page_count_unlocked();
        if page >= page_count {
            return Err(Error::PageOutOfRange { page, page_count });
        }
        self.inner
            .pages()
            .get(page as u16)
            .map_err(|e| Error::Engine(e.to_string()))
    }

    /// Unlocked helper; callers must hold `PDFIUM_LOCK`.
    fn page_count_unlocked(&self) -> u32 {
        self.inner.pages().len() as u32
    }
}

impl Document for PdfiumDocument {
    fn page_count(&self) -> u32 {
        let _guard = PDFIUM_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.page_count_unlocked()
    }

    fn page_size(&self, page: u32) -> Result<(f32, f32)> {
        let _guard = PDFIUM_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = self.page_unlocked(page)?;
        Ok((p.width().value, p.height().value))
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap> {
        let _guard = PDFIUM_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let p = self.page_unlocked(page)?;
        let width = (p.width().value * scale).round().max(1.0) as i32;
        let height = (p.height().value * scale).round().max(1.0) as i32;
        let bitmap = p
            .render(width, height, None)
            .map_err(|e| Error::Engine(e.to_string()))?;

        // Pdfium renders BGRA; convert to the RGBA the core exposes.
        let mut data = bitmap.as_raw_bytes();
        if matches!(
            bitmap.format(),
            Ok(PdfBitmapFormat::BGRA) | Ok(PdfBitmapFormat::BGRx)
        ) {
            for px in data.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }

        Ok(Bitmap {
            width: bitmap.width() as u32,
            height: bitmap.height() as u32,
            data,
        })
    }
}
