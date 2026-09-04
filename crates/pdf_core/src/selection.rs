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
/// **Trazo (Points)** — comportamiento de ROTULADOR REAL (optimización
/// 2026-08-23, petición del autor): cada línea se subraya SOLO si el trazo
/// pasa por su banda vertical (tolerancia ±[`BAND_TOL`] pt) y el tramo
/// marcado es el recorrido X del trazo dentro de la línea, clavado al bbox
/// de la línea. Con esto:
///   - "pasarse a la siguiente línea subraya todo lo que une": un trazo que
///     baja de una línea a la siguiente marca AMBAS (y las intermedias por
///     las que pasa la tinta del rotulador);
///   - en papers de DOS COLUMNAS no se "une" ni se pinta el gutter: una
///     línea de la otra columna solo se marca si el trazo llega hasta su x
///     (el clip por el bbox de la línea lo garantiza).
///
/// Un roce mínimo deja marca visible ([`MIN_STROKE_SPAN`] pt).
///
/// **Marquee (Rect)** — selección de BLOQUE: líneas cuyo bbox intersecta el
/// rect, recortadas al tramo horizontal (semántica de abarcar, no de
/// rotulador).
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
    let mut out = Vec::new();
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
        let x_max = if x_max - x_min < MIN_STROKE_SPAN {
            x_min + MIN_STROKE_SPAN
        } else {
            x_max
        };
        let x0 = span.x.max(x_min);
        let x1 = (span.x + span.w).min(x_max);
        if x1 - x0 > 0.0 {
            out.push(Rect::new(x0, span.y, x1 - x0, span.h));
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
        let gesture = Gesture::Points(vec![(20.0, 25.0), (60.0, 25.0), (60.0, 41.0)]);
        let hl = highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR)
            .expect("lines under the stroke");
        assert_eq!(hl.rects.len(), 2);
        // Line 1 (x 10..100): clipped to the stroke's horizontal extent.
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 40.0, 12.0));
        // Line 2 (x 10..90): el trazo solo la toca en su borde (x=60): el
        // tramo marcado es el tramo mínimo (3 pt) donde la rozó, no todo el
        // rango X del gesto (semántica de rotulador real, 2026-08-23).
        assert_eq!(hl.rects[1], Rect::new(60.0, 34.0, 3.0, 12.0));
        assert_eq!(hl.color, HIGHLIGHT_COLOR);
    }

    #[test]
    fn stroke_going_down_marks_lines_it_joins() {
        // "Pasar a la siguiente línea subraya todo lo que une": un trazo que
        // baja de la línea 1 a la 3 diagonalmente marca las 3 líneas (las
        // que la tinta del rotulador toca en su recorrido).
        let gesture = Gesture::Points(vec![(20.0, 25.0), (70.0, 25.0), (70.0, 41.0), (40.0, 55.0)]);
        let hl =
            highlight_under_gesture(&spans(), &gesture, HIGHLIGHT_COLOR).expect("joined lines");
        assert_eq!(hl.rects.len(), 3);
        assert_eq!(hl.rects[0], Rect::new(20.0, 20.0, 50.0, 12.0));
        assert_eq!(hl.rects[1], Rect::new(40.0, 34.0, 30.0, 12.0)); // trazo x 40..70
        assert_eq!(hl.rects[2], Rect::new(40.0, 48.0, 30.0, 12.0)); // trazo x 40..70
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
