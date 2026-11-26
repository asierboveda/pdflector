# PDFLector Repository Review, Tablet UX/Performance Audit & TDD Targets Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `subagent-driven-development` (recommended) or `executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Apply `$ponytail` (full level) throughout: smallest complete fix, zero bloat, evidence before assertions.

**Goal:** Independently verify all 15 Kanvas cards on `origin/main`, run a real hardware UX and performance audit on the connected TCL NXTPaper 11 Plus (`9469X`), publish a ranked `$ponytail-audit` report and an evidence-backed performance targets study, produce a TDD roadmap to reach those targets, and immediately fix all confirmed Must-Fix bugs (including excluding `gemini_api_key` from Android cloud backup) via strict Red→Green TDD.

**Architecture:** All work executes inside an isolated git worktree (`.worktrees/audit-main`, branch `audit/main-review-2026-11-27` off `origin/main` `fb2cc5c`) so `feat/pdflector-v2` stays completely untouched. Static verification checks git history, tests, and Rust/Kotlin code against `kanvas.json`; hardware verification builds the real ARM64 release `.so` packaged into `com.pdflector.app` and profiles it on the USB-attached TCL 9469X over ADB (`pdf_bench`, `dumpsys meminfo`, `dumpsys gfxinfo`, `logcat`, `screencap`, and scripted touch flows) using `corpus/` plus one generated mixed-orientation stress PDF. Findings feed a single `$ponytail-audit` Markdown report, a hardware-grounded target study, a TDD roadmap, and immediate Red→Green commits for Must-Fix defects.

**Tech Stack:** Rust 2024 workspace (`pdf_core`, `pdf_android`, `pdf_app`, `pdf_bench`), MuPDF (`mupdf-sys`, static C), OpenGL ES 3.0 / EGL (`GameActivity`), Kotlin/Gradle AGP 8.7.3 (`android/product`, `android/ink_bench`), Android NDK r28 (`aarch64-linux-android`), Python 3 (`pillow`, `reportlab`, `unittest`), ADB (`TCL 9469X`, Android 16 / SDK 36, 8 GB RAM, 1440×2200 @ 120 Hz).

**Spec / Context:** `PLANNING_CONTEXT.md`, `kanvas.json` (at `origin/main` `fb2cc5c`), `AGENTS.md`, `.agents/skills/pdflector-rendimiento/SKILL.md`, and the human Decisions Log below.

## Decisions Log

| ID | Topic | Decision | Rationale | Source | Date |
|---|---|---|---|---|---|
| D1 | Branch & workspace isolation | Run in new worktree `.worktrees/audit-main` (`audit/main-review-2026-11-27` off `origin/main` `fb2cc5c`) | Leaves `feat/pdflector-v2` (23-card v2 board) and stale worktrees untouched | Interview | 2026-11-27 |
| D2 | Android cloud backup of Gemini key | Exclude `gemini_api_key` from cloud backup and device transfer via `backup_rules.xml` + `data_extraction_rules.xml`; keep `notebooks.json` backed up | Closes open security risk in `PLANNING_CONTEXT.md` §1/§6 without losing user study notes | Interview | 2026-11-27 |
| D3 | Tablet benchmark PDF workload | Use existing 4-file `corpus/` + one generated mixed-orientation heavy stress PDF (`mixed_stress.pdf`); never read or touch personal PDFs on the tablet | Reproducible, deterministic, covers PR-04 mixed-page sizes & heavy load with zero privacy risk | Interview | 2026-11-27 |
| D4 | Execution autonomy | Execute Phases 0–5 autonomously, fixing confirmed Must-Fix bugs (including D2) via TDD, pausing only at the end for human stylus/feel checks, PR-04 sign-off, and approval of Phase 4 roadmap cards | Maximizes progress while preserving human control over hardware feel, PR-04 acceptance, and roadmap scope | Interview | 2026-11-27 |

## Global Constraints

- **Worktree isolation:** Run ALL commands inside `/home/asierboveda/Projects/pdflector/.worktrees/audit-main`. Never modify files in `/home/asierboveda/Projects/pdflector` (`feat/pdflector-v2`).
- **Ponytail rule (`$ponytail` full):** Take the smallest complete fix. Never add abstractions, options, or dependencies not justified by a concrete bug or measured bottleneck. Every shortcut gets `shortcut: <limit>, <when to upgrade>`.
- **TDD Iron Law (`$test-driven-development`):** No production code without a failing test first. Every Must-Fix gets a test-only Red commit (verified FAIL for the right reason) followed by a minimal Green implementation commit (verified PASS). Never weaken, skip, `#[ignore]`, or delete tests.
- **Hard CI Gate (every code change must pass ALL 7 checks):**
  1. `cargo fmt --all -- --check`
  2. `cargo clippy --all-targets -- -D warnings`
  3. `cargo clippy -p pdf_android --lib --tests -- -D warnings`
  4. `cargo clippy -p pdf_android --lib -- -D warnings` *(catches test-only dead code before Android CI)*
  5. `cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings`
  6. `cargo test -p pdf_core && cargo test -p pdf_android --lib`
  7. `python3 -m unittest discover -s tools/tests && PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests && (cd android/product && ./gradlew :app:testDebugUnitTest --no-daemon && ./gradlew -p ../ink_bench :app:testDebugUnitTest --no-daemon)`
- **Android NDK r28 env (required for every `aarch64-linux-android` or Gradle build):**
  ```bash
  export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
  export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
  export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
  ```
- **Tablet measurement discipline:** Connected device is `TCL 9469X` (`A06B4A8E6774623`, Android 16 / SDK 36, 8 cores, 8 GB RAM, 1440×2200 @ 120 Hz / 60 Hz). Always keep screen ON (`adb shell input keyevent KEYCODE_WAKEUP && adb shell svc power stayon true`), always restore `stayon false` on exit, always measure with `--release` Rust `.so` (`assembleProductDebug` compiles Rust `--release`), and never infer metrics — record `SIN MEDIR` if unmeasured.
- **Secrets hygiene:** Never commit or log any API key. Test `Clave IA` only with a synthetic dummy key (e.g., `AIzaSyDUMMY0000000000000000000000000000`) and clear it after verification.
- **Kanvas discipline:** Never mark `PR-04` done or approve proposals on behalf of the human. Do not edit `kanvas.json` directly.

## Deliverables & File Map

| File (relative to `.worktrees/audit-main`) | Purpose | Produced in |
|---|---|---|
| `docs/card-review-2026-11-27.md` | Step-by-step code & TDD verification ledger for all 15 Kanvas cards (`TD-01`..`PR-04`) | Phase 1 (Task 6) |
| `docs/benchmark-results.md` | Append-only empirical measurements (desktop + TCL 9469X hardware, PSS/RSS, frame p95, `dlcache`, cold start, 15-cycle leak test) | Phase 2 (Task 9) & Phase 5 (Task 16) |
| `docs/audit-2026-11-27.md` | Ranked `$ponytail-audit` report (Must fix / Should fix / Nice to have, Lean savings, Not checked) | Phase 3 (Task 12) |
| `docs/performance-targets.md` | Hardware-grounded study of what every performance & quality number SHOULD be on TCL 9469X vs measured baseline | Phase 4 (Task 13) |
| `docs/superpowers/plans/2026-11-27-performance-quality-tdd-roadmap.md` | Concrete `$test-driven-development` + `$ponytail` plan to hit every target & resolve Should-Fix findings, plus Kanvas batch JSON | Phase 4 (Task 14) |
| `android/product/app/src/test/kotlin/com/pdflector/app/BackupRulesTest.kt` | JVM TDD test enforcing `gemini_api_key` exclusion from cloud backup & device transfer | Phase 5 (Task 15) |
| `android/product/app/src/main/res/xml/backup_rules.xml` | API 29–30 full-backup exclusion for `gemini_api_key` | Phase 5 (Task 15) |
| `android/product/app/src/main/res/xml/data_extraction_rules.xml` | API 31+ cloud-backup & device-transfer exclusion for `gemini_api_key` | Phase 5 (Task 15) |
| `android/product/app/src/main/AndroidManifest.xml` | Wires `fullBackupContent` and `dataExtractionRules` | Phase 5 (Task 15) |
| Additional Must-Fix test + impl files | Determined empirically in Phases 1–3; each follows strict Red→Green TDD commits | Phase 5 (Task 16) |

