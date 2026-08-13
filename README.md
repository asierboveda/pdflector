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
docs/              Obsidian vault (project docs + ADRs)
```

## Setup

```bash
cargo run -p pdf_app           # launch the desktop app
cargo run -p pdf_app -- file.pdf   # open a PDF directly
python3 tools/generate_corpus.py  # generate test PDFs into corpus/ (gitignored); needs pillow + reportlab
cargo test -p pdf_core
```

No external library is needed: the binary builds with **MuPDF** (static C
shipped by `mupdf-sys`, AGPL-3.0 — decided in ADR-001). The old PDFium
fetch script is no longer used.

## License

**AGPL-3.0-or-later** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

This project is licensed under the GNU Affero General Public License version 3
or (at your option) any later version.

The PDF engine, **MuPDF** (Artifex Software), is licensed under
AGPL-3.0-or-later and is statically linked via the `mupdf` / `mupdf-sys`
crates (AGPL-3.0) — decided in ADR-001. Third-party attributions and
per-crate licenses are listed in [NOTICE](NOTICE).
