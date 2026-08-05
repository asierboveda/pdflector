//! Rendering engine abstraction (docs/PLAN.md §3.2). The single backend
//! (MuPDF, chosen in ADR-001) implements these traits, so callers never
//! depend on the concrete engine.

use std::path::Path;

pub mod mupdf;

/// RGBA8 bitmap of a rendered page, row-major, `data.len() == width * height * 4`.
#[derive(Debug)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// An opened PDF document.
pub trait Document {
    fn page_count(&self) -> u32;
    /// Page size in PDF points (1/72 inch), as (width, height).
    fn page_size(&self, page: u32) -> Result<(f32, f32)>;
    /// Renders `page` (0-based) at `scale` (1.0 = 72 dpi) into an RGBA bitmap.
    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap>;
}

/// A PDF rendering backend.
pub trait RenderEngine {
    type Document: Document;
    fn open(&self, path: &Path) -> Result<Self::Document>;
}

#[derive(Debug)]
pub enum Error {
    /// Engine used before binding/initializing its native library.
    NotInitialized,
    Io(std::io::Error),
    PageOutOfRange {
        page: u32,
        page_count: u32,
    },
    /// Error reported by the underlying PDF engine.
    Engine(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotInitialized => write!(f, "engine not initialized"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::PageOutOfRange { page, page_count } => {
                write!(
                    f,
                    "page {page} out of range (document has {page_count} pages)"
                )
            }
            Error::Engine(msg) => write!(f, "engine error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
