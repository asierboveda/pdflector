# ADR-004 — UI Android final: Slint (recomendación del spike)

> **Estado: SUPERSEDED by ADR-005 (2026-08-23). No vigente.**
> Ver `ADR-005-ui-android-nativa.md` (plataforma final: `pdf_android` nativa).
> Se conserva como contexto histórico; no usar como instrucción activa.

> **Origen**: spike de decisión histórico (Slint vs Tauri v2), time-boxed.
> **Fecha**: 2026-08-13
> **Estado original**: **Aceptado** el 2026-08-13 — **sustituido** por ADR-005 el 2026-08-23.
> **Hardware de medición**: TCL 9469X (NXTPaper 11 Plus), Android 15 (API 35),
> SoC MediaTek, 1440×2200, densidad 320 dpi, conectada por adb USB.
> **Entorno de build**: Omarchy/Arch Linux, Rust 1.97.1 (target
> `aarch64-linux-android`), NDK r28 (`~/Android/Sdk/ndk/android-ndk-r28`),
> cargo-apk 0.10.0 y xbuild 0.2.0 (rust-mobile).

---

## 1. Resumen ejecutivo

Se construyó y ejecutó un demo Slint 1.17.1 en la tablet objetivo y se midió:
APK **6,4 MB** (release, arm64, LTO+strip), build en frío **~1m20s**, consumo
**~62 MB PSS** en reposo con una lista scrolleable de 200 filas. El render y el
event loop funcionan en el dispositivo (primer frame y event loop verificados;
ver nota de redraws en §3.2 y §7.2).

**Hallazgo crítico (resuelto el 2026-08-13)**: la primera versión de este ADR
concluyó que el input táctil no llegaba a la app (el evento `InputAvailable` de
`android-activity` 0.6.1 jamás llegaba al looper). **Re-verificado el mismo día
con las mismas versiones de crates y toolchain, el input SÍ llega**: el demo
Slint recibe taps (logcat `InputAvailable` + callback `TouchArea.clicked`) y
swipes, y una app `android-activity` pura también, tanto en modo de poll
bloqueante como con timeout de 250 ms. La causa raíz del fallo del test puro
del ADR estaba en el propio test: la app **no dibujaba ningún frame** al
`ANativeWindow`, con lo que el WindowManager mantiene la ventana en
`frame=[0,0][0,0]` con `touchableRegion=<empty>` y el sistema **no entrega
ningún toque** a la ventana (reproducido y confirmado; detalle en §3.2 y §7.1).
Además, en esta tablet Slint 1.17.1 **no repinta la pantalla tras cambios de
propiedad** (el dirty handler del redraw no se dispara), así que la metodología
de verificación por diff de píxeles del spike no puede ver el feedback visual
de un tap aunque el input llegue: hallazgo secundario documentado en §7.2.

**Recomendación**: **Slint**, **aceptado** el 2026-08-13 tras re-verificar el
input en la tablet real (§7.1). Es la única opción que cumple
estructuralmente las prioridades 1 (fluidez) y 2 (RAM < 150 MB) del proyecto.
Tauri se descarta por presupuesto de RAM y por la latencia del puente
WebView/IPC para un lector de PDF.

---

## 2. Contexto

Fase 6 = decidir la UI final Android. Candidatos: Slint vs Tauri v2. La app
tiene `pdf_core` (Rust, sin UI) y un prototipo egui de escritorio. La UI
Android debe: scroll fluido 60 fps (objetivo 120), input táctil/lápiz sin
latencia, RSS < 150 MB con PDFs de 500 páginas, gratis y sin anuncios, y
`pdf_core` debe integrarse sin cambios (AGENTS.md §4). El plan ya apuntaba a
Slint como candidato fuerte ("un solo stack, Skia, bajo consumo"); la nota del
plan sobre "MIT/GPL dual" está **desactualizada** (ver §3.3).

Criterios de decisión (PLAN.md, Fase 6): fluidez de input táctil/lápiz, RAM,
integración con `pdf_core`.

---

