#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Asier Bóveda
#
# Compila ImeHelper.java a classes.dex (helper del buscador con teclado).
# Requiere las tools del SDK Android (javac del host + d8 y android.jar).
# cargo-apk no compila Java: el dex se EMBEBE en el binario Rust en tiempo de
# compilación (include_bytes! en jni.rs), así que regenerarlo exige re-build.
set -euo pipefail

SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
BT="$SDK/build-tools/35.0.0"
PLATFORM="$SDK/platforms/android-35/android.jar"
DIR="$(cd "$(dirname "$0")" && pwd)"

rm -rf "$DIR/out"
mkdir -p "$DIR/out"

javac -source 8 -target 8 \
    -bootclasspath "$PLATFORM" \
    -d "$DIR/out" \
    "$DIR/ImeHelper.java"

"$BT/d8" --release --lib "$PLATFORM" --output "$DIR/out" "$DIR/out/com/pdflector/app/"*.class
cp "$DIR/out/classes.dex" "$DIR/classes.dex"
ls -la "$DIR/classes.dex"
echo "OK: tools/ime/classes.dex (embebido en jni.rs via include_bytes!)"