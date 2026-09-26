# ADR-018 — Enviar apuntes al cuaderno cuando el margen no cabe

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

La petición es escribir notas junto a la página y, cuando no haya espacio, abrir un panel de apuntes. Hoy existen trazos libres sobre la página y `TextNote` anclada, pero no hay medición de espacio disponible. Inferir «margen libre» solo por color blanco falla en escaneos, fondos de color o PDF con `CropBox` particular.

## Decisión

Ofrecer una acción explícita **Nota en margen**. Antes de abrir el editor, evaluar un rectángulo de nota de tamaño mínimo **120 × 80 pt** alrededor del punto tocado. Es válido si queda dentro de la caja visible, deja 8 pt de separación de texto, imágenes y anotaciones existentes, y no invade el área ocupada por contenido extraído. Si no existe rectángulo válido en el margen lateral más cercano, abrir directamente una entrada vinculada a esa página en el cuaderno de ADR-017, con ancla en el punto tocado. Si la extracción de ocupación falla o es incompleta, usar el panel de cuaderno como opción segura; no escribir sobre el PDF a ciegas. El usuario siempre puede abrir el cuaderno manualmente.

Se descarta basarse en `crop_margins` y umbral RGB: detectaría mal páginas con fondos, sombras o imágenes. Se descarta ampliar el PDF artificialmente: desalinearía anotaciones y exportación.

## Diseño técnico

- `pdf_core/src/margins.rs`: calcular regiones disponibles como diferencia entre caja visible y unión de rectángulos de texto, imágenes y anotaciones, con expansión de 8 pt. Resultado `PageMarginDecision::Inline(rect) | Notebook(anchor)`, puro y cacheable por revisión de página/anotaciones. Requiere que el adaptador MuPDF exponga cajas de imagen/gráfico cuando las haya; si falta, devolver `Notebook`.
- `pdf_android/src/reader/notes.rs`: solicitar decisión en worker tras tap sobre acción Nota en margen; mostrar indicador de espera sin bloquear. `Inline` usa el editor de nota anclada; `Notebook` abre el panel preseleccionado en la página. Guardar la vinculación y el rectángulo en el modelo de ADR-017.
- `reader/geometry.rs`: convertir punto de pantalla a página mediante ADR-011, incluso con zoom, rotación y futuro Scroll. Recalcular cuando cambian anotaciones o la caja visible.

## Criterios de aceptación

1. En una página con margen ≥ 120 × 80 pt, se abre el editor en el margen y el rectángulo no solapa contenido ni anotaciones; en una página a sangre o con margen insuficiente se abre el panel de apuntes con la página correcta.
2. Un PDF escaneado o una extracción fallida abre el panel, sin colocar contenido encima de la imagen. Al girar o ampliar, el ancla de página se conserva.
3. La decisión es determinista para el mismo PDF y anotaciones. La UI no espera a MuPDF; se registra en TCL fecha, hardware, flujo, frame p95, PSS y latencia de decisión.

## Fuera de alcance

Reflujo del contenido del PDF, recorte de márgenes y clasificación semántica por IA.

## Riesgos y dependencias

Depende de ADR-011 para caja/transformación y ADR-017 para notas de página y panel. Los PDF con gráficos vectoriales complejos pueden tener ocupación difícil de extraer; el fallback al cuaderno es parte del contrato, no un fallo silencioso. No se modifica automáticamente ninguna anotación existente.
