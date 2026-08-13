// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Markdown export integration tests (Fase 3): the export.rs public API
//! against a fake engine with synthetic text spans — no MuPDF, no corpus
//! (pattern from tests/zoom.rs, extended with `Document::text`).
//!
//! PDF annotation export (`export_pdf_annotated`) is tested for real: it
//! writes standard PDF annotations, so the round-trip reopens the output
//! with MuPDF itself and checks types and properties (no external reader
//! dependency in the test harness).

use pdf_core::engine::{Bitmap, Document, PageText, Result, TextSpan};
use pdf_core::export::{export_markdown, export_markdown_to_file, export_pdf_annotated};
use pdf_core::{Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, TextNote};

/// Fake document whose pages expose fixed text spans with known bounding
/// boxes, so highlight-rect → span mapping is predictable.
struct FakeDoc;

impl Document for FakeDoc {
    fn page_count(&self) -> u32 {
        3
    }

    fn page_size(&self, _page: u32) -> Result<(f32, f32)> {
        Ok((200.0, 300.0))
    }

    fn render_page(&self, page: u32, _scale: f32) -> Result<Bitmap> {
        Ok(Bitmap {
            width: 10,
            height: 10,
            data: vec![page as u8; 10 * 10 * 4],
        })
    }

    fn text(&self, page: u32) -> Result<PageText> {
        match page {
            // Two lines at y = 10..22 and y = 24..36: a highlight rect at
            // y = 24..36 must match only the second line.
            0 => Ok(PageText {
                text: "First line of page one. Second line with a highlight.".to_string(),
                spans: vec![
                    TextSpan {
                        text: "First line of page one.".to_string(),
                        x: 10.0,
                        y: 10.0,
                        w: 200.0,
                        h: 12.0,
                    },
                    TextSpan {
                        text: "Second line with a highlight.".to_string(),
                        x: 10.0,
                        y: 24.0,
                        w: 230.0,
                        h: 12.0,
                    },
                ],
            }),
            1 => Ok(PageText {
                text: "Page two, unannotated.".to_string(),
                spans: vec![TextSpan {
                    text: "Page two, unannotated.".to_string(),
                    x: 10.0,
                    y: 10.0,
                    w: 200.0,
                    h: 12.0,
                }],
            }),
            _ => Ok(PageText {
                text: "Page three text.".to_string(),
                spans: vec![TextSpan {
                    text: "Page three text.".to_string(),
                    x: 10.0,
                    y: 10.0,
                    w: 150.0,
                    h: 12.0,
                }],
            }),
        }
    }
}

fn color() -> Color {
    Color {
        r: 255,
        g: 0,
        b: 0,
        a: 200,
    }
}

#[test]
fn export_renders_annotated_pages_with_quotes_notes_and_strokes() {
    let mut set = AnnotationSet::new();
    // Highlight on page 0 covering exactly the second line (y = 24..36).
    set.add(
        0,
        Annotation::Highlight(Highlight {
            rects: vec![Rect::new(10.0, 24.0, 150.0, 12.0)],
            color: color(),
        }),
    );
    set.add(
        2,
        Annotation::TextNote(TextNote {
            anchor: (5.0, 5.0),
            text: "Importante: revisar.".to_string(),
        }),
    );
    set.add(
        2,
        Annotation::Stroke(
            Stroke::new(vec![(5.0, 5.0), (50.0, 60.0)], 2.0, color()).expect("valid stroke"),
        ),
    );

    let md = export_markdown(&FakeDoc, &set).expect("export");

    // Exact format contract: sections per annotated page only (1-based),
    // note as blockquote, highlight quote with page attribution, stroke
    // mention without text.
    let expected = "\
## Página 1

> Second line with a highlight.
>
> — pág. 1

## Página 3

> Importante: revisar.

Dibujo en la página 3

";
    assert_eq!(md, expected);

    // The unannotated page 2 (1-based) must not appear, and the span outside
    // the highlight rect must not leak into the quote.
    assert!(
        !md.contains("## Página 2"),
        "unannotated page leaked:\n{md}"
    );
    assert!(
        !md.contains("First line of page one"),
        "unrelated span leaked into the quote:\n{md}"
    );
}

#[test]
fn highlight_without_matching_span_falls_back_to_whole_page_text() {
    let mut set = AnnotationSet::new();
    // Rect at y ≈ 100 hits no span of page 0 (spans live at y = 10..36).
    set.add(
        0,
        Annotation::Highlight(Highlight {
            rects: vec![Rect::new(0.0, 100.0, 50.0, 12.0)],
            color: color(),
        }),
    );

    let md = export_markdown(&FakeDoc, &set).expect("export");

    assert!(
        md.contains("> First line of page one. Second line with a highlight."),
        "fallback must quote the whole page text:\n{md}"
    );
    assert!(md.contains("— pág. 1"), "page number must be marked:\n{md}");
}

#[test]
fn empty_set_produces_empty_output() {
    let set = AnnotationSet::new();
    let md = export_markdown(&FakeDoc, &set).expect("export");
    assert!(md.is_empty());
}

