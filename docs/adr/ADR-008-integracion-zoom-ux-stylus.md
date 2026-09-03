# ADR-008: Integración zoom + UI/UX sobre el stack stylus (2026-09-03)

**Estado**: Aceptado — rama `main` (`72068da` merge ui_ux, `32989c1` port worker).

## Contexto

Tres ramas divergían de `2804c8b`: `ui_ux` (tokens RICOUI + shell egui),
`mejora_zoom` (pipeline GPU/zoom F0–F3) y `main` (stylus nativo Phases 0–3 +
W1–W3, donde acabó por error el trabajo de `mejora-lapiz`, rama vacía).
`mejora_zoom` y `main` reescribían `reader.rs` en direcciones opuestas.

## Decisión

1. **F2 (`gles.rs`, backend GLES3) descartado: SUPERSEDED.** `main` ya
   presenta por GPU (`gpu.rs`, ADR-006) con los mismos objetivos (textura
   perezosa, tinta vectorial, surface-recreate sin perder contexto).
   Dos backends compitiendo = regresión garantizada.
2. **F3.3 (display lists en `pdf_core`) integrado.** Sin `unwrap`, sin dep
   UI, +3 tests, 1.64–1.81× a 2× en desktop; en TCL sin regresión
   (sweep `pdf_bench` large/dense ±4 %, ruido).
3. **F3.1 (worker persistente) + F3.2 (debounce 350 ms) portados** sobre el
   `launch_render` de `main` (hilo-por-zoom = tormenta de hilos al navegar).
4. **F1 (GL desktop) descartado.** `pdf_app` es prototipo sin tests; el
   shell RICOUI (`ui_ux`, mergeado) ya redibuja esa zona. Revisit si el
   prototipo se estabiliza.
5. **Tokens de diseño en `pdf_core::theme`.** Datos agnósticos (`const`,
   sin dependencia UI) consumibles por egui y Slint: se acepta la
   excepción aparente a "core sin UI" porque no hay lógica de
   presentación, solo paleta.

## Consecuencias

- Ramas `ui_ux` (mergeada), `mejora_zoom` (porte/supersede) y
  `mejora-lapiz` (vacía) eliminadas tras esta integración.
- PSS TCL tras abrir `large_document.pdf` (500 págs): 105 MB vs 100 MB
  base (+5 MB: documento propio del worker + display lists retenidas).
- Deuda que sigue roja: `clippy --all-targets` (unwraps en tests/benches).
