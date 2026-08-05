//! Engine benchmark harness (criterion) — MuPDF is the only engine since
//! ADR-001 (Fase 0.5), so `build_engine` is the sole customization point.
//!
//! Benchmark groups:
//!   open      — cost of opening each corpus PDF (4 documents).
//!   render_1x — render page 0 of "dense" and "large" at scale 1.0 (72 dpi).
//!   render_2x — same pages at scale 2.0 (144 dpi).

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};
use std::path::PathBuf;

/// Engine under test: MuPDF (ADR-001).
fn build_engine() -> MupdfEngine {
    MupdfEngine::new().expect("failed to init mupdf")
}

/// Engine label for benchmark IDs (single engine: "mupdf").
const ENGINE_LABEL: &str = "mupdf";

/// All corpus documents with a stable label, relative to the workspace root.
fn corpus_files() -> Vec<(&'static str, PathBuf)> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    vec![
        ("dense", base.join("dense_textbook.pdf")),
        ("scanned", base.join("scanned_pages.pdf")),
        ("paper", base.join("scientific_paper.pdf")),
        ("large", base.join("large_document.pdf")),
    ]
}

fn bench_open(c: &mut Criterion) {
    let mut group = c.benchmark_group("open");
    for (label, path) in corpus_files() {
        group.bench_with_input(BenchmarkId::new(ENGINE_LABEL, label), &path, |b, path| {
            // Re-create the engine per sample so the *open* cost is what is
            // measured (creation is cheap: MuPDF context clones per thread).
            b.iter_with_setup(build_engine, |eng| {
                std::hint::black_box(eng.open(path).unwrap())
            });
        });
    }
    group.finish();
}

fn bench_render(c: &mut Criterion, scale: f32, group_name: &str) {
    let engine = build_engine();
    let mut group = c.benchmark_group(group_name);
    for (label, path) in corpus_files()
        .iter()
        .filter(|(l, _)| *l == "dense" || *l == "large")
    {
        group.bench_with_input(BenchmarkId::new(ENGINE_LABEL, label), path, |b, path| {
            // Open the document in setup (not timed); only the render is measured.
            b.iter_with_setup(
                || engine.open(path).expect("open document"),
                |doc| std::hint::black_box(doc.render_page(0, scale).unwrap()),
            );
        });
    }
    group.finish();
}

fn bench_render_1x(c: &mut Criterion) {
    bench_render(c, 1.0, "render_1x");
}

fn bench_render_2x(c: &mut Criterion) {
    bench_render(c, 2.0, "render_2x");
}

criterion_group!(benches, bench_open, bench_render_1x, bench_render_2x);
criterion_main!(benches);
