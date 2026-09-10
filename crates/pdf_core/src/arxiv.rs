// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pure logic for arXiv integration: ID parsing, query builder, and types.
//!
//! UI-independent and free of `unwrap`/`expect` outside of unit tests.

/// Category of an arXiv identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArxivId {
    /// Modern 5-digit identifier (e.g. `2401.12345`).
    Modern(String),
    /// Modern 4-digit identifier (e.g. `0706.0001`).
    Modern4(String),
    /// Classic identifier with archive/subject (e.g. `hep-th/9901001`).
    Classic(String),
}

/// Errors produced by arXiv operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArxivError {
    InvalidId(String),
    Network(String),
    Http { status: u16, body: String },
    RateLimited,
    XmlParse(String),
    Io(String),
}

impl std::fmt::Display for ArxivError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidId(id) => write!(f, "invalid arXiv identifier: {id}"),
            Self::Network(msg) => write!(f, "arXiv network error: {msg}"),
            Self::Http { status, body } => write!(f, "arXiv HTTP error {status}: {body}"),
            Self::RateLimited => write!(f, "arXiv rate limited"),
            Self::XmlParse(msg) => write!(f, "arXiv XML parse error: {msg}"),
            Self::Io(msg) => write!(f, "arXiv I/O error: {msg}"),
        }
    }
}

impl std::error::Error for ArxivError {}

/// Parses a raw string (ID, URL, or prefixed string) into `(base_id, Option<version>)`.
///
/// Converts prefixes like `https://arxiv.org/abs/`, `arXiv:`, etc.
/// Note: `/` is preserved in classic IDs (e.g. `hep-th/9901001`); local filename conversion to `_`
/// must only happen when writing to disk, never in URLs or API calls.
pub fn parse_arxiv_id(raw: &str) -> Result<(String, Option<String>), ArxivError> {
    let mut s = raw.trim().to_owned();
    for p in [
        "https://arxiv.org/abs/",
        "https://arxiv.org/pdf/",
        "http://arxiv.org/abs/",
        "http://arxiv.org/pdf/",
        "arXiv:",
        "arxiv:",
    ] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_owned();
            break;
        }
    }
    let s = s.strip_suffix(".pdf").unwrap_or(&s).trim().to_owned();
    if s.contains('_') {
        return Err(ArxivError::InvalidId(raw.into()));
    }
    let (base, ver) = match s.rfind('v') {
        Some(i) if s[i + 1..].chars().all(|c| c.is_ascii_digit()) && !s[i + 1..].is_empty() => {
            (s[..i].to_owned(), Some(s[i + 1..].to_owned()))
        }
        _ => (s.clone(), None),
    };
    let ok = base.contains('/') || (base.contains('.') && base.len() >= 8);
    if !ok {
        return Err(ArxivError::InvalidId(raw.into()));
    }
    Ok((base, ver))
}

/// Query builder for the arXiv API.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArxivQuery {
    pub search_query: Option<String>,
    pub id_list: Vec<String>,
    pub start: Option<usize>,
    pub max_results: Option<usize>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
}

impl ArxivQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_search_query(mut self, query: impl Into<String>) -> Self {
        self.search_query = Some(query.into());
        self
    }

    pub fn with_id_list(mut self, ids: Vec<String>) -> Self {
        self.id_list = ids;
        self
    }

    pub fn with_start(mut self, start: usize) -> Self {
        self.start = Some(start);
        self
    }

    pub fn with_max_results(mut self, max_results: usize) -> Self {
        self.max_results = Some(max_results);
        self
    }

    pub fn with_sort_by(mut self, sort_by: impl Into<String>) -> Self {
        self.sort_by = Some(sort_by.into());
        self
    }

    pub fn with_sort_order(mut self, sort_order: impl Into<String>) -> Self {
        self.sort_order = Some(sort_order.into());
        self
    }

    /// Converts query parameters into key-value pairs for HTTP GET request query string.
    pub fn to_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();
        if let Some(sq) = &self.search_query {
            params.push(("search_query".to_string(), sq.clone()));
        }
        if !self.id_list.is_empty() {
            params.push(("id_list".to_string(), self.id_list.join(",")));
        }
        if let Some(start) = self.start {
            params.push(("start".to_string(), start.to_string()));
        }
        if let Some(max_results) = self.max_results {
            params.push(("max_results".to_string(), max_results.to_string()));
        }
        if let Some(sb) = &self.sort_by {
            params.push(("sortBy".to_string(), sb.clone()));
        }
        if let Some(so) = &self.sort_order {
            params.push(("sortOrder".to_string(), so.clone()));
        }
        params
    }
}

