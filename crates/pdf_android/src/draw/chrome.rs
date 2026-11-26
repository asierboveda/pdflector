// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Chrome del visor: barras superior e inferior flotantes, indicador de
//! página y cursor de la goma — bitmaps
//! Canvas+JNI cacheados en `Reader` (geometría compartida con `input`).

use crate::reader::{
    Reader, page_badge_size, truncate_name, viewer_bottom_chrome_h, viewer_top_chrome_h,
};
use crate::theme;
use pdf_core::{Bitmap, Document};

use super::{
    ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign, TextFont, draw_astryx_button,
    draw_surface_card, draw_token_pill, jni_text_bitmap,
};

/// Renderiza el indicador de página "N / total" (overlay abajo a la
/// izquierda, `page_badge_size`): token flotante Astryx con numeración
/// monoespaciada tabular y punto indicador de acento.
pub(crate) fn render_page_badge(reader: &Reader) -> Option<Bitmap> {
    let (bw, bh) = page_badge_size(reader.win_w, reader.win_h);
    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let label = format!("{} / {}", reader.page + 1, pages);
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    let r = theme::RADIUS_FULL;
    rects.push(CanvasRect::rounded(
        0.0,
        0.0,
        bw as f32,
        bh as f32,
        r,
        p.badge_border(),
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        bw as f32 - 1.0,
        bh as f32 - 1.0,
        r,
        p.badge_bg(),
    ));
    // Punto semántico sutil de estado activo a la izquierda
    let dot_r = 3.0f32;
    let dot_cx = 12.0f32;
    let dot_cy = bh as f32 * 0.5;
    rects.push(CanvasRect::rounded(
        dot_cx - dot_r,
        dot_cy - dot_r,
        dot_cx + dot_r,
        dot_cy + dot_r,
        dot_r,
        p.accent,
    ));
    let ts = theme::FONT_CAPTION;
    texts.push(
        CanvasText::new(
            bw as f32 * 0.5 + 4.0,
            bh as f32 * 0.5 + ts * 0.35,
            ts,
            p.badge_text(),
            TextAlign::Center,
            true,
            label,
        )
        .with_font(TextFont::Mono),
    );
    jni_text_bitmap(bw, bh, theme::TRANSPARENT, &rects, &texts)
}

/// Rectángulo de botón (left, top, right, bottom) en px.
pub(crate) type ButtonRect = (f32, f32, f32, f32);

