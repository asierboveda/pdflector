# ADR-001 — Motor PDF: MuPDF (y licencia AGPL-3.0 del repo)

- **Estado**: Aceptado (2026-08-12)
- **Decisión de**: Fase 0.5 (docs/PLAN.md) — benchmark comparativo con datos reales
- **Fecha del benchmark**: 2026-08-10 (escritorio) y 2026-08-12 (tablet)

## Contexto

PDFLector necesita un motor de render de PDF para el lector de la tablet.
Candidatos tras el análisis de Evince (docs/investigacion/evince-rendimiento.md):
**PDFium** (Chromium, BSD-3-Clause, vía crate `pdfium-render` + lib precompilada)
y **MuPDF** (Artifex, AGPL-3.0, vía crate `mupdf` 0.8 / MuPDF 1.27.2, que vende y
compila el motor desde fuente). Criterios de decisión: velocidad de render,
consumo de RAM, facilidad de build Android y licencia resultante del repo.

## Decisión

**MuPDF es el motor de render de PDFLector.** Consecuencia: **el repositorio se
licencia AGPL-3.0** (decisión 3 del PLAN, confirmada por el autor). Se elimina
el backend PDFium sin dejar deuda técnica.

## Datos que sustentan la decisión

Método completo y reproducible: `cargo bench -p pdf_bench` (criterion) +
harness `pdf_bench` (RSS vía /proc/self/status). Corpus: 4 PDFs A4
(12/30/93/500 páginas). Escritorio: Ryzen 7 5800H. Tablet: **Lenovo Idea Tab
9469X (Android 15)**, serial A06B4A8E6774623.

### Render por página en tablet (ms, objetivo <25 ms)

| PDF @ escala | PDFium | MuPDF |
|---|---|---|
| scientific @1x | 5,4 | **4,3** |
| scientific @2x | 13,3 | **11,4** |
| scanned @1x | 27,5 | **11,9** |
| scanned @2x | 46,3 | 44,0 |
| dense @1x | 8,2 | **5,0** |
| dense @2x | 32,1 | **13,0** |
| large @2x | 29,4 | **13,1** |

MuPDF gana **7/7** casos (1,2-2,5x). Cumple <25 ms en 6/7; el único fallo
(escaneado @2x, 44 ms) es coste del decodificador de imagen, idéntico en ambos.

### Otras métricas (tablet)

| Métrica | PDFium | MuPDF |
|---|---|---|
| Apertura (large, 500 pág) | 13,5 ms | **1,4 ms** |
| RSS pico / retenido | 26,6 / 11,1 MB | 28,2 / 12,7 MB |
| Binario | 5,1 MB + .so 6,1 MB | **5,7 MB estático** |
| Build Android | OK (dlopen .so precompilada arm64) | OK (NDK r28, API 35, estático) |

Baseline de referencia (mismo corpus, escritorio): poppler/Evince 73,6 ms/pág
@1x y 326 ms/pág @2x — ambos motores lo superan 4-15x
(docs/investigacion/evince-baseline.md).

## Alternativas consideradas

1. **PDFium** — segundo mejor en todos los renders, 10x más lento en apertura,
   requiere `libpdfium.so` arm64 externa (~6 MB) y descartaría el AGPL del
   repo. Ventaja: licencia permisiva. Rechazado por rendimiento.
2. **Bindgen propio sobre MuPDF** — descartado: el crate `mupdf` 0.8 (MuPDF
   1.27.2, mantenido activamente, mismo autor que el antiguo pdfium-render)
   compila el motor vendido desde fuente y expone API safe suficiente
   (Document::open, load_page, to_pixmap) para `trait RenderEngine`.
3. **Poppler** (motor de Evince) — descartado por licencia GPL-2+ y porque el
   baseline demostró que es 4-15x más lento en esta máquina.

## Consecuencias

### Positivas
- Motor más rápido y con apertura casi instantánea (1,4 ms → carga perezosa
  natural para PDFs de 500+ páginas).
- Build estático: el apk no necesita .so externas del motor.
- MuPDF es multi-thread-safe por diseño (contexto por hilo) — sin mutex global
  como requería PDFium (evidencia en `engine/mupdf.rs`).

### Negativas / pendientes
- **Licencia AGPL-3.0** para el repo (README ya avisaba de la pendencia).
  Implica: cualquier distribución con red (no es el caso — app local) debe
  ofrecer el código fuente; requiere añadir `LICENSE` al repo.
- **Store interno de MuPDF retiene páginas cargadas** (RSS retenido 12,7 MB
  tras 20 renders vs 11,1 de PDFium): al llegar la caché de `pdf_core` (Fase 1)
  habrá que acotarlo con `mupdf::set_store_max_size` para que el presupuesto
  de RAM (<150 MB) lo gobierne `pdf_core`, no el motor.
- Regenerar `Cargo.lock` y borrar el binario de PDFium del vendor (hecho).

## Referencias

- Benchmark completo: docs/investigacion/benchmark-motores.md
- Baseline Evince/poppler: docs/investigacion/evince-baseline.md
- Análisis de patrones de Evince: docs/investigacion/evince-rendimiento.md