/// Default User-Agent header identifying the app according to arXiv API policy.
pub const ARXIV_USER_AGENT: &str =
    "PDFLector/0.1 (+https://github.com/asierboveda/pdflector; contact@pdflector.app)";

const ARXIV_DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const ARXIV_DEFAULT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Metadata of a single arXiv paper entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ArxivEntry {
    pub id: String,
    pub version: Option<String>,
    pub title: String,
    pub summary: String,
    pub authors: Vec<String>,
    pub primary_category: String,
    pub categories: Vec<String>,
    pub published: String,
    pub updated: String,
    pub pdf_url: String,
    pub abs_url: String,
    pub doi: Option<String>,
}

/// Result of a successful paper PDF download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadOutcome {
    pub final_name: String,
    pub etag: Option<String>,
    pub bytes: u64,
}

/// Client for searching and retrieving papers from the arXiv API.
pub struct ArxivClient {
    api_base: String,
    pub pdf_base: String,
    client: reqwest::blocking::Client,
    last_request: std::sync::Mutex<Option<std::time::Instant>>,
    min_interval: std::time::Duration,
}

impl ArxivClient {
    /// Creates a new production arXiv client with rate-limiting and default User-Agent.
    pub fn new() -> Result<Self, ArxivError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(ARXIV_USER_AGENT)
            .timeout(ARXIV_DEFAULT_TIMEOUT)
            .build()
            .map_err(|e| ArxivError::Network(e.to_string()))?;

        Ok(Self {
            api_base: "https://export.arxiv.org/api".to_string(),
            pdf_base: "https://arxiv.org".to_string(),
            client,
            last_request: std::sync::Mutex::new(None),
            min_interval: ARXIV_DEFAULT_INTERVAL,
        })
    }

    /// Test constructor allowing redirection to a local HTTP mock server without rate limiting delay.
    #[doc(hidden)]
    pub fn for_tests(base_url: &str) -> Self {
        let trimmed = base_url.trim_end_matches('/');
        let client = reqwest::blocking::Client::builder()
            .user_agent(ARXIV_USER_AGENT)
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| ArxivError::Network(e.to_string()))
            .unwrap_or_default();

        Self {
            api_base: format!("{trimmed}/api"),
            pdf_base: trimmed.to_string(),
            client,
            last_request: std::sync::Mutex::new(None),
            min_interval: std::time::Duration::ZERO,
        }
    }

    /// Throttles requests according to the minimum interval (1 req / 3s by default).
    pub fn throttle(&self) {
        let mut last = match self.last_request.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                std::thread::sleep(self.min_interval - elapsed);
            }
        }
        *last = Some(std::time::Instant::now());
    }

    /// Performs an arXiv search query and returns the matching entries.
    pub fn search(&self, q: &ArxivQuery) -> Result<Vec<ArxivEntry>, ArxivError> {
        self.throttle();
        let url = format!("{}/query", self.api_base);
        let mut req = self.client.get(&url);
        for (k, v) in q.to_params() {
            req = req.query(&[(k, v)]);
        }
        let resp = req.send().map_err(|e| ArxivError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 429 || status == 503 {
            return Err(ArxivError::RateLimited);
        }
        let bytes = resp
            .bytes()
            .map_err(|e| ArxivError::Network(e.to_string()))?;
        let body = String::from_utf8_lossy(&bytes);
        if status != 200 {
            return Err(ArxivError::Http {
                status,
                body: body.into_owned(),
            });
        }
        parse_atom_body(&body)
    }

    /// Fetches papers by a list of arXiv IDs.
    pub fn fetch_by_ids(&self, ids: &[&str]) -> Result<Vec<ArxivEntry>, ArxivError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut parsed_ids = Vec::with_capacity(ids.len());
        for raw in ids {
            let (id, ver) = parse_arxiv_id(raw)?;
            let id_str = match ver {
                Some(v) => format!("{id}v{v}"),
                None => id,
            };
            parsed_ids.push(id_str);
        }
        let query = ArxivQuery::new()
            .with_id_list(parsed_ids)
            .with_max_results(ids.len());
        self.search(&query)
    }

    /// Downloads a paper PDF by arXiv identifier, streaming content to the given writer.
    ///
    /// Respects the rate-limiting throttle, verifies `application/pdf` content-type,
    /// extracts resolved version/filename from `Content-Disposition`, and extracts `ETag` if present.
    pub fn download_to(
        &self,
        id: &str,
        w: &mut dyn std::io::Write,
    ) -> Result<DownloadOutcome, ArxivError> {
        let (base, _) = parse_arxiv_id(id)?;
        self.throttle();
        let url = format!("{}/pdf/{}", self.pdf_base, base);
        let mut resp = self
            .client
            .get(&url)
            .send()
            .map_err(|e| ArxivError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status == 429 || status == 503 {
            return Err(ArxivError::RateLimited);
        }
        if status != 200 {
            return Err(ArxivError::Http {
                status,
                body: String::new(),
            });
        }
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !ct.contains("application/pdf") {
            return Err(ArxivError::Http {
                status,
                body: format!("bad content-type {ct}"),
            });
        }
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        let final_name = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_disposition_filename)
            .unwrap_or_else(|| format!("{base}.pdf"));
        let bytes = std::io::copy(&mut resp, w).map_err(|e| ArxivError::Io(e.to_string()))?;
        Ok(DownloadOutcome {
            final_name,
            etag,
            bytes,
        })
    }
}

