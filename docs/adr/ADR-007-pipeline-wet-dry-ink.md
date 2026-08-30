# ADR-007: Pipeline de Tinta en Dos Capas (Wet/Dry) con Presentación Independiente

> **Estado:** Aceptado (propuesto). **Fecha:** 2026-08-30.
> **Contexto de decisión:** `PLAN-PARIDAD-STYLUS-NATIVO` + `ADR-006` (motor EGL/GLES2). Esta decisión **revisa la Fase 2** porque, pese a tener GPU + ink-stroke-modeler + 120 Hz, la experiencia física sigue lejos de la app nativa.
> **Supersede (parcialmente):** ADR-006 en su apartado de *present* por `eglSwapBuffers` único. No invalida la elección EGL/GLES2 ni el modeler, solo **cómo** se presenta la tinta en vuelo.

---

## 1. Contexto y estado real del pipeline (anclado en código)

Estado actual en `crates/pdf_android` (commit `00592ff`), verificado leyendo el código:

| Componente | Fichero | Comportamiento real |
|---|---|---|
| Modelador de trazo | `src/ink/*` + `src/prediction.rs` | `input_filter → spring_mass (ζ=1.0) → kalman_predictor (25–30 ms) → stroke_end`. Produce `confirmed_pt` (suavizado) y `predicted_pt` (Kalman). Correcto. |
| Ingesta | `src/input.rs` | 240 Hz history + presión USI + timestamps NDK. Correcto. |
| Geometría en vivo | `src/reader.rs:4079-4143` | `update_tool_gesture` alimenta el modeler, empuja el `confirmed_pt` a `points`, genera la polilínea midpoint en `ink_pts` y guarda `predicted_pt`. Correcto. |
| **Present** | `src/gpu.rs:present_viewer` | **UN solo surface EGL, `eglSwapInterval(dpy, 1)` (vsync-locked), y recompone el frame ENTERO cada frame** (clear → textura de página → TODOS los highlights → TODOS los trazos guardados → tinta en curso → segmento predicho → TODOS los overlays) y presenta con `eglSwapBuffers`. |
| Bucle principal | `src/lib.rs:641-745` | `poll_events(Some(8ms))` → `if reader.take_repaint() { reader.blit() }`. Cada present va a la cadencia del bucle, no a la cadencia de entrada. |
| 120 Hz | `src/jni.rs:enable_120hz` | `preferredRefreshRate = 120.0f` + `preferredDisplayModeId = 1`. Activado. |

### Métricas medidas (ADR-006 / benchmark-results.md)
- Present GPU: **p50 0.17 ms** (eglSwapBuffers), pero **el frame completo se recompone cada vez**. La **cadencia** queda limitada por el bucle (8 ms poll → hasta 1 frame de espera tras cada muestra).
- Modeler: **0.525 µs/llamada**. No es el cuello.
- Causa de la latencia **no es** el GPU (0.17 ms) ni el modeler (0.5 µs): **es la re-composición del frame entero y su acoplamiento a la cadencia del bucle + el vsync**.

---

## 2. Diagnóstico: por qué siguen los dos síntomas

### 2.1 Latencia percibida ~30–50 ms
Aunque la predicción Kalman estira la tinta 25–30 ms hacia delante, **la tinta en vuelo se dibuja dentro del MISMO frame completo que se presenta por vsync**. Por tanto:

```
muestra (input) → poll(8ms) → update_tool_gesture → mark_repaint
   └── espera a que el bucle llegue a `take_repaint` (hasta 1 tick)
       └── present_viewer recompone TODO el frame (page + trazos + overlays)
           └── eglSwapBuffers espera el vsync (8.33 ms a 120 Hz)
```

La tinta **no puede adelantarse a ese límite** porque es *una arista más* del frame completo. La predicción solo disimula el gap, no lo elimina. El mínimo es **≥ 1 tick del bucle + 1 vsync**; con la recomposición completa y el churn de GPU (per-stroke `Vec::collect` + `glBufferData`/`GL_STREAM_DRAW` por overlay), el frame puede pasarse de 8.33 ms → se cae a 16.6 ms → media percibida de 30–50 ms.

