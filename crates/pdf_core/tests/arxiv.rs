// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use pdf_core::arxiv::{
    ARXIV_USER_AGENT, ArxivClient, ArxivEntry, ArxivError, ArxivId, ArxivQuery, DownloadOutcome,
    parse_arxiv_id, parse_atom_body, parse_atom_for_test,
};

const ARXIV_ATOM_FIXTURE_WITH_ENTITIES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xmlns:arxiv="http://arxiv.org/schemas/atom">
  <link href="http://arxiv.org/api/query?search_query=all:electron&amp;id_list=&amp;start=0&amp;max_results=1" rel="self" type="application/atom+xml"/>
  <title type="html">ArXiv Query: search_query=all:electron&amp;id_list=&amp;start=0&amp;max_results=1</title>
  <id>http://arxiv.org/api/CW5N4v2YqL1/1</id>
  <updated>2026-09-10T00:00:00Z</updated>
  <opensearch:totalResults xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">1</opensearch:totalResults>
  <opensearch:startIndex xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">0</opensearch:startIndex>
  <opensearch:itemsPerPage xmlns:opensearch="http://a9.com/-/spec/opensearch/1.1/">1</opensearch:itemsPerPage>
  <entry>
    <id>http://arxiv.org/abs/2401.12345v2</id>
    <updated>2024-01-20T12:00:00Z</updated>
    <published>2024-01-18T10:00:00Z</published>
    <title>Sample Paper on Electron Transitions (1D-&gt;2D)</title>
    <summary>We study electron dimensional transitions (1D-&gt;2D) in quantum wires.</summary>
    <author>
      <name>Alice Smith</name>
    </author>
    <author>
      <name>Bob Jones</name>
    </author>
    <arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.1000/182</arxiv:doi>
    <link href="http://arxiv.org/abs/2401.12345v2" rel="alternate" type="text/html"/>
    <link title="pdf" href="http://arxiv.org/pdf/2401.12345v2" rel="related" type="application/pdf"/>
    <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cs.AI" scheme="http://arxiv.org/schemas/atom"/>
    <category term="cs.AI" scheme="http://arxiv.org/schemas/atom"/>
    <category term="math.PR" scheme="http://arxiv.org/schemas/atom"/>
  </entry>
</feed>"#;

#[test]
fn parse_accepts_all_id_shapes() {
    assert_eq!(parse_arxiv_id("2401.12345").unwrap().0, "2401.12345");
    assert_eq!(
        parse_arxiv_id("https://arxiv.org/abs/hep-th/9901001v3")
            .unwrap()
            .0,
        "hep-th/9901001"
    );
    assert_eq!(parse_arxiv_id("arXiv:0706.0001v2").unwrap().0, "0706.0001");
    assert!(parse_arxiv_id("hep-th_9901001").is_err());
}

#[test]
fn parse_handles_urls_and_versions() {
    let (id, ver) = parse_arxiv_id("https://arxiv.org/pdf/2401.12345v2.pdf").unwrap();
    assert_eq!(id, "2401.12345");
    assert_eq!(ver, Some("2".to_string()));

    let (id, ver) = parse_arxiv_id("http://arxiv.org/abs/0706.0001").unwrap();
    assert_eq!(id, "0706.0001");
    assert_eq!(ver, None);
}

#[test]
fn query_builder_to_params() {
    let q = ArxivQuery::new()
        .with_search_query("cat:cs.AI")
        .with_id_list(vec!["2401.12345".into(), "0706.0001".into()])
        .with_start(10)
        .with_max_results(25)
        .with_sort_by("submittedDate")
        .with_sort_order("descending");

    let params = q.to_params();
    assert_eq!(
        params,
        vec![
            ("search_query".to_string(), "cat:cs.AI".to_string()),
            ("id_list".to_string(), "2401.12345,0706.0001".to_string()),
            ("start".to_string(), "10".to_string()),
            ("max_results".to_string(), "25".to_string()),
            ("sortBy".to_string(), "submittedDate".to_string()),
            ("sortOrder".to_string(), "descending".to_string()),
        ]
    );

    let empty_q = ArxivQuery::new();
    assert!(empty_q.to_params().is_empty());
}

#[test]
fn arxiv_id_variants_and_error_display() {
    let _m = ArxivId::Modern("2401.12345".into());
    let _m4 = ArxivId::Modern4("0706.0001".into());
    let _c = ArxivId::Classic("hep-th/9901001".into());

    let err = ArxivError::InvalidId("bad_id".into());
    assert!(err.to_string().contains("bad_id"));
}

