// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! pdf_bench — engine sweep binary (single engine since ADR-001: MuPDF).
//!
//! For every PDF in the corpus: measures the open cost, then renders three
//! representative pages (first, middle, last) at scale 1.0 and 2.0, reporting
//! median-of-3 render times. Then measures the zoom fast-vs-sharp paths on
//! page 0 (software upscale of the level-0 render vs crisp re-render at
//! levels 1/2). Ends by printing the process peak RSS as reported
//! by the kernel (VmHWM from /proc/self/status) — no polling involved.
//!
//! Uses manual timing (Instant), NOT criterion: this is a quick smoke sweep,
//! the precise harness lives in `benches/open_render.rs`.

use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine, corpus_dir, scale_bitmap, scale_for_level};
use std::path::PathBuf;
use std::time::Instant;

/// All corpus documents with a stable label, relative to the workspace root.
/// Directory resolution is shared with pdf_core (`corpus_dir`).
fn corpus_files() -> Vec<(&'static str, PathBuf)> {
    let base = corpus_dir();
    vec![
        ("dense", base.join("dense_textbook.pdf")),
        ("scanned", base.join("scanned_pages.pdf")),
        ("paper", base.join("scientific_paper.pdf")),
        ("large", base.join("large_document.pdf")),
    ]
}

/// Peak resident set size of this process in KiB, from the kernel (VmHWM line
/// of /proc/self/status). None if the file/line is unavailable.
fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Median of `n` runs of `f`, in milliseconds (f64).
fn median_ms<F: FnMut() -> f64>(n: usize, mut f: F) -> f64 {
    let mut samples: Vec<f64> = (0..n).map(|_| f()).collect();
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = samples.len() / 2;
    if samples.len().is_multiple_of(2) {
        (samples[mid - 1] + samples[mid]) / 2.0
    } else {
        samples[mid]
    }
}

fn main() {
    let engine = MupdfEngine::new().expect("failed to init mupdf");
    run_sweep(&engine);
    run_zoom_section(&engine);
}

/// Zoom fast-vs-sharp path measurement (B3): for page 0 of the corpus
/// documents, times the cheap software upscale of the level-0 render
/// (scale_bitmap to the 2x/4x target sizes) against the crisp re-render at
/// ladder levels 1 (scale 2.0) and 2 (scale 4.0) — the two options the UI
/// has when the user zooms in. median_ms(3) like the rest of the sweep.
fn run_zoom_section<E: RenderEngine>(engine: &E) {
    let base = corpus_dir();
    let targets = [
        ("large", base.join("large_document.pdf")),
        ("dense", base.join("dense_textbook.pdf")),
    ];

    for (label, path) in targets {
        let doc = engine.open(&path).expect("open zoom document");
        let page = 0;

        // FAST PATH: one render at level 0 (scale 1.0), then software-scale
        // that bitmap to the level-1/level-2 target sizes (w*2/h*2, w*4/h*4).
        let base_bitmap = doc
            .render_page(page, scale_for_level(0))
            .expect("render zoom level 0");
        let (w, h) = (base_bitmap.width, base_bitmap.height);

        let scale2x = median_ms(3, || {
            let t = Instant::now();
            scale_bitmap(&base_bitmap, w * 2, h * 2).expect("scale 2x");
            t.elapsed().as_secs_f64() * 1e3
        });
        let scale4x = median_ms(3, || {
            let t = Instant::now();
            scale_bitmap(&base_bitmap, w * 4, h * 4).expect("scale 4x");
            t.elapsed().as_secs_f64() * 1e3
        });

        // SHARP PATH: crisp re-renders at ladder levels 1 (2.0) and 2 (4.0).
        let rerender1 = median_ms(3, || {
            let t = Instant::now();
            doc.render_page(page, scale_for_level(1))
                .expect("render zoom level 1");
            t.elapsed().as_secs_f64() * 1e3
        });
        let rerender2 = median_ms(3, || {
            let t = Instant::now();
            doc.render_page(page, scale_for_level(2))
                .expect("render zoom level 2");
            t.elapsed().as_secs_f64() * 1e3
        });

        println!(
            "zoom {label} p0 scale2x={scale2x:.2}ms scale4x={scale4x:.2}ms rerender1={rerender1:.2}ms rerender2={rerender2:.2}ms"
        );
    }
}

fn run_sweep<E: RenderEngine>(engine: &E) {
    println!("pdf_bench sweep (engine=mupdf)");

    for (label, path) in corpus_files() {
        // Open timing: build the engine first (once), then time the open.
        let t = Instant::now();
        let doc = engine.open(&path).expect("open document");
        let open_ms = t.elapsed().as_secs_f64() * 1e3;

        let page_count = doc.page_count();
        let last = page_count.saturating_sub(1);
        let pages = if page_count <= 1 {
            vec![0]
        } else {
            vec![0, page_count / 2, last]
        };

        let render_ms = |scale: f32| -> f64 {
            median_ms(3, || {
                let mut total = 0.0;
                for &p in &pages {
                    let t = Instant::now();
                    doc.render_page(p, scale).expect("render page");
                    total += t.elapsed().as_secs_f64() * 1e3;
                }
                total
            })
        };

        let r1 = render_ms(1.0);
        let r2 = render_ms(2.0);
        println!(
            "{label} pages={page_count} open={open_ms:.2}ms render1x={r1:.2}ms render2x={r2:.2}ms"
        );
    }

    match peak_rss_kb() {
        Some(kb) => println!("PEAK_RSS_KB={kb}"),
        None => println!("PEAK_RSS_KB=unknown"),
    }
}
