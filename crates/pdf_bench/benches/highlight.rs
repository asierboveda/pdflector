// SPDX-License-Identifier: AGPL-3.0-or-later
// Highlight hot path (Fase B): gesture -> rects alineados al texto.
// Mide el cuello de botella auditado: O(spans x puntos) + clip por línea.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use pdf_core::engine::TextSpan;
use pdf_core::{
    Color, Gesture, Rect, highlight_under_gesture, highlight_under_gesture_sorted, sort_spans_by_y,
};

const HIGHLIGHT_COLOR: Color = Color {
    r: 255,
    g: 240,
    b: 0,
    a: 128,
};
const N_LINES: &[usize] = &[20, 100, 200];
const N_POINTS: &[usize] = &[10, 50, 100];

fn spans(n: usize) -> Vec<TextSpan> {
    (0..n)
        .map(|i| TextSpan {
            text: format!("línea {i} con texto para medir highlight"),
            x: 30.0,
            y: 20.0 + i as f32 * 14.0,
            w: 500.0,
            h: 12.0,
        })
        .collect()
}

fn gesture_points(n: usize) -> Vec<(f32, f32)> {
    // Trazo diagonal que cruza ~30% de las líneas
    (0..n)
        .map(|i| {
            let t = i as f32 / n.max(1) as f32;
            (
                40.0 + t * 400.0,
                25.0 + t * (N_LINES[2] as f32 * 14.0 * 0.3),
            )
        })
        .collect()
}

fn bench_highlight(c: &mut Criterion) {
    let mut g = c.benchmark_group("highlight");
    for &lines in N_LINES {
        let sp = spans(lines);
        for &pts in N_POINTS {
            let gesture = Gesture::Points(gesture_points(pts));
            g.throughput(Throughput::Elements((lines * pts) as u64));
            g.bench_function(format!("gesture_pts{pts}_lines{lines}"), |b| {
                b.iter(|| {
                    let hl = highlight_under_gesture(&sp, &gesture, HIGHLIGHT_COLOR);
                    black_box(hl.map(|h| h.rects.len()));
                });
            });
        }
    }
    // Caso 2 columnas: 200 líneas (100 izq + 100 dcha), gesto solo izq
    let cols: Vec<TextSpan> = (0..100)
        .flat_map(|i| {
            vec![
                TextSpan {
                    text: "izq".into(),
                    x: 30.0,
                    y: 20.0 + i as f32 * 14.0,
                    w: 180.0,
                    h: 12.0,
                },
                TextSpan {
                    text: "der".into(),
                    x: 500.0,
                    y: 20.0 + i as f32 * 14.0,
                    w: 180.0,
                    h: 12.0,
                },
            ]
        })
        .collect();
    let g2 = Gesture::Points(gesture_points(50));
    g.bench_function("two_cols_200_lines_pts50", |b| {
        b.iter(|| {
            black_box(highlight_under_gesture(&cols, &g2, HIGHLIGHT_COLOR).map(|h| h.rects.len()))
        });
    });
    // Marquee rect (selección bloque)
    let sp = spans(200);
    let rect = Gesture::Rect(Rect::new(30.0, 20.0, 200.0, 100.0));
    g.bench_function("marquee_200_lines", |b| {
        b.iter(|| {
            black_box(highlight_under_gesture(&sp, &rect, HIGHLIGHT_COLOR).map(|h| h.rects.len()))
        });
    });
    // Fase B2: camino indexado en Y (ordenado UNA vez fuera del loop, como
    // el hilo UI tras `PageTextCache::prefetch`; el gesto solo busca).
    let mut sorted200 = spans(200);
    sort_spans_by_y(&mut sorted200);
    let gesture100 = Gesture::Points(gesture_points(100));
    g.throughput(Throughput::Elements(200 * 100));
    g.bench_function("sorted_pts100_lines200", |b| {
        b.iter(|| {
            black_box(
                highlight_under_gesture_sorted(&sorted200, &gesture100, HIGHLIGHT_COLOR)
                    .map(|h| h.rects.len()),
            )
        });
    });
    g.bench_function("sorted_marquee_200_lines", |b| {
        b.iter(|| {
            black_box(
                highlight_under_gesture_sorted(&sorted200, &rect, HIGHLIGHT_COLOR)
                    .map(|h| h.rects.len()),
            )
        });
    });
}

criterion_group!(all, bench_highlight);
criterion_main!(all);
