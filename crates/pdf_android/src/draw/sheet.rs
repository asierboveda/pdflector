// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Sheet de ajustes del visor: geometría de botones (compartida con
//! `input::sheet_tap`) y render del panel deslizante (Canvas+JNI).

use crate::reader::{
    Reader, sheet_act_y, sheet_btn_h, sheet_btn_w, sheet_h, sheet_nav_y, sheet_pad,
    sheet_theme_btn_w, sheet_theme_y,
};
use crate::theme;
use pdf_core::{Bitmap, Document};

use super::{
    ButtonRect, ButtonVariant, CanvasRect, CanvasText, TextAlign, TextFont, draw_astryx_button,
    draw_inset_ring, jni_text_bitmap,
};

/// Botones del sheet de ajustes del visor (S2, S3):
/// Fila 0 = 4 Temas (swatches); Fila 1 = Navegación (−10 / Pág / +10); Fila 2 = Acciones.
pub(crate) fn sheet_buttons(
    _reader: &Reader,
    win_w: f32,
    win_h: f32,
) -> Vec<(&'static str, ButtonRect)> {
    let pad = sheet_pad(win_w as i32);
    let bh = sheet_btn_h(win_h as i32);
    let mut out = Vec::with_capacity(10);

    // Fila 0: 4 temas
    let tw = sheet_theme_btn_w(win_w as i32);
    let ty = sheet_theme_y(win_h as i32);
    let themes = ["Theme:Light", "Theme:Sepia", "Theme:Dark", "Theme:Nord"];
    for (i, label) in themes.into_iter().enumerate() {
        let x0 = pad + i as f32 * (tw + pad);
        out.push((label, (x0, ty, x0 + tw, ty + bh)));
    }

    // Fila 1: 3 botones de navegación
    let bw = sheet_btn_w(win_w as i32);
    let ny = sheet_nav_y(win_h as i32);
    for (i, label) in ["-10", "N / total", "+10"].into_iter().enumerate() {
        let x0 = pad + i as f32 * (bw + pad);
        out.push((label, (x0, ny, x0 + bw, ny + bh)));
    }

    // Fila 2: 3 botones de acción
    let ay = sheet_act_y(win_h as i32);
    for (i, label) in ["← Library", "Search", "Close"].into_iter().enumerate() {
        let x0 = pad + i as f32 * (bw + pad);
        out.push((label, (x0, ay, x0 + bw, ay + bh)));
    }

    out
}

