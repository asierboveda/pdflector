# ADR-019 — Desplazar la página con un dedo

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

El visor actual distingue un tap de un long press y de un pinch. Con `ToolKind::Navigate`, mover un dedo más de `TAP_SLOP` cancela el tap; el pan de página usa dos dedos o exige una herramienta activa. Esto vuelve incómodo recorrer una página ampliada. El stylus debe seguir escribiendo o borrando sin que una palma se convierta en pan.

## Decisión

Un **dedo** desplaza la página cuando hay contenido fuera del viewport; en modo Tap esto ocurre al ampliar sobre el encuadre base, y en modo Continuous el arrastre vertical desplaza la columna (ADR-014). Un gesto empieza como candidato a tap/long press; al superar el umbral de movimiento de Android se convierte una sola vez en `FingerPan`, desarma el long press y consume el tap. Un segundo dedo transfiere el gesto a pinch sin salto. El stylus conserva su ruta de tinta/subrayado/goma y el rechazo de palma existente. El pan se limita a los bordes de la página, sin pasar a otra al arrastrar.

Se descarta usar un «modo mano» obligatorio: la intención de un arrastre sobre contenido ampliado es inequívoca. Se descarta interpretar el Up de un pan como tap: ya produjo movimiento visible.

## Diseño técnico

- `pdf_android/src/input/gestos.rs` y `input/motion.rs`: añadir estado `FingerPan` con ID de puntero, posición previa y origen. Capturar por tipo de herramienta de `MotionEvent`, no por número de punteros solamente. El árbitro resuelve en orden overlay → stylus activo/palma → tap/long press → pan → pinch; al llegar segundo dedo cancela el pan y reancla el pinch en las posiciones actuales.
- `reader/pinch.rs`: reutilizar `pan_by`, `clamp_pan` y el ancla existente, con la geometría única de ADR-011. `reader/seleccion.rs` mantiene long press estacionario para texto; una selección activa captura su propio arrastre.
- En modo Tap y zoom base `contain`, donde la hoja cabe entera, el arrastre no mueve la página ni cambia de hoja. En Continuous, `FingerPan` entrega delta vertical a ADR-014 y el pan horizontal solo se aplica si el zoom de la página excede el ancho del viewport.

## Criterios de aceptación

1. A zoom 2, arrastrar con un dedo 100 px mueve el contenido aproximadamente 100 px hasta el borde; ni cambia página ni dibuja. En zoom base sin contenido oculto, el mismo arrastre deja la página fija.
2. Un tap corto sigue pasando página una vez; mantener el dedo quieto sigue abriendo selección; un movimiento que inició pan no ejecuta tap al levantar.
3. Añadir un segundo dedo durante pan inicia zoom sin salto visible > 2 px. Durante tinta con stylus, contacto de palma no desplaza ni escala la página.
4. En TCL se documentan fecha, hardware, flujo con 100 pans, frame p95 y PSS; los gestos mantienen frame p95 < 16,6 ms.

## Fuera de alcance

Inercia en modo Tap, cambio de página por swipe y scroll continuo (ADR-014).

## Riesgos y dependencias

Depende de ADR-011 para que el clamp opere sobre los límites visibles reales. ADR-014 reutiliza el árbitro de gestos. Los umbrales de dedo deben medirse en dp y con densidad del dispositivo; el valor actual en px no se traslada a otra pantalla sin comprobarlo. [Android expone el tipo de cada puntero en `MotionEvent`](https://developer.android.com/reference/android/view/MotionEvent).
