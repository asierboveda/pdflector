# Registro de Evidencia de Rendimiento

Este documento es el **registro único y canónico de evidencia empírica** de rendimiento de PDFLector. Funciona bajo el modelo **append-only** (cronológico inverso: mediciones más recientes primero).

## Regla de oro
> **Ninguna afirmación de rendimiento sin fecha + hardware + flujo medido + métrica.**
> Si un dato no ha sido medido en hardware real, se declara explícitamente como `SIN MEDIR`. No se admiten estimaciones teóricas ni extrapolaciones como hechos.

## Cómo añadir una entrada
1. Insertar la nueva medición al principio del registro (debajo de esta cabecera).
2. Indicar fecha ISO (`AAAA-MM-DD`), hardware exacto (modelo, CPU/SoC, RAM, SO), build/commit y condiciones ambientales (pantalla ON/OFF, governor, batería).
3. Describir el flujo medido y el método de captura (`adb-bench.sh`, logcat streaming, criterion, dumpsys).
4. Documentar métricas crudas (p50, p95, RSS/PSS) sin maquillar desviaciones ni calcular fps teóricos inversos.
5. Si una medición contradice un dato previo o una decisión de diseño, documentar la discrepancia con total honestidad.

---

## 2026-10-04 — Confirmación de cierre del propietario (ADR-011)

- **Clasificación:** cierre confirmado por el propietario para los cuatro
  criterios de aceptación de ADR-011 en la TCL NXTPaper 11 Plus 9469X. Esta
  entrada registra la confirmación recibida, no una nueva medición ejecutada
  por el agente.
- **Alcance confirmado:** el propietario confirma que se completaron las
  comprobaciones de encuadre, geometría página↔pantalla dentro de tolerancia,
  alineación/selección tras rotación física, navegación/caché y rendimiento
  indicados por el ADR.
- **Registro disponible:** la evidencia anterior de encuadre, interacción y
  pruebas automatizadas permanece en las entradas precedentes. La documentación
  disponible no contiene los valores crudos del round-trip ni del ensayo de
  rendimiento confirmado por el propietario. No se reconstruyen ni se inventan
  valores de error, p95/p99, frames perdidos o PSS. Los PSS y p95 de diagnóstico
  del 2026-09-27 permanecen etiquetados como no comparables y no se presentan
  como ese ensayo.
- **Límite documental:** no se recuperaron con esta confirmación el hash/build
  exacto, la identificación por nombre/hash de cada PDF de prueba ni las
  muestras crudas del ensayo original. El cierre se atribuye explícitamente a
  la confirmación del propietario; esta entrada no convierte los diagnósticos
  anteriores en un benchmark reproducible.

---

## 2026-09-27 — Revisión física de encuadre, tinta y selección (ADR-011)

- **Clasificación:** observación funcional confirmada por el propietario en la
  TCL NXTPaper 11 Plus 9469X. No se recogieron capturas ni coordenadas crudas
  durante esta sesión; no es una medición de round-trip ni de rendimiento.
- **Build:** no se consultaron por ADB el versionCode ni el hash de la APK en
  esta sesión; no se atribuye esta observación a un identificador de build
  comprobado aquí. No se compiló ni instaló una APK durante la guía.
- **Guide, página 2:** el propietario reabrió manualmente
  `Guide_campus_virtual_26.pdf` en vertical y confirmó que se veían completos
  el título, el texto, el mapa y los cuatro bordes. Esto vuelve a comprobar el
  caso original que no se había reabierto en la pasada diagnóstica anterior.
- **Página de prueba rotada/CropBox desplazado:** con el mismo ejemplar de
  prueba en dos orientaciones físicas de la tablet, el propietario probó el
  lápiz y el gesto de selección táctil en zoom 1 y 2. Reportó tinta visible
  durante el contacto y trazo asentado alineado bajo la punta en ambos zooms y
  orientaciones. También reportó correcto el seguimiento del dedo por el gesto
  táctil. La selección descrita aquí es el gesto que sigue el dedo; esta
  observación no acredita selección semántica de texto.
- **Identificación y condiciones no capturadas:** no se anotaron el nombre ni
  el hash del PDF de prueba, muestras numéricas, capturas, temperatura, PSS ni
  refresco efectivo de esta sesión. La orientación inicial no quedó indicada;
  la segunda se obtuvo girando físicamente la tablet 90°.
- **Pendiente:** el round-trip pantalla↔página con tolerancia ≤1 punto PDF
  requiere pares de coordenadas y error calculado; la inspección visual no lo
  demuestra. PSS comparable y frame p95/p99/pérdida de frames siguen sin una
  medición controlada. Esta sesión no cierra esos criterios ni el ADR completo.

---

## 2026-09-27 — Geometría, selección y métricas diagnósticas ADB (ADR-011)

- **Clasificación:** observaciones funcionales con PDFs generados y eventos
  sintéticos en TCL. La página rotada se inspeccionó visualmente, sin probar
  sobre ella la alineación de tinta/selección. PSS y frame p95 son muestras
  diagnósticas; no constituyen un benchmark ni cierran el ADR.
- **Dispositivo/build:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36,
  1440×2200, rotación 0 (portrait), app `com.pdflector.app` 0.2.0,
  versionCode `16777473`, proceso 22604 durante las comprobaciones. El ensayo
  parte del worktree `codex/adr-011-geometria`, revisión
  `496f5127cbf47cc30f5beffffa25c470a09a71de` más cambios locales. No se
  registró el SHA-256 del APK instalado en esta pasada. Refresco efectivo
  60 Hz; el panel anuncia soporte de 120 Hz. Estado térmico 1 al inicio y fin;
  CPU/GPU reportadas 49,292 °C y skin 44,066 °C en ambas lecturas, sin cambio
  térmico significativo observado. El SoC y la batería no quedaron registrados
  en esta pasada.
- **Página abierta al inicio:** se capturó e inspeccionó antes de cualquier
  interacción; la imagen mostraba texto en español, sin título/página legible
  en la interfaz. No se interactuó con ese documento. La captura privada se
  eliminó después y no se conserva en el repositorio ni en `/tmp`.
- **A4 visible:** PDF A4 generado con borde azul inset, abierto con `ACTION_VIEW`.
  Se vieron los cuatro bordes sin recorte aparente. PDF SHA-256
  `d1b2629bbbb37eb6d7f8a91a95153f934d7f2d0d2849ecf9028653ff22aed426`;
  captura `a4-border-selection-open.png`, SHA-256
  `7e9b0ffe69ee77e6e021961c7c71d12be6e181e473f91da63a7655a4246b3152`.
- **Rotación y CropBox desplazado:** PDF generado con MediaBox
  `[0 0 620 880]`, CropBox `[75 55 545 825]`, Rotate 90. En pantalla se vio
  en formato apaisado aunque la tablet permaneció en vertical, con borde rojo
  y contenido diagonal dentro del recorte, sin clipping aparente. Esto
  comprueba presentación visible, no la alineación de tinta, selección o
  round-trip de coordenadas. PDF SHA-256
  `50d38d45e0cf84dd724959e49b1c05f2cceb1c1cc2ca0cd32c13e09ed7f0d121`;
  captura `rotated-shifted-cropbox-open.png`, SHA-256
  `0cd799316df05e0aeb06752e55c9e6a32200de3b14d08c83af5a5ce3174a3d6a`.
- **Stylus sintético:** se inyectaron `DOWN`, siete `MOVE` y `UP` por ADB sobre
  el A4 generado. Los logs registraron envíos de `gl_present` y la captura
  posterior a `UP` muestra una línea asentada aproximadamente sobre la
  trayectoria. Las capturas en DOWN/MOVE no difieren visualmente del blanco
  previo, así que esta prueba no demuestra feedback wet durante el contacto ni
  sensación/latencia del lápiz físico. El propietario ya confirmó por separado
  tinta física durante el movimiento horizontal en la build limpia; aquella es
  una observación manual independiente.
- **Selección sintética:** long-press y arrastre táctiles ADB en el texto del
  A4 seleccionaron “The quick brown fox jumps over the lazy” y mostraron el
  menú Copiar/Subrayar/IA. No se eligió ninguna acción ni se creó resaltado.
  Captura `text-selection-diagonal.png`, SHA-256
  `3071242fc7d51c90682c3af8d4f196dba64d360de1201ed975fed7fd16d56560`.
- **PSS total (kB, `dumpsys meminfo`, muestras ~1 s):** antes de la interacción
  sintética: 17:27:36.667 `298944`; 17:27:37.839 `298635`; 17:27:38.976
  `287167`; 17:27:40.111 `287167`; 17:27:41.226 `287167`. Después de la
  interacción: 17:33:09.218 `203281`; 17:33:10.372 `198961`; 17:33:11.487
  `198917`; 17:33:12.614 `198917`; 17:33:13.731 `198917`. Son estados
  diferentes y una serie breve, no una prueba controlada de PSS estable ni de
  estrés. Los datos brutos están en `/tmp/pdflector-adr011-evidence/`.
- **Frame p95 leído de logcat:** `09-27 17:31:29.814 ... 797.6 ms (120
  frames)` sin contexto controlado; y `09-27 17:32:45.527 ... 68.2 ms (120
  frames)` tras 140 comandos ADB individuales `MOVE`. La segunda ventana
  incluye tiempos ociosos y latencia de los comandos anfitriones; ninguna cifra
  es un p95 válido de frame durante interacción continua o de trabajo de app.
  No se midieron p99, frames perdidos ni lápiz→píxel. El refresco seguía en
  60 Hz. Los logs y capturas de ensayo no personales permanecen en
  `/tmp/pdflector-adr011-evidence/`.
- **Estado y limpieza:** no se abrió, anotó, copió, exportó ni borró ningún PDF
  personal. Se retiraron del dispositivo los dos PDFs generados y sus sidecars
  de anotación. Para no reabrir un documento personal cuyo URI/página guardados
  no estaban disponibles de forma segura, la aplicación quedó en Biblioteca;
  la lista de recientes puede conservar las URI ya retiradas de los dos
  fixtures. No se limpiaron datos de aplicación.
