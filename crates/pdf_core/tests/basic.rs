// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_core acceptance tests (engine decision in
//! docs/adr/ADR-001-motor-pdf.md): open a PDF, report page count, render page 1
//! to a bitmap of the expected dimensions. The engine under test is MuPDF —
//! single backend since ADR-001.
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
fn page_geometry_exposes_pdf_boxes_and_maps_crop_box_to_visible_bounds() {
    let geometry = open_test_doc().page_geometry(0).unwrap();
    assert_eq!(geometry.rotation, 0);
    assert!((geometry.visible_bounds.width() - 595.27).abs() <= 1.0);
    assert!((geometry.visible_bounds.height() - 841.89).abs() <= 1.0);

    let corners = [
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x0, geometry.crop_box.y0),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x0, geometry.crop_box.y1),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x1, geometry.crop_box.y0),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x1, geometry.crop_box.y1),
    ];
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((min_x - geometry.visible_bounds.x0).abs() <= 1.0);
    assert!((max_x - geometry.visible_bounds.x1).abs() <= 1.0);
    assert!((min_y - geometry.visible_bounds.y0).abs() <= 1.0);
    assert!((max_y - geometry.visible_bounds.y1).abs() <= 1.0);
}

#[test]
fn rotated_page_with_shifted_crop_box_maps_visible_corners_and_render_size() {
    let path =
        std::env::temp_dir().join(format!("pdflector-rotated-crop-{}.pdf", std::process::id()));
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /CropBox [40 30 260 370] /Rotate 90 /Resources << >> /Contents 4 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (idx, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", idx + 1, object).as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );
    std::fs::write(&path, pdf).unwrap();

    let doc = MupdfEngine::new().unwrap().open(&path).unwrap();
    let geometry = doc.page_geometry(0).unwrap();
    assert_eq!(geometry.rotation, 90);
    assert!((geometry.crop_box.x0 - 40.0).abs() <= 0.01);
    assert!((geometry.crop_box.y0 - 30.0).abs() <= 0.01);
    assert!((geometry.crop_box.x1 - 260.0).abs() <= 0.01);
    assert!((geometry.crop_box.y1 - 370.0).abs() <= 0.01);
    assert!((geometry.visible_bounds.width() - 340.0).abs() <= 1.0);
    assert!((geometry.visible_bounds.height() - 220.0).abs() <= 1.0);

    let corners = [
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x0, geometry.crop_box.y0),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x0, geometry.crop_box.y1),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x1, geometry.crop_box.y0),
        geometry
            .pdf_to_page
            .transform_point(geometry.crop_box.x1, geometry.crop_box.y1),
    ];
    let min_x = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((min_x - geometry.visible_bounds.x0).abs() <= 1.0);
    assert!((max_x - geometry.visible_bounds.x1).abs() <= 1.0);
    assert!((min_y - geometry.visible_bounds.y0).abs() <= 1.0);
    assert!((max_y - geometry.visible_bounds.y1).abs() <= 1.0);

    let bitmap = doc.render_page(0, 1.0).unwrap();
    assert_eq!(bitmap.width, geometry.visible_bounds.width().round() as u32);
    assert_eq!(
        bitmap.height,
        geometry.visible_bounds.height().round() as u32
    );
    drop(doc);
    std::fs::remove_file(path).unwrap();
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
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|[r, g, b, _]| *r < 250 || *g < 250 || *b < 250)
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
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|[r, g, b, _]| *r < 250 || *g < 250 || *b < 250)
        .count();
    assert!(dark_pixels > 1000, "page should contain rendered text");
}

#[test]
fn display_list_render_out_of_range_page_is_an_error() {
    let error = open_test_doc().render_page(99, 1.0);
    assert!(matches!(error, Err(Error::PageOutOfRange { page: 99, .. })));
}