## 3. Datos del spike

### 3.1 Slint — build y tamaño

Procedimiento y código reproducibles en §8. Resultados (arm64, release):

| Configuración | APK | .so | Build en frío (wall / CPU) |
|---|---|---|---|
| cargo-apk 0.10.0, release por defecto | 7,38 MB | 17,4 MB (sin strip) | 1m14s / 12m21s |
| cargo-apk + `strip = true` | 6,62 MB | 13,3 MB | 1m03s / 12m03s |
| cargo-apk + LTO + `strip` | **6,41 MB** | — | 1m20s / 8m08s |
| xbuild 0.2.0, release por defecto | **6,06 MB** | — | 1m22s / 8m12s |

- minSdk **26** (exigencia del backend `backend-android-activity-06`; el
  proyecto usa linker API 24 → hay que subir a 26 en Fase 6, es un cambio de
  una línea en `.cargo/config.toml`).
- Renderer por defecto en Android: **femtovg** (OpenGL ES). Skia es opt-in
  (`renderer-skia`). En el dispositivo quedó activo un contexto GL
  (EGL mtrack 12,4 MB, GL mtrack 7,0 MB en meminfo).
- Build con NDK r28 + Rust 1.97.1, sin Node, sin Gradle manual: **menos
  toolchain que Tauri**. El linker de `.cargo/config.toml` del repo sirve
  tal cual (basta subir API 24→26).
- Sin necesidad de tocar `pdf_core` ni el workspace (el demo es un crate
  independiente; en Fase 6 el port sería un crate `pdf_app` de UI que depende
  de `pdf_core`).

### 3.2 Slint — mediciones en la tablet real (TCL 9469X, Android 15)

Demo: ventana con lista `Flickable` de 200 filas, cabecera con contador y un
`TouchArea` de prueba. Resultados:

- **RAM (dumpsys meminfo, app en primer plano tras 6 s)**: **PSS total
  62 431 KB (~61 MB)**. Desglose: Native Heap 18,4 MB; EGL mtrack 12,4 MB;
  .so mmap 8,1 MB; GL mtrack 7,0 MB; .ttf mmap 7,1 MB. RSS bruto 210 MB
  (incluye mmap compartidos con otros procesos; la métrica relevante de coste
  de la app es PSS). El monitor de sistema TCL (`TGuard`) reporta
  `avgPss ≈ 92 MB`. Con `pdf_core`+MuPDF y la caché LRU el objetivo de
  < 150 MB RSS es alcanzable con margen.
- **Event loop + render**: el event loop funciona (Timer de 1 s dispara, los
  callbacks se ejecutan). Nota importante de la re-verificación: la
  afirmación original «un Timer que incrementa un texto redibuja la pantalla
  (diff ≠ 0)» **no se ha podido reproducir hoy**: en esta tablet Slint 1.17.1
  no repinta tras cambios de propiedad (ver §7.3); el pipeline render/present
  sí funciona cuando se fuerza (`RedrawNeeded`/`WindowResized` → `do_render`
  OK). El diff ≠ 0 original probablemente medía cambios del estado del
  sistema (barra de estado, splash, orientación), no un redraw de la app.