---

## Phase 0: Environment & Baseline Verification Gate

### Task 1: Bootstrap Worktree Cache, Corpus & ADB Readiness

**Files:**
- Verify: `.worktrees/audit-main` (git worktree on `audit/main-review-2026-11-27` at `fb2cc5c`)
- Link (gitignored): `.worktrees/audit-main/corpus` → `/home/asierboveda/Projects/pdflector/corpus`

**Interfaces:**
- Consumes: `origin/main` at `fb2cc5c`, host `/home/asierboveda/Projects/pdflector/corpus/`, NDK r28 at `~/Android/Sdk/ndk/android-ndk-r28`, USB tablet `A06B4A8E6774623`.
- Produces: Ready worktree with `corpus/` present, verified NDK toolchain, and confirmed tablet connectivity.

- [ ] **Step 1: Verify worktree clean state and symlink `corpus/`**

Run inside `/home/asierboveda/Projects/pdflector/.worktrees/audit-main`:
```bash
git status -sb
git rev-parse --short HEAD
ln -sfn /home/asierboveda/Projects/pdflector/corpus corpus
ls -la corpus/
```
Expected: `audit/main-review-2026-11-27...origin/main`, HEAD `fb2cc5c`, and all 4 PDFs (`dense_textbook.pdf`, `large_document.pdf`, `scanned_pages.pdf`, `scientific_paper.pdf`) visible.