- **Pendiente:** repetir con flujo de interacción continuo para métricas de
  frame válidas; medir PSS con estados y protocolo comparables; probar
  selección/tinta sobre la página rotada a zoom 1 y 2, rotación física de
  tablet y puntos de página↔pantalla en dispositivo; repetir el caso Guide de
  p. 2 para el cierre del bug original.

---

## 2026-09-27 — Tinta wet en horizontal (ADR-011)

- **Clasificación:** observación funcional del propietario en la TCL; no es
  una medición de latencia y no cierra el ADR-011.
- **Dispositivo/build:** TCL NXTPaper 11 Plus, modelo 9469X, Android 16/API 36.
  APK `com.pdflector.app` 0.2.0, versionCode `16777473`, instalada mediante
  actualización conservando los datos existentes. SHA-256 instalado y local:
  `9019f61cdb21774c1e624a66dc7277f47bb59868a35ba8cd53bd60444d77f749`.
- **Flujo observado:** con la tablet en horizontal, mantener el lápiz apoyado y
  moverlo. El propietario confirmó que la tinta aparece durante el movimiento
  en la build limpia; la primera build con la transformación de proyección aún
  mostraba el trazo al levantar el lápiz.
- **Diagnóstico de la build instrumentada:** callback AndroidX con superficie
  lógica de 2200×1440 y buffer prerrotado de 1440×2200; la matriz aplicada a
  los puntos llevó las coordenadas a rango de clip y se envió el frame a
  presentación. La instrumentación se retiró antes de generar la build limpia.
- **Límites:** no se midieron latencia lápiz→píxel, p95/p99 de frame, pérdida de
  frames ni refresco efectivo durante esta observación. Selección, zoom 2 y
  página rotada/CropBox desplazado siguen sin validar en la TCL.

---

## 2026-09-26 — Ejecución del encuadre PDF (ADR-011)

- **Clasificación:** verificación automática y observación visual parcial en
  dispositivo. No cierra el ADR: faltan interacción física y frame p95.
- **Código/build:** worktree `codex/adr-011-geometria`, HEAD
  `496f5127cbf47cc30f5beffffa25c470a09a71de` más cambios locales del ADR-011,
  incluidos los ya presentes al iniciar esta ejecución. APK debug ARM64
  `com.pdflector.app` 0.2.0 / versionCode `16777473`; SHA-256
  `92f01fc16aa47ca61eb3aa486da4724ec717a7d8f067398874c43d80c4198a42`.
  Construida con el wrapper Gradle 8.9 y NDK r28; build `assembleProductDebug`
  correcto e instalación final mediante `adb install -r`. El arranque frío de
  `PdfLectorActivity` devolvió `Status: ok` en 1106 ms; `firstInstallTime`
  siguió en 2026-09-07. Las capturas muestran la p. 2/18 de Guide y, al final,
  `dense_textbook.pdf`, p. 70/93.
- **Validación automática:** `cargo fmt --all -- --check`, `git diff --check`,
  `cargo clippy -p pdf_core --all-targets -- -D warnings`,
  `cargo test -p pdf_core` (186 pasaron, 0 fallaron),
  `cargo check -p pdf_android --target aarch64-linux-android --all-targets`,
  `cargo clippy -p pdf_android --target aarch64-linux-android --all-targets
  -- -D warnings` y build del APK: correctos. En la TCL pasaron 12 pruebas
  Android ARM64: 7 de `view` (incluyen cuatro de contain/margen), 3 de caché,
  transformación y mapeo a bitmap, y 2 de prefetch ±1. No se ejecutó allí el
  conjunto completo de pruebas.
- **Dispositivo/condiciones:** TCL NXTPaper 11 Plus, modelo 9469X, MT8781,
  Android 16/API 36; ADB `A06B4A8E6774623`; 1440×2200, 320 dpi, rotación 0,
  pantalla encendida, USB, batería 41 %. Refresco efectivo 60 Hz (modo 2,
  `renderFrameRate=60.0`); el panel anuncia también 120 Hz. En la sesión
  previa, la muestra térmica almacenada a las 21:07:31 fue CPU/GPU 49,292 °C,
  SoC 49,475 °C, skin 44,066 °C y batería 27,6 °C. En la sesión final, a las
  21:33, HAL informó piel 35,052 °C y batería 29,3 °C; CPU/GPU/SoC no estaban
  disponibles. No hay temperaturas inicial/final comparables.
- **Separación de builds:** las primeras muestras visuales y de PSS de esta
  entrada corresponden al APK candidato previo, SHA-256
  `b51477cded9f875af744b9b94417af8151f37bce97f3532bd3140183b0734c14`.
  El APK final
  `92f01fc16aa47ca61eb3aa486da4724ec717a7d8f067398874c43d80c4198a42` se
  instaló y probó después: muestra Guide p. 2 completa, arranca correctamente
  y volvió a `dense_textbook.pdf` p. 70/93. La serie final de PSS está separada
  de las muestras del candidato previo.
- **Flujo observado:** se abrió la posición guardada de
  `Guide_campus_virtual_26.pdf`, p. 2/18, desde la biblioteca. Se ven los
  cuatro bordes, el título «Come to our Library», el texto y el mapa completos.
  `dense_textbook.pdf`, p. 70/93, apareció centrado; su papel blanco se funde
  con el fondo, por lo que no se dan por verificadas visualmente sus cuatro
  esquinas. No se guardaron capturas ni PDFs en el repositorio. Se restauró la
  posición guardada de `dense_textbook.pdf` a p. 70/93.
- **PSS/RSS crudos (KiB):** tras navegar a la p. 2 de Guide,
  `261504/377371`; después de reposo, a las 21:07:31 y con intervalos de 4 s,
  `192318/308871`, `192146/308699`, `192146/308699`. En `dense_textbook.pdf`
  p. 70, tras restaurar, una muestra fue `339643/460105`. Son estados distintos,
  del APK candidato previo
  `b51477cded9f875af744b9b94417af8151f37bce97f3532bd3140183b0734c14`. En el
  APK final
  `92f01fc16aa47ca61eb3aa486da4724ec717a7d8f067398874c43d80c4198a42`, después
  de abrir Guide p. 2, las muestras fueron `253720/371079`, `196042/313927` a los 4 s y
  `196100/313935` a los 8 s. Esta serie corta refleja navegación/reposo, no un
  pico de estrés ni por sí sola el presupuesto estable de memoria.
- **No medido/no observado:** frame p95/p99 y frames perdidos durante
  interacción continua; lápiz→píxel; selección real (el long-press sintético
  no activó selección); escritura con lápiz, zoom 2 y cambio de orientación;
  página rotada con `CropBox` desplazado en pantalla. La prueba core sí cubre
  geométricamente ese último tipo de PDF. Un ejemplar PowerPoint independiente
  no está disponible en la prueba.
- **Protección de datos:** no se borraron ni exportaron PDFs, no se limpió el
  almacenamiento de la app y no se añadieron anotaciones. El binario temporal
  de pruebas Android se retiró de `/data/local/tmp`.

---

## 2026-09-26 — Validación automática y ciclo de vida ADB (ADR-010)

- **Clasificación:** validación parcial del ADR-010; no cierra el ADR. Los
  percentiles de interacción y los gestos físicos siguen sin medirse en esta
  tanda.
- **Código probado:** worktree `adr-011`, HEAD
  `496f5127cbf47cc30f5beffffa25c470a09a71de`, con cambios locales sin commit
  presentes. Las comprobaciones Rust se ejecutaron contra ese árbol; no se
  atribuyen a una build limpia de `main`.
- **Build instalada:** TCL NXTPaper 11 Plus, modelo `9469X`, Android 16/API 36,
  app `com.pdflector.app` 0.2.0 / versionCode `16777473` (minSdk 29,
  targetSdk 35). APK debug ARM64 local e instalada con SHA-256 idéntico:
  `d86175be519d7fcff33646b6fa492ebda8a7f531958d19fc8b52f91217b19c14`.
  No se reconstruyó ni reinstaló durante esta tanda: se comprobó la identidad
  del APK existente y no se alteraron los datos de la app. `apksigner verify`
  confirmó la firma APK v2; certificado SHA-256
  `a1b691cffc1b8ed4897e708ba25a19bc4fc2c735871f9f57d67bbc6b13fe0969`.
- **Toolchain:** Rust 1.98.1; Android NDK r28 en
  `/home/asierboveda/Android/Sdk/ndk/android-ndk-r28`, con el toolchain
  LLVM/sysroot de ese NDK para la compilación cruzada.
- **Validación automática:**
  - `cargo fmt --all -- --check`: correcto.
  - `git diff --check`: correcto tras añadir esta entrada.
  - `cargo clippy --all-targets -- -D warnings`: correcto.
  - `cargo test -p pdf_core`: 184 pruebas correctas, 0 fallidas.
  - `cargo check -p pdf_android --target aarch64-linux-android --all-targets`:
    correcto.
  - `cargo clippy -p pdf_android --target aarch64-linux-android --all-targets
    -- -D warnings`: correcto.
  - `cargo test -p pdf_android --target aarch64-linux-android --lib --no-run`:
    correcto; el binario se ejecutó en la TCL con ADB: 57 pruebas, 56
    correctas y 1 fallida. Fallo
    `ink::tests::test_proyeccion_kalman_y_modulacion`,
    `crates/pdf_android/src/ink/tests.rs:140`: lead observado `9.47 pt` frente
    a `~15 pt` esperado. Ejecución: se copió
    `target/aarch64-linux-android/debug/deps/pdf_android-bb6df0b0fcfb5913` con
    `adb push`, se aplicó `adb shell chmod 755
    /data/local/tmp/pdflector-adr010-tests` y se lanzó con
    `adb shell /data/local/tmp/pdflector-adr010-tests`. La discrepancia queda
    abierta; no se modificó código.
  - `./gradlew :app:testDebugUnitTest`: no ejecutable; el repositorio no tiene
    `android/product/gradlew` y el entorno no tiene un binario Gradle instalado.
    No se obtuvieron resultados de pruebas JVM de Kotlin.