- **Input táctil/teclas: NO funciona (observación original del spike, del
  2026-08-13 por la mañana).** Tests originales (cada uno repetido con
  capturas de pantalla y diff de píxeles):
  - `input tap` sobre un `TouchArea` con feedback visual: **0 píxeles de
    cambio** (3 builds distintos: cargo-apk y xbuild).
  - `input swipe`/drag lento sobre el `Flickable`: 0 píxeles de cambio,
    incluso capturando durante el gesto.
  - `input keyevent` (DPAD_DOWN): 0 píxeles de cambio.
  - El sistema SÍ entrega el toque a la ventana: `dumpsys input` muestra
    `TouchStates ... touchingPointers=[Pointer(id=0, FINGER)]` sobre la
    ventana `io.github.pdflector.slintdemo/android.app.NativeActivity`, con
    `frame=[0,0][1440,2200]`, `touchableRegion` completo y `status=NORMAL`.
  - El lado Rust jamás recibe el evento: con logging en `android-activity`
    (logger propio + `android_logger`), el tap no produce `InputAvailable` ni
    `MotionEvent`; el looper solo ve `POLL_TIMEOUT` (250 ms) y los comandos
    `ID_MAIN` de arranque (Start, Resume, InitWindow, WindowResized,
    ContentRectChanged, RedrawNeeded, GainedFocus).
  - **RE-VERIFICACIÓN (misma tarde, misma versión de crates/toolchain): el
    input SÍ llega.** Con el demo Slint reconstruido desde cero (§8) el tap
    produce `poll: InputAvailable` y el callback `TouchArea.clicked`
    (logcat `RustStdoutStderr: TOUCH: clicked`); el swipe produce una
    ráfaga de `InputAvailable`; los keyevents también llegan al `AInputQueue`
    (ver §7.1). La conclusión original «el input no llega» no se sostiene:
    era un artefacto del método de verificación (diff de píxeles + test sin
    dibujo), ver §7.1 y §7.3.
- **Aislamiento (observación original, explicada hoy)**: el fallo se
  reproducía con una app `android-activity` 0.6.1 **sin Slint** (solo
  `poll_events` + logging): lifecycle completo OK, `POLL_TIMEOUT` cada
  250 ms, pero el tap no dispara `LOOPER_ID_INPUT`. **Hoy se sabe por qué**:
  esa app de test **no dibujaba ningún frame al `ANativeWindow`** y el
  WindowManager mantenía su ventana en `frame=[0,0][0,0]` con
  `touchableRegion=<empty>` (visto en `dumpsys input`); el InputDispatcher no
  entrega toques a una ventana sin región táctil, por lo que el tap nunca
  llegaba a la cola y `LOOPER_ID_INPUT` no podía dispararse. Reproducido en
  la re-verificación: el mismo test sin dibujo → ventana 0x0 y cero input;
  añadiendo un `paint` mínimo (relleno de un frame) → `frame=[0,0][1440,2200]`,
  `touchableRegion` completa e input completo. No era la cadena
  NativeActivity → AInputQueue → ALooper de la ROM TCL/MTK: era la ausencia
  de dibujo en el test.
- Nota de entorno: la tablet tenía auto-rotate activo y oscilaba entre
  orientaciones durante las primeras pruebas (errores `BLASTBufferQueue`
  landscape/portrait en logcat). Para los tests finales se fijó rotación
  portrait (`settings put system accelerometer_rotation 0`) y el error
  desapareció; el fallo de input persistió igualmente.

### 3.3 Slint — licencia (cambio respecto al plan)

La nota de PLAN.md y el enunciado asumen "Slint MIT/GPL dual". **Ya no es
cierto** (verificado en crates.io y slint.dev/pricing, 2026-08-13): Slint 1.17
es **GPL-3.0-only o licencia comercial** (Royalty-Free / Software), sin opción
MIT.

- Para este proyecto (repo AGPL-3.0 desde ADR-001) **GPLv3 es compatible**:
  combinar código GPLv3 en una app AGPL-3.0 y distribuir bajo AGPL-3.0 es
  válido (la cláusula de red de AGPL está permitida por GPLv3 §7).
- Consecuencia: mientras el proyecto sea open source, Slint es gratis. Si algún
  día se quisiera una app **propietaria**, Slint exigiría licencia de pago
  (mientras Tauri, MIT/Apache-2.0, no). Es un riesgo de licencia que debe
  quedar registrado, aunque hoy no aplica (decisión del autor: proyecto
  público y gratuito).

### 3.4 Tauri v2 — evaluación desde documentación (sin montar)

No se montó un Tauri Android completo (el spike es time-boxed y montar el
toolchain Node+Gradle+WebView exige más de una sesión; además su runtime es
conocido y documentado). Datos de su modelo de runtime (documentación oficial
v2.tauri.app, crates.io):