- [ ] **Step 2: Verify NDK r28 toolchain, Rust target, and ADB device**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
which aarch64-linux-android26-clang
rustup target list --installed | grep aarch64-linux-android
adb devices -l
adb shell "getprop ro.product.model; getprop ro.build.version.release; getprop ro.build.version.sdk"
```
Expected: `aarch64-linux-android26-clang` found, `aarch64-linux-android` installed, device `A06B4A8E6774623` online (`9469X`, Android `16`, SDK `36`).

---

### Task 2: Run the Full Baseline CI Gate Locally

**Files:**
- Read-only check across entire workspace (`crates/*`, `tools/*`, `android/*`)

**Interfaces:**
- Consumes: Task 1 ready worktree.
- Produces: Exact baseline pass/fail counts and warnings for every suite before any edits.

- [ ] **Step 1: Run formatting and all 4 Rust clippy gates**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy -p pdf_android --lib --tests -- -D warnings
cargo clippy -p pdf_android --lib -- -D warnings
cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings
```
Expected: All 5 commands exit 0 with zero warnings. Record any warning or failure verbatim for `docs/card-review-2026-11-27.md`.

- [ ] **Step 2: Run Rust host test suites (`pdf_core` + `pdf_android`)**

```bash
cargo test -p pdf_core
cargo test -p pdf_android --lib
```
Expected: Every test in `pdf_core` (including corpus cache tests) and all 219+ tests in `pdf_android --lib` report `test result: ok`. Check for any `#[ignore]` tests via `grep -rn '#\[ignore' crates/`.

- [ ] **Step 3: Run Python tool tests and Kotlin JVM tests**

```bash
python3 -m unittest discover -s tools/tests
PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests
(cd android/product && ./gradlew :app:testDebugUnitTest --no-daemon)
(cd android/ink_bench && ./gradlew :app:testDebugUnitTest --no-daemon)
```
Expected: All Python tests and all 13+ Kotlin JVM tests in `android/product` plus `android/ink_bench` tests pass with exit code 0.

---

## Phase 1: Step-by-Step Code & TDD Review of All 15 Kanvas Cards (`$ponytail` Lens)

### Task 3: Review Wave 1 Foundation & Core Cards (`TD-01`, `TD-02`, `TD-03`, `CO-01`, `CO-02`, `CO-03`)

**Files:**
- Read: `kanvas.json` (full notes/checklists for `TD-01`, `TD-02`, `TD-03`, `CO-01`, `CO-02`, `CO-03`)
- Read: `crates/pdf_android/src/host/android_activity.rs`, `crates/pdf_android/src/jni/host.rs`, `crates/pdf_android/src/lib.rs` (theme tokens)
- Read: `crates/pdf_core/src/fsutil.rs`, `crates/pdf_android/src/persist.rs`, `crates/pdf_core/src/export.rs`
- Read: `crates/pdf_app/src/theme.rs`, `crates/pdf_android/src/draw/menus.rs`, `crates/pdf_android/src/draw/overlays.rs`, `crates/pdf_android/src/folder_sync.rs`

**Interfaces:**
- Consumes: Task 2 baseline gate results, git history (`6d80dc9..fb2cc5c`).
- Produces: Verification notes, TDD audit, and defect/bloat list for the 6 Foundation & Core cards.

- [ ] **Step 1: Verify git commit discipline & TDD pairs for Wave 1**

```bash
git log --stat --grep="TD-01\|TD-02\|TD-03\|CO-01\|CO-02\|CO-03\|atomic\|theme\|hitbox\|toast\|folder_sync" 6d80dc9..HEAD
```
Check each card:
- `TD-01`: Verify `cfg(target_os = "android")` gating in `crates/pdf_android/Cargo.toml` and `src/host/`; confirm no Android behavior changed and no real logic is hidden behind host-only hacks.
- `TD-02`: Run `grep -rn "allow(dead_code)" crates/pdf_android/src/` — verify zero remaining suppressions in `theme`, and audit any remaining `allow(dead_code)` elsewhere in `pdf_android`.
- `CO-01`: Read `crates/pdf_core/src/fsutil.rs` completely. Check: unique temp filename (collision-safe under concurrent writers?), `sync_all()` on file AND parent directory?, atomic `rename`, cleanup on error, permissions handling (used by `gemini_key.rs` for 0600 — check for race between create & `set_permissions`!). Grep for any remaining raw `fs::write` in `crates/pdf_android/src/persist.rs`, `crates/pdf_core/src/export.rs`, `crates/pdf_android/src/notebook.rs`, `crates/pdf_android/src/folder_sync.rs`.
- `CO-02`: Verify `crates/pdf_core/src/lib.rs` no longer exposes `theme` and `pdf_app` builds cleanly (`cargo check -p pdf_app`).
- `CO-03`: Read `crates/pdf_android/src/draw/menus.rs` and `render_toast` in `draw/overlays.rs`. Verify hit-test rects and draw rects share a single geometry struct, and check edge cases (empty string, single huge unbreakable word, multi-byte UTF-8 / Spanish accents / emoji truncation in toast).
- `TD-03`: Read `crates/pdf_android/src/folder_sync.rs` tests. Verify the 2 rewritten tests are causal (no wall-clock flakes) AND inspect the remaining `Instant::now()` test (`local_save_failure_wakes_for_retry...`) noted in `PLANNING_CONTEXT.md` §5.

- [ ] **Step 2: Stress-test Wave 1 tests for hidden flakiness**

```bash
for i in $(seq 1 5); do cargo test -p pdf_android --lib folder_sync -- --nocapture >/dev/null || { echo "FLAKE on run $i"; exit 1; }; done && echo "5/5 folder_sync runs green"
```
Expected: `5/5 folder_sync runs green`.

---

### Task 4: Review Wave 2 UI, Input & Architecture Cards (`UI-01`, `UI-02`, `IN-01`, `IN-02`, `IN-03`)

**Files:**
- Read: `crates/pdf_android/src/draw/chrome.rs`, `crates/pdf_android/src/draw/library.rs`, `crates/pdf_android/src/input/motion/{mod,viewer,library,discover,picker}.rs`
- Read: `android/product/app/src/main/kotlin/com/pdflector/app/ink/{InkStrokeGeometry.kt,InkGlRenderer.kt}`, `android/product/app/src/main/kotlin/com/pdflector/app/PdfLectorActivity.kt`
- Read: `crates/pdf_android/src/gpu/pipeline.rs` (`draw_polyline_gpu` / dry ink geometry)
- Read: `crates/pdf_core/src/prefetch.rs`, `crates/pdf_android/src/jni.rs`, `crates/pdf_android/src/jni/{window,launch,parse,media,ime}.rs`

**Interfaces:**
- Consumes: Git history for Wave 2 cards.
- Produces: Verification notes, wet-vs-dry mathematical parity check, and defect/bloat list for the 5 UI/Input/Arch cards.

- [ ] **Step 1: Audit `UI-01` and `UI-02`**

- Inspect `draw/chrome.rs` (reader theme button `"Aa"` glyph geometry & contrast) and `draw/library.rs` (`lib_add_btn_rect` shared by `⋯`, `☰`, and `＋ Añadir`). Verify hitboxes match rendered pill rects.
- Inspect `Cargo.toml` / `Cargo.lock` and `android/product/app/build.gradle.kts` dependency versions after `UI-02`; check for unused crates (`cargo tree -p pdf_android`, `cargo tree -p pdf_core`).

- [ ] **Step 2: Audit `IN-01` (Wet vs Dry Ink Parity & PR #55)**

- Read `InkStrokeGeometry.kt` and `InkGlRenderer.kt` side-by-side with Rust `draw_polyline_gpu` (grep `draw_polyline_gpu` in `crates/pdf_android/src/`).
- Verify: exact half-width formula, `0.5 px` floor, segment count for end caps / joins, coordinate space projection (`InkProjectionTest.kt`), zero-length / single-point stroke edge cases, and PR #55 (`40d24aa`) native repaint skipping while wet ink is active (verify no stuck dirty flag when pen cancels/exits reader).

- [ ] **Step 3: Audit `IN-02` and `IN-03`**

- Read `input/motion/mod.rs` and its 4 subfiles. Check if splitting left duplicated hit-test math, overly wide visibility, or dead branches.
- Read `crates/pdf_core/src/prefetch.rs`: verify `take_errors` cap (64), error propagation, and that `pdf_app` actually drains it so it cannot silently accumulate unboundedly.
- Read `crates/pdf_android/src/jni.rs` and `jni/{window,launch,parse,media,ime}.rs`: verify every JNI signature matches `PdfLectorActivity.kt` `external fun` declarations and check exception handling (`env.exception_check()` / `exception_clear()`) across JNI calls.

---

### Task 5: Review Wave 3 Product Reshape Cards (`PR-01`, `PR-02`, `PR-03`, `PR-04`)

**Files:**
- Read: `crates/pdf_android/src/reader/{scroll.rs,fling.rs,render_failures.rs,study.rs,sheet_chrome.rs,redraw.rs,tick.rs,mod.rs}`
- Read: `crates/pdf_android/src/gpu/{scroll.rs,page_slots.rs,pipeline.rs,ovl_budget.rs,textures.rs}`
- Read: `crates/pdf_android/src/gemini_key.rs`, `crates/pdf_android/src/notebook.rs`, `crates/pdf_android/src/draw/study.rs`, `crates/pdf_android/src/draw/sheet.rs`
- Read: `crates/pdf_core/src/engine/display_list_cache.rs`, `crates/pdf_core/src/engine/mupdf.rs`

**Interfaces:**
- Consumes: Wave 3 code and git commits (`aa09fec..fb2cc5c`).
- Produces: Deep verification of Continuo scroll, fling physics, mixed page sizes, render retries, Gemini key security, notebooks, and display-list LRU memory budget.

- [ ] **Step 1: Audit `PR-01` & `PR-04` (Continuo Scroll, Fling, Mixed Page Sizes, Live Undo, Retries)**

Check every edge case in code:
- **Scroll geometry (`reader/scroll.rs`, `gpu/scroll.rs`, `gpu/page_slots.rs`):** 1-page PDF, 500-page PDF, mixed portrait/landscape pages, anchor preservation when a page's real size arrives mid-scroll, zoom-in + horizontal pan clamping, page-number tracking when 2 pages are equally visible.
- **Fling (`reader/fling.rs`):** frame-rate independence (`dt` clamping on stalls/background resume — what if `dt = 5.0s`?), stop at top/bottom bounds, tap-to-brake suppressing toolbar toggle, cancellation on zoom/jump/mode-switch/page-turn/lifecycle pause.
- **Live annotation changes (`reader/anotaciones.rs`, `reader/redraw.rs`, `gpu/page_slots.rs`):** undo/redo/erase on a visible non-current page in Continuo mode invalidates the right slot texture immediately without flickering.
- **Render failure backoff (`reader/render_failures.rs`):** does backoff reset on reopen/zoom/manual retry? Is there a bounded cap on failed-page map size? Does placeholder render cleanly without infinite redraw loop spinning GPU/battery?

- [ ] **Step 2: Audit `PR-02` (In-App Gemini Key & Per-Document Notebook)**

- **Secret hygiene grep:**
  ```bash
  grep -rn "gemini_api_key\|GeminiKey\|google_key\|x-goog-api-key\|api_key" crates/ android/ tools/ .github/
  ```
  Verify:
  1. Zero occurrences of logging raw key (check `pdf_core/src/ai.rs` HTTP error logs too — does `reqwest`/HTTP error URL or debug print ever contain the key?! CRITICAL CHECK!).
  2. Check `gemini_key.rs` file creation: does `write_atomic` create temp file with default umask (0644) and `chmod 0600` *after* writing, or set 0600 *before* writing? Flag if temp file is briefly world-readable!
  3. Check `AndroidManifest.xml` for `allowBackup` / `dataExtractionRules` (confirmed Must-Fix D2).
  4. Check `notebook.rs`: atomic save, corruption recovery (corrupt JSON doesn't crash app), max note length / note count bounds, UTF-8 / newline handling, and confirm `google_key.txt` stale references in docs/skills.

- [ ] **Step 3: Audit `PR-03` (Display-List Cache & Memory Budgeting)**

- Read `crates/pdf_core/src/engine/display_list_cache.rs` and `crates/pdf_core/src/engine/mupdf.rs`.
- Check:
  1. Budget enforcement (16 entries / 24 MiB per doc): what if a single page's estimated size exceeds 24 MiB? Does it still render without thrashing or failing?
  2. In-use pinning: can a pinned entry leak (panic or early return leaving pin count > 0)?
  3. Size estimation formula vs real memory (note MuPDF's own decoded-image store — default 256 MB in MuPDF — check if `fz_context` store limit is set or uncapped in `mupdf.rs`!).
  4. Multi-document lifecycle: when switching documents in Library → Viewer → Library (15 cycles), are ALL display lists, textures, and MuPDF contexts released?

---

### Task 6: Write the 15-Card Verification Ledger

**Files:**
- Create: `docs/card-review-2026-11-27.md`

**Interfaces:**
- Consumes: Findings from Tasks 2, 3, 4, 5.
- Produces: Complete card-by-card status table (`TD-01`..`PR-04`) with commit SHAs, TDD verdict, concrete defects found, open tablet-check items, and stale-doc list.

- [ ] **Step 1: Write `docs/card-review-2026-11-27.md`**

Include:
1. Baseline CI gate summary (exact test counts & pass/fail per suite).
2. Table of all 15 cards (`ID | State on Board | Commits | TDD Verified | Static Code Verdict | Tablet Checklist Pending | Findings`).
3. Detailed notes only for cards with defects, edge-case risks, or bloat (apply `$ponytail`: concise, zero fluff).
4. List of stale documentation/skill references (e.g., `google_key.txt` in `.agents/skills/pdflector-rendimiento/SKILL.md` if outdated).

- [ ] **Step 2: Commit the card review ledger**

```bash
git add docs/card-review-2026-11-27.md
git commit -m "docs(review): verify all 15 kanvas cards on origin/main"
```

---

## Phase 2: Tablet Build, Benchmarks & Critical UX Evaluation on TCL 9469X

### Task 7: Generate Deterministic Mixed-Orientation Stress PDF

**Files:**
- Create (temporary/gitignored output): `corpus/mixed_stress.pdf`

**Interfaces:**
- Consumes: Python 3 + `pillow` + `reportlab` (verify installed or install in `/tmp/corpus-venv`).
- Produces: `corpus/mixed_stress.pdf` (80 pages: alternating portrait A4, landscape A4, dense 2-column text, high-res 1600×1200 raster image pages, complex vector plot pages, and bookmarks — deterministic via `invariant=1`).

- [ ] **Step 1: Generate `corpus/mixed_stress.pdf` without touching production code**

Check if `reportlab` and `PIL` are available in system Python (else create `/tmp/corpus-venv`):
```bash
PY=python3
python3 -c "import PIL, reportlab" 2>/dev/null || {
  python3 -m venv /tmp/corpus-venv
  /tmp/corpus-venv/bin/pip install --quiet pillow reportlab
  PY=/tmp/corpus-venv/bin/python
}
$PY - <<'EOF'
import random
from pathlib import Path
from PIL import Image, ImageDraw
from reportlab.lib.pagesizes import A4, landscape
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas

random.seed(42)
out = Path("corpus/mixed_stress.pdf")
c = canvas.Canvas(str(out), pagesize=A4, invariant=1)
p_w, p_h = A4
l_w, l_h = landscape(A4)

for idx in range(80):
    kind = idx % 4
    if kind == 0:  # Portrait dense text
        c.setPageSize((p_w, p_h))
        c.setFont("Helvetica-Bold", 14)
        c.drawString(40, p_h - 50, f"Page {idx + 1} — Portrait Dense Text (PR-04 Stress)")
        c.setFont("Helvetica", 9)
        for row in range(62):
            c.drawString(40, p_h - 75 - row * 12, f"[{idx+1:02d}:{row:02d}] The quick brown fox jumps over the lazy dog. α + β = γ; ∫ f(x) dx = F(b) - F(a).")
    elif kind == 1:  # Landscape slide / wide figure
        c.setPageSize((l_w, l_h))
        c.setFont("Helvetica-Bold", 18)
        c.drawString(40, l_h - 50, f"Page {idx + 1} — Landscape Wide Slide (842x595 pt)")
        for i in range(180):
            x0 = 40 + (i * 4.2) % (l_w - 80)
            y0 = 60 + ((i * 37) % int(l_h - 150))
            c.line(x0, y0, x0 + 35, y0 + 25)
            c.circle(x0, y0, 4, stroke=1, fill=0)
    elif kind == 2:  # Portrait heavy raster image (tests MuPDF image store + dlcache)
        c.setPageSize((p_w, p_h))
        img = Image.new("RGB", (1400, 1980), (248, 246, 240))
        d = ImageDraw.Draw(img)
        for y in range(40, 1940, 28):
            d.rectangle([50, y, 1350, y + 10], fill=((idx * 23 + y) % 180, 60, 90))
        c.drawImage(ImageReader(img), 20, 20, width=p_w - 40, height=p_h - 40)
    else:  # Landscape dense table + math text
        c.setPageSize((l_w, l_h))
        c.setFont("Helvetica-Bold", 13)
        c.drawString(40, l_h - 45, f"Page {idx + 1} — Landscape Matrix & Equations")
        c.setFont("Helvetica", 8)
        for r in range(36):
            row_txt = "  |  ".join(f"M[{r},{col}]={(r*7+col+idx)%97:02d}" for col in range(10))
            c.drawString(40, l_h - 70 - r * 14, row_txt)
    c.showPage()

c.save()
print(f"Generated {out} ({out.stat().st_size} bytes, 80 pages)")
EOF
```
Expected: `corpus/mixed_stress.pdf` created cleanly (~1–4 MB, 80 mixed portrait/landscape pages).

---

### Task 8: Build & Deploy `com.pdflector.app` (Release Rust `.so`) to TCL 9469X

**Files:**
- Read: `android/product/app/build.gradle.kts`
- Build output: `android/product/app/build/outputs/apk/debug/app-debug.apk` (contains `--release` `libpdf_android.so`)

**Interfaces:**
- Consumes: Task 1 NDK env, Task 7 `corpus/mixed_stress.pdf`.
- Produces: Freshly built `origin/main` (`fb2cc5c`) APK installed on `TCL 9469X`, test PDFs copied into app-private `files/` (zero contact with user's personal files).

- [ ] **Step 1: Build APK with release Rust `libpdf_android.so` (without `google_key.txt` to verify PR-02)**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

test ! -f crates/pdf_android/google_key.txt
(cd android/product && ./gradlew assembleProductDebug --no-daemon)
ls -lh android/product/app/build/outputs/apk/debug/app-debug.apk target/aarch64-linux-android/release/libpdf_android.so
```
Expected: Build succeeds WITHOUT `google_key.txt` (proving PR-02 dropped the compile-time key); APK and `.so` sizes recorded.

- [ ] **Step 2: Install APK on TCL 9469X and inject isolated test PDFs via `run-as`**

```bash
adb install -r android/product/app/build/outputs/apk/debug/app-debug.apk
adb shell mkdir -p /data/local/tmp/pdflector_test
for f in corpus/dense_textbook.pdf corpus/large_document.pdf corpus/scanned_pages.pdf corpus/scientific_paper.pdf corpus/mixed_stress.pdf; do
  adb push "$f" "/data/local/tmp/pdflector_test/$(basename "$f")"
  adb shell "run-as com.pdflector.app mkdir -p files/bench_corpus && run-as com.pdflector.app cp /data/local/tmp/pdflector_test/$(basename "$f") files/bench_corpus/$(basename "$f")"
done
adb shell run-as com.pdflector.app ls -lh files/bench_corpus/
```
Expected: All 5 benchmark PDFs present inside app-private `files/bench_corpus/`. Personal files on `/sdcard` untouched.

- [ ] **Step 3: Smoke launch + visual sanity screencap**

```bash
adb shell input keyevent KEYCODE_WAKEUP
adb shell svc power stayon true
adb logcat -c
adb shell am force-stop com.pdflector.app
adb shell am start -W -n com.pdflector.app/.PdfLectorActivity
sleep 2
adb exec-out screencap -p > /tmp/tcl_01_launch.png
adb logcat -d -s pdf_android:V AndroidRuntime:E | tail -n 40
```
Expected: Clean launch (no crash in `AndroidRuntime:E`), `TotalTime` logged, `/tmp/tcl_01_launch.png` visually inspected via `view_file`.

---

### Task 9: Quantitative Performance Profiling (Desktop + TCL 9469X)

**Files:**
- Create: `docs/benchmark-results.md` (append-only evidence log per `pdflector-rendimiento`)

**Interfaces:**
- Consumes: Task 8 installed app, `tools/adb-bench.sh`, `crates/pdf_bench`.
- Produces: Complete desktop + TCL 9469X numerical dataset logged in `docs/benchmark-results.md`.

- [ ] **Step 1: Desktop baseline sweep + Criterion quick run**

```bash
cargo run --release -p pdf_bench | tee /tmp/bench-desktop-sweep.txt
cargo bench -p pdf_bench -- --quick | tee /tmp/bench-desktop-criterion.txt
```
Record median open/render1x/render2x/zoom/PEAK_RSS_KB and all 8 Criterion suite medians (`open_render`, `cache_scroll`, `render_perf`, `zoom`, `annotations`, `blit`, `highlight`, `prefetch`).

- [ ] **Step 2: Tablet `pdf_bench` sweep (`RUNS=5`, screen ON)**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
RUNS=5 CORPUS_DIR=corpus ./tools/adb-bench.sh | tee /tmp/bench-tcl-harness.txt
```
Also run a 5-iteration direct check of `mixed_stress.pdf` on the tablet (or measure via app logcat) and record `PEAK_RSS_KB` and per-PDF `open`, `render1x`, `render2x`, `zoom`.

- [ ] **Step 3: Tablet App Profiling (`com.pdflector.app` real process)**

Keep screen ON (`adb shell svc power stayon true`) and confirm active display refresh mode (`dumpsys display | grep mActiveModeId`):
1. **Cold start & cold memory:**
   - `am force-stop com.pdflector.app` → `am start -W -n com.pdflector.app/.PdfLectorActivity` (repeat 3× for median `TotalTime` / `WaitTime`).
   - Capture `dumpsys meminfo com.pdflector.app` (TOTAL PSS, TOTAL RSS, Native Heap, EGL mtrack, GL mtrack).
2. **Open each test PDF via `ACTION_VIEW` (or app-private test copy) & measure idle PSS/RSS:**
   - Open `large_document.pdf` (500 pages) and `mixed_stress.pdf` (80 mixed pages).
   - Capture `dumpsys meminfo com.pdflector.app` after settle (5 s).
3. **Continuo scroll & fling 120 Hz stress (60 seconds of rapid swipes across `large_document.pdf` and `mixed_stress.pdf`):**
   - Reset gfxinfo: `adb shell dumpsys gfxinfo com.pdflector.app reset`
   - Clear logcat: `adb logcat -c`
   - Script 30 fast vertical swipes (`adb shell input swipe 720 1800 720 400 80` with 600 ms pauses so fling runs and renders ~60–100 pages).
   - Collect:
     - `adb logcat -d -s pdf_android:V | grep -E "frame p95|dlcache|render-worker"`
     - `adb shell dumpsys gfxinfo com.pdflector.app` (Total frames, Janky frames %, 50th/90th/95th/99th percentile)
     - `adb shell dumpsys meminfo com.pdflector.app` (Peak/post-scroll PSS & RSS — tests `PR-03` display-list cap AND exposes uncapped MuPDF image store impact on `mixed_stress.pdf`!).
4. **15-cycle Library ↔ Viewer memory leak test:**
   - Repeat 15×: open PDF → flip/scroll 5 pages → back to Library (`KEYCODE_BACK`).
   - Record TOTAL PSS & Native Heap & GL/EGL mtrack after cycles 1, 5, 10, 15. Compute slope (MB/cycle).
5. **Restore energy state:** `adb shell svc power stayon false`.

- [ ] **Step 4: Write all measured numbers into `docs/benchmark-results.md` and commit**

Format strictly per `pdflector-rendimiento` §7 (**Fecha + Hardware + Flujo medido + Métrica**). Never omit a failed or surprising metric; mark any unmeasured flow `SIN MEDIR`.
```bash
git add docs/benchmark-results.md
git commit -m "docs(bench): record desktop and TCL 9469X baseline measurements"
```

---

### Task 10: Critical UX Walkthrough & Visual Audit on TCL 9469X

**Files:**
- Scratch screenshots: `/tmp/tcl_ux_*.png` (inspected with `view_file`)
- Output notes fed into `docs/card-review-2026-11-27.md` (tablet column) and `docs/audit-2026-11-27.md`

**Interfaces:**
- Consumes: Task 8 installed app, Task 7 `corpus/mixed_stress.pdf`.
- Produces: Concrete visual & interaction UX defects (screenshots verified pixel-by-pixel) for every screen and card checklist.

- [ ] **Step 1: Library UX & Visual Audit (`UI-01`, `CO-03`, Astryx)**

- Capture Library screen in default theme (`/tmp/tcl_ux_01_library.png`). Inspect with `view_file`: verify `⋯`, `☰`, and `＋ Añadir` vertical alignment and heights (`UI-01`), card spacing, typography, empty/populated states.
- Tap `⋯` and `☰` menus; capture screenshots (`/tmp/tcl_ux_02_view_menu.png`, `/tmp/tcl_ux_03_settings_menu.png`). Verify menu rows align with hit targets (`CO-03`).

- [ ] **Step 2: Reader Chrome, Themes & Reading Mode Toggle (`UI-01`, `PR-01`)**

- Open `scientific_paper.pdf` and `mixed_stress.pdf`. Capture reader chrome (`/tmp/tcl_ux_04_reader_chrome.png`).
- Visually inspect the `"Aa"` theme button (`UI-01`) — confirm crisp `"Aa"` glyph, not broken box/fallback. Cycle Light → Sepia → Dark and verify contrast + page rendering.
- Open Reader Settings sheet (`/tmp/tcl_ux_05_reader_sheet.png`). Verify `MODO DE LECTURA` (`Página a página` / `Continuo`) and `ESTUDIO` (`Clave IA`, `Cuaderno`) layout, touch target sizes (≥48 dp), and zero text overlap or clipping.

- [ ] **Step 3: Continuo Scroll, Fling, Mixed Sizes & Tap-to-Brake (`PR-01`, `PR-04`)**

- Switch to `Continuo` mode on `mixed_stress.pdf`.
- Swipe to bring a landscape page (`Page 2`) and portrait page (`Page 1` / `Page 3`) simultaneously on screen; capture `/tmp/tcl_ux_06_continuo_mixed.png`. Verify landscape page uses full width with its own height (no cropped edges or giant white gaps).
- Trigger fast fling (`input swipe 720 1800 720 350 70`), wait 150 ms, tap center (`input tap 720 1100`): capture `/tmp/tcl_ux_07_fling_brake.png`. Verify fling stopped AND toolbars did NOT toggle on (`PR-04`).
- Force-stop and relaunch the same PDF: verify `Continuo` mode persisted per document (`PR-01`).

- [ ] **Step 4: Study Workflow UX — Gemini Key & Notebook (`PR-02`)**

- Open `ESTUDIO → Clave IA` (`/tmp/tcl_ux_08_gemini_key.png`). Verify missing-key message when attempting AI action without key.
- Test invalid short key (confirm clear Spanish error message). Save dummy valid-length key (`AIzaSyDUMMY0000000000000000000000000000`), verify masked display (`••••0000`), check `adb logcat -d | grep -i AIzaSyDUMMY` returns **0 matches** (secret leak test!), verify file permissions via `adb shell run-as com.pdflector.app ls -la files/gemini_api_key` (`-rw-------`). Then clear the dummy key!
- Open `ESTUDIO → Cuaderno` (`/tmp/tcl_ux_09_notebook.png`), add 2 notes, restart app, verify notes persisted in `files/notebooks.json`.
- Test portrait + landscape orientation rotation (`adb shell settings put system accelerometer_rotation 0` + `user_rotation 1` → `0`) while in Reader & Continuo mode; capture `/tmp/tcl_ux_10_landscape.png` and verify no blank frame, broken layout, or lost scroll position.

- [ ] **Step 5: Cleanup tablet temp files & restore screen settings**

```bash
adb shell settings put system user_rotation 0
adb shell settings put system accelerometer_rotation 1
adb shell svc power stayon false
adb shell rm -rf /data/local/tmp/pdflector /data/local/tmp/pdflector_test
```

---

### Task 11: Draft the Human-Only Physical Tablet Checklist

**Files:**
- Append to: `docs/card-review-2026-11-27.md`

**Interfaces:**
- Consumes: Task 10 automated UX results.
- Produces: Exact 5-item checklist of things ADB cannot physically simulate (stylus feel, wet→dry pen-lift seam for `IN-01`, palm rejection, subjective 120 Hz scroll smoothness, and human `PR-04` Done sign-off).

- [ ] **Step 1: Append the concise human-only checklist to `docs/card-review-2026-11-27.md` and commit**

```bash
git add docs/card-review-2026-11-27.md
git commit -m "docs(review): add TCL 9469X UX results and human stylus checklist"
```

---

## Phase 3: Full Repository `$ponytail-audit` Report

### Task 12: Conduct Full Repo Audit and Write `docs/audit-2026-11-27.md`

**Files:**
- Read (full end-to-end trace):
  - Entry points: `crates/pdf_android/src/lib.rs`, `android/product/app/src/main/kotlin/com/pdflector/app/PdfLectorActivity.kt`, `crates/pdf_app/src/main.rs`
  - Hot render/scroll/GPU path: `crates/pdf_core/src/engine/{mupdf.rs,display_list_cache.rs}`, `crates/pdf_android/src/gpu/{pipeline.rs,scroll.rs,page_slots.rs,textures.rs}`, `crates/pdf_android/src/reader/{redraw.rs,tick.rs,scroll.rs,fling.rs}`
  - Data writes & sync: `crates/pdf_core/src/{fsutil.rs,store.rs,folder_sync.rs,export.rs}`, `crates/pdf_android/src/{persist.rs,notebook.rs,folder_sync.rs,annotations.rs}`
  - Security / network / JNI: `crates/pdf_android/src/gemini_key.rs`, `crates/pdf_core/src/{ai.rs,arxiv.rs}`, `crates/pdf_android/src/jni/{window,launch,parse,media,ime,ink}.rs`, `AndroidManifest.xml`
  - Build/CI/deps: `Cargo.toml`, `crates/*/Cargo.toml`, `android/product/app/build.gradle.kts`, `.github/workflows/ci.yml`
- Create: `docs/audit-2026-11-27.md`

**Interfaces:**
- Consumes: Phase 1 card review (`docs/card-review-2026-11-27.md`), Phase 2 hardware numbers (`docs/benchmark-results.md`), UX screenshots, and full source grep.
- Produces: `docs/audit-2026-11-27.md` strictly conforming to `$ponytail-audit` format.

- [ ] **Step 1: Run targeted static scans for bloat, duplication, unsafe, panics, and secrets**

```bash
grep -rn "unwrap()\|expect(" crates/pdf_android/src/ crates/pdf_core/src/ | grep -v "test" | wc -l
grep -rn "allow(" crates/pdf_android/src/ crates/pdf_core/src/
grep -rn "unsafe " crates/pdf_android/src/ crates/pdf_core/src/
grep -rn "shortcut:\|ponytail:\|TODO\|FIXME\|HACK" crates/ android/ tools/
wc -l crates/pdf_android/src/**/*.rs crates/pdf_android/src/*.rs crates/pdf_core/src/*.rs crates/pdf_core/src/**/*.rs | sort -n | tail -n 25
```
Verify every candidate dead symbol with whole-tree `grep` (including tests, Kotlin JNI, Cargo.toml, XML) before reporting.

- [ ] **Step 2: Write `docs/audit-2026-11-27.md` in exact `$ponytail-audit` format**

Mandatory format rules (from `$ponytail-audit`):
1. Start with `What this repo does:` (2–3 plain-English sentences + assumed load: single student/researcher on an 8 GB Android 16 120 Hz stylus tablet reading 1–500 page PDFs with stylus ink, highlights, offline folder sync, and optional Gemini study queries).
2. Three groups, most important first (skip empty groups):
   - **Must fix:** bugs, security holes (including D2 `ALLOW_BACKUP` exposing `gemini_api_key`), data loss, breaks at assumed load.
   - **Should fix:** risky code without a test, measured real slowness/memory spikes (from Phase 2), duplication, mixed-job functions, dead/bloat code.
   - **Nice to have:** small speedups, shorter forms.
3. Number findings sequentially across all groups (`1.` .. `<=20.`), note count of smaller omitted items if any.
4. Every finding has exact `path:Lstart-Lend` and ALL 4 short plain-English parts:
   - **What this is:**
   - **Problem:** (concrete input/scenario → wrong result or measured number)
   - **Fix:** (smallest fix that works, preferring deletion)
   - **If we skip it:**
5. End with:
   - `Verdict:` (one line)
   - `Lean: -<N> lines, -<M> dependencies possible.`
   - `Not checked:` (exact files/flows not read or requiring physical stylus)

- [ ] **Step 3: Commit the audit report**

```bash
git add docs/audit-2026-11-27.md
git commit -m "docs(audit): publish full ponytail-audit report with TCL 9469X evidence"
```

---

## Phase 4: Performance Target Study ("What Should the Numbers Be?") & TDD Roadmap

### Task 13: Hardware-Grounded Study of Target Numbers (`docs/performance-targets.md`)

**Files:**
- Create: `docs/performance-targets.md`

**Interfaces:**
- Consumes: `docs/benchmark-results.md` (measured baseline), `docs/audit-2026-11-27.md`, and TCL 9469X hardware specs (`1440×2200`, `120 Hz = 8.33 ms`, `60 Hz = 16.67 ms`, `8 cores arm64-v8a`, `8 GB RAM` / `~3.9 GB` available, Android 16 / SDK 36).
- Produces: Justified quantitative target numbers (Good / Target / Stretch), gap analysis, and bottleneck attribution for every component.

- [ ] **Step 1: Derive and document exact target numbers from hardware physics + empirical measurements**

Structure `docs/performance-targets.md` with:
1. **Hardware Envelope of TCL NXTPaper 11 Plus (9469X):**
   - Display: `1440×2200` (3.17 Mpx), `120 Hz` (`8.33 ms` vsync budget, `presDeadline 8.33 ms`), `60 Hz` idle (`16.67 ms`).
   - Memory math: 1 full-screen RGBA8888 texture = `1440 × 2200 × 4 = 12.67 MB`; 3-page Continuo window at 1× = `~38 MB` GPU/CPU; 2× zoomed viewport = `~50 MB` if un-tiled; MuPDF default decoded-image store = `256 MB` uncapped; `dlcache` budget = `24 MB` / 16 pages.
   - Human perception thresholds (Nielsen / Android Vitals / stylus research): tap response `< 50 ms`, page flip `< 100 ms`, scroll/fling frame p95 `< 8.33 ms` (zero visible stutter at 120 Hz) and p99 `< 16.67 ms`, wet-ink motion-to-photon `< 16 ms` (target `< 8.33 ms` front-buffered), cold launch to first interactive frame `< 500 ms`.
2. **Master Target Table (every component):**
   - Columns: `Component & Metric | Measured Baseline (TCL 9469X) | Old Spec | Recommended Target (Hard Gate) | Stretch Goal | Bottleneck & Why This Number`
   - Rows covering:
     1. Cold launch `TotalTime` (ms)
     2. PDF open time: small (`scientific_paper`), dense (`dense_textbook`), 500p (`large_document`), mixed (`mixed_stress`) (ms)
     3. MuPDF `render1x` & `render2x` per page category (text / vector / scanned / mixed) (ms)
     4. Zoom fast upscale (`scale_bitmap`) vs crisp re-render (`2x`, `4x`) (ms)
     5. Frame time `p50 / p95 / p99` & `% janky frames` during Continuo drag, Continuo fling, pinch-zoom, and page turn (ms)
     6. `dlcache` hit ratio (%), eviction rate, and estimated-vs-actual byte calibration factor
     7. Memory PSS & RSS: cold start, idle viewer, heavy Continuo scroll (`mixed_stress.pdf`), 15-cycle Library↔Viewer peak + leak slope (MB, MB/cycle)
     8. Annotation local SQLite save latency & atomic write time (ms)
     9. Folder sync incremental scan (unchanged corpus) (ms)
     10. APK size / `libpdf_android.so` size & CI test loop time
3. **Prioritized Impact Ranking:** Which 3–5 changes buy 80% of the gap with the least code (`$ponytail` ROI table: estimated LOC vs measured ms/MB saved).

- [ ] **Step 2: Commit the target study**

```bash
git add docs/performance-targets.md
git commit -m "docs(perf): publish hardware-grounded performance target study for TCL 9469X"
```

---

### Task 14: Write the Phase 4 TDD Implementation Roadmap (`$test-driven-development`)

**Files:**
- Create: `docs/superpowers/plans/2026-11-27-performance-quality-tdd-roadmap.md`

**Interfaces:**
- Consumes: `docs/audit-2026-11-27.md` (Should-Fix & Nice-to-Have findings) and `docs/performance-targets.md` (every metric failing its Hard Gate).
- Produces: Standalone, zero-placeholder `$writing-plans` + `$test-driven-development` roadmap with concrete Red→Green tasks, exact file paths, concrete test code / benchmark assertions, and a ready-to-run `kanvas.py batch` JSON snippet (not yet applied to `kanvas.json` so the human stays in control of board approvals).

- [ ] **Step 1: Author `docs/superpowers/plans/2026-11-27-performance-quality-tdd-roadmap.md`**

Every task in the roadmap MUST include:
- Exact files to create/modify and line ranges.
- Concrete failing unit/integration test (exact Rust or Kotlin code) that proves the gap/bug exists before touching production code.
- Minimal `$ponytail` implementation sketch (no over-engineering, no speculative frameworks).
- Verification command + target metric gate on desktop and TCL 9469X.
- Dedicated `kanvas.py batch` JSON appendix grouping proposed cards cleanly (e.g., `Perf & Memory`, `Audit Cleanup`, `UX Polish`) with dependencies so the human can approve in one command.

- [ ] **Step 2: Self-review roadmap for zero placeholders and commit**

Scan the file for `TBD`, `TODO`, vague steps, or missing test assertions. Fix inline, then commit:
```bash
git add docs/superpowers/plans/2026-11-27-performance-quality-tdd-roadmap.md
git commit -m "docs(plan): add TDD roadmap to meet performance targets and audit findings"
```

---

## Phase 5: Immediate TDD Fixes for Confirmed Must-Fix Bugs

### Task 15: Must-Fix Security Fix (D2) — Exclude `gemini_api_key` from Android Cloud Backup

**Files:**
- Test (Create): `android/product/app/src/test/kotlin/com/pdflector/app/BackupRulesTest.kt`
- Create: `android/product/app/src/main/res/xml/backup_rules.xml`
- Create: `android/product/app/src/main/res/xml/data_extraction_rules.xml`
- Modify: `android/product/app/src/main/AndroidManifest.xml:6-8`

**Interfaces:**
- Consumes: `crates/pdf_android/src/gemini_key.rs` filename constant (`gemini_api_key` in `files/`), `notebooks.json` (must remain eligible for backup).
- Produces: Verified Android 10–16 backup exclusion rules preventing Google Cloud Backup and device-to-device transfer from copying `files/gemini_api_key`.

- [ ] **Step 1 (RED): Write the failing JVM test `BackupRulesTest.kt`**

Create `android/product/app/src/test/kotlin/com/pdflector/app/BackupRulesTest.kt`:
```kotlin
package com.pdflector.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

