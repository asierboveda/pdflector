# ADR-014 — Selector Tap / Scroll y lectura continua

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

Hoy el visor presenta una página y un tap izquierdo/derecho cambia de página. Un arrastre de un dedo sin herramienta activa cancela el tap. `pdf_core::scroll` contiene matemáticas de viewport, pero el producto Android no compone una columna continua. El control segmentado de la imagen debe cambiar un modo de lectura real, no solo un aspecto visual.

## Decisión

Persistir `ReadingMode::Tap | Continuous` por documento, con `Tap` como valor de migración. En Tap se mantiene la navegación existente. En Continuous se dibuja una columna virtual de páginas a ancho disponible; el arrastre vertical de un dedo y el desplazamiento inercial recorren la columna, y un tap en la página no la avanza. El selector de dos segmentos usa el estilo de la imagen como referencia: pista oscura, segmento activo cian de la paleta, esquinas redondeadas y etiquetas claras `Tap` / `Scroll`, con área táctil ≥ 48 dp por segmento. El zoom y las anotaciones usan la transformación de cada página visible.

Se descarta renderizar todo el PDF o guardar un bitmap de columna: viola el presupuesto de RAM. Se descarta simular scroll con saltos de página: no ofrece continuidad ni referencias visuales entre páginas.

## Diseño técnico

- `pdf_core/src/scroll.rs`: índice acumulado de alturas visibles por página y función que resuelve offset vertical → páginas visibles, sin UI. Construir alturas desde metadatos de página en worker; incluir separación fija de 12 dp transformada por densidad en Android.
- `pdf_android/src/reader/continuous.rs`: estado `{offset, velocidad, páginas_visibles}` y composición de solo páginas que intersectan viewport; cache LRU por bytes y prefetch ±1. Guardar `{page, fracción_vertical}` al cambiar de modo o salir, para restaurar posición entre tamaños de ventana.
- `input/motion.rs`: Continuous consume arrastre vertical de dedo según ADR-019, cancela inercia al tocar, conserva long press de selección si el dedo permanece quieto. `draw/chrome.rs` y `reader/geometry.rs` comparten los rectángulos del selector; `reader/sheet_chrome.rs` cambia modo sin perder posición. El pinch escala la columna completa con un único factor de zoom; el ancla es el punto de documento bajo el centro de los dedos, o la página visible más cercana si el centro cae en la separación.

## Criterios de aceptación

1. Alternar Tap→Scroll y Scroll→Tap deja visible la misma página y aproximadamente el mismo punto vertical (error ≤ 5 % del alto de página), incluso tras girar la tablet o reabrir el libro.
2. Scroll atraviesa al menos tres límites de página sin blanco persistente ni salto; solo páginas visibles y vecinas se renderizan. Tap en Continuous no cambia página; tap en Tap mantiene la navegación actual.
3. Long press selecciona texto en la página correcta; bolígrafo, subrayado y goma conservan coordenadas. Al volver a Tap no queda velocidad inercial.
4. Se registra en TCL fecha, hardware, flujo de 100 páginas, frame p95, PSS y número máximo de bitmaps residentes; se exige frame p95 < 16,6 ms y se documenta el resultado frente al techo de memoria vigente.

## Fuera de alcance

Diseño de la barra (ADR-012), pan horizontal de una página ampliada (ADR-019) y miniaturas.

## Riesgos y dependencias

Depende de ADR-011 (geometría única), ADR-012 (ubicación del selector) y ADR-019 (arbitraje de arrastre con dedo). La caché actual guarda crops de una ventana para el modo de hoja única: Continuous necesita crops por intersección, con origen explícito, sin crear renders a resolución máxima. El salto de PSS ya registrado en `docs/plan/DEUDA.md` impide cerrar el trabajo solo con una buena tasa de frames.