- **Arquitectura**: UI en HTML/CSS/JS dentro de un **WebView del sistema**
  (Android System WebView / Chromium); Rust detrás vía puente IPC (JSON
  sobre JNI). No empaqueta Chromium → APK pequeño (hello-world del orden de
  2-4 MB), pero **depende del WebView del dispositivo**.
- **Input**: lo maneja Chromium (camino probado por millones de apps) → el
  problema de NativeActivity de §3.2 no aplica. Pero la latencia de input del
  compositor de Chromium en un SoC de 200 € es típicamente 1-2 frames peor
  que un GL nativo, y en este segmento el System WebView de la ROM puede ser
  viejo o estar mal optimizado.
- **RAM**: un proceso WebView añade típicamente ~60-150 MB PSS (depende de la
  página); un visor de PDF con canvas + JS (V8 JIT, compositor) estará en el
  extremo alto. Sumado a `pdf_core`+MuPDF, **rompe el presupuesto de
  < 150 MB RSS** de la tablet.
- **Puente de datos**: cada página renderizada por `pdf_core` (bitmap a
  resolución de pantalla, ~8 MB RGBA o ~200-500 KB JPEG) debe cruzar el
  bridge IPC a JS (serialización + copias) y dibujarse en canvas: coste por
  frame de scroll y por página nueva que un GL nativo evita. Para 60 fps
  sostenidos con prefetch de páginas es viable solo con teselado y copias
  optimizadas; añade fricción y RAM temporal.
- **Toolchain**: Node.js + npm/pnpm/yarn + Rust + Android SDK/Gradle + Xcode
  (iOS). El prototipo egui no se reutiliza: la UI se reescribe en JS/TS.
  `pdf_core` sí se reutiliza intacto (Rust), pero a través del bridge.
- **Licencia**: MIT/Apache-2.0 (sin fricción con el AGPL del repo).

---

## 4. Criterios de decisión

| Criterio | Slint | Tauri v2 |
|---|---|---|
| Latencia táctil/lápiz | **Nativo GLES, input directo**; **verificado en la tablet** (2026-08-13, ver §7.1); lápiz real pendiente de validar | Chromium compositor + IPC; fiable pero ~1-2 frames extra vs nativo |
| RAM en reposo | **~62 MB PSS** medido en la tablet | WebView ~60-150 MB PSS adicionales → rompe presupuesto de 150 MB |
| Tamaño APK | **6,1-6,4 MB** (arm64, release) | ~2-4 MB (sin Chromium, pero depende del WebView del sistema) |
| Integración con `pdf_core` | Directa: crate UI Rust que llama a `pdf_core` (sin cambios) | vía bridge IPC; bitmaps de página por el puente |
| Complejidad de build | cargo-apk o xbuild + NDK (ya en el repo) | Node + frontend JS + Gradle + WebView |
| Stack | Uno solo: Rust (+ .slint declarativo) | Dos: Rust + JS/HTML/CSS |
| Licencia | **GPLv3 o comercial** (compatible con AGPL del repo mientras sea open source) | MIT/Apache-2.0 |

---

## 5. Decisión (recomendación del spike)

**Slint**, **aceptado el 2026-08-13**: el bloqueo de input quedó resuelto y
verificado en la tablet real (§7.1). Plan B (Tauri) queda descartado salvo que
Fase 6 encuentre un bloqueo nuevo con datos.

## 6. Justificación

1. **Prioridad 2 (RAM)**: solo Slint cumple. 62 MB PSS medidos en la tablet
   objetivo frente a un WebView que por sí solo puede consumir el presupuesto
   total de 150 MB. Para una tablet de 200 € con PDFs de 500 páginas y 4 años
   de uso previsto, esto es innegociable.
2. **Prioridad 1 (fluidez)**: el modelo de Slint (GL nativo, input directo,
   sin copias IPC) es la clase de pipeline que sostiene 60-120 fps en scroll
   con lápiz. Tauri hereda la latencia del compositor de Chromium y el coste
   de cruzar cada página por el bridge.
