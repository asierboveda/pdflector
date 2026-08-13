// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_core — PDFLector's core library: document handling, rendering, cache,
//! annotations, persistence and export. UI-independent by design (see
//! docs/PLAN.md §3): no egui, no windowing, compiles headless.

pub mod ai;
pub mod annotations;
pub mod cache;
pub mod dark;
pub mod engine;
pub mod export;
pub mod metrics;
pub mod prefetch;
pub mod scroll;
pub mod store;
pub mod sync;
pub mod zoom;

pub use ai::{AiError, OllamaClient, chunk_pages};
pub use annotations::{
    Annotated, Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, TextNote,
};
pub use cache::{CacheStats, PageKey, RenderCache, RenderedPage, scale_for_level};
pub use dark::invert_bitmap;
pub use engine::{Bitmap, Document, Error, PageText, RenderEngine, Result, TextSpan};
pub use export::{export_markdown, export_markdown_to_file, export_pdf_annotated};
pub use metrics::{FrameTimer, read_rss_kb};
pub use scroll::{Viewport, visible_and_prefetch_pages};
pub use store::{AnnotationStore, StoreError, sidecar_path};
pub use sync::{
    AnnotationWatcher, SyncError, annotations_dir, library_index_path, watch_annotations,
};
pub use zoom::{scale_bitmap, scale_level_for_zoom};

/// Corpus directory. `PDFLECTOR_CORPUS_DIR` wins when set (e.g. on device);
/// otherwise falls back to the workspace-relative corpus folder (desktop).
/// Same scheme as pdf_bench's `corpus_dir`.
pub fn corpus_dir() -> std::path::PathBuf {
    match std::env::var("PDFLECTOR_CORPUS_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus"),
    }
}
