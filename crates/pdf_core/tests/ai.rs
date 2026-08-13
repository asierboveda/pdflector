// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! ai module integration tests (Fase 5): chunking against a fake document
//! and the raw-TCP Ollama client against a local `TcpListener` — no real
//! Ollama, no MuPDF, no corpus.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use pdf_core::ai::{AiError, OllamaClient, chunk_pages};
use pdf_core::{Bitmap, Document, PageText};

// ---------------------------------------------------------------------------
// Fake document (same pattern as tests/zoom.rs)
// ---------------------------------------------------------------------------

/// Fixed page texts; page 1 is empty (no extractable text) to exercise the
/// skip-empty-pages policy. Page indices are 0-based.
const PAGE_TEXTS: &[&str] = &[
    "alpha beta gamma delta",      // page 0
    "",                            // page 1: empty, skipped
    "epsilon zeta",                // page 2
    "eta theta iota kappa lambda", // page 3
    "mu",                          // page 4
    "nu xi omicron pi rho sigma",  // page 5
];

struct FakeDoc;

impl Document for FakeDoc {
    fn page_count(&self) -> u32 {
        6
    }

    fn page_size(&self, _page: u32) -> pdf_core::Result<(f32, f32)> {
        Ok((100.0, 100.0))
    }

    fn render_page(&self, _page: u32, _scale: f32) -> pdf_core::Result<Bitmap> {
        Ok(Bitmap {
            width: 1,
            height: 1,
            data: vec![0; 4],
        })
    }

    fn text(&self, page: u32) -> pdf_core::Result<PageText> {
        match PAGE_TEXTS.get(page as usize) {
            Some(text) => Ok(PageText {
                text: (*text).to_string(),
                spans: Vec::new(),
            }),
            None => Err(pdf_core::Error::PageOutOfRange {
                page,
                page_count: 6,
            }),
        }
    }
}

/// Document whose page 0 contains one word longer than `max_chars`, to
/// exercise the documented overflow policy.
struct LongWordDoc;

impl Document for LongWordDoc {
    fn page_count(&self) -> u32 {
        1
    }

    fn page_size(&self, _page: u32) -> pdf_core::Result<(f32, f32)> {
        Ok((100.0, 100.0))
    }

    fn render_page(&self, _page: u32, _scale: f32) -> pdf_core::Result<Bitmap> {
        Ok(Bitmap {
            width: 1,
            height: 1,
            data: vec![0; 4],
        })
    }

