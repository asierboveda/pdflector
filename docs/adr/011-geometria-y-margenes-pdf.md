# ADR-011 — Geometría visible y márgenes de PDF heterogéneos

**Estado:** Decisión documentada; causa del caso concreto sin confirmar. **Fecha:** 2026-09-26.

## Contexto

Algunos PDF, entre ellos exportaciones de capturas de PowerPoint, muestran márgenes recortados o descompensados. No se adjuntó un ejemplar reproducible: la causa concreta sigue **SIN CONFIRMAR**. El código sí permite acotar la hipótesis. `MupdfDocument::page_size` usa `page.bounds()`, `view::initial_scale` calcula `max(ancho_ventana/ancho_página, alto_ventana/alto_página)` y el worker guarda un recorte de tamaño ventana centrado en X y alineado arriba en Y. Ese `cover` elimina contenido cuando las proporciones difieren. `crop_margins` inspecciona píxeles blancos, pero no tiene llamador; no explica el fallo actual. El parser MuPDF no es el sospechoso principal mientras los límites, el render y el texto coincidan.

## Decisión

Crear una sola geometría de página, derivada de los límites visibles de MuPDF y usada por render, hit testing, selección, tinta y previsualizaciones. La apertura usará **contain** (`min` de ambos cocientes), centrada en ambos ejes, con toda la caja visible y un margen de interfaz constante de 12 dp. El usuario podrá ampliar después. La caché seguirá limitada por bytes y conservará el origen de cualquier recorte; ningún recorte será una decisión implícita de encuadre. Para documentos con `MediaBox`, `CropBox` o rotación distintos, el motor devolverá además el rectángulo visible y una transformación explícita entre espacio PDF y espacio de página. MuPDF seguirá interpretando el documento.

Se descartan `cover` como política universal porque oculta contenido; recortar píxeles blancos porque puede borrar dibujos o fondos claros y cuesta un barrido; y modificar el PDF o sus cajas porque alteraría anotaciones y exportación. No se atribuye el bug a un tipo de PDF sin la comparación del ejemplar afectado.

## Diseño técnico

- `pdf_core::engine`: añadir metadatos de geometría visible por página (caja, rotación y transformación), manteniendo `Document` independiente de UI. `engine/mupdf.rs` es el único adaptador a MuPDF. La geometría se calcula fuera del frame y se cachea por página.
- `pdf_android/src/view.rs`: política `contain` en función pura; `reader/geometry.rs`: única transformación página↔pantalla, con origen X/Y y zoom. `reader/redraw.rs`, `reader/seleccion.rs`, `reader/tools.rs`, `reader/pinch.rs` y el compositor GPU consumen esa transformación. Se invalida el caché al cambiar viewport o escala.
- Diagnóstico previo de implementación: comparar en un PDF afectado y otro normal las cajas PDF, `page.bounds()`, dimensiones del bitmap, rectángulo mostrado y coordenadas de una marca en las cuatro esquinas. Si `bounds` y bitmap discrepan, corregir el adaptador MuPDF antes de aplicar la política de encuadre.

## Criterios de aceptación

1. El caso PowerPoint aportado y un PDF con proporción A4 muestran las cuatro esquinas de la caja visible al abrir, sin cortar contenido; una página rotada y otra con `CropBox` desplazado conservan alineación de texto, tinta y selección a zoom 1 y 2.
2. Un punto de página → pantalla → página difiere ≤ 1 punto PDF; la región seleccionada y el trazo aparecen bajo la posición tocada, también después de girar la tablet.
3. La navegación no renderiza páginas enteras a resolución máxima, mantiene caché LRU por bytes y prefetch ±1, y registra en TCL fecha, hardware, flujo, frame p95 y PSS según `AGENTS.md`.
4. El diagnóstico deja constancia de cuál de caja, transformación, escala o recorte provocaba el caso real; sin PDF reproducible no se declara resuelto el bug.

## Fuera de alcance

Recorte automático de blanco, edición de cajas PDF y cambios de parser.

## Riesgos y dependencias

El cambio de encuadre modifica la apariencia de todos los libros y el anclaje de zoom; requiere verificar anotaciones antiguas. Depende de obtener un PDF afectado antes de cerrar el bug. [MuPDF distingue las cajas de página](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Page.html) y [usa `CropBox` al rasterizar PDF por defecto](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/PDFPage.html); las APIs citadas son documentación del motor, no prueba del fallo de este repositorio.
