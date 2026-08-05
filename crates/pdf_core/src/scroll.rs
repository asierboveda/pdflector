//! Virtualized-scroll math (docs/PLAN.md §4, Fase 1 — "solo páginas visibles +
//! N colindantes"). Pure geometry here, so it is testable without a PDF engine;
//! the threaded prefetch queue belongs to Fase 1 B2.

use crate::cache::RenderCache;
use crate::engine::{RenderEngine, Result};

/// Window of pages currently visible in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub first_visible_page: usize,
    pub visible_count: usize,
}

/// The range of pages that should be rendered for a viewport: the visible
/// window plus `prefetch_radius` pages on each side, clamped to `[0, total)`.
///
/// Pure function — no engine involved, so it is fully testable.
pub fn visible_and_prefetch_pages(
    vp: &Viewport,
    total_pages: usize,
    prefetch_radius: usize,
) -> std::ops::Range<usize> {
    let start = vp
        .first_visible_page
        .saturating_sub(prefetch_radius)
        .min(total_pages);
    let end = (vp.first_visible_page + vp.visible_count + prefetch_radius).min(total_pages);
    start..end
}

/// Renders the visible window plus `prefetch_radius` neighbours through the
/// cache (synchronous, no threads — that is B2). Honors the cache byte budget.
pub fn populate_visible<E: RenderEngine>(
    cache: &mut RenderCache<E>,
    vp: &Viewport,
    prefetch_radius: usize,
) -> Result<()> {
    cache.ensure_visible(vp, prefetch_radius)
}