- **Dispositivo y estado:** serial `A06B4A8E6774623`; pantalla encendida,
  1440×2200, rotación 0, densidad 320 dpi. El panel admite 60/120 Hz; el modo
  efectivo leído fue 60 Hz (`mActiveModeId=2`). Batería 39 %, USB conectado.
  Muestra térmica puntual: CPU/GPU/SoC ~49.3–49.5 °C, piel 44.1 °C,
  batería 27.6 °C; estado térmico 1. No es una medición térmica antes/después.
- **Flujo ADB:** con `PdfLectorActivity` reanudada, se envió `KEYCODE_HOME` y
  se volvió con `adb shell am start -W -n
  com.pdflector.app/.PdfLectorActivity`. Resultado `Status: ok`,
  `LaunchState: WARM`, `TotalTime: 349 ms`; la actividad quedó reanudada.
  El filtro de logcat para errores de `AndroidRuntime`, `PdfLectorActivity` y
  `pdf_android` no devolvió líneas. Esto confirma un ciclo Inicio/retorno
  caliente, no la recuperación tras destruir la superficie durante un trazo.
- **Memoria (KiB, muestras puntuales):** antes del ciclo,
  PSS `345142` / RSS `467993`; después, PSS `268760` / RSS `391946`.
  Corresponden a momentos/estados distintos; no forman una serie de reposo o
  estrés y no demuestran el presupuesto de memoria estable. Frames, pérdidas
  de frame y latencia lápiz→píxel: `SIN MEDIR`.
- **Criterios del ADR aún pendientes de observación de producto:** gestos
  físicos y continuidad con zoom; transición wet→dry tras presentación
  correcta; cancelación de un gesto en la UI; cambio de página durante tinta;
  pérdida/recreación de superficie durante tinta; ejecución de la ruta Rust
  wet de respaldo cuando AndroidX rechace un segmento; y mediciones de
  interacción en la TCL. La observación manual ya registrada en esta fecha
  informa que a zoom alto el trazo aparece al levantar el lápiz, aunque la
  línea asentada se alinea con la punta; se observaron pequeños bordes. Eso no
  valida feedback continuo a zoom alto ni wet→dry.
- **Protección de datos:** no se limpió almacenamiento, no se desinstaló la
  aplicación y no se inyectaron trazos ADB en PDFs personales. Las acciones de
  este ensayo fueron lecturas ADB, extracción de la APK a `/tmp`, ejecución
  del harness de pruebas y el ciclo Inicio/retorno.

---

## 2026-09-26 — Tinta con zoom: observación manual del propietario

- **Clasificación:** observación funcional durante las pruebas de ADR-011; sin
  medición de latencia.
- **Hardware/build:** TCL NXTPaper 11 Plus 9469X, APK
  `com.pdflector.app` 0.2.0, versionCode `16777473`, SHA-256
  `d86175be519d7fcff33646b6fa492ebda8a7f531958d19fc8b52f91217b19c14`.
- **Flujo observado:** al escribir sin zoom, el trazo se muestra durante el
  gesto. Con zoom alto, el trazo no se muestra hasta levantar el lápiz; tras
  levantarlo, la línea asentada coincide con el recorrido de la punta. El
  propietario también observó pequeños bordes en el trazo.
- **Límites:** no se anotaron página, orientación ni factor de zoom exacto; no
  se midieron latencia, refresco de frames durante el gesto ni presión. La
  alineación final observada no demuestra feedback continuo correcto con zoom
  ni permite atribuir los bordes a suavizado de trayectoria o antialiasing.

---

## 2026-09-26 — Revisión A4 por ADB durante ADR-011

- **Clasificación:** observación visual y muestras de memoria en TCL; no es
  benchmark de interacción continua.
- **Hardware/build:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36. APK
  `com.pdflector.app` 0.2.0, versionCode `16777473`, SHA-256
  `d86175be519d7fcff33646b6fa492ebda8a7f531958d19fc8b52f91217b19c14`.
- **Documento:** `dense_textbook.pdf` de la biblioteca privada de la app,
  SHA-256 `4a039e5c8127a3511d2bde331255c2a8222e478b7797ef4638458ca707a25e64`.
  Se leyó solo su metadato PDF en memoria: 93 páginas, tamaño
  `595.276 × 841.89 pt`, rotación 0, `MediaBox = CropBox = [0, 0, 595.28,
  841.89]` (A4).
- **Flujo:** filtrar la biblioteca por el nombre del corpus, abrirlo y capturar
  la vista en la página 25/93. La página se ve completa y centrada. El fondo y
  el papel son blancos, así que la captura no permite distinguir con precisión
  las cuatro esquinas del rectángulo; confirma el encuadre aparente, no una
  medición de sus esquinas. No se guardó la captura en el repositorio.
- **Pantalla:** ADB informó panel 1440×2200, densidad 320 dpi, rotación 0 y
  modo activo 60 Hz (`mActiveModeId=2`; 120 Hz también soportado). Durante el
  flujo las capturas de superficie pasaron de 2200×1440 a 1440×2200; se
  registra la discrepancia sin atribuirla a un cambio físico de orientación.
- **PSS crudo (KiB, misma sesión):** tras abrir, `387198` (RSS `510027`);
  tras 6 s `293630` (`416459`); tras 11 s `293618` (`416447`); tras 16 s
  `281890` (`404719`). Una muestra posterior a interacción fue `302228`
  (`425058`). El valor inicial supera el presupuesto de pico de 350 MiB y las
  muestras asentadas superan el objetivo estable de 250 MiB; la secuencia
  decreciente no muestra crecimiento monotónico en este intervalo.
- **Frames:** `SIN MEDIR` p95 válido para interacción continua. Hubo eventos
  `gl_present` puntuales durante apertura y navegación, pero no una secuencia
  controlada de 120 presents activos; no se usan como percentil ni como prueba
  del presupuesto de frame. No se midió lápiz→píxel.
- **Límites:** esto verifica visualmente un A4 en la tablet. No prueba
  selección/tinta, zoom 2, giro físico durante selección, ni página rotada o
  `CropBox` desplazado en el dispositivo.

---

## 2026-09-26 — Encuadre completo del PDF panorámico (ADR-011)

- **Clasificación:** observación funcional en la TCL más muestras puntuales de
  sistema; no constituye una validación de latencia ni cierra el ADR.
- **Hardware:** TCL NXTPaper 11 Plus, modelo 9469X, Android 16/API 36,
  1440×2200 px, densidad 320 dpi, dispositivo ADB `A06B4A8E6774623`.
  Pantalla encendida; batería 38 %, USB conectado. El panel admite 60/120 Hz y
  `dumpsys display` informó modo activo 60 Hz (`mActiveModeId=2`). Temperatura
  puntual al final: CPU/GPU 49,292 °C, SoC 49,475 °C, skin 44,066 °C,
  batería 25,8 °C, estado térmico 1 para skin y 0 para CPU/GPU/SoC. No se
  capturó temperatura inicial comparable.
- **Build:** `com.pdflector.app` 0.2.0, versionCode `16777473`, Android ARM64
  debug, instalada con `adb install -r` sobre la instalación existente.
  Fuentes basadas en `496f5127cbf47cc30f5beffffa25c470a09a71de` más cambios
  locales sin commit. APK SHA-256
  `d86175be519d7fcff33646b6fa492ebda8a7f531958d19fc8b52f91217b19c14`.
- **Flujo observado:** abrir la posición restaurada del documento privado
  `Guide_campus_virtual_26.pdf`, página 1, y avanzar a página 2/18 en portrait.
  La captura en tablet muestra los cuatro bordes de la diapositiva y el título
  completo «Come to our Library», texto y mapa. Es la misma página panorámica
  que antes aparecía recortada. No se guardó el PDF ni la captura en el
  repositorio. No se verificaron A4, stylus/selección, zoom 2, rotación de
  tablet ni la alineación de una página rotada/CropBox desplazado en pantalla.
- **Métricas crudas:** PSS total en tres muestras separadas por 4 s sobre la
  misma página después de instalar la APK final: `195984`, `195544`, `195544`
  KiB; RSS `311509`, `311065`, `311065` KiB. Son muestras puntuales después
  de navegación, no una serie de estrés ni prueba de estabilidad. Un ensayo de
  taps ADB con pausas en la versión previa informó p95 `62,8 ms` en 120
  presents; incluye tiempo ocioso, no representa frame p95 bajo interacción
  continua ni se compara con el presupuesto de 8,33 ms. No se midió lápiz→px
  ni porcentaje de frames perdidos.
- **Límite de cierre:** queda observada la corrección de encuadre en el caso
  concreto del ADR. Las cuatro esquinas en A4, la geometría rotada/desplazada
  con anotaciones, selección y tinta en el dispositivo y el giro de tablet
  siguen pendientes de comprobación.

---

## 2026-09-26 — Verificación funcional manual de la tinta integrada (ADR-010)

- **Clasificación:** verificación funcional declarada y confirmada por el
  propietario en la TCL NXTPaper 11 Plus 9469X, con `PdfLectorActivity` en
  primer plano. La sesión no recogió versión Android, versionCode instalada,
  estado térmico ni refresco efectivo. No es una medición de rendimiento.
- **Continuidad a zoom alto:** durante un trazo físico continuo la tinta ya
  aparece antes de levantar el lápiz. El propietario lo confirmó y el vídeo de
  pantalla lo muestra en el fotograma de aproximadamente 4 s:
  `/tmp/pdflector-continuidad.h264` (fotograma:
  `/tmp/pdflector-continuidad-frame100.png`).
- **Wet→dry:** el propietario informa que el trazo permaneció igual tras
  levantar el lápiz. La captura `/tmp/pdflector-wet-dry.h264` solo decodifica
  un fotograma y no permite comprobar visualmente la transición.
