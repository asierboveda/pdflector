// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Budgeted LRU cache of per-page display lists (PR-03).
//!
//! `MupdfDocument` keeps one MuPDF display list per rendered page so later
//! zooms replay the list instead of re-parsing the page (F3.3). Before PR-03
//! that map was unbounded: every page ever rendered kept its list until the
//! document closed, so a long PDF (and the Continuo scroll mode, which renders
//! the visible pages plus neighbours) grew memory without limit.
//!
//! `DisplayListCache` bounds the resident lists by an explicit
//! [`DisplayListBudget`] (entry count AND estimated bytes) with
//! least-recently-used eviction. It is generic over the stored value and takes
//! each entry's size from the caller, so the policy is tested
//! deterministically with fake sizes; the MuPDF size estimate lives next to
//! the engine (`mupdf::estimate_display_list_bytes`).
//!
//! Rules:
//! - Values are handed out as `Rc<V>`. An entry whose `Rc` is still held by a
//!   caller (a render in progress) is "in use" and never evicted; it becomes
//!   evictable once the caller drops its clone.
//! - The entry just requested is never evicted by its own insertion. If that
//!   single entry is larger than the whole byte budget it stays resident
//!   alone (best-effort, same rule as `cache::RenderCache`), so repeated
//!   zooms of the current page still reuse its list.
//! - A budget with `max_entries == 0` or `max_bytes == 0` disables retention:
//!   every request builds a fresh list and nothing stays resident.

use std::fmt;
use std::rc::Rc;

use lru::LruCache;

/// Upper bounds for the resident display lists of ONE document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayListBudget {
    /// Maximum number of resident lists (pages).
    pub max_entries: usize,
    /// Maximum sum of the estimated sizes of the resident lists, in bytes.
    pub max_bytes: usize,
}

impl DisplayListBudget {
    /// Default for the TCL 9469X tablet: 16 lists / 24 MiB per document.
    ///
    /// - 16 entries: Continuo shows up to ~3 pages at once and prefetches ±2
    ///   neighbours (~7 pages); 16 keeps that window plus a screenful of
    ///   back-scroll warm, so scrolling back does not re-parse pages.
    /// - 24 MiB: a text page's list is in the 50-300 KiB range (16 of them
    ///   stay far below the cap), while scanned / vector-heavy pages can pin
    ///   MiBs each (compressed image data is held alive by the list); the
    ///   byte cap stops those from piling up. Next to the page cache (48 MiB plus
    ///   32 MiB deferred slack) and the thumbnails (12 MiB) it keeps the
    ///   app inside the project's RSS < 150 MB target.
    pub const TABLET_DEFAULT: Self = Self {
        max_entries: 16,
        max_bytes: 24 * 1024 * 1024,
    };

    /// `true` when this budget retains nothing (cache disabled).
    pub fn is_disabled(&self) -> bool {
        self.max_entries == 0 || self.max_bytes == 0
    }
}

impl Default for DisplayListBudget {
    fn default() -> Self {
        Self::TABLET_DEFAULT
    }
}

/// Counters of a display-list cache, cumulative since the document opened
/// (`entries`/`bytes` are the current residency).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DisplayListStats {
    /// Resident lists.
    pub entries: usize,
    /// Sum of the estimated sizes of the resident lists.
    pub bytes: usize,
    /// Highest `bytes` ever observed.
    pub peak_bytes: usize,
    /// Requests served by a resident list.
    pub hits: u64,
    /// Requests that had to build a list.
    pub misses: u64,
    /// Lists dropped to honour the budget.
    pub evictions: u64,
}

impl fmt::Display for DisplayListStats {
    /// One greppable log line, e.g.
    /// `dlcache entries=7 bytes=812345 peak=1203400 hits=40 misses=9 evictions=2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dlcache entries={} bytes={} peak={} hits={} misses={} evictions={}",
            self.entries, self.bytes, self.peak_bytes, self.hits, self.misses, self.evictions
        )
    }
}

/// One resident list and the estimated bytes it pins.
struct Entry<V> {
    value: Rc<V>,
    bytes: usize,
}

/// Budgeted LRU of display lists keyed by 0-based page index.
pub struct DisplayListCache<V> {
    budget: DisplayListBudget,
    /// Recency order lives in `lru` (MRU first); eviction is driven here so
    /// in-use entries and the just-requested page can be skipped.
    map: LruCache<u32, Entry<V>>,
    stats: DisplayListStats,
}

impl<V> DisplayListCache<V> {
    pub fn new(budget: DisplayListBudget) -> Self {
        Self {
            budget,
            map: LruCache::unbounded(),
            stats: DisplayListStats::default(),
        }
    }

    pub fn budget(&self) -> DisplayListBudget {
        self.budget
    }

