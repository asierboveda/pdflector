//! Fase 5: consulta a un LLM local vía Ollama (docs/PLAN.md §5).
//!
//! Two pieces, both UI-independent (pdf_core rules §4):
//!
//! 1. `chunk_pages` — pure text chunking: turns lazy page text into
//!    LLM-friendly chunks, each carrying a page-range prefix so the answer
//!    can cite pages.
//! 2. `OllamaClient` — a minimal HTTP client for Ollama's `/api/chat`
//!    endpoint built on `std::net::TcpStream` only (no new dependencies,
//!    no TLS: Ollama runs on the same LAN, plain HTTP).
//!
//! The app never runs models: it sends the (chunked) text over HTTP and
//! reads back the assistant's reply. `pdf_app` is expected to call `chat`
//! from a background task (generation can take minutes).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::Document;

/// How long a single request may take before we give up. Ollama with
/// `stream: false` answers only when the model finished generating; a CPU
/// laptop model on a long prompt can take minutes, so the timeout is
/// generous. Callers must run `chat` off the UI thread (see module docs).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Errors produced by the ai module.
///
/// Deliberately a new type instead of extending `crate::Error`: the core
/// `Error` enum lives in `engine.rs` and adding an HTTP/LLM category there
/// would pull network concerns into the render path. `ai` is an opt-in
/// layer on top of the core; `AiError::Text` adapts text-extraction errors
/// so callers get one error type for the whole pipeline.
#[derive(Debug)]
pub enum AiError {
    /// Ollama could not be reached (connection refused, DNS failure,
    /// connect timeout). Carries the offending URL for diagnostics.
    NotReachable { url: String, source: std::io::Error },
    /// The server answered with a non-200 status.
    Http { status: u16, body: String },
    /// Page text extraction failed (e.g. page out of range).
    Text(crate::Error),
    /// Invalid arguments to an ai API (e.g. `max_chars == 0`).
    InvalidArgument(String),
    /// The response body was not valid JSON / lacked `message.content`.
    Json(serde_json::Error),
    /// I/O failure mid-request (write or read, including read timeouts).
    Io(std::io::Error),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::NotReachable { url, source } => {
                write!(f, "ollama not reachable at {url}: {source}")
            }
            AiError::Http { status, body } => write!(f, "ollama http error {status}: {body}"),
            AiError::Text(e) => write!(f, "text extraction failed: {e}"),
            AiError::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            AiError::Json(e) => write!(f, "invalid ollama response: {e}"),
            AiError::Io(e) => write!(f, "ollama request failed: {e}"),
        }
    }
}

impl std::error::Error for AiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AiError::NotReachable { source, .. } => Some(source),
            AiError::Text(e) => Some(e),
            AiError::Json(e) => Some(e),
            AiError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AiError {
    fn from(e: std::io::Error) -> Self {
        AiError::Io(e)
    }
}

impl From<serde_json::Error> for AiError {
    fn from(e: serde_json::Error) -> Self {
        AiError::Json(e)
    }
}

impl From<crate::Error> for AiError {
    fn from(e: crate::Error) -> Self {
        AiError::Text(e)
    }
}

pub type Result<T> = std::result::Result<T, AiError>;

// ---------------------------------------------------------------------------
// Chunking
// ---------------------------------------------------------------------------

