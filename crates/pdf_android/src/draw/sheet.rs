// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Sheet de ajustes del visor: geometría de botones (compartida con
//! `input::sheet_tap`) y render del panel deslizante (Canvas+JNI).

use pdf_core::{Bitmap, Document};
use crate::reader::{
    Reader,
    sheet_act_y,
    sheet_btn_h,
    sheet_btn_w,
    sheet_h,
    sheet_nav_y,
    sheet_pad,
    sheet_theme_btn_w,
    sheet_theme_y,
};
use crate::theme;

use super::{
    ButtonRect,
    CanvasRect,
    CanvasText,
    TextAlign,
    draw_button,
    draw_card_shadow,
    jni_text_bitmap,
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

/// Renderiza el sheet de ajustes del visor (S1, S2, S3, G1, G2, G3):
/// panel de verdad 55-60% win_h, radio 24 px, sombra proyectada, estructura limpia.
pub(crate) fn render_sheet(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = sheet_h(reader.win_h);
    let pad = sheet_pad(w);
    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Sombra proyectada hacia el contenido (S1, G1)
    draw_card_shadow(
        &mut rects,
        0.0,
        h as f32 - 16.0,
        w as f32,
        h as f32 + 12.0,
        24.0,
        p.is_dark,
    );

    // Fondo base_100 con borde inferior radio 24 px (S1)
    let card_r = 24.0f32;
    rects.push(CanvasRect::rounded(
        0.0, -24.0, w as f32, h as f32, card_r, p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        -24.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        card_r - 1.0,
        p.base_100,
    ));

    // Asa central minimalista
    let handle_w = 48.0f32;
    rects.push(CanvasRect::rounded(
        (w as f32 - handle_w) / 2.0,
        10.0,
        (w as f32 + handle_w) / 2.0,
        14.0,
        2.0,
        p.base_300,
    ));

    // Título de sección en MAYÚSCULAS 11 sp neutral_content (S2)
    texts.push(CanvasText::new(
        w as f32 / 2.0,
        34.0,
        theme::FONT_LABEL_CAPS,
        p.neutral_content,
        TextAlign::Center,
        true,
        "AJUSTES",
    ));

    // Sección 1: TEMA
    let theme_y = sheet_theme_y(reader.win_h);
    texts.push(CanvasText::new(
        pad,
        theme_y - 12.0,
        theme::FONT_LABEL_CAPS,
        p.neutral_content,
        TextAlign::Left,
        true,
        "TEMA",
    ));

    // Sección 2: NAVEGACIÓN
    let nav_y = sheet_nav_y(reader.win_h);
    texts.push(CanvasText::new(
        pad,
        nav_y - 12.0,
        theme::FONT_LABEL_CAPS,
        p.neutral_content,
        TextAlign::Left,
        true,
        "LECTURA",
    ));

    // Sección 3: ACCIONES
    let act_y = sheet_act_y(reader.win_h);
    texts.push(CanvasText::new(
        pad,
        act_y - 12.0,
        theme::FONT_LABEL_CAPS,
        p.neutral_content,
        TextAlign::Left,
        true,
        "DOCUMENTO",
    ));

    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let all_themes = [
        (crate::theme::AppTheme::DefaultLight, "Claro"),
        (crate::theme::AppTheme::SepiaLight, "Sepia"),
        (crate::theme::AppTheme::DefaultDark, "Oscuro"),
        (crate::theme::AppTheme::SepiaDark, "Sepia D."),
    ];

    for (label, (l, t, r, b)) in sheet_buttons(reader, w as f32, reader.win_h as f32) {
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

            // Fondo y borde del botón de tema (alto >= 48 px, S3)
            let border_c = if is_active { p.primary } else { p.base_300 };
            let fill_c = p.base_200;
            rects.push(CanvasRect::rounded(l, t, r, b, 24.0, border_c));
            rects.push(CanvasRect::rounded(
                l + 1.0,
                t + 1.0,
                r - 1.0,
                b - 1.0,
                23.0,
                fill_c,
            ));

            // Swatch circular de 26 px: muestra el color de FONDO del tema (B4)
            // (Default-Light = #FFFFFF, Sepia-Light = #F1E8D0, Default-Dark = #242424, Sepia-Dark = #342E25)
            let swatch_x = l + 20.0;
            let swatch_cy = (t + b) / 2.0;
            let swatch_r = 13.0f32;

            // Anillo primary de 2 px solo si está activo (B4)
            if is_active {
                rects.push(CanvasRect::rounded(
                    swatch_x - swatch_r - 2.5,
                    swatch_cy - swatch_r - 2.5,
                    swatch_x + swatch_r + 2.5,
                    swatch_cy + swatch_r + 2.5,
                    swatch_r + 2.5,
                    p.primary,
                ));
                rects.push(CanvasRect::rounded(
                    swatch_x - swatch_r - 0.5,
                    swatch_cy - swatch_r - 0.5,
                    swatch_x + swatch_r + 0.5,
                    swatch_cy + swatch_r + 0.5,
                    swatch_r + 0.5,
                    fill_c,
                ));
            } else {
                rects.push(CanvasRect::rounded(
                    swatch_x - swatch_r - 1.0,
                    swatch_cy - swatch_r - 1.0,
                    swatch_x + swatch_r + 1.0,
                    swatch_cy + swatch_r + 1.0,
                    swatch_r + 1.0,
                    th_pal.base_300,
                ));
            }

            // Relleno del swatch con el color de fondo base_100 del tema destino (B4)
            rects.push(CanvasRect::rounded(
                swatch_x - swatch_r,
                swatch_cy - swatch_r,
                swatch_x + swatch_r,
                swatch_cy + swatch_r,
                swatch_r,
                th_pal.base_100,
            ));

            // Texto del tema
            texts.push(CanvasText::new(
                swatch_x + swatch_r + 10.0,
                swatch_cy + theme::FONT_CAPTION * 0.35,
                theme::FONT_CAPTION,
                if is_active {
                    p.base_content
                } else {
                    p.neutral_content
                },
                TextAlign::Left,
                is_active,
                name,
            ));
            continue;
        }

        let (fill, border, text_color, label_str) = match label {
            "-10" => (p.base_200, p.base_300, p.base_content, "◀ −10".to_string()),
            "+10" => (p.base_200, p.base_300, p.base_content, "+10 ▶".to_string()),
            "N / total" => {
                let pct = if pages > 0 {
                    ((reader.page + 1) as f32 / pages as f32 * 100.0).round()
                } else {
                    0.0
                };
                (
                    p.base_200,
                    p.base_300,
                    p.base_content,
                    format!("Pág. {} / {} ({:.0}%)", reader.page + 1, pages, pct),
                )
            }
            "← Library" => (
                p.base_200,
                p.base_300,
                p.base_content,
                "← Biblioteca".to_string(),
            ),
            "Search" => (
                p.base_200,
                p.base_300,
                p.base_content,
                "🔍 Buscar".to_string(),
            ),
            "Close" => (
                p.primary,
                p.primary,
                p.primary_content,
                "✕ Cerrar".to_string(),
            ),
            _ => (p.base_200, p.base_300, p.base_content, label.to_string()),
        };

        draw_button(
            &mut rects,
            &mut texts,
            l,
            t,
            r,
            b,
            fill,
            border,
            text_color,
            theme::FONT_BODY,
            true,
            &label_str,
        );
    }

    jni_text_bitmap(w, h, theme::TRANSPARENT, &rects, &texts)
}
