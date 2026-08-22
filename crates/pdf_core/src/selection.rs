// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Highlight-by-gesture: map a pen stroke over the page onto the text lines
//! underneath, producing one vector `Highlight` whose rects are aligned to
//! the extracted line boxes (the "underline with a marker" interaction, Fase
//! 3).
//!
//! Input is a gesture in **page coordinates**, either a series of points
//! (continuous stroke — the tablet pen path) or a single axis-aligned rect
//! (a marquee selection), plus the page's extracted lines
//! ([`PageText::spans`](crate::engine::PageText)). Output is a
//! [`Highlight`] with one [`Rect`] per covered line, clipped to the gesture
//! extent so only the underlined part of the line is painted (a plain
//! "one box per whole line" would highlight text the pen never touched).
//!
//! Pure logic: no engine or I/O, so it is unit-testable with synthetic
//! spans, and the caller ([pdf_android] / pdf_app) owns the gesture capture
//! and the lazy `Document::text` call.

use crate::annotations::{Color, Highlight, Rect};

/// Default highlight colour: classic yellow marker, ~50% alpha.
pub const HIGHLIGHT_COLOR: Color = Color {
    r: 255,
    g: 240,
    b: 0,
    a: 128,
};

/// A user gesture over the page, in page coordinates.
#[derive(Debug, Clone)]
pub enum Gesture {
    /// A freehand drag captured as a sequence of pen points.
    Points(Vec<(f32, f32)>),
    /// A marquee selection box (drag from corner to corner).
    Rect(Rect),
}

/// Builds a `Highlight` from a gesture and the page's extracted lines.
///
/// Every span whose bounding box intersects the gesture is covered by one
/// rect aligned to that span (same y/h as the line box), clipped on x to the
/// gesture's horizontal extent — the marker stops where the stroke stops.
/// Degenerate gestures (no points, empty rect) or pages without matching
/// spans yield `None`, so callers can decide whether to create the
/// annotation at all.
///
/// `color` is the highlight colour — a configurable alpha is expected here
/// (the marker look comes from alpha blending in the overlay pass).
pub fn highlight_under_gesture(
    spans: &[crate::engine::TextSpan],
    gesture: &Gesture,
    color: Color,
) -> Option<Highlight> {
    let (x_min, x_max, y_range) = gesture_extent(gesture)?;
    let mut rects: Vec<Rect> = Vec::new();
    for span in spans {
        // Quick reject: no positive-area overlap with the gesture box.
        if span.x + span.w <= x_min
            || span.x >= x_max
            || span.y + span.h <= y_range.0
            || span.y >= y_range.1
        {
            continue;
        }
        let x0 = span.x.max(x_min);
        let x1 = (span.x + span.w).min(x_max);
        if x1 - x0 <= 0.0 {
            continue;
        }
        rects.push(Rect::new(x0, span.y, x1 - x0, span.h));
    }
    if rects.is_empty() {
        None
    } else {
        Some(Highlight { rects, color })
    }
}

/// Horizontal clip range `(min, max_x)` and vertical band `(min, max_y)` of
/// the gesture, in page coordinates. A point gesture is its own bounding
/// box; the marquee rect is normalized. `None` for an empty gesture.
fn gesture_extent(gesture: &Gesture) -> Option<(f32, f32, (f32, f32))> {
    match gesture {
        Gesture::Points(pts) => {
            let mut iter = pts.iter().copied();
            let (mut x_min, mut y_min) = iter.next()?;
            let (mut x_max, mut y_max) = (x_min, y_min);
            for (x, y) in iter {
                x_min = x_min.min(x);
                x_max = x_max.max(x);
                y_min = y_min.min(y);
                y_max = y_max.max(y);
            }
            Some((x_min, x_max, (y_min, y_max)))
        }
        Gesture::Rect(r) => {
            let r = r.normalized();
            Some((r.x, r.x + r.w, (r.y, r.y + r.h)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TextSpan;

    fn line(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            x,
            y,
            w,
            h,
        }
    }

    fn spans() -> Vec<TextSpan> {
        vec![
            line("primera", 10.0, 20.0, 90.0, 12.0),
            line("segunda", 10.0, 34.0, 80.0, 12.0),
            line("tercera", 10.0, 48.0, 100.0, 12.0),
        ]
    }

    #[test]
    fn point_gesture_selects_lines_under_y_band_and_clips_x() {
        // Stroke crossing lines 1 and 2, from x=20 to x=60.
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 25.0), (60.0, 41.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("lines under the stroke");
        assert_eq!(hl.rects.len(), 2);
        // Line 1 (x 10..100): clipped to the stroke's horizontal extent.
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 40.0, 12.0));
        // Line 2 (x 10..90): clipped the same way.
        assert_eq!(hl.rects[1], Rect::new(20.0, 34.0, 40.0, 12.0));
        assert_eq!(hl.color, HIGHLIGHT_COLOR);
    }

    #[test]
    fn marquee_rect_uses_normalized_extent() {
        // Drag right-to-left (negative w) over part of line 1 only: the
        // extent is normalized, and the rect keeps the line's own y/h box
        // (the marker aligns to the text, not to the marquee band).
        let gesture = Gesture::Rect(Rect::new(60.0, 20.0, -30.0, 10.0));
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("line under the marquee");
        assert_eq!(hl.rects, vec![Rect::new(30.0, 20.0, 30.0, 12.0)]);
    }

    #[test]
    fn gesture_between_lines_selects_nothing() {
        let gesture = Gesture::Points(vec![(10.0, 15.0), (50.0, 15.0)]);
        assert!(
            highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR).is_none(),
            "stroke in the gap must not select any line"
        );
    }

    #[test]
    fn degenerate_gestures_yield_none() {
        assert!(
            highlight_under_gesture(&spans(), &Gesture::Points(vec![]), HIGHLIGHT_COLOR).is_none(),
            "an empty point list is not a gesture"
        );
        // A zero-area marquee (w == 0) matches nothing.
        assert!(
            highlight_under_gesture(
                &spans(),
                &Gesture::Rect(Rect::new(10.0, 20.0, 0.0, 12.0)),
                HIGHLIGHT_COLOR
            )
            .is_none(),
            "a zero-extent marquee matches nothing"
        );
    }

    #[test]
    fn empty_page_yields_none() {
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 60.0)]);
        assert!(highlight_under_gesture(&[], &gesture, HIGHLIGHT_COLOR).is_none());
    }
}