### 2.2 Parpadeos / micro-congelaciones
El historial de commits lo confirma: `d982411` (“eliminate front-buffer tearing flicker by using clean 120Hz double-buffering”) indica que **ya se probó el front-buffer de un solo búfer y produjo tearing**, y se revirtió a doble-buffering. Ese retroceso reintrodujo la latencia de la sección 2.1.

Además, `present_viewer` hace `clear(bg)` + redibujado total cada frame. Si un buffer se presenta a medio componer (o un `glClear` del frame anterior aterriza fuera de fase), aparece el flash. Y como **wet ink y página viven en el MISMO búfer**, la "limpieza" del segmento predicho anterior queda acoplada al redibujado de todo el frame: un frame perdido deja residuo de predicción → parpadeo/estela.

---

## 3. Decisiones de diseño

### Principio central: **separar la tinta en vuelo del frame base, y presentarla por una vía que no dependa de la cadencia del frame completo.**

Se adopta la arquitectura **en dos capas** que usa `androidx.graphics.lowlatency.GLFrontBufferedRenderer` y el *Delegated Ink Trail* de Chromium, adaptada a Rust/NDK puro:

```
                          ┌──────────────────────────────────────────────┐
                          │ ENTRADA (240 Hz, ya madura)                  │
                          │  ink-stroke-modeler → confirmed_pt/predicted │
                          └──────────────────┬───────────────────────────┘
                                             │
                        ┌────────────────────┴─────────────────────┐
                        ▼                                          ▼
     ┌──────────────────────────────────────┐      ┌─────────────────────────────────────┐
     │ CAPA WET (Tinta en Vuelo)            │      │ CAPA DRY (Tinta Seca / Base)        │
     │ • SOLO el delta del trazo + predicción │→ ← │ • Textura de página + anotaciones   │
     │ • Render target PROPIO (FBO)         │      │   consolidadas + overlays UI        │
     │ • Se actualiza a CADA muestra        │      │ • Se re-renderiza SOLO en:          │
     │ • Present inmediato (sin vsync base) │      │   commit(Up), cambio de página,     │
     │ • glScissor al bbox del lápiz        │      │   zoom/pan, cambio de UI            │
     └──────────────────┬───────────────────┘      └──────────────────┬──────────────────┘
                        │                                             │
                        └───────────────► FUSIÓN ◄────────────────────┘
                          en ACTION_UP: la capa wet se consolida en la
                          lista de anotaciones de la base y se limpia.
```

### 3.1 Capa Dry (base) — re-render on-demand
- **Contiene:** fondo, textura de página (subida solo al cambiar), anotaciones **guardadas** (trazos + highlights, convertidos a una textura/FBO que se re-rasteriza solo al commit o al invalidarse), overlays UI.
- **Se presenta a 120 Hz** con doble-buffer y `eglPresentationTimeANDROID` para marcar cuándo debe aparecer (sin acumulación de cola). Libre de tearing.
- **Nunca se recompone por una muestra del lápiz.** Es el "frame estático".

### 3.2 Capa Wet (tinta en vuelo) — render a cadencia de entrada
- **Contiene:** únicamente el delta del trazo en curso → el segmento confirmado más reciente + el segmento predicho (Kalman), con `glScissor`/damage rect al bbox de la punta.
- **Present inmediato, desacoplado del vsync del frame base.** Mecanismo en orden de preferencia (detección en runtime):
  1. **`EGL_ANDROID_front_buffer_auto_refresh`** en una superficie dedicada de un solo búfer para la wet: se escribe con `glFlush()` inmediato. **Riesgo de tearing acotado al propio tramo** (ver §4 riesgos).
  2. Si el driver/ROM no lo soporta, **superficie overlay transparente separada** (`ANativeWindow`/`SurfaceView` secundario) presentada a su propio ritmo: el tearing queda confinado al overlay y **nunca tapa la página**.
  3. Fallback mínimo: mantiene el frame base doble-buffer, pero solo re-pinta la región del delta con `glScissor` + `eglSwapBuffersWithDamageKHR` (los píxeles fuera del rect no se tocan → sin flash del `clear`).
