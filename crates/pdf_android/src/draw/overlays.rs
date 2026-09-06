// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Overlays flotantes del visor y la biblioteca: menú de selección de
//! texto, aviso breve (toast), panel "Preguntar a la IA" y snapshot de
//! la biblioteca para el fade al abrir un libro.

use crate::reader::{AiPhase, Reader, lib_content_y0};
use crate::theme;
use pdf_core::Bitmap;

use super::{
    ButtonRect, CanvasRect, CanvasText, TextAlign, copy_region, draw_button, fill_buffer,
    jni_text_bitmap,
};

// ---------------------------------------------------------------------
// Selección de texto: menú flotante (Copiar/Subrayar/IA) y aviso breve
// ---------------------------------------------------------------------
//
// Geometría COMPARTIDA entre el render (Canvas+JNI) y el tap de `input`
// (`Reader::sel_menu.buttons`): el menú se coloca cerca del rect de
// selección (centrado en su x, encima si hay sitio y si no debajo), siempre
// dentro de la ventana.

/// Layout del menú de selección calculado por `sel_menu_layout`: rect del
/// menú en px de ventana (left, top, right, bottom) + botones (etiqueta +
/// rect en px de ventana). Lo consumen `render_sel_menu` (dibujo) y
/// `Reader::open_sel_menu` (estado para el tap de `input`).
pub(crate) struct SelMenuLayout {
    pub(crate) rect: (f32, f32, f32, f32),
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
}

/// Layout del menú de selección: rectángulo del menú en px de ventana
/// (`rect`) + botones (etiqueta + rect en px de ventana), compartido por
/// `render_sel_menu` y el tap de `input::sel_menu_tap`. `None` sin selección
/// fijada. Decisión documentada: el menú "flota" cerca del rect (anclado a
/// su centro x), arriba si cabe y si no debajo — nunca tapa el rect si se
/// puede evitar, y nunca se sale de la ventana.
pub(crate) fn sel_menu_layout(reader: &Reader) -> Option<SelMenuLayout> {
    let (sl, st, sr, sb) = reader.sel_screen_rect()?;
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    let pad = 10.0f32;
    let gap = 8.0f32;
    let bw = (win_w / 6.0).clamp(64.0, 120.0);
    let bh = (win_h / 28.0).clamp(36.0, 48.0);
    let menu_w = 3.0 * bw + 2.0 * gap + 2.0 * pad;
    let menu_h = bh + 2.0 * pad;
    let cx = (sl + sr) / 2.0;
    let x = (cx - menu_w / 2.0).clamp(8.0, (win_w - menu_w - 8.0).max(8.0));
    // Encima del rect si cabe (margen de 12 px); si no, debajo.
    let mut y = st - menu_h - 12.0;
    if y < 8.0 {
        y = sb + 12.0;
    }
    let y = y.clamp(8.0, (win_h - menu_h - 8.0).max(8.0));
    let mut buttons = Vec::with_capacity(3);
    for (i, label) in ["Copiar", "Subrayar", "IA"].into_iter().enumerate() {
        let bx = x + pad + i as f32 * (bw + gap);
        buttons.push((label, (bx, y + pad, bx + bw, y + pad + bh)));
    }
    Some(SelMenuLayout {
        rect: (x, y, x + menu_w, y + menu_h),
        buttons,
    })
}

/// Renderiza el menú de selección (tarjeta flotante + 3 botones: Copiar, Subrayar, IA).
pub(crate) fn render_sel_menu(reader: &Reader) -> Option<Bitmap> {
    let layout = sel_menu_layout(reader)?;
    let (mx, my, mrx, mry) = layout.rect;
    let w = (mrx - mx) as i32;
    let h = (mry - my) as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    // Tarjeta: rect redondeado (borde + relleno)
    let r = (h as f32) * 0.5;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, w as f32, h as f32, r, p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        r,
        p.base_100,
    ));
    for (label, (l, t, rr, b)) in &layout.buttons {
        let (fill, border, text_color) = match *label {
            "Subrayar" => (p.primary, p.primary, p.primary_content),
            "IA" => (p.base_200, p.base_300, p.neutral_content),
            _ => (p.base_200, p.base_300, p.base_content),
        };
        draw_button(
            &mut rects,
            &mut texts,
            *l - mx,
            *t - my,
            *rr - mx,
            *b - my,
            fill,
            border,
            text_color,
            theme::FONT_CAPTION,
            true,
            label,
        );
    }
    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Renderiza el aviso breve (toast) con el tema activo.
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
// Misma filosofía que el menú de selección: geometría COMPARTIDA entre el
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
    pub(crate) lines: Vec<String>,
    pub(crate) scroll: usize,
    pub(crate) visible: usize,
    pub(crate) scrollable: bool,
}