    fn text(&self, _page: u32) -> pdf_core::Result<PageText> {
        Ok(PageText {
            text: format!("short {}", "x".repeat(100)),
            spans: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// chunk_pages
// ---------------------------------------------------------------------------

/// Each chunk body must respect `max_chars`, carry a page-range prefix in
/// the documented `[págs N-M]` / `[págs N]` format (1-based), skip empty
/// pages, and merge consecutive small pages.
#[test]
fn chunk_pages_respects_max_chars_and_prefixes_ranges() {
    let doc = FakeDoc;
    // pages 0,2,3,4,5 (page 1 empty -> skipped), max_chars = 40, packed
    // word-by-word:
    //   chunk 1: pages 0,2,3 -> "alpha beta gamma delta epsilon zeta eta"
    //            (39 chars; "theta" would exceed)            [págs 1-4]
    //   chunk 2: pages 3,4,5 -> "theta iota kappa lambda mu nu xi omicron"
    //            (40 chars; "pi" would exceed)               [págs 4-6]
    //   chunk 3: page 5 -> "pi rho sigma" (12)               [págs 6]
    let chunks = chunk_pages(&doc, &[0, 1, 2, 3, 4, 5], 40).expect("chunking succeeds");

    assert_eq!(chunks.len(), 3, "expected 3 chunks, got {chunks:?}");

    // Bodies (text after the prefix line) must respect max_chars.
    for chunk in &chunks {
        let body = chunk_body(chunk);
        assert!(
            body.len() <= 40,
            "chunk body {body:?} ({} chars) exceeds max_chars=40",
            body.len()
        );
    }

    // Prefixes: page 1 (0-based) is empty -> no chunk may claim page 2.
    assert!(chunks[0].starts_with("[págs 1-4]\n"));
    assert!(chunks[1].starts_with("[págs 4-6]\n"));
    assert!(chunks[2].starts_with("[págs 6]\n"));

    // Content lands in the right chunks (words preserved, single spaces).
    assert_eq!(
        chunk_body(&chunks[0]),
        "alpha beta gamma delta epsilon zeta eta"
    );
    assert_eq!(
        chunk_body(&chunks[1]),
        "theta iota kappa lambda mu nu xi omicron"
    );
    assert_eq!(chunk_body(&chunks[2]), "pi rho sigma");
}

/// A single page whose text exceeds `max_chars` is split into several
/// chunks, each still prefixed with that page.
#[test]
fn chunk_pages_splits_a_long_page_across_chunks() {
    let doc = FakeDoc;
    // Page 3 alone is 27 chars; with max_chars = 15:
    //   chunk 1: "eta theta iota" (14) -> [págs 4]
    //   chunk 2: "kappa lambda" (13)  -> [págs 4]
    let chunks = chunk_pages(&doc, &[3], 15).expect("chunking succeeds");

    assert_eq!(chunks.len(), 2, "got {chunks:?}");
    assert!(chunks[0].starts_with("[págs 4]\n"));
    assert!(chunks[1].starts_with("[págs 4]\n"));
    assert_eq!(chunk_body(&chunks[0]), "eta theta iota");
    assert_eq!(chunk_body(&chunks[1]), "kappa lambda");
    for chunk in &chunks {
        assert!(chunk_body(chunk).len() <= 15, "got {chunk:?}");
    }
}

/// A single word longer than `max_chars` is never cut in half: it overflows
/// into its own chunk (documented policy).
#[test]
fn chunk_pages_never_splits_a_word() {
    let doc = LongWordDoc;
    let chunks = chunk_pages(&doc, &[0], 10).expect("chunking succeeds");

    assert_eq!(chunks.len(), 2, "got {chunks:?}");
    assert!(chunks[0].starts_with("[págs 1]\n"));
    assert_eq!(chunk_body(&chunks[0]), "short");
    // Overflow chunk keeps the full word, uncut.
    assert!(chunks[1].starts_with("[págs 1]\n"));
    assert_eq!(chunk_body(&chunks[1]), "x".repeat(100));
}

#[test]
fn chunk_pages_propagates_text_errors() {
    let doc = FakeDoc;
    let err = chunk_pages(&doc, &[0, 99], 100).expect_err("page 99 is out of range");
    match err {
        AiError::Text(pdf_core::Error::PageOutOfRange { page: 99, .. }) => {}
        other => panic!("expected AiError::Text(PageOutOfRange), got {other:?}"),
    }
    assert!(err.to_string().contains("text extraction failed"));
}

#[test]
fn chunk_pages_rejects_zero_max_chars() {
    let doc = FakeDoc;
    let err = chunk_pages(&doc, &[0], 0).expect_err("max_chars = 0 is invalid");
    assert!(matches!(err, AiError::InvalidArgument(_)));
}

/// Returns the chunk body (everything after the first line, minus the
/// trailing newline the prefix format adds).
fn chunk_body(chunk: &str) -> &str {
    chunk
        .strip_prefix("[págs ")
        .expect("chunk must start with a [págs ...] prefix")
        .split_once('\n')
        .expect("prefix line must end with a newline")
        .1
        .trim_end_matches('\n')
}

// ---------------------------------------------------------------------------
// OllamaClient
// ---------------------------------------------------------------------------

/// Reads one full HTTP request off the socket (headers + Content-Length
/// body) and returns it as a string.
fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).expect("read request headers");
        if n == 0 {
            panic!("connection closed before request headers");
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let content_length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().expect("valid content-length"))
        })
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).expect("read request body");
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    format!("{headers}\r\n\r\n{}", String::from_utf8_lossy(&body))
}

