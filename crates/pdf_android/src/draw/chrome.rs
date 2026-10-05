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

use super::{CanvasRect, CanvasText, TextAlign, draw_button, draw_card_shadow, jni_text_bitmap};

/// Renderiza el indicador de página "N / total" (overlay abajo a la
/// izquierda, `page_badge_size`): un badge pequeño con el número actual y el
/// total. Cacheado en `Reader::page_badge` (se invalida al cambiar ventana,
/// página o modo oscuro); el tap en él avanza a la página siguiente
/// (`input::page_badge_tap` — decisión documentada: el indicador se puede
/// tocar como acceso rápido a la página siguiente).
pub(crate) fn render_page_badge(reader: &Reader) -> Option<Bitmap> {
    let (bw, bh) = page_badge_size(reader.win_w, reader.win_h);
    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let label = format!("{} / {}", reader.page + 1, pages);
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
        label,
    ));
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
    let pad_inner = 18.0f32; // [A2] Padding >= 16 px dentro de la tarjeta
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

/// Renderiza la barra superior flotante de chrome del visor (V1, V2, V4).
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

    // Sombra flotante de la barra superior (V1, G1)
    draw_card_shadow(
        &mut rects,
        margin_x,
        margin_y,
        w as f32 - margin_x,
        margin_y + card_h,
        24.0,
        p.is_dark,
    );

    // Tarjeta redondeada radio 24 px (V1)
    rects.push(CanvasRect::rounded(
        margin_x,
        margin_y,
        w as f32 - margin_x,
        margin_y + card_h,
        24.0,
        p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        margin_x + 1.0,
        margin_y + 1.0,
        w as f32 - margin_x - 1.0,
        margin_y + card_h - 1.0,
        23.0,
        p.base_100,
    ));

    // Botones (Atrás y Swatch Tema)
    let btns = viewer_top_chrome_buttons(w as f32, win_h);
    let (back_l, back_t, back_r, back_b) = btns[0].1;
    draw_button(
        &mut rects,
        &mut texts,
        back_l,
        back_t,
        back_r,
        back_b,
        p.base_200,
        p.base_300,
        p.base_content,
        theme::FONT_BODY,
        true,
        "← Biblioteca",
    );

    // Swatch circular (26 px) del color primary del tema activo como botón de ciclo de tema (V4)
    let (theme_l, theme_t, theme_r, theme_b) = btns[1].1;
    let swatch_cx = (theme_l + theme_r) / 2.0;
    let swatch_cy = (theme_t + theme_b) / 2.0;
    let swatch_r = 13.0f32;
    rects.push(CanvasRect::rounded(
        theme_l, theme_t, theme_r, theme_b, 24.0, p.base_200,
    ));
    rects.push(CanvasRect::rounded(
        swatch_cx - swatch_r - 2.0,
        swatch_cy - swatch_r - 2.0,
        swatch_cx + swatch_r + 2.0,
        swatch_cy + swatch_r + 2.0,
        swatch_r + 2.0,
        p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        swatch_cx - swatch_r,
        swatch_cy - swatch_r,
        swatch_cx + swatch_r,
        swatch_cy + swatch_r,
        swatch_r,
        p.primary,
    ));

    // Título centrado del libro (16 sp peso 600 base-content, V4)
    let title = reader
        .doc_path
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|s| s.to_str())
        .map(|s| truncate_name(s, 26))
        .unwrap_or_else(|| "PDFLector".to_string());
    texts.push(CanvasText::new(
        w as f32 / 2.0,
        margin_y + card_h * 0.5 + theme::FONT_TITLE * 0.35,
        theme::FONT_TITLE,
        p.base_content,
        TextAlign::Center,
        true,
        title,
    ));

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Renderiza la barra inferior de progreso del visor con SLIDER estilo Readest (V1, V3).
pub(crate) fn render_viewer_bottom_chrome(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = viewer_bottom_chrome_h(reader.win_h) as i32;
    let p = reader.theme.palette();
    let mut rects = Vec::new();
    let mut texts = Vec::new();

    let margin_x = (w as f32 * 0.03).clamp(24.0, 48.0);
    let card_top = 8.0f32;
    let card_h = 76.0f32;

    // Sombra flotante de la barra inferior (V1, G1)
    draw_card_shadow(
        &mut rects,
        margin_x,
        card_top,
        w as f32 - margin_x,
        card_top + card_h,
        24.0,
        p.is_dark,
    );

    // Tarjeta redondeada radio 24 px (V1)
    rects.push(CanvasRect::rounded(
        margin_x,
        card_top,
        w as f32 - margin_x,
        card_top + card_h,
        24.0,
        p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        margin_x + 1.0,
        card_top + 1.0,
        w as f32 - margin_x - 1.0,
        card_top + card_h - 1.0,
        23.0,
        p.base_100,
    ));

    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let pct = if pages > 0 {
        ((reader.page + 1) as f32 / pages as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Texto: "Página N de M · P%" dentro de la tarjeta a 13 sp base-content (V3, G3)
    let label = format!(
        "Pág. {} de {} · {:.0}%",
        reader.page + 1,
        pages,
        pct * 100.0
    );
    texts.push(CanvasText::new(
        w as f32 / 2.0,
        card_top + 26.0,
        theme::FONT_BODY,
        p.base_content,
        TextAlign::Center,
        true,
        label,
    ));

    // SLIDER: Track 6 px radio completo base-300 con fill primary (V3)
    let pad_inner = 32.0f32;
    let track_x0 = margin_x + pad_inner;
    let track_x1 = w as f32 - margin_x - pad_inner;
    let track_w = track_x1 - track_x0;
    let track_y = card_top + 46.0f32;
    let track_h = 6.0f32;

    rects.push(CanvasRect::rounded(
        track_x0,
        track_y,
        track_x1,
        track_y + track_h,
        3.0,
        p.base_300,
    ));
    let fill_w = (track_w * pct).max(0.0);
    if fill_w > 0.0 {
        rects.push(CanvasRect::rounded(
            track_x0,
            track_y,
            track_x0 + fill_w,
            track_y + track_h,
            3.0,
            p.primary,
        ));
    }

    // THUMB circular de 22 px primary con borde de 2 px base-100 (V3)
    let thumb_cx = (track_x0 + fill_w).clamp(track_x0, track_x1);
    let thumb_cy = track_y + track_h / 2.0;
    let thumb_r = 11.0f32;
    rects.push(CanvasRect::rounded(
        thumb_cx - thumb_r - 2.0,
        thumb_cy - thumb_r - 2.0,
        thumb_cx + thumb_r + 2.0,
        thumb_cy + thumb_r + 2.0,
        thumb_r + 2.0,
        p.base_100,
    ));
    rects.push(CanvasRect::rounded(
        thumb_cx - thumb_r,
        thumb_cy - thumb_r,
        thumb_cx + thumb_r,
        thumb_cy + thumb_r,
        thumb_r,
        p.primary,
    ));

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}

/// Cursor de la GOMA durante el borrado: círculo del tamaño REAL de la goma
/// (`radius_px`, = radio en puntos × escala efectiva) con borde visible y
/// relleno translúcido — el usuario ve exactamente qué se va a borrar.
pub(crate) fn render_eraser_cursor(reader: &Reader, radius_px: i32) -> Option<Bitmap> {
    let d = radius_px.max(4) * 2 + 8;
    let p = reader.theme.palette();
    // Relleno translúcido sutil (mismo tono del badge, alpha 0x66).
    let fill = (p.badge_bg() & 0x00FF_FFFF) | 0x6600_0000;
    let mut rects = Vec::new();
    let rr = (d / 2) as f32;
    rects.push(CanvasRect::rounded(
        0.0,
        0.0,
        d as f32,
        d as f32,
        rr,
        p.badge_border(),
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        d as f32 - 1.0,
        d as f32 - 1.0,
        rr - 1.0,
        fill,
    ));
    jni_text_bitmap(d, d, theme::TRANSPARENT, &rects, &[])
}
