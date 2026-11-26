// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Ayudantes puros (sin JNI) para lo que llega por Intent: percent-decoding,
//! detección de identificadores de arXiv y saneado de nombres de PDF.
//! Compilan también en host, donde se ejecutan sus tests.

/// Decodifica caracteres percent-encoded (%XX) a UTF-8.
pub(crate) fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let mut chars = s.as_bytes().iter().copied().peekable();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let (Some(d1), Some(d2)) = (hex_digit(h1), hex_digit(h2)) {
                    bytes.push((d1 << 4) | d2);
                    continue;
                }
                bytes.push(b'%');
                bytes.push(h1);
                bytes.push(h2);
                continue;
            }
            bytes.push(b'%');
            if let Some(h1) = h1 {
                bytes.push(h1);
            }
            continue;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Parsea un texto o URI recibido por intent para extraer un identificador canónico de arXiv.
///
/// Acepta:
/// - URLs arXiv: `arxiv.org/abs/<id>`, `arxiv.org/pdf/<id>` (con o sin `https://` / `http://` / `.pdf` / versión)
/// - URLs export.arxiv: `export.arxiv.org/abs/<id>`, `export.arxiv.org/pdf/<id>`
/// - URLs PaperTok: `papertok.app/#/public/paper/<base64url>` (donde la clave decodifica a `arxiv:<id>`)
/// - Esquema custom: `pdflector://arxiv/<id>`, `pdflector://<id>` (con o sin percent-encoding de barras)
/// - Identificador suelto: `2401.12345`, `2401.12345v2`, `0706.0001`, `hep-th/9901001`, `arXiv:2401.12345`
/// - Texto que contenga cualquiera de las formas anteriores rodeado de espacios/mensaje.
///
/// Devuelve `None` si la entrada no representa un paper de arXiv válido (ej. DOI, URL genérica o texto vacío).
fn trim_trailing_punct(s: &str) -> &str {
    s.trim_end_matches(|c: char| {
        matches!(
            c,
            '.' | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | ')'
                | ']'
                | '}'
                | '>'
                | '"'
                | '\''
                | '»'
                | '”'
                | '’'
        )
    })
}

fn trim_leading_punct(s: &str) -> &str {
    s.trim_start_matches(|c: char| {
        matches!(c, '(' | '[' | '{' | '<' | '"' | '\'' | '«' | '“' | '‘')
    })
}

pub(crate) fn parse_arxiv_target(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Si el texto contiene espacios en blanco (ej. texto compartido de navegador o PaperTok
    // con título + URL), intentar primero si alguna palabra parsea como paper válido.
    if trimmed.contains(|c: char| c.is_whitespace()) {
        for word in trimmed.split_whitespace() {
            let cleaned = trim_leading_punct(trim_trailing_punct(word));
            if let Some(id) = parse_single_arxiv_target(cleaned) {
                return Some(id);
            }
        }
    }

    parse_single_arxiv_target(trimmed)
}

fn parse_single_arxiv_target(input: &str) -> Option<String> {
    let input = trim_leading_punct(trim_trailing_punct(input.trim()));
    if input.is_empty() {
        return None;
    }

    // 1. PaperTok: https://papertok.app/#/public/paper/<base64url> o /public/paper/<base64url>
    if let Some(idx) = input.find("public/paper/") {
        let after = &input[idx + "public/paper/".len()..];
        // Extraer la clave hasta el primer delimitador de query/hash/path/espacio
        let key = after.split(['?', '#', '&', '/', ' ']).next().unwrap_or("");
        if !key.is_empty() {
            use base64::Engine;
            let key_trimmed = key.trim_end_matches('=');
            if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(key_trimmed)
                && let Ok(decoded_str) = String::from_utf8(bytes)
            {
                let lower = decoded_str.to_lowercase();
                if let Some(rest) = lower.strip_prefix("arxiv:")
                    && let Ok((canonical, _)) = pdf_core::parse_arxiv_id(rest)
                {
                    return Some(canonical);
                }
            }
        }
        // Si era una URL de PaperTok pero no contenía 'arxiv:' (ej. 'doi:...'), es no-arXiv.
        return None;
    }

    // 2. pdflector:// scheme
    let s = if let Some(idx) = input.find("pdflector://") {
        let after = &input[idx + "pdflector://".len()..];
        let after = after.strip_prefix("arxiv/").unwrap_or(after);
        after.strip_prefix("arxiv:").unwrap_or(after)
    } else {
        input
    };

    // 3. Percent-decode (ej. hep-th%2F9901001 -> hep-th/9901001)
    let decoded = percent_decode(s);
    let mut s = decoded.trim();

    // Eliminar fragmentos '#' o queries '?' residuales
    if let Some(q) = s.find('?') {
        s = &s[..q];
    }
    if let Some(h) = s.find('#') {
        s = &s[..h];
    }

    // 4. Prefijos de export.arxiv.org y arxiv.org sin https/http
    for prefix in [
        "https://export.arxiv.org/abs/",
        "https://export.arxiv.org/pdf/",
        "http://export.arxiv.org/abs/",
        "http://export.arxiv.org/pdf/",
        "export.arxiv.org/abs/",
        "export.arxiv.org/pdf/",
        "https://arxiv.org/abs/",
        "https://arxiv.org/pdf/",
        "http://arxiv.org/abs/",
        "http://arxiv.org/pdf/",
        "arxiv.org/abs/",
        "arxiv.org/pdf/",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }

    // Quitar extensión .pdf si está presente
    if let Some(rest) = s.strip_suffix(".pdf") {
        s = rest;
    }
    let s = trim_trailing_punct(s.trim());

    // 5. Validación canónica y rechazo estricto de esquemas ajenos (ej. DOIs, URLs no-arXiv)
    let (canonical, _) = match pdf_core::parse_arxiv_id(s) {
        Ok(pair) => pair,
        Err(_) => return None,
    };

    if canonical.starts_with("http://") || canonical.starts_with("https://") {
        return None;
    }

    if let Some((cat, num)) = canonical.split_once('/') {
        let cat_ok = !cat.is_empty()
            && cat
                .chars()
                .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.');
        let num_ok = num.len() == 7 && num.chars().all(|c| c.is_ascii_digit());
        if !cat_ok || !num_ok {
            return None;
        }
    } else if let Some((yymm, num)) = canonical.split_once('.') {
        let yymm_ok = yymm.len() == 4 && yymm.chars().all(|c| c.is_ascii_digit());
        let num_ok = (num.len() == 4 || num.len() == 5) && num.chars().all(|c| c.is_ascii_digit());
        if !yymm_ok || !num_ok {
            return None;
        }
    } else {
        return None;
    }

    Some(canonical)
}

/// Sanea un nombre de fichero recibido del sistema (DISPLAY_NAME o segmento de
/// URI) para usarlo como nombre local: solo caracteres seguros, extensión .pdf
/// garantizada y longitud acotada.
pub(crate) fn sanitize_pdf_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '[' | ']') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        return "external.pdf".to_string();
    }
    let mut s = s;
    if !s.to_ascii_lowercase().ends_with(".pdf") {
        s.push_str(".pdf");
    }
    if s.chars().count() > 80 {
        s = format!("{}.pdf", s.chars().take(76).collect::<String>());
    }
    s
}
