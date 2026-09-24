# Estado actual de PDFLector

> **LECTURA OBLIGATORIA PARA AGENTES.** Este archivo debe leerse antes de
> analizar el proyecto o proponer, generar o modificar su documentación. Su
> función es proporcionar un punto de partida factual sobre el software que
> existe. No es un roadmap, un backlog ni un plan de implementación.

## 1. Alcance y criterio de verdad

El punto de partida se reconstruyó el **2026-09-23** sobre el commit
`6743699a0d0e7129f57b9da8fe84f31034881b30`, leyendo código fuente,
manifiestos, pruebas, scripts y configuración de CI. La documentación previa
no se utilizó como fuente para describir el producto.

**Actualización 2026-09-24:** la APK principal
se ha migrado a `GameActivity` con host Kotlin/Gradle y capa de tinta AndroidX.
La build se ha instalado como actualización en la TCL, conservando datos y
firma. El proceso carga el documento previo y crea la superficie EGL; la
prueba física del trazo integrado sigue pendiente de desbloquear la tablet.

La jerarquía usada para resolver contradicciones fue:

1. Código conectado a los puntos de entrada y a los flujos ejecutables.
2. Pruebas que ejercitan ese código.
3. Manifiestos, scripts de build y configuración de CI.
4. Comentarios del código, solo cuando coinciden con la implementación.

Una función presente pero no conectada a ningún flujo se describe como código
disponible, no como comportamiento del producto. Una afirmación que requiere
hardware físico se marca como no verificada si no se midió durante esta
reconstrucción.

## 2. Qué producto existe

PDFLector es un workspace Rust cuyo producto principal es una aplicación
Android nativa que empaqueta el `cdylib` Rust mediante Gradle y lo ejecuta con
`android-activity`/`GameActivity`. La aplicación abre, organiza, renderiza y
anota documentos PDF. También ofrece búsqueda y descarga de papers de arXiv y
explicación asistida por IA de una selección.

El workspace contiene cuatro crates (`Cargo.toml:4-10`):

- `pdf_core`: biblioteca sin framework de UI para PDF, texto, anotaciones,
  persistencia, exportación, IA y arXiv.
- `pdf_android`: producto Android nativo.
- `pdf_app`: aplicación egui de escritorio que consume `pdf_core` y sirve como
  cliente funcional/banco de pruebas.
- `pdf_bench`: ejecutables y benchmarks Criterion sobre el núcleo.

Los miembros por defecto excluyen `pdf_android`; este crate se compila para un
target Android explícito (`Cargo.toml:8-10`).

## 3. Arquitectura observada

### 3.1 `pdf_core`

`pdf_core` no depende de crates de interfaz gráfica. Su API pública se agrupa
en `crates/pdf_core/src/lib.rs:9-50` y contiene:

- Abstracciones `Document` y `RenderEngine`, bitmap RGBA8, tamaños de página y
  extracción de texto (`crates/pdf_core/src/engine.rs:18-74`).
- Un backend MuPDF 0.8 que abre documentos, crea display lists por página,
  rasteriza y extrae spans de texto
  (`crates/pdf_core/src/engine/mupdf.rs:21-190`).
- Caché de renders LRU indexada por página y nivel de escala, limitada por
  bytes (`crates/pdf_core/src/cache.rs:20-234`). Una página individual mayor
  que el presupuesto puede permanecer sola en la caché.
- Prefetch asíncrono con un actor que posee su documento y su caché; las
  peticiones nuevas pueden sustituir trabajo pendiente entre páginas
  (`crates/pdf_core/src/prefetch.rs:36-318`).
- Caché LRU independiente de texto por página
  (`crates/pdf_core/src/textcache.rs:43-123`).
- Selección de texto por gesto y generación de resaltados alineados con los
  spans extraídos (`crates/pdf_core/src/selection.rs:33-123`).