    /// Returns the list of `page`, building it with `build` on a miss.
    /// `build` returns the list and its estimated size in bytes.
    ///
    /// A hit promotes the entry. A miss inserts the new list and then evicts
    /// least-recently-used entries — never `page` itself and never an entry
    /// whose `Rc` is held elsewhere — until the budget holds again (or no
    /// evictable entry is left). A failed build leaves residency untouched.
    pub fn get_or_build<E>(
        &mut self,
        page: u32,
        build: impl FnOnce() -> Result<(V, usize), E>,
    ) -> Result<Rc<V>, E> {
        if let Some(entry) = self.map.get(&page) {
            self.stats.hits += 1;
            return Ok(Rc::clone(&entry.value));
        }
        self.stats.misses += 1;
        let (value, bytes) = build()?;
        let value = Rc::new(value);
        if self.budget.is_disabled() {
            return Ok(value);
        }
        self.map.put(
            page,
            Entry {
                value: Rc::clone(&value),
                bytes,
            },
        );
        self.stats.bytes += bytes;
        self.evict_to_budget(page);
        self.stats.entries = self.map.len();
        self.stats.peak_bytes = self.stats.peak_bytes.max(self.stats.bytes);
        Ok(value)
    }

    /// Snapshot of the counters.
    pub fn stats(&self) -> DisplayListStats {
        self.stats
    }

    fn over_budget(&self) -> bool {
        self.map.len() > self.budget.max_entries || self.stats.bytes > self.budget.max_bytes
    }

