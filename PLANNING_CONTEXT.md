# Planning context: how this project is being worked on

Read this before planning or starting work. It records the process used on
2026-11-26 (one long cloud session), what was done, what went wrong, and what
is still open. Board state itself lives in `kanvas.json`; run
`python3 .agents/skills/kanvas/kanvas.py status` for the live view.

## 1. Where things stand

- **Board:** 15 cards, 14 done. **PR-04** (Continuo scroll polish) is in review,
  waiting for the human's tablet check.
- **main** = session branch `ccr-dd02141f-tcbsbp` (same commit). main was
  fast-forwarded after every card, only once CI was green.
- **Tests:** 219 `pdf_android` host tests, all `pdf_core` suites, Kotlin JVM
  tests (13 in `android/product`), clippy `-D warnings` and fmt all green in CI.
- **Open decision for the human:** Android cloud backup may include the Gemini
  key and notebooks (`allowBackup` is not set). Keep, or exclude the key?

## 2. How the human works with the board

- **Commit identity:** commits are authored by the human, with Claude as
  co-author. Set this at the start of every session:
  `git config user.name asierboveda` and
  `git config user.email 145044583+asierboveda@users.noreply.github.com`,
  and end every message with the `Co-Authored-By: Claude ...` trailer.
  On 2026-11-27 the 105 commits of the first session that were authored by
  "Claude" were rewritten to this identity and force-pushed with the human's
  OK, so commit IDs quoted in old PR comments and board notes no longer exist.

- The human uses the **chat only**; the Kanvas web board (`kanvas.py serve`)
  runs inside the cloud container and is unreachable for them.
- So the chat replaces the board for human decisions. When the human
  explicitly says so in chat, record it with
  `python3 tools/kanvas_human.py <approve|reject|done|sendback|reopen> ID... -m "..."`
  (signs as human, notes "relayed from chat", validates the board).
  Rules are in `AGENTS.md`.
- **Never infer approval.** Example: "start with pr04" was *not* treated as
  "done PR-01" (the permission check blocked it, rightly); the human then said
  "done pr01" explicitly.
- The human asked for: a commit + push at every step; agents on all cards;
  the orchestrator to review that TDD is really done.
- Human-granted permissions (still valid): merge to main and merge/close the
  Dependabot PRs (UI-02); merge this session's work into main ("merge all and
  keep all fixed").

## 3. The process (repeat this)

1. **Plan on the board.** Cards are proposed, the human approves in chat.
2. **One agent per card, in its own git worktree** (`isolation: worktree`).
   - First instruction: `git rebase ccr-dd02141f-tcbsbp`. Worktrees start from
     the repo's default branch, which can be behind the session branch (this
     bit us once: agents lacked the Linux test setup).
   - Seed the build cache: `cp -r /home/user/pdflector/target ./target`
     (MuPDF takes minutes to build otherwise).
   - Agents commit locally only; they never push or touch `kanvas.json`.
3. **TDD contract for agents:** for every slice, a test-only commit first
   (tests + minimal stubs that compile, so the tests FAIL), then the
   implementation commit. Refactors/moves: characterization tests first that
   pass on the old code. Never weaken, skip, `#[ignore]` or delete tests.
4. **Orchestrator review (do not trust the report; re-run it):**
   - base contains the session branch;
   - each test commit only adds tests/stubs (`git show --shortstat`, read any
     deleted lines);
   - check out every test commit → tests FAIL for the right reason; check out
     every impl commit → tests PASS;
   - no removed asserts / tests, no added `#[ignore]`;
   - for secrets: grep that nothing logs them.
5. **Merge into the session branch behind the gate**, and only push if every
   line says ok:
   ```bash
   cargo test -p pdf_android --lib
   cargo test -p pdf_core          # needs the corpus, see §5
   cargo clippy -p pdf_android --lib --tests -- -D warnings
   cargo clippy -p pdf_android --lib -- -D warnings     # catches test-only code
   cargo clippy --all-targets -- -D warnings            # skips pdf_android!
   cargo fmt --all -- --check
   ```
6. **Push the session branch → wait for CI → only then push to main.** CI's
   `android` job (aarch64 check + clippy) is the only real Android build; the
   container has no NDK. Use a 5-minute `send_later` reminder to check CI.
7. **Kotlin (no Android SDK here):** push the test commit alone and confirm CI
   fails on *assertions* (not compile errors) in the `android-jvm` job, then
   push the fix and confirm green. Done this way for IN-01 (runs 228 → 229).
8. `finish` the card with a summary written for the human (what changed for
   them, how it was verified, tablet checklist), commit + push, remove the
   merged worktree.

## 4. What was done (in order)

