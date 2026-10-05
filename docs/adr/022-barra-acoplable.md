# ADR-022 — Barra de herramientas acoplable y botones del lápiz

**Estado:** Implementada; validación en la TCL pendiente. **Fecha:** 2026-10-05.
**Sustituye en parte a:** [ADR-012](012-menu-de-anotacion.md) (posición de la
barra, editor de color/grosor, deshacer/rehacer y función del botón superior
del lápiz). El resto de ADR-012 sigue vigente.

## Contexto

ADR-012 fijó una barra horizontal sobre el borde inferior con Bolígrafo,
Subrayador, Goma y Recorte, `Navegar` al cerrarla, el botón superior del lápiz
alternando Bolígrafo/Subrayador y el color/grosor fuera de alcance. Al
implementarla, el propietario pidió poder colocar la barra donde le estorbe
menos mientras lee, elegir color y grosor sin botones extra y usar el botón
superior del lápiz para seleccionar tinta (ADR-013).

## Decisión

- **Posición:** la barra se acopla a cualquiera de los cuatro bordes de la
  ventana, centrada en él: vertical en los laterales, horizontal arriba y
  abajo. Una pulsación larga (dedo o lápiz) sobre la barra permite
  arrastrarla; al soltar se acopla al borde más cercano al punto. Arriba y
  abajo deja libre el espacio del chrome del visor. El borde se persiste en
  `tool_state.json`; sin valor guardado, izquierda.
- **Contenido:** Bolígrafo, Subrayador, Goma y Recorte, solo con icono y área
  táctil de 48 dp | Deshacer, Rehacer | Cerrar.
- **Color y grosor:** no tienen botón propio. Tocar de nuevo el Bolígrafo
  activo abre un popover con cinco colores y tres grosores; tocar de nuevo el
  Subrayador activo abre sus cinco colores. El popover se abre hacia el
  interior de la ventana y un tap fuera lo cierra sin dibujar.
- **Deshacer/rehacer:** historial en memoria por documento, acotado a 100
  acciones (trazo, subrayado, pasada de goma, mover/escalar de Recorte).
- **Cerrar (`Navegar`, como ADR-012):** el lápiz navega como el dedo y la
  herramienta elegida se conserva.
- **Botones físicos del lápiz**, con la barra abierta o cerrada:
  - inferior mantenido: borra;
  - superior mantenido: dibuja el lazo de Recorte (ADR-013). Ya no alterna
    Bolígrafo/Subrayador.

Se descarta la barra fija abajo de ADR-012: en apaisado tapa la última línea
de lectura y no se puede apartar. Se descartan botones dedicados de color y
grosor: alargan la barra para una acción ocasional.

## Diseño técnico

- `pdf_android/src/draw/toolbar.rs`: geometría pura compartida por dibujo y
  hit-test (`toolbar_layout`, `toolbar_popover_layout`, `ToolbarDock::nearest`)
  y bitmaps Canvas+JNI regenerados solo al cambiar el estado de la barra.
- `pdf_android/src/reader/toolbar.rs`: taps, popover, abrir/cerrar, arrastre y
  persistencia.
- `pdf_android/src/input/motion.rs`: `ToolbarPress` → `ToolbarDrag` tras
  `LONG_PRESS_MS`; un Down sobre la barra o con el popover abierto nunca inicia
  trazo ni selección. Enrutado del lápiz: selección de Recorte activa → botón
  inferior → botón superior → herramienta de la barra (si está abierta) →
  navegación.
- `pdf_android/src/undo.rs`: ediciones con anotaciones quitadas, añadidas y
  modificadas en su sitio; remapeo de ids reasignados.

## Criterios de aceptación

1. La barra se arrastra a los cuatro bordes en ambas orientaciones, queda
   centrada, no tapa el chrome del visor y recuerda el borde tras reiniciar.
2. Tocar el Bolígrafo o el Subrayador activos abre su popover; elegir color o
   grosor se aplica al siguiente trazo y se persiste.
3. Con la barra cerrada, el lápiz pasa página y no dibuja; el botón inferior
   borra y el superior selecciona.
4. Los taps en la barra o el popover no dibujan ni pasan página.
5. En la TCL se registran fecha, hardware, flujo, frame p95 y PSS.

## Riesgos

El arrastre comparte `LONG_PRESS_MS` con la selección de texto: una pulsación
larga sobre la barra nunca selecciona texto. Cambiar el botón superior rompe
la costumbre de alternar Bolígrafo/Subrayador con el lápiz.
