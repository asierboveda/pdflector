# PDFLector

Fast, lightweight PDF reader for Android tablets with a stylus. Free, no ads,
no telemetry.

Final platform: native Android (`crates/pdf_android` hosted by `android/product`).
The desktop `egui` app (`crates/pdf_app`) is a core testbed and prototype, not
the final product.

## Features

- **Reading & Performance**: Page flipping, pinch-to-zoom, and continuous panning with texture caching.
- **Stylus Annotations**:
  - Text highlighter with automatic text detection aligned to reading order.
  - Low-latency causal vector ink drawing (`ink::causal`) with AndroidX front-buffered overlay.
  - Vector eraser removing entire strokes and lasso ink crop tool.
  - Background asynchronous persistence to SQLite.
- **Discover / arXiv**: Integrated paper search across arXiv, metadata inspection, background PDF download, and immediate handoff to the reader.
- **Curated Library**: Folder selection via Android SAF / MediaStore and local folder sync without automatic deletion.
- **In-Document Search**: Interactive text search with native Android IME keyboard support.
- **AI Assistant**: Built-in Gemini panel for text explanations and cropped equation/figure analysis (requires user API key).
- **Export**: Notes export to Markdown and PDF with embedded vector annotations.

## Layout

```
crates/pdf_core/     core library (no UI): engine, render, cache, annotations, arxiv, ai, export
crates/pdf_android/  Rust cdylib for the Android product (GameActivity + EGL/GLES; packaged by android/product)
crates/pdf_app/      egui desktop testbed and prototype (not product)
crates/pdf_bench/    benchmark harness and performance sweeps
android/product/     Gradle/Kotlin host app (com.pdflector.app)
corpus/              test PDFs (gitignored; tools/generate_corpus.py)
```

## Setup

```bash
cargo run -p pdf_app               # launch the desktop testbed
cargo run -p pdf_app -- file.pdf   # open a PDF directly
python3 tools/generate_corpus.py   # generate test PDFs into corpus/ (needs pillow + reportlab)
cargo test -p pdf_core
```

No external library is needed: the binary builds with **MuPDF** (static C
shipped by `mupdf-sys`, AGPL-3.0).

## Android (tablet TCL 9469X)

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"

# Create placeholder API key if not present (required by crates/pdf_android/src/lib.rs):
echo "placeholder" > crates/pdf_android/google_key.txt

cargo check -p pdf_android --target aarch64-linux-android                  # quick check
```

The shipped product APK (`com.pdflector.app`) is a Gradle/Kotlin app in
`android/product` that hosts the Rust `cdylib` through `GameActivity`
(`PdfLectorActivity`). Its `assembleProductDebug` Gradle task
(`android/product/app/build.gradle.kts`) builds the Rust library for
`aarch64-linux-android` in release mode and packages it into the debug APK.
Signing keys live in `android/product/signing.local.properties` (gitignored),
never in the manifest.

## License

**AGPL-3.0-or-later** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

This project is licensed under the GNU Affero General Public License version 3
or (at your option) any later version.

The PDF engine, **MuPDF** (Artifex Software), is licensed under
AGPL-3.0-or-later and is statically linked via the `mupdf` / `mupdf-sys`
crates (AGPL-3.0). Third-party attributions and per-crate licenses are listed
in [NOTICE](NOTICE).
