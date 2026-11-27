# Kanvas Card & TDD Compliance Review — `origin/main` (`fb2cc5c`)

**Date:** 2026-11-27  
**Branch audited:** `origin/main` @ `fb2cc5c` (range `6d80dc9..fb2cc5c`, 15 cards)  
**Test gate at audit baseline (7/7 PASS, 0 warnings, 0 `#[ignore]`):**
- `cargo fmt --all -- --check`: PASS
- `cargo clippy --all-targets -- -D warnings`: PASS
- `cargo clippy -p pdf_android --lib --tests -- -D warnings`: PASS
- `cargo clippy -p pdf_android --lib -- -D warnings`: PASS
- `cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings`: PASS
- `cargo test -p pdf_core`: **225 passed** (107 lib + 118 integration across 15 suites); `cargo test -p pdf_android --lib`: **219 passed**; `folder_sync` 5/5 stress runs PASS; `cargo check -p pdf_app`: PASS
- Python (`pytest`): `tools/tests` **5 passed**, `tools/ink_bench/tests` **31 passed**; Kotlin JVM: `android/product` **13 passed**, `android/ink_bench` **9 passed**

---

## 1. Master Review Ledger (All 15 Cards)

| Card | Title | Kanvas State | Commits (RED → GREEN / Merge) | TDD Compliance | Code Review | TCL 9469X UX / HW Verdict |
|---|---|---|---|---|---|---|
| `TD-01` | Cleanup stale worktrees & temp build artifacts | `done` | `a9ac0a4` (chore) | N/A (hygiene) | **PASS** — `.gitignore` and clean repo hygiene verified | N/A |
| `TD-02` | Remove legacy `google_key.txt` / unused `pdf_bench` paths | `done` | `2f85820`, `7fe04ee` (test) → `eacf570` (impl), merge `7092867`, CI fix `04e3966` | **PASS** (RED→GREEN) | **PASS with stale docs** — `google_key.txt` removed from Rust & Gradle; `.agents/skills/pdflector-rendimiento/SKILL.md` still references it | **PASS** — `assembleProductDebug` succeeds without `google_key.txt` |
| `TD-03` | Fix flaky `folder_sync` wall-clock test & atomic writes | `done` | `a2f0935` (test) → `15cb232` (impl), merge `cc60ba4` | **PASS** (RED→GREEN) | **PARTIAL** — 5/5 flake runs pass, but `folder_sync.rs:1233-1254` still uses wall-clock `Instant::now()` and `fsutil.rs:30-42` has an `AlreadyExists` unlink bug | N/A (host test) |
| `CO-01` | Split monolithic viewer motion/input handler | `done` | `cdb0330` (test) → `319bbca` (impl), merge `47d8de2` | **PASS** (RED→GREEN) | **PASS with minor defect** — clean module split (`input/motion/{mod,viewer,library}.rs`); `viewer.rs:708-722` can leave stale `tool_gesture` on empty `Up` | **PASS** — tap/drag/pinch/chrome verified |
| `CO-02` | Move shared atomic I/O & helpers to `pdf_core` | `done` | `8f3841d` (test) → `718ca80` (impl), merge `84bc7c9` | **PASS** (RED→GREEN) | **PARTIAL** — duplicate `write_json_atomic` (`pdf_android/folder_sync.rs:752`) & `atomic_json` (`pdf_core/folder_sync.rs:1130`) remain; raw `fs::write` in 5 call sites (`jni/ime.rs:57` risks corrupt `classes.dex`) | N/A |
| `CO-03` | Library menu layout & hitbox unification | `done` | `3d2c5d5` (test) → `0321366` (impl), merge `b239ac7` | **PASS** (RED→GREEN) | **PARTIAL** — view/settings menus unify cleanly; minor 6 px search-chip drift (`input/motion/library.rs:425`) and dead `lib_org_chips` (`L336-367,439-447`) | **PASS** — `⋯` menu (`tcl_ux_02_view_menu.png`) aligned, crisp checkmarks, zero overlap |
| `UI-01` | Reader `"Aa"` glyph & Library header icon heights | `done` | `27f847d` (test) → `ceb40f8`, `d69002f` (impl), merge `6ea3baa` | **PASS** (RED→GREEN) | **PASS with minor contrast defect** — `"Aa"` and `⋯`/`☰`/`＋ Añadir` match heights; `DefaultDark` `"Aa"` accent underline contrast is 2.94:1 (< 3.0:1 WCAG AA UI) (`draw/chrome.rs:154-162`) | **PASS** — `"Aa"` crisp (`tcl_ux_04`), `⋯`/`☰`/`＋ Añadir` level (`tcl_ux_01`) |
| `UI-02` | Dependabot updates & stale PR/issue triage | `done` | `55f70f6`..`1ef5485` (PRs #58/#59; closed #41,#42,#44,#54,#57) | N/A (deps/triage) | **PASS** — workspace builds & clippy clean across host + ARM64 | **PASS** — APK builds & runs cleanly |
| `IN-01` | AndroidX Ink wet overlay integration + PR #55 port | `done` | `40d24aa` (PR #55 port), `c067b58` (test) → `9aa797a` (impl) | **PASS** (RED→GREEN) | **2 MUST-FIX DEFECTS** — (1) `InkStrokeGeometry.kt:68-77` emits `NaN`/`Infinity` GL fan vertices for non-finite coords; (2) `reader/tools.rs:291-298` calls `mark_repaint()` unconditionally on pen-down even when wet overlay handles stroke (PR #55 regression) | **NEEDS HUMAN STYLUS CHECK** (item #1–#3 below) |
| `IN-02` | Split `jni/ink.rs` & ink projection geometry tests | `done` | `c5a9c96` (test) → `3f16afa` (impl), merge `9611a10` | **PASS** (RED→GREEN) | **PASS** — 13 Kotlin JVM unit tests (`InkLedgerTest`, `InkProjectionTest`, `InkStrokeGeometryTest`) cleanly isolated | N/A |
| `IN-03` | Directional prefetch & JNI module split | `done` | `8bd69c2` (test) → `f5f8194`, `9dd7b73` (impl), merge `c1cbf93` | **PASS** (RED→GREEN) | **2 MUST-FIX JNI DEFECTS** — (1) `jni/ime.rs:148,178,200,237` wraps cached `GlobalRef` with owning `JClass::from_raw` (calls `DeleteLocalRef` on global ref → CheckJNI abort) and omits `exception_clear()`; (2) `jni/window.rs:101-126,154-173` omits `exception_clear()` before `setAttributes` and skips null-check on `getInsetsController()` (API 29 crash) | **PASS on API 36** (Android 16), latent crash on API 29 (`minSdk=29`) and CheckJNI builds |
| `PR-01` | Continuo vertical scroll mode + per-doc persistence | `done` | 5 RED/GREEN pairs (`94502a5→0be38c0`, `7e8b2f4→6e977c0`, `601e125→1cd6e22`, `81d4851→c425ef8`, `401df8c→96a4d8b`) + wiring `cecfce6`, merge `aa09fec` | **PASS** (5 strict RED→GREEN cycles) | **2 MUST-FIX + PERF DEFECTS** — (1) `reader/life.rs:211-238` (`ACTION_VIEW`) bypasses `load_state`, opens in `Tap` @ page 0, and overwrites `state.json` with defaults; (2) `enter_library`/`open_pdf_at` leak `self.doc`, `page_geometry_cache`, and active `scroll_fling`; (3) `PageSlots` creates/deletes new GL textures on every scroll page → `+71–75 MB` `GL mtrack` spike | **PARTIAL** — mixed portrait/landscape stacking (`tcl_ux_06`) & normal-launch persistence PASS; `ACTION_VIEW` reset bug reproduced live (`tcl_ux_10`); `frame p95 = 11.8–14.6 ms` misses 120 Hz (`8.33 ms`) |
| `PR-02` | Study workflow (Gemini API key storage + Document Notebook) | `done` | 4 RED/GREEN pairs (`f0bc73b→c5c20db`, `0a00d0a→6a2f0a3`, `641f28c→46be09d`, `dcbb39b→0989c05`) + wiring `706501a`, merge `10135fd` | **PASS** (4 strict RED→GREEN cycles) | **3 MUST-FIX SECURITY DEFECTS** — (1) **D2:** `AndroidManifest.xml:6-8` lacks `backup_rules.xml` / `data_extraction_rules.xml` → Google Cloud Backup backs up `files/gemini_api_key` despite UI promising `"Se guarda solo en este dispositivo"`; (2) `gemini_key.rs:133-138` writes temp file with default umask before `chmod 0600` (TOCTOU); (3) `close_study()` (`reader/study.rs:297-304`) leaves typed API key in Java static `ImeHelper.sText` | **PASS (functional)** — short-key error (`tcl_ux_08a`), masked `••••0000` (`tcl_ux_08e`), `-rw-------`, 0 app logcat leaks, `Borrar clave`, 2-note `Cuaderno` (`tcl_ux_09b`) & `notebooks.json` persistence all verified |
| `PR-03` | Display-list cache (`dlcache`) memory cap (24 MiB / 16 pages) | `done` | `f797ce6` (test) → `44ff2d7` (impl), merge `78e7b58` | **PASS** (RED→GREEN) | **PARTIAL (calibration gap)** — `DisplayListCache` enforces its own 16-entry / 24 MiB cap cleanly (`bytes=564 KB`), but MuPDF's process-wide `fz_store` (256 MiB default) stays uncapped | **FAILS OVERALL RAM GOAL** — `dlcache` reports `0.56 MB`, while real app hits **461.8 MB PSS** (Native Heap `+192 MB`, GL mtrack `+71 MB`) during 30-swipe `mixed_stress.pdf` scroll |
| `PR-04` | Mixed page-size layout, fling physics & tap-to-brake | `review` | 5 RED/GREEN pairs (`7f6a179→6ff6e22`, `33b0af4→2a0fe62`, `152c45d→75ab522`, `c696e9e→0a79bbc`, `d301590→e435e8b`), merge `6547834`, CI fix `a0eb70f` | **PASS** (5 strict RED→GREEN cycles) | **PARTIAL** — mixed portrait/landscape geometry & fling decay unit-tested; `MainEvent::Pause` (`lib.rs:599-606`) never cancels active `scroll_fling` or `tool_gesture`; `reader/navigation.rs:88-89` fails to invalidate `chrome_bottom_bitmap` on page turn (live-reproduced stale `Pág. 4 de 80` on Page 5) | **AUTOMATED PASS / AWAITING HUMAN SIGN-OFF** — landscape+portrait stacking (`tcl_ux_06`) and tap-to-brake without showing chrome (`tcl_ux_07`) verified on TCL 9469X; stays in `review` for human sign-off |

---

## 2. Live TCL 9469X UX Audit Summary (Screenshots Verified)

All tablet walkthroughs used **only** synthetic `corpus/` PDFs (`scientific_paper.pdf`, `large_document.pdf`, `mixed_stress.pdf`) in isolated app-private storage; zero personal PDFs were opened or screenshotted.

| Screen / Flow | Screenshot | Result | Observed UX / Visual Notes |
|---|---|---|---|
| Library Grid + Header (`UI-01`) | `tcl_ux_01_library.png` | **PASS** | `⋯`, `☰`, `＋ Añadir` aligned at identical height; crisp typography & cover thumbnails. |
| Library View Menu (`CO-03`) | `tcl_ux_02_view_menu.png` | **PASS (minor halo)** | Rows aligned, checkmarks crisp, zero clipping. Subtle cyan/light-blue edge halo around white popup shadow (premultiplied alpha blend artifact). |
| Reader Chrome & `"Aa"` (`UI-01`) | `tcl_ux_04_reader_chrome.png` | **PASS (minor halo)** | `"Aa"` glyph crisp with accent underline; clean tool dock on left. Same subtle cyan border halo on chrome bars. |
| Reader Settings Sheet (`PR-01`, `PR-02`) | `tcl_ux_05_reader_sheet.png`, `tcl_ux_08e_sheet_with_key.png` | **PASS** | `APARIENCIA`, `MODO DE LECTURA`, `NAVEGACIÓN`, `ESTUDIO` (`Cuaderno`, `Clave IA ••••0000`), `DOCUMENTO` well-spaced with ≥48 dp touch targets. |
| Page Turn with Chrome Visible | `tcl_ux_08c`, `tcl_ux_08f`, `tcl_ux_09` | **DEFECT FOUND** | Bottom scrub bar displays stale page number (`Pág. 1 de 80` on Page 2; `Pág. 4 de 80` on Page 5; `Pág. 5 de 80` on Page 4) because `goto_page()` (`reader/navigation.rs:88-89`) never sets `self.chrome_bottom_bitmap = None`. |
| Continuo Mixed Sizes (`PR-01`, `PR-04`) | `tcl_ux_06_continuo_mixed.png` | **PASS** | Landscape Page 2 (`842×595 pt`) and Portrait Page 3 (`595×842 pt`) stack cleanly at full width with distinct aspect heights, zero cropping. |
| Fling Tap-to-Brake (`PR-04`) | `tcl_ux_07_fling_brake.png` | **PASS** | Center tap during active fling stopped scroll on Page 4/5 boundary **without** toggling chrome toolbars on. |
| Gemini Key Editor (`PR-02`) | `tcl_ux_08_gemini_key.png`, `08a`, `08f` | **PASS (D2 security gap)** | Short-key validation error renders clearly; saving dummy key writes `-rw-------` with 0 `pdf_android` logcat leaks; `Borrar clave` removes file. UI claims `"Se guarda solo en este dispositivo"`, contradicting missing Android backup exclusion (D2). |
| Document Notebook (`PR-02`) | `tcl_ux_09_notebook.png`, `09a`, `09b` | **PASS** | Empty state, note editor, newest-first 2-note list, and `files/notebooks.json` persistence all clean. |
| Landscape Rotation (`2200×1440`) | `tcl_ux_10_landscape.png` | **PASS (exposes `ACTION_VIEW` bug)** | Zero blank frame, clean centered portrait page & left dock. Relaunch via `ACTION_VIEW` reset `SepiaLight`/Page 4 back to `DefaultLight`/Page 1 (confirming `reader/life.rs:211-238` bug). |

---

## 3. Stale Documentation & CI Gaps Found

1. **`.agents/skills/pdflector-rendimiento/SKILL.md:89-96,161-162,251`**: Still instructs creating `crates/pdf_android/google_key.txt` before building `pdf_android`, even though `TD-02` deleted `google_key.txt` support.
2. **`.github/workflows/ci.yml`**: Never runs `cargo test -p pdf_app` or host `cargo clippy -p pdf_android --lib --tests -- -D warnings`.
3. **`crates/pdf_android/src/cache.rs:139`**: Stale `#[allow(dead_code)]` on `full_h` (actively read at `gpu/pipeline.rs:640`).

---

## 4. Human-Only Physical Tablet Checklist (TCL NXTPaper 11 Plus — 9469X)

ADB cannot physically simulate passive/active stylus digitizer physics, palm contact, or human 120 Hz visual perception. Please verify these 5 items hands-on before moving `PR-04` from `review` to `done`:

- [ ] **1. Stylus feel & tip-to-ink alignment (`IN-01`):** Draw fast diagonal strokes and small equations in portrait and landscape with the TCL T-Pen on `mixed_stress.pdf`; confirm zero noticeable parallax, lag, or dropped stroke starts.
- [ ] **2. Wet→dry pen-lift seam (`IN-01`):** Lift the stylus cleanly mid-page and watch the handoff from the AndroidX front-buffered wet overlay to the native dry FBO; confirm zero flicker, thickness jump, color shift, or double-drawn segment.
- [ ] **3. Palm rejection while writing (`IN-01` / `CO-01`):** Rest your palm naturally on the screen while annotating in `Continuo` mode; confirm the page does not accidentally scroll, zoom, or trigger toolbar chrome.
- [ ] **4. Subjective 120 Hz Continuo scroll & fling feel (`PR-01`, `PR-04`):** Flick rapidly through `mixed_stress.pdf` (80p mixed portrait/landscape) and `large_document.pdf` (500p), then tap mid-fling to brake; note whether the measured `frame p95 = 11.8–14.6 ms` (visible stutter vs `8.33 ms` 120 Hz cadence) is acceptable for `PR-04` acceptance or should block until the Phase 4 GPU/scroll optimizations land.
- [ ] **5. Human `PR-04` sign-off on Kanvas:** If items 1–4 feel right after Phase 5 Must-Fix patches are installed, move `PR-04` from `review` → `done` in `kanvas.json` (intentionally left in `review` by this audit).