- Anotaciones vectoriales `Stroke`, `Highlight` y `TextNote`, agrupadas por
  página y con identificadores monotónicos
  (`crates/pdf_core/src/annotations.rs:90-373`).
- Persistencia de anotaciones en SQLite y resolución de sidecars
  (`crates/pdf_core/src/store.rs:90-406`).
- Observación de cambios de sidecars mediante `notify`
  (`crates/pdf_core/src/sync.rs:75-253`).
- Exportación a Markdown y a una copia PDF con anotaciones PDF estándar
  (`crates/pdf_core/src/export.rs:62-326`).
- Transformaciones de zoom, recorte, escalado e inversión de bitmap.
- Métricas de frames y RSS.
- Clientes síncronos para Ollama, Groq y Gemini
  (`crates/pdf_core/src/ai.rs:236-820`). El consumidor debe sacarlos del hilo
  de UI.
- Cliente arXiv con construcción de consultas, parsing Atom, limitación de
  frecuencia, reintentos y descarga
  (`crates/pdf_core/src/arxiv.rs:84-760`).

Las coordenadas de texto y anotación son coordenadas de página, con origen
arriba a la izquierda y el eje Y hacia abajo. La API del motor indexa las
páginas desde cero.

### 3.2 `pdf_android`

El punto de entrada es `android_main`
(`crates/pdf_android/src/lib.rs:654-743`). El proceso mantiene un `Reader` como
estado principal y cuatro modos de UI definidos en
`crates/pdf_android/src/reader/mod.rs:90-108`:

- `Library`: biblioteca curada de documentos registrados por la aplicación.
- `Picker`: selector interno o selector de PDFs disponibles mediante
  MediaStore.
- `Viewer`: visor de una página.
- `Discover`: navegación, búsqueda y descarga desde arXiv.

La aplicación recibe también intents para abrir PDFs y enlaces/identificadores
de arXiv (`crates/pdf_android/src/reader/mod.rs:72-87`,
`crates/pdf_android/src/jni.rs:574-694`).

#### Biblioteca y apertura

La biblioteca persistente no es un reflejo automático de todo MediaStore. Los
PDFs se añaden mediante el selector, se copian a `internal/pdfs/` y después se
registran en el estado de la biblioteca
(`crates/pdf_android/src/reader/library.rs:619-776`). Existen vistas de rejilla
y lista, búsqueda, ordenación, filtros de estado y portadas generadas en un
worker (`crates/pdf_android/src/reader/library_state.rs:1-189`,
`crates/pdf_android/src/thumbs.rs:197-306`).

#### Visor y render

El visor presenta una página actual. Mantiene la actual y páginas vecinas en
una `PageCache` con límite por bytes, límite de entradas y protección de la
página visible (`crates/pdf_android/src/cache.rs:153-287`). Esta caché expulsa
según el orden de inserción o reemplazo; una lectura con `peek` no actualiza la
recencia, por lo que no es una LRU estricta
(`crates/pdf_android/src/cache.rs:189-225`). Un actor de render persistente
posee su propio `MupdfDocument`, acepta lotes y permite preempción entre páginas
(`crates/pdf_android/src/reader/redraw.rs:68-180`). El hilo de UI recibe
resultados por canales y no espera a que termine el render.

La presentación del PDF usa EGL/GLES2 y un pipeline de dos capas:

- **Dry**: página y anotaciones consolidadas; se reutiliza mientras su clave de
  página, zoom, pan, tema y generación de anotaciones no cambie.
- **Wet nativa de respaldo**: geometría transitoria del trazo activo, goma y
  selección; se recompone durante el gesto cuando no se usa la capa AndroidX.

En el producto actualizado, el bolígrafo dispone además de una `SurfaceView`
transparente superior gestionada por `GLFrontBufferedRenderer`. Kotlin aloja
esta superficie en `PdfLectorActivity`; `Reader` envía segmentos causales vía
JNI y conserva las anotaciones persistentes en el pipeline Dry nativo. La
limpieza de tinta provisional espera la presentación Dry correspondiente.
La decisión y sus límites están en
[`ADR-010`](docs/adr/ADR-010-integracion-tinta-causal-producto.md).