/// Envuelve un texto en líneas de ≤ `max_chars` caracteres, respetando los
/// saltos de línea del texto (párrafos) y cortando palabras más largas que
/// la línea. `max_chars` es una ESTIMACIÓN (caracteres por línea, latino):
/// suficiente para un panel de lectura; la medición real de glifos exigiría
/// JNI (`Paint.measureText`), que se evita a propósito (cambios mínimos).
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new()); // línea en blanco: separa párrafos
            continue;
        }
        let mut cur = String::new();
        for word in para.split(' ') {
            if word.chars().count() > max_chars {
                // Palabra más larga que la línea: cortar en pedazos.
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut rest = word;
                while rest.chars().count() > max_chars {
                    let cut = rest
                        .char_indices()
                        .nth(max_chars)
                        .map(|(i, _)| i)
                        .unwrap_or(rest.len());
                    let (a, b) = rest.split_at(cut);
                    out.push(a.to_string());
                    rest = b;
                }
                if !rest.is_empty() {
                    cur = rest.to_string();
                }
                continue;
            }
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.chars().count() + 1 + word.chars().count() <= max_chars {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        out.push(cur);
    }
    out
}

/// Layout del panel de IA: tarjeta centrada (margen 16 px, ancho ≤ 560 px),
/// cabecera fija de 44 px (título + botones a la derecha) y cuerpo cuyo alto
/// se ajusta al texto: si cabe entero, panel corto SIN scroll; si desborda,
/// cuerpo limitado a ~55 % de la ventana y botones ▲/▼. El scroll actual se
/// lee de `Reader::ai_panel` (0 al abrir) y se devuelve recortado al rango
/// válido para que `Reader::rebuild_ai_panel` lo guarde. None si la ventana
/// es demasiado pequeña para un panel.
pub(crate) fn ai_panel_layout(reader: &Reader) -> Option<AiLayout> {
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    if win_w < 200.0 || win_h < 200.0 {
        return None;
    }
    // Constantes del panel (las MISMAS en layout y render).
    let ts = 13.0f32; // cuerpo
    let line_h = (ts * 1.5).round(); // 20 px
    let pad = 14.0f32;
    let header_h = 44.0f32;
    let btn = 30.0f32;
    let gap = 6.0f32;
    let margin = 16.0f32;
    let panel_w = (win_w - 2.0 * margin).clamp(200.0, 560.0);
    let max_body_h = (win_h * 0.55).clamp(120.0, 340.0);
    // Envolver el texto (estimación de caracteres por línea para latino).
    let content_w = panel_w - 2.0 * pad;
    let max_chars = ((content_w / (ts * 0.52)).floor() as usize).max(8);
    let mut lines = wrap_text(&reader.ai_text, max_chars);
    if lines.is_empty() {
        lines.push("…".to_string()); // defensa: texto vacío
    }
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
    // Botones de la cabecera, alineados a la derecha: [▲][▼][×] (▲/▼ solo
    // si `scrollable`). Misma geometría para render y tap.
    let mut buttons: Vec<(&'static str, ButtonRect)> = Vec::with_capacity(3);
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
/// Asking = "Preguntando a la IA…" en gris (estado transitorio), Answer =
/// texto claro, Error = "IA — error" con texto rojizo (`STATUS_*`).
pub(crate) fn render_ai_panel(reader: &Reader) -> Option<Bitmap> {
    let layout = ai_panel_layout(reader)?;
    let (mx, my, mrx, mry) = layout.rect;
    let w = (mrx - mx) as i32;
    let h = (mry - my) as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let p = reader.theme.palette();
    let ts = theme::FONT_BODY;
    let line_h = (ts * 1.5).round();
    let pad = 14.0f32;
    let header_h = 44.0f32;
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
    // Cabecera: título según la fase + botones ✕/▲/▼
    let (title, body_color) = match reader.ai_phase {
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
        draw_button(
            &mut rects,
            &mut texts,
            *l - mx,
            *t - my,
            *rr - mx,
            *b - my,
            p.base_200,
            p.base_300,
            p.base_content,
            theme::FONT_CAPTION,
            false,
            label,
        );
    }
    // Cuerpo: solo las líneas visibles [scroll, scroll+visible)
    let body_top = header_h + pad;
    for (i, line) in layout
        .lines
        .iter()
        .enumerate()
        .skip(layout.scroll)
        .take(layout.visible)
    {
        let y = body_top + (i - layout.scroll) as f32 * line_h + ts * 0.9;
        texts.push(CanvasText::new(
            pad,
            y,
            ts,
            body_color,
            TextAlign::Left,
            false,
            line.clone(),
        ));
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
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    );
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
