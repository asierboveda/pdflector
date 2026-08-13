// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Export of annotations (docs/PLAN.md §5, Fase 3):
//!
//! - **Markdown**: `export_markdown` returns a `String`; `export_markdown_to_file`
//!   is the only I/O wrapper. Pure logic, text extraction stays lazy —
//!   `Document::text` is only called for pages that actually contain highlights.
//! - **PDF**: `export_pdf_annotated` embeds the annotations as standard PDF
//!   annotations (/Ink, /Highlight, /Text) into a copy of the source PDF,
//!   legible in any PDF reader.
//!
//! # Output format
//!
//! One `## Página N` section per annotated page (N = page_idx + 1, 1-based
//! for the reader), in page order. Within a section, annotations keep
//! insertion (z) order. Output labels are Spanish, the app's user language:
//!
//! ```markdown
//! ## Página 1
//!
//! > Nota al margen.
//!
//! ## Página 3
//!
//! > The highlighted line, as extracted from the page's text spans.
//! >
//! > — pág. 3
//!
//! Dibujo en la página 3
//! ```
//!
//! - `TextNote` → blockquote with the note text (multi-line notes get one
//!   `> ` prefix per line).
//! - `Highlight` → blockquote with the quote: the page's `TextSpan` lines
//!   whose bounding boxes intersect any of the highlight's rects (rects are
//!   per-line boxes in page coordinates, see `annotations.rs`), joined in
//!   reading order, plus a `— pág. N` attribution. When no span intersects
//!   (imprecise mapping, image-only page), the whole page text is quoted
//!   instead with the page number marked — the documented fallback.
//! - `Stroke` → a plain "Dibujo en la página N" line (freehand strokes carry
//!   no text).

use std::path::Path;

use mupdf::color::AnnotationColor;
use mupdf::pdf::{PdfAnnotation, PdfPage};
use mupdf::{Point, Quad, Rect as PdfRect};

use crate::annotations::{Annotation, AnnotationSet, Highlight, Rect, Stroke, TextNote};
use crate::engine::{Document, Error, PageText, Result, TextSpan};

/// Renders `set` as structured Markdown: one `## Página N` section per
/// annotated page (1-based page numbers), in page order. Pure function —
/// returns a `String`, performs no I/O. Pages without annotations are
/// omitted entirely.
///
/// Text extraction is lazy: `Document::text(page)` is called only for pages
/// that contain at least one highlight (the only annotation kind that needs
/// spans to build its quote).
pub fn export_markdown(doc: &dyn Document, set: &AnnotationSet) -> Result<String> {
    let mut out = String::new();
    for page in 0..doc.page_count() {
        let anns = set.for_page(page as usize);
        if anns.is_empty() {
            continue;
        }
        // Lazy text: spans are only needed for highlight quotes; notes and
        // strokes carry no text, so skip extraction for pages without them.
        let has_highlight = anns
            .iter()
            .any(|a| matches!(a.kind, Annotation::Highlight(_)));
        let page_text = if has_highlight {
            Some(doc.text(page)?)
        } else {
            None
        };

        out.push_str(&format!("## Página {}\n\n", page + 1));
        for a in anns {
            match &a.kind {
                Annotation::TextNote(note) => {
                    out.push_str(&blockquote(&note.text));
                    out.push_str("\n\n");
                }
                Annotation::Highlight(hl) => {
                    let pt = page_text
                        .as_ref()
                        .expect("page text fetched when a highlight exists");
                    let quote = match highlight_quote(pt, hl) {
                        Some(q) => q,
                        None => {
                            // Imprecise mapping or an image-only page: quote
                            // the whole page text, page number marked.
                            if pt.text.trim().is_empty() {
                                "(texto no extraíble)".to_string()
                            } else {
                                pt.text.trim().to_string()
                            }
                        }
                    };
                    out.push_str(&blockquote(&quote));
                    out.push_str("\n>\n");
                    out.push_str(&format!("> — pág. {}\n\n", page + 1));
                }
                Annotation::Stroke(_) => {
                    out.push_str(&format!("Dibujo en la página {}\n\n", page + 1));
                }
            }
        }
    }
    Ok(out)
}

/// Convenience wrapper: exports `set` via `export_markdown` and writes the
/// UTF-8 result to `path`. The export logic itself stays I/O-free.
pub fn export_markdown_to_file(path: &Path, doc: &dyn Document, set: &AnnotationSet) -> Result<()> {
    let md = export_markdown(doc, set)?;
    std::fs::write(path, md)?; // io error → Error::Io via the From impl
    Ok(())
}

