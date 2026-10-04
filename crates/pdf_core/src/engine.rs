// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Rendering engine abstraction (docs/plan/NEXT-PLAN.md; AGENTS.md, MUST 2). The single backend
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

/// A line of extracted text with its bounding box, in displayed page points.
///
/// One span per structured-text line of the page (MuPDF stext line): the
/// natural granularity for Fase 3 highlight-by-selection, which paints a
/// rectangle per line underneath the text. Coordinates share the visible
/// page coordinate space of `Document::page_size` (origin top-left, y grows
/// downward); they are not the source PDF user-space coordinates when a page
/// has a shifted CropBox or rotation.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The extracted text of one page (docs/plan/NEXT-PLAN.md, base for Fases B y D).
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

/// Rectangle in a page coordinate system, with exclusive right and bottom
/// edges. `PageGeometry` documents the frame used by each rectangle field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl PageRect {
    pub fn width(self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(self) -> f32 {
        self.y1 - self.y0
    }
}

/// Affine transform from PDF user space to the page's visible, top-left
/// coordinate space. The six values follow `x' = a*x + c*y + e` and
/// `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl PageTransform {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn transform_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    pub fn inverse(self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() <= f32::EPSILON {
            return None;
        }
        let inv_det = det.recip();
        let a = self.d * inv_det;
        let b = -self.b * inv_det;
        let c = -self.c * inv_det;
        let d = self.a * inv_det;
        Some(Self {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }
}

/// Geometry metadata for one page, expressed in MuPDF's displayed page
/// coordinates (top-left origin, y down), plus the source PDF boxes and
/// transform needed to map PDF user-space points into that page space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageGeometry {
    pub visible_bounds: PageRect,
    pub media_box: PageRect,
    pub crop_box: PageRect,
    pub rotation: i32,
    pub pdf_to_page: PageTransform,
}

impl PageGeometry {
    pub fn from_page_size(width: f32, height: f32) -> Self {
        let visible_bounds = PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: width,
            y1: height,
        };
        Self {
            visible_bounds,
            media_box: visible_bounds,
            crop_box: visible_bounds,
            rotation: 0,
            pdf_to_page: PageTransform::IDENTITY,
        }
    }
}

/// An opened PDF document.
pub trait Document {
    fn page_count(&self) -> u32;
    /// Page size in PDF points (1/72 inch), as (width, height).
    fn page_size(&self, page: u32) -> Result<(f32, f32)>;
    /// Visible page geometry. Implementations that only expose a size retain
    /// the historical origin-at-zero behavior; PDF engines should return the
    /// visible box, source boxes, rotation and PDF-to-page transform.
    fn page_geometry(&self, page: u32) -> Result<PageGeometry> {
        let (width, height) = self.page_size(page)?;
        Ok(PageGeometry::from_page_size(width, height))
    }
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

#[cfg(test)]
mod geometry_tests {
    use super::PageTransform;

    #[test]
    fn page_transform_and_inverse_round_trip_points() {
        let transform = PageTransform {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
            e: 340.0,
            f: -40.0,
        };
        let inverse = transform.inverse().unwrap();
        let original = (80.0, 120.0);
        let page = transform.transform_point(original.0, original.1);
        let round_trip = inverse.transform_point(page.0, page.1);
        assert!((round_trip.0 - original.0).abs() <= 0.001);
        assert!((round_trip.1 - original.1).abs() <= 0.001);
    }
}