/// Serves one request with a canned 200 JSON response and returns the
/// request it received (so the caller can assert on it).
fn serve_once(listener: TcpListener, response_body: &'static str) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let request = read_request(&mut sock);
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {response_body}",
            response_body.len()
        );
        sock.write_all(response.as_bytes()).expect("write response");
        sock.flush().expect("flush response");
        request
    })
}

#[test]
fn chat_parses_message_content_and_sends_the_right_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();

    let response_body = r#"{"model":"test-model","message":{"role":"assistant","content":"hola desde el servidor falso"},"done":true,"done_reason":"stop"}"#;
    let server = serve_once(listener, response_body);

    let client = OllamaClient::with_base_url(format!("http://127.0.0.1:{port}"), "test-model");
    let reply = client
        .chat("eres un ayudante", "¿qué dice la página 3?")
        .expect("chat succeeds");

    assert_eq!(reply, "hola desde el servidor falso");

    // The request we actually sent: correct endpoint, model and messages.
    // (serde_json sorts object keys, so assert key/value pairs independently.)
    let request = server.join().expect("server thread");
    assert!(
        request.starts_with("POST /api/chat HTTP/1.1"),
        "wrong request line: {request}"
    );
    assert!(
        request.contains(r#""model":"test-model""#),
        "request: {request}"
    );
    assert!(
        request.contains(r#""role":"system""#)
            && request.contains(r#""content":"eres un ayudante""#),
        "request: {request}"
    );
    assert!(
        request.contains(r#""role":"user""#)
            && request.contains(r#""content":"¿qué dice la página 3?""#),
        "request: {request}"
    );
    assert!(request.contains(r#""stream":false"#), "request: {request}");
}

#[test]
fn chat_reports_non_200_status() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let _ = read_request(&mut sock);
        let body = "no such path";
        let response = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(response.as_bytes()).expect("write response");
    });

    let client = OllamaClient::with_base_url(format!("http://127.0.0.1:{port}"), "m");
    let err = client.chat("s", "p").expect_err("404 must be an error");
    match err {
        AiError::Http { status: 404, body } => assert_eq!(body, "no such path"),
        other => panic!("expected AiError::Http(404), got {other:?}"),
    }
    server.join().expect("server thread");
}

#[test]
fn chat_errors_when_connection_is_rejected() {
    // Bind an ephemeral port, then drop the listener: nothing listens on it
    // anymore, so connect() is refused.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let client = OllamaClient::with_base_url(format!("http://127.0.0.1:{port}"), "m");
    let err = client
        .chat("s", "p")
        .expect_err("refused connection must fail");

    match &err {
        AiError::NotReachable { url, .. } => {
            assert!(url.contains(&port.to_string()), "url: {url}")
        }
        other => panic!("expected AiError::NotReachable, got {other:?}"),
    }
    assert!(
        err.to_string().contains("ollama not reachable"),
        "message: {err}"
    );
}

#[test]
fn chat_rejects_https_base_url_without_network() {
    let client = OllamaClient::with_base_url("https://localhost:11434", "m");
    let err = client.chat("s", "p").expect_err("https is unsupported");
    assert!(matches!(err, AiError::InvalidArgument(_)));
    assert!(err.to_string().contains("http://"), "message: {err}");
}

#[test]
fn new_uses_the_default_localhost_endpoint() {
    // Deterministic: no network. The default URL is asserted via the getter
    // instead of dialing localhost:11434 (which would be flaky once the
    // author actually runs Ollama on this machine).
    let client = OllamaClient::new("llama3.2");
    assert_eq!(client.base_url(), "http://localhost:11434");
    let client = OllamaClient::with_base_url("http://192.168.1.7:11434", "m");
    assert_eq!(client.base_url(), "http://192.168.1.7:11434");
}
