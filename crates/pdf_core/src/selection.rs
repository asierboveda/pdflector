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
/// **Trazo (Points)** — marker stroke with reading-order extents:
/// lines are touched if the stroke passes through their vertical band (±[`BAND_TOL`] pt)
/// or crosses them. Touched lines are ordered by reading order (document `y`):
///   - Top line extends from the top gesture endpoint to the end of the line;
///   - Strictly intermediate lines are completely highlighted;
///   - Bottom line extends from the start of the line to the bottom gesture endpoint;
///   - When both endpoints fall on the same line: horizontal extent `[min, max]` of both endpoints;
///   - Direction agnostic: drawing top-to-bottom or bottom-to-top produces the same result;
///   - Two-column papers: a line in the other column is only touched if the stroke physically reaches it.
///
/// A minimal touch leaves a visible mark ([`MIN_STROKE_SPAN`] pt).
///
/// **Marquee (Rect)** — block selection: lines whose bbox intersects the
/// rect, clipped to the horizontal extent.
///
/// Degenerate gestures (no points, empty rect) or pages without matching
/// spans yield `None`, so callers can decide whether to create the
/// annotation at all.
pub fn highlight_under_gesture(
    spans: &[crate::engine::TextSpan],
    gesture: &Gesture,
    color: Color,
) -> Option<Highlight> {
    let rects: Vec<Rect> = match gesture {
        Gesture::Points(pts) => match_points(spans, pts),
        Gesture::Rect(r) => {
            let (x_min, x_max, (y0, y1)) = gesture_extent(&Gesture::Rect(*r))?;
            match_rect(spans, x_min, x_max, y0, y1)
        }
    };
    finish(rects, color)
}

/// Sorts spans by top edge (`y`), ascending. Precondition for
/// [`highlight_under_gesture_sorted`]: the gesture path binary-searches this
/// order, so sort ONCE per page-text extraction (amortized over the whole
/// gesture), never per `Move` event.
pub fn sort_spans_by_y(spans: &mut [crate::engine::TextSpan]) {
    spans.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
}

/// [`highlight_under_gesture`] over Y-sorted spans (see [`sort_spans_by_y`]).
/// Narrows the candidates to the gesture's vertical band in
/// `O(log N + K)` (binary search + backward walk over tall spans that start
/// above the band but reach into it), then runs the exact same per-span
/// matcher — output is identical to the linear scan.
pub fn highlight_under_gesture_sorted(
    spans_sorted: &[crate::engine::TextSpan],
    gesture: &Gesture,
    color: Color,
) -> Option<Highlight> {
    let (x_min, x_max, (gy0, gy1)) = gesture_extent(gesture)?;
    let (lo, hi) = y_band_range(spans_sorted, gy0, gy1);
    let rects: Vec<Rect> = match gesture {
        Gesture::Points(pts) => match_points(&spans_sorted[lo..hi], pts),
        Gesture::Rect(_) => match_rect(&spans_sorted[lo..hi], x_min, x_max, gy0, gy1),
    };
    finish(rects, color)
}

/// Candidate index range whose spans may intersect the vertical band
/// `[gy0, gy1]` (±[`BAND_TOL`]). Binary lower bound on the top edge
/// (monotonic in `y`), then a backward walk over tall spans that start
/// above the band but extend into it. Forward walk stops at the first span
/// fully below the band. No allocation.
fn y_band_range(spans_sorted: &[crate::engine::TextSpan], gy0: f32, gy1: f32) -> (usize, usize) {
    let mut lo = spans_sorted.partition_point(|s| s.y < gy0 - BAND_TOL);
    while lo > 0 && spans_sorted[lo - 1].y + spans_sorted[lo - 1].h >= gy0 - BAND_TOL {
        lo -= 1;
    }
    let mut hi = lo;
    while hi < spans_sorted.len() && spans_sorted[hi].y <= gy1 + BAND_TOL {
        hi += 1;
    }
    (lo, hi)
}

fn finish(rects: Vec<Rect>, color: Color) -> Option<Highlight> {
    if rects.is_empty() {
        None
    } else {
        Some(Highlight { rects, color })
    }
}

