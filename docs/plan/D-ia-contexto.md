# Fase D — IA con contexto global del documento y selección

Asistente de lectura local con recuperación de contexto global del documento (RAG BM25) para explicar selecciones de texto, ecuaciones o figuras citando páginas reales.

## Auditoría

- **Componentes existentes**:
  - `crates/pdf_core/src/ai.rs`:
    - `chunk_pages` (:155): empaquetado de texto de páginas en bloques (`[págs N-M]`) con límite configurable de caracteres.
    - Clientes API: `OllamaClient` (inferencia local), `GroqClient` (Llama 3 70B ultra-rápido) y `GeminiClient` (Gemini Flash multimodal con soporte de imagen).
  - `crates/pdf_android/src/reader/`:
    - `toast_ia.rs` (:99-101): `explain_image` SÍ adjunta el texto extraído de la página seleccionada como contexto adicional al prompt multimodal junto a la captura PNG en base64.
    - `crates/pdf_android/src/draw/ai_panel.rs`: panel deslizante en GPU para visualización de respuestas y estado de consulta.
- **Limitaciones actuales (qué falta)**:
  - No existe índice de recuperación (RAG): el visor envía únicamente la página actual o el primer chunk. Si la respuesta requiere conceptos introducidos en capítulos previos, el modelo carece de contexto global.
  - Falta un índice BM25 puro en Rust en `pdf_core` que indexe el texto de todas las páginas al abrir el documento.
  - Falta optimización de prompt con instrucción estricta de citar páginas reales (`[págs N]`).
  - Pendiente estudio empírico de ventana de contexto en la tablet y script de evaluación `tools/ai-bench.sh`.

## Objetivo

Al seleccionar un fragmento (rectángulo o trazo), generar una explicación precisa que utilice el contexto de todo el documento y cite explícitamente las páginas fuente (`[págs N-M]`), sin alucinar información ausente.

## Tareas

- [ ] D1. **Índice BM25 local (puro Rust, sin dependencias externas)**:
  - Indexar las páginas del documento a partir de `PageText`.
  - Ante una selección, consultar el índice con los términos de la selección y recuperar las $k=5$ páginas más relevantes más las 2 páginas contiguas para preservar localidad.
- [ ] D2. **Prompt con contexto global estructurado**:
  - Formatear la consulta combinando el contexto recuperado (acotado a ventana de tokens segura) y el fragmento específico seleccionado.
  - Imponer directiva de tutoría con citas exactas de páginas.
- [ ] D3. **Estudio empírico de ventana de contexto en hardware real**:
  - Probar tamaños de ventana (8k, 12k y 20k caracteres) con PDFs del corpus en la tablet TCL 9469X.
  - Medir latencia de respuesta y tasa de precisión de citas.
- [ ] D4. **Script de pruebas `tools/ai-bench.sh`**:
  - Automatizar prueba con documento de prueba para verificar que la respuesta contiene citas válidas y no códigos de error.

## Criterio de cierre

- [ ] 5 consultas sobre documento largo (> 300 páginas): al menos 4 de 5 respuestas citan correctamente páginas del documento sin inventar datos.
- [ ] Latencia de respuesta en red p50 < 15 s vía Groq o Gemini Flash.
- [ ] El hilo de interfaz permanece a 60 fps durante la consulta asíncrona sin jank en el visor.

## Cómo modificar

- Si se prefiere RAG basado en embeddings vectoriales en lugar de BM25: evaluar coste de tamaño de APK de modelos ONNX embebidos vs. BM25 sin dependencias.
- Para cambiar de proveedor: configurar claves y endpoints en `pdf_core::ai`.

## Referencias

- `crates/pdf_core/src/ai.rs`
- `crates/pdf_android/src/reader/toast_ia.rs`
- `crates/pdf_android/src/draw/ai_panel.rs`
