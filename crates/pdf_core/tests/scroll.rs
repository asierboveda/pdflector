//! Pure scroll-math tests (Fase 1, B1): `visible_and_prefetch_pages` needs no
//! engine, so every case here is a pure assertion on the returned Range.

use pdf_core::{Viewport, visible_and_prefetch_pages};

/// A viewport at page 0 clamps the start to 0 (no negative prefetch).
#[test]
fn start_clamps_to_zero_near_beginning() {
    let vp = Viewport {
        first_visible_page: 0,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 2), 0..5);
}

/// A viewport in the middle includes `prefetch_radius` neighbours each side.
#[test]
fn middle_viewport_includes_neighbours_on_both_sides() {
    let vp = Viewport {
        first_visible_page: 100,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 2), 98..105);
}

/// A viewport at the last page clamps the end to `total` (no overflow).
#[test]
fn end_clamps_to_total_at_last_page() {
    let vp = Viewport {
        first_visible_page: 497,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 2), 495..500);
}

/// A prefetch radius larger than the whole document covers everything.
#[test]
fn huge_prefetch_radius_covers_whole_document() {
    let vp = Viewport {
        first_visible_page: 0,
        visible_count: 1,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 10_000), 0..500);
}

/// An empty document yields an empty range.
#[test]
fn empty_document_yields_empty_range() {
    let vp = Viewport {
        first_visible_page: 0,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 0, 2), 0..0);
}

/// A viewport beyond the end of the document yields an empty range.
#[test]
fn viewport_beyond_end_yields_empty_range() {
    let vp = Viewport {
        first_visible_page: 600,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 2), 500..500);
}

/// A degenerate viewport with page offsets near `usize::MAX` must not
/// overflow: `first + count + radius` would panic in debug builds (and wrap
/// in release) with plain `+`. The result is the clamped empty range.
#[test]
fn degenerate_huge_viewport_does_not_overflow() {
    let vp = Viewport {
        first_visible_page: usize::MAX - 1,
        visible_count: 3,
    };
    assert_eq!(visible_and_prefetch_pages(&vp, 500, 2), 500..500);
}