3. **Integración con `pdf_core`**: directa y sin cambios con Slint (misma
   prioridad que el plan: "cambiar egui por Slint/Tauri en Fase 6 no reescribe
   ni una línea de lógica"). Con Tauri la UI se reescribe en JS y el
   transporte de bitmaps añade fricción.
4. **Toolchain**: Slint suma menos piezas al stack ya existente (Rust + NDK +
   cargo-apk; sin Node ni frontend).
5. **Licencia**: GPLv3 compatible con el AGPL del repo mientras el proyecto
   sea open source (decisión ya tomada en ADR-001). El coste de licencia solo
   aparecería en un hipotético cambio a propietario.
6. El **hallazgo de input original** se ha **resuelto y refutado**: el fallo no
   estaba en la capa compartida `android-activity` 0.6.1 (NativeActivity), sino
   en el método de verificación del spike (test sin dibujo + diff de píxeles,
   ver §7.1 y §7.2). Con el test corregido, el input funciona en el demo sobre
   la tablet real, y por eso el ADR pasa a **Aceptado**.

## 7. Consecuencias y trabajo pendiente de validación

### 7.1 [RESUELTO 2026-08-13] Bloqueo de input en la tablet real

**Diagnóstico final**: el bloqueo documentado en la primera versión de este
ADR **no se reproduce**. Con las mismas versiones (Slint 1.17.1 con
`backend-android-activity-06`, `android-activity` 0.6.1, cargo-apk 0.10.0,
Rust 1.97.1, NDK r28) y la misma tablet (TCL 9469X, Android 15 / API 35):

- **Demo Slint** (reconstruido desde cero, /tmp/slint-android-demo): `input tap`
  → logcat `poll: InputAvailable` + callback `TouchArea.clicked`
  (`RustStdoutStderr: TOUCH: clicked`). `input swipe` → ráfaga de
  `InputAvailable`. `input keyevent` → `InputAvailable` (el evento llega al
  `AInputQueue`). La ventana es `frame=[0,0][1440,2200]` con `touchableRegion`
  completa y `status=NORMAL` (dumpsys input).
- **android-activity puro** (app mínima `/tmp/aa-input-test`, solo
  `poll_events` + `input_events_iter` + paint de un frame): el tap produce
  `poll: Main(InputAvailable)` + `MotionEvent action=Down/Up` **tanto con
  `poll_events(None)` (bloqueante, como pdf_android) como con
  `poll_events(Some(250 ms))`** (como el test original del ADR y como Slint
  con timers). Los KeyEvents también llegan.
- **Camino looper vs camino directo**: el enunciado suponía que el camino
  directo `input_events_iter()` funciona y el camino ALooper→`InputAvailable`
  no. **Es al revés**: en `android-activity` 0.6.1 `InputAvailable` solo se
  emite cuando `ALooper_pollOnce` devuelve `LOOPER_ID_INPUT` (ident 2), e
  `input_events_iter()` es únicamente el drenaje posterior de la cola. La app
  `crates/pdf_android` recibe taps precisamente **por el camino del looper**
  (su `handle_input` solo se invoca desde `MainEvent::InputAvailable`). No hay
  feature flag ni modo en `android-activity` 0.6.1 para cambiar de camino: el
  camino del looper **funciona** en esta tablet una vez que la ventana es
  real (ver siguiente punto).
- **Causa raíz del fallo original del test puro**: la app de test del ADR
  («solo poll_events + logging») **nunca dibujaba al `ANativeWindow`**. En una
  NativeActivity que no dibuja, el WindowManager mantiene la ventana en
  `frame=[0,0][0,0]` con `touchableRegion=<empty>` (confirmado en `dumpsys
  input`), y el InputDispatcher **no entrega toques** a esa ventana: el tap
  jamás llega a la cola, luego `LOOPER_ID_INPUT`/`InputAvailable` no puede
  dispararse, y el looper solo ve `POLL_TIMEOUT`/`ID_MAIN`. Reproducido
  exactamente en la re-verificación: mismo test sin dibujo → ventana 0x0 y
  cero input; añadiendo un `paint` mínimo (un frame rellenado) → ventana
  completa y todo el input llega. **Regla para el port**: la primera acción
  tras `InitWindow` debe dibujar (o iniciar el renderer, como hace Slint con
  EGL/femtovg); una NativeActivity sin dibujar es intocable en esta ROM.
- **Fix / workaround**: ninguno necesario en Slint ni en android-activity.
  El port de Fase 6 debe usar el camino estándar (poll + `InputAvailable` +
  `input_events_iter()`) tal cual lo hace el backend de Slint. No hay versión
  de crate, feature flag ni parche que aplicar; el fix era corregir el
  **método de verificación** (test que dibuja) y descartar el diagnóstico
  original.
- **Pendiente físico**: probar el **lápiz real** (no solo `input tap`, que
  simula dedo) al inicio de Fase 6; la ROM tiene un overlay
  `stylus-handwriting-event-receiver` con `INTERCEPTS_STYLUS` que podría
  interceptar el stylus y hay que validarlo con hardware.

### 7.2 [NUEVO] No-repaint tras cambios de propiedad (riesgo a vigilar)

Hallazgo secundario de la re-verificación (con instrumentación del backend
`i-slint-backend-android-activity` 1.17.1 y de `i-slint-core` 1.17.1): en esta
tablet, tras el primer frame, **los cambios de propiedad no disparan
`WindowRedrawTracker::notify` ni `request_redraw`**: el Timer de 1 s se
ejecuta (callbacks OK) pero la pantalla queda congelada en el primer frame
(diff de píxeles = 0 durante minutos con contadores que cambian). El pipeline
render/present funciona si se fuerza (`RedrawNeeded`/`WindowResized` →
`do_render` OK). Consecuencias:

- La metodología del spike (diff de píxeles) **no puede detectar feedback
  visual de input** en esta configuración: el «0 píxeles de cambio» del
  tap original no demuestra que el input no llegara.
- Para el port real hay que vigilar/reportar upstream. Issues relacionados en
  slint-ui/slint: **#8692** (request_redraw no funciona en Android), **#12687**
  y **#12688** («a pending redraw does not wake the event loop» / «let a
  pending redraw shorten the poll timeout», fix mergeado 2026-07-29, **no
  publicado en ninguna release**; la última estable es 1.17.1 del 2026-07-07).