| Card | Result |
|---|---|
| TD-01 | `pdf_android` tests run on Linux + CI: Android crates behind `cfg(target_os = "android")`, host stand-ins in `src/host/` and `src/jni/host.rs`; GL ffi `*const u8` → `c_char` fix |
| UI-01 | Reader theme button draws a real "Aa" glyph; library ⋯/☰ match the ＋ Añadir pill (shared `lib_add_btn_rect`) |
| IN-02 | `input/motion.rs` split into `input/motion/{mod,viewer,library,discover,picker}.rs` |
| CO-01 | `pdf_core::write_atomic` (temp + fsync + rename) for all persist saves and Markdown export |
| CO-02 | `theme` moved from `pdf_core` to `pdf_app` |
| CO-03 | Menus drawn from their hit-test layout; toast wraps to 3 lines with "…" |
| TD-02 | Dead theme tokens and 15 `allow(dead_code)` removed |
| IN-03 | Prefetch render errors surfaced (`take_errors`); `jni.rs` split into `jni/{window,launch,parse,media,ime}.rs` |
| TD-03 | Two flaky folder_sync timing tests made causal (injectable debounce/backoff) |
| UI-02 | main fast-forwarded with Astryx + all work; Dependabot #41 #42 #44 #54 #57 closed (already applied), #58 #59 applied |
| IN-01 | Wet ink capsules with round caps/joins (`InkStrokeGeometry.kt`) matching dry `draw_polyline_gpu`; PR #55 ported (cherry-pick -x of 04fd3bc) and closed |
| PR-01 | Continuo (Scroll) reading mode per document, toggle in the reader sheet |
| PR-03 | Display-list LRU budget (16 lists / 24 MiB per doc), `dlcache` log counters |
| PR-02 | In-app Gemini key (private file, masked, never logged; `google_key.txt` no longer needed) + per-document notebook (`notebooks.json`) |
| PR-04 | Fling, per-page slot heights, live annotation changes on any visible page, render-failure back-off (in review) |

## 5. Lessons learned (feedback on the process)

- **Test-only code breaks the Android build.** The host build hides dead code
  (`cfg_attr(not(android), allow(dead_code))` in `lib.rs`), and test builds use
  everything. Anything read only by `#[cfg(test)]` code fails CI's Android
  clippy. Happened 3 times (TD-02 `success_bg/text`, a TD-02 constant assert,
  PR-04 `Fling::velocity`). Fix by putting test-only helpers under
  `#[cfg(test)]`, never with `allow`. Always run
  `cargo clippy -p pdf_android --lib -- -D warnings` and, for new functions,
  check each is used outside test modules.
- **Workspace clippy skips `pdf_android`** (not a default member). An agent
  reported "clippy clean" from it; run the `-p pdf_android` variants.
- **Never chain "test then push" without a hard gate.** Once a merge was pushed
  after one test run failed (grep exited 0). Check the result line says `ok`.
- **Timing-based tests flake on CI.** Prefer injected clocks/durations and
  ordering checks over wall-clock limits (TD-03). One remaining test still
  compares two `Instant::now()` readings (`local_save_failure_wakes_for_retry...`).
- **`pdf_core` cache tests need the corpus:** generate it (gitignored) with
  `python3 -m venv V && V/bin/pip install pillow reportlab && V/bin/python tools/generate_corpus.py`.
  Don't dismiss failing cache tests as "missing corpus" when the change
  touches the cache — generate it and run them.
- **Instructions to agents must be precise.** "A token used by a test is not
  dead" (orchestrator's wording) caused the TD-02 CI failure.
- **Parallel agents conflict on shared files** (UI-01 vs IN-02 on the add-pill
  tap). Resolve by re-applying the smaller change onto the moved code.
- **Agents can stop at usage limits;** resume the same agent with SendMessage
  instead of starting a new one (PR-01).
- **Environment limits:** no Android NDK/SDK (dl.google.com blocked), so no
  local Android or Gradle builds; the tablet (TCL 9469X) is never reachable;
  every card ends with a tablet checklist for the human.

## 6. Open items and follow-up ideas (not on the board yet)

- Decision: cloud backup of the Gemini key / notebooks (see §1).
- Human tablet checks for all UI cards (checklists are in each card's notes).
- MuPDF's own decoded-image cache (default 256 MB) is still uncapped (PR-03).
- Eraser strokes only act on the page they start on (Continuo).
- Unseen pages use a reference size until rendered (no page-size pre-scan).
- Notes are single-line (IME bridge has no line breaks); note list scrolls
  with ▲/▼ only.
- Wet ink has no antialiasing (dry has ~1 px).
- Design point: exports and local saves share one worker thread, so a save can
  wait behind one running export (TD-03 finding).
- Left untouched on purpose: branch `feat/pdflector-v2` (own board), and the
  stale `actualizar` worktree, which is on the human's computer.