    /// Evicts LRU entries until the budget holds, skipping `keep` (the page
    /// being requested) and every entry still referenced by a caller.
    fn evict_to_budget(&mut self, keep: u32) {
        while self.over_budget() {
            // `iter().rev()` walks from the least recently used entry.
            let victim = self
                .map
                .iter()
                .rev()
                .find(|(page, entry)| **page != keep && Rc::strong_count(&entry.value) == 1)
                .map(|(page, _)| *page);
            let Some(victim) = victim else {
                return; // only pinned entries left: best-effort overshoot
            };
            if let Some(entry) = self.map.pop(&victim) {
                self.stats.bytes -= entry.bytes;
                self.stats.evictions += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const KIB: usize = 1024;

    fn budget(max_entries: usize, max_bytes: usize) -> DisplayListBudget {
        DisplayListBudget {
            max_entries,
            max_bytes,
        }
    }

    /// Requests `page` with a fake list of `bytes`; returns whether it built.
    fn request(cache: &mut DisplayListCache<u32>, page: u32, bytes: usize) -> bool {
        let built = Cell::new(false);
        let value = cache
            .get_or_build::<()>(page, || {
                built.set(true);
                Ok((page, bytes))
            })
            .unwrap();
        assert_eq!(*value, page);
        built.get()
    }

    /// Deterministic pseudo-random sizes (no rand dependency), 10-130 KiB:
    /// all below the 200 KiB budget used with them, so the oversized-entry
    /// exception never applies.
    fn fake_size(page: u32) -> usize {
        (page as usize * 7919 % 13 + 1) * 10 * KIB
    }

    #[test]
    fn byte_budget_is_never_exceeded_after_many_pages() {
        let mut cache = DisplayListCache::new(budget(1000, 200 * KIB));
        let mut total_built = 0usize;
        for round in 0..3 {
            for page in 0..500u32 {
                let page = (page * 37 + round) % 400;
                if request(&mut cache, page, fake_size(page)) {
                    total_built += 1;
                }
                let s = cache.stats();
                assert!(s.bytes <= 200 * KIB, "bytes {} over budget", s.bytes);
                assert!(s.entries <= 1000);
            }
        }
        let s = cache.stats();
        assert!(s.entries > 1, "cache retained nothing: {s}");
        assert!(s.evictions > 0);
        assert_eq!(s.misses as usize, total_built);
        assert_eq!(s.misses + s.hits, 1500);
        // Every miss inserted one entry; every eviction removed one.
        assert_eq!(s.misses - s.evictions, s.entries as u64);
    }

    #[test]
    fn entry_cap_is_never_exceeded() {
        let mut cache = DisplayListCache::new(budget(4, usize::MAX));
        for page in 0..50 {
            request(&mut cache, page, 1);
            assert!(cache.stats().entries <= 4);
        }
        let s = cache.stats();
        assert_eq!(s.entries, 4);
        assert_eq!(s.bytes, 4);
        assert_eq!(s.evictions, 46);
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let mut cache = DisplayListCache::new(budget(3, usize::MAX));
        for page in 0..3 {
            assert!(request(&mut cache, page, 10));
        }
        // Touch 0: it becomes most recent, so 1 is now the LRU.
        assert!(!request(&mut cache, 0, 10), "0 must be a hit");
        assert!(request(&mut cache, 3, 10)); // evicts 1
        assert!(!request(&mut cache, 0, 10), "0 must survive");
        assert!(!request(&mut cache, 2, 10), "2 must survive");
        assert!(!request(&mut cache, 3, 10), "3 must survive");
        assert!(request(&mut cache, 1, 10), "1 was the LRU victim");
        assert_eq!(cache.stats().evictions, 2);
    }

    #[test]
    fn byte_eviction_frees_as_many_lru_entries_as_needed() {
        let mut cache = DisplayListCache::new(budget(100, 100));
        for page in 0..4 {
            request(&mut cache, page, 25); // 100 bytes, exactly at budget
        }
        assert_eq!(cache.stats().evictions, 0);
        request(&mut cache, 9, 60); // must free 60: evicts 0, 1, 2
        let s = cache.stats();
        assert_eq!(s.evictions, 3);
        assert_eq!(s.entries, 2);
        assert_eq!(s.bytes, 85);
        assert!(
            !request(&mut cache, 3, 25),
            "3 was the most recent survivor"
        );
    }

    #[test]
    fn list_in_use_is_never_evicted() {
        let mut cache = DisplayListCache::new(budget(2, usize::MAX));
        let held = cache.get_or_build::<()>(0, || Ok((0u32, 10))).unwrap();
        for page in 1..20 {
            request(&mut cache, page, 10);
        }
        // Page 0 is the oldest entry but its Rc is still held by a "render".
        assert_eq!(*held, 0);
        assert!(!request(&mut cache, 0, 10), "in-use list was evicted");
        assert!(cache.stats().entries <= 3);
        drop(held);
        // Released: it is evictable again like any other entry.
        for page in 100..103 {
            request(&mut cache, page, 10);
        }
        assert!(request(&mut cache, 0, 10), "released list stayed pinned");
        assert!(cache.stats().entries <= 2);
    }

    #[test]
    fn oversized_list_stays_alone_until_the_next_request() {
        let mut cache = DisplayListCache::new(budget(10, 100));
        request(&mut cache, 0, 40);
        request(&mut cache, 1, 40);
        request(&mut cache, 2, 500); // larger than the whole budget
        let s = cache.stats();
        assert_eq!(s.entries, 1, "oversized list must stay alone");
        assert_eq!(s.bytes, 500);
        assert_eq!(s.peak_bytes, 500);
        assert_eq!(s.evictions, 2);
        assert!(
            !request(&mut cache, 2, 500),
            "zoom on the same page reuses it"
        );
        request(&mut cache, 3, 10);
        let s = cache.stats();
        assert_eq!(s.entries, 1);
        assert_eq!(s.bytes, 10);
        assert_eq!(s.evictions, 3);
    }

    #[test]
    fn zero_budget_retains_nothing() {
        for b in [budget(0, usize::MAX), budget(usize::MAX, 0), budget(0, 0)] {
            assert!(b.is_disabled());
            let mut cache = DisplayListCache::new(b);
            assert!(request(&mut cache, 0, 10));
            assert!(request(&mut cache, 0, 10), "disabled cache must rebuild");
            let s = cache.stats();
            assert_eq!(s.entries, 0);
            assert_eq!(s.bytes, 0);
            assert_eq!(s.peak_bytes, 0);
            assert_eq!(s.misses, 2);
            assert_eq!(s.hits, 0);
            assert_eq!(s.evictions, 0);
        }
    }

    #[test]
    fn tiny_entry_budget_keeps_only_the_current_page() {
        let mut cache = DisplayListCache::new(budget(1, 1));
        for page in 0..5 {
            assert!(request(&mut cache, page, 1000));
            assert!(!request(&mut cache, page, 1000), "current page reused");
            let s = cache.stats();
            assert_eq!(s.entries, 1);
            assert_eq!(s.bytes, 1000);
        }
        let s = cache.stats();
        assert_eq!(s.hits, 5);
        assert_eq!(s.misses, 5);
        assert_eq!(s.evictions, 4);
    }

    #[test]
    fn counters_track_hits_misses_bytes_and_peak() {
        let mut cache = DisplayListCache::new(budget(2, usize::MAX));
        request(&mut cache, 0, 100);
        request(&mut cache, 1, 300);
        request(&mut cache, 1, 300);
        request(&mut cache, 2, 50); // evicts 0
        let s = cache.stats();
        assert_eq!(
            s,
            DisplayListStats {
                entries: 2,
                bytes: 350,
                peak_bytes: 400,
                hits: 1,
                misses: 3,
                evictions: 1,
            }
        );
        assert_eq!(
            s.to_string(),
            "dlcache entries=2 bytes=350 peak=400 hits=1 misses=3 evictions=1"
        );
    }

    #[test]
    fn build_error_propagates_and_leaves_cache_unchanged() {
        let mut cache = DisplayListCache::new(budget(4, usize::MAX));
        request(&mut cache, 0, 10);
        let before = cache.stats();
        let err = cache.get_or_build(1, || Err::<(u32, usize), _>("boom"));
        assert_eq!(err.err(), Some("boom"));
        let after = cache.stats();
        assert_eq!(after.entries, before.entries);
        assert_eq!(after.bytes, before.bytes);
        assert!(!request(&mut cache, 0, 10));
    }

    #[test]
    fn tablet_default_is_the_documented_budget() {
        let b = DisplayListBudget::default();
        assert_eq!(b, DisplayListBudget::TABLET_DEFAULT);
        assert_eq!(b.max_entries, 16);
        assert_eq!(b.max_bytes, 24 * 1024 * 1024);
        assert!(!b.is_disabled());
    }
}