- Mitigación a probar al inicio de Fase 6, en orden: (a) actualizar Slint en
  cuanto salga una release con #12688; (b) parchear el backend en el port
  (que el timeout del poll considere `pending_redraw`, como el fix #12688);
  (c) si el no-repaint persiste incluso con el fix, aislarlo con una
  reproducción mínima y reportarlo a slint-ui/slint. Un PDF viewer puro
  redibuja bajo demanda (scroll/tap → nuevo frame), así que este riesgo solo
  es crítico si también afecta a los redraws forzados por input; verificar en
  el primer port.

### 7.3 Trabajo pendiente restante

1. **Lápiz real en la tablet** (validación física, ver §7.1).
2. **Linker API 24 → 26** en `.cargo/config.toml` (minSdk del backend
   android-activity). Cambio de una línea en Fase 6; no afecta a MuPDF
   (ADR-001 compila con el clang wrapper del NDK).
3. **Licencia Slint**: actualizar la tabla de decisiones de PLAN.md (el
   "MIT/GPL dual" ya no existe) y registrar que la app queda AGPL-3.0 con
   dependencia GPLv3 — compatible, pero impide futuro propietario sin licencia
   comercial Slint.
4. **Mediciones finales pendientes** (con el port completo): frame time p95 en
   scroll con el corpus de 500 páginas, RSS final (`pdf_core`+MuPDF+UI),
   latencia de trazo de lápiz (no se necesita presión), consumo de batería en
   uso diario, y la semana de uso real del criterio de Fase 6.
5. **Femtovg vs Skia**: el demo usó femtovg (default). Antes del port
   definitivo, medir frame time p95 con ambos en la tablet (Skia puede
   interesar para trazados de anotaciones vectoriales del ADR-002/plan).

---

## 8. Procedimiento de build usado (reproducible)

Demo completo fuera del repo (en `/tmp`, desechable; fuente intacta en
`/tmp/slint-android-demo/` durante la sesión del spike; reconstruido en la
re-verificación con la misma config):

```toml
# Cargo.toml (demo de spike)
[package]
name = "slint-android-demo"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
slint = { version = "1.17.1", features = ["backend-android-activity-06"] }

[package.metadata.android]
package = "io.github.pdflector.slintdemo"
build_targets = ["aarch64-linux-android"]
apk_name = "slint-pdf-demo"
version = "0.1.0"

[package.metadata.android.sdk]
min_sdk_version = 26
target_sdk_version = 35

[package.metadata.android.signing.release]
path = "/home/asierboveda/.android/debug.keystore"
keystore_password = "android"

[profile.release]
lto = true
codegen-units = 1
strip = true
```

```rust
// src/lib.rs (núcleo)
slint::slint! {
    export component MainWindow inherits Window {
        in-out property <int> tick: 0;
        // ... lista Flickable de 200 filas + TouchArea de prueba ...
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app).unwrap();
    MainWindow::new().unwrap().run().unwrap();
}
```

Build y despliegue (2 rutas equivalentes):

```bash
# ruta A: cargo-apk 0.10.0 (ya instalado en el entorno)
cargo apk build --release
adb install -r target/release/apk/slint-pdf-demo.apk

# ruta B: xbuild 0.2.0 (el recomendado por la doc de Slint)
cargo install --git https://github.com/rust-mobile/xbuild.git --locked
x build --platform android --arch arm64 --format apk --release
adb install -r target/x/release/android/slint-android-demo.apk
```

Requisitos: `rustup target add aarch64-linux-android`, NDK r28,
`ANDROID_HOME=~/Android/Sdk`, linker del NDK en PATH
(`aarch64-linux-android24-clang`, subir a 26 en Fase 6) y keystore de
debug. Nota: `/tmp` del entorno es tmpfs de 6,8 GB; un build con LTO completo
no cabe junto con otros trabajos — con `strip` sin LTO bastó.

App de diagnóstico de la re-verificación (android-activity puro, dos modos de
poll mediante features): `/tmp/aa-input-test/` — dibuja un frame (requisito
para que la ventana sea touchable, ver §7.1), drena `input_events_iter()` en
cada iteración y loguea cada `PollEvent`. Modos: `--features blocking`
(`poll_events(None)`) y `--features timeout` (`poll_events(Some(250ms))`).
Ambos reciben taps/keyevents en la tablet.

Medición de RAM (los valores de §3.2):

```bash
adb shell am start -n io.github.pdflector.slintdemo/android.app.NativeActivity
adb shell "dumpsys meminfo io.github.pdflector.slintdemo"
```

---

## 9. Referencias

- Slint docs Android: `docs.slint.dev/.../guide/platforms/mobile/android/`
  (xbuild + `backend-android-activity-06`, minSdk 26).
- Licencia Slint: crates.io (`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0
  OR LicenseRef-Slint-Software-3.0`) y slint.dev/pricing (GPLv3 / comercial).
- android-activity 0.6.1 (crate `android-activity`, rust-mobile) — capa
  NativeActivity/GameActivity usada por el backend de Slint.
- Tauri v2 docs: v2.tauri.app (WebView del sistema, requisitos Node).
- ADR-001 (motor MuPDF, repo AGPL-3.0) y PLAN.md Fase 6 (spike UI final).
- Re-verificación 2026-08-13: issues de slint-ui/slint **#8692**
  («request_redraw isn't working for me on Android»), **#12687** y **#12688**
  («Android: a pending redraw does not wake the event loop» / «let a pending
  redraw shorten the poll timeout», fix mergeado 2026-07-29, sin release aún).
  Sin issues equivalentes abiertos en rust-mobile/android-activity sobre
  input + Android 15 que afecten a esta cadena.
