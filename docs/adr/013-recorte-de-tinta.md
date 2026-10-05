# ADR-013 — Seleccionar, mover y escalar tinta manuscrita

**Estado:** Implementada; validación en la TCL pendiente. **Fecha:** 2026-09-26; activación y deshacer actualizados el 2026-10-05.

## Contexto

La «herramienta de recorte» solicitada actúa sobre escritura manuscrita añadida por el usuario; no recorta imágenes del PDF ni reconoce texto. Cada trazo actual es un `Stroke` con puntos, grosor, color e ID en `AnnotationSet`, persistido en el sidecar SQLite. El borrado puede fragmentar trazos. Mover píxeles del bitmap rompería zoom, exportación y persistencia.

## Decisión

Usar un lazo cerrado dibujado con lápiz para elegir trazos **completos** de la página actual; mostrar una caja con asa de escala uniforme. Arrastrar desplaza y el asa escala entre 0,5× y 2×, siempre dentro de la misma página. Selección y vista previa son transitorias; al soltar se transforman los puntos y el ancho de cada `Stroke` en una única operación atómica que conserva ID, orden y color. Para limitar la intención a ajustes pequeños, el desplazamiento acumulado de una operación se acota a 25 % del menor lado de la página; el usuario puede repetirla. Si algún trazo saldría de la caja visible, se clampa toda la transformación sin deformar el grupo.

**Activación (2026-10-05, ver [ADR-022](022-barra-acoplable.md)):** el botón **Recorte** vive en la barra de herramientas del visor, junto a Bolígrafo, Subrayador y Goma, y es ahí donde debe estar. Con Recorte elegido, el lápiz dibuja el lazo. Además, **mantener pulsado el botón superior del lápiz** y dibujar con él hace el lazo, con la barra abierta o cerrada. Con trazos seleccionados, el lápiz arrastra la caja para mover o su asa inferior derecha para escalar; tocar fuera de la caja descarta la selección sin dibujar. Cada movimiento o escala confirmado es una acción del historial de deshacer/rehacer de la sesión.

Se descarta seleccionar por caja de texto extraído: la tinta no tiene texto semántico. Se descarta copiar/pegar o rasterizar el grupo: añadiría duplicados y perdería calidad vectorial.

## Diseño técnico

- `pdf_core/src/annotations.rs`: añadir búsqueda espacial de `Stroke` contra polígono y transformación afín uniforme sobre una lista de IDs, con resultado `Result` que rechaza IDs ausentes, página distinta, puntos no finitos o escala fuera del rango. La operación conserva los ID y reemplaza los trazos en una transacción lógica; devuelve la instantánea anterior, que el historial de deshacer de la sesión (`pdf_android/src/undo.rs`) usa para deshacer y rehacer.
- `pdf_android/src/reader/recorte.rs`: máquina `Lazo → Seleccionado → Mover/Escalar → Confirmar/Cancelar`; convierte muestras a puntos de página mediante la geometría de ADR-011. El lazo y la caja viven en overlay, no en el sidecar.
- `input/motion.rs` enruta solo stylus a esta herramienta; un contacto de palma se ignora y dos dedos intencionados tras levantar el lápiz permiten zoom. `reader/anotaciones.rs` guarda una vez al confirmar por el worker de persistencia; `gpu/dry_key.rs` invalida dry únicamente tras el commit. Se usan los trazos originales para vista previa, sin escribir por cada Move.

## Criterios de aceptación

1. Un lazo que intersecta dos trazos selecciona exactamente esos dos; un lazo vacío no modifica anotaciones. PDF, highlights y notas quedan intactos.
2. Un movimiento o escala válidos preservan número de trazos, ID, orden, color y página; tras cerrar y abrir el libro reaparecen en las coordenadas nuevas. Cancelar o perder la superficie conserva las antiguas.
3. Un movimiento que excede 25 % o un escalado fuera de 0,5–2× queda limitado visiblemente; ningún punto ni grosor sale inválido. Deshacer recupera las coordenadas previas en una sola acción.
4. Tinta y selección coinciden a zoom 1 y 2, con margen de error ≤ 1 punto PDF. En TCL se miden fecha, hardware, flujo de 200 trazos, frame p95 y PSS.

## Fuera de alcance

OCR, mover contenido original del PDF, selección entre páginas, rotación, duplicado y selección parcial de un trazo.

## Riesgos y dependencias

Depende del menú de ADR-012 y la geometría de ADR-011. La persistencia actual reescribe el conjunto de anotaciones en cada `save`; la implementación debe confirmar fuera del hilo de UI y comprobar que el orden no cambia. La cota de 200 trazos requiere medición real, no una promesa de rendimiento.
