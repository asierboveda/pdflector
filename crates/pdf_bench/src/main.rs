//! pdf_bench — RSS/timing harness for the render pipeline (docs/PLAN.md).
//! Renders N pages of a PDF at a scale and reports open time, per-page render
//! time and /proc/self/status VmRSS/VmHWM. Runs on desktop and on Android
//! (adb shell /data/local/tmp/pdfbench <pdf> [pages] [scale]).
//! Usage: cargo run -p pdf_bench --release -- <pdf> [pages] [scale]

use std::path::PathBuf;

use pdf_core::{Document, RenderEngine};

fn proc_status_kb(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        (it.next().map(|k| k.trim_end_matches(':') == key) == Some(true))
            .then(|| it.next()?.parse().ok())
    })?
}

fn main() {
    let mut args = std::env::args().skip(1);
    let pdf = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/large_document.pdf")
    });
    let pages: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20);
    let scale: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2.0);

    let engine = pdf_core::engine::mupdf::MupdfEngine::new();

    let rss_baseline = proc_status_kb("VmRSS");
    let t0 = std::time::Instant::now();
    let doc = engine.open(&pdf).expect("open pdf");
    let open_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let rss_after_open = proc_status_kb("VmRSS");

    let mut rendered = 0;
    let t0 = std::time::Instant::now();
    for page in 0..pages.min(doc.page_count()) {
        doc.render_page(page, scale).expect("render page");
        rendered += 1;
    }
    let render_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let rss_after_render = proc_status_kb("VmRSS");
    let peak = proc_status_kb("VmHWM");

    println!("engine: mupdf");
    println!("pdf: {}", pdf.display());
    println!("pages rendered: {rendered} @ scale {scale}");
    println!("open time: {open_ms:.2} ms");
    println!(
        "render total: {render_ms:.1} ms | per page: {:.2} ms",
        render_ms / rendered.max(1) as f64
    );
    println!("rss baseline: {rss_baseline:?} kB");
    println!("rss after open: {rss_after_open:?} kB");
    println!("rss after render: {rss_after_render:?} kB");
    println!("vmhwm (process peak): {peak:?} kB");
}
