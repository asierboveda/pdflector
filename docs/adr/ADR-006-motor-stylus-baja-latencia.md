# ADR-006 — Motor de stylus de baja latencia: EGL/GLES2 + predicción Hermite

> **Estado:** Aceptado (Fase 0 de PLAN-PARIDAD-STYLUS-NATIVO completada, 2026-08-28).
> **Nota:** El plan referenciaba "ADR-005" como nombre de entregable; ADR-005 ya
> existía (UI Android nativa), por lo que esta decisión ocupa el ADR-006.

## Contexto

PDFLector renderiza hoy por CPU software (`ANativeWindow_lock` + memcpy +
`unlockAndPost`, 1440×2200 RGBA). La app nativa de notas de la TCL consigue
trailing lag < 0.4 cm; nuestro pipeline introduce 2-3 vsync de cola de
composición además del coste de copia de ~12 MB por frame. El plan exige
latencia punta-tinta < 15 ms, p95 de frame estable y PSS < 150 MB, medidos en la
TCL 9469X (Helio G99, Mali-G57, panel 60/120 Hz).

## Decisión

**Pipeline de presentación: EGL/GLES2 (Opción A.2) con predicción de trazo por
Hermite amortiguado (Opción B.3 corregida).**

### Datos de los spikes (TCL 9469X, Android 15, 2026-08-28)

**Spike 1 — presentación** (`crates/pdf_spike`, micro-app NativeActivity con
bucle de input idéntico y dos presenters intercambiables por tap):

| Métrica (TCL real) | SW (`lock`+copy+post) | EGL (GLES2 clear+swap) |
|---|---|---|
| Coste del present, p50 (n=65/70) | **3.75 ms** | **0.17 ms** (22×) |
| Coste del present, p95 | 7.30 ms | 0.36 ms |
| Coste del present, max | 21.26 ms | 4.11 ms |
| Cadencia en gesto, p50 | 8.52 ms | 16.66 ms (vsync exacto) |
| Cadencia en gesto, p95 (<30 ms) | 14.23 ms | **17.02 ms** |
| PSS tras 12 trazos | — | 24.7 MB |

- SW pierde: el `unlockAndPost` de un buffer de 12.7 MB domina el frame y su
  variabilidad (max 21 ms) rompe el presupuesto p95 de 16.6 ms.
- EGL presenta con `eglSwapBuffers` en < 0.5 ms y se sincroniza al vsync
  exacto (p50 = 16.66 ms con p95 = 17.02 ms: triple buffering sin drops).
- Descartes: **A.1 Front-buffer** — requiere APIs no estándar y arriesga
  tearing en la ROM TCL; EGL cubre el mismo objetivo con APIs públicas.
  **A.3 Vulkan** — complejidad de sincronización manual sin beneficio medible
  sobre GLES2 para clear/triángulos. **A.4 Slint** — capa de abstracción extra
  sin pipeline de stylus probado; mantener pdf_android nativo (coherente con
  ADR-005). **A.5 CPU optimizado** — techo físico confirmado por el spike
  (p95 14 ms solo en el present, sin contar composición).

**Spike 2 — predicción** (benchmark host-side, 4 trazos sintéticos con ruido de
digitizer σ=0.5 px a 240 Hz, mediana de error |P_pred − P_real| en px):

| Δt | Taylor med | Hermite med | Kalman(α-β) med |
|---|---|---|---|
| 8 ms | **1.0-2.3** | 0.9-2.9 | 1.4-22.6 |
| 16 ms | **1.4-7.1** | 1.7-9.5 | 2.5-46.0 |
| 24 ms | **2.1-13.9** | 3.3-18.7 | 3.4-71.7 |

- **Taylor** gana por ~10 % en mediana pero puede "latigar" en giros bruscos
  (riesgo del plan §B.1); su aceleración sin filtro explota con ruido.
- **Hermite amortiguado** (velocidad constante + aceleración clampada a
  |a| ≤ 0.02 px/ms² con paso-bajo) pierde ~10 % frente a Taylor pero garantiza
  continuidad C1 con el trazo confirmado (sin quiebros visuales) y es estable
  en todos los casos.
