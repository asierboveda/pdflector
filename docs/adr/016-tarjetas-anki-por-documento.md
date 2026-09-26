# ADR-016 — Tarjeta Anki desde una selección

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

El menú de selección ya obtiene texto, rectángulo y página. El usuario quiere pulsar **Tarjeta** y obtener una pregunta/respuesta en una «carpeta» con nombre del libro. En Anki la unidad de organización equivalente es un **mazo**, no una carpeta de archivos. AnkiDroid ofrece un proveedor Android para crear mazos y notas; actualmente PDFLector no tiene integración ni almacenamiento de tarjetas.

## Decisión

Generar una única tarjeta de tipo básico por selección, mediante Gemini con salida JSON estructurada `{pregunta, respuesta}`. El botón crea y guarda la tarjeta automáticamente cuando la respuesta pasa validación; muestra ambos campos y el destino, con acción de editar en AnkiDroid. El destino es un submazo `PDFLector::<título normalizado>`; si dos documentos comparten título, se añade un sufijo corto derivado de la ruta canónica que ya identifica su sidecar, y se conserva la asignación al cambiar solo el título mostrado. La conexión con AnkiDroid vive en Android por `ContentResolver` y JNI; `pdf_core` define la solicitud, la validación y el mapeo documental, pero no conoce Android.

Se descartan `.apkg` por cada tarjeta (importaciones repetidas y duplicados), AnkiConnect (requiere servidor de escritorio) y escribir directamente la base de AnkiDroid (acoplamiento a esquema privado). Tampoco se añade un crate GPL/AGPL; se usa el contrato público del proveedor Android.

## Diseño técnico

- `pdf_core/src/ai.rs`: generador estructurado sobre el transporte y credencial de ADR-015, con `generationConfig.responseFormat.text` (`mimeType: APPLICATION_JSON`, esquema de dos cadenas requeridas). Prompt limitado al fragmento y, si hace falta, texto de la página vecina; validar JSON, campos no vacíos, longitud razonable, pregunta distinta de respuesta y que la respuesta pueda apoyarse en el fragmento. Guardar ruta de documento, página y fragmento como procedencia local, no como campo visible adicional de la tarjeta básica.
- `pdf_android/src/reader/tarjetas.rs`: orquestar selección → generación asíncrona → guardado. Estado de operación por ID para evitar duplicados tras rotación o doble tap. Si la IA falla, no se crea mazo ni tarjeta. Antes de insertar, buscar en AnkiDroid una etiqueta determinista `pdflector_<hash de ruta, página, selección>`; escribirla en la nota y guardar localmente el ID devuelto. Así un cierre entre inserción y confirmación se recupera por búsqueda de etiqueta y no duplica. Una acción explícita «Crear otra» usa un nuevo ID de operación.
- `pdf_core/src/store.rs`: tablas `anki_drafts` y `anki_receipts` en el sidecar del documento, separadas de `annotations`; guardan borrador y recibo de inserción para reintentos, sin que `AnnotationStore::save` las borre.
- `PdfLectorActivity.kt` o un adaptador Kotlin dedicado: detectar AnkiDroid y permiso del proveedor; consultar/crear submazo y tipo básico, insertar una nota con pregunta/respuesta, comprobar ID devuelto. Nunca abrir ni modificar la base interna de Anki. Manejar proveedor ausente, colección no configurada y permiso denegado con aviso claro y conservación de un borrador local reintentable.
- `reader/seleccion.rs` y `input/gestos.rs`: añadir acción tipada `Tarjeta` al menú contextual existente. El nombre de mazo se deriva del título de biblioteca, saneado para no introducir `::` extra, y la identidad se conserva en el registro local.

## Criterios de aceptación

1. Una pulsación sobre un fragmento con texto genera exactamente una nota básica con pregunta y respuesta no vacías en el submazo del libro; se ve en AnkiDroid después de reiniciar ambas apps.
2. Dos libros homónimos usan submazos distintos; reabrir o cambiar el título mostrado del mismo libro conserva su submazo. Repetir un tap por rotación o error de red no duplica la nota, incluso si la app se cierra tras la inserción.
3. Sin AnkiDroid, sin permiso, sin clave Gemini o sin red, no se indica éxito ni se pierde el borrador; un reintento explícito guarda una sola nota.
4. Un fragmento sin texto extraíble no activa esta acción; las fórmulas visuales permanecen cubiertas por ADR-015. UI y proveedor operan sin bloquear el frame; se mide en TCL fecha, hardware, flujo, frame p95, PSS y latencia generación→guardado.

## Fuera de alcance

Sincronización con AnkiWeb, mazos remotos, imágenes/MathJax en la tarjeta, generación masiva y revisión espaciada dentro de PDFLector.

## Riesgos y dependencias

Depende del transporte y la credencial de ADR-015; no depende del menú de herramientas ADR-012, pues la acción vive en el menú de selección ya existente. El proveedor puede variar entre versiones de AnkiDroid; fijar una versión de referencia al implementar y comprobar permisos/URI en la tablet. [Contrato oficial del proveedor](https://github.com/ankidroid/Anki-Android/blob/main/api/src/main/java/com/ichi2/anki/FlashCardsContract.kt), [manual de submazos `Padre::Hijo`](https://github.com/ankidroid/ankidroiddocs/blob/main/manual.asc) y [configuración de salida JSON de Gemini](https://ai.google.dev/api/generate-content). La calidad de la tarjeta generada no está garantizada por la validación sintáctica.
