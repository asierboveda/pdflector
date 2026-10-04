## Resumen
<!-- Qué hace este PR y por qué. -->

## Issue
<!-- Qué Issue de GitHub cierra o afecta este PR. Si no aplica, escribe N/A. -->

## Verificación
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test -p pdf_core`
- [ ] Compilación Android aarch64 (`cargo check -p pdf_android --target aarch64-linux-android --all-targets` o `cargo apk build --release`)
- [ ] Si toca `pdf_android` / interacción: probado en la tablet TCL con lápiz y logcat sin errores (`adb logcat -d -s pdf_android:V`)
- [ ] Si toca render/caché/memoria: medición con formato exigido (fecha + hardware + flujo + métrica) registrada en `docs/benchmark-results.md`
- [ ] Documentación actualizada según AGENTS.md
