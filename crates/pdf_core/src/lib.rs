//! pdf_core — PDFLector's core library: document handling, rendering, cache,
//! annotations, persistence and export. UI-independent by design (see
//! docs/PLAN.md §3): no egui, no windowing, compiles headless.

pub mod engine;

pub use engine::{Bitmap, Document, Error, RenderEngine, Result};
