//! Background prefetch with a priority queue (visible pages first).
//!
//! Actor model: the single worker thread owns `(engine, doc, cache)` for its
//! whole life. This is required because `MupdfDocument` is neither `Sync` nor
//! `Send` (raw `*mut fz_document` bound to the thread-local MuPDF context), so
//! the cache must be *built inside* the worker thread — never moved across the
//! thread boundary. The client talks to the worker over a `std::sync::mpsc`
//! channel (stdlib, no new dependency).
//!
//! Each `request` replaces the previous one: the worker only processes the
//! list it just received, it keeps no accumulated wishlist. Rendering already
//! in progress when a new request arrives is not cancelled (B2 minimal); the
//! new list simply takes over as the work queue.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::cache::{CacheStats, PageKey, RenderCache};
use crate::engine::{RenderEngine, Result};
use crate::scroll::Viewport;

/// Commands sent to the worker thread.
enum Cmd {
    Stop,
    /// `(visibles, prefetch)`: visible pages are rendered first, prefetch
    /// pages after. The previous wishlist is discarded on receipt.
    Request(Vec<PageKey>, Vec<PageKey>),
    /// Client asks for the worker's resident pages; the worker answers on
    /// `reply` (cross-thread snapshot, used by tests and the debug overlay).
    Snapshot(Sender<Vec<PageKey>>),
}

/// Prefetch controller: owns the channel to the renderer worker and a
/// thread-safe snapshot of the worker's cache stats.
///
/// `Prefetcher::open` spawns the worker; the document and cache live in that
/// thread and are torn down there too (`Drop` joins the worker).
pub struct Prefetcher<E: RenderEngine> {
    tx: Sender<Cmd>,
    handle: Option<JoinHandle<()>>,
    stats: Arc<Mutex<CacheStats>>,
    /// Number of `Cmd::Request` submitted by the client (incremented before
    /// each `send`). Drives `await_idle_timeout`.
    requested: Arc<AtomicU64>,
    /// Number of `Cmd::Request` fully processed by the worker.
    completed: Arc<AtomicU64>,
    _marker: std::marker::PhantomData<E>,
}

impl<E: RenderEngine> Prefetcher<E> {
    /// Spawns the renderer worker and waits for its init result.
    ///
    /// The engine is moved into the worker, which opens `path` and builds the
    /// byte-bounded `RenderCache` in its own thread (the MuPDF context is
    /// per-thread TLS, so the document must be created where it is used).
    pub fn open(engine: E, path: &Path, byte_budget: usize) -> Result<Self>
    where
        E: Send + 'static,
    {
        let (tx, rx) = channel::<Cmd>();
        let (init_tx, init_rx) = channel::<Result<()>>();
        let stats = Arc::new(Mutex::new(CacheStats::default()));
        let stats_worker = stats.clone();
        let requested = Arc::new(AtomicU64::new(0));
        let completed = Arc::new(AtomicU64::new(0));
        let completed_worker = completed.clone();
        let owned_path = path.to_owned();

        let handle = std::thread::spawn(move || {
            // Worker thread: sole owner of (engine, doc, cache).
            let mut cache = match RenderCache::open(engine, &owned_path, byte_budget) {
                Ok(cache) => cache,
                Err(err) => {
                    let _ = init_tx.send(Err(err));
                    return;
                }
            };
            let _ = init_tx.send(Ok(()));

            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Cmd::Stop => break,
                    Cmd::Request(visibles, prefetch) => {
                        // Visible pages first, then prefetch neighbours.
                        for key in visibles.iter().chain(prefetch.iter()) {
                            let _ = cache.get_or_render(key.page_idx, key.scale_level);
                        }
                        if let Ok(mut snapshot) = stats_worker.lock() {
                            *snapshot = *cache.stats();
                        }
                        // The request is fully processed: release waiters. This
                        // must happen after rendering so `await_idle_timeout`
                        // never returns while pages are still being rendered.
                        completed_worker.fetch_add(1, Ordering::Relaxed);
                    }
                    Cmd::Snapshot(reply) => {
                        let _ = reply.send(cache.resident_keys());
                    }
                }
            }
        });

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Prefetcher {
                tx,
                handle: Some(handle),
                stats,
                requested,
                completed,
                _marker: std::marker::PhantomData,
            }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                let _ = handle.join();
                Err(crate::engine::Error::Engine(
                    "prefetcher worker exited before init completed".to_string(),
                ))
            }
        }
    }

    /// Submits the current viewport as a render/prefetch request: visible
    /// pages first, then `radius` neighbours, both at `scale_level`.
    ///
    /// Non-blocking (send-only). Replaces any previous pending request.
    pub fn request(&self, vp: &Viewport, total: usize, radius: usize, scale_level: u32) {
        let range = crate::scroll::visible_and_prefetch_pages(vp, total, radius);
        // Visible window, clamped to the document, in visible-first order.
        let visible_end = vp
            .first_visible_page
            .saturating_add(vp.visible_count)
            .min(total);
        let visibles: Vec<PageKey> = (vp.first_visible_page..visible_end)
            .map(|page_idx| PageKey {
                page_idx,
                scale_level,
            })
            .collect();

        // Prefetch = the computed range minus what is already visible
        // (guaranteed disjoint from `visibles`).
        let visible_set: HashSet<usize> = visibles.iter().map(|k| k.page_idx).collect();
        let prefetch: Vec<PageKey> = range
            .filter(|page_idx| !visible_set.contains(page_idx))
            .map(|page_idx| PageKey {
                page_idx,
                scale_level,
            })
            .collect();

        // Publish the request BEFORE sending it, so `await_idle_timeout` can
        // never observe it as completed before it was even submitted.
        self.requested.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Request(visibles, prefetch));
    }

    /// Discards any pending wishlist. Rendering already in progress is not
    /// interrupted (the worker has no cancellation; see module docs).
    pub fn cancel_pending(&self) {
        let _ = self.tx.send(Cmd::Request(Vec::new(), Vec::new()));
    }

    /// Latest snapshot of the worker's cache counters.
    pub fn stats_snapshot(&self) -> CacheStats {
        self.stats.lock().map(|s| *s).unwrap_or_default()
    }

    /// Thread-safe snapshot of the worker's resident pages, most-recently-used
    /// first. Round-trips through the worker's command channel.
    pub fn resident_pages(&self) -> Vec<PageKey> {
        let (reply_tx, reply_rx) = channel::<Vec<PageKey>>();
        let _ = self.tx.send(Cmd::Snapshot(reply_tx));
        reply_rx.recv().unwrap_or_default()
    }

    /// Waits until the worker has processed every `Cmd::Request` submitted up
    /// to the moment this method is called, or `timeout` elapses. Returns
    /// `true` if the worker went idle (i.e. the last request was fully
    /// rendered and its stats published).
    ///
    /// Contract: do NOT call `request()` concurrently while waiting — the
    /// `requested` snapshot taken here only covers requests issued before the
    /// call. Postcondition on `true`: `stats_snapshot()` accounts for all
    /// requests launched up to the call.
    ///
    /// Lightweight: polls the completion counter every 2ms; safe to call right
    /// after `request()` (the worker may already be done, which is success).
    pub fn await_idle_timeout(&self, timeout: Duration) -> bool {
        let snapshot = self.requested.load(Ordering::Relaxed);
        let deadline = Instant::now() + timeout;
        loop {
            if self.completed.load(Ordering::Relaxed) >= snapshot {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl<E: RenderEngine> Drop for Prefetcher<E> {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
