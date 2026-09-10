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
