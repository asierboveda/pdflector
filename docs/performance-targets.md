# PDFLector — Hardware-Grounded Performance Target Study (TCL NXTPaper 11 Plus / 9469X)

**Date:** 2026-11-27  
**Baseline measured against:** `origin/main` @ `fb2cc5c` (`docs/benchmark-results.md`)

---

## 1. Hardware Envelope — TCL NXTPaper 11 Plus (`9469X`)

| Resource | Physical Spec | Engineering Budget Derived |
|---|---|---|
| **Display** | `1440×2200` (`3.168 Mpx`), `120 Hz` active (`mActiveModeId=1`), `60 Hz` idle | `120 Hz` vsync = **`8.33 ms`** (rendering + upload + swap must stay `< 6.5 ms` CPU+GPU to absorb SurfaceFlinger jitter). `60 Hz` vsync = **`16.67 ms`**. |
| **SoC / GPU** | MediaTek Helio G100 (`8× arm64-v8a`: 2× Cortex-A76 + 6× Cortex-A55), Mali-G57 MC2 (GLES 3.2, app uses GLES2/EGL) | 2 big cores handle render-worker + UI thread cleanly; Mali-G57 MC2 is bandwidth-bound on full-screen `glTexImage2D` (~10–12.7 MB RGBA8 uploads stall 4–8 ms if unpooled or allocated fresh every page). |
| **RAM** | `8 GB` LPDDR4X (~`3.9 GB` available to foreground apps under Android 16 SDK 36; target headroom for 4–6 GB tablets too) | 1 full-screen RGBA8888 texture = `1440 × 2200 × 4 = 12.67 MB`. Dual FBOs (`dry` + `wet`) = `25.34 MB`. 4 Continuo `PageSlots` at 1× = `~43 MB`. Uncapped MuPDF `fz_store` defaults to **`256 MB`** (dominated our `461.8 MB` PSS spike). `dlcache` budget = `24 MB` / 16 pages. |
| **Human Perception Thresholds** | Nielsen / Android Vitals / Stylus latency literature | Tap feedback `< 50 ms`; page flip `< 100 ms`; Continuo scroll/fling `p95 < 8.33 ms` (zero visible stutter at 120 Hz) and `p99 < 16.67 ms`; front-buffered wet-ink motion-to-photon `< 16 ms` (stretch `< 8.33 ms`); cold launch to interactive frame `< 500 ms` (Android Vitals good threshold). |

---

## 2. Master Target Table (Baseline vs. Old Spec vs. Recommended Targets)