class BackupRulesTest {
    private val mainDir = File("src/main")

    private fun parseXml(path: String): Element {
        val file = File(mainDir, path)
        assertTrue("Expected XML file to exist: ${file.path}", file.isFile)
        val factory = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = true }
        return factory.newDocumentBuilder().parse(file).documentElement
    }

    @Test
    fun manifestWiresBackupAndDataExtractionRulesAndKeepsBackupEnabled() {
        val root = parseXml("AndroidManifest.xml")
        val app = root.getElementsByTagName("application").item(0) as Element
        val ns = "http://schemas.android.com/apk/res/android"
        assertFalse(
            "Backup should stay enabled so notebooks remain backed up",
            app.getAttributeNS(ns, "allowBackup") == "false",
        )
        assertTrue(
            "Manifest must declare android:fullBackupContent=\"@xml/backup_rules\"",
            app.getAttributeNS(ns, "fullBackupContent") == "@xml/backup_rules",
        )
        assertTrue(
            "Manifest must declare android:dataExtractionRules=\"@xml/data_extraction_rules\"",
            app.getAttributeNS(ns, "dataExtractionRules") == "@xml/data_extraction_rules",
        )
    }

    @Test
    fun backupRulesExcludeGeminiKeyOnly() {
        val root = parseXml("res/xml/backup_rules.xml")
        val excludes = root.getElementsByTagName("exclude")
        val excludedPaths = (0 until excludes.length).map { i ->
            val el = excludes.item(i) as Element
            el.getAttribute("domain") to el.getAttribute("path")
        }
        assertTrue(
            "backup_rules.xml must exclude file/gemini_api_key",
            ("file" to "gemini_api_key") in excludedPaths,
        )
        assertFalse(
            "backup_rules.xml must not exclude notebooks.json",
            excludedPaths.any { it.second.contains("notebooks") },
        )
    }

    @Test
    fun dataExtractionRulesExcludeGeminiKeyFromCloudAndDeviceTransfer() {
        val root = parseXml("res/xml/data_extraction_rules.xml")
        for (section in listOf("cloud-backup", "device-transfer")) {
            val nodes = root.getElementsByTagName(section)
            assertTrue("Missing <$section> in data_extraction_rules.xml", nodes.length == 1)
            val excludes = (nodes.item(0) as Element).getElementsByTagName("exclude")
            val excludedPaths = (0 until excludes.length).map { i ->
                val el = excludes.item(i) as Element
                el.getAttribute("domain") to el.getAttribute("path")
            }
            assertTrue(
                "<$section> must exclude file/gemini_api_key",
                ("file" to "gemini_api_key") in excludedPaths,
            )
            assertFalse(
                "<$section> must not exclude notebooks.json",
                excludedPaths.any { it.second.contains("notebooks") },
            )
        }
    }
}
```

- [ ] **Step 2 (Verify RED): Run Gradle JVM unit test and confirm it FAILS for the expected reason**

```bash
(cd android/product && ./gradlew :app:testDebugUnitTest --tests com.pdflector.app.BackupRulesTest --no-daemon)
```
Expected: FAIL — `Manifest must declare android:fullBackupContent="@xml/backup_rules"` and `Expected XML file to exist: src/main/res/xml/backup_rules.xml`.

- [ ] **Step 3: Commit the failing test-only commit**

```bash
git add android/product/app/src/test/kotlin/com/pdflector/app/BackupRulesTest.kt
git commit -m "test(android): require backup rules to exclude gemini_api_key"
```

- [ ] **Step 4 (GREEN): Add `backup_rules.xml`, `data_extraction_rules.xml`, and wire `AndroidManifest.xml`**

Create `android/product/app/src/main/res/xml/backup_rules.xml`:
```xml
<?xml version="1.0" encoding="utf-8"?>
<full-backup-content>
    <exclude domain="file" path="gemini_api_key" />
