//! Real-engine cache tests (Fase 1, B1): byte-bounded LRU cache over MuPDF and
//! REAL corpus PDFs. No mocks: every miss is an actual MuPDF render and every
//! byte figure is measured from real bitmap dimensions.

use pdf_core::cache::{RenderCache, scale_for_level};
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::{Document, RenderEngine};

fn corpus(name: &str) -> std::path::PathBuf {
    pdf_core::corpus_dir().join(name)
}

fn open_doc(name: &str) -> MupdfDocument {
    let engine = MupdfEngine::new().expect("mupdf init");
    engine.open(&corpus(name)).expect("open corpus pdf")
}

/// Real footprint of `page` at scale level 0 (1.0, 72 dpi), in bytes.
fn page_bytes(doc: &MupdfDocument, page: u32) -> usize {
    let bmp = doc.render_page(page, scale_for_level(0)).expect("render");
    bmp.width as usize * bmp.height as usize * 4
}

fn open_cache(name: &str, budget: usize) -> RenderCache<MupdfEngine> {
    RenderCache::open(
        MupdfEngine::new().expect("mupdf init"),
        &corpus(name),
        budget,
    )
    .expect("open cache")
}

/// Hit/miss semantics: the first call renders (miss), the identical second call
/// is a hit and does NOT re-render (miss count stays put).
#[test]
fn first_call_misses_second_call_hits_without_rerender() {
    let mut cache = open_cache("dense_textbook.pdf", 64 * 1024 * 1024);

    let p = cache.get_or_render(0, 0).expect("render page 0");
    // byte_size is computed from real dimensions and matches the buffer.
    assert_eq!(
        p.byte_size,
        p.bitmap.width as usize * p.bitmap.height as usize * 4
    );
    assert_eq!(p.byte_size, p.bitmap.data.len());
    assert!(p.byte_size > 0);
    let first_bytes = p.byte_size;

    let s = cache.stats();
    assert_eq!(s.misses, 1);
    assert_eq!(s.hits, 0);
    assert_eq!(s.entries, 1);
    assert_eq!(s.current_bytes, first_bytes);

    // Identical call: a hit, and no second render happened.
    cache.get_or_render(0, 0).expect("render page 0 again");
    let s = cache.stats();
    assert_eq!(s.hits, 1);
    assert_eq!(s.misses, 1);
    assert_eq!(s.entries, 1);
    assert_eq!(s.current_bytes, first_bytes);
}

/// LRU eviction order: with a budget holding exactly two pages, rendering
/// p0, p1, p2 must evict the least recently used (p0). p1 stays resident (hit),
/// p0 is gone (miss).
#[test]
fn lru_eviction_evicts_least_recently_used_page() {
    let doc = open_doc("dense_textbook.pdf");

    // Measure the REAL footprints of pages 0, 1 and 2 at scale level 0.
    let s0 = page_bytes(&doc, 0);
    let s1 = page_bytes(&doc, 1);
    let s2 = page_bytes(&doc, 2);
    // Budget that holds any two of them but not all three: exactly one
    // eviction is forced by p2.
    let budget = (s0 + s1).max(s1 + s2);

    let mut cache = open_cache("dense_textbook.pdf", budget);

    for page in [0, 1, 2] {
        cache.get_or_render(page, 0).expect("render");
    }

    let s = cache.stats();
    assert_eq!(s.misses, 3);
    assert_eq!(s.evictions, 1, "exactly the LRU page must be evicted");
    assert!(s.current_bytes <= budget, "budget respected after eviction");

    // p1 was most recently used → resident → hit, no render.
    cache.get_or_render(1, 0).expect("p1 must still be cached");
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 3);

    // p0 was the evicted LRU entry → miss → re-render.
    cache.get_or_render(0, 0).expect("p0 re-render");
    let s = cache.stats();
    assert_eq!(s.misses, 4);
    assert_eq!(s.hits, 1);
}

/// Byte-budget enforcement: the invariant `current_bytes <= budget` holds at
/// every step while scrolling through 30 pages of large_document.pdf.
#[test]
fn byte_budget_invariant_holds_while_scrolling_a_large_document() {
    const BUDGET: usize = 4 * 1024 * 1024;
    let mut cache = open_cache("large_document.pdf", BUDGET);

    for page in 0..30 {
        cache.get_or_render(page, 0).expect("render");
        let s = cache.stats();
        assert!(
            s.current_bytes <= BUDGET,
            "invariant broken at page {page}: {} > {BUDGET}",
            s.current_bytes
        );
    }

    let s = cache.stats();
    assert_eq!(s.misses, 30, "every page rendered exactly once");
    // Corpus pages are ~2 MB at 72 dpi, so 4 MB holds two; every page after
    // the first two forces one eviction.
    assert!(
        s.evictions >= 25,
        "expected ~28 evictions with a 4 MB budget, got {}",
        s.evictions
    );
    assert!(s.entries >= 1, "resident entries survive");
}

