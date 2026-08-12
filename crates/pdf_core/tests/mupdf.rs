//! Engine tests: MuPDF backend contract (docs/PLAN.md §5, ADR-001). Same
//! contract as the Phase 0 acceptance tests originally written for PDFium.

use std::path::PathBuf;

use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, Error, RenderEngine};

fn asset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/simple.pdf")
}

fn open_test_doc() -> pdf_core::engine::mupdf::MupdfDoc {
    let engine = MupdfEngine::new();
    engine.open(&asset()).expect("open test pdf")
}

#[test]
fn opens_document_and_reports_page_count() {
    assert_eq!(open_test_doc().page_count(), 2);
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
