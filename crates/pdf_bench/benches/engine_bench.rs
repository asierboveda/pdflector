//! Render pipeline benchmarks (docs/PLAN.md): open and render-page timings
//! for the MuPDF engine over the 4-PDF corpus, page 1 and middle page, at
//! 1x (72 dpi) and 2x (144 dpi). RSS peak is measured by the `pdf_bench`
//! binary (main.rs) in a clean process via /proc/self/status.
//! Reference numbers: docs/investigacion/evince-baseline.md (poppler) and
//! docs/investigacion/benchmark-motores.md (engine shootout, ADR-001).
//! Run: `cargo bench -p pdf_bench`

use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};

const PDFS: [&str; 4] = [
    "scientific_paper.pdf",
    "scanned_pages.pdf",
    "dense_textbook.pdf",
    "large_document.pdf",
];
const SCALES: [f32; 2] = [1.0, 2.0];

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn bench_open(c: &mut Criterion) {
    let engine = MupdfEngine::new();
    let mut group = c.benchmark_group("open");
    group.sample_size(20);
    for pdf in PDFS {
        let path = corpus_dir().join(pdf);
        group.bench_with_input(BenchmarkId::new("mupdf", pdf), &path, |b, p| {
            b.iter(|| engine.open(p).unwrap());
        });
    }
    group.finish();
}

fn bench_render(c: &mut Criterion) {
    let engine = MupdfEngine::new();
    let mut group = c.benchmark_group("render");
    group.sample_size(10);
    for pdf in PDFS {
        let path = corpus_dir().join(pdf);
        let doc = engine.open(&path).unwrap();
        let mid = doc.page_count() / 2;
        for page in [0, mid] {
            for scale in SCALES {
                let id = BenchmarkId::new("mupdf", format!("{pdf}/p{page}@{scale}x"));
                group.bench_with_input(id, &scale, |b, &s| {
                    b.iter(|| doc.render_page(page, s).unwrap());
                });
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_open, bench_render
}

criterion_main!(benches);
