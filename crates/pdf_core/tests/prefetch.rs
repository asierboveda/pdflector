// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Real-engine prefetch tests (Fase 1, B2): background prefetch over MuPDF and
//! REAL corpus PDFs. No mocks, no shared state: every test opens its own
//! `Prefetcher` and every miss is an actual MuPDF render.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pdf_core::CacheStats;
use pdf_core::Document;
use pdf_core::RenderEngine;
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
/// Deliberately NOT `await_idle_timeout`: that method only proves the worker
/// finished its queue ("is it idle yet?"), while these tests need the stronger
/// guarantee that a concrete, deterministic number of pages was rendered.
/// Waiting for the miss count is the robust signal here.
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
/// renders its own pages. Under the preemption contract (newer requests abort
/// stale wishlists at the next page boundary), the in-flight far request is
/// ABANDONED when the cancel arrives — only the subset already rendered before
/// the cancel counts, and the reissue near page 5 adds exactly its 7 pages.
///
/// The far request is sized (radius 100 → 200 pages) so it can never finish
/// before the 100 ms sleep + cancel even in release builds (~400 ms+ of
/// renders) — that makes "not all far pages rendered" a robust assertion.
#[test]
fn cancel_pending_then_reissue_only_renders_the_new_pages() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);

    // Huge request in flight: radius 100 around page 400 → pages 300..=499
    // (far too many to finish before the cancel below).
    let far = Viewport {
        first_visible_page: 400,
        visible_count: 3,
    };
    prefetcher.request(&far, 500, 100, 0);
    std::thread::sleep(Duration::from_millis(100)); // let the worker pick it up
    prefetcher.cancel_pending();

    // The cancel preempts the far wishlist: the worker goes idle with ONLY the
    // subset of far pages it had already rendered (strictly fewer than all 200).
    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(30)),
        "cancel must drain the worker"
    );
    let after_cancel = prefetcher.stats_snapshot();
    assert!(
        after_cancel.misses < 200,
        "far request must be preempted by the cancel, not fully rendered: {} misses",
        after_cancel.misses
    );

    // Reissue a small request near page 5: exactly 7 new renders, nothing else.
    let before = after_cancel.misses;
    let near = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&near, 500, 2, 0);
    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(30)),
        "reissue must drain"
    );
    let after = prefetcher.stats_snapshot();
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

/// Regression: `cancel_pending` used to skip the `requested` counter, so after
/// `request -> cancel -> reissue -> await_idle_timeout` the await returned
/// `true` while the reissued request was still queued (the empty cancel had
/// already bumped `completed` past the stale `requested` snapshot). Now the
/// await must cover the reissue too: after `true`, the reissued pages 3..=9
/// MUST already be resident — whatever happened to the far (preempted) request.
///
/// Under the preemption contract the far wishlist (radius 20 around page 300)
/// is abandoned as soon as the cancel/reissue arrive, so the miss count is no
/// longer a fixed 50; the invariant that this regression protects is
/// `await_idle == true ⟹ reissue rendered`.
#[test]
fn await_idle_after_cancel_reissue_waits_for_the_new_request() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);

    // Large request in flight (pages 280..=322), then cancel, then a small
    // reissue near page 5 — all queued back-to-back while the worker is busy.
    let far = Viewport {
        first_visible_page: 300,
        visible_count: 3,
    };
    prefetcher.request(&far, 500, 20, 0);
    prefetcher.cancel_pending();
    let near = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&near, 500, 2, 0);

    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(30)),
        "worker must go idle after draining the reissued request"
    );
    // The reissue must be FULLY rendered by the time the await returns: the
    // whole point of this regression (the old bug returned early, missing the
    // reissued request).
    for page in 3..=9 {
        assert!(
            prefetcher
                .resident_pages()
                .iter()
                .any(|k| k.page_idx == page),
            "await returned true but reissued page {page} is not resident"
        );
    }
    // The far (preempted) request must have been abandoned: fewer misses than
    // the full 43 far pages + 7 reissue pages it would cost without preemption.
    let s = prefetcher.stats_snapshot();
    assert!(
        s.misses < 50,
        "far wishlist must be preempted by cancel/reissue; got {} misses",
        s.misses
    );
}

/// Preemption (prefetch efectivo): a burst of requests while the worker is
/// busy must NOT render the stale wishlists to completion — only the last
/// request's pages (plus whatever was in flight when it arrived).
///
/// Deterministic framing: a ~400-page stale wishlist near page 400 is queued
/// and IMMEDIATELY followed by a tiny request near page 5. The worker can
/// render at most a couple of stale pages before the preemption kicks in, so
/// the total miss count stays ≤ 10 instead of ~400.
#[test]
fn newer_request_preempts_stale_wishlist() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
    let stale = Viewport {
        first_visible_page: 400,
        visible_count: 3,
    };
    let fresh = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    // Back-to-back: the fresh request is queued before the worker can get far
    // into the ~400-page stale wishlist.
    prefetcher.request(&stale, 500, 200, 0); // pages 200..=499 (~400 pages)
    prefetcher.request(&fresh, 500, 2, 0); // pages 3..=9 (7 pages)

    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(60)),
        "worker must drain the fresh request"
    );
    let s = prefetcher.stats_snapshot();
    assert!(
        s.misses <= 10,
        "stale wishlist (~400 pages) must be preempted after at most a couple \
         of in-flight renders; got {} misses",
        s.misses
    );
    assert!(
        (7..=10).contains(&s.misses),
        "fresh request renders pages 3..=9 (7), plus at most a couple of stale \
         pages already in flight before the preemption; got {} misses",
        s.misses
    );
    for page in 3..=9 {
        assert!(
            prefetcher
                .resident_pages()
                .iter()
                .any(|k| k.page_idx == page),
            "fresh page {page} not resident"
        );
    }
}

/// `get_page` answers `Some(bitmap)` for a resident key — a deep copy whose
/// dimensions match a direct 72 dpi render of the same page — and `None` for
/// a key the prefetch never touched. The client polls the returned channel
/// with `try_recv`; here `recv_timeout` just gives the test a bounded wait.
#[test]
fn get_page_returns_resident_bitmap_or_none() {
    let prefetcher = open_prefetcher("large_document.pdf", 32 * 1024 * 1024);
    let vp = Viewport {
        first_visible_page: 5,
        visible_count: 3,
    };
    prefetcher.request(&vp, 500, 2, 0);
    assert!(
        prefetcher.await_idle_timeout(Duration::from_secs(30)),
        "request must render before polling"
    );

    // Resident page: Some bitmap, deep copy, size consistent with the real
    // 72 dpi render of page 5 (MuPDF `render_page` at scale 1.0).
    let rx = prefetcher.get_page(5, 0);
    let bitmap = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker must answer get_page")
        .expect("page 5 at level 0 must be resident");
    let engine = MupdfEngine::new().expect("mupdf init");
    let doc = engine
        .open(&corpus("large_document.pdf"))
        .expect("open doc");
    let direct = doc.render_page(5, 1.0).expect("direct render");
    assert_eq!(
        (bitmap.width, bitmap.height),
        (direct.width, direct.height),
        "resident bitmap must match the rendered page size"
    );
    assert_eq!(
        bitmap.data.len(),
        bitmap.width as usize * bitmap.height as usize * 4
    );

    // A page no request ever touched: not resident -> None.
    let rx = prefetcher.get_page(499, 0);
    let answer = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker must answer get_page");
    assert!(answer.is_none(), "page 499 was never rendered");
}
