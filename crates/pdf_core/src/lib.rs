//! pdf_core — PDFLector's core library: document handling, rendering, cache,
//! annotations, persistence and export. UI-independent by design (see
//! docs/PLAN.md §3): no egui, no windowing, compiles headless.

pub mod cache;
pub mod engine;
pub mod scroll;

pub use cache::{CacheStats, PageKey, RenderCache, RenderedPage, scale_for_level};
pub use engine::{Bitmap, Document, Error, RenderEngine, Result};
pub use scroll::{Viewport, visible_and_prefetch_pages};

/// Corpus directory. `PDFLECTOR_CORPUS_DIR` wins when set (e.g. on device);
/// otherwise falls back to the workspace-relative corpus folder (desktop).
/// Same scheme as pdf_bench's `corpus_dir`.
pub fn corpus_dir() -> std::path::PathBuf {
    match std::env::var("PDFLECTOR_CORPUS_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus"),
    }
}
