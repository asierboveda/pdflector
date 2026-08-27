# ADR-005 — UI Android final: stack nativo propio (`pdf_android`), en lugar de Slint

> **Supercede a**: ADR-004 (2026-08-13), que quedó **Superseded**.
> **Fecha**: 2026-08-23
> **Origen**: test de decisión pedido por el autor (2026-08-23): medir el stack
> nativo en hardware real y decidir con datos si consolidar `pdf_android` o
> portar a Slint. Prioridades del proyecto: 1) fluidez, 2) RAM (AGENTS.md §2).

## 1. Contexto

- ADR-004 eligió **Slint** como UI final cuando `pdf_android` era solo un harness
  de la Fase 1 (reproducía un PDF al `ANativeWindow`). Desde entonces la app
  nativa se convirtió en el producto real: biblioteca premium (2026-08-18),
  visor de una hoja, pinch-zoom, selección de texto, y recién la Fase 3.5 de
  anotaciones (boli + resaltador con detección de texto, barra de herramientas).
- `pdf_android` son ~12 600 líneas (draw.rs 3 746, reader.rs 4 336) de
  NativeActivity + JNI + render propio a `ANativeWindow`, **sin una línea de
  Slint**.

## 2. Test de decisión (2026-08-23, hardware real)

Hardware: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, 1440×2200).
Build release con NDK r28, APK 6,4 MB. Mediciones tomadas el 2026-08-23:

### 2.1 Fluidez (prioridad 1)

| Métrica | Native (medido) | Objetivo | Resultado |
|---|---|---|---|
| Coste blit/compose frame completo (2000×1200, espejo de draw.rs, desktop) | **1,29 ms** (compose) + 0,89 ms (blit) | < 16,6 ms (60 fps) | ✅ margen ~7-12× |
| fill_buffer 2k1k (desktop) | 180 µs | — | ✅ |
| Render página 1x en la tablet (harness release, hoy) | dense 13,9 / paper 11,0 / large 14,6 ms | < 25 ms | ✅ 3/4 (scanned 30,6 ms = worst case raster conocido) |
| PEAK_RSS core (MuPDF + caché, harness release) | **31,8 MB** | < 150 MB | ✅ margen ~4,7× |

El camino de pintado nativo (blit por filas, LUT, banda cacheada — auditoría
2026-08-22, ganancias 31-89 %) usa **1,3-2,2 ms por frame**: 60 fps se cumplen
con una holgura de un orden de magnitud antes de tocar siquiera el presupuesto
del render de página.

### 2.2 RAM (prioridad 2) — el dato que faltaba

| Métrica | Valor medido | Nota |
|---|---|---|
| App real release, biblioteca (PSS / RSS, dumpsys) | **PSS 169 MB / RSS 277 MB** | Supera el objetivo nominal 150 MB |
| Desglose PSS | Native Heap 122,8 MB · Graphics (EGL/ANativeWindow) 37,1 MB · Code 3,9 · Java 2,8 | Los buffers de ventana (~38 MB en 1440×2200 ×3) son fijos de cualquier stack GLES |
| Core sin UI | 31,8 MB | Caché LRU pdf_android limitada a 48 MiB / 5 entradas |

**Lectura honesta**: el objetivo de 150 MB de AGENTS.md §8 se pensó sobre el
core (RSS del harness); el producto con UI supera esa cifra. Pero el
contribuyente dominante (122 MB de native heap) es **configuración del código
nativo actual** (caché 48 MiB + bitmaps de biblioteca cacheados + portadas +
arenas del allocator), no una propiedad del framework: se ataca optimizando
(método: reducir presupuestos, medir PSS antes/después), no portando.

### 2.3 Lo que Slint aportaría (vs. sus costes)

| Criterio | Native `pdf_android` | Slint (ADR-004) |
|---|---|---|
| Latencia táctil/lápiz | Directa, verificada; boli+resaltador Fase 3.5 ya integrados | Directa (GLES), input verificado; **lápiz real sin validar** (§7.3) |
| Fluidez | Medida: 1,3-2,2 ms/frame blit + render < 25 ms tablet | Sin medir en visor PDF; riesgo **no-repaint tras cambios de propiedad** en Android (§7.2, fix #12688 **sin release**) — crítico para un visor que redibuja por scroll/tap |
| RAM | 169 MB PSS con culpables identificados y optimizables | Proyección: demo 62 MB PSS (lista, sin PDF) + core 32 MB + buffers ventana ≈ 130-160 MB → **no resuelve el problema** |
| Integración `pdf_core` | Directa | Directa |
| Reescribir UI probada | 0 | ~12 600 líneas |
| Licencia | AGPL (repo) | GPLv3/comercial (compatible hoy, riesgo futuro) |

## 3. Decisión

**Consolidar `pdf_android` (stack nativo propio) como UI final Android.**
No se porta a Slint. ADR-004 queda **Superseded**.

Justificación (por prioridades): (1) fluidez — el nativo ya cumple con margen
de un orden de magnitud y Slint añade un riesgo de no-repaint documentado sin
fix en ninguna release; (2) RAM — ambos stacks comparten el presupuesto
impuesto por el pipeline de ventana + bitmaps; el exceso actual se corrige
optimizando el código nativo, no cambiando de framework; (3) coste — portar
reescribe 12 600 líneas probadas en la tablet contra las prioridades del
proyecto (cambio mínimo, aprender, no rehacer lo que funciona).

## 4. Consecuencias

- **AGENTS.md**: eliminar "Slint vs Tauri" de decisiones pendientes (§6);
  `pdf_android` entra en la arquitectura (§4) y el stack (§5); definir "hecho"
  con paso Android y métrica **PSS** (§8).
- **PLAN.md Fase 6**: redefinir como "consolidación + validación del stack
  nativo" (lápiz físico, frame time p95 en scroll, test de estrés 200 trazos,
  semana de uso), no como spike Slint/Tauri.
- **minSdk 24 → 26**: NO aplica (solo lo exigía el backend de Slint). Se queda
  en 24.
- **Trabajo de optimización derivado (pendiente, priorizado)**:
  1. Bajar el native heap de la app real (caché 48 MiB, tamaños de
     lib_header/lib_band, portadas, arenas) hasta PSS < 150 MB. Medición:
     `dumpsys meminfo` PSS antes/después, mismo flujo (biblioteca con 4 PDFs).
  2. Frame time p95 del visor real en scroll (overlay de debug en pdf_android).
  3. Verificar metric objetivo: expresar el objetivo de RAM como **PSS**
     (métrica de coste real del sistema), renumerado en AGENTS.md §8 / PLAN §4.
- Registrar en `memory.md` (2026-08-23).

## 5. Referencias

- ADR-004 (Superseded), ADR-001 (MuPDF), ADR-002/003 (patrones Evince).
- `docs/benchmark-results.md` (auditoría 2026-08-22: blit 1,29 ms/frame;
  sweep tablet 2026-08-23 de esta decisión).
- `memory.md` (2026-08-18 biblioteca premium; 2026-08-22 Fase 3.5).