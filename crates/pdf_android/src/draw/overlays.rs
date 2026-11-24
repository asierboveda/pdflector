// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Overlays flotantes del visor y la biblioteca: aviso breve (toast),
//! panel "Preguntar a la IA" y snapshot de la biblioteca para el fade al
//! abrir un libro.

use crate::reader::{AiPhase, Reader, lib_content_y0};
use crate::theme;
use pdf_core::Bitmap;

use super::{
    AiLine, AiLineKind, ButtonRect, CanvasRect, CanvasText, TextAlign, copy_region, draw_button,
    fill_buffer, format_ai_response, jni_text_bitmap,
};

/// Tipografía del panel de IA (px): cuerpo, encabezado y ecuación, con su
/// interlineado. Compartida por layout, render y el scroll por arrastre.
pub(crate) const AI_BODY_SIZE: f32 = 18.0;
pub(crate) const AI_HEADING_SIZE: f32 = 21.0;
pub(crate) const AI_EQUATION_SIZE: f32 = 19.0;
pub(crate) const AI_LINE_H: f32 = 28.0;

// ---------------------------------------------------------------------
// Aviso breve (toast)
// ---------------------------------------------------------------------
pub(crate) fn render_toast(reader: &Reader) -> Option<Bitmap> {
    let msg = reader.toast.as_ref()?.0.clone();
    let (bw, bh) = ((reader.win_w / 6).max(140), (reader.win_h / 60).max(30));
    let p = reader.theme.palette();
    let (bg, border, text) = (p.badge_bg(), p.badge_border(), p.badge_text());
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    let r = 999.0f32;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, bw as f32, bh as f32, r, border,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        bw as f32 - 1.0,
        bh as f32 - 1.0,
        r,
        bg,
    ));
    let ts = theme::FONT_CAPTION;
    texts.push(CanvasText::new(
        bw as f32 / 2.0,
        bh as f32 * 0.5 + ts * 0.35,
        ts,
        text,
        TextAlign::Center,
        true,
        msg,
    ));
    jni_text_bitmap(bw, bh, theme::TRANSPARENT, &rects, &texts)
}

