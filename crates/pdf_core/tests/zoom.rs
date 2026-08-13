// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Zoom support integration tests (Fase 1 B3): selective cache trimming and
//! the public API surface. Uses a fake engine with synthetic bitmaps — no
//! MuPDF, no corpus (see `zoom.rs` unit tests for `scale_bitmap` /
//! `scale_level_for_zoom` details).

use std::path::Path;

use pdf_core::cache::RenderCache;
use pdf_core::engine::{Bitmap, Document, PageText, RenderEngine, Result};
use pdf_core::{scale_bitmap, scale_level_for_zoom};

/// Fake engine whose bitmaps scale with the ladder, giving each entry a
/// distinct, predictable byte footprint: level 0 -> 10x10 (400 B),
/// level 1 -> 20x20 (1600 B), level 2 -> 40x40 (6400 B).
struct FakeEngine;

struct FakeDoc;

impl Document for FakeDoc {
    fn page_count(&self) -> u32 {
        4
    }

    fn page_size(&self, _page: u32) -> Result<(f32, f32)> {
        Ok((100.0, 100.0))
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap> {
        let w = (10.0 * scale) as u32;
        let h = (10.0 * scale) as u32;
        Ok(Bitmap {
            width: w,
            height: h,
            data: vec![page as u8; w as usize * h as usize * 4],
        })
    }

    fn text(&self, _page: u32) -> Result<PageText> {
        Ok(PageText {
            text: String::new(),
            spans: Vec::new(),
        })
    }
}

impl RenderEngine for FakeEngine {
    type Document = FakeDoc;

    fn open(&self, _path: &Path) -> Result<Self::Document> {
        Ok(FakeDoc)
    }
}

fn fake_cache() -> RenderCache<FakeEngine> {
    RenderCache::open(FakeEngine, Path::new("unused.pdf"), 1 << 20).expect("open fake cache")
}

#[test]
fn trim_keeps_only_the_requested_level_with_correct_accounting() {
    let mut cache = fake_cache();
    // page 0 at level 0 (400 B), page 1 at level 1 (1600 B),
    // page 2 at level 2 (6400 B), page 3 at level 0 (400 B).
    cache.get_or_render(0, 0).expect("render");
    cache.get_or_render(1, 1).expect("render");
    cache.get_or_render(2, 2).expect("render");
    cache.get_or_render(3, 0).expect("render");

    let s = cache.stats();
    assert_eq!(s.entries, 4);
    assert_eq!(s.current_bytes, 400 + 1600 + 6400 + 400);
    assert_eq!(s.evictions, 0);

    cache.trim_to_scale_level(0);

    let s = cache.stats();
    assert_eq!(s.entries, 2);
    assert_eq!(s.current_bytes, 400 + 400);
    assert_eq!(s.evictions, 2);

    // Only (page 0, level 0) and (page 3, level 0) are still resident.
    let keys = cache.resident_keys();
    assert!(keys.contains(&pdf_core::cache::PageKey {
        page_idx: 0,
        scale_level: 0
    }));
    assert!(keys.contains(&pdf_core::cache::PageKey {
        page_idx: 3,
        scale_level: 0
    }));
    assert_eq!(keys.len(), 2);

    // The surviving level-0 entry is now a hit; the trimmed level-1 entry
    // misses and re-renders (fresh bytes re-added to the accounting).
    cache.get_or_render(0, 0).expect("hit");
    cache.get_or_render(1, 1).expect("re-render");
    let s = cache.stats();
    assert_eq!(s.hits, 1);
    assert_eq!(s.misses, 5);
    assert_eq!(s.entries, 3);
    assert_eq!(s.current_bytes, 400 + 400 + 1600);
}

#[test]
fn trim_to_a_level_with_no_resident_entries_evicts_everything() {
    let mut cache = fake_cache();
    cache.get_or_render(0, 0).expect("render");
    cache.get_or_render(1, 0).expect("render");
    assert_eq!(cache.stats().current_bytes, 800);

    cache.trim_to_scale_level(1);

    let s = cache.stats();
    assert_eq!(s.entries, 0);
    assert_eq!(s.current_bytes, 0);
    assert_eq!(s.evictions, 2);
    assert!(cache.resident_keys().is_empty());
}

#[test]
fn trim_with_nothing_to_evict_is_a_noop() {
    let mut cache = fake_cache();
    cache.get_or_render(0, 0).expect("render");
    let before = *cache.stats();

    cache.trim_to_scale_level(0);

    let s = cache.stats();
    assert_eq!(s.entries, before.entries);
    assert_eq!(s.current_bytes, before.current_bytes);
    assert_eq!(s.evictions, before.evictions);
    // The surviving entry still hits afterwards.
    cache.get_or_render(0, 0).expect("hit");
    assert_eq!(cache.stats().hits, 1);
}

#[test]
fn public_api_exports_are_reachable() {
    // Re-exported at crate root (lib.rs `pub use`), exercised here so a
    // signature drift in the export surface fails the build.
    let src = Bitmap {
        width: 2,
        height: 2,
        data: vec![255; 2 * 2 * 4],
    };
    let out = scale_bitmap(&src, 4, 4).expect("scale via root export");
    assert_eq!((out.width, out.height), (4, 4));
    assert_eq!(scale_level_for_zoom(2.0), 1);
    // Ladder helper and error type are re-exported too.
    assert_eq!(pdf_core::scale_for_level(1), 2.0);
    let err = scale_bitmap(&src, 0, 4).expect_err("zero target width");
    assert!(matches!(err, pdf_core::Error::InvalidArgument(_)));
}
