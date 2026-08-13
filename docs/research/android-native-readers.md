# Lectores de PDF nativos en Android — investigación de patrones

> Investigación de referencia para PDFLector (Fase 1 → 6). Objetivo: cómo
> construye la gente **lectores de PDF nativos en Android** (Rust/NDK y
> referencias Java/MuPDF), y qué patrones de rendimiento/gestos usan en
> dispositivos de gama baja (nuestro caso: TCL NXTPaper 11 Plus, MT8781 8×
> Cortex-A55, sin GPU potente).
>
> **Fecha**: 2026-08-13. Clones locales (rama `main`, shallow) en `/tmp/research/`:
> `android-activity`, `rust-android-examples`, `egui`, `mupdf-android-viewer`,
> `LibreraReader`, `koreader`. Las citas usan `repos/ruta:línea`.

## 0. Resumen ejecutivo (qué copiar)

1. **Un solo hilo de render + cola de prioridad por visibilidad** es el patrón
   dominante en los lectores reales (Evince, Librera/ebookdroid, MuPDF viewer).
   Librera ordena los trabajos de decodificación por cercanía al viewport
   (`DecodeServiceBase` + `TaskComparator`); el render NUNCA se lanza en
   paralelo sin límite — el cuello de botella se serializa deliberadamente.
2. **Render por "tiles" jerárquicos o "página entera + parche HQ"**: Librera
   subdivide cada página en `PageTreeNode` con niveles de detalle y solo
   rasteriza los nodos visibles a la resolución pedida; el viewer oficial de
   MuPDF renderiza la página entera a zoom mínimo y solo re-renderiza un
   **parche** de alta calidad de la zona visible (con solo **2 bitmaps HQ
   compartidos** entre todas las páginas). Es la clave para zoom fluido y RAM
   acotada en gama baja.
3. **Blit a `ANativeWindow` en Rust**: `NativeWindow::set_buffers_geometry` +
   `lock(dirty_rect)` + escritura por `lines()` respetando `stride` (que puede
   ser ≥ width). El lock **se postea solo al dropear el guard**, y admite
   **dirty rects** para redibujar solo la zona sucia. Ya lo usamos en
   `pdf_android`, pero no explotamos dirty rects.
4. **Máquina de gestos propia con temporizadores** (KOReader): el detector de
   gestos es un state machine por contacto (tap/pan/hold/swipe/pinch/double-tap
   y variantes de dos dedos) con **intervalos configurables** (`ges_tap_interval`,
   `ges_hold_interval`, `ges_swipe_interval`) y **separación explícita del slot
   del lápiz** del dedo. En Rust, `android-activity` ya expone presión
   (`pointer.pressure()`) y botones de stylus (`stylus_primary/secondary`).
5. **Lifecycle a prueba de bombas**: `android_main` se llama varias veces si la
   Activity se recrea; la práctica común es `android:configChanges` para evitar
   recreación por rotación + `MainEvent::SaveState`/`Resume` para persistir
   posición, y `MainEvent::LowMemory` para vaciar la caché.

---

## 1. Repos estudiados

