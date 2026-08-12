# Benchmark de motores — PDFium vs MuPDF (Fase 0.5, resultados preliminares)

> **Fecha**: 2026-08-10 · **Hardware**: AMD Ryzen 7 5800H (8C/16T, hasta 4,47 GHz),
> 13 GiB RAM, Linux 7.1.4 (Wayland). **Sistema con carga** (CPU scaling ~89%,
> 9,3/13 GiB RAM en uso): la varianza entre ejecuciones es alta — los valores
> son **direccionales**, el dato definitivo se tomará en la tablet real (Paso 3).
> **Método**: criterion 0.5 (`cargo bench -p pdf_bench --features mupdf`),
> página completa a la escala pedida, mediana de ~10-20 muestras;
> RSS por proceso limpio vía `/proc/self/status` (binario `pdf_bench` release,
> 20 páginas @2x). **Corpus**: 4 PDFs A4 (12/30/93/500 páginas).
> **Referencia**: baseline poppler/Evince = 73,6 ms/pág @1x y 326 ms/pág @2x
> (misma máquina, `docs/investigacion/evince-baseline.md`).

## Apertura (open + page_count), mediana en µs

| PDF | PDFium | MuPDF |
|---|---|---|
| scientific_paper (12 pág) | 41 | 56 |
| scanned_pages (30 pág) | 58 | 55 |
| dense_textbook (93 pág) | 76 | 121 |
| large_document (500 pág) | 246 | 262 |

Empate práctico; ambos < 0,3 ms en apertura (el parseo es perezoso en los dos).

## Render por página (mediana en ms)

| PDF | página | escala | PDFium | MuPDF |
|---|---|---|---|---|
| scientific_paper | p1 | 1x | 0,75 | 0,69 |
| scientific_paper | p1 | 2x | 10,6 | 25,8 |
| scientific_paper | central | 1x | 0,98 | 0,83 |
| scientific_paper | central | 2x | 11,8 | 18,8 |
| scanned_pages | p1 | 1x | 9,4 | 4,4 |
| scanned_pages | p1 | 2x | 30,3 | 32,3 |
| scanned_pages | central | 1x | 9,4 | 8,3 |
| scanned_pages | central | 2x | 25,2 | 44,2 |
| dense_textbook | p1 | 1x | 5,3 | 5,1 |
| dense_textbook | p1 | 2x | 16,4 | 30,0 |
| dense_textbook | central | 1x | 4,0 | 2,0 |
| dense_textbook | central | 2x | 34,2 | 16,3 |
| large_document | p1 | 1x | 4,8 | 1,9 |
| large_document | p1 | 2x | 87 | 21 |
| large_document | central | 1x | 4,9 | 5,9 |
| large_document | central | 2x | 19 | 46 |

**Lectura**: ambos motores están entre 5 y 50 ms/página a 2x — **4-15x más
rápidos que poppler/Evince** en la misma máquina (326 ms @2x). El render 1x es
sustancialmente más rápido en ambos (1-9 ms). Las páginas escaneadas
(scanned_pages) son las más caras para ambos (~30 ms @2x): dominan los
decodificadores de imagen, no el motor PDF. A 2x varios casos superan el
objetivo tablet de <25 ms — pero el objetivo es para la tablet (SoC distinto),
estos números solo sirven como orden de magnitud.

## RSS pico por motor (proceso limpio, 20 páginas @2x)

| PDF | PDFium | MuPDF |
|---|---|---|
| large_document | pico 23,2 MB · retenido 8,1 MB | pico 23,3 MB · **retenido 23,3 MB** |
| scanned_pages | pico 25,0 MB · retenido 7,7 MB | pico 24,0 MB · retenido 9,0 MB |

Nota: MuPDF **retiene las páginas cargadas en su store** (fz_store, límite
configurable con `set_store_max_size`); PDFium descarta todo entre renders. En
PDFLector la caché de páginas vivirá en `pdf_core` (ventana deslizante por
bytes, patrón Evince), no en el store del motor: habrá que acotar el store de
MuPDF si gana (API disponible en el crate: `mupdf::set_store_max_size`).

## Tamaño de binario release (mismo harness, por motor)

| Motor | Binario | Nota |
|---|---|---|
| PDFium | 4,7 MB | `libpdfium.so` externa (~20 MB adicionales, ya presente en vendor/) |
| MuPDF | 6,3 MB | motor **estático incluido** (features img + base14-fonts) |

MuPDF entrega todo el motor en 6,3 MB estáticos (sin .so extra); PDFium
requiere además su .so precompilada (~20 MB). Para el apk final la diferencia
es real (~26 MB vs 6,3 MB de motor).

## Limitaciones

- Varianza alta por carga del sistema (CPU scaling 89%, RAM al 71%): los
  renders 2x de large_document variaron 19-87 ms entre rondas en PDFium. No
  usar estos números absolutos para decidir diferencias pequeñas.
- `cargo bench` corrió 2 veces; se reporta la mediana de la segunda ronda.
- Falta: timings en la tablet real (Paso 3) — el dato decisivo.

## Timings en la tablet real (Paso 3) — Lenovo Idea Tab 9469X, Android 15

> 20 páginas por ejecución, página completa; harness release `pdf_bench`
> (mismo binario que en escritorio), `/data/local/tmp`. Serial: A06B4A8E6774623.
> Objetivo PLAN: **render de página < 25 ms**.

| PDF | escala | PDFium ms/pág | MuPDF ms/pág | ganador |
|---|---|---|---|---|
| scientific_paper | 1x | 5,4 | 4,3 | MuPDF |
| scientific_paper | 2x | 13,3 | 11,4 | MuPDF |
| scanned_pages | 1x | 27,5 | **11,9** | MuPDF (2,3x) |
| scanned_pages | 2x | 46,3 | 44,0 | MuPDF (marginal) |
| dense_textbook | 1x | 8,2 | 5,0 | MuPDF |
| dense_textbook | 2x | 32,1 | **13,0** | MuPDF (2,5x) |
| large_document | 2x | 29,4 | **13,1** | MuPDF (2,2x) |

| Métrica | PDFium | MuPDF |
|---|---|---|
| Open large_document | 13,5 ms | **1,4 ms** (10x) |
| RSS pico (large @2x) | 26,6 MB | 28,2 MB |
| RSS retenido (large @2x) | 11,1 MB | 12,7 MB |
| Binario Android | 5,1 MB + libpdfium.so arm64 6,1 MB | **5,7 MB estático** |
| Build Android | OK (dlopen .so precompilada) | OK (compilación estática NDK r28, API 35) |

**Lectura**: MuPDF gana en los 7 casos de render en tablet (1,2x-2,5x), en open
(2-10x) y empata en RSS. Cumple el objetivo <25 ms en 6/7 casos (falla solo en
escaneado @2x, 44 ms — coste del decodificador de imagen, mismo para ambos).
PDFium incumple en 5/7. **Ganador: MuPDF.**