// ---------------------------------------------------------------------
// Panel de "Preguntar a la IA" (Parte 2): tarjeta flotante con respuesta
// ---------------------------------------------------------------------
//
// Geometría COMPARTIDA entre el
// render (Canvas+JNI) y el tap de `input` (`Reader::ai_panel.buttons`). El
// panel es una tarjeta centrada (horizontal y verticalmente) con cabecera
// (título + botones ✕/▲/▼) y cuerpo de texto envuelto en líneas. El texto
// se envuelve AQUÍ en Rust: se estima el ancho de cada carácter en ~0.52 ×
// tamaño de fuente (alfabeto latino; no hay medición real de glifos vía
// JNI) y cada línea es un `CanvasText`; las líneas fuera de la ventana de
// scroll se saltan, así que el recorte del cuerpo es gratis. Decisiones:
//
// - El alto del panel se AJUSTA al texto: si cabe entero (≤ 55 % de la
//   ventana) no hay scroll; si desborda, el cuerpo se limita y aparecen
//   los botones ▲/▼ (scroll por línea, `Reader::ai_scroll`).
// - La cabecera siempre muestra el botón ✕ (cerrar); un tap FUERA del panel
//   también lo cierra (`input::ai_panel_tap`).
// - El error (sin red / key inválida / error del proveedor) se muestra en
//   el MISMO panel, en rojo, con el mensaje de `AiError` (Display).

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
/// cuerpo cuyo alto se ajusta al texto: si cabe entero, panel corto SIN
/// scroll; si desborda, cuerpo limitado al ~78 % de la ventana, con scroll
/// por arrastre y botones ▲/▼. El scroll actual se lee de `Reader::ai_panel`
/// (0 al abrir) y se devuelve recortado al rango válido para que
/// `Reader::rebuild_ai_panel` lo guarde. None si la ventana es demasiado
/// pequeña para un panel.
pub(crate) fn ai_panel_layout(reader: &Reader) -> Option<AiLayout> {
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    if win_w < 200.0 || win_h < 200.0 {
        return None;
    }
    // Constantes del panel (las MISMAS en layout y render).
    let ts = AI_BODY_SIZE;
    let line_h = AI_LINE_H;
    let pad = 20.0f32;
    let header_h = 52.0f32;
    let btn = 34.0f32;
    let gap = 6.0f32;
    let margin = 16.0f32;
    let panel_w = (win_w - 2.0 * margin).clamp(200.0, 1080.0);
    let max_body_h = (win_h * 0.78).clamp(160.0, (win_h - 120.0).max(160.0));
    // Formatear y envolver (estimación de caracteres por línea, latino).
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
    // Misma geometría para render y tap.
    let mut buttons: Vec<(&'static str, ButtonRect)> = Vec::with_capacity(4);
    let btn_top = y + (header_h - btn) / 2.0;
    let btn_bottom = btn_top + btn;
    let mut bx = x + panel_w - pad - btn; // el más a la derecha: ✕
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
    // Recortar el scroll actual al rango válido (0..total−visible).
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

/// Renderiza el panel de IA (tarjeta oscura redondeada + cabecera con título
/// y botones + cuerpo con las líneas VISIBLES, saltando las que quedan fuera
/// de la ventana de scroll) a un bitmap RGBA8 del tamaño del panel con fondo
/// transparente. Título y color del cuerpo según la fase (`AiPhase`):
/// Confirm = aviso antes de enviar, Asking = "Preguntando a la IA…",
/// Answer = texto claro, Error = "IA — error" con texto rojizo (`STATUS_*`).
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
    // Tarjeta: rect redondeado (borde + relleno)
    let r = 16.0f32;
    rects.push(CanvasRect::rounded(
        0.0,
        0.0,
        w as f32,
        h as f32,
        r,
        p.popup_border(),
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        (r - 1.0).max(0.0),
        p.popup_bg(),
    ));
    // Divisor bajo la cabecera
    rects.push(CanvasRect::sharp(
        1.0,
        header_h,
        w as f32 - 1.0,
        header_h + 1.0,
        p.popup_border(),
    ));
    // Cabecera: título según la fase + botones Explicar/▲/▼/✕
    let (title, body_color) = match reader.ai_phase {
        AiPhase::Confirm => ("Explicar selección", p.base_content),
        AiPhase::Asking => ("Preguntando a la IA…", p.neutral_content),
        AiPhase::Answer => ("Preguntar a la IA", p.base_content),
        AiPhase::Error => ("IA — error", p.status_text()),
    };
    texts.push(CanvasText::new(
        pad,
        header_h * 0.5 + ts * 0.35,
        ts,
        p.base_content,
        TextAlign::Left,
        true,
        title,
    ));
    for (label, (l, t, rr, b)) in &layout.buttons {
        let (bg, border, fg, bold) = if *label == "Explicar" {
            (p.primary, p.primary, p.primary_content, true)
        } else {
            (p.base_200, p.base_300, p.base_content, false)
        };
        draw_button(
            &mut rects,
            &mut texts,
            *l - mx,
            *t - my,
            *rr - mx,
            *b - my,
            bg,
            border,
            fg,
            theme::FONT_CAPTION,
            bold,
            label,
        );
    }
    // Cuerpo: solo las líneas visibles [scroll, scroll+visible). Cada
    // línea dibuja sus segmentos (normal/negrita) en orden; las ecuaciones
    // van centradas y los encabezados, más grandes.
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
        let y = body_top + (i - layout.scroll) as f32 * line_h + size * 0.9;
        // Avance estimado por carácter (el espacio es más estrecho que la
        // media; contarlo igual dejaba huecos alrededor de las negritas).
        let advance = |text: &str| -> f32 {
            text.chars()
                .map(|c| if c == ' ' { 0.30 } else { 0.52 })
                .sum::<f32>()
                * size
        };
        let plain_w: f32 = line.segs.iter().map(|s| advance(&s.text)).sum();
        let mut x = if line.kind == AiLineKind::Equation {
            ((w as f32 - plain_w) / 2.0).max(pad)
        } else {
            pad
        };
        for seg in &line.segs {
            if seg.text.is_empty() {
                continue;
            }
            texts.push(CanvasText::new(
                x,
                y,
                size,
                body_color,
                TextAlign::Left,
                seg.bold,
                seg.text.clone(),
            ));
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