- **Interacción durante escritura y cancelación:** el propietario aclara que
  los contactos de dedos se ignoran mientras se escribe para no desplazar la
  hoja. En la secuencia de cancelación solicitada, el resultado reportado fue
  «la tinta permaneció o quedó anotación»; la captura
  `/tmp/pdflector-cancelacion.h264` solo tiene un fotograma y no permite
  separar tinta provisional de anotaciones ya asentadas. El propietario
  confirma que no hay una acción disponible para quitar tinta asentada; esa
  capacidad queda para trabajo posterior ya previsto.
- **Cambio de página durante el trazo y pérdida de superficie:** el propietario
  declara ambos flujos comprobados y conformes. No se obtuvo captura o log
  utilizable de esos dos casos en esta sesión, por lo que el registro conserva
  explícitamente su carácter de confirmación del propietario.
- **Cierre:** el propietario da por cumplida y verificada funcionalmente la
  integración de ADR-010. No se infieren latencia, refresco, estabilidad
  temporal ni cumplimiento de los presupuestos de `AGENTS.md`.

---

## 2026-09-26 — Resolución del fallo de proyección Kalman (ADR-010)

- **Alcance:** se cierra el fallo de
  `ink::tests::test_proyeccion_kalman_y_modulacion`. Esa verificación unitaria,
  por sí sola, no cerraba ADR-010; la verificación funcional manual posterior
  del mismo día queda registrada en la entrada anterior.
- **Diagnóstico confirmado:** en la propagación `F·P` de
  `KalmanFilter1D::update`, `fp12` usaba `p[2][1] * dt` donde la matriz exige
  `p[2][2] * dt`. El término omitía la varianza de aceleración al propagar la
  covarianza velocidad-aceleración. No había discrepancia de unidades ni del
  horizonte: el test alimenta puntos cada `0.004 s`, usa `500 pt/s` y el
  predictor rectilíneo conserva su horizonte de `0.030 s` (`15 pt`).
- **Cambio aplicado:** se corrigió únicamente ese índice en
  `crates/pdf_android/src/ink/kalman_predictor.rs`. No se cambió el test ni se
  relajó su esperado.
- **Reproducción anterior al fix:** en la TCL NXTPaper 11 Plus 9469X, el test
  Android ARM64 informó `lead: 9.47 pt vs esperado 15.00 pt` y falló.
- **Verificación posterior:** el mismo test, compilado para
  `aarch64-linux-android` y ejecutado por ADB en la TCL, pasó: `1 passed; 0
  failed; 0 ignored; 0 measured; 55 filtered out`. No se midieron latencia ni
  calidad visual del trazo en esta ejecución unitaria.
- **Suite de `pdf_core`:** `cargo test -p pdf_core` terminó con código 0:
  181 pruebas pasaron, 0 fallaron; 0 doctests.
- **Límite:** esta prueba unitaria, por sí sola, solo cierra el fallo de la
  proyección; no valida otros criterios funcionales de tinta. El cierre manual
  posterior de ADR-010 queda documentado en la entrada más reciente.

---

## 2026-09-26 — Alineación de tinta integrada en TCL, vertical y horizontal

- **Clasificación:** observación funcional manual; no es una medición de
  rendimiento.
- **Hardware:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36, ADB
  `A06B4A8E6774623`. Captura vertical 1440×2200 y horizontal 2200×1440.
  No se registraron temperatura ni condiciones de batería.
- **Build:** `com.pdflector.app`, versionCode `16777473`, firma igual a la
  instalación existente. Build candidata basada en `2b4d108`, con la
  corrección local de proyección de `InkGlRenderer.kt`, aún sin commit.
- **Flujo y resultado:** el propietario hizo trazos cortos en ambas
  orientaciones y confirmó que la tinta quedó alineada con la punta. Durante
  una lectura activa se observó 120 Hz; en reposo se observó 60 Hz. Son
  observaciones puntuales, no una serie de muestras.
- **Sin medir:** latencia lápiz→píxel, continuidad, frames perdidos, coste por
  evento, repintado nativo adicional, transición Wet→Dry, PSS y estado térmico.
  No se puede concluir sobre H2 ni H3 con esta prueba.
- **Limpieza de marcas:** tras la indicación del propietario de deshacerlas,
  una captura posterior no mostró el trazo de prueba en la página visible. El
  propietario cree que las marcas se deshicieron; no se confirmó una inspección
  completa de todas las páginas.

---

## 2026-09-24 — Integración de la APK principal: build e inicio, sin medición de tinta

- **Clasificación:** validación funcional parcial, no benchmark de latencia.
- **Hardware:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36,
  1440×2200, ADB `A06B4A8E6774623`. Batería 77 % y lectura puntual 28,2 °C;
  no se registró estado térmico inicial/final de un ensayo.
- **Build:** `com.pdflector.app` versionCode `16777473`, código basado en
  `6743699` con cambios locales sin commit; APK debug SHA-256
  `bb22b539030508c452c74033128f58f10e02f0e9d9ccc9ebc72da1cb7141f19e`.
  Firma SHA-256 `a1b691cffc1b8ed4897e708ba25a19bc4fc2c735871f9f57d67bbc6b13fe0969`,
  igual que la aplicación instalada anteriormente.
- **Flujo:** build Gradle con Rust release ARM64, `adb install -r`, arranque de
  `PdfLectorActivity`, inspección de logcat y de superficies. El primer
  arranque cayó por ausencia de tema AppCompat; tras declararlo, el proceso
  cargó el PDF previo y 37 anotaciones, y creó EGL/GLES2. La actualización
  mantuvo `/data/user/0/com.pdflector.app`.
- **Pantalla:** la actividad solicitó 120 Hz; `dumpsys display` mostraba modo
  activo 60 Hz (`mActiveModeId=2`) mientras la tablet estaba bloqueada. No es
  una medición del refresco durante escritura.
- **Resultado no medido:** posición del trazo en el PDF, continuidad,
  transición wet→dry, p50/p95/p99, lápiz→píxel, frames perdidos, PSS y uso
  térmico durante escritura. La pantalla bloqueada impidió la prueba física.

---

## 2026-09-23 — Corrección de alineación vertical en Ink Bench

- **Clasificación:** prueba funcional de coordenadas en dispositivo, no
  medición de latencia.
- **Hardware y build:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36,
  1440×2200; APK debug aislada `com.pdflector.inkbench` sobre HEAD
  `6743699a0d0e7129f57b9da8fe84f31034881b30` con cambios locales sin
  commit. APK corregida SHA-256
  `b6b765c4ebb5321f9def013351fa6b74ab63a0367fd57cdb40bff54c0d847144`.
- **Condiciones:** pantalla encendida y tablet conectada por ADB. La vista de
  control indicaba 60 Hz en la captura previa. No se registró una ventana
  térmica ni se midió el refresco durante el trazo corregido.
- **Hallazgo con lápiz físico:** el propietario movió la punta de izquierda a
  derecha y hacia arriba; la tinta avanzó hacia abajo. El log temporal de un
  `Down` mostró coordenadas locales/raw `(431,1111)`, origen de ambas vistas
  `(0,0)`, tamaño 1440×2200; AndroidX entregó buffer 1440×2200 y matriz de
  transformación identidad. La tinta apareció cerca de Y=1089, coherente
  con `2200−1111`: la proyección GL invertía Y una vez de más.
- **Regresión reproducible:** tras reiniciar la actividad, se inyectó
  `adb shell input stylus swipe 300 900 700 1300 500`. La captura previa
  mostraba tinta aproximadamente entre `(300,1300)` y `(700,900)`, y
  `tools/ink_bench/check_alignment.py` falló para ambos extremos de entrada.
  Se eliminó el signo negativo de Y en el vertex shader, se compilaron y
  pasaron 9 pruebas JVM, se instaló la APK corregida y se repitió exactamente
  el swipe. La comprobación de píxeles naranjas a 12 px de ambos extremos
  `(300,900)` y `(700,1300)` pasó.
- **Confirmación física:** con la APK corregida, el propietario probó el lápiz
  y confirmó que la tinta sale justo bajo la punta. Es una observación manual
  de alineación, no una medida de latencia ni un ensayo prolongado.
- **Límites:** la inyección ADB verifica coordenadas del trazo asentado, no
  latencia, presión, inclinación, rotación, continuidad en sesiones largas,
  FPS ni calidad bajo otros flujos.

---

## 2026-09-23 — Arranque del experimento AndroidX de tinta

- **Clasificación:** observación de instalación, superficies y memoria en
  reposo. No es una medición de rendimiento de escritura.
- **Hardware:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36, 1440×2200,
  dispositivo ADB `A06B4A8E6774623`.
- **Build:** APK debug aislada `com.pdflector.inkbench` 0.1, `versionCode=1`,
  construida sobre HEAD `6743699a0d0e7129f57b9da8fe84f31034881b30` con
  cambios locales sin commit. SHA-256 de la APK:
  `c3826bfadf373291c03bd5b194872e229e8c59ad72f3f7618b444b87636311ca`.
- **Condiciones:** pantalla encendida, batería 73 %, USB cargando. Batería
  29,6 °C según `dumpsys battery`; `thermalservice` informó skin 44,1 °C
  con estado 1. Son lecturas puntuales, no una ventana térmica estable.
- **Flujo:** instalación con ADB, arranque de `MainActivity`, captura de
  pantalla, consulta de SurfaceFlinger/display/meminfo, salida al launcher y
  regreso a la actividad. Se mantuvo el mismo proceso y no apareció un crash
  en el logcat filtrado durante esa secuencia.
- **Presentación:** la ventana solicitó 120 Hz y el `SurfaceView` solicitó
  120 Hz (`ExactOrMultiple`). SurfaceFlinger mostró un
  `FrontBufferedSurfaceControl` independiente. El modo físico efectivo siguió
  en 60 Hz (`mActiveModeId=2`, `renderFrameRate=60.0`). La cifra de 1000 Hz
  declarada por la capa front es una preferencia de esa capa, **no** el
  refresco físico del panel.
