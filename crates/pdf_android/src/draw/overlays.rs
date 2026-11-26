// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Overlays flotantes del visor y la biblioteca: aviso breve (toast),
//! panel "Preguntar a la IA" y snapshot de la biblioteca para el fade al
//! abrir un libro.

use crate::reader::{AiPhase, Reader, lib_content_y0};
use crate::theme;
use pdf_core::Bitmap;

use super::{
    AiLine, AiLineKind, ButtonRect, ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign,
    TextFont, copy_region, draw_astryx_button, draw_surface_card, draw_token_pill, fill_buffer,
    format_ai_response, jni_text_bitmap,
};

/// Tipografía del panel de IA (px): cuerpo, encabezado y ecuación, con su
/// interlineado. Compartida por layout, render y el scroll por arrastre.
pub(crate) const AI_BODY_SIZE: f32 = 18.0;
pub(crate) const AI_HEADING_SIZE: f32 = 21.0;
pub(crate) const AI_EQUATION_SIZE: f32 = 19.0;
pub(crate) const AI_LINE_H: f32 = 28.0;

/// Máximo de líneas del aviso; el resto se trunca con "…".
pub(crate) const TOAST_MAX_LINES: usize = 3;
/// Margen mínimo (px) entre el aviso y el borde del viewport.
pub(crate) const TOAST_MARGIN: i32 = 16;
/// Relleno horizontal (px) del texto dentro del aviso.
pub(crate) const TOAST_PAD_X: f32 = 16.0;
/// Ancho medio estimado (px) de un carácter `FONT_CAPTION` en negrita.
pub(crate) const TOAST_CHAR_W: f32 = theme::FONT_CAPTION * 0.58;

/// Layout del aviso: tamaño del bitmap y líneas ya envueltas/truncadas.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToastLayout {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) lines: Vec<String>,
    /// Caracteres máximos por línea con el ancho elegido.
    pub(crate) max_chars: usize,
}

/// Interlineado (px) de las líneas del aviso.
const TOAST_LINE_H: f32 = theme::FONT_CAPTION * 1.4;
/// Relleno vertical (px) del texto dentro del aviso.
const TOAST_PAD_Y: f32 = 7.0;
/// Ancho máximo (px) del aviso aunque el viewport sea mayor (legibilidad).
const TOAST_MAX_W: i32 = 720;

/// Layout puro del aviso: el tamaño mínimo es el de la píldora clásica
/// (1/6 del ancho, 1/60 del alto); si el texto no cabe, crece hasta el ancho
/// del viewport (menos `TOAST_MARGIN` por lado, tope `TOAST_MAX_W`), envuelve
/// por palabras hasta `TOAST_MAX_LINES` y trunca con "…" lo que sobre.
pub(crate) fn toast_layout(msg: &str, win_w: i32, win_h: i32) -> ToastLayout {
    let min_w = (win_w / 6).max(140);
    let min_h = (win_h / 60).max(32);
    let max_w = (win_w - 2 * TOAST_MARGIN).clamp(1, TOAST_MAX_W);
    let max_h = (win_h - 2 * TOAST_MARGIN).max(1);
    let wrap_chars = chars_fitting(max_w);
    let lines = wrap_toast_lines(msg, wrap_chars, TOAST_MAX_LINES);
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    // +1 px: margen frente al redondeo para que `longest` quepa siempre.
    let text_w = (longest as f32 * TOAST_CHAR_W + 2.0 * TOAST_PAD_X).ceil() as i32 + 1;
    let w = text_w.max(min_w).min(max_w);
    let text_h = (lines.len() as f32 * TOAST_LINE_H + 2.0 * TOAST_PAD_Y).ceil() as i32;
    let h = text_h.max(min_h).min(max_h);
    ToastLayout {
        w,
        h,
        lines,
        max_chars: chars_fitting(w),
    }
}

/// Caracteres estimados que caben en un aviso de `w` px de ancho.
fn chars_fitting(w: i32) -> usize {
    (((w as f32 - 2.0 * TOAST_PAD_X) / TOAST_CHAR_W).floor() as usize).max(1)
}