La implementación está en `crates/pdf_android/src/gpu/pipeline.rs:508-514` y
`crates/pdf_android/src/gpu/pipeline.rs:679-965`. La geometría de tinta se
dibuja con tiras de triángulos y discos con antialiasing, usando buffers scratch
reutilizados (`crates/pdf_android/src/gpu/pipeline.rs:267-413`).

#### Entrada, zoom y tinta

La entrada distingue tacto y stylus, consume muestras históricas del lápiz y
normaliza presión y tiempo (`crates/pdf_android/src/input/dispatch.rs:19-84`,
`crates/pdf_android/src/input/stylus.rs:14-97`). La máquina de gestos conecta:

- taps y navegación;
- pinch y pan;
- selección por pulsación prolongada;
- tinta;
- resaltado;
- borrado.

Estas rutas están activas en `crates/pdf_android/src/input/motion.rs:179-575` y
`crates/pdf_android/src/reader/tools.rs:15-399`. Las anotaciones se almacenan en
coordenadas de página. La ruta conectada del bolígrafo usa muestras reales
mediante `ink/engine.rs` y `ink/causal.rs`, sin extrapolación futura. El
resaltador mantiene su camino de selección de texto.

El movimiento de pinch actualiza la transformación sin renderizar cada evento;
al finalizar solicita el bitmap nítido. La aplicación conserva rutas CPU de
composición/blit como apoyo, pero la presentación principal del visor está
conectada al pipeline GPU.

#### Selección e IA

Una selección permite copiar, resaltar o preguntar a la IA. La consulta se
ejecuta en un hilo de fondo y el hilo de UI sondea el canal sin bloquear
(`crates/pdf_android/src/reader/toast_ia.rs:25-151`):

- Si hay una imagen recortada de la selección, se envía a Gemini junto con el
  texto extraído como contexto adicional.
- Si no hay imagen y sí texto, se usa Groq.

Las claves se incorporan durante la compilación desde ficheros locales
referenciados con `include_str!` (`crates/pdf_android/src/lib.rs:361-366`). El
código no muestra telemetría, pero las consultas de IA y arXiv sí realizan
peticiones de red.

#### Discover/arXiv

Discover implementa feed por categorías, búsqueda, paginación, caché de
respuestas y descarga. Un worker dedicado conserva el cliente de red. Las
descargas se escriben primero en un archivo `.part` y se renombran al terminar
(`crates/pdf_android/src/discover.rs:50-207`,
`crates/pdf_android/src/discover.rs:238-686`).

#### Persistencia

La aplicación guarda estado interno en varios JSON: progreso de biblioteca,
estado del visor, recientes, papers descargados, preferencias de Discover y
estado de herramientas (`crates/pdf_android/src/persist.rs:109-529`). Las
anotaciones usan el sidecar SQLite de `pdf_core` y se guardan fuera del hilo de
UI (`crates/pdf_android/src/reader/anotaciones.rs:16-79`).

Las escrituras JSON inspeccionadas son best-effort y no usan un patrón temporal
más rename; un error o JSON inválido suele degradar a valores por defecto.

### 3.3 `pdf_app`

`pdf_app` es un cliente egui para escritorio definido principalmente en
`crates/pdf_app/src/main.rs`. Usa `pdf_core` para abrir y renderizar PDFs,
scroll continuo, prefetch, caché de texturas, creación de anotaciones,
persistencia SQLite, exportación Markdown/PDF, observación de sidecars y chat
local mediante Ollama. No comparte la implementación de UI nativa de Android.

### 3.4 `pdf_bench` y herramientas