/// Test helper exposing `parse_atom_body`.
pub fn parse_atom_for_test(body: &str) -> Result<Vec<ArxivEntry>, ArxivError> {
    parse_atom_body(body)
}

fn parse_content_disposition_filename(disposition: &str) -> Option<String> {
    for part in disposition.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=')
            && k.trim().eq_ignore_ascii_case("filename")
        {
            let clean = v.trim().trim_matches(['"', ' ']);
            if !clean.is_empty() {
                return Some(clean.to_owned());
            }
        }
    }
    None
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn resolve_entity_ref(raw: &str) -> Option<char> {
    match raw {
        "gt" => Some('>'),
        "lt" => Some('<'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ if raw.starts_with("#x") || raw.starts_with("#X") => u32::from_str_radix(&raw[2..], 16)
            .ok()
            .and_then(char::from_u32),
        _ if raw.starts_with('#') => raw[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}
fn handle_element_attributes(e: &quick_xml::events::BytesStart<'_>, cur: &mut Option<ArxivEntry>) {
    let n = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
    match n.as_str() {
        "link" => {
            let mut href = String::new();
            let mut rel = String::new();
            let mut title = String::new();
            for a in e.attributes().flatten() {
                let key = String::from_utf8_lossy(a.key.as_ref());
                let val = String::from_utf8_lossy(&a.value).into_owned();
                match key.as_ref() {
                    "href" => href = val,
                    "rel" => rel = val,
                    "title" => title = val,
                    _ => {}
                }
            }
            if let Some(c) = cur.as_mut() {
                if rel == "related" && title == "pdf" {
                    c.pdf_url = href;
                } else if rel == "alternate" {
                    c.abs_url = href;
                }
            }
        }
        "category" => {
            if let Some(c) = cur.as_mut() {
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == b"term" {
                        let term = String::from_utf8_lossy(&a.value).into_owned();
                        c.categories.push(term);
                    }
                }
            }
        }
        "primary_category" => {
            if let Some(c) = cur.as_mut() {
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == b"term" {
                        let term = String::from_utf8_lossy(&a.value).into_owned();
                        c.primary_category = term;
                    }
                }
            }
        }
        _ => {}
    }
}

/// Parses an Atom XML response body from arXiv into a vector of [`ArxivEntry`].
pub fn parse_atom_body(body: &str) -> Result<Vec<ArxivEntry>, ArxivError> {
    let mut r = quick_xml::Reader::from_str(body);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut cur: Option<ArxivEntry> = None;
    let mut field = String::new();
    let mut author_name = String::new();
    let mut doi_buf = String::new();

    loop {
        match r
            .read_event_into(&mut buf)
            .map_err(|e| ArxivError::XmlParse(e.to_string()))?
        {
            quick_xml::events::Event::Start(e) => {
                let n = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match n.as_str() {
                    "entry" => cur = Some(ArxivEntry::default()),
                    "title" | "summary" | "id" | "published" | "updated" | "name" | "doi" => {
                        field = n.clone();
                        if n == "name" {
                            author_name.clear();
                        } else if n == "doi" {
                            doi_buf.clear();
                        }
                    }
                    _ => field.clear(),
                }
                handle_element_attributes(&e, &mut cur);
            }
            quick_xml::events::Event::Empty(e) => {
                handle_element_attributes(&e, &mut cur);
            }
            quick_xml::events::Event::GeneralRef(r) => {
                let raw = std::str::from_utf8(r.as_ref())
                    .map_err(|e| ArxivError::XmlParse(e.to_string()))?;
                if let (Some(ch), Some(c)) = (resolve_entity_ref(raw), cur.as_mut()) {
                    match field.as_str() {
                        "title" => c.title.push(ch),
                        "summary" => c.summary.push(ch),
                        "name" => author_name.push(ch),
                        _ => {}
                    }
                }
            }
            quick_xml::events::Event::Text(t) => {
                let raw = std::str::from_utf8(t.as_ref())
                    .map_err(|e| ArxivError::XmlParse(e.to_string()))?;
                let v = quick_xml::escape::unescape(raw)
                    .map_err(|e| ArxivError::XmlParse(e.to_string()))?;
                if let Some(c) = cur.as_mut() {
                    match field.as_str() {
                        "title" => c.title.push_str(&v),
                        "summary" => c.summary.push_str(&v),
                        "id" => {
                            c.abs_url = v.trim().to_string();
                        }
                        "published" => c.published.push_str(v.trim()),
                        "updated" => c.updated.push_str(v.trim()),
                        "name" => author_name.push_str(&v),
                        "doi" => doi_buf.push_str(v.trim()),
                        _ => {}
                    }
                }
            }
            quick_xml::events::Event::End(e) => {
                let n = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match n.as_str() {
                    "entry" => {
                        if let Some(mut c) = cur.take() {
                            c.title = collapse_whitespace(&c.title);
                            c.summary = collapse_whitespace(&c.summary);
                            let raw_id = if !c.abs_url.is_empty() {
                                c.abs_url.clone()
                            } else if !c.id.is_empty() {
                                c.id.clone()
                            } else {
                                String::new()
                            };
                            let (id, ver) = parse_arxiv_id(&raw_id)
                                .or_else(|_| {
                                    parse_arxiv_id(raw_id.rsplit('/').next().unwrap_or(""))
                                })
                                .map_err(|e| {
                                    ArxivError::XmlParse(format!(
                                        "invalid arxiv id in entry '{raw_id}': {e}"
                                    ))
                                })?;
                            c.id = id;
                            c.version = ver;
                            if c.pdf_url.is_empty() && !c.id.is_empty() {
                                c.pdf_url = format!("https://arxiv.org/pdf/{}", c.id);
                            }
                            if c.primary_category.is_empty() && !c.categories.is_empty() {
                                c.primary_category = c.categories[0].clone();
                            }
                            out.push(c);
                        }
                    }
                    "name" => {
                        let trimmed = author_name.trim();
                        if let Some(c) = cur.as_mut().filter(|_| !trimmed.is_empty()) {
                            c.authors.push(trimmed.to_string());
                        }
                        author_name.clear();
                    }
                    "doi" => {
                        let trimmed = doi_buf.trim();
                        if let Some(c) = cur.as_mut().filter(|_| !trimmed.is_empty()) {
                            c.doi = Some(trimmed.to_string());
                        }
                        doi_buf.clear();
                    }
                    _ => {}
                }
                field.clear();
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}