/// Marker-by-stroke matcher over a candidate slice (shared by the linear
/// and the Y-indexed paths; behaviour documented on
/// [`highlight_under_gesture`]).
fn match_points(spans: &[crate::engine::TextSpan], pts: &[(f32, f32)]) -> Vec<Rect> {
    if pts.is_empty() || spans.is_empty() {
        return Vec::new();
    }

    // 1. Identify touched spans using line vertical band tolerance (±BAND_TOL)
    // and segment crossing logic, exactly as before.
    let mut touched: Vec<&crate::engine::TextSpan> = Vec::new();
    for span in spans {
        let y0 = span.y - BAND_TOL;
        let y1 = span.y + span.h + BAND_TOL;
        let within = |p: &(f32, f32)| p.1 >= y0 && p.1 <= y1;
        let mut x_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        for p in pts {
            if within(p) {
                x_min = x_min.min(p.0);
                x_max = x_max.max(p.0);
            }
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let a_in = within(&a);
            let b_in = within(&b);
            let crosses = (a.1 < y0 && b.1 > y1) || (a.1 > y1 && b.1 < y0);
            if a_in || b_in || crosses {
                x_min = x_min.min(a.0.min(b.0));
                x_max = x_max.max(a.0.max(b.0));
            }
        }
        if !x_min.is_finite() {
            continue;
        }
        let eff_x_max = if x_max - x_min < MIN_STROKE_SPAN {
            x_min + MIN_STROKE_SPAN
        } else {
            x_max
        };
        let x0 = span.x.max(x_min);
        let x1 = (span.x + span.w).min(eff_x_max);
        if x1 - x0 > 0.0 {
            touched.push(span);
        }
    }

    if touched.is_empty() {
        return Vec::new();
    }

    // 2. Sort touched spans by document order (y ascending, tie-break by x).
    touched.sort_by(|a, b| {
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let p_down = pts[0];
    let p_up = pts[pts.len() - 1];

    // 3. Map pen-down and pen-up to touched spans: span containing py,
    // or nearest touched span in Y if outside all touched spans.
    let find_span_idx = |p: (f32, f32)| -> usize {
        let (px, py) = p;
        let mut best_idx = 0;
        let mut best_dy = f32::INFINITY;
        let mut best_dx = f32::INFINITY;
        for (idx, span) in touched.iter().enumerate() {
            let dy = if py < span.y {
                span.y - py
            } else if py > span.y + span.h {
                py - (span.y + span.h)
            } else {
                0.0
            };
            let dx = if px < span.x {
                span.x - px
            } else if px > span.x + span.w {
                px - (span.x + span.w)
            } else {
                0.0
            };
            if dy < best_dy || (dy == best_dy && dx < best_dx) {
                best_dy = dy;
                best_dx = dx;
                best_idx = idx;
            }
        }
        best_idx
    };

    let down_idx = find_span_idx(p_down);
    let up_idx = find_span_idx(p_up);

    let mut out = Vec::new();

    if down_idx == up_idx {
        // Both endpoints fall on the same line: [min, max] of both endpoints' X.
        let span = touched[down_idx];
        let x_min = p_down.0.min(p_up.0);
        let mut x_max = p_down.0.max(p_up.0);
        if x_max - x_min < MIN_STROKE_SPAN {
            x_max = x_min + MIN_STROKE_SPAN;
        }
        let x0 = span.x.max(x_min);
        let x1 = (span.x + span.w).min(x_max);
        if x1 - x0 > 0.0 {
            out.push(Rect::new(x0, span.y, x1 - x0, span.h));
        }
    } else {
        // Multi-line selection: top line from extreme to line end,
        // intermediate lines full, bottom line from line start to extreme.
        let sup_idx = down_idx.min(up_idx);
        let inf_idx = down_idx.max(up_idx);
        let x_sup = if down_idx < up_idx { p_down.0 } else { p_up.0 };
        let x_inf = if down_idx < up_idx { p_up.0 } else { p_down.0 };

        let total = inf_idx - sup_idx;
        for (offset, &span) in touched[sup_idx..=inf_idx].iter().enumerate() {
            let (x_min, mut x_max) = if offset == 0 {
                (x_sup, span.x + span.w)
            } else if offset == total {
                (span.x, x_inf)
            } else {
                (span.x, span.x + span.w)
            };

            if x_max >= x_min {
                if x_max - x_min < MIN_STROKE_SPAN {
                    x_max = x_min + MIN_STROKE_SPAN;
                }
                let x0 = span.x.max(x_min);
                let x1 = (span.x + span.w).min(x_max);
                if x1 - x0 > 0.0 {
                    out.push(Rect::new(x0, span.y, x1 - x0, span.h));
                }
            }
        }
    }

    out
}

/// Block-marquee matcher over a candidate slice (shared by both paths).
fn match_rect(
    spans: &[crate::engine::TextSpan],
    x_min: f32,
    x_max: f32,
    y0: f32,
    y1: f32,
) -> Vec<Rect> {
    let mut out = Vec::new();
    for span in spans {
        if span.x + span.w <= x_min || span.x >= x_max || span.y + span.h <= y0 || span.y >= y1 {
            continue;
        }
        let x0 = span.x.max(x_min);
        let x1 = (span.x + span.w).min(x_max);
        if x1 - x0 > 0.0 {
            out.push(Rect::new(x0, span.y, x1 - x0, span.h));
        }
    }
    out
}

/// Tolerancia vertical de la banda de una línea para el trazo del rotulador
/// (pt): un deslizamiento casi perfectamente horizontal sigue marcando su
/// línea (fue el `MIN_GESTURE_H` de la iteración anterior).
const BAND_TOL: f32 = 1.0;
/// Tramo mínimo marcado por línea (pt): un trazo que roza una línea deja
/// marca visible en vez de un rect de ancho 0.
const MIN_STROKE_SPAN: f32 = 3.0;

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
        // Under reading order: top line extends from pen-down to line end,
        // bottom line extends from line start to pen-up.
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 25.0), (60.0, 41.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("lines under the stroke");
        assert_eq!(hl.rects.len(), 2);
        // Line 1 (x 10..100): pen-down at x=20 to line end (x=100) -> w=80.
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 80.0, 12.0));
        // Line 2 (x 10..90): line start (x=10) to pen-up at x=60 -> w=50.
        assert_eq!(hl.rects[1], Rect::new(10.0, 34.0, 50.0, 12.0));
        assert_eq!(hl.color, HIGHLIGHT_COLOR);
    }

    #[test]
    fn stroke_going_down_marks_lines_it_joins() {
        // Stroke crossing lines 1, 2, and 3: top line from pen-down to line end,
        // intermediate line complete, bottom line from line start to pen-up.
        let gesture = Gesture::Points(vec![(20.0, 25.0), (70.0, 25.0), (70.0, 41.0), (40.0, 55.0)]);
        let hl =
            highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR).expect("joined lines");
        assert_eq!(hl.rects.len(), 3);
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 80.0, 12.0)); // line 1: 20..100
        assert_eq!(hl.rects[1], Rect::new(10.0, 34.0, 80.0, 12.0)); // line 2: full line 10..90
        assert_eq!(hl.rects[2], Rect::new(10.0, 48.0, 30.0, 12.0)); // line 3: 10..40
    }

    #[test]
    fn diagonal_mitad_a_mitad_en_2_lineas() {
        // Diagonal from middle of line 1 (x=50) to middle of line 2 (x=50):
        // line 1 from x=50 to line end (x=100); line 2 from line start (x=10) to x=50.
        let gesture = Gesture::Points(vec![(50.0, 25.0), (50.0, 40.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("two lines selected");
        assert_eq!(hl.rects.len(), 2);
        assert_eq!(hl.rects[0], Rect::new(50.0, 20.0, 50.0, 12.0));
        assert_eq!(hl.rects[1], Rect::new(10.0, 34.0, 40.0, 12.0));
    }

    #[test]
    fn gesto_abajo_arriba_espejado() {
        // Same diagonal gesture drawn bottom-to-top produces the exact same highlight rects.
        let gesture_down = Gesture::Points(vec![(50.0, 25.0), (50.0, 40.0)]);
        let gesture_up = Gesture::Points(vec![(50.0, 40.0), (50.0, 25.0)]);
        let hl_down = highlight_under_gesture(&spans(), &gesture_down, HIGHLIGHT_COLOR)
            .expect("down gesture");
        let hl_up =
            highlight_under_gesture(&spans(), &gesture_up, HIGHLIGHT_COLOR).expect("up gesture");
        assert_eq!(hl_up.rects, hl_down.rects);
    }

    #[test]
    fn tres_lineas_intermedia_completa() {
        // Gesture crossing lines 1, 2, and 3:
        // top line from pen-down (x=30) to line end (x=100),
        // intermediate line 2 completely highlighted (x=10..90),
        // bottom line 3 from line start (x=10) to pen-up (x=60).
        let gesture = Gesture::Points(vec![(30.0, 25.0), (50.0, 40.0), (60.0, 55.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("three lines selected");
        assert_eq!(hl.rects.len(), 3);
        assert_eq!(hl.rects[0], Rect::new(30.0, 20.0, 70.0, 12.0));
        assert_eq!(hl.rects[1], Rect::new(10.0, 34.0, 80.0, 12.0)); // full intermediate line
        assert_eq!(hl.rects[2], Rect::new(10.0, 48.0, 50.0, 12.0));
    }

    #[test]
    fn una_sola_linea_sin_cambios() {
        // Stroke on a single line: [min, max] of both endpoints clipped to line bbox.
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 25.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("single line selected");
        assert_eq!(hl.rects, vec![Rect::new(20.0, 20.0, 40.0, 12.0)]);
    }

    #[test]
    fn suelta_en_gutter_nearest() {
        // Stroke starts on line 1 at (20, 25), crosses line 2, and pen-up is released
        // in the gutter below line 2 at (60, 46.5) (line 2 y is 34..46).
        // Nearest touched line in Y is line 2.
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 40.0), (60.0, 46.5)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("lines under stroke");
        assert_eq!(hl.rects.len(), 2);
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 80.0, 12.0));
        assert_eq!(hl.rects[1], Rect::new(10.0, 34.0, 50.0, 12.0));
    }

    #[test]
    fn two_columns_do_not_join_across_the_gutter() {
        // Paper científico de DOS COLUMNAS: subrayar la línea de la columna
        // izquierda NO arrastra a las líneas de la columna derecha aunque el
        // bbox del gesto (y) las abarque: el rotulador solo marca lo que toca.
        let cols = vec![
            line("izq1", 30.0, 20.0, 180.0, 12.0),
            line("izq2", 30.0, 34.0, 180.0, 12.0),
            line("izq3", 30.0, 48.0, 180.0, 12.0),
            line("der1", 500.0, 20.0, 180.0, 12.0),
            line("der2", 500.0, 34.0, 180.0, 12.0),
            line("der3", 500.0, 48.0, 180.0, 12.0),
        ];
        // Trazo dentro de la columna izquierda, bajando de la línea 1 a la 3.
        let gesture = Gesture::Points(vec![
            (40.0, 25.0),
            (190.0, 25.0),
            (190.0, 41.0),
            (50.0, 55.0),
        ]);
        let hl =
            highlight_under_gesture(&cols, &gesture, HIGHLIGHT_COLOR).expect("left column lines");
        assert_eq!(hl.rects.len(), 3, "solo la columna izquierda");
        assert!(hl.rects.iter().all(|r| r.x < 500.0), "nada en la derecha");
        // La derecha se marca SOLO si el trazo llega hasta su x: mismo gesto
        // pero terminando dentro de la columna derecha.
        let gesture2 = Gesture::Points(vec![(40.0, 25.0), (190.0, 25.0), (560.0, 55.0)]);
        let hl2 = highlight_under_gesture(&cols, &gesture2, HIGHLIGHT_COLOR)
            .expect("reaches right column");
        assert!(
            hl2.rects.iter().any(|r| r.x >= 500.0),
            "la derecha sí al llegar"
        );
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
