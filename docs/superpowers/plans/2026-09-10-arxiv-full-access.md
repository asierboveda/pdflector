# ArXiv Full Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** La app sirve papers sola: Buscar + Feed por categorías + Ficha + descarga a biblioteca, más handoff PaperTok→PDFLector (tap Leer abre la APK).

**Architecture:** `pdf_core::arxiv` (lógica pura, reqwest blocking+rustls ya compilado, quick-xml streaming) + `DiscoverWorker` único (1 hilo, 1 conexión, mpsc + try_recv en tick) + `UiMode::Discover` (planos cacheados mutuamente exclusivos con Library) + intent-filters VIEW pdflector:// y SEND text/plain que alimentan el mismo parse+descarga.

**Tech Stack:** Rust, reqwest 0.11 blocking+rustls-tls, quick-xml 0.41 default-features=false, android-activity 0.6, cargo-apk 0.10, MuPDF.

**Spec:** docs/superpowers/specs/2026-09-10-arxiv-fast-access-design.md

## Global Constraints

- `pdf_core` sin UI (nunca depende de egui/pdf_android).
- Hilo UI nunca bloquea: red solo en worker, UI solo `try_recv()` en `tick()`.
- PSS producto <150MB: descarga streaming 8KB a `.part` + rename, XML streaming sin DOM, caché feed con tope bytes+LRU.
- Sin `unwrap`/`expect` en `crates/pdf_core/src` fuera de `#[cfg(test)]` (usa `Result`).
- Biblioteca nunca borra sola: solo `clear_library` explícito; fallos dejan status/log.
- Rate-limit arXiv: 1 req/3s, 1 conexión; `User-Agent: PDFLector/0.1 (+url-proyecto; email)`; backoff con `Retry-After` ante 403/503.
- `/` clásico solo se convierte a `_` en nombre fichero local, jamás en URL (`hep-th_9901001` en id_list → 400).
- Español chat, inglés código/commits.

---

## File Structure

