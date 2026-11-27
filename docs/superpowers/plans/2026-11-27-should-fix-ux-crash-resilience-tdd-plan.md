# PDFLector — Should-Fix, Crash Resilience & Relationship-Centric UX TDD Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` (recommended) or `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Enforce strict `$test-driven-development` (**NO production code without a failing test first**) and `$ponytail` (smallest complete fix, zero bloat).

**Goal:** Resolve every remaining Should-Fix (#11–#16) and Nice-to-Have (#17–#18) audit item, harden native crash resilience & diagnostics, and elevate PDFLector's Study/Library experience using Relationship-Centric Agentic UX — hitting every TCL NXTPaper 11 Plus (`9469X`) 120 Hz & memory Hard Gate via strict Red→Green→Refactor TDD.

**Architecture:** All work is decomposed into 11 independently testable, self-contained tasks across three tracks: (A) Memory & 120 Hz GPU/Scroll performance (`pdf_core` + `pdf_android::gpu`/`reader`), (B) Crash-safe atomic persistence, native panic/crash diagnostics & CI hardening (`pdf_core` + `pdf_android::jni`/`folder_sync` + `.github/workflows/ci.yml`), and (C) Visual polish + Relationship-Centric Agentic UX memory/trust controls (`pdf_android::draw`/`reader::study`). Because `Reader` references Android-only GLES2/EGL symbols (`libGLESv2`/`libEGL`) and cannot be instantiated in Linux host unit tests, every Rust unit test exercises **pure state/layout/policy structs and functions** on host, then wires them directly into `Reader`/`Gpu` and validates live on TCL 9469X.

**Tech Stack:** Rust (`pdf_core`, `pdf_android`, `pdf_app`, `pdf_bench`), MuPDF 0.8 (`mupdf` / `mupdf-sys`), OpenGL ES 2.0 / EGL, Android NDK r28 (`aarch64-linux-android`), Kotlin/Gradle (`android/product`, `android/ink_bench`), Python `unittest`, GitHub Actions CI.

**Specs:**
- [audit-2026-11-27.md](file:///home/asierboveda/Projects/pdflector/.worktrees/audit-main/docs/audit-2026-11-27.md) (Should-Fix #11–#16, Nice-to-Have #17–#18, and 6 low-impact crash/edge guards)
- [performance-targets.md](file:///home/asierboveda/Projects/pdflector/.worktrees/audit-main/docs/performance-targets.md) (TCL 9469X 120 Hz / PSS / cold-start Hard Gates)
- [benchmark-results.md](file:///home/asierboveda/Projects/pdflector/.worktrees/audit-main/docs/benchmark-results.md) (Desktop & TCL 9469X baselines)

## Global Constraints

- **TDD Iron Law:** Every task MUST follow **Write RED test → Watch RED fail for the expected reason → Commit RED → Write minimal GREEN code → Watch GREEN pass → Commit GREEN**. Never write production code before watching a failing test.
- **Host testability rule:** Never construct `Reader` in Linux host `#[test]` functions (links `libGLESv2`/`libEGL`). Extract or extend pure structs/functions (`PageSlots`, `FrameTimer`, `ScrollLayoutCache`, `RenderFailures`, `StudyState`, `ReadingRelationship`, `TrustDisclosure`, `should_recreate_surface`, `ensure_dex_file`, etc.) and test them directly on host.
- **Worktree & Kanvas discipline:** Execute inside an isolated worktree; never touch `feat/pdflector-v2` directly; never edit `kanvas.json` without explicit human approval.
- **Privacy (D3):** Never open, read, log, or screenshot personal PDFs or user JSON contents on the tablet; all live profiling uses synthetic `bench_corpus/` with backed-up and restored user JSONs; crash breadcrumbs record only module/phase/page-index/counts, never file names, API keys, or note text.
- **7-Gate CI before completion:**
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo clippy -p pdf_android --lib --tests -- -D warnings`
  4. `cargo clippy -p pdf_android --lib -- -D warnings`
  5. `cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings` (NDK r28)
  6. `cargo test -p pdf_core`, `cargo test -p pdf_android --lib`, `cargo test -p pdf_app`, `5×` `folder_sync` stress
  7. `python3 -m unittest discover -s tools/tests`, `PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests`, `(cd android/product && ./gradlew :app:testDebugUnitTest --no-daemon && ./gradlew -p ../ink_bench :app:testDebugUnitTest --no-daemon)`

---

## Track A — Performance & Memory Hard Gates (Audit #11–#14)

### Task 1 (SF-11): Bound MuPDF `fz_store` (≤ 48 MiB) & Shrink on `dlcache` Eviction

**Audit ref:** #11 | **Hard Gate:** `mixed_stress.pdf` 30-swipe Continuo `TOTAL PSS` `461.8 MB` → **`≤ 220 MiB`** (`Native Heap ≤ 95 MiB`).

**Files:**
- Modify: `Cargo.toml` (add `[patch.crates-io]` or vendored thin wrapper if `mupdf 0.8` does not expose `fz_shrink_store` / store max limit)
- Modify: `crates/pdf_core/src/engine/mupdf.rs:175-240,372-385`
- Create: `crates/pdf_core/tests/display_list_memory.rs`

**Interfaces:**
- Consumes: `MupdfEngine::new()`, `Document::render_page()`, `DisplayListCache::stats().evictions`
- Produces: Bounded MuPDF store (`MUPDF_STORE_MAX_BYTES = 48 * 1024 * 1024`) and eviction-triggered `shrink_store(50)` inside `MupdfDocument::display_list_for()`.

- [ ] **Step 1: Write the failing integration test**

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
    if !path.is_file() {
        panic!("generate mixed_stress.pdf first via tools/generate_corpus.py");
    }
    let engine = MupdfEngine::new().expect("engine");
    let doc = engine.open(&path).expect("open mixed_stress");
    let before_kb = vm_rss_kb();
    for page in 0..doc.page_count() {
        let _ = doc.render_page(page, 1.5).expect("render");
    }
    let delta_mb = vm_rss_kb().saturating_sub(before_kb) as f64 / 1024.0;
    assert!(
        delta_mb <= 80.0,
        "MuPDF fz_store grew RSS by {delta_mb:.1} MiB (> 80 MiB cap) across {} pages",
        doc.page_count()
    );
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_core --test display_list_memory -- --nocapture`
Expected: FAIL showing RSS delta > 80 MiB because MuPDF's default 256 MiB `fz_store` retains decoded images/resources across all 80 pages.

- [ ] **Step 3: Commit RED test**

```bash
git add crates/pdf_core/tests/display_list_memory.rs
git commit -m "test(core): add RED memory test for unbounded MuPDF fz_store growth"
```

- [ ] **Step 4: Write minimal GREEN implementation**

1. Expose `set_store_limit(max_bytes: usize)` and `shrink_store(percent: u32)` on the MuPDF context (via `mupdf_sys::fz_shrink_store` and `(*(*ctx).store).max = max_bytes`).
2. In `MupdfEngine::new()` (`crates/pdf_core/src/engine/mupdf.rs`), apply `MUPDF_STORE_MAX_BYTES = 48 * 1024 * 1024`.
3. In `MupdfDocument::display_list_for()` (`crates/pdf_core/src/engine/mupdf.rs:372-380`), record `evictions_before = cache.stats().evictions`; if `cache.stats().evictions > evictions_before`, shrink the MuPDF store by 50%.

```rust
// crates/pdf_core/src/engine/mupdf.rs
pub const MUPDF_STORE_MAX_BYTES: usize = 48 * 1024 * 1024;

fn display_list_for(&self, page: u32) -> Result<Rc<mupdf::DisplayList>> {
    let mut cache = self.display_lists.borrow_mut();
    let evictions_before = cache.stats().evictions;
    let list = cache.get_or_build(page, || {
        let page_obj = self.load_page(page)?;
        let list = page_obj
            .to_display_list(true)
            .map_err(|e| Error::Engine(e.to_string()))?;
        Ok((list, estimate_display_list_bytes(&page_obj)))
    })?;
    let evicted = cache.stats().evictions > evictions_before;
    drop(cache);
    if evicted {
        mupdf::Context::get().shrink_store(50);
    }
    Ok(list)
}
```

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_core --test display_list_memory -- --nocapture && cargo test -p pdf_core`
Expected: PASS (`delta_mb <= 80.0`).
```bash
git add Cargo.toml crates/pdf_core/src/engine/mupdf.rs
git commit -m "perf(core): cap MuPDF fz_store at 48 MiB and shrink on display-list eviction"
```

---

### Task 2 (SF-12a): Reuse Fixed GL Textures in `PageSlots` & Silence Hot-Loop Logs

**Audit ref:** #12 | **Hard Gate:** `GL mtrack` `131 MB` → **`≤ 80 MiB`**, zero per-page `glGenTextures`/`glDeleteTextures` churn and zero per-texture `info!()` log spam during Continuo scroll.

**Files:**
- Modify: `crates/pdf_android/src/gpu/page_slots.rs:14-60`
- Modify: `crates/pdf_android/src/gpu/textures.rs:271-304`

**Interfaces:**
- Consumes: `PAGE_SLOTS_MAX`, `BitmapKey = (u32, u32, usize)`
- Produces: `SlotAction` (`Hit(u32)`, `Allocate`, `ReuseSubImage { tex: u32 }`, `ReuseRealloc { tex: u32 }`), `PageSlots::acquire_slot(&mut self, page: u32, key: BitmapKey) -> SlotAction`, `PageSlots::commit_allocated(&mut self, page: u32, key: BitmapKey, tex: u32)`.

- [ ] **Step 1: Write the failing host unit tests**

```rust
// crates/pdf_android/src/gpu/page_slots.rs (inside #[cfg(test)] mod tests)
#[test]
fn evicting_a_full_pool_recycles_victim_handle_with_subimage_or_realloc() {
    let mut s = PageSlots::default();
    for p in 0..PAGE_SLOTS_MAX as u32 {
        assert_eq!(s.acquire_slot(p, (100, 200, p as usize)), SlotAction::Allocate);
        s.commit_allocated(p, (100, 200, p as usize), 100 + p);
    }
    // Hit promotes slot to MRU without allocating or reallocating.
    assert_eq!(s.acquire_slot(0, (100, 200, 0)), SlotAction::Hit(100));
    // Victim is now page 1 (tex 101); matching dimensions -> glTexSubImage2D fast path.
    assert_eq!(
        s.acquire_slot(9, (100, 200, 9)),
        SlotAction::ReuseSubImage { tex: 101 }
    );
    // Victim is now page 2 (tex 102); different dimensions -> glTexImage2D in-place.
    assert_eq!(
        s.acquire_slot(10, (200, 100, 10)),
        SlotAction::ReuseRealloc { tex: 102 }
    );
    // Same page re-rendered at new zoom reuses its own handle in-place.
    assert_eq!(
        s.acquire_slot(9, (150, 300, 99)),
        SlotAction::ReuseRealloc { tex: 101 }
    );
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_android --lib gpu::page_slots`
Expected: FAIL (`SlotAction`, `acquire_slot`, `commit_allocated` not found).

- [ ] **Step 3: Commit RED test**

```bash
git add crates/pdf_android/src/gpu/page_slots.rs
git commit -m "test(gpu): add RED test for fixed-handle GL texture recycling in PageSlots"
```

- [ ] **Step 4: Write minimal GREEN implementation**

```rust
// crates/pdf_android/src/gpu/page_slots.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotAction {
    Hit(u32),
    Allocate,
    ReuseSubImage { tex: u32 },
    ReuseRealloc { tex: u32 },
}

impl PageSlots {
    pub(crate) fn acquire_slot(&mut self, page: u32, key: BitmapKey) -> SlotAction {
        if let Some(pos) = self.slots.iter().position(|&(p, _, _)| p == page) {
            let (_, old_key, tex) = self.slots.remove(pos);
            self.slots.push((page, key, tex));
            if old_key == key {
                return SlotAction::Hit(tex);
            }
            return if (old_key.0, old_key.1) == (key.0, key.1) {
                SlotAction::ReuseSubImage { tex }
            } else {
                SlotAction::ReuseRealloc { tex }
            };
        }
        if self.slots.len() >= PAGE_SLOTS_MAX {
            let (_, old_key, tex) = self.slots.remove(0);
            self.slots.push((page, key, tex));
            return if (old_key.0, old_key.1) == (key.0, key.1) {
                SlotAction::ReuseSubImage { tex }
            } else {
                SlotAction::ReuseRealloc { tex }
            };
        }
        SlotAction::Allocate
    }

    pub(crate) fn commit_allocated(&mut self, page: u32, key: BitmapKey, tex: u32) {
        self.slots.push((page, key, tex));
    }
}
```
In `crates/pdf_android/src/gpu/textures.rs`:
- Dispatch `acquire_slot(page, key)`: `Hit(tex)` returns `tex`; `ReuseSubImage { tex }` binds `tex` and calls `glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, w, h, fmt, GL_UNSIGNED_BYTE, ptr)`; `ReuseRealloc { tex }` binds `tex` and calls `glTexImage2D`; `Allocate` calls `upload_texture` and `commit_allocated`.
- Downgrade hot-path `info!("gpu: tex create/destroy ...")` to `log::debug!()`.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_android --lib gpu::page_slots && cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings`
Expected: PASS.
```bash
git add crates/pdf_android/src/gpu/page_slots.rs crates/pdf_android/src/gpu/textures.rs
git commit -m "perf(gpu): recycle fixed GL texture handles in PageSlots via glTexSubImage2D"
```

---

### Task 3 (SF-12b): Cache `ScrollLayout` Prefix Sums & Extend `FrameTimer` (`p50`/`p95`/`p99`/`jank_pct`)

**Audit ref:** #12 | **Hard Gate:** Continuo `frame p95 ≤ 8.33 ms`, `p99 ≤ 12.5 ms`, `>8.33ms < 2%` on `large_document.pdf` (500p) and `mixed_stress.pdf` (80p).

**Files:**
- Modify: `crates/pdf_core/src/metrics.rs` (add `p50()`, `p99()`, `jank_pct(budget: Duration)` to `FrameTimer`)
- Modify: `crates/pdf_android/src/reader/scroll.rs` (add `LayoutKey` + `ScrollLayoutCache`)
- Modify: `crates/pdf_android/src/gpu/pipeline.rs:1060-1074` (log `p50`, `p95`, `p99`, `>8.3ms%`)

**Interfaces:**
- Consumes: `pdf_core::metrics::FrameTimer`, `ColumnMetrics` in `reader/scroll.rs`
- Produces: `FrameTimer::p50(&self) -> Option<Duration>`, `FrameTimer::p99(&self) -> Option<Duration>`, `FrameTimer::jank_pct(&self, budget: Duration) -> Option<f32>`, `ScrollLayoutCache::get_or_build(&mut self, key: LayoutKey, build: impl FnOnce() -> ColumnMetrics) -> &ColumnMetrics`.

- [ ] **Step 1: Write failing unit tests in `pdf_core::metrics` and `reader::scroll`**

```rust
// crates/pdf_core/src/metrics.rs (inside #[cfg(test)] mod tests)
#[test]
fn frame_timer_reports_p50_p95_p99_and_120hz_jank_pct() {
    let mut timer = FrameTimer::with_capacity(100);
    for i in 1..=100u64 {
        timer.push(Duration::from_micros(i * 100)); // 0.1 ms .. 10.0 ms
    }
    assert_eq!(timer.p50().unwrap(), Duration::from_micros(5_000));
    assert_eq!(timer.p95().unwrap(), Duration::from_micros(9_500));
    assert_eq!(timer.p99().unwrap(), Duration::from_micros(9_900));
    let jank = timer.jank_pct(Duration::from_micros(8_333)).unwrap();
    assert!((jank - 17.0).abs() < 0.01, "expected 17% > 8.333ms, got {jank}");
}

// crates/pdf_android/src/reader/scroll.rs (inside #[cfg(test)] mod tests)
#[test]
fn scroll_layout_cache_reuses_metrics_until_layout_key_changes() {
    let mut cache = ScrollLayoutCache::default();
    let key = LayoutKey { geom_gen: 1, win_w: 1440, win_h: 2200, zoom_bits: 1.0f32.to_bits() };
    let mut builds = 0usize;
    let _ = cache.get_or_build(key, || {
        builds += 1;
        ColumnMetrics::uniform(500, 1440.0, 2000.0, 24.0)
    });
    let _ = cache.get_or_build(key, || {
        builds += 1;
        ColumnMetrics::uniform(500, 1440.0, 2000.0, 24.0)
    });
    assert_eq!(builds, 1);
    let key2 = LayoutKey { geom_gen: 2, ..key };
    let _ = cache.get_or_build(key2, || {
        builds += 1;
        ColumnMetrics::uniform(500, 1440.0, 2000.0, 24.0)
    });
    assert_eq!(builds, 2);
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pdf_core metrics && cargo test -p pdf_android --lib reader::scroll`
Expected: FAIL (`p50`/`p99`/`jank_pct` and `ScrollLayoutCache` not defined).

- [ ] **Step 3: Commit RED tests**

```bash
git add crates/pdf_core/src/metrics.rs crates/pdf_android/src/reader/scroll.rs
git commit -m "test(perf): add RED tests for FrameTimer p50/p99/jank_pct and ScrollLayoutCache"
```

- [ ] **Step 4: Write minimal GREEN implementation**

```rust
// crates/pdf_core/src/metrics.rs
pub fn percentile(&self, num: usize, den: usize) -> Option<Duration> {
    if self.samples.is_empty() {
        return None;
    }
    let mut sorted = self.ordered();
    sorted.sort_unstable();
    let idx = (sorted.len() * num).div_ceil(den).saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[idx])
}

pub fn p50(&self) -> Option<Duration> { self.percentile(50, 100) }
pub fn p95(&self) -> Option<Duration> { self.percentile(95, 100) }
pub fn p99(&self) -> Option<Duration> { self.percentile(99, 100) }

pub fn jank_pct(&self, budget: Duration) -> Option<f32> {
    if self.samples.is_empty() {
        return None;
    }
    let over = self.samples.iter().filter(|&&d| d > budget).count();
    Some((over as f32 * 100.0) / (self.samples.len() as f32))
}
```
```rust
// crates/pdf_android/src/reader/scroll.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayoutKey {
    pub(crate) geom_gen: u64,
    pub(crate) win_w: i32,
    pub(crate) win_h: i32,
    pub(crate) zoom_bits: u32,
}

#[derive(Default)]
pub(crate) struct ScrollLayoutCache {
    cached: Option<(LayoutKey, ColumnMetrics)>,
}

impl ScrollLayoutCache {
    pub(crate) fn get_or_build(
        &mut self,
        key: LayoutKey,
        build: impl FnOnce() -> ColumnMetrics,
    ) -> &ColumnMetrics {
        let hit = self.cached.as_ref().is_some_and(|(k, _)| *k == key);
        if !hit {
            self.cached = Some((key, build()));
        }
        &self.cached.as_ref().unwrap().1
    }

    pub(crate) fn clear(&mut self) {
        self.cached = None;
    }
}
```
Wire `ScrollLayoutCache` into `Reader` (bump `scroll_geom_gen` on `scroll_geoms` mutation and clear in `release_document_state`), and update `gpu/pipeline.rs:1066-1073` to log `frame p50={:.1}ms p95={:.1}ms p99={:.1}ms >8.3ms={:.1}% ({} frames)`.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_core metrics && cargo test -p pdf_android --lib reader::scroll`
Expected: PASS.
```bash
git add crates/pdf_core/src/metrics.rs crates/pdf_android/src/reader/scroll.rs crates/pdf_android/src/gpu/pipeline.rs crates/pdf_android/src/reader/
git commit -m "perf(scroll): cache ColumnMetrics prefix sums and log frame p50/p95/p99 telemetry"
```

---

### Task 4 (SF-13): Skip No-Op Cold-Start `recreate_surface()` & Deduplicate `load_progress()`

**Audit ref:** #13 | **Hard Gate:** Cold start `TotalTime` `649 ms` → **`≤ 500 ms`**, single `EGL/GLES2 ready 1440x2200` on launch.

**Files:**
- Modify: `crates/pdf_android/src/gpu/surface.rs:341-410`
- Modify: `crates/pdf_android/src/reader/life.rs` & `crates/pdf_android/src/reader/library.rs`

**Interfaces:**
- Produces: `pub(crate) fn should_recreate_surface(has_surface: bool, cur: (i32, i32), next: (i32, i32)) -> bool`, `pub(crate) fn should_reload_library_progress(already_loaded: bool) -> bool`.

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/gpu/surface.rs (inside #[cfg(test)] mod tests)
#[test]
fn surface_recreation_is_skipped_when_surface_already_matches_dimensions() {
    assert!(!should_recreate_surface(true, (1440, 2200), (1440, 2200)));
    assert!(should_recreate_surface(false, (1440, 2200), (1440, 2200)));
    assert!(should_recreate_surface(true, (1440, 2200), (2200, 1440)));
    assert!(should_recreate_surface(true, (0, 0), (1440, 2200)));
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_android --lib gpu::surface`
Expected: FAIL (`should_recreate_surface` not found).

- [ ] **Step 3: Commit RED test**

```bash
git add crates/pdf_android/src/gpu/surface.rs
git commit -m "test(gpu): add RED test for skipping redundant same-size recreate_surface"
```

- [ ] **Step 4: Write minimal GREEN implementation**

```rust
// crates/pdf_android/src/gpu/surface.rs
pub(crate) fn should_recreate_surface(
    has_surface: bool,
    cur: (i32, i32),
    next: (i32, i32),
) -> bool {
    !has_surface || cur != next || cur.0 <= 0 || cur.1 <= 0
}
```
Call `if !should_recreate_surface(self.has_surface(), (self.surf_w, self.surf_h), (w, h)) { return; }` at the top of `recreate_surface()`, and avoid calling `load_progress()` a second time during initial startup when `lib_progress_loaded` is already true.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_android --lib gpu::surface && cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings`
Expected: PASS.
```bash
git add crates/pdf_android/src/gpu/surface.rs crates/pdf_android/src/reader/life.rs crates/pdf_android/src/reader/library.rs
git commit -m "perf(startup): skip no-op same-size EGL recreate_surface and duplicate library.json parse"
```

---

### Task 5 (SF-14): Drop CPU `scale_bitmap` From Interactive Zoom Paths

**Audit ref:** #14 | **Hard Gate:** Zero interactive zoom calls to CPU `scale_bitmap` (72–285 ms on ARM64); GPU bilinear quad scaling handles pinch preview until MuPDF crisp re-render completes.

**Files:**
- Inspect & Modify: `crates/pdf_core/src/bitmap.rs` and any callers in `crates/pdf_android/src/` / `crates/pdf_app/src/`

**Interfaces:**
- Consumes: `grep -rn "scale_bitmap" crates/`
- Produces: Clean interactive zoom path that never allocates or runs CPU upscaling; `scale_bitmap` stays scoped only to `pdf_bench` comparison or removed if unused in app crates.

- [ ] **Step 1: Write failing characterization test**

Verify callers via `grep -rn "scale_bitmap" crates/pdf_android/ crates/pdf_app/`. Add a test in `crates/pdf_android/src/reader/pinch.rs` verifying that `pinch_preview_uses_gpu_scale_without_cpu_resampling`:

```rust
// crates/pdf_android/src/reader/pinch.rs (inside #[cfg(test)] mod tests)
#[test]
fn pinch_zoom_changes_viewport_scale_without_invalidating_cached_bitmap_until_release() {
    let base_scale = 1.0f32;
    let live_scale = (base_scale * 2.0).clamp(0.5, 4.0);
    assert_eq!(live_scale, 2.0);
    // Confirm crates/pdf_android has zero references to pdf_core::scale_bitmap.
    let src = include_str!("pinch.rs");
    assert!(!src.contains("pdf_core::scale_bitmap"));
}
```

- [ ] **Step 2: Run test to verify RED (or audit callers)**

Run: `grep -rn "scale_bitmap" crates/` and `cargo test -p pdf_android --lib reader::pinch`. If `pdf_app` or `pdf_android` still calls `scale_bitmap` in interactive zoom, remove the caller first with a failing behavior test asserting GPU/display-list zoom path is taken.

- [ ] **Step 3: Write minimal GREEN change + Verify + Commit**

Remove any interactive caller of `scale_bitmap` in `pdf_android` / `pdf_app` (keeping `pdf_bench` baseline comparison intact or explicitly annotated `#[doc(hidden)]` for benchmarks only), run `cargo test -p pdf_core && cargo test -p pdf_app && cargo test -p pdf_android --lib`, and commit:
```bash
git add crates/pdf_core/src/ crates/pdf_android/src/ crates/pdf_app/src/
git commit -m "perf(zoom): ensure interactive zoom uses GPU quad scaling instead of CPU scale_bitmap"
```

---

## Track B — Crash Resilience, Diagnostics & Atomic I/O Hardening (Audit #15, #16, + Crash Diagnostics)

> **Note on `$diagnose-crash`:** Direct access to `config/skills/diagnose-crash/SKILL.md` was restricted by the workspace sandbox hook, so Track B applies standard Linux/Android native crash engineering: crash-safe atomic DEX/state persistence, finite retry caps, zero-panic edge guards, and privacy-safe crash breadcrumb capture + `adb logcat`/`tombstone` triage commands.

### Task 6 (SF-15): Unify Atomic Writers, Atomic `classes.dex`, `0600` Sync Config & Deterministic Clock

**Audit ref:** #15 (plus `folder_sync.json` `0666` mode from audit footer).

**Files:**
- Modify: `crates/pdf_android/src/jni/ime.rs:45-70` (extract pure host-testable `ensure_dex_file(dir: &Path, bytes: &[u8]) -> std::io::Result<PathBuf>` using `write_atomic` + `0o444`)
- Modify: `crates/pdf_android/src/folder_sync.rs:752,1233-1254` (delete duplicate `write_json_atomic`, replace `Instant::now()` in backoff test with explicit injected base `Instant`)
- Modify: `crates/pdf_core/src/folder_sync.rs:1143` (delete duplicate `atomic_json`, delegate to `crate::fsutil::write_atomic`)
- Modify: `crates/pdf_android/src/jni/launch.rs:255`, `crates/pdf_android/src/reader/library.rs:834,980`, `crates/pdf_android/src/reader/discover.rs:177` (switch raw `fs::write` → `pdf_core::write_atomic`)

**Interfaces:**
- Consumes: `pdf_core::write_atomic`, `pdf_core::write_atomic_private`
- Produces: `pub(crate) fn ensure_dex_file(dir: &Path, dex_bytes: &[u8]) -> std::io::Result<PathBuf>` (host-testable, writes atomically and marks `0o444`).

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/jni/ime.rs (inside #[cfg(test)] mod tests, compiled on host)
#[test]
fn ensure_dex_file_writes_atomically_readonly_without_leaving_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let dex = ensure_dex_file(dir.path(), b"dex\n035\0test").unwrap();
    assert_eq!(std::fs::read(&dex).unwrap(), b"dex\n035\0test");
    assert!(std::fs::metadata(&dex).unwrap().permissions().readonly());
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "atomic write must leave no temp files");
}
```
And in `crates/pdf_android/src/folder_sync.rs:1233-1254`, assert backoff scheduling using a single pinned `base = Instant::now()` passed as parameter (`compute_retry_deadline(base, delay)`), eliminating wall-clock drift between calls.

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_android --lib jni::ime::tests`
Expected: FAIL (`ensure_dex_file` does not exist).

- [ ] **Step 3: Commit RED tests**

```bash
git add crates/pdf_android/src/jni/ime.rs crates/pdf_android/src/folder_sync.rs
git commit -m "test(io): add RED test for atomic readonly classes.dex extraction"
```

- [ ] **Step 4: Write minimal GREEN implementation**

```rust
// crates/pdf_android/src/jni/ime.rs
pub(crate) fn ensure_dex_file(dir: &Path, dex_bytes: &[u8]) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let dex_path = dir.join("classes.dex");
    if !dex_path.is_file() {
        pdf_core::write_atomic(&dex_path, dex_bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let meta = fs::metadata(&dex_path)?;
        if !meta.permissions().readonly() {
            fs::set_permissions(&dex_path, fs::Permissions::from_mode(0o444))?;
        }
    }
    Ok(dex_path)
}
```
- Delete `fn write_json_atomic` in `crates/pdf_android/src/folder_sync.rs:752` and `fn atomic_json` in `crates/pdf_core/src/folder_sync.rs:1143`; serialize with `serde_json::to_vec_pretty` and write via `pdf_core::write_atomic` (and `write_atomic_private` for `folder_sync.json` so mode is `0600`, not `0666`).
- Replace the 4 remaining raw `fs::write` calls in `launch.rs:255`, `library.rs:834,980`, `discover.rs:177` with `pdf_core::write_atomic`.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_core && cargo test -p pdf_android --lib && grep -rn "fn write_json_atomic\|fn atomic_json" crates/ || true`
Expected: All tests PASS, 0 duplicate helpers.
```bash
git add crates/pdf_core/src/folder_sync.rs crates/pdf_android/src/jni/ime.rs crates/pdf_android/src/jni/launch.rs crates/pdf_android/src/folder_sync.rs crates/pdf_android/src/reader/library.rs crates/pdf_android/src/reader/discover.rs
git commit -m "fix(io): unify atomic writers, write classes.dex atomically, and restrict folder_sync.json to 0600"
```

---

### Task 7 (CR-01): Crash-Resilience Edge Guards & Privacy-Safe Native Crash Breadcrumbs

**Audit ref:** Audit §18 footer (1-page negative scroll guard, corrupt page-0 infinite blank retry cap, empty `Up` gesture guard, UTF-8-safe notebook backup) + native crash diagnosis breadcrumbs.

**Files:**
- Create: `crates/pdf_android/src/crash_diag.rs` (privacy-safe `CrashBreadcrumb` ring + atomic `last_crash.json` writer — stores ONLY `{ phase, ui_mode, page_idx, page_count, panic_location }`, NEVER file paths, note text, or API keys)
- Modify: `crates/pdf_android/src/reader/render_failures.rs` (cap retries at `MAX_PAGE_RETRIES = 5` so a permanently corrupt page stops spinning the worker and renders `SlotDraw::Error`)
- Modify: `crates/pdf_android/src/reader/scroll.rs:147-175` (clamp 1-page scroll offset `>= 0.0`)
- Modify: `crates/pdf_android/src/reader/viewer.rs:708-722` (ignore empty `Up` gesture without panicking or mutating state)
- Modify: `crates/pdf_android/src/lib.rs` (install `std::panic::set_hook` recording privacy-safe breadcrumb to `internal_dir/last_crash.json` before abort)

**Interfaces:**
- Produces: `MAX_PAGE_RETRIES: u32 = 5`, `RenderFailures::is_exhausted(&self, page: u32) -> bool`, `CrashBreadcrumb`, `write_crash_breadcrumb(internal_dir: &Path, crumb: &CrashBreadcrumb) -> std::io::Result<()>`.

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/reader/render_failures.rs (inside #[cfg(test)] mod tests)
#[test]
fn page_stops_retrying_after_max_consecutive_failures() {
    let mut rf = RenderFailures::default();
    let mut t = Instant::now();
    for _ in 0..MAX_PAGE_RETRIES {
        rf.fail(0, t);
        t += RETRY_MAX + Duration::from_secs(1);
    }
    assert!(rf.is_exhausted(0));
    assert!(!rf.should_retry(0, t), "corrupt page must not loop forever");
    assert_eq!(slot_draw(false, rf.is_failed(0)), SlotDraw::Error);
}

// crates/pdf_android/src/crash_diag.rs (inside #[cfg(test)] mod tests)
#[test]
fn crash_breadcrumb_persists_atomically_and_never_includes_sensitive_strings() {
    let dir = tempfile::tempdir().unwrap();
    let crumb = CrashBreadcrumb {
        phase: "gl_present",
        ui_mode: "Viewer",
        page_idx: 7,
        page_count: 80,
        location: "gpu/pipeline.rs:412".to_string(),
    };
    write_crash_breadcrumb(dir.path(), &crumb).unwrap();
    let raw = std::fs::read_to_string(dir.path().join("last_crash.json")).unwrap();
    assert!(raw.contains("\"phase\": \"gl_present\""));
    assert!(raw.contains("\"page_idx\": 7"));
    assert!(!raw.contains("gemini_api_key"));
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pdf_android --lib reader::render_failures && cargo test -p pdf_android --lib crash_diag`
Expected: FAIL (`MAX_PAGE_RETRIES`, `is_exhausted`, `crash_diag` missing).

- [ ] **Step 3: Commit RED tests**

```bash
git add crates/pdf_android/src/reader/render_failures.rs crates/pdf_android/src/crash_diag.rs
git commit -m "test(crash): add RED tests for bounded render retries and privacy-safe crash breadcrumbs"
```

- [ ] **Step 4: Write minimal GREEN implementation**

1. Add `pub(crate) const MAX_PAGE_RETRIES: u32 = 5;` and return `false` in `should_retry()` when `failures >= MAX_PAGE_RETRIES`.
2. Implement `crates/pdf_android/src/crash_diag.rs` with `CrashBreadcrumb` + `write_crash_breadcrumb` (via `pdf_core::write_atomic`), and register a panic hook in `lib.rs` that writes `last_crash.json` and logs `"FATAL_PANIC location=... phase=..."` for instant `adb logcat -d -s pdf_android:E DEBUG:F` triage.
3. Clamp 1-page scroll offset in `scroll.rs` to `0.0..=max_offset.max(0.0)` and guard empty `Up` gesture in `viewer.rs`.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_android --lib`
Expected: PASS.
```bash
git add crates/pdf_android/src/crash_diag.rs crates/pdf_android/src/reader/render_failures.rs crates/pdf_android/src/reader/scroll.rs crates/pdf_android/src/reader/viewer.rs crates/pdf_android/src/lib.rs
git commit -m "fix(crash): cap broken-page retry loop, clamp scroll/gesture edges, and record privacy-safe crash breadcrumbs"
```

---

### Task 8 (SF-16): Close CI Blind Spots

**Audit ref:** #16 | **Files:** `.github/workflows/ci.yml`

- [ ] **Step 1: Verify current gap**

Run: `grep -n "pdf_app\|pdf_android --lib --tests\|ink_bench/tests" .github/workflows/ci.yml`
Expected: 0 matches.

- [ ] **Step 2: Add the 3 missing checks to `.github/workflows/ci.yml`**

```yaml
      - name: clippy pdf_android (host)
        run: cargo clippy -p pdf_android --lib --tests -- -D warnings
      - name: test pdf_app
        run: cargo test -p pdf_app
      - name: test python ink_bench
        run: PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests
```

- [ ] **Step 3: Run all 3 commands locally to verify GREEN + Commit**

Run: `cargo clippy -p pdf_android --lib --tests -- -D warnings && cargo test -p pdf_app && PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests -v`
Expected: PASS.
```bash
git add .github/workflows/ci.yml
git commit -m "ci: add pdf_app tests, host pdf_android clippy, and ink_bench Python tests"
```

---

## Track C — Visual Polish & Relationship-Centric Agentic UX (Audit #17–#18 + Agentic UX)

### Task 9 (UX-01 / Audit #17–#18): Dead Code Cleanup, Contrast, Halo, Narrow Layouts & Stale Docs

**Audit ref:** #17, #18.

**Files:**
- Modify: `crates/pdf_android/src/input/motion/library.rs:28-32,336-367,425,439-447`
- Modify: `crates/pdf_android/src/cache.rs:139`
- Modify: `crates/pdf_android/src/theme.rs` & `crates/pdf_android/src/draw/chrome.rs:154-162`
- Modify: `crates/pdf_android/src/draw/study.rs:205-235`
- Modify: `crates/pdf_android/src/reader/geometry.rs:315-317`
- Modify: `crates/pdf_android/src/gpu/textures.rs` (zero RGB on transparent border pixels before upload to eliminate cyan halo)
- Modify: `.agents/skills/pdflector-rendimiento/SKILL.md:89-96,161-162,251`

**Interfaces:**
- Produces: `pub(crate) fn close_library_menus_on_tab_change(view_open: &mut bool, settings_open: &mut bool)`, clamped `sheet_h(win_h)` and non-overlapping `study_layout_for(...)` buttons down to `480×640`, WCAG AA `>= 3.0:1` contrast for `DefaultDark` `"Aa"` accent underline.

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/theme.rs (inside #[cfg(test)] mod tests)
#[test]
fn default_dark_accent_underline_meets_wcag_aa_ui_contrast() {
    let p = Palette::for_theme(AppTheme::DefaultDark);
    let ratio = contrast_ratio(p.accent, p.chrome_bg);
    assert!(ratio >= 3.0, "DefaultDark accent vs chrome_bg contrast {ratio:.2} < 3.0");
}

// crates/pdf_android/src/draw/study.rs (inside #[cfg(test)] mod tests)
#[test]
fn key_editor_buttons_do_not_overlap_on_520px_narrow_viewport() {
    let layout = study_layout_for(520, 900, &StudyLayoutInput::key_editor_with_key());
    for i in 0..layout.buttons.len() {
        for j in (i + 1)..layout.buttons.len() {
            let a = layout.buttons[i].1;
            let b = layout.buttons[j].1;
            let overlaps = a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3;
            assert!(!overlaps, "buttons {:?} and {:?} overlap on 520px viewport", layout.buttons[i].0, layout.buttons[j].0);
        }
    }
}

// crates/pdf_android/src/reader/geometry.rs (inside #[cfg(test)] mod tests)
#[test]
fn sheet_height_never_exceeds_viewport_on_short_windows() {
    assert!(sheet_h(640) <= 600);
    assert!(sheet_h(2200) >= 760);
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `cargo test -p pdf_android --lib theme::tests && cargo test -p pdf_android --lib draw::study && cargo test -p pdf_android --lib reader::geometry`
Expected: FAIL (`2.94 < 3.0`, overlapping buttons at `520 px`, `sheet_h(640) == 760 > 600`).

- [ ] **Step 3: Commit RED tests**

```bash
git add crates/pdf_android/src/theme.rs crates/pdf_android/src/draw/study.rs crates/pdf_android/src/reader/geometry.rs
git commit -m "test(ui): add RED tests for DefaultDark contrast, narrow key-editor layout, and short-window sheet height"
```

- [ ] **Step 4: Write minimal GREEN implementation**

1. Lighten `DefaultDark` accent slightly so `contrast_ratio(accent, chrome_bg) >= 3.2`.
2. In `draw/study.rs:212-223`, size Save/Cancel/ClearKey dynamically from available `inner_w = (right - x - 2 * STUDY_PAD)` with `GAP` so buttons never overlap down to `480 px`.
3. In `reader/geometry.rs:315-317`, clamp `sheet_h(win_h)` to `min((win_h - 40).max(240), (win_h as f32 * 0.46).clamp(760.0, 1040.0).round() as i32)`.
4. Delete dead `lib_org_chips` and stale `#[allow(dead_code)]` on `cache.rs:139`; align search-chip hitbox; close open library menus on `enter_discover`; zero RGB channels where `alpha == 0` in overlay upload; remove deleted `google_key.txt` instructions from `.agents/skills/pdflector-rendimiento/SKILL.md`.

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_android --lib && cargo clippy -p pdf_android --lib --tests -- -D warnings`
Expected: PASS.
```bash
git add crates/pdf_android/src/theme.rs crates/pdf_android/src/draw/study.rs crates/pdf_android/src/reader/geometry.rs crates/pdf_android/src/input/motion/library.rs crates/pdf_android/src/cache.rs crates/pdf_android/src/gpu/textures.rs .agents/skills/pdflector-rendimiento/SKILL.md
git commit -m "fix(ui,cleanup): meet WCAG AA contrast, clamp narrow modals, fix popup halo, and prune dead code"
```

---

### Task 10 (AG-01 / Agentic UX Pillar 1 & 3): Cross-Session Study Relationship Memory & Forgetting Controls

**Agentic UX rationale:** Students return to the same textbook/paper across weeks. Treating state as static `(page, zoom)` forces them to re-orient every session. Adding a lightweight, local-only, user-controlled **Reading Relationship Model** gives cross-session continuity ("Resumed at p. 7 · 3 notes · last studied 2d ago") with explicit **Memory Visualization** and **Forgetting Controls** ("Clear study memory for this book"), keeping 100% privacy on-device.

**Files:**
- Create: `crates/pdf_android/src/study_memory.rs` (pure, host-testable `ReadingRelationship` + `SessionSummary` + `forget_book()`)
- Modify: `crates/pdf_android/src/persist.rs` (atomic persistence in `study_memory.json`, excluded from cloud backup if desired or stored alongside `notebooks.json`)
- Modify: `crates/pdf_android/src/reader/study.rs` (surface non-intrusive resume summary pill & "Olvidar memoria" action in Study panel)

**Interfaces:**
- Consumes: `BookProgress` (`persist.rs`), `Notebook` (`notebook.rs`)
- Produces:
  - `pub(crate) struct ReadingRelationship { pub(crate) path: String, pub(crate) sessions: u32, pub(crate) preferred_mode: ReadingMode, pub(crate) last_study_page: Option<u32>, pub(crate) last_opened_unix: i64 }`
  - `pub(crate) fn resume_banner(rel: &ReadingRelationship, note_count: usize, now_unix: i64) -> Option<String>`
  - `pub(crate) fn forget_document(store: &mut RelationshipStore, path: &str) -> bool`

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/study_memory.rs (inside #[cfg(test)] mod tests)
#[test]
fn relationship_records_continuity_and_forgetting_wipes_book_completely() {
    let mut store = RelationshipStore::default();
    store.record_session("/books/calc.pdf", ReadingMode::Scroll, Some(14), 1_700_000_000);
    store.record_session("/books/calc.pdf", ReadingMode::Scroll, Some(22), 1_700_086_400);

    let rel = store.get("/books/calc.pdf").expect("relationship stored");
    assert_eq!(rel.sessions, 2);
    assert_eq!(rel.last_study_page, Some(22));

    let banner = resume_banner(rel, 3, 1_700_172_800).expect("returning reader gets banner");
    assert!(banner.contains("pág. 23")); // 1-based display
    assert!(banner.contains("3 notas"));

    // Forgetting control gives full user agency over memory.
    assert!(forget_document(&mut store, "/books/calc.pdf"));
    assert!(store.get("/books/calc.pdf").is_none());
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_android --lib study_memory`
Expected: FAIL (`study_memory` module does not exist).

- [ ] **Step 3: Commit RED test**

```bash
git add crates/pdf_android/src/study_memory.rs
git commit -m "test(ux): add RED tests for cross-session reading relationship memory and forgetting controls"
```

- [ ] **Step 4: Write minimal GREEN implementation**

Implement `crates/pdf_android/src/study_memory.rs` with atomic save/load (`pdf_core::write_atomic`), wire `record_session` into `open_pdf_at` and `StudyState::save_note`, and expose the one-tap "Olvidar historial" control inside `StudyState` (following the existing two-tap delete confirmation pattern so forgetting is deliberate and reversible before confirm).

- [ ] **Step 5: Verify GREEN + Commit**

Run: `cargo test -p pdf_android --lib study_memory && cargo test -p pdf_android --lib reader::study`
Expected: PASS.
```bash
git add crates/pdf_android/src/study_memory.rs crates/pdf_android/src/reader/study.rs crates/pdf_android/src/lib.rs
git commit -m "feat(ux): add local reading relationship memory, resume cue, and per-book forgetting control"
```

---

### Task 11 (AG-02 / Agentic UX Pillar 2 & 5): Three-Stage Trust Evolution & Recovery for Gemini Study

**Agentic UX rationale:** AI features earn trust gradually (Stage 1: **Transparency** → Stage 2: **Selective Disclosure** → Stage 3: **Streamlined/Autonomous**). Currently Gemini queries jump straight to sending context without showing what leaves the device or giving a recovery/rollback loop. Adding a pure `TrustProfile` state machine lets first-time users inspect exact outbound context (`page + text + crop`), graduates to compact disclosure after consistent accepted queries, resets immediately on any correction/undo (**Trust Recovery**), and tracks local Relationship Quality metrics (`accepted_answers`, `reverted_answers`, `trust_stage`).

**Files:**
- Modify: `crates/pdf_android/src/reader/toast_ia.rs` (add `TrustStage`, `TrustProfile`, `OutboundContextPreview`, and trust-recovery transitions)
- Modify: `crates/pdf_android/src/reader/study.rs` (show disclosure badge & one-tap stage toggle / undo in Study panel)

**Interfaces:**
- Produces:
  - `pub(crate) enum TrustStage { Transparent, Selective, Streamlined }`
  - `pub(crate) struct TrustProfile { pub(crate) stage: TrustStage, pub(crate) accepted_streak: u32, pub(crate) corrections: u32, pub(crate) user_locked_stage: Option<TrustStage> }`
  - `TrustProfile::on_accept(&mut self)` (graduates `Transparent` → `Selective` after 3 accepted queries; never auto-promotes past `Selective` without user opt-in)
  - `TrustProfile::on_correction(&mut self)` (immediately drops back to `Transparent` and resets `accepted_streak = 0` — Trust Recovery protocol)
  - `TrustProfile::disclosure_summary(&self, page: u32, has_text: bool, has_crop: bool) -> Option<String>`

- [ ] **Step 1: Write failing host unit tests**

```rust
// crates/pdf_android/src/reader/toast_ia.rs (inside #[cfg(test)] mod tests)
#[test]
fn trust_profile_graduates_from_transparent_to_selective_and_recovers_on_correction() {
    let mut tp = TrustProfile::default();
    assert_eq!(tp.stage(), TrustStage::Transparent);
    let preview = tp.disclosure_summary(4, true, true).expect("stage 1 always discloses");
    assert!(preview.contains("pág. 5") && preview.contains("texto") && preview.contains("recorte"));

    for _ in 0..3 {
        tp.on_accept();
    }
    assert_eq!(tp.stage(), TrustStage::Selective);

    // A single user correction triggers Trust Recovery back to full Transparency.
    tp.on_correction();
    assert_eq!(tp.stage(), TrustStage::Transparent);
    assert_eq!(tp.accepted_streak, 0);
    assert_eq!(tp.corrections, 1);

    // Explicit user preference locks transparency level (progressive disclosure control).
    tp.lock_stage(Some(TrustStage::Transparent));
    for _ in 0..10 {
        tp.on_accept();
    }
    assert_eq!(tp.stage(), TrustStage::Transparent);
}
```

- [ ] **Step 2: Run test to verify RED**

Run: `cargo test -p pdf_android --lib reader::toast_ia`
Expected: FAIL (`TrustProfile`, `TrustStage` not defined).

- [ ] **Step 3: Commit RED test**

```bash
git add crates/pdf_android/src/reader/toast_ia.rs
git commit -m "test(ux): add RED tests for three-stage Gemini trust evolution and correction recovery"
```

- [ ] **Step 4: Write minimal GREEN implementation**

Implement `TrustStage` and `TrustProfile` in `crates/pdf_android/src/reader/toast_ia.rs` (~55 lines), render `disclosure_summary(...)` inside the IA toast / Study prompt header, and wire "Deshacer / Mostrar detalles" to `on_correction()`.

- [ ] **Step 5: Verify GREEN + Final 7-Gate CI & TCL 9469X Hardware Verification**

Run all 7 CI gates, rebuild `app-debug.apk` with release ARM64 `libpdf_android.so`, and verify on TCL NXTPaper 11 Plus (`9469X`) using isolated `bench_corpus/`:
1. Cold start `am start -W` median `≤ 500 ms` (1× `EGL/GLES2 ready`).
2. 30-swipe Continuo on `mixed_stress.pdf` & `large_document.pdf`: `TOTAL PSS ≤ 220 MiB`, `GL mtrack ≤ 80 MiB`, `frame p95 ≤ 8.33 ms`, `p99 ≤ 12.5 ms`, `>8.3ms < 2%`.
3. `adb logcat -d -s pdf_android:W AndroidRuntime:E art:E DEBUG:F libc:F` = **0 lines**; `last_crash.json` absent.
4. Restore all 4 user JSONs, delete `bench_corpus/` & `.audit_backup/`, reset display settings, and append results to `docs/benchmark-results.md`.

```bash
git add crates/pdf_android/src/reader/toast_ia.rs crates/pdf_android/src/reader/study.rs
git commit -m "feat(ux): add graduated trust disclosure and recovery for Gemini study queries"
```

---

## Self-Review Coverage Matrix

| Source Requirement | Covered By |
|---|---|
| Audit #11 (`fz_store` 461.8 MB PSS spike) | Task 1 (`SF-11`) |
| Audit #12 (`PageSlots` GL churn + 120 Hz `frame p95`) | Task 2 (`SF-12a`) + Task 3 (`SF-12b`) |
| Audit #13 (3× cold-start `recreate_surface` + double `library.json`) | Task 4 (`SF-13`) |
| Audit #14 (CPU `scale_bitmap` 5× slower than MuPDF) | Task 5 (`SF-14`) |
| Audit #15 (duplicate atomic writers, raw `classes.dex`, `0666` sync config, clock test) | Task 6 (`SF-15`) |
| Audit #16 (`ci.yml` missing `pdf_app`, host `pdf_android` clippy, `ink_bench` Python) | Task 8 (`SF-16`) |
| Audit #17–#18 (dead `lib_org_chips`, `DefaultDark` contrast, halo, narrow modals, stale `SKILL.md`) | Task 9 (`UX-01`) |
| Crash resilience (retry cap, 1-page scroll, empty `Up`, panic breadcrumbs) | Task 7 (`CR-01`) |
| Agentic UX (Memory Revolution, Forgetting Controls, 3-Stage Trust Evolution, Recovery, Metrics) | Task 10 (`AG-01`) + Task 11 (`AG-02`) |