#[test]
fn parses_atom_entry_with_entities() {
    let xml = ARXIV_ATOM_FIXTURE_WITH_ENTITIES;
    let entries = parse_atom_for_test(xml).unwrap();
    assert_eq!(entries.len(), 1);
    let entry: &ArxivEntry = &entries[0];
    assert_eq!(entry.id, "2401.12345");
    assert_eq!(entry.version, Some("2".to_string()));
    assert_eq!(entry.primary_category, "cs.AI");
    assert_eq!(entry.doi, Some("10.1000/182".to_string()));
    assert_eq!(entry.authors, vec!["Alice Smith", "Bob Jones"]);
    assert!(entry.summary.contains("(1D->2D)"));
    assert!(entry.pdf_url.contains("/pdf/"));
}

const ARXIV_ATOM_CLASSIC_AND_MULTI_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/hep-th/9901001v3</id>
    <updated>1999-01-20T12:00:00Z</updated>
    <published>1999-01-01T10:00:00Z</published>
    <title>
      Classic String Theory
      Paper
    </title>
    <summary>
      Abstract of classic paper
      with newlines.
    </summary>
    <author><name>Physicist One</name></author>
    <link href="http://arxiv.org/abs/hep-th/9901001v3" rel="alternate" type="text/html"/>
    <link title="pdf" href="http://arxiv.org/pdf/hep-th/9901001v3" rel="related" type="application/pdf"/>
    <category term="hep-th"/>
    <category term="math-ph"/>
  </entry>
  <entry>
    <id>http://arxiv.org/abs/2301.00001v1</id>
    <title>Second Paper</title>
    <summary>Summary of second paper with numeric entity &#62; and hex &#x3E;.</summary>
    <author><name>Author Two</name></author>
  </entry>
</feed>"#;

const ARXIV_ATOM_EMPTY_FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Empty Feed</title>
</feed>"#;

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

fn serve_once(
    listener: TcpListener,
    status: u16,
    content_type: &'static str,
    response_body: &'static str,
) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let request = read_request(&mut sock);
        let status_line = match status {
            200 => "HTTP/1.1 200 OK",
            400 => "HTTP/1.1 400 Bad Request",
            404 => "HTTP/1.1 404 Not Found",
            429 => "HTTP/1.1 429 Too Many Requests",
            500 => "HTTP/1.1 500 Internal Server Error",
            503 => "HTTP/1.1 503 Service Unavailable",
            other => panic!("unexpected test status {other}"),
        };
        let response = format!(
            "{status_line}\r\n\
             Content-Type: {content_type}\r\n\
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
fn parses_classic_and_multi_entry_feed() {
    let entries = parse_atom_body(ARXIV_ATOM_CLASSIC_AND_MULTI_FIXTURE).unwrap();
    assert_eq!(entries.len(), 2);

    // Classic ID entry
    let e0 = &entries[0];
    assert_eq!(e0.id, "hep-th/9901001");
    assert_eq!(e0.version, Some("3".to_string()));
    assert_eq!(e0.title, "Classic String Theory Paper");
    assert_eq!(e0.summary, "Abstract of classic paper with newlines.");
    assert_eq!(e0.authors, vec!["Physicist One"]);
    assert_eq!(e0.primary_category, "hep-th");
    assert_eq!(e0.categories, vec!["hep-th", "math-ph"]);
    assert_eq!(e0.pdf_url, "http://arxiv.org/pdf/hep-th/9901001v3");

    // Second entry with numeric entities and auto-derived pdf_url
    let e1 = &entries[1];
    assert_eq!(e1.id, "2301.00001");
    assert!(e1.summary.contains("numeric entity > and hex >."));
}

#[test]
fn parses_atom_empty_feed() {
    let entries = parse_atom_body(ARXIV_ATOM_EMPTY_FIXTURE).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn arxiv_client_search_sends_user_agent_and_params() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    let server = serve_once(
        listener,
        200,
        "application/atom+xml",
        ARXIV_ATOM_FIXTURE_WITH_ENTITIES,
    );

    let client = ArxivClient::for_tests(&format!("http://127.0.0.1:{port}"));
    let query = ArxivQuery::new()
        .with_search_query("electron")
        .with_max_results(1);

    let entries = client.search(&query).expect("search succeeds");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "2401.12345");

    let request = server.join().expect("server thread");
    assert!(
        request.starts_with("GET /api/query?"),
        "unexpected request line: {request}"
    );
    assert!(
        request.contains("search_query=electron"),
        "missing search_query in: {request}"
    );
    assert!(
        request.contains("max_results=1"),
        "missing max_results in: {request}"
    );
    assert!(
        request.contains(&format!("user-agent: {ARXIV_USER_AGENT}")),
        "missing User-Agent in: {request}"
    );
}