/// Envuelve `msg` por palabras en líneas de `max_chars` como mucho (las
/// palabras más largas se parten) y, si no cabe en `max_lines`, trunca la
/// última con "…".
fn wrap_toast_lines(msg: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let max_lines = max_lines.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut cur: Vec<char> = Vec::new();
    let mut truncated = false;
    'words: for word in msg.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        loop {
            let sep = usize::from(!cur.is_empty());
            if cur.len() + sep + word.len() <= max_chars {
                if sep == 1 {
                    cur.push(' ');
                }
                cur.extend(word);
                continue 'words;
            }
            if cur.is_empty() {
                // Palabra más larga que una línea (rutas, URLs): corte duro.
                let rest = word.split_off(max_chars);
                lines.push(word.into_iter().collect());
                word = rest;
            } else {
                lines.push(std::mem::take(&mut cur).into_iter().collect());
            }
            if lines.len() == max_lines {
                truncated = true;
                break 'words;
            }
        }
    }
    if !truncated && (!cur.is_empty() || lines.is_empty()) {
        lines.push(cur.into_iter().collect());
    }
    if truncated && let Some(last) = lines.last_mut() {
        let mut chars: Vec<char> = last.chars().collect();
        chars.truncate(max_chars - 1);
        while chars.last().is_some_and(|c| c.is_whitespace()) {
            chars.pop();
        }
        chars.push('…');
        *last = chars.into_iter().collect();
    }
    lines
}

// ---------------------------------------------------------------------
// Aviso breve (toast) — píldora invertida Astryx de alto contraste
// ---------------------------------------------------------------------
pub(crate) fn render_toast(reader: &Reader) -> Option<Bitmap> {
    let msg = &reader.toast.as_ref()?.0;
    let layout = toast_layout(msg, reader.win_w, reader.win_h);
    let (bw, bh) = (layout.w, layout.h);
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    if layout.lines.len() <= 1 {
        draw_token_pill(
            &mut rects,
            &mut texts,
            (0.0, 0.0, bw as f32, bh as f32),
            p.bg_inverted,
            Some(p.border_emphasized),
            p.on_inverted,
            theme::FONT_CAPTION,
            TextFont::Sans,
            0.01,
            true,
            layout.lines.first().map_or("", String::as_str),
        );
    } else {
        // Varias líneas: tarjeta con radio de card (una píldora de radio h/2
        // recortaría las esquinas del texto).
        let (fw, fh) = (bw as f32, bh as f32);
        let rad = theme::RADIUS_CARD;
        rects.push(CanvasRect::rounded(
            0.0,
            0.0,
            fw,
            fh,
            rad,
            p.border_emphasized,
        ));
        rects.push(CanvasRect::rounded(
            1.0,
            1.0,
            fw - 1.0,
            fh - 1.0,
            (rad - 1.0).max(0.0),
            p.bg_inverted,
        ));
        let y0 = (fh - layout.lines.len() as f32 * TOAST_LINE_H) * 0.5;
        for (i, line) in layout.lines.iter().enumerate() {
            let cy = y0 + (i as f32 + 0.5) * TOAST_LINE_H + theme::FONT_CAPTION * 0.35;
            texts.push(
                CanvasText::new(
                    fw * 0.5,
                    cy,
                    theme::FONT_CAPTION,
                    p.on_inverted,
                    TextAlign::Center,
                    true,
                    line.as_str(),
                )
                .with_tracking(0.01),
            );
        }
    }
    jni_text_bitmap(bw, bh, theme::TRANSPARENT, &rects, &texts)
}

// ---------------------------------------------------------------------
// Panel de "Preguntar a la IA": diálogo flotante Astryx
// ---------------------------------------------------------------------

/// Layout del panel de IA calculado por `ai_panel_layout`: rect del panel en
/// px de ventana + botones (✕ siempre; ▲/▼ solo si desborda) + las líneas
/// envueltas del texto actual + conteos de scroll. Lo consumen
/// `render_ai_panel` (dibujo) y `Reader::rebuild_ai_panel` (estado para el
/// tap de `input`).
pub(crate) struct AiLayout {
    pub(crate) rect: (f32, f32, f32, f32),
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
    pub(crate) lines: Vec<AiLine>,
    pub(crate) scroll: usize,
    pub(crate) visible: usize,
    pub(crate) scrollable: bool,
}

