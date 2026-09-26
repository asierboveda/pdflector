# ADR-020 — Abrir referencias a figuras en una vista previa

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

El motor actual extrae texto por líneas y renderiza páginas, pero no expone enlaces internos ni un índice de leyendas. Algunos PDF enlazan «Fig. 2» a una página; otros dejan la referencia como texto plano. Un tap ordinario cambia de página, por lo que el hit test de una referencia debe preceder a la navegación.

## Decisión

Hacer activables referencias a figuras con destino **verificable**. Prioridad: enlace interno del PDF cuyo origen coincide con una referencia textual; si no existe, índice local de leyendas `Fig./Figure/Figura + número` y coincidencia única de identificador. Si dos leyendas coinciden o la extracción falla, la referencia no se activa y se informa al mantenerla pulsada; nunca se abre una figura posiblemente incorrecta. El destino se muestra como recorte de la figura y su leyenda en un popup pequeño sobre la página actual, con ampliar/cerrar; no altera página, zoom ni posición de lectura. Los enlaces externos mantienen su tratamiento separado.

Se descarta un modelo de IA para descubrir figuras: introduciría red, latencia y falsos vínculos en una tarea de navegación local. Se descarta ir a la página destino y volver: pierde contexto y no cumple el popup pedido.

## Diseño técnico

- `pdf_core/src/engine.rs` y `engine/mupdf.rs`: exponer enlaces internos con rect de origen, página destino y posición; MuPDF 0.8 dispone de `PdfPage::resolved_links`. Añadir a la extracción índices de palabras con cajas para hit testing de `Fig. N` sin activar toda la línea.
- `pdf_core/src/figures.rs`: índice bajo demanda de leyendas por documento. Vincular identificador normalizado a una única `{página, rect_leyenda, rect_figura?}`; elegir gráfico/imagen contiguo sobre la leyenda si el motor ofrece caja, y si no, mostrar la página destino ajustable dentro del popup. Construcción incremental en worker, cancelable al cerrar documento, con límite de memoria; no bloquear el tap ni indexar todo el PDF en el hilo UI.
- `pdf_core::engine`: operación de render de región con escala acotada para popup. `reader/figures.rs` solicita ≤ 1024 × 1024 px; `gpu` mantiene un caché separado ≤ 8 MiB. `input/gestos.rs` resuelve primero popup y enlaces, luego tap de página. `draw/overlays.rs` coloca popup de hasta 45 % del ancho y 40 % del alto de ventana, con botón Ampliar para leer una figura densa. Los rectángulos pasan por la geometría de ADR-011.

## Criterios de aceptación

1. En un PDF con enlace interno a figura y otro con referencia de texto sin enlace pero leyenda única, tocar el identificador abre la figura o la página destino con leyenda en popup; la página y el zoom originales no cambian al cerrar.
2. Una referencia ambigua, externa o sin leyenda verificable no abre un destino incorrecto. Tocar fuera del popup lo cierra sin pasar página.
3. El popup permite ampliar y desplazar su contenido, conserva nitidez suficiente para leer la leyenda y no deja una textura residente tras cerrarse; caché ≤ 8 MiB.
4. En TCL se registran fecha, hardware, flujo de apertura de 20 figuras, latencia tap→popup, frame p95 y PSS; el render y el índice no bloquean el hilo UI.

## Fuera de alcance

OCR de PDF escaneados, referencias a tablas/ecuaciones y navegación web de enlaces externos.

## Riesgos y dependencias

Depende de ADR-011 para coordenadas; no depende del modo Scroll. La correspondencia leyenda→imagen puede fallar en figuras vectoriales o maquetación a dos columnas: el fallback es página destino ajustable, no un recorte inventado. [MuPDF documenta enlaces con bounds y página/URI](https://mupdf.readthedocs.io/en/1.28.5/reference/javascript/types/Page.html); el binding MuPDF 0.8 instalado para este repositorio se inspeccionó localmente y ofrece `resolved_links`.
