# Resultados del benchmark — Fase 0.5 (2026-08-05)

Hardware: AMD Ryzen 7 5800H, 16 hilos. Rust 1.97.1, release build
(`cargo run --release -p pdf_bench`).

## Tabla comparativa

| PDF (páginas) | Motor | open (ms) | render 1x (ms) | render 2x (ms) | RSS pico (KB) |
|---|---|---|---|---|---|
| dense (93) | PDFium | 0.17 | 9.69 | 35.34 | 32520 |
| dense (93) | MuPDF | 0.11 | 3.53 | 8.51 | 25572 |
| scanned (30) | PDFium | 0.09 | 20.01 | 66.20 | 32520 |
| scanned (30) | MuPDF | 0.07 | 8.93 | 35.38 | 25572 |
| paper (12) | PDFium | 0.08 | 1.72 | 26.44 | 32520 |
| paper (12) | MuPDF | 0.07 | 2.18 | 6.95 | 25572 |
| large (500) | PDFium | 0.21 | 6.86 | 35.10 | 32520 |
| large (500) | MuPDF | 0.09 | 3.98 | 10.19 | 25572 |

## Notas
- Métricas: mediana de 3 runs (páginas 0/mitad/última) para render; open una vez.
- RSS pico: VmHWM de /proc/self/status (cada motor en proceso separado).
- Build: PDFium host ~0.5 s (lib precompilada `vendor/pdfium/lib/libpdfium.so`,
  solo compilación Rust), MuPDF host 29.96 s la 1ª vez (C de `mupdf-sys` 0.8.0).
- Android cross (ver memory): PDFium 1 comando; MuPDF pendiente de C2.
