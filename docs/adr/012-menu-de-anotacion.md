# ADR-012 — Menú de anotación del visor

**Estado:** Implementada con cambios; sustituida en parte por [ADR-022](022-barra-acoplable.md) (posición, color/grosor, deshacer/rehacer y botón superior del lápiz). **Fecha:** 2026-09-26.

## Contexto

El producto ya tiene tinta, subrayado y goma vectoriales, pero el control del lápiz está repartido entre `PenMode`, botones físicos, `ToolKind`, gestos, barra y menús de selección. La imagen adjunta se toma como referencia visual del control segmentado, no como especificación de colores ni de texto. El menú flotante de selección (`Copiar`, `Subrayar`, `IA`) y el sheet de ajustes tienen fines distintos. Unificar sus estados por etiquetas de botones haría ambiguo el despacho de gestos.

## Decisión

Establecer una barra de anotación propia con cuatro acciones mutuamente excluyentes: **Bolígrafo, Subrayador, Goma y Recorte**. `Navegar` es el estado al cerrar la barra. La goma reutiliza el borrado vectorial existente; Recorte activa la selección de tinta de ADR-013. La herramienta elegida gobierna el siguiente gesto del lápiz; el botón superior físico cambia entre Bolígrafo y Subrayador y el inferior activa Goma solo mientras se mantiene pulsado, restaurando después la elección previa. Seleccionar Recorte y pulsar el botón superior sale de Recorte. El dedo navega y nunca dibuja. El control segmentado **Tap / Scroll** de lectura se alojará junto a esta barra, pero su estado pertenece al visor y se define en ADR-014; al cambiar de herramienta no cambia el modo de lectura.

Se descarta convertir el menú de selección de texto en barra permanente: mezclaría acciones sobre texto con herramientas de dibujo. Se descarta replicar tinta en otro modelo: `AnnotationSet` y la capa dry/wet siguen siendo la fuente de verdad.

## Diseño técnico

- `pdf_android/src/annotations.rs`: definir el estado de herramienta activo, incluidas Goma y Recorte, con una única transición de activación/cierre. Migrar `PenMode` al estado elegido por la barra, preservando el último Bolígrafo/Subrayador para el botón físico. Persistir la selección en `tool_state.json` con valor seguro Bolígrafo si falta el campo; `Navegar` rige al cerrar la barra y no cambia esa preferencia guardada.
- `reader/sheet_chrome.rs` y `reader/mod.rs`: acciones tipadas de la barra; cancelar limpiamente el gesto anterior antes del cambio. `draw/chrome.rs` dibuja una barra flotante sobre el borde inferior seguro, con los cuatro iconos en una fila y estado activo de alto contraste; `reader/geometry.rs` aporta rectángulos compartidos entre dibujo y hit testing; `input/gestos.rs` despacha por acción, no por texto visible. Los colores salen de la paleta existente.
- `input/motion.rs` conserva separación dedo/lápiz y rechazo de palma. En `Down` de stylus toma una instantánea de la herramienta elegida y aplica la excepción del botón inferior; no la cambia a mitad de trazo. Los estados de borrado y recorte invocan sus operaciones sobre coordenadas de página. Una superposición abierta captura primero los eventos y no permite pasar un tap a cambio de página.

## Criterios de aceptación

1. Se muestran cuatro herramientas con nombre e icono reconocible, área táctil ≥ 48 dp, indicador inequívoco de selección y navegación disponible al cerrar.
2. Bolígrafo, Subrayador y Goma producen exactamente una operación sobre el modelo vectorial por gesto; Recorte entra en el flujo definido por ADR-013 una vez implementado.
3. Cambiar herramienta durante un gesto lo cancela sin guardar un trazo incompleto; el botón superior alterna Bolígrafo/Subrayador, el inferior borra temporalmente y el dedo no pinta por accidente.
4. Los taps en barra, menú contextual o control segmentado no avanzan página; el estado visual y el hit test comparten la misma geometría.
5. En TCL se registran fecha, hardware, flujo de cambio de herramienta, frame p95 y PSS.

## Fuera de alcance

Editor de color/grosor, rediseño del menú de selección de texto y lógica interna de Recorte o Scroll.

## Riesgos y dependencias

Cambiar la prioridad de eventos puede reintroducir dobles taps o falsos gestos. ADR-013 y ADR-014 dependen de este contrato de UI; este ADR se puede implementar solo con Recorte deshabilitado hasta que ADR-013 lo complete, sin presentar una herramienta inoperante.
