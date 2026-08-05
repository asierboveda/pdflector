# PDFLector

Fast, lightweight PDF reader for Android tablets with a stylus. Free, no ads,
no telemetry. Personal learning project (first real Rust project).

Desktop-first development on Linux; Android is the final target.

## Docs

- [`docs/PROYECTO.md`](docs/PROYECTO.md) — vision and priorities (Spanish)
- [`docs/PLAN.md`](docs/PLAN.md) — phased implementation plan (Spanish)
- [`AGENTS.md`](AGENTS.md) — rules for AI agents working on this repo

## Layout

```
crates/pdf_core/   core library (no UI): engine, render, cache, annotations
crates/pdf_app/    egui desktop prototype
crates/pdf_bench/  benchmark harness (Phase 0.5+)
corpus/            test PDFs (gitignored; tools/generate_corpus.py)
vendor/pdfium/     prebuilt PDFium (gitignored; tools/fetch_pdfium.sh)
docs/              Obsidian vault (project docs + ADRs)
```

## Setup

```bash
./tools/fetch_pdfium.sh        # one-time: downloads pinned libpdfium
cargo run -p pdf_app           # launch the desktop app
cargo run -p pdf_app -- file.pdf   # open a PDF directly
cargo test -p pdf_core
```

## License

To be decided after ADR-001 (engine choice): MIT/Apache-2.0 with PDFium,
AGPL if MuPDF wins the Phase 0.5 benchmark.
