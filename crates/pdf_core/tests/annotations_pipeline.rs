// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! End-to-end annotation pipeline on a real PDF (Fase 3): text extraction →
//! highlight-by-gesture → annotation set → SQLite sidecar round-trip → raster
//! overlay on the rendered page bitmap. Keeps the pure units (selection.rs,
//! annotations.rs, overlay.rs) honest against MuPDF's real span boxes.

use std::path::PathBuf;

use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::store::AnnotationStore;
use pdf_core::{
    Annotation, AnnotationSet, Document, Gesture, RenderEngine, ViewTransform,
    composite_annotations, highlight_under_gesture, smooth_polyline,
};

fn asset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/simple.pdf")
}

fn engine() -> MupdfEngine {
    MupdfEngine::new().expect("mupdf engine init")
}

#[test]
fn highlight_pipeline_on_real_pdf_survives_store_round_trip() {
    let doc = engine().open(&asset()).expect("open test pdf");
    let spans = doc.text(0).expect("text(0)").spans;
    assert!(!spans.is_empty(), "simple.pdf page 0 has extractable lines");

    // Gesture: a pen stroke over the FIRST line, from its left edge to the
    // middle. The highlight must clip to the stroke's x extent.
    let line = &spans[0];
    let gesture = Gesture::Points(vec![
        (line.x, line.y + line.h * 0.5),
        (line.x + line.w * 0.4, line.y + line.h * 0.5),
    ]);
    let hl = highlight_under_gesture(&spans, &gesture, pdf_core::HIGHLIGHT_COLOR)
        .expect("a real line under the stroke");
    assert_eq!(hl.rects.len(), 1);
    let r = hl.rects[0];
    assert_eq!(r.y, line.y, "rect keeps the line's y");
    assert_eq!(r.h, line.h, "rect keeps the line's height");
    assert!(
        (r.x - line.x).abs() < 0.01,
        "rect starts at the stroke start"
    );
    assert!(r.w < line.w, "rect clips to the stroke extent");

    // Store round-trip through the SQLite sidecar (temp file).
    let db = std::env::temp_dir().join(format!("pdflector-pipeline-{}.db", std::process::id()));
    let store = AnnotationStore::open(&db).expect("open store");
    let mut set = AnnotationSet::new();
    let id = set.add(0, Annotation::Highlight(hl)).expect("add");
    store.save(&set).expect("save");
    let loaded = store.load().expect("load");
    assert_eq!(loaded, set);
    assert_eq!(loaded.for_page(0)[0].id, id, "ids survive the round-trip");
    std::fs::remove_file(&db).ok();
}

#[test]
fn overlay_paints_selected_highlight_on_rendered_bitmap() {
    let doc = engine().open(&asset()).expect("open test pdf");
    let spans = doc.text(0).expect("text(0)").spans;
    let line = &spans[0];

    let gesture = Gesture::Points(vec![
        (line.x, line.y + 1.0),
        (line.x + line.w, line.y + 1.0),
    ]);
    let hl = highlight_under_gesture(&spans, &gesture, pdf_core::HIGHLIGHT_COLOR)
        .expect("highlight over the whole line");

    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Highlight(hl)).expect("add");
    let anns = set.for_page(0);

    // Render the page at zoom 2 and composite the annotation layer on top.
    let zoom = 2.0f32;
    let mut bmp = doc.render_page(0, zoom).expect("render");
    let pixel_at = |buf: &[u8], w: usize, x: usize, y: usize| -> (u8, u8, u8) {
        let i = (y * w + x) * 4;
        (buf[i], buf[i + 1], buf[i + 2])
    };
    // Screen coords: inside the (full-line) highlight...
    let sx = ((line.x + line.w * 0.5) * zoom) as usize;
    let sy = ((line.y + line.h * 0.5) * zoom) as usize;
    // ...and a far pixel below the last line (unmarked area).
    let fy = (((spans.last().unwrap().y + 100.0) * zoom) as usize).min(bmp.height as usize - 1);
    let fx = 5usize;
    let before_inner = pixel_at(&bmp.data, bmp.width as usize, sx, sy);
    let before_far = pixel_at(&bmp.data, bmp.width as usize, fx, fy);

    composite_annotations(
        &mut bmp.data,
        bmp.width,
        bmp.height,
        &anns,
        &ViewTransform {
            zoom,
            offset_x: 0.0,
            offset_y: 0.0,
        },
    );
    let after_inner = pixel_at(&bmp.data, bmp.width as usize, sx, sy);
    let after_far = pixel_at(&bmp.data, bmp.width as usize, fx, fy);
    assert_ne!(
        before_inner, after_inner,
        "the marker tint must change the page pixel under the line"
    );
    assert_eq!(
        before_far, after_far,
        "unmarked area must keep its page colour untouched"
    );
}

#[test]
fn smoothed_stroke_and_highlight_serialize_end_to_end() {
    // The Fase 3 boli drawings are stored smoothed: a capture is Catmull-Rom
    // interpolated, stored as an Ink stroke, and survives the store
    // round-trip byte-identically.
    let capture = vec![(10.0, 10.0), (30.0, 12.0), (55.0, 30.0), (80.0, 32.0)];
    let smoothed = smooth_polyline(&capture, 4);
    assert!(smoothed.len() > capture.len(), "interpolation adds points");
    assert_eq!(smoothed.first(), Some(&(10.0, 10.0)), "anchor kept");
    assert_eq!(smoothed.last(), Some(&(80.0, 32.0)), "end kept");

    let stroke = Annotation::Stroke(
        pdf_core::Stroke::new(
            smoothed,
            1.5,
            pdf_core::Color {
                r: 0,
                g: 0,
                b: 200,
                a: 255,
            },
        )
        .expect("valid stroke"),
    );
    let db = std::env::temp_dir().join(format!("pdflector-ink-{}.db", std::process::id()));
    let store = AnnotationStore::open(&db).expect("open store");
    let mut set = AnnotationSet::new();
    set.add(0, stroke).expect("add");
    store.save(&set).expect("save");
    let loaded = store.load().expect("load");
    assert_eq!(loaded, set);
    std::fs::remove_file(&db).ok();
}