#[test]
fn export_markdown_to_file_writes_utf8_markdown() {
    let mut set = AnnotationSet::new();
    set.add(
        0,
        Annotation::TextNote(TextNote {
            anchor: (0.0, 0.0),
            // Multi-line note: each line must get its own `> ` prefix.
            text: "Línea uno.\nLínea dos.".to_string(),
        }),
    );
    let path =
        std::env::temp_dir().join(format!("pdflector_export_test_{}.md", std::process::id()));

    export_markdown_to_file(&path, &FakeDoc, &set).expect("write to file");

    let written = std::fs::read_to_string(&path).expect("read back");
    assert!(written.contains("## Página 1"));
    assert!(
        written.contains("> Línea uno.\n> Línea dos."),
        "multi-line note blockquote:\n{written}"
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// PDF export: round-trip through real MuPDF (standard annotations)
// ---------------------------------------------------------------------------

use mupdf::pdf::{PdfAnnotationType, PdfDocument};

/// Asset: tests/assets/simple.pdf (2-page A4, committed).
fn pdf_asset() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/assets/simple.pdf")
}

/// Temp file for one export; removed on drop.
struct TempOut(std::path::PathBuf);

impl TempOut {
    fn new(ext: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "pdflector_export_pdf_test_{}_{}.{ext}",
            std::process::id(),
            rand_suffix()
        )))
    }
}

impl std::ops::Deref for TempOut {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempOut {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Cheap unique suffix so parallel test runs don't collide on the temp file.
fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .subsec_nanos() as u64
}

/// Collects every annotation of `path` as (page, subtype, annotation), in
/// document order — the round-trip reader.
fn read_annotations(
    path: &std::path::Path,
) -> Vec<(i32, PdfAnnotationType, mupdf::pdf::PdfAnnotation)> {
    let doc = PdfDocument::open(path).expect("reopen exported pdf");
    let count = doc.page_count().expect("page count");
    let mut out = Vec::new();
    for page_idx in 0..count {
        let page = doc.load_pdf_page(page_idx).expect("load page");
        for annot in page.annotations() {
            out.push((page_idx, annot.r#type().expect("type"), annot));
        }
    }
    out
}

#[test]
fn export_pdf_annotated_embeds_standard_annotations() {
    let mut set = AnnotationSet::new();
    // Page 0 (1-based 1): one stroke + one highlight, like the Markdown test.
    let stroke_points = vec![(10.0, 20.0), (30.5, 40.25), (50.0, 60.0)];
    set.add(
        0,
        Annotation::Stroke(Stroke::new(stroke_points.clone(), 2.5, color()).expect("valid stroke")),
    );
    set.add(
        0,
        Annotation::Highlight(Highlight {
            rects: vec![Rect::new(72.0, 72.0, 160.0, 20.0)],
            color: color(),
        }),
    );
    // Page 1: one text note.
    set.add(
        1,
        Annotation::TextNote(TextNote {
            anchor: (100.0, 100.0),
            text: "nota de prueba".to_string(),
        }),
    );

    let out = TempOut::new("pdf");
    export_pdf_annotated(&pdf_asset(), &set, &out).expect("export pdf");
    assert!(out.exists(), "output written");

    let anns = read_annotations(&out);
    // 3 annotations: /Ink + /Highlight on page 0, /Text on page 1.
    assert_eq!(anns.len(), 3, "annotations: {anns:?}");
    let mut kinds: Vec<String> = anns.iter().map(|(_, t, _)| format!("{t:?}")).collect();
    kinds.sort();
    assert_eq!(kinds, vec!["Highlight", "Ink", "Text"]);

    // Subtypes land on the right pages and keep their geometry.
    let mut ink_points = None;
    let mut highlight_quads = 0;
    let mut note_contents = None;
    for (page, ty, annot) in &anns {
        match ty {
            PdfAnnotationType::Ink => {
                assert_eq!(*page, 0, "ink must be on page 0");
                ink_points = Some(annot.ink_list().expect("ink list"));
            }
            PdfAnnotationType::Highlight => {
                assert_eq!(*page, 0, "highlight must be on page 0");
                highlight_quads = annot.quad_points().expect("quads").len();
            }
            PdfAnnotationType::Text => {
                assert_eq!(*page, 1, "note must be on page 1");
                note_contents = annot.contents().expect("contents").map(str::to_string);
            }
            other => panic!("unexpected annotation type {other:?}"),
        }
    }

    // Ink vertices round-trip exactly (same Fitz page space).
    let ink = ink_points.expect("ink annotation present");
    assert_eq!(ink.len(), 1, "one ink stroke");
    assert_eq!(
        ink[0],
        vec![
            mupdf::Point::new(10.0, 20.0),
            mupdf::Point::new(30.5, 40.25),
            mupdf::Point::new(50.0, 60.0),
        ]
    );
    // One rect → one quad.
    assert_eq!(highlight_quads, 1);
    // Note text survives.
    assert_eq!(note_contents.as_deref(), Some("nota de prueba"));
}

#[test]
fn export_pdf_annotated_fails_cleanly_on_missing_source() {
    let missing = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets/does_not_exist.pdf");
    let set = AnnotationSet::new();
    let out = TempOut::new("pdf");
    let err = export_pdf_annotated(&missing, &set, &out).expect_err("must fail");
    assert!(
        err.to_string().contains("engine error"),
        "open failure must surface as Engine error, got: {err}"
    );
}

#[test]
fn export_pdf_annotated_with_empty_set_writes_unmodified_copy() {
    let set = AnnotationSet::new();
    let out = TempOut::new("pdf");
    export_pdf_annotated(&pdf_asset(), &set, &out).expect("export pdf");
    // No annotations added; the copy must still open and keep the page count.
    assert_eq!(read_annotations(&out).len(), 0);
    let doc = PdfDocument::open(&*out).expect("reopen");
    assert_eq!(doc.page_count().expect("page count"), 2);
}