`pdf_bench` contiene un sweep manual y benchmarks Criterion de apertura,
render, caché/scroll, prefetch, zoom, blit, resaltado y anotaciones. El script
`tools/adb-bench.sh` compila y ejecuta mediciones en Android mediante ADB,
recoge información del proceso, capturas y métricas publicadas en logcat.

Existe además un experimento aislado en `android/ink_bench/`, con
`applicationId` distinto (`com.pdflector.inkbench`). Usa `SurfaceView` y
AndroidX `GLFrontBufferedRenderer` para presentar segmentos causales del
lápiz. No abre PDFs ni está conectado al visor o a la persistencia del
producto. El mismo contrato y motor causal bajo
`crates/pdf_android/src/ink/engine.rs` y `causal.rs` están conectados al
`Reader` del producto actualizado. El
analizador de `tools/ink_bench/` procesa capturas JSONL de forma separada;
no captura eventos por sí mismo. El alcance, uso y límites del experimento
están en [`android/ink_bench/README.md`](android/ink_bench/README.md) y la
decisión aprobada en
[`ADR-009`](docs/adr/ADR-009-evaluacion-tinta-causal-front-buffer.md).

La CI ejecuta formato, clippy y tests de `pdf_core` en host. En un job separado
instala NDK r28 y comprueba/clippy `pdf_android` para
`aarch64-linux-android` (`.github/workflows/ci.yml:1-55`). La CI no instala ni
ejecuta la APK en una tablet.

## 4. Estado verificado en esta reconstrucción

Comandos ejecutados el 2026-09-23 sobre el commit indicado:

```text
cargo test -p pdf_core
    181 tests superados, 0 fallos; 0 doctests

cargo check -p pdf_app
    correcto

cargo fmt --all -- --check
    correcto

cargo check -p pdf_android --target aarch64-linux-android --all-targets
    correcto con Rust 1.98.1, target aarch64-linux-android y NDK r28
```

La compilación prueba consistencia estática, no comportamiento en dispositivo.
En esta reconstrucción no se instaló la APK, no se usó una tablet y no se
midieron latencia, FPS, memoria, consumo o calidad visual. Por tanto este
archivo no atribuye cifras de rendimiento actuales al producto.

Los tests `#[cfg(test)]` que viven dentro de `pdf_android` se compilan para el
target Android durante `cargo check --all-targets`, pero no fueron ejecutados
en hardware en esta reconstrucción.

### Validación posterior del experimento de tinta (2026-09-23)

El módulo `android/ink_bench/` compila como APK debug y sus 9 pruebas JVM
pasaron. La APK experimental se instaló y arrancó en la TCL 9469X; se observó
la vista de control, la superficie independiente y la petición de 120 Hz.
El panel permaneció a 60 Hz en esa observación. El retorno al launcher y a la
actividad no produjo un crash observable. Un trazo físico posterior mostró
inversión vertical; se corrigió la proyección GL y una prueba ADB de alineación
del trazo asentado pasó en la TCL. El propietario confirmó manualmente que la
tinta corregida aparece bajo la punta. La continuidad, latencia y calidad en
otros flujos siguen **no verificadas**. Los detalles de dispositivo, build y
método están en `docs/benchmark-results.md`.

### Validación de integración en la APK principal (2026-09-24)

La nueva build `com.pdflector.app` (versionCode `16777473`, minSdk 29) se
compiló con Gradle, Rust release y `libpdf_android.so` para ARM64. Pasaron las
pruebas JVM del módulo, el check y clippy estrictos para Android, y dos
pruebas Rust acotadas de PointerUp/reconciliación Dry ejecutadas en la TCL.
La firma SHA-256 coincide con la aplicación previamente instalada y
`adb install -r` conservó sus datos. Un primer arranque falló por falta de
tema AppCompat; se corrigió el manifiesto y la actualización posterior
arrancó, cargó el documento y las 37 anotaciones preexistentes, y creó la
superficie EGL. La tablet estaba bloqueada durante esta observación: todavía
no hay validación física de trazo, subrayado, wet→dry ni percentiles de
latencia de la APK integrada. Los detalles se registran en
`docs/benchmark-results.md`.