</full-backup-content>
```

Create `android/product/app/src/main/res/xml/data_extraction_rules.xml`:
```xml
<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
    <cloud-backup>
        <exclude domain="file" path="gemini_api_key" />
    </cloud-backup>
    <device-transfer>
        <exclude domain="file" path="gemini_api_key" />
    </device-transfer>
</data-extraction-rules>
```

Update `android/product/app/src/main/AndroidManifest.xml` `<application>` tag to include:
```xml
    <application
        android:label="PDFLector"
        android:allowBackup="true"
        android:fullBackupContent="@xml/backup_rules"
        android:dataExtractionRules="@xml/data_extraction_rules"
        android:supportsRtl="true">
```

- [ ] **Step 5 (Verify GREEN): Run Gradle JVM unit tests and commit implementation**

```bash
(cd android/product && ./gradlew :app:testDebugUnitTest --no-daemon)
```
Expected: `BackupRulesTest` (all 3 tests) AND all existing JVM tests PASS.

```bash
git add android/product/app/src/main/AndroidManifest.xml \
        android/product/app/src/main/res/xml/backup_rules.xml \
        android/product/app/src/main/res/xml/data_extraction_rules.xml
git commit -m "fix(android): exclude gemini_api_key from cloud backup and device transfer"
```

---

### Task 16: TDD Fix Loop for Additional Must-Fix Bugs & Final Verification Gate

**Files:**
- Driven strictly by the **Must fix** section of `docs/audit-2026-11-27.md` (from Phases 1–3).
- Update: `docs/benchmark-results.md` (post-fix verification delta if any Must-Fix affected runtime/memory).

**Interfaces:**
- Consumes: `docs/audit-2026-11-27.md` Must-Fix items, Task 15 security fix.
- Produces: Red→Green commit pairs for every confirmed Must-Fix bug, full 7-check CI gate pass, freshly built & verified APK on TCL 9469X, and final executive handback.

- [ ] **Step 1: For EVERY remaining Must-Fix finding in `docs/audit-2026-11-27.md`, execute one strict TDD slice**

For each finding `N`:
1. Write one minimal failing test reproducing the exact concrete case from finding `N`.
2. Run that test alone and verify it FAILS for the expected reason (not a compile error).
3. Commit the test-only commit: `test(<scope>): reproduce audit finding #<N> (...)`.
4. Implement the smallest `$ponytail` fix in production code (grep all callers first; fix once in shared code).
5. Re-run the test and verify it PASSES.
6. Commit the implementation commit: `fix(<scope>): resolve audit finding #<N> (...)`.