- **Kalman α-β** falla en giros bruscos (46-73 px): la ganancia fija reacciona
  tarde; el EKF completo queda fuera por coste de calibración (riesgo §B.2).
- El predictor de plataforma (B.4) no existe en la ROM: no hay API pública de
  predicción en MotionEvent (Android 15), descartado por diseño.

**Spike 3 — instrumentación:** `dumpsys gfxinfo` no rastrea NativeActivity (0
frames HWUI) y la ROM TCL no expone ftrace a `shell` (atrace vacío, sin root).
La medición de pipeline se realiza con telemetría propia (`spike_present` /
`spike_frame`) + fases de vsync de `dumpsys SurfaceFlinger` (app/SF phase,
60 Hz activo, 120 Hz disponible). La validación final de latencia física
punta-tinta queda para la prueba de cámara 240 fps del protocolo §4 con boli
físico (no automatizable por adb).

## Consecuencias

1. `pdf_android` migrará su presentación a EGL/GLES2: la página renderizada por
   MuPDF se sube una vez como textura (`GL_TEXTURE_2D`) y la tinta/overlays se
   dibujan como geometría vectorial; `pdf_core` permanece intacto (el motor
   sigue entregando bitmaps CPU).
2. La predicción se implementará en `crates/pdf_android/src/prediction.rs`
   (módulo puro ya validado en `pdf_spike/src/prediction.rs`) con horizonte
   Δt = 16 ms (1 vsync a 60 Hz) y dibujo del tramo efímero como capa separada
   del trazo confirmado.
3. La fase de introducción de texturas debe revalidar el presupuesto de
   memoria (page cache GPU): PSS del spike con contexto EGL = 24.7 MB;
   el render MuPDF de página (12 MB CPU) se mantiene + copia GPU ≈ +12 MB,
   dentro del margen sobre los 150 MB.
4. El panel soporta 120 Hz: la migración EGL habilita activarlo como fase
   posterior (p95 objetivo < 8.3 ms), sin cambios de arquitectura.

## Fase 2 — Presentación EGL/GLES2 en el visor (2026-08-28)

Implementación de la decisión 1: `crates/pdf_android/src/gpu.rs` (FFI EGL/GLES2 propio,
~1200 líneas, sin crates nuevas — khronos-egl descartado). El visor presenta con
`eglSwapBuffers`: página como textura perezosa (key `(page, rendered_zoom, len)`;
TexImage2D solo al cambiar tamaño, TexSubImage2D si no), tinta como TRIANGLE_STRIP
(2 vértices/punto, normal perpendicular, AA en FS) con curvas midpoint muestreadas
en página (la misma polilínea `ink_pts` que se persiste: una sola fuente de verdad),
overlays como quads de bitmaps Canvas+JNI premultiplicados (cache FIFO ≤8 por puntero
con copia owned), dark mode como uniform `uDark`. Library/Picker conservan el camino
SW con dirty rect; el ciclo Viewer↔Library hace `drop_surface`/`recreate_surface`
(el contexto EGL sobrevive). `page_frame`/`gesture_base`/`pred_layer`/`tool_dirty`
desaparecen del Reader (~1700 líneas eliminadas en draw.rs/reader.rs).

Estado: clippy verde (`-D warnings -D clippy::unwrap_used`), pdf_core intacto (70/70).
EGL/GLES2 verificado en TCL (Mali-G57 MC2, 2200x1440, PSS 52.9 MB). Medición completa del
`gl_present` en gesto ejecutada 2026-08-29: p50 4.55 / p95 10.00 ms (n=1768), p95 10.48 ms
(n=1395 segunda tanda), 0 FAIL en 3735 frames, page flips p95 11.64 ms, dark mode/suspensión/
Library↔Viewer sin regresiones, PSS 116–126 MB (trazos) / 71.4 MB (Library). Detalle completo
en `docs/benchmark-results.md`, Fase 2. Pendiente Fase 3 (boli físico): latencia punta-tinta
240 fps, test ciego, goma BTN_STYLUS2, cero-pop, presión USI 2.0.