- `crates/pdf_core/src/arxiv.rs` (NUEVO): `ArxivId`, `parse_arxiv_id`, `ArxivQuery`, `ArxivEntry`, `ArxivError`, `ArxivClient`. Única capa red arXiv.
- `crates/pdf_core/tests/arxiv.rs` (NUEVO): TcpListener local, molde `tests/ai.rs`.
- `crates/pdf_core/src/lib.rs`, `crates/pdf_core/Cargo.toml`: `pub mod arxiv`, `quick-xml=0.41 default-features=false`.
- `crates/pdf_android/src/discover.rs` (NUEVO, hermano `thumbs.rs`): `DiscoverCmd/Msg/DiscoverWorker`, `FeedCache` disco ≤4MiB LRU TTL 10min.
- `crates/pdf_android/src/reader/discover_state.rs` + `reader/discover.rs` (NUEVOS): `DiscoverState/Tab/Screen/Phase`, métodos Reader.
- `crates/pdf_android/src/reader/discover_categories.rs` (NUEVO): ~45 cats arXiv + etiquetas ES.
- Edits: `reader/mod.rs` (`UiMode::Discover` + campos), `reader/tick.rs` (`pump_discover` + `discover_busy`), `reader/geometry.rs` (`lib_tabs_rect`, `disc_*`), `reader/redraw.rs` (rama Discover), `draw/discover.rs`+`draw/mod.rs`+`draw/library.rs` (tabs compartidas), `input/motion.rs` (`discover_tap`), `persist.rs` (`PaperMeta/papers.json`, `DiscoverPrefs`), `reader/library.rs` (`library_add_entry`), `reader/life.rs` (init), `jni.rs` (`launch_intent_request` + `open_url` allowlist), `pdf_android/Cargo.toml` (filtros VIEW pdflector:// + SEND, reescribir comentario INTERNET).

---

### Task 1: pdf_core arxiv IDs + query builder

**Files:**
- Create: `crates/pdf_core/src/arxiv.rs`
- Modify: `crates/pdf_core/src/lib.rs`
- Test: `crates/pdf_core/tests/arxiv.rs`

**Interfaces:**
- Consumes: nada.
- Produces: `pub enum ArxivId { Modern(String), Modern4(String), Classic(String) }`, `pub fn parse_arxiv_id(raw: &str) -> Result<(String, Option<String>), ArxivError>`, `pub struct ArxivQuery { ... }`, `impl ArxivQuery { pub fn to_params(&self) -> Vec<(String,String)> }`, `pub enum ArxivError { InvalidId(String), Network(String), Http{status:u16,body:String}, RateLimited, XmlParse(String), Io(String) }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parse_accepts_all_id_shapes() {
    assert_eq!(parse_arxiv_id("2401.12345").unwrap().0, "2401.12345");
    assert_eq!(parse_arxiv_id("https://arxiv.org/abs/hep-th/9901001v3").unwrap().0, "hep-th/9901001");
    assert_eq!(parse_arxiv_id("arXiv:0706.0001v2").unwrap().0, "0706.0001");
    assert!(parse_arxiv_id("hep-th_9901001").is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pdf_core --test arxiv parse_accepts_all_id_shapes`
Expected: FAIL (file not found / function not defined).

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ArxivError { InvalidId(String), Network(String), Http{status:u16,body:String}, RateLimited, XmlParse(String), Io(String) }
impl std::fmt::Display for ArxivError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{:?}", self) } }
impl std::error::Error for ArxivError {}

pub fn parse_arxiv_id(raw: &str) -> Result<(String, Option<String>), ArxivError> {
    let mut s = raw.trim().to_owned();
    for p in ["https://arxiv.org/abs/", "https://arxiv.org/pdf/", "http://arxiv.org/abs/", "arXiv:", "arxiv:"] {
        if let Some(r) = s.strip_prefix(p) { s = r.to_owned(); break; }
    }
    let s = s.strip_suffix(".pdf").unwrap_or(&s).trim().to_owned();
    if s.contains('_') { return Err(ArxivError::InvalidId(raw.into())); }
    let (base, ver) = match s.rfind('v') {
        Some(i) if s[i+1..].chars().all(|c| c.is_ascii_digit()) => (s[..i].to_owned(), Some(s[i+1..].to_owned())),
        _ => (s.clone(), None),
    };
    let ok = base.contains('/') || (base.contains('.') && base.len() >= 8);
    if !ok { return Err(ArxivError::InvalidId(raw.into())); }
    Ok((base, ver))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pdf_core --test arxiv`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_core/src/arxiv.rs crates/pdf_core/tests/arxiv.rs crates/pdf_core/src/lib.rs
git commit -m "feat(core): arxiv id parsing and query builder"
```

### Task 2: ArxivClient search + Atom streaming parser

**Files:**
- Modify: `crates/pdf_core/src/arxiv.rs`
- Modify: `crates/pdf_core/Cargo.toml`
- Test: `crates/pdf_core/tests/arxiv.rs`

**Interfaces:**
- Consumes: `parse_arxiv_id`, `ArxivError` de Task 1.
- Produces: `pub struct ArxivEntry { pub id: String, pub version: Option<String>, pub title: String, pub summary: String, pub authors: Vec<String>, pub primary_category: String, pub categories: Vec<String>, pub published: String, pub updated: String, pub pdf_url: String, pub abs_url: String, pub doi: Option<String> }`, `impl ArxivClient { pub fn new() -> Result<Self, ArxivError>, pub fn search(&self, q: &ArxivQuery) -> Result<Vec<ArxivEntry>, ArxivError>, pub fn fetch_by_ids(&self, ids: &[&str]) -> Result<Vec<ArxivEntry>, ArxivError> }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parses_atom_entry_with_entities() {
    let xml = include_str!("fixtures/arxiv_one.xml"); // contiene (1D-&gt;2D)
    let entries = parse_atom_for_test(xml).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].summary.contains("(1D->2D)"));
    assert!(entries[0].pdf_url.contains("/pdf/"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pdf_core --test arxiv parses_atom_entry_with_entities`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```toml
# crates/pdf_core/Cargo.toml, añadir:
quick-xml = { version = "0.41", default-features = false }
```

```rust
use quick_xml::{Reader, events::Event};
pub fn parse_atom_body(body: &str) -> Result<Vec<ArxivEntry>, ArxivError> {
    let mut r = Reader::from_str(body);
    r.trim_text(true);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut cur: Option<ArxivEntry> = None;
    let mut field = String::new();
    loop {
        match r.read_event_into(&mut buf).map_err(|e| ArxivError::XmlParse(e.to_string()))? {
            Event::Start(e) => {
                let n = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match n.as_str() {
                    "entry" => cur = Some(ArxivEntry::default()),
                    "title"|"summary"|"id"|"published"|"updated" => field = n,
                    "name" => field = n,
                    "link" => {
                        let (mut href, mut rel, mut title) = (String::new(), String::new(), String::new());
                        for a in e.attributes().flatten() {
                            match String::from_utf8_lossy(a.key.as_ref()).as_ref() {
                                "href" => href = String::from_utf8_lossy(&a.value).into_owned(),
                                "rel" => rel = String::from_utf8_lossy(&a.value).into_owned(),
                                "title" => title = String::from_utf8_lossy(&a.value).into_owned(),
                                _ => {}
                            }
                        }
                        if let Some(c) = cur.as_mut() {
                            if rel == "related" && title == "pdf" { c.pdf_url = href; }
                            if rel == "alternate" { c.abs_url = href; }
                        }
                    }
                    "category" => {
                        for a in e.attributes().flatten() {
                            if String::from_utf8_lossy(a.key.as_ref()) == "term" {
                                if let Some(c) = cur.as_mut() { c.categories.push(String::from_utf8_lossy(&a.value).into_owned()); }
                            }
                        }
                    }
                    _ => field.clear(),
                }
            }
            Event::Text(t) => {
                let v = t.xml_content().map_err(|e| ArxivError::XmlParse(e.to_string()))?.into_owned();
                if let Some(c) = cur.as_mut() {
                    match field.as_str() {
                        "title" => c.title.push_str(&v),
                        "summary" => c.summary.push_str(&v),
                        "id" => { c.abs_url = v.clone(); }
                        "published" => c.published = v,
                        "updated" => c.updated = v,
                        "name" => c.authors.push(v.trim().to_owned()),
                        _ => {}
                    }
                }
            }
            Event::End(e) => {
                if String::from_utf8_lossy(e.local_name().as_ref()) == "entry" {
                    if let Some(mut c) = cur.take() {
                        let (id, ver) = parse_arxiv_id(c.abs_url.rsplit('/').next().unwrap_or(""))?;
                        c.id = id; c.version = ver;
                        if c.pdf_url.is_empty() { c.pdf_url = format!("https://arxiv.org/pdf/{}", c.id); }
                        out.push(c);
                    }
                }
                field.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pdf_core --test arxiv`
Expected: PASS. Además `cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used`.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_core/src/arxiv.rs crates/pdf_core/Cargo.toml crates/pdf_core/tests/arxiv.rs
git commit -m "feat(core): arxiv search with streaming atom parser"
```

### Task 3: Descarga throttled + streaming .part

**Files:**
- Modify: `crates/pdf_core/src/arxiv.rs`
- Test: `crates/pdf_core/tests/arxiv.rs`

**Interfaces:**
- Consumes: `ArxivClient`, `ArxivError` de Tasks 1-2.
- Produces: `pub struct DownloadOutcome { pub final_name: String, pub etag: Option<String>, pub bytes: u64 }`, `impl ArxivClient { pub fn download_to(&self, id: &str, w: &mut dyn std::io::Write) -> Result<DownloadOutcome, ArxivError> }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn download_streams_pdf_and_reads_disposition() {
    let srv = spawn_pdf_fixture(b"%PDF-1.4 fake", "1706.03762v7.pdf");
    let c = ArxivClient::for_tests(&srv);
    let mut buf: Vec<u8> = Vec::new();
    let out = c.download_to("1706.03762", &mut buf).unwrap();
    assert!(buf.starts_with(b"%PDF-"));
    assert_eq!(out.final_name, "1706.03762v7.pdf");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pdf_core --test arxiv download_streams_pdf_and_reads_disposition`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
impl ArxivClient {
    pub fn download_to(&self, id: &str, w: &mut dyn std::io::Write) -> Result<DownloadOutcome, ArxivError> {
        let (base, _) = parse_arxiv_id(id)?;
        self.throttle();
        let url = format!("{}/pdf/{}", self.pdf_base, base);
        let mut resp = self.client.get(&url).send().map_err(|e| ArxivError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 429 || status == 503 { return Err(ArxivError::RateLimited); }
        if status != 200 { return Err(ArxivError::Http{status, body: String::new()}); }
        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        if !ct.contains("application/pdf") { return Err(ArxivError::Http{status, body: format!("bad content-type {ct}")}); }
        let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(|s| s.to_owned());
        let final_name = resp.headers().get("content-disposition").and_then(|v| v.to_str().ok())
            .and_then(|d| d.rsplit("filename=").next()).map(|s| s.trim_matches(['"',' ']).to_owned())
            .unwrap_or_else(|| format!("{base}.pdf"));
        let n = std::io::copy(&mut resp, w).map_err(|e| ArxivError::Io(e.to_string()))?;
        Ok(DownloadOutcome { final_name, etag, bytes: n })
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pdf_core --test arxiv`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_core/src/arxiv.rs crates/pdf_core/tests/arxiv.rs
git commit -m "feat(core): throttled arxiv pdf download with disposition"
```

### Task 4: persist papers.json + DiscoverPrefs

**Files:**
- Modify: `crates/pdf_android/src/persist.rs`
- Test: `cargo test -p pdf_core` (funciones puras, sin device)

**Interfaces:**
- Consumes: `ArxivEntry` de Task 2.
- Produces: `pub struct PaperMeta { path, arxiv_id, title, authors: Vec<String>, updated: String }`, `pub fn touch_paper(papers: &[PaperMeta], m: PaperMeta) -> Vec<PaperMeta>`, `pub struct DiscoverPrefs { cats: Vec<String> }`, `pub fn load_papers/save_papers/load_discover/save_discover` (best-effort, nunca propagan).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn touch_paper_upserts_by_path() {
    let a = touch_paper(&[], PaperMeta::test("a.pdf", "2401.1"));
    let b = touch_paper(&a, PaperMeta::test("a.pdf", "2401.1"));
    assert_eq!(a.len(), 1); assert_eq!(b.len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pdf_core touch_paper_upserts_by_path` (o test local equivalente si vive en android-stub)
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

```rust
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct PaperMeta { pub path: String, pub arxiv_id: String, #[serde(default)] pub title: String, #[serde(default)] pub authors: Vec<String>, #[serde(default)] pub updated: String }
pub fn touch_paper(papers: &[PaperMeta], m: PaperMeta) -> Vec<PaperMeta> {
    let mut v: Vec<PaperMeta> = papers.iter().filter(|p| p.path != m.path).cloned().collect();
    v.push(m); v
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pdf_core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/persist.rs
git commit -m "feat(android): papers metadata and discover prefs store"
```

### Task 5: DiscoverWorker + DiscoverState + tick

**Files:**
- Create: `crates/pdf_android/src/discover.rs`, `crates/pdf_android/src/reader/discover_state.rs`, `crates/pdf_android/src/reader/discover.rs`, `crates/pdf_android/src/reader/discover_categories.rs`
- Modify: `crates/pdf_android/src/reader/mod.rs`, `crates/pdf_android/src/reader/tick.rs`

**Interfaces:**
- Consumes: `ArxivClient/download_to/search/fetch_by_ids`, `touch_paper`, `touch_progress` existentes.
- Produces: `pub enum DiscoverCmd { Feed, Search(String), More, Download(String), Cancel }`, `pub enum DiscoverMsg { ... }`, `pub fn pump_discover(app)`, `pub fn discover_busy(&self) -> bool`.

- [ ] **Step 1: Write the failing test** (lógica pura de paginación/caché, sin device)

```rust
#[test]
fn feed_cache_key_stable() {
    assert_eq!(feed_cache_key(&["cs.AI".into()], 0), feed_cache_key(&["cs.AI".into()], 0));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pdf_core feed_cache_key_stable`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation** (worker 1 hilo + `AtomicBool` cancel + `FeedCache` ≤4MiB LRU TTL 10min; `pump_discover` = `try_recv` + `library_add_entry` tras rename; `needs_tick` incluye `discover_busy()`).

- [ ] **Step 4: Run host checks**

Run: `cargo test -p pdf_core && cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/discover.rs crates/pdf_android/src/reader/discover*.rs crates/pdf_android/src/reader/tick.rs crates/pdf_android/src/reader/mod.rs
git commit -m "feat(android): discover worker with bg fetch and tick pump"
```

### Task 6: UI Discover (geometry/redraw/draw/motion)

**Files:**
- Modify: `reader/geometry.rs`, `reader/redraw.rs`, `draw/discover.rs` (nuevo), `draw/mod.rs`, `draw/library.rs`, `input/motion.rs`, `reader/library.rs`, `gpu/pipeline.rs`

**Interfaces:**
- Consumes: `DiscoverState`, `pump_discover`, `discover_busy` de Task 5.
- Produces: pestaña Biblioteca/Descubrir en header, pantallas Feed/Buscar/Ficha/Áreas, tarjetas + ficha-antes-descargar + etiquetas Preprint/Open-access + atribución arXiv.

- [ ] **Step 1: Build para tablet y medir regresión**

Run: `cargo apk build -p pdf_android --release --target aarch64-linux-android`
Expected: compila; scroll rejilla sigue p95 <16ms (screencap + dumpsys en TCL).

- [ ] **Step 2-4: Implementar tabs + lista + ficha por slices** (cada slice con rebuild_* análogo a library, splice de fila para progreso, routing Discover en los 2 `match self.mode` de redraw.rs y 3 de motion.rs).

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/src/reader/geometry.rs crates/pdf_android/src/reader/redraw.rs crates/pdf_android/src/draw/discover.rs crates/pdf_android/src/input/motion.rs
git commit -m "feat(android): discover tab with feed search and detail card"
```

### Task 7: Handoff PaperTok→PDFLector

**Files:**
- Modify: `crates/pdf_android/Cargo.toml`, `crates/pdf_android/src/jni.rs`, `crates/pdf_android/src/reader/mod.rs`, `crates/pdf_android/src/reader/life.rs`

**Interfaces:**
- Consumes: `parse_arxiv_id`, `download_to`, `DiscoverWorker` de Tasks 1-5.
- Produces: `pub enum LaunchRequest { File(LaunchPdf), Remote(String) }`, `pub fn launch_intent_request(app) -> Option<LaunchRequest>`.

- [ ] **Step 1: Verificar filtros generados**

```bash
cargo apk build -p pdf_android --release --target aarch64-linux-android
adb shell dumpsys package com.pdflector.app | grep -A3 IntentFilter
```

Filtros nuevos:

```toml
[[package.metadata.android.application.activity.intent_filter]]
actions = ["android.intent.action.VIEW"]
categories = ["android.intent.category.DEFAULT", "android.intent.category.BROWSABLE"]
[[package.metadata.android.application.activity.intent_filter.data]]
scheme = "pdflector"
[[package.metadata.android.application.activity.intent_filter]]
actions = ["android.intent.action.SEND"]
categories = ["android.intent.category.DEFAULT"]
[[package.metadata.android.application.activity.intent_filter.data]]
mime_type = "text/plain"
```

- [ ] **Step 2: Probar handoff por adb (sin PaperTok)**

```bash
adb shell am start -W -a android.intent.action.VIEW -d "pdflector://arxiv/2401.12345"
adb shell am start -W -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT "https://arxiv.org/abs/2401.12345"
```

Expected: arranca, status "Descargando…", PDF en `internal/pdfs/`, entrada en biblioteca.

- [ ] **Step 3: Implementar `launch_intent_request`** (leer `getAction` + `EXTRA_TEXT` antes del early-return `data.is_null`; brazo `pdflector` percent-decode; parser acepta `arxiv.org/abs|pdf`, `export.arxiv.org`, `papertok.app/#/public/paper/<base64url arxiv:>`, `pdflector://arxiv/<id>`, ID suelto; no-arXiv → status sin tocar biblioteca).

- [ ] **Step 4: Rama `Remote(id)` en `life.rs:163`** (spawn worker, streaming `.part`→rename, `touch_progress`+`papers.json`+`reload_curated_library`+`open_library_entry`).

- [ ] **Step 5: Commit**

```bash
git add crates/pdf_android/Cargo.toml crates/pdf_android/src/jni.rs crates/pdf_android/src/reader/mod.rs crates/pdf_android/src/reader/life.rs
git commit -m "feat(android): papertok handoff via scheme and send"
```

Petición a PaperTok (fuera de este repo, issue-first): botón en tarjeta handoff existente (`PDFViewer.jsx` coarse-pointer) con `intent://arxiv/{id}#Intent;scheme=pdflector;package=com.pdflector.app;S.browser_fallback_url=https%3A%2F%2Farxiv.org%2Fpdf%2F{id};end`, copy ES/EN, test helper, flag `VITE_PDFLECTOR_ENABLED` en `deployFlags.js`.

## Self-review

- Spec §1-§6 cubiertos: T1 IDs, T2 search/parser, T3 throttle/descarga, T4 storage, T5 worker/tick, T6 UI, T7 handoff. Contradicciones InAppDesigner resueltas: nombre local `arxiv_{id}.pdf`, Fase C incluida (usuario la exige), quick-xml como crate nuevo en `.so` declarado.
- Sin placeholders: cada paso trae código/comando/expected. Tipos consistentes: `ArxivEntry/ArxivError/ArxivClient/DownloadOutcome/DiscoverCmd-Msg/LaunchRequest/PaperMeta` verbatim en todas las tareas.
