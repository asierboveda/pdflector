# ADR-015 — Explicar fórmulas seleccionadas con Gemini

**Estado:** Decisión documentada; implementación pendiente. **Fecha:** 2026-09-26.

## Contexto

Ya existe selección rectangular y extracción de texto/PNG en `reader/seleccion.rs`. `reader/toast_ia.rs` manda imagen a `pdf_core::ai::GeminiClient::explain_image`, texto sin imagen a Groq, y crea un hilo por consulta. `GeminiClient` usa HTTPS `generateContent` con `x-goog-api-key`. La clave actual procede de `include_str!(google_key.txt)`: el build exige el fichero y una APK con clave real permitiría extraerla. La suscripción Google AI Pro da ventajas en AI Studio; [el uso de la Gemini API se factura y limita por separado](https://ai.google.dev/gemini-api/docs/google-ai-plans).

## Decisión

La acción **Explicar fórmula** usará siempre Gemini multimodal: PNG de la región seleccionada más texto extraído de esa región y número de página como contexto. Nunca enviará el documento entero. La ruta de autenticación del producto personal será una clave API que el propietario introduce en ajustes, cifrada en almacenamiento privado con una clave de Android Keystore y obtenida por JNI al iniciar la solicitud. No se empaquetará una clave real en la APK. El modelo será un identificador configurable con valor inicial documentado y validado mediante `models.list` o una consulta mínima; no se asumirá que «Pro» concede acceso a un modelo concreto. La app mostrará antes del primer envío que la selección sale a Google.

Se descarta usar la cuenta de Gemini de la app como token de API: no es una credencial compatible. Se descarta OCR local nuevo porque el PNG multimodal ya cubre fórmulas no extraíbles. Se descarta mantener Groq como fallback silencioso para fórmulas, porque cambiaría la interpretación y la privacidad sin indicarlo.

## Diseño técnico

- `pdf_core/src/ai.rs`: conservar el adaptador HTTP sin UI, ampliar la petición con texto+imagen y respuesta tipada `{explicación, limitación}` o error; controlar 429/503 con reintento acotado y `Retry-After`, timeout finito y tamaño máximo de imagen. No registrar claves, prompt ni respuesta.
- `pdf_android/src/reader/seleccion.rs`: recortar la imagen **desde el bitmap que contiene la selección** o pedir al worker un render acotado de esa región si cae fuera del crop residente. Registrar la página y el rect en puntos PDF; limitar PNG y pedir consentimiento inicial antes de enviarlo.
- `reader/toast_ia.rs`: una petición activa por panel con ID/generación; resultado tardío de otra selección o documento se descarta. El worker de red recibe una copia efímera de la clave; la UI solo consume estado `Cargando/Respuesta/Error` y permite reintentar. `jni.rs` y `PdfLectorActivity.kt` aportan ajuste y acceso a credencial Keystore. Durante la migración el placeholder de build puede permanecer, pero nunca se usa como credencial productiva; al cambiar ese requisito, se actualizan `AGENTS.md`, `CONTRIBUTING.md` y el skill de build en el mismo commit.

## Criterios de aceptación

1. Una fórmula vectorial y una fórmula incluida como imagen reciben explicación asociada a la región elegida y página correcta; el modo sin texto extraíble sigue funcionando con imagen.
2. Sin clave, sin cuota, sin red o con respuesta bloqueada se muestra un error recuperable; no se envía nada ni se crea respuesta falsa. Cerrar el panel impide mostrar resultados tardíos.
3. La APK distribuible no contiene credenciales reales; la clave no aparece en Git, logs, URL ni backup. Borrar el ajuste elimina la copia cifrada y deshabilita la acción.
4. La llamada HTTP y el render de región nunca bloquean UI; en TCL se registra fecha, hardware, flujo, frame p95, PSS y latencia de respuesta p50/p95 por separado.

## Fuera de alcance

RAG del PDF completo (fase D aplazada), resolución simbólica garantizada y servidor multiusuario. No se afirma exactitud matemática sin revisión humana.

## Riesgos y dependencias

Una clave introducida en un cliente sigue siendo extraíble por un atacante que controle el proceso; [Google recomienda un proxy servidor para apps públicas](https://ai.google.dev/gemini-api/docs/api-key). La decisión de clave local solo vale para uso personal con clave del propietario; una distribución pública con clave compartida requiere otro ADR. [La API documenta `generateContent` multimodal](https://ai.google.dev/api/generate-content). Este ADR es independiente de la fase D y aporta el cliente reutilizable para ADR-016.