/// Layout del panel de IA: tarjeta grande centrada (margen 16 px, ancho
/// hasta 1080 px), cabecera de 52 px (título + botones a la derecha) y
/// cuerpo cuyo alto se ajusta al texto.
pub(crate) fn ai_panel_layout(reader: &Reader) -> Option<AiLayout> {
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    if win_w < 200.0 || win_h < 200.0 {
        return None;
    }
    let ts = AI_BODY_SIZE;
    let line_h = AI_LINE_H;
    let pad = 20.0f32;
    let header_h = 52.0f32;
    let btn = 34.0f32;
    let gap = 6.0f32;
    let margin = 16.0f32;
    let panel_w = (win_w - 2.0 * margin).clamp(200.0, 1080.0);
    let max_body_h = (win_h * 0.78).clamp(160.0, (win_h - 120.0).max(160.0));
    let content_w = panel_w - 2.0 * pad;
    let max_chars = ((content_w / (ts * 0.52)).floor() as usize).max(8);
    let lines = format_ai_response(&reader.ai_text, max_chars);
    let total = lines.len();
    let visible = ((max_body_h / line_h).floor() as usize).max(1);
    let scrollable = total > visible;
    let body_h = if scrollable {
        max_body_h
    } else {
        (total as f32 * line_h).max(line_h)
    };
    let panel_h = header_h + body_h + pad;
    let x = (win_w - panel_w) / 2.0;
    let y = ((win_h - panel_h) / 2.0).max(8.0);
    // Botones de la cabecera, alineados a la derecha: [Explicar][▲][▼][×].
    let mut buttons: Vec<(&'static str, ButtonRect)> = Vec::with_capacity(4);
    let btn_top = y + (header_h - btn) / 2.0;
    let btn_bottom = btn_top + btn;
    let mut bx = x + panel_w - pad - btn;
    buttons.push(("×", (bx, btn_top, bx + btn, btn_bottom)));
    if scrollable {
        bx -= btn + gap;
        buttons.push(("▼", (bx, btn_top, bx + btn, btn_bottom)));
        bx -= btn + gap;
        buttons.push(("▲", (bx, btn_top, bx + btn, btn_bottom)));
    }
    if reader.ai_phase == AiPhase::Confirm {
        let confirm_w = 104.0f32;
        bx -= confirm_w + gap;
        buttons.push(("Explicar", (bx, btn_top, bx + confirm_w, btn_bottom)));
    }
    let max_scroll = total.saturating_sub(visible);
    let scroll = reader
        .ai_panel
        .as_ref()
        .map(|p| p.scroll)
        .unwrap_or(0)
        .min(max_scroll);
    Some(AiLayout {
        rect: (x, y, x + panel_w, y + panel_h),
        buttons,
        lines,
        scroll,
        visible,
        scrollable,
    })
}

/// Renderiza el diálogo flotante de IA en lenguaje Astryx (`bg_popover` +
/// banda de cabecera `bg_surface` + badge semántico por fase + bloques de
/// ecuación monoespaciados sobre `bg_muted`).
pub(crate) fn render_ai_panel(reader: &Reader) -> Option<Bitmap> {
    let layout = ai_panel_layout(reader)?;
    let (mx, my, mrx, mry) = layout.rect;
    let w = (mrx - mx) as i32;
    let h = (mry - my) as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let p = reader.theme.palette();
    let ts = AI_BODY_SIZE;
    let line_h = AI_LINE_H;
    let pad = 20.0f32;
    let header_h = 52.0f32;
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Superficie Dialog Astryx (`bg_popover` + `border_emphasized`)
    let card_border = if reader.ai_phase == AiPhase::Error {
        p.error_border
    } else {
        p.border_emphasized
    };
    draw_surface_card(
        &mut rects,
        (0.0, 0.0, w as f32, h as f32),
        theme::RADIUS_CARD,
        p.bg_popover,
        card_border,
        Elevation::None,
        p.is_dark,
    );

    // Banda superior de cabecera ligeramente diferenciada (`bg_surface`) + hairline
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        w as f32 - 1.0,
        header_h,
        theme::RADIUS_CARD - 1.0,
        p.bg_surface,
    ));
    rects.push(CanvasRect::sharp(
        1.0,
        header_h - theme::RADIUS_CARD,
        w as f32 - 1.0,
        header_h,
        p.bg_surface,
    ));
    rects.push(CanvasRect::sharp(
        1.0,
        header_h,
        w as f32 - 1.0,
        header_h + 1.0,
        p.border,
    ));

    // Badge semántico de fase + título en la cabecera
    let (tag, tag_bg, tag_fg, title, body_color) = match reader.ai_phase {
        AiPhase::Confirm => (
            "CONFIRMAR",
            p.accent_muted,
            p.on_accent_muted,
            "Explicar selección",
            p.text_primary,
        ),
        AiPhase::Asking => (
            "PROCESANDO",
            p.warning_bg,
            p.warning_text,
            "Consultando IA…",
            p.text_secondary,
        ),
        AiPhase::Answer => ("IA", p.teal_bg, p.teal_text, "Explicación", p.text_primary),
        AiPhase::Error => (
            "ERROR",
            p.error_bg,
            p.error_text,
            "No se pudo responder",
            p.error_text,
        ),
    };
    let tag_h = 22.0f32;
    let tag_w = (tag.chars().count() as f32 * 8.0 + 18.0).max(38.0);
    let tag_t = (header_h - tag_h) * 0.5;
    draw_token_pill(
        &mut rects,
        &mut texts,
        (pad, tag_t, pad + tag_w, tag_t + tag_h),
        tag_bg,
        None,
        tag_fg,
        theme::FONT_MICRO,
        TextFont::Sans,
        theme::TRACKING_CAPS,
        true,
        tag,
    );
    texts.push(
        CanvasText::new(
            pad + tag_w + 10.0,
            header_h * 0.5 + theme::FONT_TITLE * 0.34,
            theme::FONT_TITLE,
            p.text_primary,
            TextAlign::Left,
            true,
            title,
        )
        .with_tracking(theme::TRACKING_TIGHT),
    );

    for (label, (l, t, rr, b)) in &layout.buttons {
        let local_rect = (*l - mx, *t - my, *rr - mx, *b - my);
        let variant = if *label == "Explicar" {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        };
        draw_astryx_button(
            &mut rects,
            &mut texts,
            local_rect,
            variant,
            &p,
            theme::FONT_CAPTION,
            *label == "Explicar",
            label,
        );
    }

    // Cuerpo: solo las líneas visibles [scroll, scroll+visible).
    let body_top = header_h + pad;
    for (i, line) in layout
        .lines
        .iter()
        .enumerate()
        .skip(layout.scroll)
        .take(layout.visible)
    {
        let size = match line.kind {
            AiLineKind::Heading => AI_HEADING_SIZE,
            AiLineKind::Equation => AI_EQUATION_SIZE,
            AiLineKind::Body => ts,
        };
        let row_top = body_top + (i - layout.scroll) as f32 * line_h;
        let y = row_top + size * 0.9;
        let advance = |text: &str| -> f32 {
            text.chars()
                .map(|c| if c == ' ' { 0.30 } else { 0.52 })
                .sum::<f32>()
                * size
        };
        let plain_w: f32 = line.segs.iter().map(|s| advance(&s.text)).sum();

        if line.kind == AiLineKind::Equation && plain_w > 0.0 {
            rects.push(CanvasRect::rounded(
                pad,
                row_top + 1.0,
                w as f32 - pad,
                row_top + line_h - 1.0,
                theme::RADIUS_INNER,
                p.bg_muted,
            ));
            rects.push(CanvasRect::rounded(
                pad,
                row_top + 4.0,
                pad + 3.0,
                row_top + line_h - 4.0,
                1.5,
                p.accent,
            ));
        }

        let mut x = if line.kind == AiLineKind::Equation {
            ((w as f32 - plain_w) * 0.5).max(pad + 10.0)
        } else {
            pad
        };
        let font = match line.kind {
            AiLineKind::Equation => TextFont::Mono,
            AiLineKind::Heading | AiLineKind::Body => TextFont::Sans,
        };
        let tracking = if line.kind == AiLineKind::Heading {
            theme::TRACKING_TIGHT
        } else {
            0.0
        };
        let seg_color = if line.kind == AiLineKind::Heading && reader.ai_phase != AiPhase::Error {
            p.text_primary
        } else {
            body_color
        };

        for seg in &line.segs {
            if seg.text.is_empty() {
                continue;
            }
            texts.push(
                CanvasText::new(
                    x,
                    y,
                    size,
                    seg_color,
                    TextAlign::Left,
                    seg.bold || line.kind == AiLineKind::Heading,
                    seg.text.clone(),
                )
                .with_font(font)
                .with_tracking(tracking),
            );
            x += advance(&seg.text);
        }
    }
    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Snapshot de la pantalla de BIBLIOTECA (zona fija + banda actual) a un