| Repo | Qué es | Relevancia para PDFLector |
|------|--------|---------------------------|
| [rust-mobile/android-activity](https://github.com/rust-mobile/android-activity) | Crate "glue" oficial para apps Rust nativas en Android (`android_main`, ANativeWindow, input, JNI). Backend de winit/egui/Slint | **Núcleo**: ya es nuestra dependencia en `pdf_android` |
| [rust-mobile/rust-android-examples](https://github.com/rust-mobile/rust-android-examples) | Ejemplos oficiales: blit puro a ANativeWindow (`na-mainloop`), egui+winit+wgpu (`agdk-egui`), etc. | Blit a bajo nivel y patrón de bucle de eventos |
| [ArtifexSoftware/mupdf-android-viewer](https://github.com/ArtifexSoftware/mupdf-android-viewer) | Viewer oficial de MuPDF para Android (Java + JNI al core C) | Render 2 niveles (entera+parche), gestos, SAF, bitmaps compartidos |
| [foobnix/LibreraReader](https://github.com/foobnix/LibreraReader) | Lector de libros para Android (incluye PDF vía MuPDF; motor ebookdroid) | Tiling jerárquico (`PageTree`), cola de decodificación priorizada, caché en disco |
| [emilk/egui](https://github.com/emilk/egui) (+eframe/winit) | UI Rust con soporte Android oficial (`hello_android`, `eframe::NativeOptions.android_app`) | Candidato UI de Fase 6; manejo de Suspended/Resumed y safe area |
| [koreader/koreader](https://github.com/koreader/koreader) | Lector multi-formato (LuaJIT + MuPDF), port Android vía SDL/luajit-launcher | Detector de gestos más completo del mercado, soporte lápiz, low-end |

Complementario (consultado en registry cargo, no clonado): **Slint** —
`i-slint-backend-android-activity` (renderer Skia/wgpu por defecto, renderer
software disponible) — directamente relevante para el spike Slint vs Tauri de
la Fase 6.

---

## 2. Técnicas

### 2.1 Entry point Rust en Android: `android_main` + android-activity

- El patrón canónico (todos los ecosistemas) es exponer un
  `#[unsafe(no_mangle)] fn android_main(app: AndroidApp)` en una `cdylib`
  (`android-activity/README.md`, `egui/examples/hello_android/src/lib.rs:13-26`,
  `slint examples/native-gestures/src/lib.rs`).
- **`android_main` se ejecuta en un hilo de bucle dedicado**, no en el hilo
  Java main; se puede ejecutar **varias veces** si la Activity se destruye y
  recrea (rotación por defecto). Por eso los ejemplos protegen la inicialización
  global con `OnceLock` (`rust-android-examples/agdk-egui/src/lib.rs:159-181`).
- **`android_on_create` (opcional)** corre en el hilo Java main dentro de
  `onCreate`, útil para JNI/logging (`rust-android-examples/na-mainloop/src/lib.rs:47-100`).
- **NativeActivity vs GameActivity**: NativeActivity no requiere nada de
  Java/Kotlin (puro Rust, más simple) pero no tiene IME (teclado en pantalla)
  completo; GameActivity (AGDK, `androidx.games:games-activity`) trae
  `AppCompatActivity` + IME + input moderno, a costa de un proyecto Gradle con
  stub Java (`android-activity/README.md:166-215`). Para PDFLector (sin texto
  de entrada intensivo) **NativeActivity es suficiente** — es lo que ya usamos
  (`crates/pdf_android/Cargo.toml`: `features = ["native-activity"]`).

### 2.2 Blit a ANativeWindow (stride/formato/dirty rects)

Ejemplo canónico de blit CPU: `rust-android-examples/na-mainloop/src/lib.rs:203-216`
(`dummy_render`) + `android-activity` (crate `ndk`):

- `NativeWindow::set_buffers_geometry(w, h, Some(HardwareBufferFormat::R8G8B8A8_UNORM))`
  — fijar formato (0,0 = conservar tamaño). RGB565 (bpp 2) también existe y
  ahorra ancho de banda en gama baja (ya lo soportamos: `draw.rs::fill_buffer`
  bpp 2).
- `NativeWindow::lock(Option<&mut Rect>)` → `NativeWindowBufferLockGuard`
  (**se postea automáticamente al dropear**). El `Rect` opcional es el
  **dirty rect**: Android amplía solo la zona a redibujar — ideal para blit
  parcial (scroll/anotaciones) sin re-pintar toda la pantalla
  (`ndk-0.9.0/src/native_window.rs:275-299`).
- El buffer expone `width()`, `height()`, **`stride()` (≥ width, con padding)**
  y `format()`; `lines()` itera filas **respetando el stride** y recortando al
  ancho visible (`ndk-0.9.0/src/native_window.rs:317-379`).
- En `pdf_android` ya bliteamos con `fill_buffer`/`copy_region` sobre el buffer
  (bpp 4 y 2), pero **sin dirty rects**: cada frame es un blit completo.

### 2.3 Surface lifecycle

- `MainEvent::InitWindow` / `TerminateWindow` / `WindowResized` /
  `RedrawNeeded` / `ConfigChanged` / `LowMemory` / `Pause` / `Resume` /
  `Destroy` (`rust-android-examples/na-mainloop/src/lib.rs:116-171`): el
  `NativeWindow` solo existe entre InitWindow y TerminateWindow; fuera de ahí,
  `app.native_window()` devuelve `None` y **no se puede dibujar**.
- eframe/winit: `Resumed` → crear window+surface GL; `Suspended` → **dropear
  window y surface** (en Android sí llega el evento, en desktop no)
  (`egui/crates/eframe/src/native/glow_integration.rs:94-102, 1359-1361`).
- **`android:configChanges="orientation|screenSize|screenLayout|keyboardHidden"`**
  evita que la rotación destruya la Activity (recomendación explícita de
  android-activity, `android-activity/README.md:83-118`); si aun así se recrea,
  `MainEvent::SaveState`/`Resume { loader }` permiten persistir estado
  (`na-mainloop/src/lib.rs:126-141`).
- `MainEvent::LowMemory` es el sitio para vaciar la caché LRU
  (`na-mainloop/src/lib.rs:151-153`).
- Frame pacing: `poll_events(Some(timeout))` con timeout de 16 ms como "vsync"
  aproximado sin GPU (`na-mainloop/src/lib.rs:82, 104-108`) — ya lo hacemos en
  `pdf_android/lib.rs` (`poll_events(Some(16ms))` cuando `needs_tick`).

### 2.4 Gestos táctiles (tap / swipe / pinch / lápiz)

- **winit → android-activity**: los `MotionEvent` se mapean a `Touch` con
  `phase` (Started/Ended/Moved/Cancelled), `id` = `pointer_id`, `location` y
  **`force` = presión normalizada** (`winit-0.30.13/src/platform_impl/android/mod.rs:381-418`).
  En Started/Ended solo se emite el pointer que cambió; en Moved/Cancelled,
  todos (`winit .../mod.rs:391-402`).
- **android-activity a pelo** (nuestro caso): `input_events_iter()` da
  `MotionEvent` con `action()`, `pointer_count()`, `pointer_at_index(i)`,
  `pointer.history()` (muestras intermedias), `event_time()`
  (`na-mainloop/src/lib.rs:273-311`) y botones `stylus_primary` /
  `stylus_secondary` + `pressure()` (crate `android-activity/src/input.rs:412-416`).
  **El lápiz llega como MotionEvent con presión y botones**, no como evento
  separado: la distinción dedo/lápiz hay que hacerla explícitamente.
- **KOReader** tiene el detector de gestos más completo (1524 líneas,
  `koreader/frontend/device/gesturedetector.lua`): por contacto (down/up
  timers), gestos `tap`, `pan`, `hold`, `swipe`, `pinch`, `double_tap`,
  `inward/outward_pan`, y variantes de dos dedos (`two_finger_tap`, etc.);
  **intervalos configurables** (`ges_tap_interval`, `ges_hold_interval`,
  `ges_swipe_interval`, `gesturedetector.lua:97-103`) — para calibrar en gama
  baja (retrasar el hold, acortar el swipe). Además **separa el slot del lápiz
  del dedo** en `device/input.lua:259` para que el puntero nunca se confunda
  con un dedo fantasma.
- **MuPDF viewer (Java)**: `GestureDetector` (onFling con velocidad →
  `OverScroller.fling` con clamps de bounds; onScroll → desplazamiento directo)
  + `ScaleGestureDetector` (pinch con foco = punto de anclaje), ambos alimentados
  desde `onTouchEvent` (`mupdf-android-viewer/.../ReaderView.java:391-475, 490-544`).
  El fling **respeta bounds** (spring-back si salió fuera) y se cancela al
  tocar (`onDown` fuerza `mScroller.forceFinished`).
- **Estado actual en PDFLector**: `input.rs` ya tiene máquina de gestos
  multitáctil (tap con `TAP_SLOP`, pinch zoom con factor relativo anclado,
  arrastre del sheet). Faltan: long-press/hold, double-tap, gestos de dos dedos
  y manejo explícito del lápiz (botones/presión).

### 2.5 Render en hilos y colas de prioridad

- **Librera/ebookdroid** (`LibreraReader/app/src/main/java/org/ebookdroid/core/`):
  - `DecodeServiceBase`: **un único hilo decodificador** (`ExecutorRunnable`)
    que consume una lista de `Task` ordenada por `TaskComparator` — prioridad
    según la **visibilidad del nodo en el viewport actual** (`PageTreeNodeComparator`
    + campo `priority`), luego antigüedad (`DecodeServiceBase.java:632-700, 855-883`).
    Los renders no visibles simplemente se quedan atrás; no hay avalancha de
    hilos.
  - Cada `PageTreeNode` tiene `AtomicBoolean decodingNow` (no se encola dos
    veces el mismo nodo) y `stopDecodingThisNode` al reciclarse
    (`PageTreeNode.java:26-80`).
- **MuPDF viewer**: `CancellableAsyncTask` (envuelve `AsyncTask` para poder
  cancelar, que en `AsyncTask` es `final`) — **antes de renderizar se cancela
  el task pendiente** (`reinit()` cancela `mDrawEntire`/`mDrawPatch`),
  evitando renders obsoletos encolados (`mupdf-android-viewer/.../PageView.java:104-130`).
  `MuPDFCore` serializa todas las llamadas JNI con `synchronized` (un solo
  `fz_context` protegido por mutex) (`MuPDFCore.java:77-244`).
- En `pdf_core` ya hay **prefetch actor 1-worker** (`prefetch.rs`) y el contexto
  MuPDF vive en el TLS del hilo creador (documentado en `evince-architecture.md`),
  coherente con "serializar el render".

### 2.6 Caché y memoria (bitmaps, tiling, disco)

- **MuPDF viewer**: solo **2 bitmaps HQ compartidos** (tamaño pantalla) para
  TODOS los `PageView` + 1 bitmap "página entera" por view reciclado
  (`PageAdapter.java:60-100`): memoria máxima ≈ 3 × pantalla. La página se
  renderiza a zoom mínimo (nítida) y al hacer zoom se re-renderiza un parche
  HQ de la zona visible; al soltar el pinch, parche nuevo (`PageView.java:100-107,
  252-267, 464-491`). Los bitmaps se `recycle()` explícitamente
  (`PageView.releaseBitmaps`, `PageAdapter.releaseBitmaps`).
- **Librera**: `PageTree`/`PageTreeNode` = tiling jerárquico: cada página se
  subdivide en nodos por niveles de detalle; cada nodo renderiza su
  `pageSliceBounds` (porción de página) al bitmap que corresponde a su nivel y
  zoom (`PageTreeNode.java:26-80`). `BitmapManager` centraliza reciclaje de
  bitmaps. Además **caché de páginas en disco** (`PageCacheFile`, key
  `md5(path + lastModified + pages + fullscreen)`, `org/ebookdroid/common/cache/PageCacheFile.java:19-31`)
  para reapertura instantánea.
- **Evince** (ya documentado en `evince-architecture.md`): caché LRU por bytes,
  un hilo de render, cola priorizada.
- **Relación con PDFLector**: ya tenemos caché LRU por bytes (pdf_core) y
  render "cover × rendered_zoom" con re-render nítido al soltar el pinch
  (`zoom.rs`). El salto pendiente es el **parche HQ / tiling** para que el zoom
  no re-renderice la página completa.

### 2.7 Apertura de documentos: SAF / MediaStore

- **MuPDF viewer**: `Intent.ACTION_OPEN_DOCUMENT` + `CATEGORY_OPENABLE` +
  `EXTRA_MIME_TYPES` (`app/.../LibraryActivity.java:28-31`), y apertura por
  `ACTION_VIEW` desde otras apps (`DocumentActivity.java:228-245`); lectura vía
  `ContentResolver.openInputStream(uri)` — el sistema ya concedió el permiso,
  no hace falta `READ_EXTERNAL_STORAGE` (`DocumentActivity.java:153-157`).
- **PDFLector ya lo tiene**: `jni.rs` implementa `ACTION_VIEW` + `openInputStream`
  y consulta a `MediaStore.Files` (API 29+). Sin cambios necesarios; el detalle
  útil restante es persistir el permiso con `takePersistableUriPermission` si se
  quisiera abrir el mismo PDF sin pasar por el picker.

### 2.8 UI Rust en Android: egui/winit vs Slint (contexto Fase 6)

- **egui/eframe**: `android_main` + `NativeOptions { android_app: Some(app) }`;
  eframe crea/destruye window+GL surface en Resumed/Suspended; **safe area
  (barra de estado) sin resolver**: el ejemplo reserva 32 px a mano
  (`egui/examples/hello_android/src/lib.rs:33-46`, referencia a winit#3910);
  egui-winit tiene `safe_area.rs` solo para iOS de momento.
- **Slint**: backend `i-slint-backend-android-activity` sobre el mismo
  `AndroidApp`; renderer por defecto **Skia (GPU/wgpu)** y renderer
  **software** disponible (i-slint-renderer-software) — relevante si la GPU del
  MT8781 (Mali-G57, sin driver Vulkan fiable) empuja a CPU; ya gestiona
  **safe area** (`set_window_item_safe_area`).
- **Conclusión para el spike Fase 6**: ambos van sobre android-activity, así
  que el harness `pdf_android` actual (blit directo) es independiente de la
  decisión. Slint ofrece safe-area y renderer software; egui ofrece un ejemplo
  Android mantenido pero con safe area manual.

---

## 3. Aplicable a PDFLector (qué mejorar, con prioridad)

> Estado de partida: `pdf_android` ya tiene blit a ANativeWindow con stride,
> gestos multitáctiles (tap/pinch), sheet animado a 60 fps, SAF/MediaStore,
> frame pacing por `poll_events(16ms)`, caché LRU por bytes y prefetch 1-worker
> en `pdf_core`. Prioridades: P1 = impacto alto / esfuerzo bajo-medio,
> P2 = medio, P3 = futuro.

| # | Propuesta | Fuente | Prioridad |
|---|-----------|--------|-----------|
| 1 | **Parche HQ de la zona visible en zoom** (render página entera a zoom mínimo + parche del viewport al soltar el pinch, en vez de re-render completo). Candidato a benchmark en la tablet: frame time de zoom. | mupdf-android-viewer `PageView` | **P1** |
| 2 | **Cola de render priorizada por visibilidad + cancelación de renders obsoletos** (una sola cola, orden por distancia al viewport, `AtomicBool` por página para no encolar dos veces). Extiende el prefetch actual de pdf_core. | Librera `DecodeServiceBase` / MuPDF viewer `CancellableAsyncTask` | **P1** |
| 3 | **Dirty rects en el blit** (`lock(Some(&mut rect))` + redibujar solo la zona sucia) para scroll/anotaciones; medir con el overlay de debug (bytes bliteados/frame). | ndk `native_window.rs:275-299` | **P2** |
| 4 | **Ampliar la máquina de gestos**: hold/long-press (para menú contextual de anotación), double-tap (zoom toggle), dos dedos (pan/tap), y **distinción lápiz/dedo** (botones `stylus_primary/secondary` + presión) con intervalos configurables. | koreader `gesturedetector.lua` / android-activity `input.rs` | **P2** |
| 5 | **Lifecycle Android formalizado**: `android:configChanges` + guardar posición por `SaveState`/`Resume` (o persistencia propia), y vaciar la caché LRU en `MainEvent::LowMemory`. | android-activity README / na-mainloop | **P2** |
| 6 | **Tiling jerárquico** (niveles de detalle por nodo) solo si el parche (1) no basta en zoom profundo; más complejo. | Librera `PageTree` | **P3** |
| 7 | Caché de páginas en disco para reapertura instantánea (opcional; Syncthing-friendly). | Librera `PageCacheFile` | **P3** |
| 8 | En el spike Slint de Fase 6: probar **renderer software de Slint** en la tablet (GPU Mali sin Vulkan) y safe-area; egui requiere hack manual de 32 px. | slint backend / egui hello_android | **P3 (Fase 6)** |

**Riesgo/nota**: el patrón "un solo hilo de render" (1 y 2) contradice
aparentemente el objetivo de 120 fps; la experiencia de Evince/Librera/MuPDF es
que la **serialización con prioridad** da mejor frame time p95 que N hilos
rasterizando a la vez, porque evita thundering herd de MuPDF (contexto no-Send,
TLS). Todo cambio debe medirse con el harness adb existente (Fase 1).

---

## 4. Fuentes

- Clones locales: `/tmp/research/{android-activity,rust-android-examples,egui,mupdf-android-viewer,LibreraReader,koreader}` (shallow, 2026-08-13).
- Crates en registry: `ndk-0.9.0`, `winit-0.30.13`, `i-slint-backend-android-activity-1.17.1`.
- Búsquedas GitHub (2026-08-13): `android pdf reader`, `android pdf rust`,
  `mupdf android`, `librera`, `slint android`, `koreader android`,
  `gh search code "android_main" --language Rust`, `gh search code "ANativeWindow" --language Rust`.
- Documentación: android-activity README (NativeActivity vs GameActivity,
  lifecycle, configChanges), winit android platform impl, eframe glow
  integration (Suspended/Resumed), ndk crate docs.
- Proyecto: `AGENTS.md`, `docs/PLAN.md` (Fases 1 y 6), `docs/research/evince-architecture.md`,
  `crates/pdf_android/` (estado actual: `draw.rs`, `zoom.rs`, `input.rs`, `jni.rs`, `lib.rs`).