#[test]
fn arxiv_client_search_handles_rate_limit_and_http_errors() {
    // Test 429 Too Many Requests
    let listener429 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port429 = listener429.local_addr().expect("addr").port();
    let server429 = serve_once(listener429, 429, "text/plain", "Too Many Requests");
    let client429 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port429}"));
    let err429 = client429
        .search(&ArxivQuery::new())
        .expect_err("429 rate limit");
    assert_eq!(err429, ArxivError::RateLimited);
    server429.join().expect("join");

    // Test 503 Service Unavailable
    let listener503 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port503 = listener503.local_addr().expect("addr").port();
    let server503 = serve_once(listener503, 503, "text/plain", "Service Unavailable");
    let client503 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port503}"));
    let err503 = client503
        .search(&ArxivQuery::new())
        .expect_err("503 rate limit");
    assert_eq!(err503, ArxivError::RateLimited);
    server503.join().expect("join");

    // Test 500 Internal Server Error
    let listener500 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port500 = listener500.local_addr().expect("addr").port();
    let server500 = serve_once(listener500, 500, "text/plain", "Internal Error Details");
    let client500 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port500}"));
    let err500 = client500.search(&ArxivQuery::new()).expect_err("500 error");
    match err500 {
        ArxivError::Http { status: 500, body } => {
            assert!(body.contains("Internal Error Details"));
        }
        other => panic!("expected Http 500, got {other:?}"),
    }
    server500.join().expect("join");
}

#[test]
fn arxiv_client_fetch_by_ids_validates_and_fetches() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    let server = serve_once(
        listener,
        200,
        "application/atom+xml",
        ARXIV_ATOM_CLASSIC_AND_MULTI_FIXTURE,
    );

    let client = ArxivClient::for_tests(&format!("http://127.0.0.1:{port}"));

    // Empty list returns Ok(empty) without making HTTP call
    let empty = client.fetch_by_ids(&[]).unwrap();
    assert!(empty.is_empty());

    // Invalid ID fails immediately before network
    let bad = client.fetch_by_ids(&["hep-th_9901001"]).unwrap_err();
    assert!(matches!(bad, ArxivError::InvalidId(_)));

    // Valid query
    let entries = client
        .fetch_by_ids(&["hep-th/9901001v3", "https://arxiv.org/abs/2301.00001v1"])
        .expect("fetch succeeds");
    assert_eq!(entries.len(), 2);

    let request = server.join().expect("server thread");
    assert!(
        request.contains("id_list=hep-th%2F9901001v3%2C2301.00001v1")
            || request.contains("id_list=hep-th/9901001v3,2301.00001v1"),
        "unexpected query params: {request}"
    );
}

fn spawn_pdf_fixture(body: &'static [u8], filename: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let _req = read_request(&mut sock);
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/pdf\r\n\
             Content-Disposition: inline; filename=\"{filename}\"\r\n\
             ETag: \"12345-fake\"\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            body.len()
        );
        sock.write_all(response.as_bytes()).expect("write headers");
        sock.write_all(body).expect("write body");
        sock.flush().expect("flush response");
    });
    format!("http://{addr}")
}

#[test]
fn download_streams_pdf_and_reads_disposition() {
    let srv = spawn_pdf_fixture(b"%PDF-1.4 fake", "1706.03762v7.pdf");
    let c = ArxivClient::for_tests(&srv);
    let mut buf: Vec<u8> = Vec::new();
    let out: DownloadOutcome = c.download_to("1706.03762", &mut buf).unwrap();
    assert!(buf.starts_with(b"%PDF-"));
    assert_eq!(out.final_name, "1706.03762v7.pdf");
    assert_eq!(out.bytes, b"%PDF-1.4 fake".len() as u64);
    assert_eq!(out.etag.as_deref(), Some("\"12345-fake\""));
}

