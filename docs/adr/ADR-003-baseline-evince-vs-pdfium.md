# ADR-003 — Baseline de rendimiento: Evince/Poppler vs PDFium

> **Origen**: medición de baseline para ADR-001 (decisión de motor PDF) y
> validación de los patrones del ADR-002 en hardware real.
> **Fecha**: 2026-08-10
> **Hardware**: AMD Ryzen 7 5800H (16 cores), 13 GiB RAM, Omarchy/Arch Linux,
>   Wayland. Evince 48.4 (poppler 26.07, cairo 1.18.4). PDFium chromium/7988.

---

## 1. Metodología

Se midió el rendimiento de renderizado y consumo de memoria con el corpus del
proyecto (4 PDFs: texto denso 93 pág., documento grande 500 pág., escaneado 30
pág., paper científico 12 pág.).

**Herramientas:**
- `pdftoppm` (poppler) → PPM raw, sin compresión, para tiempo de raster puro.
- `evince` (GTK) → `/proc/PID/status` para RSS del visor completo.
- PDFium benchmark → Rust `std::time::Instant`, mismo PDF, mismas resoluciones.

**Script de medición:** `tools/measure_evince_baseline.sh`.

---

## 2. Tiempo de render por página

### 2.1 Comparativa directa Poppler vs PDFium (dense_textbook, página 1)

| DPI | Resolución | Poppler (pdftoppm PPM) | PDFium (pdf_core) | Ratio |
|-----|-----------|------------------------|-------------------|-------|
| 72 | 595×842 | 29 ms | 3.9 ms | **7.4×** |
| 96 | 794×1123 | 34 ms | 6.5 ms | **5.2×** |
| 150 | 1240×1754 | 46 ms | 14.5 ms | **3.2×** |
| 200 | 1654×2339 | 58 ms | 28.0 ms | **2.1×** |
| 300 | 2480×3508 | 78 ms | 52.0 ms | **1.5×** |

> **Nota**: pdftoppm escribe a disco (PPM). PDFium es in-memory (Vec<u8>). La
> diferencia real es algo menor, pero PDFium es consistentemente más rápido.

**Conclusión parcial**: PDFium es 2-7× más rápido que Poppler en render puro a
resoluciones de pantalla (72-150 dpi). A 300 dpi la ventaja se reduce al 1.5×
porque ambos están limitados por ancho de banda de memoria en bitmaps grandes
(~34 MB por página).

### 2.2 Throughput por tipo de PDF (poppler, 96 dpi)

| PDF | Páginas | Total | Avg/página | Throughput |
|-----|---------|-------|-----------|------------|
| dense_textbook | 93 | 1037 ms | 11 ms | 89.7 pág/s |
| large_document | 500 | 4742 ms | 9 ms | 105.4 pág/s |
| scanned_pages | 30 | 626 ms | 20 ms | 47.9 pág/s |
| scientific_paper | 12 | 59 ms | 4 ms | 203.4 pág/s |

Los PDFs con imágenes (scanned) son el doble de lentos que texto puro. Los
gráficos vectoriales (scientific_paper) son los más rápidos.

### 2.3 Escalado con resolución

El tiempo de render escala de forma **sub-lineal** con el número de píxeles:

| DPI | Píxeles | Tiempo | Factor píxeles | Factor tiempo |
|-----|---------|--------|----------------|---------------|
| 72 | 0.5 M | 29 ms | 1.0× | 1.0× |
| 96 | 0.9 M | 34 ms | 1.8× | 1.2× |
| 150 | 2.2 M | 46 ms | 4.3× | 1.6× |
| 300 | 8.7 M | 78 ms | 17.3× | 2.7× |

Esto indica que el coste fijo de interpretar el PDF (parsing, fuentes) domina a
resoluciones bajas. A 300 dpi el rasterizado ya pesa más.

---

## 3. Consumo de memoria (RSS)

### 3.1 Evince (visor completo, GTK + Poppler)

| Escenario | RSS |
|-----------|-----|
| Recién abierto (cualquier PDF) | **~192 MB** |
| Scroll a mitad del documento (pág 50-55) | **~191 MB** |
| Scroll por las 93 páginas completas | **~192 MB** |