/// `clear` drops every resident page and forces a re-render on the next call.
#[test]
fn clear_resets_resident_state_and_forces_rerender() {
    let mut cache = open_cache("dense_textbook.pdf", 64 * 1024 * 1024);

    cache.get_or_render(0, 0).expect("render");
    assert_eq!(cache.stats().entries, 1);

    cache.clear();
    let s = cache.stats();
    assert_eq!(s.entries, 0);
    assert_eq!(s.current_bytes, 0);

    // Same page afterwards is a fresh miss (counters keep their history, so
    // misses goes 1 -> 2): the bitmap really was dropped from the map.
    cache.get_or_render(0, 0).expect("re-render");
    let s = cache.stats();
    assert_eq!(s.misses, 2);
    assert_eq!(s.hits, 0);
    assert_eq!(s.entries, 1);
}

/// Scroll population: viewport at page 5 (count 3) with radius 2 renders
/// exactly pages [3, 10) through the real engine and respects the budget.
#[test]
fn visible_and_prefetch_populate_cache_within_budget() {
    const BUDGET: usize = 4 * 1024 * 1024;
    let mut cache = open_cache("large_document.pdf", BUDGET);

    assert_eq!(cache.page_count(), 500);

    let vp = pdf_core::Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    // Pure math cross-check, matches the real populate below.
    assert_eq!(
        pdf_core::visible_and_prefetch_pages(&vp, cache.page_count(), 2),
        3..10
    );

    pdf_core::scroll::populate_visible(&mut cache, &vp, 2).expect("populate");

    let s = cache.stats();
    assert_eq!(s.misses, 7, "pages 3..=9 rendered once each");
    assert!(
        s.current_bytes <= BUDGET,
        "budget respected while populating"
    );
    assert!(s.entries >= 1, "some pages stay resident");
}

/// A zoom level differs from its neighbours in cache space: same page at two
/// scales is two independent entries (misses).
#[test]
fn different_scale_levels_are_independent_entries() {
    let mut cache = open_cache("dense_textbook.pdf", 64 * 1024 * 1024);

    let a_bytes = cache.get_or_render(0, 0).expect("render 1x").byte_size;
    let b_bytes = cache.get_or_render(0, 1).expect("render 2x").byte_size;
    assert_ne!(a_bytes, b_bytes, "2x bitmap is ~4x the bytes");

    let s = cache.stats();
    assert_eq!(s.misses, 2);
    assert_eq!(s.entries, 2);
    assert_eq!(s.current_bytes, a_bytes + b_bytes);

    // Both entries still hit individually.
    cache.get_or_render(0, 0).expect("1x hit");
    cache.get_or_render(0, 1).expect("2x hit");
    let s = cache.stats();
    assert_eq!(s.hits, 2);
    assert_eq!(s.misses, 2);
}

/// A single page larger than the whole budget evicts everything and is stored
/// alone (documented best-effort behaviour): the invariant is relaxed for that
/// one entry, and it still serves hits afterwards.
#[test]
fn page_larger_than_budget_is_stored_alone_after_full_eviction() {
    const BUDGET: usize = 4 * 1024 * 1024;
    let mut cache = open_cache("dense_textbook.pdf", BUDGET);

    cache.get_or_render(0, 0).expect("render 1x"); // ~2 MB, fits
    assert_eq!(cache.stats().entries, 1);

    // Scale level 2 = 4.0 (288 dpi): an A4 page is ~32 MB — over the budget.
    cache.get_or_render(0, 2).expect("render 4x");
    let s = cache.stats();
    assert_eq!(s.entries, 1, "the oversized page is stored alone");
    assert_eq!(s.evictions, 1, "the resident 1x page was evicted");
    assert!(
        s.current_bytes > BUDGET,
        "single oversized page breaks the budget"
    );

    // The oversized entry is cached normally afterwards.
    cache.get_or_render(0, 2).expect("4x hit");
    let s = cache.stats();
    assert_eq!(s.hits, 1);
    assert_eq!(s.misses, 2);
}