| # | Component & Metric | Measured Baseline (TCL 9469X) | Old Spec | Recommended Target (Hard Gate) | Stretch Goal | Bottleneck & Why This Number |
|---|---|---|---|---|---|---|
| 1 | **Cold launch `TotalTime`** (ms) | **`649 ms`** median (`633–759 ms`) | `< 1000 ms` | **`≤ 500 ms`** | `≤ 350 ms` | `recreate_surface` destroys/recreates EGLSurface + Dual FBOs 3× at identical `1440×2200` (~150–200 ms wasted) + duplicate `library.json` load. Cutting redundant surface churn hits `≤ 500 ms` without changing startup architecture. |
| 2 | **PDF open time** (`paper` / `dense` / `large 500p` / `mixed 80p`) | `0.20 / 0.50 / 0.32 ms` (`mixed`: `SIN MEDIR` in CLI) | `< 100 ms` | **`≤ 10 ms`** (all ≤ 500p) | `≤ 2 ms` | Already 200× faster than old spec (`< 0.5 ms` on ARM64). Keep tight regression gate (`≤ 10 ms`) to catch accidental eager page-tree walks. |
| 3 | **MuPDF `render1x` / `render2x`** (text / vector / scanned) | Text/large: **`10.3–13.2 ms`** / `39.0–44.0 ms`; Scanned: **`31.1 ms`** / `122.1 ms` | `< 50 ms` (1×) | Text/vector `1× ≤ 16 ms`, `2× ≤ 50 ms`; Scanned `1× ≤ 35 ms`, `2× ≤ 130 ms` | Text `1× ≤ 10 ms`, `2× ≤ 35 ms` | Pure CPU rasterization on Cortex-A76 off the UI thread. Already comfortably fast because it runs asynchronously in `render-worker`; `1× ≤ 16 ms` ensures prefetch stays ahead of 60–120 Hz flings. |
| 4 | **Zoom fast upscale (`scale_bitmap`) vs. crisp re-render (`2×` / `4×`)** | CPU `scale2x`: **`71.9 ms`**, `scale4x`: **`285.3 ms`** vs. MuPDF `rerender1`: **`14.7 ms`**, `rerender2`: **`55.0 ms`** | None | **Eliminate CPU `scale_bitmap` from zoom (`0 ms` CPU; GPU quad `< 1 ms`)**; crisp `2× ≤ 55 ms`, `4× ≤ 90 ms` | Crisp `2× ≤ 40 ms` (viewport-clipped tile) | CPU upscaling is **5× slower** than MuPDF re-render. GLES2 bilinear quad scaling costs `< 0.3 ms` on Mali-G57 MC2 while waiting for MuPDF. |
| 5 | **Frame time `p50 / p95 / p99`** & `% janky` (Continuo drag/fling, pinch, page turn) | Continuo 30-swipe **`p95 = 11.8–14.6 ms`** (`p50/p99`: `SIN MEDIR`, `gfxinfo`: `0` due to EGL direct) | `< 16.6 ms` (60 Hz) | **`p95 ≤ 8.33 ms`** (120 Hz), `p99 ≤ 12.5 ms`, `< 2%` frames `> 8.33 ms` | `p50 ≤ 4.0 ms`, `p95 ≤ 7.0 ms`, `p99 ≤ 8.33 ms` | Unpooled `PageSlots` texture churn (`glGenTextures` + `glTexImage2D` + `glDeleteTextures` + per-tex `info!()` log) plus O(N) `scroll_layout` rebuilds per frame push `p95` 42–75% above the `8.33 ms` 120 Hz budget. |
| 6 | **`dlcache` hit ratio, eviction rate & byte calibration** | Forward 30-swipe: `entries=16`, `bytes=0.31–0.56 MB`, `hits=0 / misses=80–84`; Relaunch: `hits=1 / misses=3`; **actual RSS delta `+192 MB` vs `0.56 MB` reported (`340×` under-estimate)** | `24 MiB` / 16 pages | Calibration ratio within `2×` of native heap delta; back-and-forth scroll hit rate **`≥ 70%`**; MuPDF `fz_store` capped **`≤ 48 MiB`** | Back-and-forth hit rate `≥ 85%`; MuPDF `fz_store ≤ 32 MiB` | `dlcache` only accounts for display-list command structs, ignoring MuPDF's decoded image/font store (`256 MiB` default). Bounding `fz_store` closes the 340× gap. |
| 7 | **Memory PSS & RSS** (idle, heavy Continuo `mixed_stress`, 15-cycle Library↔Viewer slope) | Idle: **`176–184 MiB` PSS**; Continuo `mixed_stress`: **`461.8 MB` PSS** (`586.8 MB` RSS); `large_document`: **`285.0 MiB` PSS**; 8-trip Library↔Viewer: **`284.1 MiB` peak**, **`+9.9 MiB/trip` retained** (`+61.6 MiB` Graphics) | `< 200 MB` PSS | Idle **`≤ 150 MiB` PSS**; Heavy Continuo (`mixed_stress`) **`≤ 220 MiB` PSS** (`≤ 340 MiB` RSS); Library↔Viewer leak slope **`≤ 0.5 MiB/cycle`** (steady state after cycle 2) | Idle `≤ 120 MiB` PSS; Heavy Continuo `≤ 185 MiB` PSS; slope `≤ 0.1 MiB/cycle` | Three independent leaks/spikes: (a) uncapped MuPDF `fz_store` (`+192 MB`), (b) unpooled `PageSlots` GL churn (`+71–75 MB`), (c) `enter_library` retaining `self.doc` + GPU/geometry caches (`+79 MiB`). Fixing all three brings heavy Continuo under `220 MiB` and eliminates growth across cycles. |
| 8 | **Annotation SQLite save & atomic write latency** (ms) | Desktop Criterion: `100` annots **`17.2 ms`**, `1000` annots **`36.8 ms`** (tablet: `SIN MEDIR`) | `< 50 ms` | **`≤ 25 ms`** for 100 annots / **`≤ 50 ms`** for 1000 annots (off UI thread) | `≤ 10 ms` (incremental row upsert instead of full rewrite) | Already off the main draw path via `folder_sync` actor, but heavy folder export/scan shares the same single actor thread; isolating local SQLite saves prevents UI-visible save stalls. |
| 9 | **Folder sync incremental scan** (unchanged corpus) | `SIN MEDIR` on tablet (D3 privacy); desktop unit suite `< 5 ms` | None | **`≤ 150 ms`** for 500 unchanged PDFs (stat-only, zero re-hash), **never blocking startup or SQLite saves** | `≤ 50 ms` | Logcat showed 496 synced files scanned on startup; decoupling startup scan from first frame and skipping unchanged `(mtime, size)` keeps scans invisible. |
| 10 | **Binary size & CI test loop time** | `libpdf_android.so`: **`16 MB`**; `app-debug.apk`: **`25 MB`**; Desktop 7-check CI (warm): **`~35 s`** | None | `libpdf_android.so ≤ 18 MB`; `app-debug.apk ≤ 30 MB`; Full host CI (`fmt + clippy + test + pytest + gradle`) **`≤ 90 s`** | `libpdf_android.so ≤ 14 MB` (`strip` + `lto = "thin"`); CI `≤ 60 s` | Healthy today; adding `pdf_app` tests and host `pdf_android` clippy adds `< 5 s` while closing CI blind spots. |

