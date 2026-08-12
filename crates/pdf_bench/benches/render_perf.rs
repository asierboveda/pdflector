//! Render perf benchmark for the sole MuPDF backend (ADR-001).
//! Compares the 4-PDF corpus at 1× and 2× on page 1 and the middle page.
//! Run with:
//!     cargo bench -p pdf_bench --bench render_perf
//! Results land in target/criterion/<group>/report/ and are also written
//! verbatim to stdout for easy grep / paste.

use std::path::PathBuf;
use std::time::Instant;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use pdf_core::{Document, RenderEngine};

use pdf_core::engine::mupdf::MupdfEngine;

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("corpus")
}

struct Case {
    file: &'static str,
    label: &'static str,
    mid: u32, // 0-based middle page index
}

fn corpus() -> Vec<Case> {
    vec![
        Case {
            file: "scientific_paper.pdf",
            label: "paper_12p",
            mid: 5,
        },
        Case {
            file: "scanned_pages.pdf",
            label: "scanned_30p",
            mid: 14,
        },
        Case {
            file: "dense_textbook.pdf",
            label: "dense_93p",
            mid: 46,
        },
        Case {
            file: "large_document.pdf",
            label: "large_500p",
            mid: 249,
        },
    ]
}

fn pdf_path(file: &str) -> PathBuf {
    corpus_root().join(file)
}

fn bench_open(c: &mut Criterion) {
    let mut g = c.benchmark_group("open/mupdf");
    for case in corpus() {
        let path = pdf_path(case.file);
        g.bench_with_input(BenchmarkId::from_parameter(case.label), &path, |b, p| {
            b.iter(|| {
                let engine = MupdfEngine::new().expect("init mupdf engine");
                let doc = black_box(engine.open(p).unwrap());
                black_box(doc.page_count())
            });
        });
    }
    g.finish();
}

fn bench_render(c: &mut Criterion) {
    let mut g = c.benchmark_group("render/mupdf");
    for case in corpus() {
        let path = pdf_path(case.file);
        let engine = MupdfEngine::new().expect("init mupdf engine");
        let doc = engine.open(&path).unwrap();

        for (scale_label, scale) in [("1x", 1.0_f32), ("2x", 2.0)] {
            for (page_label, page) in [("p1", 0_u32), ("pmid", case.mid)] {
                let id = format!("{}/{}/s{}", case.label, page_label, scale_label);
                g.throughput(Throughput::Elements(1));
                g.bench_with_input(
                    BenchmarkId::from_parameter(id.clone()),
                    &(page, scale),
                    |b, &(p, s)| {
                        b.iter(|| {
                            let bmp = black_box(doc.render_page(p, s).unwrap());
                            black_box(bmp.data.len())
                        });
                    },
                );
            }
        }
    }
    g.finish();
}

fn benches(c: &mut Criterion) {
    bench_open(c);
    bench_render(c);
}

criterion_group!(all, benches);
criterion_main!(all);

// Touched at startup so `--list` succeeds even before any measurement.
#[allow(dead_code)]
fn _startup_marker() -> Instant {
    Instant::now()
}
