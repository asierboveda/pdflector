# Fase D — IA con contexto completo + selección (5-7 días, must)

> Tu requisito: "yo señalo una parte que no entiendo, debe explicar con contexto de todo el PDF". Actual: `chunk_pages` corta el PDF en trozos de `max_chars` sin índice, y `explain_image` manda solo el crop PNG sin texto.

## Auditoría

- `ai.rs`: `chunk_pages` (word-packing, prefijo `[págs N-M]`) correcto pero sin retrieval: manda 1 chunk al LLM (el de la selección o el primero). No hay RAG.
- `ai.rs`: `GroqClient::chat` (texto, 70B) ok, `GeminiClient::explain_image` (visión, flash) ok, pero `pdf_android/reader.rs` solo manda `sel_page_rect` + crop PNG (base64) sin texto circundante. El LLM alucina sin contexto.
- `pdf_core` ya tiene `Document::text()` por página (stext), base para índice TF-IDF/BM25 puro Rust (sin deps).
- Competencia: ChatPDF y `koreader/koreader` (plugin) usan embeddings locales + rerank; Adobe Acrobat AI usa `page_range` + vision.

## Objetivo

Selección (rect o lápiz) → explicación que cita `págs N-M` reales del documento, con contexto global, sin inventar.

## Tareas

- [ ] D1. **Índice BM25 local** (puro Rust, sin deps): al abrir PDF, `build_index(doc, pages)` → `Vec<(page_idx, Vec<String>)>` + posting list. Query = texto de la selección (extraído de `PageText` spans intersectados). Top-k=5 páginas más relevantes (BM25) + 2 páginas vecinas de la selección (localidad).
- [ ] D2. **Prompt con contexto**: `system: "Eres tutor del PDF, cita [págs N] siempre"` + `user: "Contexto global (k páginas BM25, truncado a 12k chars):\n[ págs 3-5 ]...\n\nSelección que no entiendo (pág X, crop PNG base64):\n[ págs X ] texto...\n\nExplica la selección usando el contexto global, cita páginas."` → `GeminiClient::explain_image` (visión) o `GroqClient::chat` (texto puro). Si selección vacía, manda solo contexto global.
- [ ] D3. **Estudio de contexto**: medir ventana óptima: ¿12k chars bastan para scientific_paper 12p? Probar 8k/12k/20k + k=3/5/8 en TCL con 5 PDFs del corpus, medir alucinación (cita inventada) y latencia Groq/Gemini (reqwest rustls).
- [ ] D4. **Harness `adb`**: `tools/ai-bench.sh` que abre `corpus/scientific_paper.pdf`, selecciona pág 5, pide explicación, valida que la respuesta contiene `[págs` y no `404`.

## Criterio de cierre

- [ ] 5 preguntas sobre `large_document.pdf` (500p) → 4/5 respuestas citan `págs` correctas y no inventan (revisión manual)
- [ ] Latencia p50 <15s en TCL vía Groq/Gemini (reqwest timeout 300s ya existe)

## Cómo modificar

- Si quieres embeddings locales (no BM25): añade `fastembed` crate (pero aumenta APK). BM25 ya es 90% del beneficio sin deps.
- Si quieres RAG por chunks y no por página: cambia `chunk_pages` a `chunk_by_tokens` (usa `max_chars` como ahora).

## Referencias

- `crates/pdf_core/src/ai.rs`, `engine/mupdf.rs: text()`, `pdf_android/src/reader.rs: sel_page_rect`
- Competencia: `ByteApps/prime-pdf-viewer` no tiene IA — tu diferenciador.
