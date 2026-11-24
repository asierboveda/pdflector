// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Formato de la respuesta de la IA para el panel: convierte el Markdown y
//! el LaTeX que devuelve Gemini en líneas con segmentos (normal/negrita)
//! listas para envolver y dibujar con Canvas.

/// Segmento de texto con un único estilo (el Canvas solo distingue
/// normal/negrita).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiSeg {
    pub(crate) text: String,
    pub(crate) bold: bool,
}

/// Tipo de línea ya formateada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AiLineKind {
    Body,
    Heading,
    Equation,
}

/// Línea lista para dibujar: segmentos en orden y tipo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiLine {
    pub(crate) segs: Vec<AiSeg>,
    pub(crate) kind: AiLineKind,
}

impl AiLine {
    pub(crate) fn plain_text(&self) -> String {
        self.segs.iter().map(|s| s.text.as_str()).collect()
    }

    fn is_blank(&self) -> bool {
        self.plain_text().trim().is_empty()
    }
}

/// Convierte la respuesta cruda (Markdown + LaTeX) en líneas envueltas a
/// `max_chars` caracteres, con negritas, viñetas y ecuaciones legibles.
pub(crate) fn format_ai_response(raw: &str, max_chars: usize) -> Vec<AiLine> {
    let max_chars = max_chars.max(8);
    let mut out: Vec<AiLine> = Vec::new();
    let mut in_display_math = false;
    for raw_line in raw.lines() {
        let line = raw_line.trim();
        match line {
            "$$" => {
                in_display_math = !in_display_math;
                continue;
            }
            "\\[" => {
                in_display_math = true;
                continue;
            }
            "\\]" => {
                in_display_math = false;
                continue;
            }
            _ => {}
        }
        if line.is_empty() {
            if !out.last().is_some_and(|l: &AiLine| l.is_blank()) {
                out.push(AiLine {
                    segs: Vec::new(),
                    kind: AiLineKind::Body,
                });
            }
            continue;
        }
        // Ecuación en bloque en una sola línea: $$...$$ o \[...\]
        let display = strip_delim(line, "$$", "$$").or_else(|| strip_delim(line, "\\[", "\\]"));
        if let Some(eq) = display {
            out.push(AiLine {
                segs: vec![AiSeg {
                    text: latex_to_text(eq.trim()),
                    bold: false,
                }],
                kind: AiLineKind::Equation,
            });
            continue;
        }
        if in_display_math {
            out.push(AiLine {
                segs: vec![AiSeg {
                    text: latex_to_text(line),
                    bold: false,
                }],
                kind: AiLineKind::Equation,
            });
            continue;
        }
        // Encabezado Markdown y listas (viñeta normalizada).
        let mut kind = AiLineKind::Body;
        let work: String = if line.starts_with('#') {
            kind = AiLineKind::Heading;
            line.trim_start_matches('#').trim().to_string()
        } else if let Some(item) = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .or_else(|| line.strip_prefix("+ "))
        {
            format!("• {}", item.trim())
        } else {
            line.to_string()
        };
        let converted = latex_to_text(&work);
        let segs = parse_markdown_segments(&converted, kind == AiLineKind::Heading);
        for wrapped in wrap_segments(&segs, max_chars) {
            out.push(AiLine {
                segs: wrapped,
                kind,
            });
        }
    }
    while out.last().is_some_and(|l: &AiLine| l.is_blank()) {
        out.pop();
    }
    while out.first().is_some_and(|l: &AiLine| l.is_blank()) {
        out.remove(0);
    }
    if out.is_empty() {
        out.push(AiLine {
            segs: vec![AiSeg {
                text: "…".to_string(),
                bold: false,
            }],
            kind: AiLineKind::Body,
        });
    }
    out
}

fn strip_delim<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    s.strip_prefix(open)?.strip_suffix(close)
}

/// Parsea `**negrita**` y `*cursiva*` (la cursiva se dibuja normal: el
/// Canvas no tiene itálica) en segmentos.
fn parse_markdown_segments(text: &str, force_bold: bool) -> Vec<AiSeg> {
    let mut segs: Vec<AiSeg> = Vec::new();
    let mut rest = text;
    let push = |segs: &mut Vec<AiSeg>, t: &str, bold: bool| {
        if t.is_empty() {
            return;
        }
        if let Some(last) = segs.last_mut()
            && last.bold == bold
        {
            last.text.push_str(t);
            return;
        }
        segs.push(AiSeg {
            text: t.to_string(),
            bold,
        });
    };
    loop {
        if let Some(idx) = rest.find("**") {
            let (before, after) = rest.split_at(idx);
            push(&mut segs, before, force_bold);
            let after = &after[2..];
            match after.find("**") {
                Some(end) => {
                    push(&mut segs, &after[..end], true);
                    rest = &after[end + 2..];
                }
                None => {
                    push(&mut segs, after, force_bold);
                    rest = "";
                }
            }
        } else {
            // Cursiva de un asterisco: se quitan las marcas.
            let cleaned: String = rest.chars().filter(|c| *c != '*').collect();
            push(&mut segs, &cleaned, force_bold);
            break;
        }
        if rest.is_empty() {
            break;
        }
    }
    if segs.is_empty() {
        segs.push(AiSeg {
            text: String::new(),
            bold: force_bold,
        });
    }
    segs
}

