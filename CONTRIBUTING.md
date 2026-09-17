# Contribuir a PDFLector

Las convenciones del proyecto (arquitectura, prioridades, cómo trabajar,
documentación obligatoria) están en [`AGENTS.md`](AGENTS.md) y [`docs/PROYECTO.md`](docs/PROYECTO.md).
El roadmap activo es [`docs/plan/NEXT-PLAN.md`](docs/plan/NEXT-PLAN.md) (fases A–F).

Proyecto personal de aprendizaje en Rust; aportes externos no esperados, pero bienvenidos.

## Verificación local (antes de abrir PR)

```bash
# 1. Corpus de prueba y tests del core
python3 tools/generate_corpus.py        # generar corpus si no existe (requiere pillow + reportlab)
cargo test -p pdf_core

# 2. Formato y linting estricto
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings

# 3. Verificación de compilación Android (NDK r28, ver README.md)
# crates/pdf_android/src/lib.rs:420,422 incluye claves mediante include_str!.
# En un clon limpio, es imprescindible crear ficheros placeholder (igual que hace el CI en .github/workflows/ci.yml):
echo "placeholder" > crates/pdf_android/groq_key.txt
echo "placeholder" > crates/pdf_android/google_key.txt
cargo check -p pdf_android --target aarch64-linux-android
```

## Medición y despliegue en hardware real

Para compilar la APK, desplegar en la tablet TCL NXTPaper 11 Plus vía `adb`, ejecutar
benchmarks con `pdf_bench` y medir métricas reales de producto (PSS con `dumpsys meminfo`,
frame time p95 de `gl_present`, etc.), sigue el procedimiento documentado en:
- [Skill de rendimiento y medición](.opencode/skills/pdflector-rendimiento/SKILL.md)