- **ACTIVE solo durante un gesto.** Al `ACTION_UP` → `commit()`: la capa wet se vacía y la base se marca para re-render una vez.

### 3.3 Sincronización / cadencia
- `AChoreographer` (NDK) para el frame base: render a la cadencia real del panel (120 Hz), en vez de `poll_events + take_repaint`.
- La capa wet no espera al Choreographer: se alimenta desde el evento de entrada (o un hilo de render dedicado que consume la cola de muestras).

### 3.4 Eliminar el churn de GPU por frame
- VBO de tinta **persistente con capacidad reservada** (no `Vec` + `glBufferData` por tramo).
- Trazos guardados: rasterizar a un FBO/textura solo al commit/acumular, no re-transformar `s.points` cada frame.
- Overlays: cachear quads, subir a textura solo al invalidarse (ya usa `overlay_tex` por puntero — mantener).

---

## 4. Alternativas descartadas (y por qué)

| Opción | Veredicto | Evidencia |
|---|---|---|
| **A. Mantener frame único recompuesto cada present** | ❌ | Es el estado actual; recompone todo por vsync y acopla la tinta a la cadencia del bucle (causa de ambos síntomas). |
| **B. True single-buffer front-buffer sobre el MISMO surface** | ❌ | `EGL_ANDROID_front_buffer_auto_refresh` en un solo búfer **tearing por definición** (scanout lee mientras GPU escribe). Ya se probó y se revirtió (`d982411`). Aplicarlo al frame entero rompería la página. |
| **C. Dos capas con present independiente (wet aislada)** | ✅ | Es el mecanismo de `GLFrontBufferedRenderer`: la wet vive en una superficie/FBO separado, así el tearing (si lo hay) no toca la base, y la wet puede adelantarse al vsync base. |
| **D. Delegated Ink Trail de plataforma / `MotionPredictor` (API 31+)** | ⚠️ Parcial | `MotionPredictor` es **Java**; desde `NativeActivity` no hay equivalente NDK directo (`input.h` no lo expone). Exigiría puente JNI por cada evento (latencia + sobrecarga) y el *Ink trail* de plataforma es opt-in de la capa `View`. Se reserva como **mejora futura** con puente JNI acotado, no como vía primaria. |
| **E. Sigue CPU `ANativeWindow_lock`** | ❌ | Techo medido en Spike 1: present p50 3.75 ms / p95 7.30 / max 21.26 ms — ya rompe el presupuesto antes de componer. |

---

## 5. Consecuencias y riesgos

- **Riesgo 1 (tearing en wet):** mitigado aislando la wet en su propia superficie/FBO. El tearing, si el driver no lo evita, queda confinado al tramo; la página nunca parpadea. **Medir en la TCL/Mali**: si persiste visiblemente en la wet, usar la opción 2 (overlay transparente) que lo confina aún más.
- **Riesgo 2 (driver de la TCL con front-buffer):** el propio historial dice que el single-buffer dio tearing. La decisión **no** reutiliza single-buffer para el frame entero: solo para el *delta* de una superficie aislada. El fallback (§3.2 opción 3) garantiza funcionar sin esa extensión.
- **Riesgo 3 (complejidad):** pasar de un surface a dos render targets + `AChoreographer` + `ASurfaceControl`/overlay. Es el coste de la paridad nativa. Se mitiga con la fase incremental (puede entregarse con la opción 3 primero y escalar a 1/2).

---

## 6. Invariantes respetadas (AGENTS.md)

