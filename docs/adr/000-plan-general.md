# ADR-000 — Orden y dependencias de decisiones pendientes

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

Este índice ordena **las decisiones de los ADR-011 a ADR-021** para que se puedan convertir en Issues atómicos. No sustituye el roadmap vigente `docs/plan/NEXT-PLAN.md` ni declara cerrada ninguna fase. Las mediciones y el cierre de Issues siguen las reglas de `AGENTS.md`. Cada ADR registra una decisión de arquitectura pendiente de implementación; las observaciones del repositorio y las hipótesis se identifican dentro de cada documento.

## Orden recomendado

| Orden | ADR | Entrega aislada | Depende de |
|---:|---|---|---|
| 1 | [011 — geometría y márgenes](011-geometria-y-margenes-pdf.md) | Encuadre completo y transformación común; diagnóstico del PDF afectado | Ejemplar del bug para cierre |
| 2 | [019 — pan con un dedo](019-pan-con-un-dedo.md) | Desplazamiento de hoja ampliada con arbitraje de tap/selección/lápiz | 011 |
| 3 | [021 — zoom inmediato](021-zoom-dos-dedos-inmediato.md) | Primer feedback en el siguiente present y render nítido asíncrono | 011 |
| 4 | [012 — menú de anotación](012-menu-de-anotacion.md) | Barra de cuatro herramientas y alojamiento del selector de lectura | Ninguna; Recorte se muestra inactivo hasta 013 |
| 5 | [013 — recorte de tinta](013-recorte-de-tinta.md) | Lazo, mover y escalar trazos persistidos | 011, 012 |
| 6 | [017 — cuaderno general](017-cuaderno-de-notas-generales.md) | Notas textuales y manuscritas por documento | Ninguna |
| 7 | [018 — desborde de margen](018-desborde-del-margen-a-notas.md) | Elegir entre nota sobre la página y panel de apuntes | 011, 017 |
| 8 | [020 — popup de figuras](020-vista-previa-de-figuras.md) | Enlaces y referencias de texto verificables, sin abandonar página | 011 |
| 9 | [014 — Tap / Scroll continuo](014-modo-tap-y-scroll-continuo.md) | Control segmentado y columna virtual real | 011, 012, 019 |
| 10 | [015 — explicación Gemini](015-gemini-explicacion-formulas.md) | Fórmulas multimodales con credencial de usuario | Ninguna; usa selección existente |
| 11 | [016 — tarjetas Anki](016-tarjetas-anki-por-documento.md) | Una tarjeta básica en submazo del libro | 015 |

El orden prioriza la corrección geométrica, porque pan, zoom, selección de tinta, notas en margen, enlaces y scroll dependen de saber qué punto de página corresponde a cada píxel. Las entradas sin dependencia pueden implementarse antes si hay una sola persona disponible para cada fichero; el orden no autoriza trabajo simultáneo sobre `reader/mod.rs`, `input/motion.rs`, `engine.rs`, `store.rs` ni `ai.rs`.

## Límite de cada Issue

Cada fila da lugar a **un Issue** con el criterio de cierre del ADR enlazado. Ningún Issue se cierra por compilación solamente cuando toca render, caché o entrada: hace falta medición en la TCL con fecha, hardware, flujo y métrica registrada en `docs/benchmark-results.md`. Para ADR-011 además hace falta reproducir el PDF de PowerPoint afectado; sin ese artefacto solo se puede entregar la mejora general de encuadre, no declarar resuelto el bug específico. ADR-015 es una evolución de la explicación de selección ya presente y ADR-016 reutiliza ese cliente; ninguno inicia la fase D de contexto global del PDF, aplazada en `NEXT-PLAN.md`.

## Reparto sin solapamiento

Los ADR-011, 019, 021 y 014 comparten geometría/gestos/render y deben entrar en serie. ADR-012 precede a 013 y 014 por la barra. ADR-017 precede a 018 por el modelo de notas. ADR-015 precede a 016 por transporte y autenticación. ADR-020 puede avanzar tras 011 si nadie edita `engine.rs`, `input/gestos.rs` o `draw/overlays.rs` sin coordinar propiedad de fichero. Cada integrador parte de `main` actualizado, trabaja en un worktree propio e integra una sola rama por turno, como exige `AGENTS.md`.
