# Baseline de rendimiento — Evince (poppler) en escritorio

> **Fecha**: 2026-08-10 · **Hardware**: AMD Ryzen 7 5800H (8C/16T, hasta 4,47 GHz),
> 13 GiB RAM, Linux 7.1.4-arch1-1 (Wayland/Hyprland) · **Software**: Evince 48.4,
> Poppler 26.07.0 · **Corpus**: `corpus/large_document.pdf` (500 páginas A4,
> 543 kB, texto).
> **Medición reproducible**: `tools/bench-evince/bench_evince.sh` (usa `pdftoppm`,
> que comparte exactamente el pipeline de render de Evince: poppler + cairo,
> single-thread, página completa a la resolución pedida).

## 1. Resultados

### Render por página (poppler single-thread, página completa)

| Escala | Píxeles/página (A4) | Total 500 pág (3 reps) | ms/página | max RSS proceso |
|---|---|---|---|---|
| 72 dpi (1x) | 595×842 | 36,72 / 36,84 / 37,84 s | **73,6** | 22,5 MB |
| 144 dpi (2x) | 1190×1684 | 164,36 / 162,06 s | **326** | 28,0 MB |

### Apertura + render de la primera página (incluye carga del documento)

| Escala | Tiempo (3 reps) |
|---|---|
| 144 dpi (2x) | 0,42 / 0,36 / 0,35 s |
| 216 dpi (3x) | 0,60 / 0,60 / 0,60 s |

### RSS del visor GUI (Evince 48.4 abierto sobre el PDF de 500 páginas)

| Situación | RSS |
|---|---|
| Ventana abierta, página 1, tras 8 s (frío) | 197 920 kB |
| Reabierto con caché de disco caliente, 8 s | 197 952 kB |

## 2. Lectura de los datos

1. **El render es el coste dominante y cuadra con la ley de píxeles**:
   cuadruplicar píxeles (1x→2x) cuadruplica el tiempo (73,6→326 ms/página).
   Implicación directa: a resolución "2x" (típica en pantallas HiDPI), un scroll
   de página sin caché costaría **326 ms/frame** → 3 fps. La caché de texturas
   + render asíncrono no son una opción, son obligatorios (y Evince los tiene).
2. **Evince renderiza a la resolución pedida y nunca retiene todas las
   páginas**: max RSS del proceso de render es ~28 MB (renderiza y suelta).
3. **El RSS del visor GUI (~198 MB) supera nuestro presupuesto objetivo**
   (<150 MB en tablet). Evince no es un modelo de frugalidad: su caché por
   defecto es 50 MB + GTK/GL + document model. Sirve como tope superior a
   superar, no como objetivo.
4. **Apertura rápida**: página 1 visible en ~0,36 s con poppler puro (sin
   thumbnails). Cualquier motor de PDFLector (PDFium/MuPDF) se comparó
   contra estos números en la Fase 0.5 (ver `docs/benchmark-results.md`).

## 3. Cómo se usa esto en el proyecto

- **Fase 0.5 (benchmark PDFium vs MuPDF)**: misma máquina, mismo corpus, mismo
  método (single-thread, página completa, mismas escalas 72/144/216 dpi) →
  comparación directa de ms/página y RSS contra estos valores de poppler.
  Criterio adicional del ADR-001: si PDFium o MuPDF iguala o supera estos
  números, el motor es viable.
- **Fase 1 (lectura fluida)**: estos datos fijan el presupuesto de caché en
  escritorio: a 2x una página pesa ~8 MB RGBA (1190×1684×4). Con 50 MB de caché
  (el default de Evince) caben ~6 páginas; con nuestra ventana deslizante
  (viewport + preload) el consumo se mantiene acotado sin importar la
  profundidad del documento.
- **Referencia de fluidez**: 326 ms/página @2x ⇒ el objetivo de 16,6 ms/frame
  en scroll solo es alcanzable si el frame de scroll nunca dispara un render
  (solo composición de texturas). Es exactamente el patrón de Evince analizado
  en `docs/research/evince-architecture.md`.

## 4. Limitaciones de esta medición

- `pdftoppm` incluye el encode PNG por página (~coste pequeño a 72/144 dpi,
  despreciable frente a 73/326 ms). No hay acceso al render "desnudo" de
  poppler sin compilar un harness.
- El RSS GUI de Evince incluye GTK4, GL y el resto del runtime: es el dato de
  "app completa", comparable con `dumpsys meminfo` del apk en Fase 1, no con el
  RSS del motor solo.
- Hardware de escritorio; la tablet (TCL NXTPaper 11 Plus) se midió en Fase 1
  (2026-08-12) con `pdf_bench` cross-compilado vía `adb` (RSS pico 26,7 MB; ver
  `docs/benchmark-results.md`).