- **Memoria en reposo tras arranque de la APK final:** PSS total 62 096 KB;
  Graphics 14 589 KB; Native Heap 15 204 KB; RSS total 187 281 KB. Una sola muestra no
  demuestra estabilidad ni ausencia de crecimiento.
- **Límites:** contadores de input, callbacks y commits estaban a cero. No se
  ejecutó un trazo físico con stylus ni captura externa a ≥240 fps; no hay
  percentiles de latencia, frames perdidos o calidad visual que puedan
  aprobar la arquitectura. El resultado de 60 Hz tampoco acredita el objetivo
  de 120 Hz. La APK final incorporó la lectura de timestamps de MotionEvent
  en nanosegundos para API 34+; se reinstaló, abrió y permaneció sin crash
  observable.

---

## 2026-09-23 — Baseline previo al spike de tinta causal/front-buffer

- **Clasificación:** observado en hardware; todavía no es una medición de
  latencia lápiz→píxel ni una comparativa de motores.
- **Hardware:** TCL NXTPaper 11 Plus 9469X, Android 16/API 36, resolución
  1440×2200. Dispositivo ADB `A06B4A8E6774623`.
- **Build instalada:** `com.pdflector.app` 0.1.0, `versionCode=16777472`, APK
  SHA-256
  `303a92c6236b671d4f888ad9dc269fe6ac257b5b3465a13b2ee1632200733ebd`.
  La checkout estaba en `6743699a0d0e7129f57b9da8fe84f31034881b30` con
  cambios sin commit cuyo diff tenía SHA-256
  `9a46d0ef208acaf1ba8cfbc4707a6db0be57f07d3fd92019ab1b22d30d335ba0`.
- **Condiciones:** batería 73 %, alimentación USB activa; estado térmico del
  servicio `1`. Lecturas HAL: batería 30,7 °C y skin 33,8 °C. Las lecturas
  cacheadas eran superiores (skin 44,1 °C), por lo que esta sesión no se usa
  para comparar rendimiento.
- **Refresco:** el panel anuncia 120,00001 y 60 Hz. Con PDFLector visible y
  enfocado, SurfaceFlinger registró la solicitud de la capa como `120.00 Hz`,
  compatibilidad `Exact`, pero el modo físico efectivo permaneció en 60 Hz
  (`mActiveModeId=2`, `renderFrameRate=60.0`). La configuración de la ROM
  mostraba `ignore_app_preferred_refresh_rate_request=true`. Por tanto,
  **solicitar 120 Hz no demuestra que el panel esté funcionando a 120 Hz**.
- **Memoria tras arranque y 2 s de estabilización:** PSS total 176 445 KB;
  Graphics 78 462 KB; Native Heap 83 740 KB; RSS total 286 994 KB.
- **Validación host:** `git diff --check` correcto y toda la suite de
  `pdf_core` correcta. `cargo test -p pdf_android` en host no es una prueba
  válida: falló al compilar `ndk-sys`, que solo soporta targets Android. La
  validación Android debe usar el target `aarch64-linux-android` indicado en
  `AGENTS.md`.
- **Limitaciones:** no se ejecutó un gesto físico controlado, no se midió
  lápiz→píxel y no se modificó el ajuste persistente de refresco. Estas
  métricas quedan como `NO VERIFICADO` hasta ejecutar el protocolo con stylus
  y cámara externa de al menos 240 fps.

### Corpus fijado para las comparaciones

| Fichero | SHA-256 |
|---|---|
| `dense_textbook.pdf` | `e17d216c032b333a957b9f2f1af12edd5103b886172b6fedb72fbbeb6a11995f` |
| `large_document.pdf` | `f4c4397134a942efd9f5ab196540d5adec56855181b10b1b809cdb483a0b0f10` |
| `scanned_pages.pdf` | `4faf8fffab903214c2188d30459896071c7e1f2556c48e6718f08eca904e4943` |
| `scientific_paper.pdf` | `7d0cc80dca643efa7d0f10748c60b0780711a79023761f343ad79700acb33ac7` |

---

## 2026-09-07 — Batch TCL: Cold start, scroll de biblioteca y PSS bajo interacción

- **Hardware**: TCL NXTPaper 11 Plus (modelo 9469X, MediaTek MT8781 8× Cortex-A55, 8 GB RAM, Android 15, pantalla 1440×2200 portrait @ 320 dpi). Pantalla ON (`svc power stayon true`), verificado retorno a `stayon false` + Dozing al finalizar.
- **Build**: Release `f51d75c` (`pdf_android`, optimizaciones de velocidad de pase A–D).
- **Flujo medido**: Tres escenarios combinados en la tablet: cold start del visor restaurando página, scroll continuo en biblioteca y retención de memoria PSS en ráfaga de pases de página.

### 1. Primer frame cold-start (Viewer)
- **Método**: `am force-stop` + `logcat -c` + marcador COLDMARK + `am start`; medición hasta el primer evento `gl_present|blit`. Restaura libro de prueba *Análisis Funcional* (346 páginas, tipografía densa real) en pág. 61 (SepiaDark).
- **Métrica**: Mediana de 3 ejecuciones: **349 ms** (runs: 338, 355, 349 ms). **NO alcanza el objetivo <200 ms**.
- **Desglose run 1**:
  - Apertura del documento MuPDF (346 pp): 213 ms.
  - Inicialización de ventana nativa (`InitWindow`): 73 ms.
  - Primer `gl_present`: 52 ms (cálculo de presentación 14.1 ms, swap 2.8 ms).
- **Conclusión**: El cuello de botella reside en la apertura del PDF y la inicialización del sistema de ventanas, no en el pipeline de presentación GPU.

### 2. Scroll en biblioteca con 11 libros (E3 parcial)
- **Método**: Interacción continua con swipes en rejilla 3×3 y carousel. Medición de intervalos entre presents y tiempos de ejecución del frame.
- **Rejilla (N=535 presents en 14.1 s, 5+5 swipes)**:
  - Intervalo entre frames: mediana 8.0 ms, **p95 10.0 ms** (máximo intra-swipe 11.0 ms).
  - Coste `gl_present`: mediana 1.22 ms, p95 2.61 ms, máx 5.06 ms.
  - Swap time: mediana 0.83 ms, p95 2.20 ms.
  - Re-renders durante scroll: **0**.
- **Carousel "Seguir leyendo" (N=556 presents en 15.4 s)**:
  - Intervalo entre frames: mediana 8.0 ms, p95 10.0 ms. Coste present: mediana 1.19 ms, p95 2.58 ms.
- **Nota**: Se validó con 11 libros con overflow visual; la variante sintética de 256 portadas quedó pendiente por inviabilidad de carga manual vía UI sin harness específico.

### 3. PSS bajo interacción continua (15 pases de página)
- **Método**: 15 cambios de página consecutivos (págs. 60→75) sobre libro denso de 346 páginas. Muestreo de PSS mediante `dumpsys meminfo` en reposo y tras cada ráfaga.
- **Métricas**:
  - Latencia de turno: 15/15 completados (hits en caché: 5–8 ms; misses con render: 133–149 ms).
  - Evolución PSS total:
    - Base reposo: **234 713 KB** (~229.2 MB).
    - +5 turnos: 242 790 KB.
    - +10 turnos: 248 394 KB.
    - +15 turnos: **287 565 KB** (~280.8 MB).
    - Tras 20 s de quietud: 287 522 KB; tras 40 s: 287 518 KB (memoria asentada, sin fugas continuas).
  - Desglose final: Native Heap 176.0 MB, GL mtrack 68.7 MB, EGL mtrack 28.9 MB, Total RSS 416.6 MB.
  - Salto abrupto de +39.2 MB detectado entre turnos 11 y 15 atribuido a acumulación de display lists en MuPDF.

### 4. Menú Sheet lateral (E2)
- **Métrica**: Apertura completada en 13 presents (~130 ms, 1.04–4.06 ms c/u) con 1 creación de textura overlay 1440×924. Cierre en 13 presents (1.27–4.27 ms). **0 re-renderizados de página (`render page`), 0 desalojos de caché, 0 recreaciones de textura de página**.

---

## 2026-09-07 — Velocidad de pase de página y residencias de caché

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200). Pantalla ON.
- **Build**: Release `25a8dd7` (prefetch direccional, crop a ventana y desalojo diferido).
- **Flujo medido**: Taps secuenciales de cambio de página con pausas de 2.2 s sobre *dense_textbook.pdf* (93 pág) y *Análisis Funcional* (346 pág, tipografía real compleja).
- **Métricas**:
  - Baseline previo (sin crop a ventana, bitmap 27.4 MB en landscape): 12/12 turnos en fallo de caché a ~115 ms (0% hits por evicción inmediata en cada inserción).
  - Con crop a ventana (12.7 MB/página) y evicción diferida:
    - Serie de 15 turnos en *Análisis Funcional* (portrait): `9, 158, 8, 8, 155, 8, 6, 102, 7, 6, 6, 7, 102, 10, 10 ms`.
    - **11/15 turnos son hits en caché a 6–10 ms** (p50 ≈ 8 ms).
    - 4/15 turnos son misses con render completo a 102–168 ms.
    - Residencia mínima verificada: ≥3 páginas simultáneas en caché.
  - PSS observado durante la sesión: 190–205 MB.
- **Deuda detectada**:
  - Display lists de `MupdfDocument` acumulativas sin cota de evicción (+65 a +79 MB tras 22 turnos, ~6–7 MB/página nueva en documentos complejos).
  - 2 frames negros transitorios registrados en ~40 turnos (mitigados con guardas de fallback y validación de bitmap).

---

## 2026-09-06 — Presentación GPU Dry/Overlays y estabilidad de transiciones EGL

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Mali-G57 MC2, Android 16). Pantalla ON, batería al 7% en carga.
- **Build**: Release `f5381e9` (pipeline EGL productor único, surface persistente).
- **Flujo medido**: 10 ciclos consecutivos de transición Library ↔ Viewer, pan interactivo con stylus USI y medición de PSS en reposo.

