# PDFLector

Fast, lightweight PDF reader for Android tablets with a stylus. Native Rust, free,
no ads, no telemetry.

PDFLector is a personal engineering project with a clear goal: a PDF reader that
feels instant on a tablet — smooth scrolling, low memory, and useful reading
tools (annotations, dark mode, text selection and an optional AI assistant for
selected passages).

## Status

- **Active development.** Native Android app (library + reader) is the main
  target, with an egui desktop prototype used for fast iteration.
- **Measured on the target device** (TCL NXTPaper 11 Plus, Android 15, MuPDF
  release): **60+ fps at 1× zoom on 3 of 4 test documents** (dense, paper,
  large; the worst case is a raster-scanned PDF) and **peak RSS ≈ 27 MB** against
  a <150 MB target. Full numbers in [docs/benchmark-results.md](docs/benchmark-results.md).

## Features

Reader

- Fast rendering via **MuPDF** (AGPL-3.0) — chosen over PDFium after a measured
  benchmark: 2.7–4× faster render, −21% peak RSS ([ADR-001](docs/adr/ADR-001-motor-pdf.md))
- Pinch-zoom and smooth scroll with a byte-budget LRU page cache
  (5× peak-RAM reduction on the 500-page stress test)
- Long-press text selection (copy + highlight)
- Annotations with the stylus
- Dark mode

Library

- Continue Reading + My Library sections
- Recent documents carousel + files grid

AI assistant (optional — requires API keys)

- "Explain selection": text via Groq, image via Gemini vision, with a hybrid
  fallback (see `crates/pdf_core/src/ai.rs`)

## Architecture

Cargo workspace with four crates:

```
crates/pdf_core/     core library (no UI): engine, render, cache, annotations, AI clients
crates/pdf_app/      egui desktop prototype (fast iteration)
crates/pdf_android/  native Android app (library + reader UI, NativeActivity)
crates/pdf_bench/    criterion benchmark harness
docs/                project docs + ADRs (Obsidian vault)
```

Key decisions are recorded as ADRs:
[ADR-001 engine](docs/adr/ADR-001-motor-pdf.md) ·
[ADR-002 architecture](docs/adr/ADR-002-arquitectura-evince-android.md) ·
[ADR-003 baseline](docs/adr/ADR-003-baseline-evince-vs-pdfium.md) ·
[ADR-004 UI](docs/adr/ADR-004-ui-android.md)

## Getting started

Desktop prototype (Linux):

```bash
cargo run -p pdf_app                # launch the reader
cargo run -p pdf_app -- file.pdf    # open a PDF directly
cargo test -p pdf_core              # run the test suite
```

Android app (cross-compile):

```bash
# Requires: Android NDK r28, platform 35, Rust target aarch64-linux-android
cargo build -p pdf_android --target aarch64-linux-android
```

> The AI assistant needs API keys: create `crates/pdf_android/groq_key.txt` and
> `crates/pdf_android/google_key.txt` (gitignored), or set `GROQ_API_KEY` /
> `GOOGLE_API_KEY` at build time. Without keys the crate **still compiles** —
> the AI feature is disabled at runtime (see `build.rs` and the `.example` files).

Test PDFs are generated with `python3 tools/generate_corpus.py` (pillow +
reportlab) into `corpus/` (gitignored).

## Docs

- [docs/PROYECTO.md](docs/PROYECTO.md) — vision and priorities (Spanish)
- [docs/PLAN.md](docs/PLAN.md) — phased implementation plan (Spanish)
- [docs/benchmark-results.md](docs/benchmark-results.md) — measured performance
- [AGENTS.md](AGENTS.md) — rules for AI agents working on this repo

## License

**AGPL-3.0-or-later** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

PDFLector statically links **MuPDF** (Artifex Software), also AGPL-3.0-or-later,
via the `mupdf` / `mupdf-sys` crates (ADR-001). Third-party attributions are
listed in [NOTICE](NOTICE).