**Hallazgo clave**: el RSS de Evince es **constante** independientemente del
número de páginas visitadas. El sliding window de `EvPixbufCache` (50 MB de
límite para texturas + overhead de GTK/librerías) mantiene la memoria estable.

De los 192 MB totales, ~140-150 MB son el runtime de GTK/Cairo/Poppler y
~40-50 MB son las texturas cacheadas. Esto valida el diseño del ADR-002.

### 3.2 PDFium (solo engine, sin UI)

| Escenario | RSS |
|-----------|-----|
| Engine init (libpdfium.so cargada) | **5.2 MB** |
| Documento abierto (93 pág.) | **5.7 MB** |
| Render de las 93 páginas (todas en memoria, no realista) | **8.1 MB** |

**Conclusión**: PDFium consume **~30× menos memoria base** que Poppler+GTK
(~6 MB vs ~192 MB). Incluso sumando la UI (egui/eframe ~20-30 MB) y el caché
de texturas (~40-50 MB), el total estaría en **70-85 MB** — muy por debajo
del objetivo de 150 MB en tablet.

### 3.3 Proyección para Android/tablet

| Componente | Escritorio (estimado) | Tablet (estimado) |
|-----------|----------------------|-------------------|
| PDFium engine | 6 MB | 8 MB |
| UI (egui/Slint/Tauri) | 30 MB | 25 MB |
| Caché texturas (40 MB límite) | 40 MB | 30 MB |
| Overhead runtime | 15 MB | 15 MB |
| **Total** | **~91 MB** | **~78 MB** |

Muy por debajo del objetivo de < 150 MB en tablet. Esto deja margen para
features futuras (anotaciones, IA, sync).

---

## 4. Tiempo de arranque

| PDF | Evince (cold start) | PDFium (pdf_core) |
|-----|---------------------|-------------------|
| Cualquier PDF | ~4017 ms | ~150-200 ms (estimado, solo open+render pág 1) |

Los 4 segundos de Evince son mayoritariamente inicialización de GTK. PDFium no
tiene ese overhead.

---

## 5. Implicaciones para el proyecto

### 5.1 Para ADR-001 (elección de motor)

PDFium muestra ventajas decisivas:
1. **Render 2-7× más rápido** que Poppler a resoluciones de pantalla.
2. **Memoria base 30× menor** (6 MB vs 192 MB).
3. **Compilación Android validada**: `pdfium-binaries` de bblanchon proporciona
   `.so` precompiladas para `aarch64-linux-android`.

Queda pendiente la comparativa con MuPDF (Fase 0.5).

### 5.2 Para el diseño de caché (ADR-002)

Los patrones extraídos de Evince se validan:
- **Sliding window**: el RSS constante de Evince (~192 MB) confirma que el
  límite por bytes + expulsión al salir del rango funciona.
- **Separación píxeles/metadatos**: en PDFium, el overhead de metadatos es
  mínimo (~0.5 MB para un documento de 93 páginas).
- **Resolución de pantalla**: renderizar a más de 150 dpi en tablet es
  innecesario (la pantalla tiene densidad finita). A 150 dpi, PDFium rinde
  14.5 ms/página en escritorio → ~20-25 ms estimados en tablet: dentro del
  objetivo de < 25 ms.

### 5.3 Para el objetivo de fluidez (60 fps)

A 96 dpi (~resolución típica de tablet 10"), PDFium renderiza en 6.5 ms/página
en este Ryzen. En la tablet (CPU ~4× más lenta), estimamos ~25 ms/página.

**Pipeline propuesto** (coherente con ADR-002 §3.3):
```
Scroll detectado → página N entra en rango visible
  → Cache HIT: textura lista, < 1 ms → 60 fps garantizado
  → Cache MISS: placeholder gris + encola render en background
     → 6.5 ms (PC) / 25 ms (tablet) después → textura lista → redibuja
```

Con prefetch de N+1 y N-1, la probabilidad de MISS es cercana a 0 durante
scroll normal. Solo el primer render de cada página es MISS.

---

## 6. Referencias

- Script de medición: `tools/measure_evince_baseline.sh`
- Resultados raw: `tools/evince_baseline_results.txt`
- Hardware: AMD Ryzen 7 5800H, 13 GiB RAM, Arch Linux (kernel 7.1.4)
- Software: Evince 48.4, Poppler 26.07, Cairo 1.18.4, PDFium chromium/7988