### 1. Ciclos de transición Library ↔ Viewer (Estabilidad EGL)
- **Condición previa (build 1143e9e)**: `eglCreateWindowSurface` fallaba con error `0x3003` (`EGL_BAD_ALLOC`) en 7 de cada 10 transiciones debido a alternancia de productores (`ANativeWindow_lock` en CPU para biblioteca vs EGL en GPU para visor).
- **Resultado con productor único GPU (`f5381e9`)**:
  - **0 recreaciones de superficie EGL en 10 ciclos**.
  - **0 errores `EGL_BAD_ALLOC`** (0 fallos de superficie).
  - Coste de presentación de biblioteca por GPU: **4.6–6.7 ms** (frente a 4.2–19.4 ms en software).

### 2. Pan continuo sin re-rasterización (DryKey)
- **Métrica**: 459 frames de presentación durante arrastre continuo con stylus (`input stylus swipe`, tool_type=stylus):
  - **p50: 3.15 ms · p90: 3.64 ms · p95: 4.19 ms · máx: 17.90 ms**.
  - Re-renderizados de capa Dry durante el desplazamiento: **0** (`fbo create = 0`). El pan es pura traslación de quad con swap de buffers.

### 3. Memoria PSS
- **Métricas**:
  - Arranque: **118 MB**.
  - Pico tras 10 ciclos rápidos consecutivos: **232 MB**.
  - Reposo asentado tras interacción: **174–178 MB** (estabilizado, sin crecimiento monótono).

---

## 2026-09-05 — Subrayado y persistencia en hardware real

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200). Stylus físico USI 2.0.
- **Build**: Release `pdf_android`, arquitectura Dual FBO (Wet/Dry sobre GLES2/EGL).
- **Flujo medido**: Subrayado continuo interactivo con lápiz sobre texto real y guardado en SQLite.
- **Métricas**:
  - Presentación GPU durante el trazo (`gl_present`): **1.08–5.33 ms** (p50: 2.8 ms, **p95: 3.5 ms**).
  - Algoritmo de intersección de texto `highlight_under_gesture_sorted`:
    - Host x86 (Ryzen 7 5800H): **9.82 µs** (optimizado frente a 28.07 µs del baseline no ordenado).
    - Tablet TCL (MT8781): **< 0.1 ms**.
  - Persistencia SQLite (`save_annotations`): Ejecución asíncrona en hilo de fondo (**0 ms de bloqueo en hilo UI**).

---

## 2026-09-05 — Composición de anotaciones y StrokeCache

> **Nota histórica**: Código medido eliminado el 2026-09-18 (limpieza: compositor CPU sin consumidores de producción); la medición se conserva como registro.

- **Hardware**: AMD Ryzen 7 5800H (8C/16T), Linux release build. Benchmark criterion (`benches/composite.rs`) configurado a resolución nativa de la tablet TCL (1440×2200).
- **Flujo medido**: Fusión de capa de anotaciones sobre bitmap de página completa. Comparación entre rasterización euclidiana directa vs hit en `StrokeCache`.
- **Métricas**:

| Escenario (1440×2200) | Rasterización directa optimizada | Con `StrokeCache` (hit) | Speedup |
|---|---:|---:|---:|
| 10 trazos, 0 resaltados | 531 µs | — | Baseline |
| 50 trazos, 10 resaltados | 1.51 ms | — | — |
| 100 trazos, 100 resaltados | 3.67 ms | — | — |
| **200 trazos**, 0 resaltados | **4.39–4.55 ms** | **2.39 ms** | **2.2×** |

- **Notas de diseño**:
  - La optimización de distancia euclidiana en `draw_segment` descarta el cálculo de `sqrt()` en ~85% de los píxeles (núcleo y fondo mediante radios al cuadrado), bajando el tiempo directo de 5.40 ms a 4.39 ms (-18.5%).
  - `StrokeCache` retiene la capa rasterizada y reduce el coste por frame a **2.39 ms** mediante blit con aritmética entera.

---

## 2026-09-05 — Carga asíncrona de biblioteca (ThumbWorker)

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla 1440×2200).
- **Build**: Release `pdf_android`, actor MPSC `ThumbWorker` con `MupdfEngine` en hilo dedicado.
- **Flujo medido**: Apertura de biblioteca y scroll continuo en rejilla 3×3 con carga progresiva de portadas en segundo plano.
- **Métricas**:
  - Frame time de blit durante scroll (`blit 1440x2200`): **5.56–6.36 ms (p95: ~6.1 ms)**.
  - Bloqueo de E/S síncrona en hilo UI: **0 ms** (`try_recv` no bloqueante sobre canal).
  - Eliminado el límite arbitrario de 50 libros de la política de retención previa.

---

## 2026-09-04 — Barrido inicial TCL y consumo PSS de la aplicación

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, 8 GB RAM, Android 15 / SDK 36, pantalla 1440×2200 @ 320 dpi). Pantalla encendida con `stayon true`.
- **Build**: Release aarch64 (`crates/pdf_bench`), corpus de 4 documentos en `/data/local/tmp/pdflector/corpus`.
- **Flujo medido**: `tools/adb-bench.sh --runs 5` (mediana de páginas 0, central y final en cada ejecución) y arranque de la aplicación para medición de PSS con `dumpsys meminfo`.
- **Métricas de render por página**:

| Ejecución | dense (93p) 1x | scanned (30p) 1x | paper (12p) 1x | large (500p) 1x | RSS pico (KB) |
|:---:|---:|---:|---:|---:|---:|
| Run 1 | 12.61 ms | 35.95 ms | 11.52 ms | 14.87 ms | 27 088 |
| Run 2 | 14.05 ms | 38.50 ms | 11.36 ms | 14.96 ms | 26 984 |
| Run 3 | 13.22 ms | 33.45 ms | 12.29 ms | 14.35 ms | 27 128 |
| Run 4 | 13.14 ms | 33.57 ms | 12.10 ms | 14.63 ms | 27 172 |
| Run 5 | 15.18 ms | 33.46 ms | 11.75 ms | 14.40 ms | 27 452 |

- **Memoria de la app real**:
  - Arranque en biblioteca: **PSS 110 352 KB (~107.8 MB)** (Native Heap 51.9 MB, Graphics 47.3 MB, Code 4.3 MB; RSS 235 189 KB).
  - Tras 130 pases de página por tap (195 presents): **PSS 208 882 KB (~204 MB)**. El incremento se debe a la retención de texturas Wet/Dry y búferes EGL acumulados.

---

## 2026-09-03 — Verificación en tablet de Display Lists retenidas

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Android 15).
- **Flujo medido**: Barrido `pdf_bench` a escala 2x tras incorporar display lists retenidas en `MupdfDocument`.
- **Métricas**:
  - *large_document.pdf* (2x): 70.35 ms → **68.73 ms**.
  - *dense_textbook.pdf* (2x): 69.54 ms → **66.89 ms**.
  - Conclusión: Variación dentro del margen de ruido térmico; sin regresión observable frente a la ejecución base.

---

## 2026-08-30 — Validación de experiencia física con stylus USI 2.0

- **Hardware**: TCL NXTPaper 11 Plus (9469X, pantalla 1440×2200), Stylus USI 2.0 con botón físico. Build release `pdf_android`.
- **Flujo medido**: Pruebas manuales e instrumentadas de dibujo, borrado por hardware y latencia percibida.
- **Resultados**:
  - **Goma por botón físico (`BTN_STYLUS2` 0x20)**: Transición inmediata a borrado al pulsar (`erase: stroke 10 -> 3 pieces`). Retorno a modo tinta al soltar sin parpadeo.
  - **Shader de tinta GPU**: Corregido `FS_INK_SRC` a premultiplied alpha (`vec4(rgb * a, a)`), eliminando desaturaciones de contraste en fondos claros y oscuros.
  - **Transición sin flashes**: `FS_OVERLAY_SRC` premultiplicado por `uAlpha` eliminó destellos blancos en cambios de estado.
  - **Cero-Pop**: La polilínea simplificada persistida converge con el trazo en vivo (`simplify_polyline` 0.35 pt).
  - **Latencia percibida vs App Nativa (TCL Notes)**:
    - PDFLector presenta a 60 Hz vía `eglSwapBuffers` (tiempo de frame ~1.5–4.0 ms, latencia total del pipeline ~16–30 ms).
    - La app propietaria de TCL recurre a rendering directo en front-buffer (<10 ms), perceptiblemente más inmediata debido a los límites de paso por SurfaceFlinger en apps estándar de Android.

---

## 2026-08-30 — Display Lists vs Re-parse completo en MuPDF

- **Hardware**: AMD Ryzen 7 5800H (8C/16T), Linux release build.
- **Flujo medido**: Renderizado mediante display list retenida en memoria (`fz_run_display_list`) frente a re-parse vectorial completo (`Page::to_pixmap`) por escala.
- **Métricas**:

| Documento | Escala | Re-parse base | Display List | Speedup |
|---|:---:|---:|---:|---:|
| large_document.pdf | 2× | 2.04 ms | 1.13 ms | **1.81×** |
| large_document.pdf | 4× | 5.76 ms | 4.51 ms | **1.28×** |
| dense_textbook.pdf | 2× | 2.33 ms | 1.42 ms | **1.64×** |
| dense_textbook.pdf | 4× | 6.96 ms | 5.15 ms | **1.35×** |
| scientific_paper.pdf | 2× | 3.10 ms | 1.84 ms | **1.68×** |

- **Conclusión**: En la escala habitual de pinch (2x) la display list acelera el render entre 1.6× y 1.8×. A 4x domina el tiempo de rasterizado de píxeles sobre la interpretación del árbol vectorial.

---

## 2026-08-28/29 — Migración a presentación EGL/GLES2 en Viewer