/// bitmap RGBA8 del tamaño de la ventana: se captura justo antes de abrir un
/// libro y el visor lo funde sobre la página durante `LIB_FADE_MS`
/// (transición visual al abrir; ver `blit_lib_fade`). Puro memcpy por filas.
pub(crate) fn compose_library_snapshot(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    if w <= 0 || h <= 0 {
        return None;
    }
    let content_y0 = lib_content_y0(reader);
    let mut out = Bitmap {
        width: w as u32,
        height: h as u32,
        data: vec![0u8; w as usize * h as usize * 4],
    };
    // Fondo base y planos cacheados.
    let dst = out.data.as_mut_ptr();
    let p = reader.theme.palette();
    fill_buffer(dst, w as usize, h as usize, w as usize, 4, p.rgba_lib_bg());
    if let Some(header) = reader.library.lib_header.as_ref() {
        copy_region(dst, w as usize, h as usize, w as usize, 4, header, 0, 0);
    }
    if let Some((band, origin)) = reader.library.lib_band.as_ref() {
        let sy = content_y0 - (reader.library.lib_scroll as i32 - *origin);
        copy_region(dst, w as usize, h as usize, w as usize, 4, band, 0, sy);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORTS: [(i32, i32); 4] = [(1600, 2560), (2560, 1600), (1200, 1920), (480, 800)];

    /// El texto envuelto cabe en el ancho del aviso.
    fn assert_text_fits(l: &ToastLayout) {
        assert!(
            l.max_chars as f32 * TOAST_CHAR_W + 2.0 * TOAST_PAD_X <= l.w as f32 + 0.5,
            "max_chars={} no cabe en w={}",
            l.max_chars,
            l.w
        );
        for line in &l.lines {
            assert!(
                line.chars().count() <= l.max_chars,
                "línea de {} > {} caracteres: {line:?}",
                line.chars().count(),
                l.max_chars
            );
        }
    }

    #[test]
    fn short_toast_keeps_single_line_pill_size() {
        for (w, h) in VIEWPORTS {
            let l = toast_layout("Guardado", w, h);
            assert_eq!(l.lines, vec!["Guardado".to_string()]);
            assert_eq!((l.w, l.h), ((w / 6).max(140), (h / 60).max(32)));
        }
    }

    #[test]
    fn long_toast_stays_within_viewport_and_is_truncated() {
        let msg = "Error al sincronizar con Google Drive: la respuesta del servidor \
                   no es válida (HTTP 502 Bad Gateway). "
            .repeat(12);
        for (w, h) in VIEWPORTS {
            let l = toast_layout(&msg, w, h);
            assert!(
                l.w <= w - 2 * TOAST_MARGIN,
                "{w}x{h}: ancho {} desborda",
                l.w
            );
            assert!(
                l.h <= h - 2 * TOAST_MARGIN,
                "{w}x{h}: alto {} desborda",
                l.h
            );
            assert!(l.w > 0 && l.h > 0);
            assert!(
                !l.lines.is_empty() && l.lines.len() <= TOAST_MAX_LINES,
                "{w}x{h}: {} líneas",
                l.lines.len()
            );
            assert!(
                l.lines.last().is_some_and(|s| s.ends_with('…')),
                "{w}x{h}: la última línea no termina en elipsis: {:?}",
                l.lines.last()
            );
            assert_text_fits(&l);
        }
    }

    #[test]
    fn medium_toast_wraps_without_losing_words() {
        let msg = "No se pudo abrir el documento porque el archivo está dañado o protegido";
        let l = toast_layout(msg, 480, 800);
        assert!(
            l.lines.len() > 1,
            "debería envolver en 480 px: {:?}",
            l.lines
        );
        assert!(l.lines.len() <= TOAST_MAX_LINES);
        assert_eq!(l.lines.join(" "), msg);
        assert!(l.w <= 480 - 2 * TOAST_MARGIN);
        assert_text_fits(&l);
    }

    #[test]
    fn unbreakable_word_is_split_and_truncated() {
        let msg = "/storage/emulated/0/Download/".repeat(20);
        let l = toast_layout(&msg, 1200, 1920);
        assert!(l.lines.len() <= TOAST_MAX_LINES);
        assert!(l.lines.last().is_some_and(|s| s.ends_with('…')));
        assert!(l.w <= 1200 - 2 * TOAST_MARGIN);
        assert_text_fits(&l);
    }
}
