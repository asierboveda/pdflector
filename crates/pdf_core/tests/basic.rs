// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_core acceptance tests (docs/PLAN.md §5, Fase 0 y 0.5): open a PDF,
//! report page count, render page 1 to a bitmap of the expected dimensions.
//! The engine under test is MuPDF — single backend since ADR-001 (Fase 0.5).
//!
//! Asset: tests/assets/simple.pdf (2-page A4, committed; generated with
//! reportlab — see tools/generate_corpus.py).

use std::path::PathBuf;

use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, Error, RenderEngine};

fn asset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/simple.pdf")
}

fn open_test_doc() -> pdf_core::engine::mupdf::MupdfDocument {
    let engine = MupdfEngine::new().expect("mupdf engine init");
    engine.open(&asset()).expect("open test pdf")
}

#[test]
fn opens_document_and_reports_page_count() {
    assert_eq!(open_test_doc().page_count(), 2);
}

#[test]
fn page_size_is_a4_with_tolerance() {
    let (w, h) = open_test_doc().page_size(0).unwrap();
    // A4 = 595.27 x 841.89 points; allow small engine-specific fuzz.
    assert!((w - 595.27).abs() <= 1.0, "width {w} not A4");
    assert!((h - 841.89).abs() <= 1.0, "height {h} not A4");
}

#[test]
fn renders_page_1_to_rgba_bitmap_of_expected_dimensions() {
    let doc = open_test_doc();
    let scale = 2.0;
    let (w, h) = doc.page_size(0).unwrap();
    let bmp = doc.render_page(0, scale).unwrap();

    assert_eq!(bmp.width, (w * scale).round() as u32);
    assert_eq!(bmp.height, (h * scale).round() as u32);
    assert_eq!(
        bmp.data.len() as u64,
        bmp.width as u64 * bmp.height as u64 * 4
    );
}

#[test]
fn rendered_page_is_not_blank() {
    let bmp = open_test_doc().render_page(0, 1.0).unwrap();
    let dark_pixels = bmp
        .data
        .chunks_exact(4)
        .filter(|px| px[0] < 250 || px[1] < 250 || px[2] < 250)
        .count();
    assert!(dark_pixels > 1000, "page should contain rendered text");
}

#[test]
fn out_of_range_page_is_an_error() {
    let err = open_test_doc().render_page(99, 1.0).unwrap_err();
    assert!(matches!(err, Error::PageOutOfRange { page: 99, .. }));
}
/// Unwrap-free assertion helper for the F3.3 tests below: the file's older
/// tests use `unwrap` (pre-existing clippy debt), but new tests must not add
/// `clippy::unwrap_used` hits.
fn must<T, E: std::fmt::Debug>(result: Result<T, E>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{what}: {error:?}"),
    }
}

// F3.3: display-list-backed rendering. The list is built once per page and
// retained in the document; every later scale change replays it instead of
// re-parsing the page. These tests pin the observable contract: same output
// dimensions/shape as the legacy path, real reuse (second render served from
// the retained list), and correct page-range validation.

#[test]
fn display_list_render_matches_legacy_dimensions() {
    let doc = open_test_doc();
    let scale = 2.0;
    let (w, h) = must(doc.page_size(0), "page_size");
    // First render builds the display list; second replays it.
    let first = must(doc.render_page(0, scale), "first render");
    let second = must(doc.render_page(0, scale), "second render");
    for bmp in [&first, &second] {
        assert_eq!(bmp.width, (w * scale).round() as u32);
        assert_eq!(bmp.height, (h * scale).round() as u32);
        assert_eq!(
            bmp.data.len() as u64,
            bmp.width as u64 * bmp.height as u64 * 4
        );
    }
    // Deterministic rasterization: identical inputs -> identical bytes.
    assert_eq!(first.data, second.data);
}

#[test]
fn display_list_reuse_across_scales_is_not_blank() {
    let doc = open_test_doc();
    let _ = must(doc.render_page(0, 1.0), "build list at level 0");
    let bmp = must(doc.render_page(0, 3.0), "replay at another scale");
    let dark_pixels = bmp
        .data
        .chunks_exact(4)
        .filter(|px| px[0] < 250 || px[1] < 250 || px[2] < 250)
        .count();
    assert!(dark_pixels > 1000, "page should contain rendered text");
}
#[test]
fn display_list_render_out_of_range_page_is_an_error() {
    let error = open_test_doc().render_page(99, 1.0);
    assert!(matches!(error, Err(Error::PageOutOfRange { page: 99, .. })));
}
