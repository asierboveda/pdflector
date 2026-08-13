# ADR-001: Motor de renderizado PDF — MuPDF

**Estado**: Aceptado — 2026-08-05
**Fase**: 0.5 (benchmark)

## Contexto

Evaluación de PDFium (Chromium, Apache-2.0) vs MuPDF (Artifex, AGPL-3.0) para
un lector de PDFs en tablet Android de 200 € con lápiz, donde las prioridades
1 y 2 son **fluidez (60-120 fps)** y **RAM mínima (< 150 MB objetivo)**.

Benchmark sobre corpus (4 PDFs A4, AMD Ryzen 7 5800H, release build, Rust 1.97.1):

| PDF (págs) | Motor | open | render 1x | render 2x | RSS pico |
|---|---|---|---|---|---|
| dense (93)  | PDFium | 0,17 ms | 9,69 ms | 35,34 ms | 32 520 KB |
| dense (93)  | MuPDF  | 0,11 ms | 3,53 ms |  8,51 ms | 25 572 KB |
| scanned (30)| PDFium | 0,09 ms | 20,01 ms| 66,20 ms | 32 520 KB |
| scanned (30)| MuPDF  | 0,07 ms | 8,93 ms | 35,38 ms | 25 572 KB |
| paper (12)  | PDFium | 0,08 ms | 1,72 ms | 26,44 ms | 32 520 KB |
| paper (12)  | MuPDF  | 0,07 ms | 2,18 ms |  6,95 ms | 25 572 KB |
| large (500) | PDFium | 0,21 ms | 6,86 ms | 35,10 ms | 32 520 KB |
| large (500) | MuPDF  | 0,09 ms | 3,98 ms | 10,19 ms | 25 572 KB |

Detalle en `docs/benchmark-results.md`. Build Android (aarch64-linux-android,
NDK r28, API 24): PDFium 1 comando / 22 s; MuPDF 17 s + 1 env var
(`BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=...` para bindgen).
Ambos cross-compilan; fricción baja en ambos casos.

## Decisión

**MuPDF** como motor único y por defecto.

## Justificación

- **Prioridad 1 (fluir)**: MuPDF es 2,7-4× más rápido en render — más fácil
  sostener 120 fps con caché LRU más pequeña.
- **Prioridad 2 (RAM)**: MuPDF consume 21% menos RSS de pico (25,6 MB vs 32,5
  MB base, medido en harness sin caché). Más margen para el objetivo de 150 MB
  en la tablet con PDFs grandes.
- **Android**: MuPDF compila limpio para aarch64 (fricción de 1 env var,
  documentada y replicable vía `tools/`).
- **Licencia**: MuPDF es AGPL-3.0 → el proyecto aprueba licenciarse bajo
  **AGPL-3.0** (LICENSE añadido). AGPL es compatible con la decisión 3 del
  plan ("código público en GitHub"): publicar el código en GitHub cumple el
  requisito de ofrecer la fuente, y, como toda licencia copyleft, obliga a
  quien modifique el proyecto y no abra su trabajo a que abra el suyo (la
  cláusula de red extiende ese deber incluso a uso en servidores). Para un
  proyecto personal de aprendizaje esto es un **plus**: garantiza que el
  código y sus derivados se mantienen abiertos, y la fricción de licencia es
  cero para un proyecto público y gratuito. Finalmente, el rendimiento
  justifica la decisión: la ganancia de velocidad (2,7-4×) y de RAM (-21%) es
  medible y ataca directamente las prioridades 1 y 2 del proyecto.

## Consecuencias

- Repo licenciado **AGPL-3.0** (ver LICENSE). README y AGENTS.md actualizados.
- Backend PDFium **eliminado** (crates/pdf_core/src/engine/pdfium.rs,
  tests/basic.rs, dep pdfium-render, feature `pdfium`). El linker de Android
  en `.cargo/config.toml` ya no es imprescindible en este punto, pero se
  conserva **activo** (linker `aarch64-linux-android24-clang` + comentario de
  la env var `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`); será útil para
  Fase 1 y Fase 6.
- `vendor/pdfium/` y los scripts `tools/fetch_pdfium*.sh` quedaron
  obsoletos: los scripts se eliminaron con el backend (commit `e56a818`,
  2026-08-12) y `/vendor` está gitignored (ausente en el repo).
- Decisiones pendientes (§6 AGENTS.md) actualizadas: motor y licencia resueltas.
