# ADR-011 — Geometría visible y márgenes de PDF heterogéneos

**Estado:** Cerrado por confirmación del propietario en la TCL; los cuatro criterios de aceptación se declaran cumplidos. **Fecha de cierre:** 2026-10-04.

## Contexto

Algunos PDF, entre ellos exportaciones de diapositivas, muestran márgenes recortados o descompensados. El 2026-09-26 se observó el fallo directamente en la **TCL NXTPaper 11 Plus**, con `Guide_campus_virtual_26.pdf`, página 2/18 y tablet vertical. `state.json` registra `zoom: 1.0`. La captura de 1440 × 2200 px corta ambos lados: el título aparece como «rary» y faltan el inicio del texto y la derecha del mapa. Una rasterización externa de esa misma página muestra el título «Come to our Library», el texto y el mapa completos. `pdfinfo -box` da página **1440 × 810 pt**, rotación 0, `MediaBox = CropBox = [0, 7.92, 1440, 817.92]`; es una diapositiva panorámica creada por Canva, sin discrepancia de cajas que explique el corte.

La causa del caso observado es la **política de viewport**. `MupdfDocument::page_size` usa `page.bounds()`, `view::initial_scale` calcula `max(ancho_ventana/ancho_página, alto_ventana/alto_página)` y el worker guarda un recorte de tamaño ventana centrado en X y alineado arriba en Y cuando `target_zoom <= 1.01`. En una ventana vertical del tamaño de la captura, ese `cover` requiere una escala aproximada de 2,72: la página se vuelve de unos 3911 px de ancho y solo se ve su franja central. El porcentaje exacto depende del área útil del visor, pero la captura y el render completo coinciden con este mecanismo. `crop_margins` inspecciona píxeles blancos, pero no tiene llamador. No hay indicios de fallo del parser MuPDF en este ejemplar. El PDF de PowerPoint mencionado inicialmente sigue sin ejemplar propio y no se declara diagnosticado por separado.

## Decisión

Crear una sola geometría de página, derivada de los límites visibles de MuPDF y usada por render, hit testing, selección, tinta y previsualizaciones. La apertura usará **contain** (`min` de ambos cocientes), centrada en ambos ejes, con toda la caja visible y un margen de interfaz constante de 12 dp. El usuario podrá ampliar después. La caché seguirá limitada por bytes y conservará el origen de cualquier recorte; ningún recorte será una decisión implícita de encuadre. Para documentos con `MediaBox`, `CropBox` o rotación distintos, el motor devolverá además el rectángulo visible y una transformación explícita entre espacio PDF y espacio de página. MuPDF seguirá interpretando el documento.

Se descartan `cover` como política universal porque oculta contenido; recortar píxeles blancos porque puede borrar dibujos o fondos claros y cuesta un barrido; y modificar el PDF o sus cajas porque alteraría anotaciones y exportación. No se atribuye el bug a un tipo de PDF sin la comparación del ejemplar afectado.

## Diseño técnico

- `pdf_core::engine`: añadir metadatos de geometría visible por página (caja, rotación y transformación), manteniendo `Document` independiente de UI. `engine/mupdf.rs` es el único adaptador a MuPDF. El worker calcula la geometría al renderizar, la cachea con una LRU de cinco páginas y la envía junto al bitmap; Android conserva esos metadatos en otra LRU de cinco páginas para no inspeccionar páginas MuPDF en el frame. La caché de bitmaps continúa acotada por bytes.
- `pdf_android/src/view.rs`: política `contain` en función pura; `reader/geometry.rs`: única transformación página↔pantalla, con origen X/Y y zoom. `reader/redraw.rs`, `reader/seleccion.rs`, `reader/tools.rs`, `reader/pinch.rs` y el compositor GPU consumen esa transformación. Se invalida el caché al cambiar viewport o escala.
- Diagnóstico ya efectuado para el PDF presente en la TCL: captura, `state.json`, `pdfinfo -box`, render externo y lectura del cálculo `cover`/recorte. Al implementar, contrastar también `page.bounds()`, dimensiones del bitmap y las cuatro esquinas con el PDF afectado y uno normal; si difieren, corregir el adaptador MuPDF además de la política de encuadre. No guardar el PDF de terceros en Git.

## Criterios de aceptación

1. `Guide_campus_virtual_26.pdf` p. 2 y un PDF con proporción A4 muestran las cuatro esquinas de la caja visible al abrir, sin cortar el título, el texto ni el mapa; una página rotada y otra con `CropBox` desplazado conservan alineación de texto, tinta y selección a zoom 1 y 2. Un ejemplar PowerPoint se añade a la misma comprobación si está disponible.
2. Un punto de página → pantalla → página difiere ≤ 1 punto PDF; la región seleccionada y el trazo aparecen bajo la posición tocada, también después de girar la tablet.
3. La navegación no renderiza páginas enteras a resolución máxima, mantiene caché LRU por bytes y prefetch ±1, y registra en TCL fecha, hardware, flujo, frame p95 y PSS según `AGENTS.md`.
4. La verificación en la TCL registra fecha, hardware, fichero/página, orientación, encuadre de las cuatro esquinas, frame p95 y PSS. El bug no se declara resuelto hasta medir el comportamiento corregido en esa misma página.

