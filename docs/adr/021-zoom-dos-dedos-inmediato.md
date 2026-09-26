# ADR-021 — Respuesta inmediata del zoom con dos dedos

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

El pinch ya existe: `input/motion.rs` entra en `GestureKind::Pinch` con el segundo dedo; `reader/pinch.rs::set_zoom_fast` escala el bitmap residente y `set_zoom_sharp` pide render nítido al worker al soltar. La clasificación `Undecided/Pan/Zoom` retrasa el inicio visible hasta superar umbrales; además, el crop a ventana en zoom base puede carecer de píxeles al ampliar hacia un borde. «Instantáneo» no debe describirse como función ausente, sino como contrato de latencia y continuidad a comprobar en la TCL.

## Decisión

Mantener dos fases: transformación GPU inmediata del bitmap residente durante cada Move y render nítido asíncrono por worker. Desde el primer Move cuya variación de distancia supera el ruido táctil calibrado (objetivo inicial: 1 % de distancia o 2 dp, lo que sea mayor), aplicar zoom anclado al centro; el movimiento paralelo de ambos dedos aplica pan sin impedir que después empiece zoom. Al cambiar de modo, reanclar distancia y posición para que no haya salto. La política de caché garantiza que el preview solo use píxeles disponibles; zonas recién visibles muestran color de papel hasta que llegue un crop adecuado, sin estirar un borde falso.

Se descarta renderizar MuPDF sincrónicamente en cada Move: bloquearía UI y aumentaría memoria. Se descarta predicción de gesto: la página debe seguir muestras reales, igual que la tinta causal aprobada.

## Diseño técnico

- `input/motion.rs`: clasificar con delta acumulado en dp y razón de distancias; actualizar ancla solo en transiciones de modo. `reader/pinch.rs` consume la geometría única de ADR-011 y solicita repintado dentro del ciclo de presentación, sin espera de I/O.
- `reader/redraw.rs` y `cache.rs`: el preview distingue bitmap completo de crop residente mediante `full_w/full_h/crop_x/crop_y`; el worker pide regiones visibles nuevas y una resolución final dentro del presupuesto de bytes. Se mantiene prefetch ±1 y se descartan respuestas de secuencia obsoleta.
- Instrumentar timestamp de `MotionEvent`, primer present que cambia escala y llegada del render nítido. Medir tasa de refresco efectiva de la TCL; no inferir latencia lápiz/píxel ni dedo/píxel a partir de tiempos de CPU.

## Criterios de aceptación

1. En TCL, el primer Move que supera el umbral produce un frame con escala distinta en el siguiente present, medido por trazas de evento/present; la mediana y p95 de esa espera se registran con fecha, hardware, flujo y tasa efectiva. Objetivo p95 ≤ 16,6 ms a 60 Hz; objetivo ambicioso ≤ 8,33 ms a 120 Hz efectivo.
2. Diez pinches consecutivos entre 1× y 3× no muestran salto de ancla > 2 px en transición pan→zoom o fast→sharp, ni banda estirada de un crop insuficiente.
3. El render final llega por worker y no bloquea UI; se registran frame p95, PSS y tiempo hasta nitidez. Se conservan los límites de memoria y la tabla de escenarios de `AGENTS.md`.

## Fuera de alcance

Cambio de motor MuPDF, nuevo gesto de doble tap, zoom de popup y superresolución.

## Riesgos y dependencias

Depende de ADR-011 para transformación y límites visibles; es independiente de ADR-019. El umbral de 1 %/2 dp es punto de partida sujeto a medición de ruido real, no resultado ya demostrado. El zoom mayor que la resolución del crop puede verse provisionalmente menos nítido; la prioridad es respuesta sin bloqueo y corrección geométrica.
