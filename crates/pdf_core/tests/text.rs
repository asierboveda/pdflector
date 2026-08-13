// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_core text extraction tests (docs/PLAN.md §3.2, base for Fase 3
//! highlight-by-selection and Fase 5 chunking): `Document::text` is lazy —
//! only invoked on demand, never during render/scroll.
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
fn text_of_simple_pdf_is_non_empty() {
    let doc = open_test_doc();
    let page_text = doc.text(0).expect("text(0) should succeed");

    assert!(
        !page_text.text.trim().is_empty(),
        "page 0 should have extractable text"
    );
    assert!(
        !page_text.spans.is_empty(),
        "page 0 should yield at least one line span"
    );
    // Spans are lines with a real bounding box in page coordinates.
    for span in &page_text.spans {
        assert!(
            span.w > 0.0 && span.h > 0.0,
            "span {:?} should have a non-empty bbox",
            span.text
        );
        assert!(!span.text.is_empty(), "span text should not be empty");
    }
}

#[test]
fn text_out_of_range_is_an_error() {
    let doc = open_test_doc();
    let err = doc.text(99).unwrap_err();
    assert!(matches!(err, Error::PageOutOfRange { page: 99, .. }));
}
