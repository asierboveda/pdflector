# Acceso rápido a papers arXiv desde PDFLector — Design (2026-09-10)

> Estado: propuesto, pendiente revisión usuario. Sin código. Scope: Fase A (pegar ID) ahora + Fase B (buscar+ficha) fase 2. Fase C (feed categorías PaperTok-lite) aparcada post-v1.
> Verificado en vivo 2026-09-10: `export.arxiv.org/api/query` + `/pdf/{id}`. `reqwest blocking+rustls-tls` ya en `.so` Android. `INTERNET` ya declarado.

## 1. Arquitectura

```
UI thread (pdf_android, 60fps)          BG worker (std::thread::spawn + mpsc)
─────────────────────────────          ──────────────────────────────────────
Library header [Descubrir] ──cmd──> DownloadWorker / DiscoverWorker
  try_recv() en tick (poll 8ms)  <──msg──  pdf_core::arxiv (lógica pura)
                                              │ streaming std::io::copy → .part → rename
                                              v
                                   internal/pdfs/arxiv_{id}.pdf → library.json + papers.json best-effort
                                   → ThumbWorker (portada) + sidecar annotations/<stem>-<hash8>.db
```

* `pdf_core::arxiv` sin UI, devuelve `Result<T, ArxivError>`, sin `unwrap`. Re-export en `lib.rs` como `ai`.
* `pdf_android` solo coordina: spawn + `try_recv` en `tick.rs` + flag en `needs_tick` (molde `toast_ia.rs:60 ask_ai`). Descarga cancelable tipo `ThumbWorker` (`thumbs.rs:194`).
* Pestaña "Descubrir" en zona fija `lib_header_h=135px` (`geometry.rs`) — coste cero en scroll banda (~18MiB, memcpy p95 6.1ms TCL).

## 2. Componentes

| Pieza | Ubicación | Molde |
|---|---|---|
| `parse_arxiv_id()` — acepta `YYMM.NNNNN`, `YYMM.NNNN`, `arch/SCYYMMNNN`, `vN` opcional, prefijo `arXiv:`, URLs `/abs/` y `/pdf/` | `crates/pdf_core/src/arxiv.rs` (nuevo) | `store.rs sidecar_path`, `jni.rs:440 sanitize_pdf_name` |
| `ArxivClient::download(id, &mut writer)` — GET `https://arxiv.org/pdf/{id}`, sigue 301, lee `content-disposition` (versión resuelta) + `etag` | mismo módulo, `reqwest::blocking::Client` reutilizado + `User-Agent: PDFLector/0.1 (+url; email)` + timeout 10-20s propio (no 300s de `ai.rs`) | `ai.rs:593 GroqClient`, `ai.rs:731 GeminiClient` |
| `ArxivClient::fetch_meta()` + parser Atom streaming (solo Fase B) | mismo módulo + `quick-xml 0.41` (MIT, solo `memchr`, `default-features=false`, sin `encoding`) | receta `Reader::from_reader` + `buf.clear()`, `unescape()` (feed real contiene `&gt;`) |
| `ArxivEntry {id,version,title,summary,authors,primary_category,categories,published,updated,pdf_url,abs_url,doi}` | mismo módulo, `serde` | `PaperBuilder.create()` PaperTok (canonicalId, whitespace collapse) |
| `DownloadWorker` + `DiscoverState` | `pdf_android/src/reader/` | `thumbs.rs ThumbWorker`, `library_state.rs LibraryState`, `tick.rs pump_thumbs` |
| `papers.json` (nuevo, best-effort, `#[serde(default)]`) — metadatos por path; `library.json: Vec<BookProgress>` intacto | `internal/` junto a `library.json` | `persist.rs load/save_progress`, `touch_progress` pura |

## 3. Data flow

* **Fase A:** pegar `2401.12345` / `https://arxiv.org/abs/2401.12345` → `parse_arxiv_id` → worker GET `/pdf/{id}` (200 directo última versión) → streaming 8KB a `internal/pdfs/arxiv_2401.12345.pdf.part` → rename → `touch_progress` + append `papers.json` (id, pdf_url, abs_url, timestamp) → `reload_curated_library` → `open_library_entry`. Sin XML.
* **Fase B:** query `ti:/au:/abs:/cat:/all:` + `AND/OR/ANDNOT` → GET `export.arxiv.org/api/query` con throttle global 1req/3s + 1 conexión (`Mutex<Instant>`) + `max_results=25` (~55KB) + `start` paginación → `quick-xml` → `Vec<ArxivEntry>` → UI ficha (título/autores/abstract/categorías/etiqueta Preprint) → tap descarga usa href `rel="related" title="pdf"` versionado del feed, no reconstruye URL.
* Nombres locales: ID moderno `2301.07041.pdf` tal cual; clásico `hep-th/9901001` → `hep-th_9901001.pdf` (solo disco, jamás en URL — `hep-th_9901001` en `id_list` da 400).

## 4. Error handling

* `ArxivError {Network(reqwest), Http{status,body}, RateLimited, InvalidId, XmlParse, Io}` — `Display` con URL como `AiError`.
* `4xx/5xx` = error duro (el viejo `<title>Error</title>`+200 ya no existe). Feed 200 con 0 entries = "id no encontrado", no fallo red. `Disconnected` en `try_recv` = "sin respuesta" (molde `tick.rs:61`).
* 403/503 + `Retry-After` → backoff exponencial + jitter, máx 3 reintentos, luego mensaje UI. Nunca reintentar en bucle (arXiv lo trata como ataque).
* Todo best-effort en persistencia: lectura corrupta → `Vec::new()` + warn; escritura falla → log, nunca rompe arranque. Sin borrado automático (E4): caché solo fuera de biblioteca curada o con borrado explícito.

## 5. Testing

* `pdf_core/tests/arxiv.rs` con `TcpListener` local + `with_base_url` (molde `tests/ai.rs`, `ai.rs:511/706 doc(hidden)`) — sin salir a internet.
* Casos: IDs moderno/clásico/versionado/URL abs/pdf/prefijo `arXiv:`; `id_list` con `/` literal y `%2F`; feed vacío; 400/500; entidades `&gt;`; whitespace título/summary; `max_results` respeta `id_list`; throttle ≥3s (mock tiempo).
* Sin tests de UI; verificación manual TCL: pegar ID → PDF en `pdfs/` + entrada biblioteca + portada lazy, `dumpsys meminfo` PSS plano.

## 6. Roadmap y no-objetivos

* v1 intacto (A→B→C→E). Esto es capa desacoplada `pdf_core`, post-v1 para UI. Comentario `INTERNET` en `pdf_android/Cargo.toml` ("no hay otros usos de red") se reescribe al añadirlo.
* No-objetivos: ranking/afinidad/follows/semántica PaperTok, OpenAlex/Semantic Scholar (exigen key / 429), re-hostear PDFs (prohibido; CC0 solo metadatos), descarga masiva/precarga (robots `Crawl-delay: 15`, ToU 1req/3s), mostrar PDF como respaldado por arXiv. Atribución requerida en About: "Thank you to arXiv for use of its open access interoperability."

## Self-review

* Sin TBD/TODO; errores con comportamiento real 2026-09-10 (no manual 2007); `/`→`_` solo disco; `quick-xml` coste declarado como crate nuevo en `.so` (no "ya en grafo"); memoria O(1) streaming; single-file scope para plan.
