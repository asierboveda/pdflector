# Contribuir a PDFLector

Las convenciones del proyecto (arquitectura, prioridades, cómo trabajar,
documentación obligatoria) están en [`AGENTS.md`](AGENTS.md) y [`docs/PROYECTO.md`](docs/PROYECTO.md).
El roadmap activo es [`docs/plan/NEXT-PLAN.md`](docs/plan/NEXT-PLAN.md) (fases A–E);
`docs/PLAN.md` y las fases 1–6 son histórico de referencia.

Proyecto personal de aprendizaje en Rust; aportes externos no esperados, pero bienvenidos.

## Verificación local (antes de abrir PR)

```bash
python3 tools/generate_corpus.py        # si falta corpus
cargo test -p pdf_core
cargo fmt --all && cargo clippy --all-targets -- -D warnings
# Android (NDK r28, ver README):
cargo check -p pdf_android --target aarch64-linux-android
```