/// The quote under a highlight: the span lines of `pt` whose bounding boxes
/// intersect any of `hl.rects`, joined in reading order, deduplicated (a
/// span could be hit by two adjacent rects). Highlight rects are per-line
/// boxes in the same page-coordinate space as the spans, so a positive bbox
/// intersection is the expected match.
///
/// Returns `None` when no span matched — the caller falls back to the whole
/// page text (the documented imprecise path).
fn highlight_quote(pt: &PageText, hl: &Highlight) -> Option<String> {
    let mut matched: Vec<usize> = Vec::new();
    for (i, span) in pt.spans.iter().enumerate() {
        if hl.rects.iter().any(|r| intersects(r, span)) {
            matched.push(i);
        }
    }
    if matched.is_empty() {
        return None;
    }
    // `enumerate` keeps reading order, so `dedup` on the ascending indices
    // preserves it.
    matched.dedup();
    let quote = matched
        .iter()
        .map(|&i| pt.spans[i].text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if quote.trim().is_empty() {
        return None;
    }
    Some(quote)
}

/// True when `span`'s bounding box overlaps `rect` with positive area in
/// both axes. Both are in page coordinates (top-left origin, y grows down);
/// `Rect` is normalized to `w >= 0, h >= 0` by `AnnotationSet::add` and
/// spans have `w > 0, h > 0`, so the strict comparisons are well-defined.
fn intersects(rect: &Rect, span: &TextSpan) -> bool {
    rect.x < span.x + span.w
        && span.x < rect.x + rect.w
        && rect.y < span.y + span.h
        && span.y < rect.y + rect.h
}

/// Prefixes every line of `text` with `> ` (empty lines become a bare `>`),
/// producing a CommonMark blockquote. `str::lines` handles `\n` and `\r\n`.
fn blockquote(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// PDF export (Fase 3: annotations legibles en cualquier lector)
// ---------------------------------------------------------------------------

/// Exports `set` into a copy of the PDF at `src_pdf`, embedding each
/// annotation as a **standard PDF annotation** (ISO 32000-1 §12.5), and
/// writes the result to `out`.
///
/// Mapping (Fase 3 requirement: annotations legible in any PDF reader):
///
/// | Model kind   | PDF Subtype | How                                        |
/// |--------------|-------------|--------------------------------------------|
/// | `Stroke`     | /Ink        | polyline vertices, /BS /W = stroke width   |
/// | `Highlight`  | /Highlight  | one /QuadPoints quad per rect             |
/// | `TextNote`   | /Text       | /Contents = note text, icon rect at anchor |
///
/// Engine: the crate's `mupdf::pdf` module (the same MuPDF used for
/// rendering, ADR-001) is the only engine that can *write* standard PDF
/// annotations. The `Document`/`RenderEngine` traits model the read-only
/// render/text surface, not PDF-level write access, so this function binds
/// the concrete `PdfDocument` API directly — an intentional exception to
/// AGENTS.md §4.2: that dependency is already hard (pdf_core links mupdf
/// for rendering) and MuPDF is the single backend by ADR-001.
///
/// Coordinates are page coordinates (top-left origin, y grows down) — the
/// same space as the annotation model and as MuPDF's Fitz page space, so
/// points and rects are passed through unchanged.
///
/// The output is a full rewrite (`PdfWriteOptions::default()`), not an
/// incremental update: simplest correct output, and an incremental save
/// would keep stale xref generations that hurt the Syncthing-friendly byte
/// stability goal (Fase 3 store).
pub fn export_pdf_annotated(src_pdf: &Path, set: &AnnotationSet, out: &Path) -> Result<()> {
    // mupdf's `save` takes a `&str` path; reject non-UTF-8 output paths with
    // a clear error instead of silently lossy-encoding them.
    let out_name = out.to_str().ok_or_else(|| {
        Error::InvalidArgument(format!("output path is not valid UTF-8: {}", out.display()))
    })?;

    // `load_pdf_page`/`save` take `&self`: pages are refcounted and owned
    // separately, so the document handle itself stays immutable.
    let doc = mupdf::pdf::PdfDocument::open(src_pdf)
        .map_err(|e| Error::Engine(format!("open {}: {e}", src_pdf.display())))?;
    let page_count = doc
        .page_count()
        .map_err(|e| Error::Engine(e.to_string()))?
        .max(0);

    for page_idx in 0..page_count {
        let anns = set.for_page(page_idx as usize);
        if anns.is_empty() {
            continue;
        }
        let mut page = doc
            .load_pdf_page(page_idx)
            .map_err(|e| Error::Engine(e.to_string()))?;
        for a in anns {
            match &a.kind {
                Annotation::Stroke(s) => add_ink_annotation(&mut page, s)?,
                Annotation::Highlight(h) => add_highlight_annotation(&mut page, h)?,
                Annotation::TextNote(n) => add_text_note_annotation(&mut page, n)?,
            }
        }
    }

    doc.save(out_name)
        .map_err(|e| Error::Engine(format!("save {}: {e}", out.display())))?;
    Ok(())
}

/// `Stroke` → /Ink: a single ink stroke carrying the polyline vertices, with
/// the border width set to the pen width (PDF readers draw /Ink with /BS /W)
/// and the stroke colour. `AnnotationSet::add` guarantees ≥ 2 points, so the
/// ink stroke is never degenerate.
fn add_ink_annotation(page: &mut PdfPage, s: &Stroke) -> Result<()> {
    let points: Vec<Point> = s.points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let mut annot = page
        .add_ink_annotation([points])
        .map_err(|e| Error::Engine(format!("ink annotation: {e}")))?;
    annot
        .set_color(rgb(s.color.r, s.color.g, s.color.b))
        .map_err(|e| Error::Engine(e.to_string()))?;
    annot
        .set_border_width(s.width)
        .map_err(|e| Error::Engine(e.to_string()))?;
    set_opacity(&mut annot, s.color.a)?;
    Ok(())
}

/// `Highlight` → /Highlight: one /QuadPoints quad per rect (rects are
/// per-line boxes, each becomes its own quad). Zero-area rects (w or h == 0)
/// are skipped — they cannot form a valid quad — and a highlight left with
/// no quads is rejected with a clear error instead of a confusing engine
/// message.
fn add_highlight_annotation(page: &mut PdfPage, h: &Highlight) -> Result<()> {
    let quads: Vec<Quad> = h
        .rects
        .iter()
        .filter(|r| r.w > 0.0 && r.h > 0.0)
        .map(|r| Quad::from(PdfRect::new(r.x, r.y, r.x + r.w, r.y + r.h)))
        .collect();
    if quads.is_empty() {
        return Err(Error::InvalidArgument(
            "highlight has no non-degenerate rects".to_string(),
        ));
    }
    let mut annot = page
        .add_highlight_annotation(quads)
        .map_err(|e| Error::Engine(format!("highlight annotation: {e}")))?;
    annot
        .set_color(rgb(h.color.r, h.color.g, h.color.b))
        .map_err(|e| Error::Engine(e.to_string()))?;
    set_opacity(&mut annot, h.color.a)?;
    Ok(())
}

/// `TextNote` → /Text: the note text as /Contents and a fixed-size icon rect
/// (a 20 pt square, the classic note icon) with the anchor as its top-left
/// corner. The model has no colour for notes, so the icon keeps the reader
/// default.
fn add_text_note_annotation(page: &mut PdfPage, n: &TextNote) -> Result<()> {
    let rect = PdfRect::new(n.anchor.0, n.anchor.1, n.anchor.0 + 20.0, n.anchor.1 + 20.0);
    page.add_text_annotation(rect, &n.text)
        .map_err(|e| Error::Engine(format!("text annotation: {e}")))?;
    Ok(())
}

/// Builds an `AnnotationColor` from our 0–255 channels (PDF takes 0–1
/// floats). Alpha is handled separately via `set_opacity`.
fn rgb(r: u8, g: u8, b: u8) -> AnnotationColor {
    AnnotationColor::Rgb {
        red: r as f32 / 255.0,
        green: g as f32 / 255.0,
        blue: b as f32 / 255.0,
    }
}

/// Maps our alpha channel to the PDF /CA opacity (0–1). Fully opaque
/// annotations (a == 255) keep the reader default, so no /CA is emitted.
fn set_opacity(annot: &mut PdfAnnotation, a: u8) -> Result<()> {
    if a < 255 {
        annot
            .set_opacity(a as f32 / 255.0)
            .map_err(|e| Error::Engine(e.to_string()))?;
    }
    Ok(())
}