/// Renderiza el sheet de ajustes del visor en lenguaje Astryx:
/// superficie `bg_popover` elevada, mini-tarjetas de vista previa de tema con anillo
/// de foco, secciones con tracking y controles `RADIUS_ELEMENT`.
pub(crate) fn render_sheet(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = sheet_h(reader.win_h);
    let pad = sheet_pad(w);
    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    let card_r = theme::RADIUS_PAGE;

    // Superficie elevada del panel
    rects.push(CanvasRect::rounded(
        0.0,
        -card_r,
        w as f32,
        h as f32,
        card_r,
        p.border_emphasized,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        -card_r,
        w as f32 - 1.0,
        h as f32 - 1.0,
        card_r - 1.0,
        p.bg_popover,
    ));

    // Asa central sutil
    let handle_w = 44.0f32;
    rects.push(CanvasRect::rounded(
        (w as f32 - handle_w) / 2.0,
        10.0,
        (w as f32 + handle_w) / 2.0,
        14.0,
        2.0,
        p.border_emphasized,
    ));

    // Cabecera de panel con tracking Astryx
    texts.push(
        CanvasText::new(
            w as f32 / 2.0,
            35.0,
            theme::FONT_LABEL_CAPS,
            p.text_secondary,
            TextAlign::Center,
            true,
            "AJUSTES DE LECTURA",
        )
        .with_tracking(theme::TRACKING_CAPS),
    );

    // Secciones con divisores hairline y kickers tracked
    let theme_y = sheet_theme_y(reader.win_h);
    let nav_y = sheet_nav_y(reader.win_h);
    let act_y = sheet_act_y(reader.win_h);
    for (y, title, draw_rule) in [
        (theme_y, "APARIENCIA", false),
        (nav_y, "NAVEGACIÓN", true),
        (act_y, "DOCUMENTO", true),
    ] {
        if draw_rule {
            rects.push(CanvasRect::sharp(
                pad,
                y - 34.0,
                w as f32 - pad,
                y - 33.0,
                p.border,
            ));
        }
        texts.push(
            CanvasText::new(
                pad,
                y - 12.0,
                theme::FONT_LABEL_CAPS,
                p.text_secondary,
                TextAlign::Left,
                true,
                title,
            )
            .with_tracking(theme::TRACKING_CAPS),
        );
    }

    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let all_themes = [
        (theme::AppTheme::DefaultLight, "Claro"),
        (theme::AppTheme::SepiaLight, "Papel"),
        (theme::AppTheme::DefaultDark, "Oscuro"),
        (theme::AppTheme::SepiaDark, "Nocturno"),
    ];

    for (label, rect) in sheet_buttons(reader, w as f32, reader.win_h as f32) {
        let (l, t, r, b) = rect;
        if label.starts_with("Theme:") {
            let idx = match label {
                "Theme:Light" => 0,
                "Theme:Sepia" => 1,
                "Theme:Dark" => 2,
                "Theme:Nord" => 3,
                _ => 0,
            };
            let (th, name) = all_themes[idx];
            let is_active = reader.theme == th;
            let th_pal = th.palette();

            // Contenedor Astryx con anillo de selección activo (`RADIUS_CONTAINER`)
            if is_active {
                draw_inset_ring(
                    &mut rects,
                    rect,
                    theme::RADIUS_CONTAINER,
                    p.accent,
                    p.accent_muted,
                );
            } else {
                rects.push(CanvasRect::rounded(
                    l,
                    t,
                    r,
                    b,
                    theme::RADIUS_CONTAINER,
                    p.border,
                ));
                rects.push(CanvasRect::rounded(
                    l + 1.0,
                    t + 1.0,
                    r - 1.0,
                    b - 1.0,
                    theme::RADIUS_CONTAINER - 1.0,
                    p.bg_card,
                ));
            }

            // Mini-tarjeta de vista previa del tema (superficie + líneas de tinta + barra de acento)
            let pv_h = ((b - t) - 16.0).max(24.0);
            let pv_w = (pv_h * 0.88).clamp(24.0, 34.0);
            let pv_l = l + 10.0;
            let pv_t = (t + b - pv_h) * 0.5;
            let pv_r = pv_l + pv_w;
            let pv_b = pv_t + pv_h;
            rects.push(CanvasRect::rounded(
                pv_l,
                pv_t,
                pv_r,
                pv_b,
                theme::RADIUS_INNER + 1.0,
                th_pal.border_emphasized,
            ));
            rects.push(CanvasRect::rounded(
                pv_l + 1.0,
                pv_t + 1.0,
                pv_r - 1.0,
                pv_b - 1.0,
                theme::RADIUS_INNER,
                th_pal.bg_body,
            ));
            // Dos micro-líneas de texto simulado + barra de acento del tema
            rects.push(CanvasRect::rounded(
                pv_l + 5.0,
                pv_t + 6.0,
                pv_r - 5.0,
                pv_t + 8.5,
                1.0,
                th_pal.text_primary,
            ));
            rects.push(CanvasRect::rounded(
                pv_l + 5.0,
                pv_t + 12.0,
                pv_r - 9.0,
                pv_t + 14.0,
                1.0,
                th_pal.text_secondary,
            ));
            rects.push(CanvasRect::rounded(
                pv_l + 5.0,
                pv_b - 8.5,
                pv_r - 5.0,
                pv_b - 4.5,
                2.0,
                th_pal.accent,
            ));

            // Nombre del tema
            let text_x = pv_r + 10.0;
            let text_cy = (t + b) * 0.5 + theme::FONT_CAPTION * 0.35;
            texts.push(CanvasText::new(
                text_x,
                text_cy,
                theme::FONT_CAPTION,
                if is_active {
                    p.on_accent_muted
                } else {
                    p.text_primary
                },
                TextAlign::Left,
                is_active,
                name,
            ));
            continue;
        }

        if label == "N / total" {
            let pct = if pages > 0 {
                ((reader.page + 1) as f32 / pages as f32 * 100.0).round()
            } else {
                0.0
            };
            rects.push(CanvasRect::rounded(
                l,
                t,
                r,
                b,
                theme::RADIUS_ELEMENT,
                p.border,
            ));
            rects.push(CanvasRect::rounded(
                l + 1.0,
                t + 1.0,
                r - 1.0,
                b - 1.0,
                theme::RADIUS_ELEMENT - 1.0,
                p.bg_muted,
            ));
            let label_str = format!("Pág. {} / {} · {:.0}%", reader.page + 1, pages, pct);
            texts.push(
                CanvasText::new(
                    (l + r) * 0.5,
                    (t + b) * 0.5 + theme::FONT_BODY * 0.35,
                    theme::FONT_BODY,
                    p.text_primary,
                    TextAlign::Center,
                    true,
                    label_str,
                )
                .with_font(TextFont::Mono),
            );
            continue;
        }

        let (variant, label_str) = match label {
            "-10" => (ButtonVariant::Secondary, "‹ −10 págs."),
            "+10" => (ButtonVariant::Secondary, "+10 págs. ›"),
            "← Library" => (ButtonVariant::Secondary, "← Biblioteca"),
            "Search" => (ButtonVariant::Secondary, "Buscar"),
            "Close" => (ButtonVariant::Primary, "Listo"),
            _ => (ButtonVariant::Secondary, label),
        };

        draw_astryx_button(
            &mut rects,
            &mut texts,
            rect,
            variant,
            &p,
            theme::FONT_BODY,
            true,
            label_str,
        );
    }

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}
