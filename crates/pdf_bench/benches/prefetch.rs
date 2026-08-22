// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Prefetch-effectiveness benchmark (Fase 1 B2): how many pages the worker
//! actually renders during a fast-scroll burst, and how quickly the newest
//! viewport becomes resident.
//!
//! The worker preempts stale wishlists (see `pdf_core/src/prefetch.rs`):
//! a burst of viewports must render only the LAST request's pages (plus a
//! couple in flight), instead of grinding through every intermediate wishlist.
//!
//!   prefetch/burst_misses        — number of distinct page renders (misses)
//!                                  after a 10-request fast-scroll burst over
//!                                  pages 40..=49, measured as the worker's
//!                                  final miss counter. Without preemption a
//!                                  radius-5 burst renders up to ~160 pages;
//!                                  with preemption only the final viewport's
//!                                  ~11 pages (plus in-flight).
//!   prefetch/burst_time_to_resident — time until the LAST request's visible
//!                                  pages are resident (the frame that opens
//!                                  after the fling settles).
//!
//! Run with:
//!     cargo bench -p pdf_bench --bench prefetch
//! Results land in target/criterion/<group>/report/.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use pdf_core::Viewport;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::prefetch::Prefetcher;

fn large_path() -> PathBuf {
    pdf_core::corpus_dir().join("large_document.pdf")
}

/// Renders a 10-viewport fast-scroll burst (non-overlapping viewports at
/// pages 50, 60, ..., 140 with radius 5: each wishlist is ~11 pages, so a
/// worker WITHOUT preemption would render all 110) and returns the worker's
/// final miss count. With preemption only the final viewport (~11 pages) plus
/// a couple of in-flight renders happen.
fn run_burst() -> u64 {
    let prefetcher = Prefetcher::open(
        MupdfEngine::new().expect("mupdf init"),
        &large_path(),
        64 * 1024 * 1024,
    )
    .expect("open prefetcher");
    for k in 0..10 {
        let vp = Viewport {
            first_visible_page: 50 + 10 * k,
            visible_count: 3,
        };
        prefetcher.request(&vp, 500, 5, 0);
    }
    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(60)),
        "worker must drain the burst"
    );
    let s = prefetcher.stats_snapshot();
    println!(
        "[prefetch] burst rendered {} pages (11 * 10 = 110 counterfactual)",
        s.misses
    );
    black_box(s.misses);
    s.misses
}

/// Time until the burst's last viewport pages are resident, from the moment
/// the last request was submitted.
fn run_time_to_resident() -> Duration {
    let prefetcher = Prefetcher::open(
        MupdfEngine::new().expect("mupdf init"),
        &large_path(),
        64 * 1024 * 1024,
    )
    .expect("open prefetcher");
    let start = Instant::now();
    for k in 0..10 {
        let vp = Viewport {
            first_visible_page: 50 + 10 * k,
            visible_count: 3,
        };
        prefetcher.request(&vp, 500, 5, 0);
    }
    let last = 50 + 9 * 10; // final viewport first page
    // Poll: the final viewport pages `last..=last+2` are resident once rendered.
    loop {
        let resident: Vec<usize> = prefetcher
            .resident_pages()
            .iter()
            .map(|k| k.page_idx)
            .collect();
        if (last..=last + 2).all(|p| resident.contains(&p)) {
            let d = start.elapsed();
            println!("[prefetch] final viewport resident in {d:?}");
            black_box(d);
            return d;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn benches(c: &mut Criterion) {
    let mut g = c.benchmark_group("prefetch");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(8));
    g.bench_function("burst_misses", |b| {
        b.iter(run_burst);
    });
    g.bench_function("burst_time_to_resident", |b| {
        b.iter(|| {
            let d = run_time_to_resident();
            black_box(d);
            d
        });
    });
    g.finish();
}

criterion_group!(all, benches);
criterion_main!(all);
