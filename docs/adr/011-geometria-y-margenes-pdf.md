# ADR-011 — Geometría visible y márgenes de PDF heterogéneos

**Estado:** Decisión documentada; causa confirmada para el caso observado en la TCL; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

Algunos PDF, entre ellos exportaciones de diapositivas, muestran márgenes recortados o descompensados. El 2026-09-26 se observó el fallo directamente en la **TCL NXTPaper 11 Plus**, con `Guide_campus_virtual_26.pdf`, página 2/18 y tablet vertical. `state.json` registra `zoom: 1.0`. La captura de 1440 × 2200 px corta ambos lados: el título aparece como «rary» y faltan el inicio del texto y la derecha del mapa. Una rasterización externa de esa misma página muestra el título «Come to our Library», el texto y el mapa completos. `pdfinfo -box` da página **1440 × 810 pt**, rotación 0, `MediaBox = CropBox = [0, 7.92, 1440, 817.92]`; es una diapositiva panorámica creada por Canva, sin discrepancia de cajas que explique el corte.

La causa del caso observado es la **política de viewport**. `MupdfDocument::page_size` usa `page.bounds()`, `view::initial_scale` calcula `max(ancho_ventana/ancho_página, alto_ventana/alto_página)` y el worker guarda un recorte de tamaño ventana centrado en X y alineado arriba en Y cuando `target_zoom <= 1.01`. En una ventana vertical del tamaño de la captura, ese `cover` requiere una escala aproximada de 2,72: la página se vuelve de unos 3911 px de ancho y solo se ve su franja central. El porcentaje exacto depende del área útil del visor, pero la captura y el render completo coinciden con este mecanismo. `crop_margins` inspecciona píxeles blancos, pero no tiene llamador. No hay indicios de fallo del parser MuPDF en este ejemplar. El PDF de PowerPoint mencionado inicialmente sigue sin ejemplar propio y no se declara diagnosticado por separado.

## Decisión

Crear una sola geometría de página, derivada de los límites visibles de MuPDF y usada por render, hit testing, selección, tinta y previsualizaciones. La apertura usará **contain** (`min` de ambos cocientes), centrada en ambos ejes, con toda la caja visible y un margen de interfaz constante de 12 dp. El usuario podrá ampliar después. La caché seguirá limitada por bytes y conservará el origen de cualquier recorte; ningún recorte será una decisión implícita de encuadre. Para documentos con `MediaBox`, `CropBox` o rotación distintos, el motor devolverá además el rectángulo visible y una transformación explícita entre espacio PDF y espacio de página. MuPDF seguirá interpretando el documento.

Se descartan `cover` como política universal porque oculta contenido; recortar píxeles blancos porque puede borrar dibujos o fondos claros y cuesta un barrido; y modificar el PDF o sus cajas porque alteraría anotaciones y exportación. No se atribuye el bug a un tipo de PDF sin la comparación del ejemplar afectado.

## Diseño técnico

- `pdf_core::engine`: añadir metadatos de geometría visible por página (caja, rotación y transformación), manteniendo `Document` independiente de UI. `engine/mupdf.rs` es el único adaptador a MuPDF. La geometría se calcula fuera del frame y se cachea por página.
- `pdf_android/src/view.rs`: política `contain` en función pura; `reader/geometry.rs`: única transformación página↔pantalla, con origen X/Y y zoom. `reader/redraw.rs`, `reader/seleccion.rs`, `reader/tools.rs`, `reader/pinch.rs` y el compositor GPU consumen esa transformación. Se invalida el caché al cambiar viewport o escala.
- Diagnóstico ya efectuado para el PDF presente en la TCL: captura, `state.json`, `pdfinfo -box`, render externo y lectura del cálculo `cover`/recorte. Al implementar, contrastar también `page.bounds()`, dimensiones del bitmap y las cuatro esquinas con el PDF afectado y uno normal; si difieren, corregir el adaptador MuPDF además de la política de encuadre. No guardar el PDF de terceros en Git.

## Criterios de aceptación

1. `Guide_campus_virtual_26.pdf` p. 2 y un PDF con proporción A4 muestran las cuatro esquinas de la caja visible al abrir, sin cortar el título, el texto ni el mapa; una página rotada y otra con `CropBox` desplazado conservan alineación de texto, tinta y selección a zoom 1 y 2. Un ejemplar PowerPoint se añade a la misma comprobación si está disponible.
2. Un punto de página → pantalla → página difiere ≤ 1 punto PDF; la región seleccionada y el trazo aparecen bajo la posición tocada, también después de girar la tablet.
3. La navegación no renderiza páginas enteras a resolución máxima, mantiene caché LRU por bytes y prefetch ±1, y registra en TCL fecha, hardware, flujo, frame p95 y PSS según `AGENTS.md`.
4. La verificación en la TCL registra fecha, hardware, fichero/página, orientación, encuadre de las cuatro esquinas, frame p95 y PSS. El bug no se declara resuelto hasta medir el comportamiento corregido en esa misma página.

## Fuera de alcance

Recorte automático de blanco, edición de cajas PDF y cambios de parser.

## Riesgos y dependencias

El cambio de encuadre modifica la apariencia de todos los libros y el anclaje de zoom; requiere verificar anotaciones antiguas. El PDF afectado está en el almacenamiento privado de la app en la TCL, no en el repositorio: si desaparece, hará falta recuperar el mismo fichero o uno equivalente antes de cerrar el bug. Un PDF PowerPoint concreto podría tener además otra anomalía; este diagnóstico no la presupone. [MuPDF distingue las cajas de página](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Page.html) y [usa `CropBox` al rasterizar PDF por defecto](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFPage.html); esas APIs describen el motor, mientras que la atribución a `cover` se apoya en la captura y el código de PDFLector.
