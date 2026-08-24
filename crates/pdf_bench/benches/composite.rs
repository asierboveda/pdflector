// SPDX-License-Identifier: AGPL-3.0-or-later
// Composite annotation layer (Fase C): overlay vectorial sobre bitmap.
// Mide fill_rect (highlights) + draw_stroke (trazos) a 1440x2200 (TCL).

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use pdf_core::{
    Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, ViewTransform, composite_annotations,
};

const W: u32 = 1440;
const H: u32 = 2200;

fn bitmap(w: u32, h: u32) -> Vec<u8> {
    vec![0xFF; (w * h * 4) as usize]
}

fn bench_composite(c: &mut Criterion) {
    // We need &[&Annotated] - build set and leak refs via owned vec
    let cases = [(10, 0), (50, 10), (200, 0), (100, 100)];
    let mut g = c.benchmark_group("composite");
    for (strokes, hls) in cases {
        let mut set = AnnotationSet::new();
        for i in 0..strokes {
            let y = 20.0 + (i % 60) as f32 * 30.0;
            let s = Stroke::new(
                vec![(30.0, y), (400.0, y + 4.0)],
                2.5,
                Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 200,
                },
            )
            .unwrap();
            set.add(0, Annotation::Stroke(s));
        }
        for i in 0..hls {
            let y = 20.0 + (i % 60) as f32 * 14.0;
            set.add(
                0,
                Annotation::Highlight(Highlight {
                    rects: vec![Rect::new(30.0, y, 180.0, 12.0)],
                    color: Color {
                        r: 255,
                        g: 240,
                        b: 0,
                        a: 128,
                    },
                }),
            );
        }
        let anns: Vec<pdf_core::Annotated> = set.for_page(0).into_iter().cloned().collect();
        let total = strokes + hls;
        g.throughput(Throughput::Elements(total as u64));
        g.bench_function(format!("strokes{strokes}_hl{hls}_{W}x{H}"), |b| {
            b.iter(|| {
                let mut buf = bitmap(W, H);
                let refs: Vec<&pdf_core::Annotated> = anns.iter().collect();
                composite_annotations(&mut buf, W, H, &refs, &ViewTransform::IDENTITY);
                black_box(buf[0]);
            });
        });
    }
    g.finish();
}

criterion_group!(all, bench_composite);
criterion_main!(all);
