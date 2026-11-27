# PDFLector — Phase 4 Performance & Quality TDD Roadmap

**Date:** 2026-11-27  
**Baseline:** `origin/main` @ `fb2cc5c` + Phase 5 Must-Fix patches  
**Sources:** `docs/audit-2026-11-27.md` (Should-Fix & Nice-to-Have #11–#18) and `docs/performance-targets.md` (TCL 9469X Hard Gates)  
**Methodology:** Strict `$test-driven-development` (RED test commit → minimal `$ponytail` GREEN implementation commit → 7-check CI + TCL 9469X verification).

> Note: Phase 5 (`audit/main-review-2026-11-27`) already fixes Must-Fix items #1–#10. This roadmap covers the remaining performance, cleanup, and UX tasks needed to hit every Hard Gate in `docs/performance-targets.md`. The Kanvas batch JSON in Appendix A is **proposal-only** and has NOT been applied to `kanvas.json`.

---

## Task PF-01: Bound MuPDF `fz_store` (≤ 48 MiB) & Shrink on `dlcache` Eviction

**Audit ref:** #11 | **Target Gate:** `mixed_stress.pdf` Continuo PSS `461.8 MB` → **`≤ 220 MiB`**

### Files
- `Cargo.toml` (workspace `[patch.crates-io]` or upstream PR for `mupdf 0.8` exposing `Context::shrink_store(percent: u32)` & `Context::set_store_limit(bytes: usize)` — required because `mupdf::Context.inner` is `pub(crate)`)
- `crates/pdf_core/src/engine/mupdf.rs:175-240,372-385`
- `crates/pdf_core/tests/display_list_memory.rs` (new integration test)

### Step 1 (RED) — Integration Test
```rust
// crates/pdf_core/tests/display_list_memory.rs
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};
use std::path::Path;

fn vm_rss_kb() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .expect("VmRSS line")
}

#[test]
fn rendering_all_mixed_stress_pages_keeps_rss_delta_bounded() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/mixed_stress.pdf");
    let engine = MupdfEngine::new().expect("engine");
    let doc = engine.open(&path).expect("open mixed_stress");
    let before_kb = vm_rss_kb();
    for page in 0..doc.page_count() {
        let _ = doc.render_page(page, 1.5).expect("render");
    }
    let delta_mb = vm_rss_kb().saturating_sub(before_kb) as f64 / 1024.0;
    assert!(
        delta_mb <= 80.0,
        "MuPDF fz_store grew RSS by {delta_mb:.1} MiB (> 80 MiB cap) across 80 pages"
    );
}
```
Run `cargo test -p pdf_core --test display_list_memory` and confirm FAIL before patching.

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Expose `pub fn set_store_limit(&mut self, max_bytes: usize)` and `pub fn shrink_store(&mut self, percent: u32)` on `mupdf::Context` (10 lines calling `mupdf_sys::fz_shrink_store` and capping `(*(*self.inner).store).max`).
2. In `MupdfEngine::new()` (`crates/pdf_core/src/engine/mupdf.rs:180`), call `ctx.set_store_limit(48 * 1024 * 1024)`.
3. In `MupdfDocument::display_list_for()` (`mupdf.rs:372-385`), when `display_lists.insert(...)` reports an eviction (`evictions` incremented), call `mupdf::Context::get().shrink_store(50)`.

### Step 3 — Verification
- Desktop: `cargo test -p pdf_core --test display_list_memory` PASS.
- TCL 9469X: 30-swipe Continuo on `mixed_stress.pdf` → `dumpsys meminfo` Native Heap `≤ 95 MiB`, `TOTAL PSS ≤ 220 MiB`.

---

## Task PF-02: Reuse Fixed GL Textures in `PageSlots` & Silence Hot-Loop Texture Logs

**Audit ref:** #12 | **Target Gate:** `GL mtrack` `131 MB` → **`≤ 80 MiB`**; `frame p95` `14.6 ms` → **`≤ 9.5 ms`** (paired with `PF-04` → **`≤ 8.33 ms`**)

### Files
- `crates/pdf_android/src/gpu/page_slots.rs:14-60`
- `crates/pdf_android/src/gpu/textures.rs:271-304`

### Step 1 (RED) — Host Unit Tests
```rust
// crates/pdf_android/src/gpu/page_slots.rs (inside #[cfg(test)] mod tests)
#[test]
fn evicting_a_full_pool_recycles_the_victim_texture_handle_without_deleting_it() {
    let mut s = PageSlots::default();
    for p in 0..PAGE_SLOTS_MAX as u32 {
        assert_eq!(s.acquire_slot(p, (100, 200, p as usize)), SlotAction::Allocate);
        s.commit_allocated(p, (100, 200, p as usize), 100 + p);
    }
    // Next page reuses LRU victim tex 100 with matching dimensions (glTexSubImage2D fast path).
    assert_eq!(
        s.acquire_slot(9, (100, 200, 9)),
        SlotAction::ReuseSubImage { tex: 100 }
    );
    // Different dimensions reuse the same handle via glTexImage2D in-place.
    assert_eq!(
        s.acquire_slot(10, (200, 100, 10)),
        SlotAction::ReuseRealloc { tex: 101 }
    );
}
```
Run `cargo test -p pdf_android --lib gpu::page_slots` and confirm compile/test FAIL.

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Replace `insert() -> Vec<u32>` with `acquire_slot(page, key) -> SlotAction` (`Hit(u32)`, `Allocate`, `ReuseSubImage { tex }`, `ReuseRealloc { tex }`) keeping `(page, key, tex)` and recycling the LRU handle once `slots.len() == PAGE_SLOTS_MAX`.
2. In `gpu/textures.rs:273-283`, branch on `SlotAction`: `Hit` returns immediately; `ReuseSubImage` calls `glTexSubImage2D` (zero GPU page allocation); `ReuseRealloc` calls `glTexImage2D` on the existing handle; `Allocate` calls `upload_texture` only for the first 4 pages.
3. Change `info!("gpu: tex destroy ...")` (`textures.rs:298`) and `upload_texture`'s `info!()` to `log::debug!()` so scrolling at 120 Hz does not spam `logd`.

### Step 3 — Verification
- Host: `cargo test -p pdf_android --lib gpu::page_slots` PASS.
- TCL 9469X: 30-swipe Continuo on `large_document.pdf` and `mixed_stress.pdf` → `GL mtrack ≤ 85 MiB`, zero `gpu: tex destroy` lines during scroll, `frame p95` reduced by ≥ 3 ms.

---

## Task PF-03: Skip No-Op Cold-Start `recreate_surface()` & Deduplicate `load_progress()`

**Audit ref:** #13 | **Target Gate:** Cold launch `TotalTime` `649 ms` → **`≤ 500 ms`**

### Files
- `crates/pdf_android/src/gpu/surface.rs` (`recreate_surface` / `has_surface` / cached `(width, height)`)
- `crates/pdf_android/src/reader/life.rs` (initial `load_progress`)

### Step 1 (RED) — Host Unit Test
```rust
// crates/pdf_android/src/gpu/surface.rs (inside #[cfg(test)] mod tests)
#[test]
fn surface_resize_is_noop_when_surface_exists_at_same_dimensions() {
    assert!(!should_recreate_surface(true, (1440, 2200), (1440, 2200)));
    assert!(should_recreate_surface(false, (1440, 2200), (1440, 2200)));
    assert!(should_recreate_surface(true, (1440, 2200), (2200, 1440)));
}
```
Run `cargo test -p pdf_android --lib gpu::surface` and confirm FAIL.

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Add pure guard `should_recreate_surface(has_surface: bool, cur: (u32, u32), next: (u32, u32)) -> bool` and call it at the top of `WindowResized` / `recreate_surface` before destroying `EGLSurface` and Dual FBOs.
2. Skip the second `load_progress` call on initial launch when `self.library.lib_books` was already populated during `Reader::new`.

### Step 3 — Verification
- Host: `cargo test -p pdf_android --lib` PASS.
- TCL 9469X: 3× cold `am start -W` on `scientific_paper.pdf` → logcat shows **1** `EGL/GLES2 ready 1440x2200` (not 3), median `TotalTime ≤ 500 ms`.

---

## Task PF-04: Cache `scroll_layout` Prefix Sums & Emit `frame p50/p95/p99` Telemetry

**Audit ref:** #12 | **Target Gate:** Continuo `frame p95 ≤ 8.33 ms`, `p99 ≤ 12.5 ms` on `large_document.pdf` (500p) and `mixed_stress.pdf` (80p)

### Files
- `crates/pdf_android/src/reader/scroll.rs` (`ScrollLayout` cache keyed by `(geom_gen, viewport_w, viewport_h, zoom_bits)`)
- `crates/pdf_android/src/gpu/pipeline.rs` (frame timing histogram: log `p50`, `p95`, `p99`, `jank_120hz_pct`)

### Step 1 (RED) — Host Unit Tests
```rust
// crates/pdf_android/src/reader/scroll.rs (inside #[cfg(test)] mod tests)
#[test]
fn scroll_layout_cache_reuses_prefix_sums_until_key_changes() {
    let mut cache = ScrollLayoutCache::default();
    let key = LayoutKey { geom_gen: 1, vw: 1440, vh: 2200, zoom_bits: 1.0f32.to_bits() };
    let mut builds = 0usize;
    let _ = cache.get_or_build(key, || { builds += 1; vec![0.0; 500] });
    let _ = cache.get_or_build(key, || { builds += 1; vec![0.0; 500] });
    assert_eq!(builds, 1, "identical layout key must not rebuild 500-page prefix sum");
    let key2 = LayoutKey { geom_gen: 2, ..key };
    let _ = cache.get_or_build(key2, || { builds += 1; vec![0.0; 500] });
    assert_eq!(builds, 2);
}

// crates/pdf_android/src/gpu/pipeline.rs (inside #[cfg(test)] mod tests)
#[test]
fn frame_stats_reports_p50_p95_p99_and_120hz_jank_pct() {
    let samples: Vec<f32> = (1..=100).map(|i| i as f32 * 0.1).collect(); // 0.1..10.0 ms
    let st = FrameStats::from_ms(&samples);
    assert!((st.p50 - 5.0).abs() < 0.2);
    assert!((st.p95 - 9.5).abs() < 0.2);
    assert!((st.p99 - 9.9).abs() < 0.2);
    assert!((st.jank_120hz_pct - 16.0).abs() < 1.5); // > 8.33 ms
}
```

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Store `ScrollLayoutCache` on `Reader`; increment `scroll_geom_gen` only when `scroll_geoms` or `scroll_ref_pts` mutates.
2. Replace `p95`-only calculation in `gpu/pipeline.rs` with `FrameStats::from_ms` logging `frame p50={:.1}ms p95={:.1}ms p99={:.1}ms >8.3ms={:.1}%`.

### Step 3 — Verification
- Host: `cargo test -p pdf_android --lib` PASS.
- TCL 9469X: 30-swipe Continuo on `large_document.pdf` and `mixed_stress.pdf` → `p95 ≤ 8.33 ms`, `p99 ≤ 12.5 ms`, `>8.3ms < 2%`.

---

## Task CL-01: Remove Duplicate Atomic JSON Writers, Atomic `classes.dex`, Deterministic Sync Clock

**Audit ref:** #14, #15 | **Target Gate:** Zero duplicate `write_json_atomic` helpers; atomic DEX extraction; 0 wall-clock sleeps/timers in `folder_sync` tests.

### Files
- `crates/pdf_android/src/folder_sync.rs:752,1233-1254`
- `crates/pdf_core/src/folder_sync.rs:1130`
- `crates/pdf_android/src/jni/ime.rs:57`
- `crates/pdf_android/src/jni/launch.rs:255`
- `crates/pdf_android/src/reader/library.rs:834,980`
- `crates/pdf_android/src/reader/discover.rs:177`

### Step 1 (RED) — Host Unit Tests
```rust
// crates/pdf_android/src/jni/ime.rs (host-testable helper)
#[test]
fn ensure_dex_on_disk_uses_atomic_write_and_leaves_no_tmp_file() {
    let dir = tempfile::tempdir().unwrap();
    let dex = ensure_dex_file(dir.path(), b"dex\n035\0test").unwrap();
    assert_eq!(std::fs::read(&dex).unwrap(), b"dex\n035\0test");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "no leftover .tmp files");
}
```
And replace wall-clock `Instant::now()` assertion in `folder_sync.rs:1233-1254` with an injected monotonic counter `FakeClock`.

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Delete duplicate `write_json_atomic` (`pdf_android/folder_sync.rs:752`) and `atomic_json` (`pdf_core/folder_sync.rs:1130`); call `pdf_core::fsutil::write_atomic`.
2. Switch `jni/ime.rs:57` (`classes.dex`), `jni/launch.rs:255`, `reader/library.rs:834,980`, and `reader/discover.rs:177` to `pdf_core::fsutil::write_atomic`.
3. Remove unused CPU `scale_bitmap` callers from interactive zoom if any remain outside `crates/pdf_bench`.

### Step 3 — Verification
- `cargo test -p pdf_core` & `cargo test -p pdf_android --lib` PASS; `5/5` `folder_sync` stress runs PASS; `grep -rn "fn write_json_atomic\|fn atomic_json" crates/` returns 0 hits.

---

## Task CL-02: Close CI Blind Spots, Remove Dead Code & Stale Docs

**Audit ref:** #16, #17

### Files
- `.github/workflows/ci.yml`
- `crates/pdf_android/src/input/motion/library.rs:28-32,336-367,425,439-447`
- `crates/pdf_android/src/cache.rs:139`
- `.agents/skills/pdflector-rendimiento/SKILL.md:89-96,161-162,251`

### Step 1 (RED) — Host Unit Tests
```rust
// crates/pdf_android/src/input/motion/library.rs (tests)
#[test]
fn switching_to_discover_tab_closes_open_library_menus() {
    let mut r = Reader::test_stub_library();
    r.view_menu_open = true;
    r.settings_menu_open = true;
    r.select_library_tab(LibraryTab::Discover);
    assert!(!r.view_menu_open && !r.settings_menu_open);
}

#[test]
fn search_chip_hitbox_matches_rendered_chip_origin_within_1px() {
    let layout = library_header_layout(1440, 2200);
    assert!((layout.search_chip_hit.x - layout.search_chip_draw.x).abs() <= 1.0);
}
```

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Add `cargo test -p pdf_app` and `cargo clippy -p pdf_android --lib --tests -- -D warnings` to `.github/workflows/ci.yml`.
2. Delete dead `lib_org_chips` (`input/motion/library.rs:336-367,439-447`), fix 6 px search-chip hitbox offset (`L425`), close menus on tab switch (`L28-32`), remove stale `#[allow(dead_code)]` at `cache.rs:139`.
3. Remove obsolete `google_key.txt` instructions from `.agents/skills/pdflector-rendimiento/SKILL.md`.

### Step 3 — Verification
- All 7 CI commands pass locally with 0 warnings; `cargo test -p pdf_app` PASS.

---

## Task UX-01: Theme Contrast (`≥ 3.0:1`), Premultiplied Alpha Halo & Narrow-Screen Modal Clamps

**Audit ref:** #18

### Files
- `crates/pdf_android/src/draw/chrome.rs:154-162`
- `crates/pdf_android/src/draw/study.rs:215-223`
- `crates/pdf_android/src/reader/geometry.rs:315-317`
- `crates/pdf_android/src/gpu/textures.rs` / `gpu/pipeline.rs` (overlay blend mode)

### Step 1 (RED) — Host Unit Tests
```rust
// crates/pdf_android/src/draw/chrome.rs (tests)
#[test]
fn default_dark_aa_underline_meets_wcag_aa_ui_contrast() {
    let p = Theme::DefaultDark.palette();
    let ratio = contrast_ratio(p.accent, p.chrome_bg);
    assert!(ratio >= 3.0, "DefaultDark Aa underline contrast {ratio:.2}:1 < 3.0:1");
}

// crates/pdf_android/src/draw/study.rs (tests)
#[test]
fn key_editor_buttons_do_not_overlap_on_narrow_viewport() {
    let rects = key_editor_button_rects(480, 800, true);
    assert!(rects.delete.right() <= rects.cancel.left());
    assert!(rects.cancel.right() <= rects.save.left());
}
```

### Step 2 (GREEN) — Minimal `$ponytail` Implementation
1. Adjust `DefaultDark` accent luminance so `contrast_ratio(accent, chrome_bg) >= 3.2`.
2. Use `GL_ONE, GL_ONE_MINUS_SRC_ALPHA` (matching premultiplied software bitmaps) or un-premultiply border shadow pixels to eliminate the cyan border halo around white popups/chrome.
3. Clamp Key Editor button widths for `w < 560` and clamp Reader Sheet max height to `(vh - 48).max(240)`.

### Step 3 — Verification
- Host tests PASS; visual screenshot check on TCL 9469X (`tcl_ux_02`, `tcl_ux_04`) confirms zero cyan border halo.

---

## Appendix A — Ready-to-Run `kanvas.py batch` JSON (Proposal Only — NOT Applied)

Review and apply manually via `python3 tools/kanvas.py batch <json>` if approved:

```json
{
  "ops": [
    {
      "op": "add",
      "id": "PF-01",
      "title": "Bound MuPDF fz_store (<=48 MiB) and shrink on dlcache eviction",
      "column": "Perf & Memory",
      "state": "backlog",
      "priority": "high",
      "deps": []
    },
    {
      "op": "add",
      "id": "PF-02",
      "title": "Reuse 4 fixed GL textures in PageSlots and demote hot-loop tex logs",
      "column": "Perf & Memory",
      "state": "backlog",
      "priority": "high",
      "deps": []
    },
    {
      "op": "add",
      "id": "PF-03",
      "title": "Skip no-op cold-start EGLSurface recreate and dedup startup load_progress",
      "column": "Perf & Memory",
      "state": "backlog",
      "priority": "medium",
      "deps": []
    },
    {
      "op": "add",
      "id": "PF-04",
      "title": "Cache scroll_layout prefix sums and emit frame p50/p95/p99 telemetry",
      "column": "Perf & Memory",
      "state": "backlog",
      "priority": "high",
      "deps": ["PF-02"]
    },
    {
      "op": "add",
      "id": "CL-01",
      "title": "Delete duplicate atomic JSON writers, atomic classes.dex, deterministic sync clock",
      "column": "Audit Cleanup",
      "state": "backlog",
      "priority": "medium",
      "deps": []
    },
    {
      "op": "add",
      "id": "CL-02",
      "title": "Add pdf_app tests & host pdf_android clippy to CI, delete dead lib_org_chips & stale docs",
      "column": "Audit Cleanup",
      "state": "backlog",
      "priority": "low",
      "deps": []
    },
    {
      "op": "add",
      "id": "UX-01",
      "title": "Fix DefaultDark Aa contrast, popup alpha halo, and narrow-screen modal clamps",
      "column": "UX Polish",
      "state": "backlog",
      "priority": "low",
      "deps": []
    }
  ]
}
```