/// Envuelve segmentos en líneas de ≤ `max_chars` por palabras, conservando
/// el estilo de cada palabra.
fn wrap_segments(segs: &[AiSeg], max_chars: usize) -> Vec<Vec<AiSeg>> {
    // Palabras con su estilo, en orden.
    let mut flat: Vec<(String, bool)> = Vec::new();
    for seg in segs {
        for w in seg.text.split(' ') {
            if !w.is_empty() {
                flat.push((w.to_string(), seg.bold));
            }
        }
    }
    let mut lines: Vec<Vec<AiSeg>> = Vec::new();
    let mut cur: Vec<AiSeg> = Vec::new();
    let mut cur_len = 0usize;
    let push_word = |cur: &mut Vec<AiSeg>, cur_len: &mut usize, w: &str, bold: bool| {
        let extra = if *cur_len == 0 {
            w.chars().count()
        } else {
            w.chars().count() + 1
        };
        if let Some(last) = cur.last_mut()
            && last.bold == bold
        {
            if *cur_len > 0 {
                last.text.push(' ');
            }
            last.text.push_str(w);
        } else {
            if *cur_len > 0
                && let Some(last) = cur.last_mut()
            {
                last.text.push(' ');
            }
            cur.push(AiSeg {
                text: w.to_string(),
                bold,
            });
        }
        *cur_len += extra;
    };
    for (w, bold) in flat {
        let wlen = w.chars().count();
        if wlen > max_chars {
            // Palabra más larga que la línea: cortarla.
            if cur_len > 0 {
                lines.push(std::mem::take(&mut cur));
                cur_len = 0;
            }
            let mut rest = w.as_str();
            while rest.chars().count() > max_chars {
                let cut = rest
                    .char_indices()
                    .nth(max_chars)
                    .map(|(i, _)| i)
                    .unwrap_or(rest.len());
                let (a, b) = rest.split_at(cut);
                lines.push(vec![AiSeg {
                    text: a.to_string(),
                    bold,
                }]);
                rest = b;
            }
            if !rest.is_empty() {
                push_word(&mut cur, &mut cur_len, rest, bold);
            }
            continue;
        }
        if cur_len > 0 && cur_len + 1 + wlen > max_chars {
            lines.push(std::mem::take(&mut cur));
            cur_len = 0;
        }
        push_word(&mut cur, &mut cur_len, &w, bold);
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Convierte LaTeX habitual a texto Unicode legible: fracciones, scripts,
/// símbolos y delimitadores. Lo desconocido pierde la barra y las llaves.
pub(crate) fn latex_to_text(input: &str) -> String {
    let mut s = input.to_string();
    for delim in ["$$", "\\(", "\\)", "\\[", "\\]"] {
        s = s.replace(delim, "");
    }
    s = s
        .replace('$', "")
        .replace("\\left", "")
        .replace("\\right", "")
        .replace("\\displaystyle", "")
        .replace("\\,", " ")
        .replace("\\;", " ")
        .replace("\\ ", " ");
    let s = replace_frac(&s);
    let s = replace_sqrt(&s);
    let s = replace_scripts(&s);
    let s = replace_commands(&s);
    s.chars()
        .filter(|c| *c != '{' && *c != '}')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lee un grupo `{...}` a partir de `start` (índice tras '{'). Devuelve
/// (contenido, índice tras '}').
fn read_group(s: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[start..i], i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn replace_frac(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(pos) = ["\\frac", "\\dfrac", "\\tfrac"]
            .iter()
            .filter_map(|cmd| rest.find(cmd).map(|i| (i, *cmd)))
            .min_by_key(|(i, _)| *i)
        else {
            out.push_str(rest);
            break;
        };
        let (idx, cmd) = pos;
        out.push_str(&rest[..idx]);
        let after = &rest[idx + cmd.len()..];
        let Some(open) = after.find('{') else {
            out.push_str(cmd);
            rest = after;
            continue;
        };
        if after[..open].trim().is_empty()
            && let Some((num, next)) = read_group(after, open + 1)
        {
            let after2 = &after[next..];
            if let Some(open2) = after2.find('{')
                && after2[..open2].trim().is_empty()
                && let Some((den, next2)) = read_group(after2, open2 + 1)
            {
                let num_t = latex_to_text(num);
                let den_t = latex_to_text(den);
                let wrap = |t: &str| {
                    if t.chars().count() > 1 && t.contains(['+', '-', ' ']) {
                        format!("({t})")
                    } else {
                        t.to_string()
                    }
                };
                out.push_str(&format!("{}/{}", wrap(&num_t), wrap(&den_t)));
                rest = &after2[next2..];
                continue;
            }
        }
        out.push_str(cmd);
        rest = after;
    }
    out
}

fn replace_sqrt(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("\\sqrt") {
        out.push_str(&rest[..idx]);
        let after = &rest[idx + 5..];
        if let Some(open) = after.find('{')
            && after[..open].trim().is_empty()
            && let Some((inner, next)) = read_group(after, open + 1)
        {
            out.push_str(&format!("√({})", latex_to_text(inner)));
            rest = &after[next..];
        } else {
            out.push('√');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

const SUPERSCRIPT: &[(char, char)] = &[
    ('0', '⁰'),
    ('1', '¹'),
    ('2', '²'),
    ('3', '³'),
    ('4', '⁴'),
    ('5', '⁵'),
    ('6', '⁶'),
    ('7', '⁷'),
    ('8', '⁸'),
    ('9', '⁹'),
    ('+', '⁺'),
    ('-', '⁻'),
    ('=', '⁼'),
    ('(', '⁽'),
    (')', '⁾'),
    ('n', 'ⁿ'),
    ('i', 'ⁱ'),
    ('x', 'ˣ'),
];
const SUBSCRIPT: &[(char, char)] = &[
    ('0', '₀'),
    ('1', '₁'),
    ('2', '₂'),
    ('3', '₃'),
    ('4', '₄'),
    ('5', '₅'),
    ('6', '₆'),
    ('7', '₇'),
    ('8', '₈'),
    ('9', '₉'),
    ('+', '₊'),
    ('-', '₋'),
    ('=', '₌'),
    ('(', '₍'),
    (')', '₎'),
    ('i', 'ᵢ'),
    ('j', 'ⱼ'),
    ('n', 'ₙ'),
    ('x', 'ₓ'),
    ('k', 'ₖ'),
];

fn map_script(text: &str, table: &[(char, char)], marker: char) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match table.iter().find(|(k, _)| *k == c) {
            Some((_, v)) => out.push(*v),
            None => {
                out.push(marker);
                out.push(c);
            }
        }
    }
    out
}

fn replace_scripts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if (c == '^' || c == '_') && i + 1 < bytes.len() {
            let table = if c == '^' { SUPERSCRIPT } else { SUBSCRIPT };
            if bytes[i + 1] == b'{' {
                if let Some((inner, next)) = read_group(s, i + 2) {
                    out.push_str(&map_script(inner, table, c));
                    i = next;
                    continue;
                }
            } else if (bytes[i + 1] as char).is_ascii_alphanumeric() {
                let ch = bytes[i + 1] as char;
                out.push_str(&map_script(&ch.to_string(), table, c));
                i += 2;
                continue;
            }
        }
        // Copiar el carácter UTF-8 completo.
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

fn replace_commands(s: &str) -> String {
    const CMDS: &[(&str, &str)] = &[
        ("\\alpha", "α"),
        ("\\beta", "β"),
        ("\\gamma", "γ"),
        ("\\delta", "δ"),
        ("\\epsilon", "ε"),
        ("\\varepsilon", "ε"),
        ("\\theta", "θ"),
        ("\\lambda", "λ"),
        ("\\mu", "μ"),
        ("\\pi", "π"),
        ("\\sigma", "σ"),
        ("\\phi", "φ"),
        ("\\omega", "ω"),
        ("\\Delta", "Δ"),
        ("\\Sigma", "Σ"),
        ("\\Omega", "Ω"),
        ("\\infty", "∞"),
        ("\\sum", "∑"),
        ("\\prod", "∏"),
        ("\\int", "∫"),
        ("\\partial", "∂"),
        ("\\nabla", "∇"),
        ("\\cdot", "·"),
        ("\\times", "×"),
        ("\\div", "÷"),
        ("\\pm", "±"),
        ("\\leq", "≤"),
        ("\\geq", "≥"),
        ("\\neq", "≠"),
        ("\\approx", "≈"),
        ("\\rightarrow", "→"),
        ("\\leftarrow", "←"),
        ("\\Rightarrow", "⇒"),
        ("\\Leftarrow", "⇐"),
        ("\\in", "∈"),
        ("\\notin", "∉"),
        ("\\subset", "⊂"),
        ("\\cup", "∪"),
        ("\\cap", "∩"),
        ("\\forall", "∀"),
        ("\\exists", "∃"),
        ("\\ldots", "…"),
        ("\\dots", "…"),
        ("\\log", "log"),
        ("\\ln", "ln"),
        ("\\exp", "exp"),
        ("\\sin", "sin"),
        ("\\cos", "cos"),
        ("\\tan", "tan"),
        ("\\min", "min"),
        ("\\max", "max"),
        ("\\lim", "lim"),
    ];
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let rest = &s[i..];
            let name_len = rest[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .map(|c| c.len_utf8())
                .sum::<usize>();
            let cmd = &rest[..1 + name_len];
            // Comandos que desenvuelven su grupo: \text{...}, \mathrm{...}
            if matches!(
                cmd,
                "\\text" | "\\mathrm" | "\\mathbf" | "\\operatorname" | "\\textbf"
            ) {
                let after = &rest[cmd.len()..];
                if let Some(open) = after.find('{')
                    && after[..open].trim().is_empty()
                    && let Some((inner, next)) = read_group(after, open + 1)
                {
                    out.push_str(&latex_to_text(inner));
                    i += cmd.len() + next;
                    continue;
                }
            }
            match CMDS.iter().find(|(k, _)| *k == cmd) {
                Some((_, sym)) => out.push_str(sym),
                None => out.push_str(&cmd[1..]), // \cmd desconocido: sin barra
            }
            i += cmd.len();
            continue;
        }
        let ch_len = utf8_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{AiLineKind, format_ai_response, latex_to_text};

    fn plain(lines: &[super::AiLine]) -> Vec<String> {
        lines.iter().map(|l| l.plain_text()).collect()
    }

    #[test]
    fn strips_bold_markers_and_marks_segment() {
        let lines = format_ai_response("La **derivada** mide el cambio.", 80);
        let texts = plain(&lines);
        assert_eq!(texts, vec!["La derivada mide el cambio."]);
        let bold: Vec<&str> = lines[0]
            .segs
            .iter()
            .filter(|s| s.bold)
            .map(|s| s.text.trim())
            .collect();
        assert_eq!(bold, vec!["derivada"]);
    }

    #[test]
    fn unordered_list_becomes_bullets() {
        let lines = format_ai_response("- uno\n- dos", 80);
        assert_eq!(plain(&lines), vec!["• uno", "• dos"]);
    }

    #[test]
    fn heading_is_marked_and_clean() {
        let lines = format_ai_response("## Idea principal\nTexto normal.", 80);
        assert_eq!(lines[0].kind, AiLineKind::Heading);
        assert_eq!(lines[0].plain_text(), "Idea principal");
        assert_eq!(lines[1].kind, AiLineKind::Body);
    }

    #[test]
    fn latex_fraction_and_scripts_become_unicode() {
        assert_eq!(latex_to_text(r"\frac{a+b}{2}"), "(a+b)/2");
        assert_eq!(latex_to_text(r"\frac{1}{2}"), "1/2");
        assert_eq!(latex_to_text(r"x^{2} + y^{n}"), "x² + yⁿ");
        assert_eq!(latex_to_text(r"H_{2}O"), "H₂O");
    }

    #[test]
    fn latex_symbols_and_delimiters() {
        assert_eq!(latex_to_text(r"$\alpha \leq \sum x_i$"), "α ≤ ∑ xᵢ");
        assert_eq!(latex_to_text(r"\(E = mc^{2}\)"), "E = mc²");
    }

    #[test]
    fn display_equation_is_centered_line() {
        let lines = format_ai_response("Antes\n$$E = mc^{2}$$\nDespués", 80);
        assert_eq!(lines[1].kind, AiLineKind::Equation);
        assert_eq!(lines[1].plain_text(), "E = mc²");
    }

    #[test]
    fn wraps_long_lines_to_max_chars() {
        let lines = format_ai_response(
            "una explicación bastante larga que debe partirse en varias líneas cortas",
            20,
        );
        assert!(lines.len() > 2);
        for l in &lines {
            assert!(
                l.plain_text().chars().count() <= 20,
                "línea: {}",
                l.plain_text()
            );
        }
    }
}
