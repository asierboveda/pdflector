#!/usr/bin/env bash
# Downloads the pinned prebuilt PDFium binary (bblanchon/pdfium-binaries,
# BSD-3-Clause) into vendor/pdfium/. Needed to build/run pdf_core (pdfium
# backend) and its tests. vendor/pdfium is gitignored on purpose.
set -euo pipefail

PDFIUM_RELEASE="chromium/7988" # PDFium 153.0.7988.0
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/vendor/pdfium"

if [ -f "$DEST/lib/libpdfium.so" ]; then
    echo "PDFium already present at $DEST"
    exit 0
fi

mkdir -p "$DEST"
curl -sL "https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_RELEASE}/pdfium-linux-x64.tgz" \
    | tar xz -C "$DEST"
echo "PDFium ${PDFIUM_RELEASE} extracted to $DEST"