- **Hardware**: TCL NXTPaper 11 Plus (9469X, Mali-G57 MC2, Android 15). APK release aarch64.
- **Flujo medido**: Reemplazo del pipeline `ANativeWindow_lock` + `memcpy` de software por `eglSwapBuffers` con texturas GPU. Trazos generados mediante `input stylus swipe`.
- **Métricas**:
  - `gl_present` con trazo activo (n=1768): **p50: 4.55 ms · p90: 8.64 ms · p95: 10.00 ms** · p99: 13.56 ms · máx: 26.92 ms.
  - Frames aislados > 16.6 ms: 4 de 1768 (0.23%), todos en el rango 16.69–26.92 ms, nunca en ráfagas consecutivas.
  - Pase de página (doc 442 páginas, n=27): **p50: 5.65 ms · p95: 11.64 ms**.
  - PSS durante dibujo continuo: 116–126 MB.
  - PSS en biblioteca (tras liberar textura de página): **71.4 MB**.
  - PSS pico en documento complejo OCR (442 páginas): **152.6 MB** (GL mtrack 49.6 MB, EGL 24.8 MB, Heap nativo 62.8 MB).

---

## 2026-08-27 — Trazo de tinta con History batching y Bézier punto medio

- **Hardware**: TCL NXTPaper 11 Plus (9469X, Android 15). Release aarch64.
- **Flujo medido**: Captura de eventos con `input stylus swipe`. Muestreo de tiempos de dibujo por dirty rect en `draw.rs`.
- **Métricas**:
  - Blit durante el gesto (n=156): **p50: 4.80 ms · p95: 5.78 ms · máx: 7.46 ms**.
  - Frames superiores a 16.6 ms en toda la sesión: **0 de 156**.
  - Tamaño de dirty rects procesados: 13×42 a 53×94 px (solo el incremento diferencial del trazo).
  - PSS a lo largo de la sesión: 110 MB en arranque → 145.6 MB estable tras más de 20 trazos y cambios de página.

---

## 2026-08-24 — PageTextCache en hardware real

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781, Android 15).
- **Flujo medido**: Extracción de texto de página (`stext`) y consulta en `PageTextCache` (LRU 512 páginas).
- **Métricas**:

| Documento | Extracción fría pág 0 | Extracción fría intermedia | Miss de caché pág 10 | Hit de caché pág 10 | Prefetch 20 págs |
|---|---:|---:|---:|---:|---:|
| dense_textbook (93p) | **9.5 ms** | 2.3 / 0.25 ms | 2.4 ms | **0.000 ms** | 48 ms total |
| scientific_paper (12p) | **17.1 ms** | 0.5 / 0.4 ms | 0.38 ms | **0.000 ms** | — |

- **Conclusión**: La primera extracción de texto requiere entre 9 y 17 ms en la CPU de la tablet. Con `PageTextCache`, los accesos subsiguientes para subrayado toman 0.000 ms en el hilo UI.

---

## 2026-08-24 — Benchmark de resaltado y composición en escritorio

> **Nota histórica**: Código medido de composición CPU (`composite_annotations`) eliminado el 2026-09-18 (limpieza: compositor CPU sin consumidores de producción); la medición se conserva como registro.

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Linux release build.
- **Flujo medido**: `cargo bench -p pdf_bench` para algoritmos de selección y mezcla.
- **Métricas**:
  - Intersección de resaltado (`highlight_under_gesture`):
    - 10 puntos / 20 líneas: ~1.3 µs.
    - 50 puntos / 100 líneas: ~9.6 µs.
    - 100 puntos / 200 líneas: ~36 µs.
    - Dos columnas (200 líneas): ~20 µs.
  - Fusión de anotaciones (`composite_annotations` a 1440×2200):
    - 10 trazos: 0.64 ms.
    - 50 trazos + 10 resaltados: 2.14 ms.
    - 200 trazos: **6.8 ms**.

---

## 2026-08-24 — Corrección de canal alfa en trazo en tiempo real

> **Nota histórica**: Mecanismo de composición CPU (`composite_annotations`, `composite_annotations_alpha`) sustituido por pipeline GPU (ADR-007) y eliminado el 2026-09-18; la medición se conserva como registro.

- **Hardware**: TCL NXTPaper 11 Plus (9469X).
- **Flujo medido**: Captura visual (screencap) durante el trazo de stylus con la herramienta Boli activa.
- **Diagnóstico previo**: El trazo en curso era invisible durante el arrastre porque `composite_annotations` generaba un mapa con alfa 0, omitiendo el pintado en `copy_region_blend`.
- **Métricas con `composite_annotations_alpha`**:
  - Píxeles visibles a mitad de gesto: de **0 px** a **928 px** (coincidencia geométrica exacta con el dedo/lápiz).
  - Píxeles al soltar el trazo: **2205 px**.
  - Coste de `blit_composed`: 4–8 ms por frame durante la interacción.

---

## 2026-08-22 — Optimización de primitivas de blit y prefetch de páginas

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux, release build.
- **Flujo medido**: Microbenchmarks de primitivas de copia en `pdf_bench` (espejo de `draw.rs` a 2000×1200) y prefetch en ráfaga de navegación.
- **Métricas de Blit (resolución 2000×1200, mediana)**:

| Operación | Baseline previo | Con optimización | Reducción |
|---|---:|---:|:---:|
| `blit/page_1to1_bpp4_dark` | 1.376 ms | **154 µs** | **−89 %** |
| `blit/page_zoom135_bpp4_light` | 1.292 ms | **727 µs** | **−44 %** |
| `blit/page_zoom135_bpp4_dark` | 2.391 ms | **556 µs** | **−77 %** |
| `blit/page_1to1_bpp4_light` | 332 µs | 368 µs | Sin cambio (ruido) |
| `blit/page_1to1_bpp2 (RGB565)` | 1.153 ms | 1.167 ms | Sin cambio |

- **Optimizaciones clave**:
  - Inversión de modo oscuro en bpp 4 convertida de iteración byte a byte a operación XOR en u32 (`val ^ 0x00FF_FFFF`), permitiendo vectorización automática del compilador.
  - Acceso directo en escala por vecino más cercano sin comprobaciones de límites redundantes por píxel en slice.
- **Prefetch preemptivo (`prefetch.rs`)**:
  - En una ráfaga de 10 viewports no solapados de 11 páginas cada uno, las páginas efectivamente procesadas cayeron de 110 a **22 páginas (−80%)** al descartar solicitudes obsoletas en vuelo. Tiempo hasta residencia del viewport final: 35.8 ms.

---

## 2026-08-13 — Zoom en tablet TCL: Escala software vs Re-renderizado

- **Hardware**: TCL NXTPaper 11 Plus (9469X, MT8781 8× A55, Android 15, pantalla encendida, 33 °C). Release aarch64.
- **Flujo medido**: Comparación entre escalado por software (`scale_bitmap`, nearest-neighbor en CPU) y re-renderizado vectorial nativo con MuPDF.
- **Métricas (mediana de 3 ejecuciones)**:

| Escenario | Escalado software (`scale_bitmap`) | Re-renderizado vectorial (MuPDF) | Ratio de penalización SW |
|---|---:|---:|:---:|
| large_document pág 0 → 2× | 69.4–70.2 ms | **14.9–16.6 ms** | ~4.5× más lento |
| large_document pág 0 → 4× | 275.8–281.5 ms | **53.2–56.1 ms** | ~5.2× más lento |
| dense_textbook pág 0 → 2× | 69.9 ms | **16.1–16.9 ms** | ~4.3× más lento |
| dense_textbook pág 0 → 4× | 321.4–325.1 ms | **57.3–59.4 ms** | ~5.6× más lento |

- **Decisión arquitectónica**: El reescalado software en CPU sin extensiones SIMD/NEON es prohibitivo para interactividad en la tablet. El zoom inmediato debe realizarse mediante texturas en GPU, seguido de un re-renderizado asíncrono nítido en segundo plano.

---

## 2026-08-13 — Zoom en escritorio: Escala software vs Re-renderizado

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux.
- **Flujo medido**: `crates/pdf_bench/benches/zoom.rs` sobre *large_document.pdf* (página 0).
- **Métricas**:
  - `scale_bitmap` a 2×: 55.9 ms.
  - `scale_bitmap` a 4×: 214.5–219.9 ms.
  - Re-render nativo MuPDF a nivel 1 (2×): **3.3–3.4 ms**.
  - Re-render nativo MuPDF a nivel 2 (4×): **11.9–12.5 ms**.
  - Conclusión idéntica a la tablet: el re-render vectorial es entre 16× y 18× más rápido en CPU que el remuestreo naïve por píxeles.

---

## 2026-08-12 — Sweep inicial en tablet TCL NXTPaper 11 Plus

- **Hardware**: TCL NXTPaper 11 Plus (modelo 9469X, MediaTek MT8781 8× Cortex-A55, 8 GB RAM, Android 15, pantalla 1440×2200 @ 320 dpi). Pantalla encendida con `stayon true`.
- **Build**: Binario `pdf_bench` release compilado para `aarch64-linux-android` (NDK r28 + sysroot). Motor MuPDF.
- **Flujo medido**: Barrido de apertura y renderizado completo a 1x (72 dpi) y 2x (144 dpi) sobre los 4 documentos del corpus.
- **Métricas**:

| Documento (páginas) | open (ms) | render 1x (ms) | render 2x (ms) |
|---|---:|---:|---:|
| dense_textbook (93p) | 0.40 | 14.51 | 44.18 |
| scanned_pages (30p) | 0.15 | 31.34 | 119.01 |
| scientific_paper (12p) | 0.16 | 11.64 | 38.44 |
| large_document (500p) | 0.25 | 15.40 | 44.73 |

- **Memoria**: RSS pico de **26 688 KB (~26.7 MB)**.
- **Ausencia de shootout en tablet**: En la tablet TCL **únicamente se ejecutó MuPDF**. No existe un benchmark comparativo de motores en Android (`android-tablet-shootout.md` nunca existió ni fue medido).

---

## 2026-08-10 — Baseline de referencia: Evince (poppler) en escritorio

Esta medición establece el comportamiento de referencia de un visor estándar de escritorio basado en Poppler + Cairo, sirviendo como evidencia empírica para la toma de decisiones en ADR-003.

