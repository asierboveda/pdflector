#!/usr/bin/env bash
# Downloads the pinned prebuilt PDFium binary for Android ARM64
# (bblanchon/pdfium-binaries, BSD-3-Clause) into vendor/pdfium-android-arm64/.
# Needed to cross-compile pdf_core (pdfium backend) for aarch64-linux-android
# (spike Fase 0.5). vendor/ is gitignored on purpose.
set -euo pipefail

PDFIUM_RELEASE="chromium/7988" # PDFium 153.0.7988.0 — same release as the Linux build
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/pdfium-android-arm64"

if [ -f "$DEST/lib/libpdfium.so" ]; then
    echo "PDFium Android ARM64 already present at $DEST"
    exit 0
fi

mkdir -p "$DEST"
curl -sL "https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_RELEASE}/pdfium-android-arm64.tgz" \
    | tar xz -C "$DEST"
echo "PDFium ${PDFIUM_RELEASE} (android-arm64) extracted to $DEST"