/// Geometría de los botones de la barra superior flotante del visor (V1, V2, V4).
pub(crate) fn viewer_top_chrome_buttons(win_w: f32, win_h: f32) -> [(&'static str, ButtonRect); 2] {
    let margin_x = (win_w * 0.03).clamp(24.0, 48.0);
    let margin_y = (win_h * 0.02).clamp(28.0, 48.0);
    let card_h = 68.0f32;
    let btn_h = 48.0f32;
    let btn_y = margin_y + (card_h - btn_h) / 2.0;
    let back_w = 138.0f32;
    let theme_btn_size = 48.0f32;
    let pad_inner = 18.0f32;
    [
        (
            "Back",
            (
                margin_x + pad_inner,
                btn_y,
                margin_x + pad_inner + back_w,
                btn_y + btn_h,
            ),
        ),
        (
            "Theme",
            (
                win_w - margin_x - pad_inner - theme_btn_size,
                btn_y,
                win_w - margin_x - pad_inner,
                btn_y + btn_h,
            ),
        ),
    ]
}

/// Rects y textos del botón de ciclo de tema de la barra superior del visor,
/// dentro de `rect` (geometría pura, testeable en host).
fn theme_button_parts(
    rect: ButtonRect,
    p: &theme::ThemePalette,
) -> (Vec<CanvasRect>, Vec<CanvasText>) {
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    // Botón de ciclo de tema: contenedor Astryx RADIUS_ELEMENT con glifo "Aa"
    // (apariencia) en `text_primary` y subrayado de acento del tema activo.
    // Antes era una muestra `bg_body` con aro `border_emphasized` sobre
    // `bg_muted`: colores casi idénticos (p. ej. #F1F4F7 sobre #E6ECF2), así
    // que en la TCL solo se veía la franja azul de acento.
    let (tl, tt, tr, tb) = rect;
    rects.push(CanvasRect::rounded(
        tl,
        tt,
        tr,
        tb,
        theme::RADIUS_ELEMENT,
        p.border_emphasized,
    ));
    rects.push(CanvasRect::rounded(
        tl + 1.0,
        tt + 1.0,
        tr - 1.0,
        tb - 1.0,
        theme::RADIUS_ELEMENT - 1.0,
        p.bg_muted,
    ));
    // Glifo centrado: la línea base queda 0.3 em bajo el centro para que
    // ascendentes y subrayado ocupen el centro óptico del botón.
    let ts = theme::FONT_H1;
    let cx = (tl + tr) * 0.5;
    let baseline = (tt + tb) * 0.5 + ts * 0.3;
    texts.push(CanvasText::new(
        cx,
        baseline,
        ts,
        p.text_primary,
        TextAlign::Center,
        true,
        "Aa",
    ));
    // Subrayado de acento bajo el glifo (indicador del tema activo)
    let dash_w = 16.0f32;
    rects.push(CanvasRect::rounded(
        cx - dash_w * 0.5,
        baseline + 4.0,
        cx + dash_w * 0.5,
        baseline + 7.0,
        1.5,
        p.accent,
    ));

    (rects, texts)
}

/// Renderiza la barra superior flotante Astryx del visor.
pub(crate) fn render_viewer_top_chrome(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let win_h = reader.win_h as f32;
    let h = viewer_top_chrome_h(reader.win_h) as i32;
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();

    let margin_x = (w as f32 * 0.03).clamp(24.0, 48.0);
    let margin_y = (win_h * 0.02).clamp(28.0, 48.0);
    let card_h = 68.0f32;
    let card_rect = (margin_x, margin_y, w as f32 - margin_x, margin_y + card_h);

    // Superficie flotante Astryx (`bg_popover` + hairline enfatizado + Elevation::Med)
    draw_surface_card(
        &mut rects,
        card_rect,
        theme::RADIUS_CARD,
        p.bg_popover,
        p.border_emphasized,
        Elevation::Med,
        p.is_dark,
    );

    // Botones (← Biblioteca y selector de tema)
    let btns = viewer_top_chrome_buttons(w as f32, win_h);
    draw_astryx_button(
        &mut rects,
        &mut texts,
        btns[0].1,
        ButtonVariant::Ghost,
        &p,
        theme::FONT_BODY,
        true,
        "← Biblioteca",
    );

    // Botón de ciclo de tema (geometría pura en `theme_button_parts`)
    let (icon_rects, icon_texts) = theme_button_parts(btns[1].1, &p);
    rects.extend(icon_rects);
    texts.extend(icon_texts);

    // Título editorial limpio (sin extensión .pdf)
    let raw_title = reader
        .doc_path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("PDFLector");
    let title = truncate_name(raw_title, 32);
    texts.push(
        CanvasText::new(
            w as f32 / 2.0,
            margin_y + card_h * 0.5 + theme::FONT_TITLE * 0.34,
            theme::FONT_TITLE,
            p.text_primary,
            TextAlign::Center,
            true,
            title,
        )
        .with_tracking(theme::TRACKING_TIGHT),
    );

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Renderiza la barra inferior flotante Astryx con scrubber y pill tabular de progreso.
pub(crate) fn render_viewer_bottom_chrome(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = viewer_bottom_chrome_h(reader.win_h) as i32;
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();

    let margin_x = (w as f32 * 0.03).clamp(24.0, 48.0);
    let card_top = 8.0f32;
    let card_h = 76.0f32;
    let card_rect = (margin_x, card_top, w as f32 - margin_x, card_top + card_h);

    draw_surface_card(
        &mut rects,
        card_rect,
        theme::RADIUS_CARD,
        p.bg_popover,
        p.border_emphasized,
        Elevation::Med,
        p.is_dark,
    );

    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let pct = if pages > 0 {
        ((reader.page + 1) as f32 / pages as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let pad_inner = 28.0f32;
    let track_x0 = margin_x + pad_inner;
    let track_x1 = w as f32 - margin_x - pad_inner;
    let track_w = (track_x1 - track_x0).max(1.0);

    // Fila superior asimétrica: contador tabular a la izquierda + token tonal de % a la derecha
    let page_label = format!("Pág. {} de {}", reader.page + 1, pages);
    texts.push(
        CanvasText::new(
            track_x0,
            card_top + 27.0,
            theme::FONT_BODY,
            p.text_primary,
            TextAlign::Left,
            true,
            page_label,
        )
        .with_font(TextFont::Mono),
    );

    let pct_str = format!("{:.0}%", pct * 100.0);
    let pill_w = 54.0f32;
    let pill_h = 22.0f32;
    let pill_rect = (
        track_x1 - pill_w,
        card_top + 11.0,
        track_x1,
        card_top + 11.0 + pill_h,
    );
    draw_token_pill(
        &mut rects,
        &mut texts,
        pill_rect,
        p.accent_muted,
        None,
        p.on_accent_muted,
        theme::FONT_CAPTION,
        TextFont::Mono,
        0.0,
        true,
        &pct_str,
    );

    // Scrubber Astryx: pista 6 px + relleno accent + thumb con halo sutil
    let track_y = card_top + 48.0f32;
    let track_h = 6.0f32;
    rects.push(CanvasRect::rounded(
        track_x0,
        track_y,
        track_x1,
        track_y + track_h,
        3.0,
        p.track,
    ));
    let fill_w = (track_w * pct).max(0.0);
    if fill_w > 0.0 {
        rects.push(CanvasRect::rounded(
            track_x0,
            track_y,
            track_x0 + fill_w,
            track_y + track_h,
            3.0,
            p.accent,
        ));
    }

    let thumb_cx = (track_x0 + fill_w).clamp(track_x0, track_x1);
    let thumb_cy = track_y + track_h / 2.0;
    let thumb_r = 10.0f32;
    // Halo de foco Astryx
    rects.push(CanvasRect::rounded(
        thumb_cx - thumb_r - 4.0,
        thumb_cy - thumb_r - 4.0,
        thumb_cx + thumb_r + 4.0,
        thumb_cy + thumb_r + 4.0,
        thumb_r + 4.0,
        p.accent_muted,
    ));
    rects.push(CanvasRect::rounded(
        thumb_cx - thumb_r - 2.0,
        thumb_cy - thumb_r - 2.0,
        thumb_cx + thumb_r + 2.0,
        thumb_cy + thumb_r + 2.0,
        thumb_r + 2.0,
        p.bg_popover,
    ));
    rects.push(CanvasRect::rounded(
        thumb_cx - thumb_r,
        thumb_cy - thumb_r,
        thumb_cx + thumb_r,
        thumb_cy + thumb_r,
        thumb_r,
        p.accent,
    ));

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Cursor de la GOMA durante el borrado: círculo del tamaño REAL de la goma
/// con anillo Astryx y relleno translúcido.
pub(crate) fn render_eraser_cursor(reader: &Reader, radius_px: i32) -> Option<Bitmap> {
    let d = radius_px.max(4) * 2 + 8;
    let p = reader.theme.palette();
    let fill = (p.accent_muted & 0x00FF_FFFF) | 0x5500_0000;
    let mut rects = Vec::new();
    let rr = (d / 2) as f32;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, d as f32, d as f32, rr, p.accent,
    ));
    rects.push(CanvasRect::rounded(
        1.5,
        1.5,
        d as f32 - 1.5,
        d as f32 - 1.5,
        (rr - 1.5).max(0.0),
        fill,
    ));
    jni_text_bitmap(d, d, theme::TRANSPARENT, &rects, &[])
}

#[cfg(test)]
mod tests {
    use super::{ButtonRect, CanvasText, TextAlign, theme_button_parts, viewer_top_chrome_buttons};
    use crate::theme::AppTheme;

    const THEMES: [AppTheme; 4] = [
        AppTheme::DefaultLight,
        AppTheme::SepiaLight,
        AppTheme::DefaultDark,
        AppTheme::SepiaDark,
    ];

    /// Ventanas de la TCL 9469X (apaisada y vertical) y una pequeña.
    const WINDOWS: [(f32, f32); 3] = [(2000.0, 1200.0), (1200.0, 2000.0), (800.0, 600.0)];

    /// Caja aproximada (l, t, r, b) que ocupa un glifo de texto: ascendente
    /// ~0.8 em sobre la línea base, descendente ~0.25 em y ~0.65 em de avance
    /// por carácter (estimación conservadora de Roboto en negrita).
    fn glyph_box(t: &CanvasText) -> ButtonRect {
        let w = t.text.chars().count() as f32 * t.size * 0.65;
        let (l, r) = match t.align {
            TextAlign::Left => (t.x, t.x + w),
            TextAlign::Center => (t.x - w / 2.0, t.x + w / 2.0),
            TextAlign::Right => (t.x - w, t.x),
        };
        (l, t.y - t.size * 0.8, r, t.y + t.size * 0.25)
    }

    fn inside(inner: ButtonRect, outer: ButtonRect) -> bool {
        inner.0 >= outer.0 && inner.1 >= outer.1 && inner.2 <= outer.2 && inner.3 <= outer.3
    }

    /// Luminancia relativa WCAG de un color 0xAARRGGBB.
    fn luminance(c: u32) -> f32 {
        let ch = |v: u32| {
            let s = (v & 0xFF) as f32 / 255.0;
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c >> 16) + 0.7152 * ch(c >> 8) + 0.0722 * ch(c)
    }

    fn contrast(a: u32, b: u32) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn theme_button_draws_a_full_glyph_inside_its_rect() {
        for (w, h) in WINDOWS {
            let rect = viewer_top_chrome_buttons(w, h)[1].1;
            for th in THEMES {
                let p = th.palette();
                let (_, texts) = theme_button_parts(rect, &p);
                assert!(
                    !texts.is_empty(),
                    "{th:?} {w}x{h}: el botón de tema no dibuja ningún glifo (solo rects)"
                );
                for t in &texts {
                    let gb = glyph_box(t);
                    assert!(
                        inside(gb, rect),
                        "{th:?} {w}x{h}: glifo {:?} {gb:?} fuera del botón {rect:?}",
                        t.text
                    );
                    assert!(
                        contrast(t.color, p.bg_muted) >= 4.5,
                        "{th:?}: glifo {:?} sin contraste con el relleno del botón",
                        t.text
                    );
                }
            }
        }
    }

    #[test]
    fn theme_button_rects_stay_inside_its_rect() {
        for (w, h) in WINDOWS {
            let rect = viewer_top_chrome_buttons(w, h)[1].1;
            for th in THEMES {
                let (rects, _) = theme_button_parts(rect, &th.palette());
                for r in &rects {
                    let rr = (r.left, r.top, r.right, r.bottom);
                    assert!(inside(rr, rect), "{th:?}: rect {rr:?} fuera de {rect:?}");
                }
            }
        }
    }
}