## 5. Contradicciones y límites detectados

Estos puntos describen el código actual y deben resolverse antes de convertir
afirmaciones históricas en documentación normativa:

1. Hay comentarios extensos en `crates/pdf_android/src/lib.rs` que dicen que la
   creación de tinta y resaltados fue eliminada de la UI. Sin embargo, el input,
   `Reader::begin_tool_gesture`, `update_tool_gesture`, `end_tool_gesture`, el
   render Wet y la persistencia siguen conectados. El estado verificable es que
   el código de esas funciones está activo; su accesibilidad y UX final deben
   comprobarse en el dispositivo.
2. `pdf_core` se presenta como núcleo independiente de UI, pero también expone
   `theme::DesignTokens`. Es una dependencia conceptual de presentación, aunque
   no introduce una dependencia técnica de egui o Android.
3. El modelador de tinta mantiene estado O(1) por muestra, pero el gesto completo
   acumula puntos en varios `Vec`; no es correcto describir todo el recorrido de
   tinta como libre de asignaciones (`crates/pdf_android/src/annotations.rs:132-224`).
4. Los clientes de IA de `pdf_core` son síncronos y pueden esperar hasta cinco
   minutos. Android los desplaza a workers; otros consumidores deben hacer lo
   mismo (`crates/pdf_core/src/ai.rs:34-38`).
5. Algunos fallos de render/prefetch se descartan de forma best-effort y no
   llegan como error estructurado a la UI
   (`crates/pdf_core/src/prefetch.rs:106-118`,
   `crates/pdf_android/src/reader/redraw.rs:100-180`).
6. La ruta manual HTTP de Ollama depende de cierre de conexión o
   `Content-Length`; no implementa respuestas chunked
   (`crates/pdf_core/src/ai.rs:378-423`).
7. Las escrituras JSON de estado Android y la exportación a un PDF de salida no
   son transacciones atómicas observables en el código inspeccionado.
8. `gl_str` recorre el puntero devuelto por `glGetString` sin una guarda nula
   (`crates/pdf_android/src/gpu/shaders.rs:173-180`). Esto es un riesgo técnico,
   no evidencia de un fallo observado.
9. `PageCache::clear` vacía páginas y contadores, pero conserva los campos
   históricos `protected` e `insert_seq`
   (`crates/pdf_android/src/cache.rs:229-235`). No se observó un fallo asociado.

## 6. Qué no puede afirmarse desde este estado

Sin una ejecución o medición adicional no debe afirmarse:

- que una función visible en código sea usable o esté libre de defectos en la
  tablet;
- que se alcancen 60/120 FPS o una latencia concreta;
- que se cumpla un presupuesto concreto de memoria;
- que las APIs externas respondan correctamente con credenciales reales;
- que los flujos de permisos, intents, teclado o lifecycle funcionen en todas
  las versiones de Android;
- que una función no conectada sea parte del producto.

## 7. Uso de este archivo para documentación futura

Antes de redactar documentación nueva, un agente debe:

1. Leer este archivo completo.
2. Verificar en el código cualquier detalle que vaya a convertir en regla o
   compromiso de producto.
3. Separar estado actual, decisiones, procedimientos, evidencia y trabajo
   pendiente; no mezclarlos en un mismo documento.
4. Marcar como no verificado aquello que dependa de ejecución real.
5. No convertir riesgos detectados en tareas o prioridades sin una decisión
   explícita del propietario del proyecto.

Este archivo debe actualizarse cuando cambien la arquitectura observable, los
flujos conectados, las capacidades verificadas o los comandos de validación.
No debe contener planes futuros, prioridades ni criterios aspiracionales.