/// Chunking policy (documented; verified by tests/ai.rs):
///
/// * Text is extracted lazily, only for the pages in `pages` — nothing else
///   in the document is touched (Fase 5 never renders, never preloads).
/// * Page indices are **0-based** (same convention as `Document::text`);
///   the prefix shows **1-based** page numbers, i.e. the numbers a user sees
///   in a PDF viewer, so the LLM cites human-readable pages.
/// * Pages whose text is empty/whitespace are skipped (no chunk is emitted
///   for them), but they still count for the caller-provided ordering.
/// * Chunks never split a word: text is tokenized on whitespace and words
///   are greedily packed into a chunk while `body.len() <= max_chars`.
/// * Consecutive small pages are merged into one chunk (prefix `[págs 3-5]`);
///   a chunk may also hold part of a single page (prefix `[págs 5]`).
/// * A single word longer than `max_chars` overflows by design: it forms its
///   own chunk (prefix still applied) rather than being cut in half.
/// * The `[págs N-M]` prefix adds a small fixed overhead (~10-15 chars) on
///   top of `max_chars`; the body text itself is always `<= max_chars`.
/// * `pages` must be ascending and duplicate-free (the caller builds the
///   selection; the prefix ranges only make sense on sorted input).
pub fn chunk_pages(doc: &dyn Document, pages: &[u32], max_chars: usize) -> Result<Vec<String>> {
    if max_chars == 0 {
        return Err(AiError::InvalidArgument(
            "max_chars must be > 0".to_string(),
        ));
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut body = String::new();
    // Page range covered by the current chunk body (0-based internally,
    // printed 1-based by `finish_chunk`).
    let mut range_start: Option<u32> = None;
    let mut range_end: u32 = 0;

    for &page in pages {
        let page_text = doc.text(page)?.text;
        let words: Vec<&str> = page_text.split_whitespace().collect();
        if words.is_empty() {
            continue; // empty page: nothing to chunk (policy above)
        }
        if range_start.is_none() {
            range_start = Some(page);
        }
        range_end = page;

        for word in words {
            // Would appending this word blow the budget? If so, flush the
            // current chunk and start a fresh one (still on this page).
            let needed = if body.is_empty() {
                word.len()
            } else {
                body.len() + 1 + word.len()
            };
            if needed > max_chars && !body.is_empty() {
                chunks.push(finish_chunk(
                    range_start.expect("set above"),
                    range_end,
                    &body,
                ));
                body.clear();
                range_start = Some(page);
                range_end = page;
            }
            if body.is_empty() {
                body.push_str(word);
            } else {
                body.push(' ');
                body.push_str(word);
            }
        }
    }

    if !body.is_empty() {
        chunks.push(finish_chunk(
            range_start.expect("set above"),
            range_end,
            &body,
        ));
    }
    Ok(chunks)
}

/// Builds the user-facing chunk: `[págs N-M]\n<body>\n` (single page renders
/// as `[págs N]`). Page numbers are 1-based, as printed in PDF viewers.
fn finish_chunk(start: u32, end: u32, body: &str) -> String {
    let range = if start == end {
        format!("págs {}", start + 1)
    } else {
        format!("págs {}-{}", start + 1, end + 1)
    };
    format!("[{range}]\n{body}\n")
}

// ---------------------------------------------------------------------------
// Ollama HTTP client
// ---------------------------------------------------------------------------

/// Minimal Ollama client over `std::net::TcpStream` (no new dependencies).
///
/// Talks plain HTTP to `/api/chat` with `stream: false` and reads the
/// assistant message back. Only `http://` base URLs are supported (no TLS —
/// Ollama is expected on the local network; document this if it ever moves
/// off-LAN). IPv6 literals are not supported.
pub struct OllamaClient {
    base_url: String,
    model: String,
}

impl OllamaClient {
    /// Ollama on the default localhost endpoint.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
            model: model.into(),
        }
    }

    /// Ollama on a custom endpoint, e.g. `http://192.168.1.10:11434`.
    pub fn with_base_url(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
        }
    }

    /// The configured endpoint (mostly for diagnostics / future UI display).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Sends a single-turn chat request and returns the assistant's reply.
    ///
    /// POSTs `{"model":..., "messages":[{system},{user}], "stream":false}`
    /// to `<base_url>/api/chat` and parses `message.content` from the JSON
    /// response. Connection failures surface as `AiError::NotReachable`
    /// ("ollama not reachable at <url>"); non-200 responses as
    /// `AiError::Http`. May block for up to `REQUEST_TIMEOUT` while the
    /// model generates — call off the UI thread.
    pub fn chat(&self, system: &str, prompt: &str) -> Result<String> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": prompt },
            ],
            "stream": false,
        })
        .to_string();

        let (host, port) = parse_base_url(&self.base_url)?;
        let mut stream =
            TcpStream::connect((host.as_str(), port)).map_err(|source| AiError::NotReachable {
                url: self.base_url.clone(),
                source,
            })?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;

        let host_header = if port == 80 {
            host
        } else {
            format!("{host}:{port}")
        };
        let request = format!(
            "POST /api/chat HTTP/1.1\r\n\
             Host: {host_header}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        );
        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        let (status, response_body) = read_http_response(&mut stream)?;
        if status != 200 {
            return Err(AiError::Http {
                status,
                body: response_body,
            });
        }

        #[derive(serde::Deserialize)]
        struct ChatResponse {
            message: ChatMessage,
        }
        #[derive(serde::Deserialize)]
        struct ChatMessage {
            content: String,
        }

        let parsed: ChatResponse = serde_json::from_slice(response_body.as_bytes())?;
        Ok(parsed.message.content)
    }
}