---

## 3. Prioritized `$ponytail` Impact Ranking (80% of the Gap for Minimal Code)

| Rank | Change | Est. LOC | Expected Measured Gain on TCL 9469X | Fixes Audit Finding |
|---|---|---|---|---|
| **1** | **Cap / shrink MuPDF `fz_store` (`32–48 MiB`)** on render-worker init & page eviction | ~20 LOC | **`-160 to -190 MB` Native Heap / PSS** on `mixed_stress.pdf` (`461.8 → ~270 MB` immediately) | #11 |
| **2** | **Reuse 4 fixed GL texture IDs in `PageSlots`** (`glTexSubImage2D` / in-place `glTexImage2D`, remove hot-loop `info!()`) | ~45 LOC | **`-50 to -70 MB` `GL mtrack`** (`165 → ~95 MB` Graphics) AND **`-3 to -5 ms` `frame p95`** (`14.6 → ~9–10 ms`) | #12 |
| **3** | **Drop `self.doc`, `page_geometry_cache`, `scroll_geoms`, `annotations` in `enter_library()`** (implemented immediately in Phase 5!) | ~15 LOC | Eliminates **`+79 MiB` cross-cycle retention** (`+9.9 → ~0 MiB/cycle`) & fixes stale cross-doc geometry | #6 |
| **4** | **Skip no-op `recreate_surface()` when `has_surface()` & dims unchanged** + dedup startup `load_progress()` | ~15 LOC | **`-150 to -200 ms` cold start** (`649 → ~450–490 ms`, passing `< 500 ms` Hard Gate) | #13 |
| **5** | **Cache `scroll_layout` prefix sums (invalidate only on geometry/zoom/viewport change)** + emit `p50/p95/p99` telemetry | ~40 LOC | **`-1.5 to -3 ms` `frame p95`** on 500p docs (`large_document.pdf`: `12.9 → < 8.33 ms` 120 Hz Hard Gate) | #12 |

Total estimated production diff to clear **every** Hard Gate on TCL 9469X: **`~135 LOC`**, zero new dependencies.
