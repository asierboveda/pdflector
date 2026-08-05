//! Phase 0 acceptance tests (docs/PLAN.md §5, Fase 0): open a PDF, report
//! page count, render page 1 to a bitmap of the expected dimensions.
//! Asset: tests/assets/simple.pdf (2-page A4, committed; generated with
//! reportlab — uv + python, see tools/generate_corpus.py for the big corpus).

use std::path::PathBuf;

use pdf_core::engine::pdfium::PdfiumEngine;
use pdf_core::{Document, Error, RenderEngine};

fn lib_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium/lib/libpdfium.so")
}

fn asset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/simple.pdf")
}

fn open_test_doc() -> pdf_core::engine::pdfium::PdfiumDocument {
    let engine = PdfiumEngine::new(&lib_path()).expect("bind libpdfium");
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