/// Splits `http://host[:port]` into (host, port). Port defaults to 80 when
/// omitted; a trailing `/` is tolerated. Everything else (https, paths,
/// IPv6 literals) is rejected with a clear error — see `OllamaClient` docs.
fn parse_base_url(base_url: &str) -> Result<(String, u16)> {
    let rest = base_url.strip_prefix("http://").ok_or_else(|| {
        AiError::InvalidArgument(format!(
            "unsupported base_url {base_url:?}: only http:// is supported (no TLS)"
        ))
    })?;
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return Err(AiError::InvalidArgument(format!(
            "base_url {base_url:?} has no host"
        )));
    }
    if rest.contains('[') {
        return Err(AiError::InvalidArgument(format!(
            "base_url {base_url:?}: IPv6 literals are not supported"
        )));
    }
    match rest.split_once(':') {
        Some((host, port_part)) => {
            if port_part.contains('/') {
                return Err(AiError::InvalidArgument(format!(
                    "base_url {base_url:?}: path prefixes are not supported"
                )));
            }
            let port: u16 = port_part.parse().map_err(|_| {
                AiError::InvalidArgument(format!(
                    "base_url {base_url:?}: invalid port {port_part:?}"
                ))
            })?;
            if host.is_empty() {
                return Err(AiError::InvalidArgument(format!(
                    "base_url {base_url:?} has no host"
                )));
            }
            Ok((host.to_string(), port))
        }
        None => Ok((rest.to_string(), 80)),
    }
}

/// Reads a full HTTP response off the socket: headers up to `\r\n\r\n`,
/// then the body — exactly `Content-Length` bytes when the header says so,
/// otherwise until EOF (`Connection: close` guarantees the server closes).
/// Returns (status, body-as-utf8).
fn read_http_response(stream: &mut TcpStream) -> Result<(u16, String)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err(AiError::Http {
                status: 0,
                body: format!(
                    "connection closed before response headers (got {:?})",
                    String::from_utf8_lossy(&buf)
                ),
            });
        }
        buf.extend_from_slice(&tmp[..n]);
    };

    let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let status = parse_status(&headers);
    let content_length = parse_content_length(&headers);

    let mut body = buf[header_end + 4..].to_vec();
    if let Some(len) = content_length {
        while body.len() < len {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                break; // server closed early; keep what we got
            }
            body.extend_from_slice(&tmp[..n]);
        }
        body.truncate(len);
    } else {
        // No Content-Length: read until EOF (we asked for Connection: close).
        loop {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
    }

    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// Extracts the status code from the response status line
/// (`HTTP/1.1 200 OK` → 200). 0 if unparseable.
fn parse_status(headers: &str) -> u16 {
    headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// Content-Length header, case-insensitive. `None` when absent.
fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}
