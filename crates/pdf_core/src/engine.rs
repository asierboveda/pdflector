// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Rendering engine abstraction (docs/PLAN.md §3.2). The single backend
//! (MuPDF, chosen in ADR-001) implements these traits, so callers never
//! depend on the concrete engine.

use std::path::Path;

pub mod mupdf;

/// RGBA8 bitmap of a rendered page, row-major, `data.len() == width * height * 4`.
///
/// `Clone` is derived (deep copy of `data`) so a worker-owned cache can hand
/// out copies of resident pages across the channel without transferring
/// ownership — see `Prefetcher::get_page`.
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A line of extracted text with its bounding box, in PDF points.
///
/// One span per structured-text line of the page (MuPDF stext line): the
/// natural granularity for Fase 3 highlight-by-selection, which paints a
/// rectangle per line underneath the text. Coordinates share the page
/// coordinate space of `Document::page_size` (origin top-left, y grows
/// downward).
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The extracted text of one page (docs/PLAN.md §3.2).
///
/// Lives next to the `Document` trait (like `Bitmap`) because it is part of
/// the engine abstraction's data contract: `Document::text` returns it and
/// callers must not depend on the concrete engine. Kept in `engine.rs` so no
/// `lib.rs` wiring is needed for the module; if it grows (words, search hits)
/// in Fase 3/5 it can move to its own `src/text.rs` with a trivial move.
#[derive(Debug, Clone, PartialEq)]
pub struct PageText {
    /// Plain text of the page, in reading order.
    pub text: String,
    /// Per-line spans with bounding boxes (page coordinates), for
    /// highlight-by-selection. Empty if the page has no extractable text
    /// (e.g. a pure image scan).
    pub spans: Vec<TextSpan>,
}

/// An opened PDF document.
pub trait Document {
    fn page_count(&self) -> u32;
    /// Page size in PDF points (1/72 inch), as (width, height).
    fn page_size(&self, page: u32) -> Result<(f32, f32)>;
    /// Renders `page` (0-based) at `scale` (1.0 = 72 dpi) into an RGBA bitmap.
    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap>;
    /// Extracts the text of `page` (0-based), lazily: only called when
    /// needed (selection/highlight, search, chunking) — never during
    /// render/scroll, which stay on the bitmap path.
    fn text(&self, page: u32) -> Result<PageText>;
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
    /// Invalid arguments to a pdf_core API (e.g. a malformed bitmap passed to
    /// `zoom::scale_bitmap`).
    InvalidArgument(String),
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
            Error::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
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
