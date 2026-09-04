// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Fase B2: el camino indexado en Y (`highlight_under_gesture_sorted`, con
//! spans pre-ordenados por `sort_spans_by_y`) debe devolver EXACTAMENTE lo
//! mismo que el oráculo lineal (`highlight_under_gesture`) en cualquier
//! layout y gesto. Sintéticos, sin motor.

use pdf_core::engine::TextSpan;
use pdf_core::{
    HIGHLIGHT_COLOR, Rect, highlight_under_gesture, highlight_under_gesture_sorted, sort_spans_by_y,
};

fn line(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextSpan {
    TextSpan {
        text: text.to_string(),
        x,
        y,
        w,
        h,
    }
}

fn one_col() -> Vec<TextSpan> {
    vec![
        line("primera", 10.0, 20.0, 90.0, 12.0),
        line("segunda", 10.0, 34.0, 80.0, 12.0),
        line("tercera", 10.0, 48.0, 100.0, 12.0),
    ]
}

fn two_cols() -> Vec<TextSpan> {
    (0..3)
        .flat_map(|i| {
            let y = 20.0 + i as f32 * 14.0;
            vec![
                line("izq", 30.0, y, 180.0, 12.0),
                line("der", 500.0, y, 180.0, 12.0),
            ]
        })
        .collect()
}

fn dense_page(n: usize) -> Vec<TextSpan> {
    (0..n)
        .map(|i| line("línea", 30.0, 20.0 + i as f32 * 14.0, 500.0, 12.0))
        .collect()
}

fn with_headline() -> Vec<TextSpan> {
    // Titular alto (h=40) seguido de líneas normales: el span alto empieza
    // MUY por encima de la banda del gesto pero la intersecta (caso que la
    // búsqueda binaria ingenua sobre `y` solo perdería).
    vec![
        line("TITULAR", 30.0, 20.0, 400.0, 40.0),
        line("segunda", 30.0, 64.0, 400.0, 12.0),
        line("tercera", 30.0, 78.0, 400.0, 12.0),
    ]
}

fn rects_of(
    f: fn(&[TextSpan], &pdf_core::Gesture, pdf_core::Color) -> Option<pdf_core::Highlight>,
    spans: &[TextSpan],
    g: &pdf_core::Gesture,
) -> Option<Vec<Rect>> {
    f(spans, g, HIGHLIGHT_COLOR).map(|h| h.rects)
}

fn check_equivalence(spans: &[TextSpan], gestures: &[pdf_core::Gesture]) {
    let mut sorted = spans.to_vec();
    sort_spans_by_y(&mut sorted);
    for g in gestures {
        assert_eq!(
            rects_of(highlight_under_gesture_sorted, &sorted, g),
            rects_of(highlight_under_gesture, spans, g),
            "sorted path must match the linear oracle"
        );
    }
}

#[test]
fn sorted_path_matches_oracle_on_layouts_and_gestures() {
    use pdf_core::Gesture;
    let diagonal = Gesture::Points(vec![(20.0, 25.0), (70.0, 25.0), (70.0, 41.0), (40.0, 55.0)]);
    let gap = Gesture::Points(vec![(10.0, 15.0), (50.0, 15.0)]);
    let empty_pts = Gesture::Points(vec![]);
    let marquee = Gesture::Rect(Rect::new(30.0, 20.0, 200.0, 100.0));
    let marquee_neg = Gesture::Rect(Rect::new(60.0, 20.0, -30.0, 10.0));
    let gestures = [diagonal, gap, empty_pts, marquee, marquee_neg];
    check_equivalence(&one_col(), &gestures);
    check_equivalence(&two_cols(), &gestures);
    check_equivalence(&with_headline(), &gestures);
    check_equivalence(&[], &gestures);
}

#[test]
fn sorted_path_matches_oracle_on_dense_page_with_long_gesture() {
    use pdf_core::Gesture;
    let spans = dense_page(200);
    let pts: Vec<(f32, f32)> = (0..100)
        .map(|i| {
            let t = i as f32 / 100.0;
            (40.0 + t * 400.0, 25.0 + t * (200.0 * 14.0 * 0.3))
        })
        .collect();
    check_equivalence(
        &spans,
        &[
            Gesture::Points(pts),
            Gesture::Rect(Rect::new(30.0, 20.0, 200.0, 100.0)),
        ],
    );
}

#[test]
fn sort_spans_by_y_orders_by_top_edge_stably() {
    let mut spans = vec![
        line("c", 0.0, 48.0, 10.0, 12.0),
        line("a", 0.0, 20.0, 10.0, 12.0),
        line("b", 0.0, 34.0, 10.0, 12.0),
    ];
    sort_spans_by_y(&mut spans);
    let ys: Vec<f32> = spans.iter().map(|s| s.y).collect();
    assert_eq!(ys, vec![20.0, 34.0, 48.0]);
}

#[test]
fn sorted_path_finds_tall_span_starting_above_the_band() {
    use pdf_core::Gesture;
    // Trazo horizontal a y=70: solo lo toca el titular (20..60)? No:
    // banda = 70±1 → [69,71]; titular acaba en 60 → NO toca. La que toca es
    // "segunda" (64..76). El oráculo manda; el indexado debe coincidir.
    let spans = with_headline();
    let mut sorted = spans.clone();
    sort_spans_by_y(&mut sorted);
    let g = Gesture::Points(vec![(40.0, 70.0), (300.0, 70.0)]);
    assert_eq!(
        rects_of(highlight_under_gesture_sorted, &sorted, &g),
        rects_of(highlight_under_gesture, &spans, &g)
    );
    // Y trazo a y=55 (dentro del titular 20..60 + tol): debe marcarlo aunque
    // empiece 35pt por encima de la banda.
    let g2 = Gesture::Points(vec![(40.0, 55.0), (300.0, 55.0)]);
    let r = rects_of(highlight_under_gesture_sorted, &sorted, &g2).expect("headline must match");
    assert_eq!(r, vec![Rect::new(40.0, 20.0, 260.0, 40.0)]);
}
