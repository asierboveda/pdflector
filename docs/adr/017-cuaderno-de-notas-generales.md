# ADR-017 — Cuaderno de notas generales por documento

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

`Annotation::TextNote` representa texto anclado a un punto de una página. No cubre apuntes generales del libro ni una superficie amplia de escritura con lápiz. El sidecar SQLite ya identifica el PDF y preserva anotaciones vectoriales; reutilizar su tabla `annotations` obligaría a fingir una página para notas globales.

## Decisión

Cada documento tendrá un **cuaderno** con entradas generales y entradas vinculadas a una página. Una entrada contiene título opcional, cuerpo de texto y trazos vectoriales en coordenadas propias del lienzo de nota; las vinculadas guardan `page_idx` y, si nacen de un margen, `anchor_rect` en coordenadas de página. El usuario abre el cuaderno desde el visor, crea/edita una entrada y vuelve a la misma posición. El PDF y sus anotaciones no cambian al escribir en el cuaderno.

Se descarta usar `TextNote` con `page_idx = 0` para apuntes generales porque produciría marcadores falsos y exportación confusa. Se descarta un fichero Markdown por nota como almacén principal: no ofrece guardado transaccional entre texto y trazos ni IDs estables. La exportación Markdown existente podrá incorporar notas en una fase posterior.

## Diseño técnico

- `pdf_core/src/notes.rs`: modelo `Note {id, page_idx?, anchor_rect?, title, body, strokes, updated_at}` y operaciones crear, editar, listar, borrar explícitamente. Validar coordenadas y versión de formato; no importar UI. El sidecar existente, resuelto desde la ruta local del PDF, establece la identidad del documento sin crear una segunda clave.
- `pdf_core/src/store.rs`: tablas nuevas `notes` y `note_strokes` en el sidecar del documento, migradas con `CREATE TABLE IF NOT EXISTS` y transacciones. No alterar ni eliminar filas de `annotations`; el `save` actual de anotaciones no toca estas tablas. Lectura paginada o bajo demanda: no cargar todos los trazos de todos los cuadernos al abrir la biblioteca.
- `pdf_android/src/reader/notes.rs`: panel de cuaderno y estado de edición; texto mediante IME Android por `jni.rs`, lápiz mediante la ruta causal existente transformada al lienzo de nota. Los overlays capturan input antes del PDF. Guardar al cerrar o cambiar de entrada mediante worker; confirmar visualmente solo tras éxito durable. El panel presenta entradas generales primero y luego las de la página actual.

## Criterios de aceptación

1. Crear una nota general con texto y tinta, cerrar/reabrir la app y recuperar texto, trazos y orden. Crear una nota de página en p. 7 y verla desde p. 7 sin que aparezca como nota de p. 1.
2. Una operación de guardado fallida mantiene el borrador en memoria y muestra error; no destruye anotaciones anteriores. Borrar una nota requiere acción explícita del usuario.
3. La entrada con ≥ 200 trazos no congela UI al guardar o abrir; en TCL se registra fecha, hardware, flujo, frame p95 y PSS. El PDF sigue respetando la cota de caché.
4. Importar/abrir un sidecar antiguo conserva exactamente sus anotaciones; el esquema nuevo no cambia la exportación actual sin una decisión adicional.

## Fuera de alcance

Sincronización de cuadernos, búsqueda full text, OCR de tinta, exportación nueva y apertura automática por falta de margen (ADR-018).

## Riesgos y dependencias

La entrada de texto requiere puente con IME Android y convivencia con el overlay de tinta. La ruta actual del sidecar incorpora hash de ruta: mover un archivo con su sidecar exige la misma resolución/migración que hoy requieren las anotaciones; no se borra nada automáticamente. ADR-018 depende de este cuaderno; este ADR es útil por sí solo.
