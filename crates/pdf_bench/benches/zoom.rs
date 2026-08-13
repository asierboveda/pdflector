//! Fase 1 B3 zoom benchmark: cost of the two zoom paths on a rendered page of
//! large_document.pdf (page 0), plus the cache-invalidation cost.
//!
//! zoom/scale_bitmap         — fast path: pure software `scale_bitmap` of the
//!                             already-rendered level-0 bitmap up to the
//!                             target sizes of ×1.5, ×2, ×4 (µs of scaling,
//!                             no engine work; the re-render is not timed).
//! zoom/rerender             — sharp path: full engine re-render of the same
//!                             page at ladder levels 1 and 2
//!                             (`scale_for_level` = ×2 and ×4), the crisp
//!                             path that replaces the scaled bitmap; compare
//!                             against zoom/scale_bitmap.
//! zoom/trim_to_scale_level  — cost of invalidating the stale ladder level of
//!                             a cache populated at levels 0 and 1, the
//!                             budget cleanup the UI pays on every zoom
//!                             change (B3 flow).
//!
//! Run with:
//!     cargo bench -p pdf_bench --bench zoom
//! Results land in target/criterion/<group>/report/.

use std::path::PathBuf;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use pdf_core::cache::RenderCache;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{
    Document, RenderEngine, corpus_dir, scale_bitmap, scale_for_level, scale_level_for_zoom,
};

/// large_document.pdf page 0: the page every group scales / re-renders.
const PAGE: u32 = 0;
/// Ladder level of the cached base bitmap (72 dpi baseline).
const LEVEL_BASE: u32 = 0;
/// Pages populated at each level in the trim benchmark.
const TRIM_PAGES: usize = 10;

fn large_path() -> PathBuf {
    // Directory resolution is shared with pdf_core (`corpus_dir`).
    corpus_dir().join("large_document.pdf")
}

fn build_engine() -> MupdfEngine {
    MupdfEngine::new().expect("failed to init mupdf")
}

/// Fast path: `scale_bitmap` of the level-0 bitmap up to the target sizes of
/// several zooms. The base render happens once, outside the timed loop; the
/// iteration measures only the software bilinear scaling. The benchmark id
/// records which ladder level the sharp path would re-render for that zoom
/// (`scale_level_for_zoom`: ×1.5 and ×2 → level 1, ×4 → level 2).
fn bench_scale_bitmap(c: &mut Criterion) {
    let engine = build_engine();
    let doc = engine.open(&large_path()).expect("open document");
    let base = doc
        .render_page(PAGE, scale_for_level(LEVEL_BASE))
        .expect("render base page");

    let mut g = c.benchmark_group("zoom/scale_bitmap");
    for (zoom, label) in [(1.5_f32, "z1.5"), (2.0, "z2"), (4.0, "z4")] {
        // Target sizes the app would ask for at this zoom: base × factor.
        let (tw, th) = (
            (base.width as f32 * zoom).round() as u32,
            (base.height as f32 * zoom).round() as u32,
        );
        let sharp_level = scale_level_for_zoom(zoom);
        g.throughput(Throughput::Elements(1));
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("{label}_vs_level{sharp_level}")),
            &(tw, th),
            |b, &(tw, th)| {
                b.iter(|| {
                    let out = scale_bitmap(&base, tw, th).expect("scale");
                    black_box(out.data.len())
                });
            },
        );
    }
    g.finish();
}

/// Sharp path: full engine re-render of the same page at the ladder levels
/// the cache would use after a zoom change (`scale_for_level(1)` = ×2,
/// `scale_for_level(2)` = ×4). Same page as the fast path so the two groups
/// are directly comparable.
fn bench_rerender(c: &mut Criterion) {
    let engine = build_engine();
    let doc = engine.open(&large_path()).expect("open document");
    let mut g = c.benchmark_group("zoom/rerender");
    for level in [1_u32, 2] {
        let scale = scale_for_level(level);
        g.throughput(Throughput::Elements(1));
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("level{level}_x{}", scale as u32)),
            &(PAGE, scale),
            |b, &(p, s)| {
                b.iter(|| {
                    let bmp = doc.render_page(p, s).expect("render");
                    black_box(bmp.data.len())
                });
            },
        );
    }
    g.finish();
}

/// Bytes a single page occupies at `level` (RGBA8, real rendered dimensions).
fn page_bytes_at(level: u32) -> usize {
    let engine = build_engine();
    let doc = engine.open(&large_path()).expect("open document");
    let bmp = doc
        .render_page(PAGE, scale_for_level(level))
        .expect("render for sizing");
    bmp.width as usize * bmp.height as usize * 4
}

/// Cost of `trim_to_scale_level`: dropping the stale ladder level from a
/// populated cache (level 0 after a zoom to level 1). The budget is sized
/// dynamically from the real page bytes so both levels fit without LRU
/// evictions during setup; the measured body is only the trim itself.
fn bench_trim(c: &mut Criterion) {
    let bytes_per_page = page_bytes_at(0) + page_bytes_at(1);
    let budget = bytes_per_page * TRIM_PAGES;

    let mut g = c.benchmark_group("zoom/trim_to_scale_level");
    // Setup re-renders 2 × TRIM_PAGES pages per sample: keep the sample count
    // low so the bench stays quick (the measured body is a single trim).
    g.sample_size(15);
    g.warm_up_time(Duration::from_millis(500));
    g.measurement_time(Duration::from_secs(3));
    g.bench_function("drop_level0_keep_level1", |b| {
        b.iter_with_setup(
            || {
                let mut cache =
                    RenderCache::open(build_engine(), &large_path(), budget).expect("open cache");
                for page in 0..TRIM_PAGES {
                    cache.get_or_render(page, 0).expect("render level 0");
                }
                for page in 0..TRIM_PAGES {
                    cache.get_or_render(page, 1).expect("render level 1");
                }
                assert_eq!(
                    cache.stats().evictions,
                    0,
                    "budget must fit both levels without LRU evictions"
                );
                cache
            },
            |mut cache| {
                cache.trim_to_scale_level(1);
                black_box(cache.stats().evictions)
            },
        );
    });
    g.finish();
}

fn benches(c: &mut Criterion) {
    bench_scale_bitmap(c);
    bench_rerender(c);
    bench_trim(c);
}

criterion_group!(all, benches);
criterion_main!(all);
