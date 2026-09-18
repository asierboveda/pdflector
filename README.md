# PDFLector

Fast, lightweight PDF reader for Android tablets with a stylus. Free, no ads,
no telemetry. Personal learning project (first real Rust project).

Final platform: native Android (`crates/pdf_android`, ADR-005).
The desktop `egui` app (`crates/pdf_app`) is a core testbed and prototype,
not the final product. Active roadmap: phases A–F (`docs/plan/NEXT-PLAN.md`).

## Features

- **Reading & Performance**: Fluid page flipping, pinch-to-zoom, and continuous panning with texture caching. Sustained 60 fps minimum (frame time < 16.6 ms) and target 120 fps (frame time < 8.33 ms) on the TCL NXTPaper 11 Plus 120 Hz display, with low memory footprint (PSS < 150 MB).
- **Stylus Annotations**:
  - Text highlighter with automatic text detection aligned to reading order.
  - Low-latency vector ink drawing with motion prediction (Kalman filter and spring-mass model).
  - Vector eraser removing entire strokes.
  - Background asynchronous persistence to SQLite.
- **Discover / arXiv**: Integrated paper search across arXiv, metadata inspection, background PDF download, and immediate handoff to the reader.
- **Curated Library**: Folder selection via Android SAF / MediaStore; retains files without automatic deletion.
- **In-Document Search**: Interactive text search with real Android native IME keyboard support.
- **Themes**: 4 color schemes (Default Light, Sepia Light, Default Dark, Sepia Dark).
- **AI Assistant**: Built-in panel powered by Groq (`llama-3.3-70b-versatile`) for text explanations and Google Gemini (`gemini-flash-latest`) for equation and figure crop analysis (requires user API keys).
- **Export**: Notes export to Markdown and PDF with embedded vector annotations (available in desktop testbed).

## Docs

- [`docs/PROYECTO.md`](docs/PROYECTO.md) — vision and product scope (Spanish)
- [`docs/plan/NEXT-PLAN.md`](docs/plan/NEXT-PLAN.md) — active roadmap, phases A–F (Spanish)
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — contribution guidelines and local verification
- [`AGENTS.md`](AGENTS.md) — operational rules for AI agents working on this repo

## Layout

```
crates/pdf_core/     core library (no UI): engine, render, cache, annotations, arxiv, ai, export
crates/pdf_android/  final native Android app (ADR-005, NativeActivity + EGL/GLES)
crates/pdf_app/      egui desktop testbed and prototype (not product)
crates/pdf_bench/    benchmark harness and performance sweeps
corpus/              test PDFs (gitignored; tools/generate_corpus.py)
docs/                project documentation and ADRs (active roadmap: NEXT-PLAN A–F)
```

## Setup

```bash
cargo run -p pdf_app               # launch the desktop testbed
cargo run -p pdf_app -- file.pdf   # open a PDF directly
python3 tools/generate_corpus.py   # generate test PDFs into corpus/ (needs pillow + reportlab)
cargo test -p pdf_core
```

No external library is needed: the binary builds with **MuPDF** (static C
shipped by `mupdf-sys`, AGPL-3.0 — decided in ADR-001). The old PDFium
fetch script is no longer used.

## Android (tablet TCL 9469X)

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

# Create placeholder API keys if not present (required by crates/pdf_android/src/lib.rs):
echo "placeholder" > crates/pdf_android/groq_key.txt
echo "placeholder" > crates/pdf_android/google_key.txt

cargo check -p pdf_android --target aarch64-linux-android                  # quick check
cargo apk build -p pdf_android --release --target aarch64-linux-android # build APK
```

See also: `docs/README.md` (documentation index), `docs/adr/` (architectural decision records),
`docs/benchmark-results.md` (benchmark measurements), and `.opencode/skills/` (operational skills).

## License

**AGPL-3.0-or-later** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

This project is licensed under the GNU Affero General Public License version 3
or (at your option) any later version.

The PDF engine, **MuPDF** (Artifex Software), is licensed under
AGPL-3.0-or-later and is statically linked via the `mupdf` / `mupdf-sys`
crates (AGPL-3.0) — decided in ADR-001. Third-party attributions and
per-crate licenses are listed in [NOTICE](NOTICE).