#[test]
fn download_handles_rate_limiting_and_http_errors() {
    // 429
    let l429 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port429 = l429.local_addr().expect("addr").port();
    let s429 = serve_once(l429, 429, "text/plain", "Too Many Requests");
    let c429 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port429}"));
    let mut buf = Vec::new();
    let err429 = c429.download_to("1706.03762", &mut buf).unwrap_err();
    assert_eq!(err429, ArxivError::RateLimited);
    s429.join().expect("join");

    // 503
    let l503 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port503 = l503.local_addr().expect("addr").port();
    let s503 = serve_once(l503, 503, "text/plain", "Service Unavailable");
    let c503 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port503}"));
    let mut buf = Vec::new();
    let err503 = c503.download_to("1706.03762", &mut buf).unwrap_err();
    assert_eq!(err503, ArxivError::RateLimited);
    s503.join().expect("join");

    // 404
    let l404 = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port404 = l404.local_addr().expect("addr").port();
    let s404 = serve_once(l404, 404, "text/plain", "Not Found");
    let c404 = ArxivClient::for_tests(&format!("http://127.0.0.1:{port404}"));
    let mut buf = Vec::new();
    let err404 = c404.download_to("1706.03762", &mut buf).unwrap_err();
    match err404 {
        ArxivError::Http { status: 404, .. } => {}
        other => panic!("expected Http 404, got {other:?}"),
    }
    s404.join().expect("join");

    // Bad content-type (e.g. text/html)
    let l_ct = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port_ct = l_ct.local_addr().expect("addr").port();
    let s_ct = serve_once(l_ct, 200, "text/html", "<html>captcha</html>");
    let c_ct = ArxivClient::for_tests(&format!("http://127.0.0.1:{port_ct}"));
    let mut buf = Vec::new();
    let err_ct = c_ct.download_to("1706.03762", &mut buf).unwrap_err();
    match err_ct {
        ArxivError::Http { status: 200, body } => {
            assert!(body.contains("bad content-type text/html"));
        }
        other => panic!("expected Http bad content-type, got {other:?}"),
    }
    s_ct.join().expect("join");
}

#[test]
fn download_fallback_filename_when_no_disposition() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let _req = read_request(&mut sock);
        let response = "HTTP/1.1 200 OK\r\n\
                        Content-Type: application/pdf\r\n\
                        Content-Length: 10\r\n\
                        Connection: close\r\n\
                        \r\n\
                        0123456789";
        sock.write_all(response.as_bytes()).expect("write");
        sock.flush().expect("flush");
    });

    let client = ArxivClient::for_tests(&format!("http://{addr}"));
    let mut buf = Vec::new();
    let out = client.download_to("2401.12345", &mut buf).unwrap();
    assert_eq!(out.final_name, "2401.12345.pdf");
    assert_eq!(out.bytes, 10);
    assert_eq!(out.etag, None);
    assert_eq!(buf, b"0123456789");
    handle.join().expect("join");
}

#[test]
fn download_classic_id_path_and_user_agent() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("client connects");
        let req = read_request(&mut sock);
        let response = "HTTP/1.1 200 OK\r\n\
                        Content-Type: application/pdf\r\n\
                        Content-Disposition: inline; filename=\"hep-th9901001v3.pdf\"\r\n\
                        Content-Length: 5\r\n\
                        Connection: close\r\n\
                        \r\n\
                        %PDF-";
        sock.write_all(response.as_bytes()).expect("write");
        sock.flush().expect("flush");
        req
    });

    let client = ArxivClient::for_tests(&format!("http://{addr}"));
    let mut buf = Vec::new();
    let out = client.download_to("hep-th/9901001v3", &mut buf).unwrap();
    assert_eq!(out.final_name, "hep-th9901001v3.pdf");
    assert_eq!(out.bytes, 5);

    let req = handle.join().expect("join");
    assert!(req.starts_with("GET /pdf/hep-th/9901001 "));
    assert!(req.contains(&format!("user-agent: {ARXIV_USER_AGENT}")));
}

#[test]
fn download_invalid_id_fails_before_network() {
    let client = ArxivClient::for_tests("http://127.0.0.1:9");
    let mut buf = Vec::new();
    let err = client.download_to("not_an_arxiv_id", &mut buf).unwrap_err();
    assert!(matches!(err, ArxivError::InvalidId(_)));
    assert!(buf.is_empty());
}