- **Hardware**: AMD Ryzen 7 5800H (8C/16T, hasta 4.47 GHz), 13 GiB RAM, Linux 7.1.4-arch1-1 (Wayland/Hyprland).
- **Software**: Evince 48.4, Poppler 26.07.0.
- **Corpus**: `corpus/large_document.pdf` (500 páginas A4, 543 kB, texto vectorial puro).
- **Método de captura**: `tools/bench-evince/bench_evince.sh` utilizando `pdftoppm` (comparte exactamente el pipeline monohilo de renderizado de Evince: poppler + cairo, página completa a la escala solicitada).

### 1. Renderizado monohilo de página completa (Poppler)

| Escala | Píxeles por página (A4) | Tiempo total 500 págs (3 repeticiones) | Tiempo medio por página | RSS máximo del proceso |
|---|:---:|:---:|---:|---:|
| 72 dpi (1×) | 595 × 842 | 36.72 / 36.84 / 37.84 s | **73.6 ms** | 22.5 MB |
| 144 dpi (2×) | 1190 × 1684 | 164.36 / 162.06 s | **326.0 ms** | 28.0 MB |

### 2. Apertura inicial y render de primera página (incluye parseo del documento)

| Escala | Tiempo (3 repeticiones) | Mediana |
|---|:---:|---:|
| 144 dpi (2×) | 0.42 / 0.36 / 0.35 s | **~0.36 s** |
| 216 dpi (3×) | 0.60 / 0.60 / 0.60 s | **0.60 s** |

### 3. Consumo RSS del visor gráfico Evince 48.4 (500 páginas)

| Estado | Consumo RSS |
|---|---:|
| Ventana abierta en página 1 tras 8 s (arranque en frío) | **197 920 kB** (~193.3 MB) |
| Reapertura con caché de disco caliente (8 s) | **197 952 kB** (~193.3 MB) |

### Deducciones clave para el proyecto
1. **La escala cuadruplica el coste de rasterizado**: Pasar de 1× (72 dpi) a 2× (144 dpi) eleva el renderizado de 73.6 ms a 326.0 ms por página. A resolución nativa de lectura, el renderizado en vivo durante un frame de scroll provocaría caídas intolerables de fluidez (~3 fps).
2. **Evince mantiene memoria acotada en render**: Su proceso libera buffers tras pintar (RSS máx 28 MB), pero el visor gráfico completo supera los 197 MB de RSS debido al entorno GTK/GL y la estructura interna del documento.

---

## 2026-08-05 — Comparativa de motores en escritorio (PDFium vs MuPDF) y discrepancia metodológica

- **Hardware**: AMD Ryzen 7 5800H (8C/16T, 3.2–4.4 GHz), 16 GB RAM, Arch Linux (kernel 6.16). Compilación release en Rust 1.97.1.
- **Corpus**: 4 documentos estándar (`paper_12p`, `scanned_30p`, `dense_93p`, `large_500p`).

### La discrepancia metodológica no resuelta
Existen dos conjuntos de mediciones independientes realizados en la misma máquina host durante la Fase 0.5 que arrojaron resultados diametralmente opuestos:

1. **Sweep simple de `pdf_bench` (mediana de 3 corridas en páginas 0, central y final)**:
   - Reportó a **MuPDF como ganador indiscutible**: entre 2.7× y 4× más rápido en renderizado y un 21% menos de consumo de memoria pico.
   - Estos datos fueron la base tomada en cuenta para la redacción y aprobación de **ADR-001**.
2. **Benchmark exhaustivo Criterion (`crates/pdf_bench/benches/engine_shootout.rs`)**:
   - Con un tamaño de muestra N=100 tras 3 segundos de calentamiento por caso, **PDFium resultó ganador en 14 de las 16 pruebas de renderizado (87.5%)**, mostrando ser típicamente 2× a 4× más veloz.
   - PDFium también superó a MuPDF en 3 de las 4 pruebas de apertura de archivo.

Esta divergencia nunca fue aclarada formalmente en su momento (atribuible a diferencias entre corridas monohilo aisladas sin warm-up frente a la saturación de bucle cerrado con optimización de caché de Criterion, y a variaciones en flags de compilación de las bibliotecas estáticas C).

**Decisión del dueño del proyecto**: A pesar de la ventaja demostrada por PDFium en el benchmark Criterion en escritorio, **se mantiene la decisión de ADR-001 en favor de MuPDF**. Los motivos determinantes son:
- Distribución autocontenida: MuPDF se compila como biblioteca estática unificada dentro del binario Rust (6.48 MB totales), eliminando la dependencia de binarios dinámicos pesados y fragmentados como `libpdfium.so` (12.53 MB en disco).
- Ergonomía de integración de tipos C bajo `mupdf-sys` y previsibilidad de licencias y compilación cruzada hacia Android NDK.
- En la tablet TCL solo se desplegó y midió MuPDF; no se llegó a realizar una comparativa en hardware móvil.

A continuación se transcriben los datos brutos de ambas mediciones:

### A. Resultados del Sweep Simple (`pdf_bench`) — Base de ADR-001

| Documento (páginas) | Motor | Apertura (ms) | Render 1× (ms) | Render 2× (ms) | RSS pico (KB) |
|---|---|---:|---:|---:|---:|
| dense_textbook (93p) | PDFium | 0.17 | 9.69 | 35.34 | 32 520 |
| dense_textbook (93p) | MuPDF | **0.11** | **3.53** | **8.51** | **25 572** |
| scanned_pages (30p) | PDFium | 0.09 | 20.01 | 66.20 | 32 520 |
| scanned_pages (30p) | MuPDF | **0.07** | **8.93** | **35.38** | **25 572** |
| scientific_paper (12p) | PDFium | 0.08 | **1.72** | 26.44 | 32 520 |
| scientific_paper (12p) | MuPDF | **0.07** | 2.18 | **6.95** | **25 572** |
| large_document (500p) | PDFium | 0.21 | 6.86 | 35.10 | 32 520 |
| large_document (500p) | MuPDF | **0.09** | **3.98** | **10.19** | **25 572** |

### B. Resultados del Shootout Criterion (N=100)

#### Apertura de documento (mediana en microsegundos)
| Documento | Páginas | PDFium (µs) | MuPDF (µs) | Ratio (PDFium / MuPDF) |
|---|---:|---:|---:|:---:|
| scientific_paper | 12 | 54.98 | 55.67 | 0.99× (Empate) |
| scanned_pages | 30 | **56.70** | 70.26 | **1.24× (PDFium)** |
| dense_textbook | 93 | **82.37** | 245.80 | **2.98× (PDFium)** |
| large_document | 500 | **186.03** | 384.32 | **2.07× (PDFium)** |

#### Renderizado de página completa (mediana en milisegundos)
| Documento | Página | Escala | PDFium (ms) | MuPDF (ms) | Ratio y Ganador |
|---|---|:---:|---:|---:|:---:|
| paper_12p | p1 | 1× | **0.502** | 1.752 | **3.49× (PDFium)** |
| paper_12p | p_mitad | 1× | **0.556** | 5.806 | **10.45× (PDFium)** |
| paper_12p | p1 | 2× | **10.477** | 23.087 | **2.20× (PDFium)** |
| paper_12p | p_mitad | 2× | **10.448** | 27.967 | **2.68× (PDFium)** |
| scanned_30p | p1 | 1× | **3.304** | 11.918 | **3.61× (PDFium)** |
| scanned_30p | p_mitad | 1× | **6.299** | 7.299 | **1.16× (PDFium)** |
| scanned_30p | p1 | 2× | **14.457** | 58.994 | **4.08× (PDFium)** |
| scanned_30p | p_mitad | 2× | **18.051** | 28.591 | **1.58× (PDFium)** |
| dense_93p | p1 | 1× | 2.614 | **1.725** | **0.66× (MuPDF)** |
| dense_93p | p_mitad | 1× | **3.459** | 4.521 | **1.31× (PDFium)** |
| dense_93p | p1 | 2× | **20.533** | 54.127 | **2.64× (PDFium)** |
| dense_93p | p_mitad | 2× | **13.450** | 47.979 | **3.57× (PDFium)** |
| large_500p | p1 | 1× | **2.699** | 9.950 | **3.69× (PDFium)** |
| large_500p | p_mitad | 1× | **2.917** | 5.373 | **1.84× (PDFium)** |
| large_500p | p1 | 2× | 33.263 | **30.487** | **0.92× (MuPDF)** |
| large_500p | p_mitad | 2× | **13.749** | 18.160 | **1.32× (PDFium)** |

#### Tamaño de artefacto resultante
| Motor | Binario ejecutable | Bibliotecas dinámicas externas | Espacio total en disco |
|---|---:|---:|---:|
| PDFium | 4.86 MB | `libpdfium.so` 7.67 MB | **12.53 MB** |
| MuPDF | **6.48 MB** | 0 MB (enlace estático) | **6.48 MB** |

---

## 2026-08-05 — Eficiencia de Caché LRU vs Retención Ingenua

- **Hardware**: AMD Ryzen 7 5800H (16 hilos), Arch Linux release build.
- **Flujo medido**: `crates/pdf_bench/benches/cache_scroll.rs` sobre 50 páginas de `large_document.pdf` a escala 1× (72 dpi) con MuPDF. Comparación entre retención arbitraria de bitmaps y ventana acotada a 8 MB.
- **Métricas**:

| Escenario | Tiempo total (ms) | Pico de memoria VMHWM (KB) |
|---|---:|---:|
| `naive_hold_50p_1x` (retener 50 páginas) | 108.02 ms | 107 412 KB (~105 MB) |
| `cache_8mb_firstpass_50p_1x` (primera pasada) | 74.78 ms | **21 104 KB (~20.6 MB)** |
| `cache_8mb_pass2_50p_1x` (hit sobre residentes) | **0.35 ms** | 21 184 KB (~20.7 MB) |

- **Conclusión**: La caché LRU limitada a 8 MB reduce la memoria RAM pico en un factor de 5× (de 105 MB a 20.6 MB). El camino de acierto en caché insume 0.35 ms frente a los 74 ms del recorrido con renderizado.
