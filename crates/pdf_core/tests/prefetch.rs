//! Real-engine prefetch tests (Fase 1, B2): background prefetch over MuPDF and
//! REAL corpus PDFs. No mocks, no shared state: every test opens its own
//! `Prefetcher` and every miss is an actual MuPDF render.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pdf_core::CacheStats;
use pdf_core::Viewport;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::prefetch::Prefetcher;

fn corpus(name: &str) -> PathBuf {
    pdf_core::corpus_dir().join(name)
}

fn open_prefetcher(name: &str, budget: usize) -> Prefetcher<MupdfEngine> {
    Prefetcher::open(
        MupdfEngine::new().expect("mupdf init"),
        &corpus(name),
        budget,
    )
    .expect("open prefetcher")
}

/// Polls until the worker's miss counter reaches `target` (a request renders a
/// known, deterministic number of pages) or `timeout` elapses. Returns the
/// last snapshot.
///
/// This is deliberately NOT `await_idle_timeout`: that method samples twice
/// 2ms apart and declares the worker idle if both match — but the worker only
/// publishes stats after finishing a whole request, so calling it right after
/// `request()` returns `true` prematurely (~2ms, with 0 misses, while the
/// render is still running). Waiting for a concrete miss count is the only
/// robust signal with this API.
fn wait_misses(prefetcher: &Prefetcher<MupdfEngine>, target: u64, timeout: Duration) -> CacheStats {
    let deadline = Instant::now() + timeout;
    loop {
        let s = prefetcher.stats_snapshot();
        if s.misses >= target {
            return s;
        }
        if Instant::now() >= deadline {
            return s;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A `request` renders visible pages plus radius neighbours in the background
/// and the cache ends up populated with at least the visible window.
#[test]
fn prefetch_populates_cache_in_background() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
    let vp = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&vp, 500, 2, 0);

    let s = wait_misses(&prefetcher, 7, Duration::from_secs(30));
    assert_eq!(s.misses, 7, "pages 3..=9 rendered exactly once");
    assert!(s.entries >= 3, "entries={}", s.entries);
}

/// With a small budget (~2 resident pages) the visible pages must be rendered
/// BEFORE the prefetch neighbours: the most-recently-used tail of the cache is
/// the last prefetch page (9), and the first visible page (5) is evicted. If
/// prefetch were rendered first, the tail would be a visible page instead.
#[test]
fn visible_pages_are_rendered_before_prefetch() {
    let prefetcher = open_prefetcher("large_document.pdf", 4 * 1024 * 1024);
    let vp = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&vp, 500, 2, 0);

    let s = wait_misses(&prefetcher, 7, Duration::from_secs(30));
    assert_eq!(s.misses, 7, "pages 3..=9 rendered once each");

    let resident = prefetcher.resident_pages();
    assert!(
        resident.iter().any(|k| k.page_idx == 9),
        "prefetch tail (page 9) must be the most recently rendered; got {resident:?}"
    );
    assert!(
        !resident.iter().any(|k| k.page_idx == 5),
        "first visible page (5) must have been evicted by the later prefetch; got {resident:?}"
    );
}

/// `request` is non-blocking: submitting a large wishlist (radius 50, ~52
/// pages) must return almost instantly, even while the worker is rendering.
#[test]
fn request_does_not_block_the_client() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
    let vp = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    let start = Instant::now();
    prefetcher.request(&vp, 500, 50, 0);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(200),
        "request() blocked the caller for {elapsed:?}"
    );
}

/// Dropping the prefetcher joins the worker cleanly: it finishes any render in
/// flight, processes Stop and exits. Verified from a separate thread so a hang
/// fails the test via `recv_timeout` instead of hanging the whole suite.
#[test]
fn drop_does_not_hang() {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
        let vp = Viewport {
            first_visible_page: 5,
            visible_count: 3,
        };
        // Small request: a few pages are still being rendered when `drop` runs.
        prefetcher.request(&vp, 500, 1, 0);
        drop(prefetcher); // joins the worker
        let _ = done_tx.send(());
    });

    done_rx
        .recv_timeout(Duration::from_secs(15))
        .expect("drop() hung: worker did not join in 15s");
    handle.join().expect("drop thread panicked");
}

/// After `cancel_pending` the pipeline stays usable and a reissued request only
/// renders its own pages: the counters grow by exactly the 7 pages 3..=9, and
/// the far-away (400-range) pages are never re-rendered.
#[test]
fn cancel_pending_then_reissue_only_renders_the_new_pages() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);

    // Large request in flight: pages 390..=412 (radius 10 around page 400).
    let far = Viewport {
        first_visible_page: 400,
        visible_count: 3,
    };
    prefetcher.request(&far, 500, 10, 0);
    std::thread::sleep(Duration::from_millis(100)); // let the worker pick it up
    prefetcher.cancel_pending();

    // Radius 10 around page 400 renders exactly pages 390..=412 (23 pages).
    let after_cancel = wait_misses(&prefetcher, 23, Duration::from_secs(30));
    assert_eq!(
        after_cancel.misses, 23,
        "far request must render pages 390..=412"
    );

    // Reissue a small request near page 5: exactly 7 new renders, nothing else.
    let before = after_cancel.misses;
    let near = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&near, 500, 2, 0);
    let after = wait_misses(&prefetcher, before + 7, Duration::from_secs(30));
    assert_eq!(
        after.misses - before,
        7,
        "only pages 3..=9 may render after cancel; misses {} -> {}",
        before,
        after.misses
    );
}

/// `await_idle_timeout` must genuinely wait for the worker to finish, not just
/// sample twice and call it idle (the old ~2ms premature-`true` bug). Radius 5
/// around pages 5..=7 renders pages 0..=12 (13 pages); after `true` the stats
/// must already account for all 13 misses.
#[test]
fn test_await_idle_realmente_espera() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
    let vp = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&vp, 500, 5, 0);

    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(5)),
        "worker must go idle after processing the request"
    );
    let s = prefetcher.stats_snapshot();
    assert_eq!(
        s.misses, 13,
        "pages 0..=12 rendered exactly once; got {}",
        s.misses
    );
}
