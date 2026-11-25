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

// ---------------------------------------------------------------------
// Aviso breve (toast) — píldora invertida Astryx de alto contraste
// ---------------------------------------------------------------------
pub(crate) fn render_toast(reader: &Reader) -> Option<Bitmap> {
    let msg = reader.toast.as_ref()?.0.clone();
    let (bw, bh) = ((reader.win_w / 6).max(140), (reader.win_h / 60).max(32));
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();
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
        &msg,
    );
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
