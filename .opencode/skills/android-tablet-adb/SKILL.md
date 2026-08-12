---
name: android-tablet-adb
description: Despliegue y medición en la tablet Lenovo Idea Tab (9469X) vía adb:
cross-compile Rust a aarch64-linux-android (NDK r28/API 35), push de binarios y
corpus, ejecución del harness pdf_bench (timings+RSS), dumpsys meminfo y logcat.
Úsalo cuando toque medir en la tablet o instalar/verificar algo en ella.
---

## Precondiciones

- Tablet por USB (`adb devices`): Lenovo Idea Tab **9469X**, Android 15,
  serial A06B4A8E6774623.
- NDK r28 en `~/Android/Sdk/ndk/` + target Rust `aarch64-linux-android`
  (`rustup target add aarch64-linux-android`).
- Toolchain ya configurado en `.cargo/config.toml` del repo (linker, CC/AR,
  `BINDGEN_EXTRA_CLANG_ARGS` con sysroot NDK) — **NO reconfigurar**.

## Build

```bash
cargo build -p pdf_bench --target aarch64-linux-android --release
```

## Despliegue y medición

```bash
adb push target/aarch64-linux-android/release/pdf_bench /data/local/tmp/
adb push corpus/<pdf>.pdf /data/local/tmp/
adb shell chmod +x /data/local/tmp/pdf_bench
adb shell '/data/local/tmp/pdf_bench /data/local/tmp/<pdf>.pdf 20 2.0'
# reporta: open time, ms/página, VmRSS/VmHWM
```

## Memoria de la app (Fase 1+)

```bash
adb shell dumpsys meminfo <paquete>
adb logcat
```

## Limpieza

```bash
adb shell rm -f /data/local/tmp/pdf_bench /data/local/tmp/*.pdf
```

## Problemas conocidos

- Error `pthreadtypes-arch.h` (glibc del host) al compilar mupdf-sys → falta
  `BINDGEN_EXTRA_CLANG_ARGS` con `--sysroot` del NDK en `.cargo/config.toml`.
- Error "file in wrong format" al linkear → falta el `linker` del NDK en
  `[target.aarch64-linux-android]` del `.cargo/config.toml`.