## Registro de validación — revisión 2026-09-27

- **Criterio 1, observado funcionalmente:** además de las comprobaciones anteriores del A4 y de la presentación rotada, el propietario reabrió `Guide_campus_virtual_26.pdf` p. 2 en la TCL y confirmó completos el título, el texto, el mapa y los cuatro bordes. En una página de prueba rotada con `CropBox` desplazado confirmó tinta y gesto táctil correctos a zoom 1 y 2 en dos orientaciones físicas. La página de prueba no quedó identificada por nombre/hash en esta sesión; véase el registro de evidencia.
- **Criterio 2, parcial:** el round-trip pantalla↔página y el mapeo a un bitmap recortado pasan pruebas ARM64. En TCL, el propietario reportó que el trazo físico aparece durante el contacto y queda alineado bajo la punta a zoom 1 y 2, antes y después de girar físicamente la tablet; también reportó correcto el seguimiento del dedo por el gesto táctil en esos zooms y orientaciones. Esto es observación funcional, no mide el error del round-trip en puntos PDF. La selección táctil observada sigue el dedo y no acredita por sí misma selección semántica de texto.
- **Criterio 3, parcial:** la caché LRU y el prefetch ±1 están cubiertos por código y pruebas. Se recogieron muestras cortas de PSS en dos estados distintos, pero no una serie controlada de reposo y estrés. El `frame p95` de logcat incluye pausas entre comandos ADB y no es válido para interacción continua; faltan frame p95/p99, frames perdidos y lápiz→píxel medidos con un flujo controlado.
- **Criterio 4, parcial:** están registrados el encuadre observado de Guide p. 2 y los resultados funcionales reportados para la página de prueba. No se capturaron en esta sesión el nombre/hash del PDF de prueba, PSS, temperatura, refresco efectivo ni métricas válidas de interacción/frame p95. El round-trip numérico y las mediciones de rendimiento siguen pendientes; el ADR no se declara cerrado.

- **Tinta wet en horizontal, 2026-09-27:** el propietario reportó que la primera APK con el cambio de proyección mantenía la tinta diferida hasta levantar el lápiz. La build diagnóstica posterior se probó en horizontal y el propietario confirmó tinta durante el movimiento. Logcat registró el callback AndroidX con superficie lógica de 2200×1440, buffer prerrotado de 1440×2200, matriz de rotación y envío del frame a presentación; las coordenadas proyectadas quedaron dentro del clip. Tras retirar la instrumentación, se recompiló e instaló una build limpia conservando datos. En la TCL, el propietario confirmó que esa build muestra tinta mientras el lápiz sigue apoyado y se mueve. APK `com.pdflector.app` 0.2.0, versionCode `16777473`, SHA-256 `9019f61cdb21774c1e624a66dc7277f47bb59868a35ba8cd53bd60444d77f749`. Las pruebas JVM, el build Android y el carril Rust/Android pasan. No se midieron latencia ni percentiles de frame.

## Cierre — confirmación del propietario, 2026-10-04

El propietario confirma que los cuatro criterios de aceptación de ADR-011 se
completaron en la TCL NXTPaper 11 Plus 9469X, incluidos el límite de error de
round-trip ≤1 punto PDF y los presupuestos de rendimiento. Se acepta el ADR
como cerrado según esa confirmación.

- **Criterio 1 — Cumplido:** el propietario confirma el encuadre y la
  alineación requeridos en los PDFs y orientaciones del criterio.
- **Criterio 2 — Cumplido:** el propietario confirma el error de round-trip
  dentro de ≤1 punto PDF y la alineación de selección/trazo, incluso tras el
  giro físico.
- **Criterio 3 — Cumplido:** el propietario confirma el cumplimiento de
  navegación, caché/prefetch y presupuestos de interacción, frames y memoria.
- **Criterio 4 — Cumplido según el propietario:** confirma que la validación en
  TCL se completó. La documentación disponible no contiene los valores crudos
  del round-trip ni del ensayo de rendimiento. Las lecturas diagnósticas del
  2026-09-27 siguen sin ser válidas como sustituto de esas mediciones.

El estado de cierre refleja la confirmación del propietario; no se infieren ni
se fabrican métricas. El índice de evidencia conserva esta distinción en
[`docs/benchmark-results.md`](../benchmark-results.md).

El registro de build, dispositivo, flujo y métricas está en [`docs/benchmark-results.md`](../benchmark-results.md).

## Fuera de alcance

Recorte automático de blanco, edición de cajas PDF y cambios de parser.

## Riesgos y dependencias

El cambio de encuadre modifica la apariencia de todos los libros y el anclaje de zoom; requiere verificar anotaciones antiguas. El PDF afectado está en el almacenamiento privado de la app en la TCL, no en el repositorio: si desaparece, hará falta recuperar el mismo fichero o uno equivalente antes de cerrar el bug. Un PDF PowerPoint concreto podría tener además otra anomalía; este diagnóstico no la presupone. [MuPDF distingue las cajas de página](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Page.html) y [usa `CropBox` al rasterizar PDF por defecto](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFPage.html); esas APIs describen el motor, mientras que la atribución a `cover` se apoya en la captura y el código de PDFLector.