- [ ] **Step 2: Run the FULL 7-check Hard CI Gate**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo clippy -p pdf_android --lib --tests -- -D warnings
cargo clippy -p pdf_android --lib -- -D warnings
cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings
cargo test -p pdf_core
cargo test -p pdf_android --lib
python3 -m unittest discover -s tools/tests
PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests
(cd android/product && ./gradlew :app:testDebugUnitTest --no-daemon)
(cd android/ink_bench && ./gradlew :app:testDebugUnitTest --no-daemon)
```
Expected: ALL commands exit 0 with `ok` / `BUILD SUCCESSFUL` and zero warnings.

- [ ] **Step 3: Rebuild APK, reinstall on TCL 9469X, and re-verify fixes on hardware**

```bash
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/android-ndk-r28"
export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

(cd android/product && ./gradlew assembleProductDebug --no-daemon)
adb install -r android/product/app/build/outputs/apk/debug/app-debug.apk
adb shell dumpsys package com.pdflector.app | grep -E "ALLOW_BACKUP|versionName|lastUpdateTime"
adb shell am force-stop com.pdflector.app
adb shell am start -W -n com.pdflector.app/.PdfLectorActivity
sleep 2
adb logcat -d -s pdf_android:V AndroidRuntime:E | tail -n 30
```
Expected: Clean launch, zero crashes, backup rules bundled in APK (`aapt2 dump xmltree` or `BackupRulesTest` + package flags confirmed), and any fixed Must-Fix bug verified live on the tablet.

- [ ] **Step 4: Verify `feat/pdflector-v2` was never touched and prepare final handback**

```bash
git -C /home/asierboveda/Projects/pdflector status -sb
git status -sb
git log --oneline origin/main..HEAD
```
Expected: `/home/asierboveda/Projects/pdflector` is still clean on `feat/pdflector-v2`; `.worktrees/audit-main` has clean linear commits (docs + Red/Green TDD Must-Fix pairs) ready for human review, physical stylus/PR-04 sign-off, and approval to push/merge and propose Phase 4 cards on Kanvas.