1. **`pdf_core` desacoplado:** toda la lógica de present/modeler vive en `pdf_android`. `pdf_core` queda intacto (ya lo está).
2. **Cero alocaciones en el hot path:** VBOs persistentes, FBO de capas preasignado, sin `Vec` por tramo.
3. **PSS < 150 MB:** dos FBOs de tamaño de pantalla + una textura de página ≈ cifra acotada; validar.
4. **Rust nativo:** FFI EGL/GLES2 propio (ya validado); JNI mínimo (solo `enable_120hz` actual).
5. **Licencias:** solo primitivas públicas de Khronos/Android (MIT/Apache-2.0).

---

## 7. Plan de implementación por fases

### Fase W1 — Separar el frame en two passes con FBO (sin present independiente aún)
- Crear dos FBOs (`dry_fbo`, `wet_fbo`) del tamaño de la ventana.
- `present_viewer` deja de re-dibujar todo: pasa a **`render_dry()`** (solo si inválido) + **`render_wet()`** (solo el delta, con `glScissor` al bbox del trazo) + composición `dry ⊕ wet` → present.
- Resultado verificable: el coste por muestra baja a ∝ delta, la base deja de re-renderizarse por muestra.

### Fase W2 — Present de la wet desacoplado del vsync base
- Detectar en runtime `EGL_ANDROID_front_buffer_auto_refresh` y la disponibilidad de superficie overlay.
- Presentar la wet con `glFlush` inmediato (opción 1/2), la dry con `eglSwapBuffers` + `eglPresentationTimeANDROID`.
- Añadir `AChoreographer` para el frame base (120 Hz real), quitando la dependencia de `poll_events + take_repaint` para el present.

### Fase W3 — Commit y limpieza de la wet
- `ACTION_UP`: vaciar la wet (glClear solo del wet FBO) y marcar la dry inválida un frame (para consolidar el trazo guardado).
- Eliminar el churn: VBO persistente, trazos guardados rasterizados al FBO dry (no re-transformados por frame).

### Fase W4 — Verificación instrumental y cierre
- Medición completa (ver §8), frente a app nativa, y commit con resultados en `benchmark-results.md`.

---

## 8. Estrategia de verificación y medición

### 8.1 Latencia física punta-tinta (objetivo < 10–15 ms)
- **Cámara lenta 240 fps** cenital con línea de fondo de referencia (una regla con marcas). Medir el desfase en píxeles entre la punta física y el frente de tinta a velocidad de barrido fija (p. ej. 200 mm/s). Convertir px → ms con la velocidad conocida.
- Validar los tres casos: recta rápida, curva en 'S', giro en 'Z' + soltar.
- **Meta:** desfase < 0.5 cm a 200 mm/s (≈ < 15 ms). Comparar contra la app nativa de la TCL en el mismo protocolo.

### 8.2 Tearing / parpadeo (objetivo: 0 frames con flash)
- Grabación de vídeo a 60/120 fps durante 30 s de escritura continua; contar frames donde la página o la base "se limpian" o aparece mitad-frame. **Meta: 0.**
- `dumpsys SurfaceFlinger` + logcat del swap para detectar missed vsyncs.

### 8.3 Estabilidad de frame (p95)
- Telemetría `gl_present` ya existente desglosando: `render_dry` (ms) y `render_wet` (ms) + `swap` (ms) + `vsync` (ms). **Meta: p95 del frame total < 8.33 ms a 120 Hz, 0 frames > 16.6 ms** en ráfaga de 100+ muestras.

### 8.4 Memoria y caja
- `adb shell dumpsys meminfo com.pdflector.app`: **PSS < 150 MB** (objetivo más estricto < 100 MB) tras 100+ trazos.
- `cargo test -p pdf_core --lib` (70/70), `cargo check -p pdf_android --target aarch64-linux-android`, `cargo clippy --target aarch64-linux-android -- -D warnings -D clippy::unwrap_used`, `cargo fmt --all -- --check`.

### 8.5 Pruebas funcionales
- Trazo con presión (grosor variable), cursor de goma por botón, zoom/pan sobre la textura dry, cambio Library↔Viewer con drop/recreate de la superficie, dark mode.
