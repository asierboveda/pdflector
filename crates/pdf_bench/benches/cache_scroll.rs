//! Fase 1 B1 cache benchmark — NEW BASELINE (no desktop comparison, no
//! regression against previous builds).
//!
//! large_document.pdf, pages 0..50 at 72 dpi (scale 1.0):
//!   naive_hold          — render every page and hold all bitmaps (no cache).
//!   cache_8mb_firstpass — same sequence through `RenderCache` (8 MB budget).
//!   cache_8mb_pass2     — second pass over the SAME cache, visiting only the
//!                         pages still resident: every access is a hit, so it
//!                         measures the pure hit cost (~zero rendering).
//!
//! Criterion measures timings. Peak RSS (VmHWM) is measured per scenario in a
//! separate child process because the kernel peak is monotonic within a single
//! process and would be polluted by the naive scenario.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use criterion::{Criterion, black_box};
use pdf_core::cache::RenderCache;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine, corpus_dir};

/// scale_for_level(0) == 1.0 == 72 dpi.
const SCALE_LEVEL: u32 = 0;
const SCALE: f32 = 1.0;
const PAGE_COUNT: usize = 50;
const BUDGET_BYTES: usize = 8 * 1024 * 1024;

fn large_path() -> PathBuf {
    // Directory resolution is shared with pdf_core (`corpus_dir`).
    corpus_dir().join("large_document.pdf")
}

fn build_engine() -> MupdfEngine {
    MupdfEngine::new().expect("failed to init mupdf")
}

fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Scenario bodies shared by the criterion benches and the RSS child processes.
fn scenario_naive() {
    let engine = build_engine();
    let doc = engine.open(&large_path()).expect("open document");
    let mut all = Vec::with_capacity(PAGE_COUNT);
    for page in 0..PAGE_COUNT {
        all.push(doc.render_page(page as u32, SCALE).expect("render"));
    }
    black_box(all.iter().map(|b| b.data.len()).sum::<usize>());
}

fn scenario_cache() {
    let mut cache =
        RenderCache::open(build_engine(), &large_path(), BUDGET_BYTES).expect("open cache");
    for page in 0..PAGE_COUNT {
        cache.get_or_render(page, SCALE_LEVEL).expect("render");
    }
    black_box(cache.stats().misses);
}

fn scenario_cache_pass2() {
    let mut cache =
        RenderCache::open(build_engine(), &large_path(), BUDGET_BYTES).expect("open cache");
    for page in 0..PAGE_COUNT {
        cache.get_or_render(page, SCALE_LEVEL).expect("render");
    }
    let resident = cache.resident_pages();
    let misses_before = cache.stats().misses;
    // Second pass over the pages still resident in the 8 MB cache: all hits.
    for page in &resident {
        cache.get_or_render(*page, SCALE_LEVEL).expect("hit");
    }
    assert_eq!(
        cache.stats().misses,
        misses_before,
        "second pass must not render anything"
    );
    black_box(cache.stats().hits);
}

fn bench_cache_scroll(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache_scroll");

    group.bench_function("naive_hold_50p_1x", |b| {
        let engine = build_engine();
        let doc = engine.open(&large_path()).expect("open document");
        b.iter(|| {
            let mut all = Vec::with_capacity(PAGE_COUNT);
            for page in 0..PAGE_COUNT {
                all.push(doc.render_page(page as u32, SCALE).expect("render"));
            }
            black_box(all.iter().map(|b| b.data.len()).sum::<usize>())
        });
    });

    group.bench_function("cache_8mb_firstpass_50p_1x", |b| {
        b.iter_with_setup(
            || RenderCache::open(build_engine(), &large_path(), BUDGET_BYTES).expect("open cache"),
            |mut cache| {
                for page in 0..PAGE_COUNT {
                    cache.get_or_render(page, SCALE_LEVEL).expect("render");
                }
                black_box(cache.stats().misses)
            },
        );
    });
    group.bench_function("cache_8mb_pass2_50p_1x", |b| {
        b.iter_with_setup(
            || {
                let mut cache = RenderCache::open(build_engine(), &large_path(), BUDGET_BYTES)
                    .expect("open cache");
                for page in 0..PAGE_COUNT {
                    cache.get_or_render(page, SCALE_LEVEL).expect("render");
                }
                let resident = cache.resident_pages();
                (cache, resident)
            },
            |(mut cache, resident)| {
                // Second pass over the resident pages: every access is a hit.
                for page in &resident {
                    cache.get_or_render(*page, SCALE_LEVEL).expect("hit");
                }
                black_box(cache.stats().hits)
            },
        );
    });

    group.finish();
}

/// Peak RSS of one scenario, measured in a clean child process.
fn rss_child(scenario: &str) -> u64 {
    let out = Command::new(std::env::current_exe().expect("current_exe"))
        .env("CACHE_SCROLL_RSS", scenario)
        .output()
        .expect("spawn rss child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .find_map(|l| l.strip_prefix("VMHWM_KB="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn rss_report() {
    println!("[cache_scroll] peak RSS per scenario (VmHWM, separate processes):");
    for scenario in ["naive", "cache_8mb", "cache_pass2"] {
        let kb = rss_child(scenario);
        println!("[cache_scroll]   {scenario}: VMHWM_KB={kb}");
    }
}

/// Real numbers for the report: how many pages stay resident under the 8 MB
/// budget after a full 0..50 sweep, and which page is the most expensive.
fn findings() {
    let mut cache =
        RenderCache::open(build_engine(), &large_path(), BUDGET_BYTES).expect("open cache");
    let mut worst = (0usize, 0.0f64);
    for page in 0..PAGE_COUNT {
        let t = std::time::Instant::now();
        cache.get_or_render(page, SCALE_LEVEL).expect("render");
        let dt = t.elapsed().as_secs_f64() * 1e3;
        if dt > worst.1 {
            worst = (page, dt);
        }
    }
    println!(
        "[cache_scroll] pages_resident_in_8mb={} worst_page=page{} time={:.2}ms",
        cache.stats().entries,
        worst.0,
        worst.1
    );
}

fn main() {
    // Child mode: run one scenario, report its own peak RSS, exit before any
    // criterion work so the kernel peak is not polluted.
    if let Ok(scenario) = std::env::var("CACHE_SCROLL_RSS") {
        let rss = match scenario.as_str() {
            "naive" => {
                scenario_naive();
                peak_rss_kb()
            }
            "cache_8mb" => {
                scenario_cache();
                peak_rss_kb()
            }
            "cache_pass2" => {
                scenario_cache_pass2();
                peak_rss_kb()
            }
            _ => None,
        };
        println!("VMHWM_KB={}", rss.unwrap_or(0));
        return;
    }

    let mut c = Criterion::default()
        .configure_from_args()
        .warm_up_time(Duration::from_millis(500))
        .sample_size(15)
        .measurement_time(Duration::from_secs(3));
    bench_cache_scroll(&mut c);
    findings();
    rss_report();
    c.final_summary();
}
