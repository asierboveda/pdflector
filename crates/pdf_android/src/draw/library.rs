// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pantalla de biblioteca: cabecera fija + zona scrolleable, filas de
//! chips/carousel, blit de la biblioteca y pegado de portadas cacheadas.

use crate::android_activity::ndk::native_window::NativeWindow;
use crate::persist;
use crate::reader::{
    GRID_CELL_PAD, LibraryCoverFit, Reader, cover_size_multiplier, entry_title, grid_cell_h,
    grid_cell_rect, grid_cell_w, grid_cover_h, grid_cover_w, grid_pad, lib_add_btn_w, lib_chip_h,
    lib_chips, lib_collection_chips, lib_content_y0, lib_empty_state_geom, lib_grid_y0,
    lib_header_h, lib_search_h, list_row_gap, list_row_h, list_row_rect, picker_row_h,
    settings_menu_button_rect, truncate_name, view_menu_button_rect,
};
use crate::theme;
use log::warn;
use pdf_core::Bitmap;

use super::{
    ButtonRect, ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign, TextFont, copy_region,
    draw_astryx_button, draw_inset_ring, draw_settings_menu, draw_surface_card, draw_token_pill,
    draw_view_menu, fill_buffer, jni_text_bitmap, primitives::copy_region_blend,
    settings_menu_geometry, view_menu_geometry,
};

/// Dibuja una portada editorial tipográfica Astryx mientras la miniatura del PDF
/// todavía no está en caché: lomo lateral sutil, filete de acento, micro-etiqueta
/// `PDF` monoespaciada y título en serif.
fn draw_cover_placeholder(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    (x0, y0, w, h): (f32, f32, f32, f32),
    title: &str,
    p: &theme::ThemePalette,
    compact: bool,
) {
    // Banda de lomo izquierdo
    let spine_w = if compact { 4.0 } else { 7.0 };
    rects.push(CanvasRect::sharp(
        x0 + 1.0,
        y0 + 1.0,
        x0 + 1.0 + spine_w,
        y0 + h - 1.0,
        p.bg_muted,
    ));
    rects.push(CanvasRect::sharp(
        x0 + 1.0 + spine_w,
        y0 + 1.0,
        x0 + 2.0 + spine_w,
        y0 + h - 1.0,
        p.border,
    ));

    let inner_x = x0 + spine_w + if compact { 6.0 } else { 14.0 };
    let inner_cx = x0 + spine_w * 0.5 + w * 0.5;

    if compact {
        rects.push(CanvasRect::rounded(
            inner_x,
            y0 + 12.0,
            (inner_x + 16.0).min(x0 + w - 6.0),
            y0 + 14.0,
            1.0,
            p.accent,
        ));
        texts.push(
            CanvasText::new(
                inner_cx,
                y0 + h * 0.58,
                theme::FONT_MICRO,
                p.text_secondary,
                TextAlign::Center,
                true,
                truncate_name(title, 7),
            )
            .with_font(TextFont::Serif),
        );
    } else {
        // Filete de acento superior + micro-kicker PDF
        let rule_w = (w * 0.24).clamp(18.0, 34.0);
        rects.push(CanvasRect::rounded(
            inner_x,
            y0 + 18.0,
            inner_x + rule_w,
            y0 + 20.5,
            1.25,
            p.accent,
        ));
        texts.push(
            CanvasText::new(
                inner_x,
                y0 + 36.0,
                theme::FONT_MICRO,
                p.text_secondary,
                TextAlign::Left,
                true,
                "PDF",
            )
            .with_font(TextFont::Mono)
            .with_tracking(theme::TRACKING_CAPS),
        );
        // Título editorial centrado en Serif
        let max_chars = ((w - spine_w - 22.0) / (theme::FONT_BODY * 0.56)) as usize;
        texts.push(
            CanvasText::new(
                inner_cx,
                y0 + h * 0.54 + theme::FONT_BODY * 0.3,
                theme::FONT_BODY,
                p.text_primary,
                TextAlign::Center,
                true,
                truncate_name(title, max_chars.clamp(6, 14)),
            )
            .with_font(TextFont::Serif)
            .with_tracking(theme::TRACKING_TIGHT),
        );
        // Pequeña línea decorativa inferior
        rects.push(CanvasRect::rounded(
            inner_x,
            y0 + h - 22.0,
            (x0 + w - 16.0).max(inner_x + 12.0),
            y0 + h - 20.5,
            0.75,
            p.border,
        ));
    }
}

/// Dibuja el token de porcentaje Astryx sobre la esquina inferior derecha de una portada
/// usando tinta invertida de alto contraste (`bg_inverted` / `on_inverted`) y cifras Mono.
fn draw_cover_progress_badge(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    cover_rect: ButtonRect,
    pct: f32,
    p: &theme::ThemePalette,
    compact: bool,
) {
    let pct_val = (pct * 100.0).round() as u32;
    if pct_val == 0 {
        return;
    }
    let pct_str = format!("{pct_val}%");
    let (badge_w, badge_h, inset, ts) = if compact {
        (38.0f32, 20.0f32, 4.0f32, theme::FONT_MICRO)
    } else {
        (46.0f32, 22.0f32, 8.0f32, theme::FONT_MICRO)
    };
    let badge_r = cover_rect.2 - inset;
    let badge_b = cover_rect.3 - inset;
    let badge_l = badge_r - badge_w;
    let badge_t = badge_b - badge_h;
    draw_token_pill(
        rects,
        texts,
        (badge_l, badge_t, badge_r, badge_b),
        p.bg_inverted,
        Some(p.border_emphasized),
        p.on_inverted,
        ts,
        TextFont::Mono,
        0.02,
        true,
        &pct_str,
    );
}

/// Render de la ZONA FIJA de la biblioteca (cabecera editorial Astryx + campo de
/// búsqueda + chips de colección/asignatura + franja de estado + overlay de menú si está abierto).
pub(crate) fn render_library_header(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h_fixed = lib_content_y0(reader);
    if w <= 0 || h_fixed <= 0 {
        return None;
    }

    let card_b = if reader.view_menu_open {
        view_menu_geometry(reader.win_w, reader.win_h).0.3
    } else if reader.settings_menu_open {
        settings_menu_geometry(reader.win_w, reader.win_h).0.3
    } else {
        0.0
    };
    let h_total = if reader.view_menu_open || reader.settings_menu_open {
        h_fixed.max(card_b.ceil() as i32 + 20)
    } else {
        h_fixed
    };

    let pad = grid_pad(w);
    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();
    let p = reader.theme.palette();

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Superficie elevada de barra superior (`bg_surface`) + hairline `border` 1 px
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        h_fixed as f32,
        p.bg_surface,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        h_fixed as f32 - 1.0,
        w as f32,
        h_fixed as f32,
        p.border,
    ));

    // ---- CABECERA: control segmentado [Biblioteca | Descubrir] + CTA primario "＋ Añadir" ----
    let top_pad = 36.0f32;
    let btn_w = lib_add_btn_w(w);
    let btn_h = ((header_h - top_pad) * 0.52).clamp(38.0, 46.0);
    let btn_y = top_pad + (header_h - top_pad - btn_h) / 2.0;
    let btn_x = w as f32 - pad - btn_w;
    super::draw_header_tabs(
        &mut rects,
        &mut texts,
        w,
        reader.win_h,
        &reader.theme,
        crate::reader::UiMode::Library,
    );
    draw_astryx_button(
        &mut rects,
        &mut texts,
        (btn_x, btn_y, btn_x + btn_w, btn_y + btn_h),
        ButtonVariant::Primary,
        &p,
        theme::FONT_BODY,
        true,
        "＋ Añadir",
    );

    // ---- Botones de MENÚ: Vista "⋯" y Ajustes "☰" (azulejos Astryx `RADIUS_ELEMENT`) ----
    let mut menu_tile = |open: bool, rect: ButtonRect, ch: &str| {
        let (l, t, r, b) = rect;
        if open {
            draw_inset_ring(
                &mut rects,
                rect,
                theme::RADIUS_ELEMENT,
                p.accent,
                p.accent_muted,
            );
        } else {
            draw_surface_card(
                &mut rects,
                rect,
                theme::RADIUS_ELEMENT,
                p.bg_card,
                p.border_emphasized,
                Elevation::None,
                p.is_dark,
            );
        }
        let fg = if open {
            p.on_accent_muted
        } else {
            p.text_primary
        };
        texts.push(CanvasText::new(
            (l + r) * 0.5,
            (t + b) * 0.5 + theme::FONT_BODY * 0.35,
            theme::FONT_BODY,
            fg,
            TextAlign::Center,
            open,
            ch,
        ));
    };
    menu_tile(
        reader.view_menu_open,
        view_menu_button_rect(w, reader.win_h),
        "⋯",
    );
    menu_tile(
        reader.settings_menu_open,
        settings_menu_button_rect(w, reader.win_h),
        "☰",
    );

    // ---- CAMPO de búsqueda (input rehundido Astryx `RADIUS_ELEMENT`) ----
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;
    let field_r = w as f32 - pad;
    let (summary, has_filter) = search_summary(reader);

    if has_filter {
        draw_inset_ring(
            &mut rects,
            (pad, search_y, field_r, search_y + search_hh),
            theme::RADIUS_ELEMENT,
            p.accent,
            p.bg_card,
        );
    } else {
        draw_surface_card(
            &mut rects,
            (pad, search_y, field_r, search_y + search_hh),
            theme::RADIUS_ELEMENT,
            p.bg_body,
            p.border_emphasized,
            Elevation::None,
            p.is_dark,
        );
    }

    // Glifo discreto de búsqueda a la izquierda
    texts.push(CanvasText::new(
        pad + 18.0,
        search_y + search_hh * 0.65,
        theme::FONT_BODY,
        if has_filter {
            p.text_accent
        } else {
            p.text_secondary
        },
        TextAlign::Left,
        false,
        "⌕",
    ));

    if has_filter {
        texts.push(
            CanvasText::new(
                pad + 40.0,
                search_y + search_hh * 0.64,
                theme::FONT_BODY,
                p.text_primary,
                TextAlign::Left,
                true,
                truncate_name(&summary, 28),
            )
            .with_tracking(theme::TRACKING_TIGHT),
        );
        let xw = (search_hh - 10.0).max(22.0);
        let xx = field_r - 12.0 - xw;
        let xy = search_y + (search_hh - xw) * 0.5;
        draw_token_pill(
            &mut rects,
            &mut texts,
            (xx, xy, xx + xw, xy + xw),
            p.bg_muted,
            Some(p.border),
            p.text_secondary,
            theme::FONT_CAPTION,
            TextFont::Sans,
            0.0,
            true,
            "✕",
        );
    } else {
        texts.push(CanvasText::new(
            pad + 40.0,
            search_y + search_hh * 0.64,
            theme::FONT_BODY,
            p.text_secondary,
            TextAlign::Left,
            false,
            "Buscar por título o carpeta…",
        ));
    }

    // ---- CHIPS de colección / asignatura (tokens píldora Astryx `RADIUS_FULL`) ----
    for (label, rect, active, _) in lib_collection_chips(reader) {
        let (fill, border, fg) = if active {
            (p.accent_muted, Some(p.accent), p.on_accent_muted)
        } else {
            (p.bg_card, Some(p.border), p.text_secondary)
        };
        draw_token_pill(
            &mut rects,
            &mut texts,
            rect,
            fill,
            border,
            fg,
            theme::FONT_CAPTION,
            TextFont::Sans,
            0.0,
            active,
            &label,
        );
    }

    // ---- FRANJA de estado semántica (si la hay) ----
    if let Some(status) = reader.status.as_deref() {
        let row_h = picker_row_h(reader.win_h) as f32;
        let status_top = h_fixed as f32 - row_h;
        rects.push(CanvasRect::sharp(
            0.0,
            status_top,
            w as f32,
            h_fixed as f32,
            p.status_bg(),
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            status_top,
            4.0,
            h_fixed as f32,
            p.status_border(),
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            h_fixed as f32 - 1.0,
            w as f32,
            h_fixed as f32,
            p.status_border(),
        ));
        texts.push(CanvasText::new(
            pad,
            status_top + row_h * 0.62,
            theme::FONT_CAPTION,
            p.status_text(),
            TextAlign::Left,
            true,
            status,
        ));
    }

    // Menús dropdown abiertos: dibujar sobre la cabecera
    if reader.view_menu_open {
        draw_view_menu(reader, &mut rects, &mut texts);
    } else if reader.settings_menu_open {
        draw_settings_menu(reader, &mut rects, &mut texts);
    }

    jni_text_bitmap(w, h_total, theme::TRANSPARENT, &rects, &texts)
}

/// Render de la BANDA de contenido de la biblioteca (zona scrolleable sobre `bg_body`:
/// rejilla editorial de portadas, lista de tarjetas Astryx o empty state).
pub(crate) fn render_library_zone(
    reader: &Reader,
    band_origin: i32,
    band_h: i32,
) -> Option<Bitmap> {
    let w = reader.win_w;
    if w <= 0 || band_h <= 0 {
        return None;
    }
    let yof = -band_origin as f32;
    let p = reader.theme.palette();

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    if reader.library_list.is_empty() && !reader.library.lib_master {
        let content_y0 = lib_content_y0(reader);
        let shift = -(content_y0 as f32 + band_origin as f32);
        draw_empty_state(reader, &mut rects, &mut texts, shift);
    } else {
        let grid_y0 = lib_grid_y0(w, reader.win_h);
        if reader.is_grid() {
            let cols = reader.effective_grid_cols();
            let cell_h = grid_cell_h(w, cols, reader.cover_size);
            let cell_w = grid_cell_w(w, cols);
            let cover_w = grid_cover_w(w, cols, reader.cover_size);
            let cover_h = grid_cover_h(w, cols, reader.cover_size);
            let title_ts = theme::FONT_BODY;
            let char_w = title_ts * 0.55;
            let max_chars = (((cell_w - 2.0 * GRID_CELL_PAD) / char_w) as usize).max(3);
            let row_first = (((band_origin as f32 - grid_y0) / cell_h).floor().max(0.0)) as usize;
            let row_last = (((band_origin + band_h) as f32 - grid_y0) / cell_h)
                .ceil()
                .max(0.0) as usize;
            for row in row_first..row_last {
                for col in 0..cols {
                    let Some(entry) = reader.grid_entry_at(row, col) else {
                        continue;
                    };
                    let (cx, cy_rel, _, _) =
                        grid_cell_rect(w, 0, row, col, cols, reader.cover_size);
                    let cy = yof + grid_y0 + cy_rel;
                    let pct =
                        persist::progress_for(&reader.library.lib_books, &reader.entry_path(entry))
                            .map(|bp| bp.pct())
                            .unwrap_or(0.0);

                    if !reader.hide_covers {
                        let cover_x0 = cx + (cell_w - cover_w) / 2.0;
                        let cover_y0 = cy + 4.0;
                        let cover_rect =
                            (cover_x0, cover_y0, cover_x0 + cover_w, cover_y0 + cover_h);
                        draw_surface_card(
                            &mut rects,
                            cover_rect,
                            theme::RADIUS_CONTAINER,
                            p.bg_card,
                            p.border,
                            Elevation::Low,
                            p.is_dark,
                        );
                        if reader.thumbs.peek(&entry.uri).is_none() {
                            draw_cover_placeholder(
                                &mut rects,
                                &mut texts,
                                (cover_x0, cover_y0, cover_w, cover_h),
                                &entry_title(entry),
                                &p,
                                false,
                            );
                        }
                        if reader.cover_progress && pct > 0.0 {
                            draw_cover_progress_badge(
                                &mut rects, &mut texts, cover_rect, pct, &p, false,
                            );
                        }
                        // Título editorial bajo la portada
                        let text_y0 = cy + 4.0 + cover_h + 10.0;
                        texts.push(
                            CanvasText::new(
                                cx + GRID_CELL_PAD,
                                text_y0 + title_ts * 0.85,
                                title_ts,
                                p.text_primary,
                                TextAlign::Left,
                                true,
                                truncate_name(&entry_title(entry), max_chars),
                            )
                            .with_tracking(theme::TRACKING_TIGHT),
                        );
                        // Subtítulo (carpeta / asignatura)
                        let author_y0 = text_y0 + 20.0;
                        texts.push(CanvasText::new(
                            cx + GRID_CELL_PAD,
                            author_y0 + theme::FONT_CAPTION * 0.85,
                            theme::FONT_CAPTION,
                            p.text_secondary,
                            TextAlign::Left,
                            false,
                            truncate_name(&reader.library_caption(entry), max_chars),
                        ));
                        // Barra de progreso Astryx
                        let bar_y = author_y0 + 20.0;
                        let track_w = cell_w - 2.0 * GRID_CELL_PAD;
                        rects.push(CanvasRect::rounded(
                            cx + GRID_CELL_PAD,
                            bar_y,
                            cx + GRID_CELL_PAD + track_w,
                            bar_y + 4.0,
                            2.0,
                            p.track,
                        ));
                        if pct > 0.0 {
                            let fill_w = (track_w * pct).clamp(4.0, track_w);
                            rects.push(CanvasRect::rounded(
                                cx + GRID_CELL_PAD,
                                bar_y,
                                cx + GRID_CELL_PAD + fill_w,
                                bar_y + 4.0,
                                2.0,
                                p.accent,
                            ));
                        }
                    } else {
                        // Modo Rejilla sin portadas: tarjeta compacta Astryx con indicador lateral
                        draw_surface_card(
                            &mut rects,
                            (cx, cy, cx + cell_w, cy + 100.0),
                            theme::RADIUS_CONTAINER,
                            p.bg_card,
                            p.border,
                            Elevation::Low,
                            p.is_dark,
                        );
                        rects.push(CanvasRect::rounded(
                            cx + 1.0,
                            cy + 14.0,
                            cx + 4.5,
                            cy + 86.0,
                            1.75,
                            if pct > 0.0 {
                                p.accent
                            } else {
                                p.border_emphasized
                            },
                        ));

                        texts.push(
                            CanvasText::new(
                                cx + 16.0,
                                cy + 28.0,
                                title_ts,
                                p.text_primary,
                                TextAlign::Left,
                                true,
                                truncate_name(&entry_title(entry), max_chars),
                            )
                            .with_tracking(theme::TRACKING_TIGHT),
                        );
                        texts.push(CanvasText::new(
                            cx + 16.0,
                            cy + 52.0,
                            theme::FONT_CAPTION,
                            p.text_secondary,
                            TextAlign::Left,
                            false,
                            truncate_name(&reader.library_caption(entry), max_chars),
                        ));
                        let bar_y = cy + 72.0;
                        let track_w = cell_w - 32.0;
                        rects.push(CanvasRect::rounded(
                            cx + 16.0,
                            bar_y,
                            cx + 16.0 + track_w,
                            bar_y + 4.0,
                            2.0,
                            p.track,
                        ));
                        if pct > 0.0 {
                            let fill_w = (track_w * pct).clamp(4.0, track_w);
                            rects.push(CanvasRect::rounded(
                                cx + 16.0,
                                bar_y,
                                cx + 16.0 + fill_w,
                                bar_y + 4.0,
                                2.0,
                                p.accent,
                            ));
                        }
                    }
                }
            }
        } else {
            // MODO LISTA: tarjetas horizontales Astryx sobre `bg_body`
            let row_h = list_row_h(reader.win_h, reader.cover_size);
            let row_gap = list_row_gap();
            let total_row_h = row_h + row_gap;
            let idx_first = (((band_origin as f32 - grid_y0) / total_row_h)
                .floor()
                .max(0.0)) as usize;
            let idx_last = (((band_origin + band_h) as f32 - grid_y0) / total_row_h)
                .ceil()
                .max(0.0) as usize;
            for i in idx_first..idx_last {
                let Some(entry) = reader.list_entry_at(i) else {
                    continue;
                };
                let (rx, ry_rel, rx2, _ry2) =
                    list_row_rect(w, 0, i, reader.win_h, reader.cover_size);
                let ry = yof + grid_y0 + ry_rel;

                draw_surface_card(
                    &mut rects,
                    (rx, ry, rx2, ry + row_h),
                    theme::RADIUS_CONTAINER,
                    p.bg_card,
                    p.border,
                    Elevation::Low,
                    p.is_dark,
                );

                let pct =
                    persist::progress_for(&reader.library.lib_books, &reader.entry_path(entry))
                        .map(|bp| bp.pct())
                        .unwrap_or(0.0);

                let text_x = if !reader.hide_covers {
                    let cw = (60.0 * cover_size_multiplier(reader.cover_size)).round();
                    let ch = (90.0 * cover_size_multiplier(reader.cover_size)).round();
                    let cx = rx + 12.0;
                    let cy = ry + (row_h - ch) / 2.0;
                    let cover_rect = (cx, cy, cx + cw, cy + ch);

                    draw_surface_card(
                        &mut rects,
                        cover_rect,
                        theme::RADIUS_ELEMENT,
                        p.bg_surface,
                        p.border,
                        Elevation::None,
                        p.is_dark,
                    );

                    if reader.thumbs.peek(&entry.uri).is_none() {
                        draw_cover_placeholder(
                            &mut rects,
                            &mut texts,
                            (cx, cy, cw, ch),
                            &entry_title(entry),
                            &p,
                            true,
                        );
                    }
                    if reader.cover_progress && pct > 0.0 {
                        draw_cover_progress_badge(
                            &mut rects, &mut texts, cover_rect, pct, &p, true,
                        );
                    }
                    cx + cw + 18.0
                } else {
                    rx + 20.0
                };

                let title_ts = theme::FONT_TITLE;
                let char_w = title_ts * 0.55;
                let max_chars = (((rx2 - text_x - 120.0) / char_w) as usize).max(8);

                texts.push(
                    CanvasText::new(
                        text_x,
                        ry + 34.0,
                        title_ts,
                        p.text_primary,
                        TextAlign::Left,
                        true,
                        truncate_name(&entry_title(entry), max_chars),
                    )
                    .with_tracking(theme::TRACKING_TIGHT),
                );

                texts.push(CanvasText::new(
                    text_x,
                    ry + 58.0,
                    theme::FONT_CAPTION,
                    p.text_secondary,
                    TextAlign::Left,
                    false,
                    truncate_name(&reader.library_caption(entry), max_chars),
                ));

                let bar_y = ry + 82.0;
                let bar_w = rx2 - text_x - 94.0;
                rects.push(CanvasRect::rounded(
                    text_x,
                    bar_y,
                    text_x + bar_w,
                    bar_y + 4.0,
                    2.0,
                    p.track,
                ));
                if pct > 0.0 {
                    let fill_w = (bar_w * pct).clamp(4.0, bar_w);
                    rects.push(CanvasRect::rounded(
                        text_x,
                        bar_y,
                        text_x + fill_w,
                        bar_y + 4.0,
                        2.0,
                        p.accent,
                    ));
                }
                let pct_text = format!("{:.0}%", pct * 100.0);
                texts.push(
                    CanvasText::new(
                        rx2 - 20.0,
                        bar_y + 4.0,
                        theme::FONT_CAPTION,
                        if pct > 0.0 {
                            p.text_accent
                        } else {
                            p.text_secondary
                        },
                        TextAlign::Right,
                        true,
                        pct_text,
                    )
                    .with_font(TextFont::Mono),
                );
            }
        }

        // Sin resultados con filtro activo: callout Astryx centrado
        if reader.library.lib_filtered.is_empty()
            && (!reader.library.lib_query.is_empty()
                || reader.library.lib_letter.is_some()
                || reader.library.lib_folder.is_some()
                || reader.library.lib_status.is_some()
                || reader.library.lib_master)
        {
            let pad = grid_pad(w);
            let box_l = pad.max((w as f32 - 520.0) * 0.5);
            let box_r = w as f32 - box_l;
            let box_t = yof + 18.0;
            let box_b = box_t + 64.0;
            draw_surface_card(
                &mut rects,
                (box_l, box_t, box_r, box_b),
                theme::RADIUS_CONTAINER,
                p.bg_card,
                p.border,
                Elevation::None,
                p.is_dark,
            );
            texts.push(CanvasText::new(
                w as f32 * 0.5,
                box_t + 38.0,
                theme::FONT_BODY,
                p.text_secondary,
                TextAlign::Center,
                false,
                if !reader.library.lib_query.is_empty() {
                    "Sin resultados. Borra la búsqueda con ✕."
                } else if reader.library.lib_subject.is_some() {
                    "Sin documentos en esta asignatura. Elige «Todas»."
                } else if reader.library.lib_master {
                    "Aún no hay PDFs del máster. Sincroniza la carpeta con FolderSync."
                } else {
                    "Sin resultados con los filtros actuales."
                },
            ));
        }
    }

    jni_text_bitmap(w, band_h, p.bg_body, &rects, &texts)
}

/// Render de la fila HORIZONTAL de chips del panel de BÚSQUEDA `row`.
pub(crate) fn render_search_chip_row(reader: &Reader, row: usize) -> Option<Bitmap> {
    let chips = lib_chips(reader, row);
    if chips.is_empty() {
        return None;
    }
    let scroll = if row == 0 {
        reader.library.lib_letters_x
    } else {
        reader.library.lib_folders_x
    };
    let row_w = chips
        .iter()
        .map(|(_, (_, _, r, _), _)| r + scroll)
        .fold(grid_pad(reader.win_w), f32::max)
        .ceil() as i32
        + grid_pad(reader.win_w) as i32;
    let row_h = lib_chip_h(reader.win_h).ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }
    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (label, (l, t, r, b), active) in &chips {
        let (gl, gr) = (l + scroll, r + scroll);
        let (fill, border, fg) = if *active {
            (p.accent_muted, Some(p.accent), p.on_accent_muted)
        } else {
            (p.bg_card, Some(p.border), p.text_secondary)
        };
        draw_token_pill(
            &mut rects,
            &mut texts,
            (gl, 0.0, gr, b - t),
            fill,
            border,
            fg,
            theme::FONT_CAPTION,
            TextFont::Sans,
            0.0,
            *active,
            label,
        );
    }
    jni_text_bitmap(row_w, row_h, p.bg_body, &rects, &texts)
}

/// Copia la fila `row` (bitmap) sobre `dst` (zona fija o banda) con su
/// esquina superior izquierda en `(sx, sy)` px, recortada a los bordes de
/// `dst` (mismo contrato que `copy_region`). El scroll horizontal de la fila
/// se aplica como `sx = -scroll_x`.
pub(crate) fn splice_row(dst: &mut Bitmap, row: &Bitmap, sx: i32, sy: i32) {
    if dst.width == 0 || dst.height == 0 || row.width == 0 || row.height == 0 {
        return;
    }
    copy_region(
        dst.data.as_mut_ptr(),
        dst.width as usize,
        dst.height as usize,
        dst.width as usize,
        4,
        row,
        sx,
        sy,
    );
}

/// Blit de la biblioteca: fondo, cabecera fija y banda scrolleable.
pub(crate) fn blit_library(
    window: &NativeWindow,
    bg_rgba: [u8; 4],
    header: Option<&Bitmap>,
    band: Option<(&Bitmap, i32)>,
    scroll: i32,
    content_y0: i32,
    toast: Option<(&Bitmap, i32, i32)>,
) {
    let Ok(mut guard) = window.lock(None) else {
        warn!("ANativeWindow_lock failed");
        return;
    };
    let bpp = match guard.format().bytes_per_pixel() {
        Some(b) => b,
        None => {
            warn!(
                "buffer format without bytes_per_pixel: {:?}",
                guard.format()
            );
            return;
        }
    };
    let dst_w = guard.width();
    let dst_h = guard.height();
    let dst_stride = guard.stride(); // en píxeles
    let dst = guard.bits() as *mut u8;

    // Fondo del buffer según el tema activo
    fill_buffer(dst, dst_w, dst_h, dst_stride, bpp, bg_rgba);

    if let Some((band, origin)) = band {
        let sy = content_y0 - (scroll - origin);
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, band, 0, sy);
    }
    if let Some(h) = header {
        copy_region_blend(dst, dst_w, dst_h, dst_stride, bpp, h, 0, 0);
    }
    if let Some((t, tx, ty)) = toast {
        copy_region_blend(dst, dst_w, dst_h, dst_stride, bpp, t, tx, ty);
    }
}

/// Pega las portadas CACHEADAS de la rejilla o lista sobre la banda
pub(crate) fn paste_lib_thumbs(reader: &Reader, band: &mut Bitmap, band_origin: i32) {
    let w = reader.win_w;
    if w <= 0 || band.width == 0 || band.height == 0 || reader.hide_covers {
        return;
    }
    let grid_y0 = lib_grid_y0(w, reader.win_h);

    if reader.is_grid() {
        let cols = reader.effective_grid_cols();
        let cell_w = grid_cell_w(w, cols);
        let cell_h = grid_cell_h(w, cols, reader.cover_size);
        let cover_w = grid_cover_w(w, cols, reader.cover_size);
        let cover_h = grid_cover_h(w, cols, reader.cover_size);
        let row_first = (((band_origin as f32 - grid_y0) / cell_h).floor().max(0.0)) as usize;
        let row_last = (((band_origin + band.height as i32) as f32 - grid_y0) / cell_h)
            .ceil()
            .max(0.0) as usize;
        for row in row_first..row_last {
            for col in 0..cols {
                let Some(entry) = reader.grid_entry_at(row, col) else {
                    continue;
                };
                let Some(thumb) = reader.thumbs.peek(&entry.uri) else {
                    continue;
                };
                let (cx, cy_rel, _, _) = grid_cell_rect(w, 0, row, col, cols, reader.cover_size);
                let cover_x0 = (cx + (cell_w - cover_w) / 2.0).round() as i32;
                let cover_y0 = (grid_y0 + cy_rel - band_origin as f32 + 4.0).round() as i32;
                paste_thumb(
                    &mut band.data,
                    band.width as usize,
                    thumb,
                    cover_x0,
                    cover_y0,
                    cover_w as i32,
                    cover_h as i32,
                    reader.cover_fit,
                );
            }
        }
    } else {
        let row_h = list_row_h(reader.win_h, reader.cover_size);
        let row_gap = list_row_gap();
        let total_row_h = row_h + row_gap;
        let idx_first = (((band_origin as f32 - grid_y0) / total_row_h)
            .floor()
            .max(0.0)) as usize;
        let idx_last = (((band_origin + band.height as i32) as f32 - grid_y0) / total_row_h)
            .ceil()
            .max(0.0) as usize;
        let cw = (60.0 * cover_size_multiplier(reader.cover_size)).round() as i32;
        let ch = (90.0 * cover_size_multiplier(reader.cover_size)).round() as i32;
        for i in idx_first..idx_last {
            let Some(entry) = reader.list_entry_at(i) else {
                continue;
            };
            let Some(thumb) = reader.thumbs.peek(&entry.uri) else {
                continue;
            };
            let (rx, ry_rel, _, _) = list_row_rect(w, 0, i, reader.win_h, reader.cover_size);
            let cover_x0 = (rx + 12.0).round() as i32;
            let cover_y0 =
                (grid_y0 + ry_rel - band_origin as f32 + (row_h - ch as f32) / 2.0).round() as i32;
            paste_thumb(
                &mut band.data,
                band.width as usize,
                thumb,
                cover_x0,
                cover_y0,
                cw,
                ch,
                reader.cover_fit,
            );
        }
    }

    if reader.cover_progress {
        let p = reader.theme.palette();
        let mut badge_rects: Vec<CanvasRect> = Vec::new();
        let mut badge_texts: Vec<CanvasText> = Vec::new();

        if reader.is_grid() {
            let cols = reader.effective_grid_cols();
            let cell_w = grid_cell_w(w, cols);
            let cell_h = grid_cell_h(w, cols, reader.cover_size);
            let cover_w = grid_cover_w(w, cols, reader.cover_size);
            let cover_h = grid_cover_h(w, cols, reader.cover_size);
            let row_first = (((band_origin as f32 - grid_y0) / cell_h).floor().max(0.0)) as usize;
            let row_last = (((band_origin + band.height as i32) as f32 - grid_y0) / cell_h)
                .ceil()
                .max(0.0) as usize;
            for row in row_first..row_last {
                for col in 0..cols {
                    let Some(entry) = reader.grid_entry_at(row, col) else {
                        continue;
                    };
                    let pct =
                        persist::progress_for(&reader.library.lib_books, &reader.entry_path(entry))
                            .map(|bp| bp.pct())
                            .unwrap_or(0.0);
                    if pct > 0.0 {
                        let (cx, cy_rel, _, _) =
                            grid_cell_rect(w, 0, row, col, cols, reader.cover_size);
                        let cover_x0 = cx + (cell_w - cover_w) / 2.0;
                        let cover_y0 = grid_y0 + cy_rel - band_origin as f32 + 4.0;
                        draw_cover_progress_badge(
                            &mut badge_rects,
                            &mut badge_texts,
                            (cover_x0, cover_y0, cover_x0 + cover_w, cover_y0 + cover_h),
                            pct,
                            &p,
                            false,
                        );
                    }
                }
            }
        } else {
            let row_h = list_row_h(reader.win_h, reader.cover_size);
            let row_gap = list_row_gap();
            let total_row_h = row_h + row_gap;
            let idx_first = (((band_origin as f32 - grid_y0) / total_row_h)
                .floor()
                .max(0.0)) as usize;
            let idx_last = (((band_origin + band.height as i32) as f32 - grid_y0) / total_row_h)
                .ceil()
                .max(0.0) as usize;
            let cw = (60.0 * cover_size_multiplier(reader.cover_size)).round();
            let ch = (90.0 * cover_size_multiplier(reader.cover_size)).round();
            for i in idx_first..idx_last {
                let Some(entry) = reader.list_entry_at(i) else {
                    continue;
                };
                let pct =
                    persist::progress_for(&reader.library.lib_books, &reader.entry_path(entry))
                        .map(|bp| bp.pct())
                        .unwrap_or(0.0);
                if pct > 0.0 {
                    let (rx, ry_rel, _, _) =
                        list_row_rect(w, 0, i, reader.win_h, reader.cover_size);
                    let cx = rx + 12.0;
                    let cy = grid_y0 + ry_rel - band_origin as f32 + (row_h - ch) / 2.0;
                    draw_cover_progress_badge(
                        &mut badge_rects,
                        &mut badge_texts,
                        (cx, cy, cx + cw, cy + ch),
                        pct,
                        &p,
                        true,
                    );
                }
            }
        }

        if !badge_rects.is_empty()
            && let Some(overlay) = jni_text_bitmap(
                band.width as i32,
                band.height as i32,
                theme::TRANSPARENT,
                &badge_rects,
                &badge_texts,
            )
        {
            copy_region_blend(
                band.data.as_mut_ptr(),
                band.width as usize,
                band.height as usize,
                band.width as usize,
                4,
                &overlay,
                0,
                0,
            );
        }
    }
}

/// Resumen del filtro de BÚSQUEDA activo para el campo ("M" / "Download" /
/// "M · Download"): texto mostrable + si hay filtro (para el "✕").
fn search_summary(reader: &Reader) -> (String, bool) {
    if !reader.library.lib_query.is_empty() {
        return (reader.library.lib_query.clone(), true);
    }
    let mut parts = Vec::new();
    if let Some(l) = reader.library.lib_letter {
        parts.push(l.to_string());
    }
    if let Some(f) = &reader.library.lib_folder {
        parts.push(f.trim_end_matches('/').to_string());
    }
    if parts.is_empty() {
        (String::new(), false)
    } else {
        (parts.join(" · "), true)
    }
}

/// Dibuja el EMPTY STATE editorial de la biblioteca en lenguaje Astryx:
/// libro arquitectónico elevado con lomo, filete de acento y marcapáginas +
/// título display + subtítulo conciso + botón primario Astryx.
fn draw_empty_state(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    shift_y: f32,
) {
    let Some(g) = lib_empty_state_geom(reader) else {
        return;
    };
    let p = reader.theme.palette();
    let (bx, by, br, bb) = g.book;
    let book_rect = (bx, by + shift_y, br, bb + shift_y);

    // Lomo arquitectónico izquierdo
    draw_surface_card(
        rects,
        (bx - 10.0, by + 5.0 + shift_y, bx + 2.0, bb - 5.0 + shift_y),
        4.0,
        p.accent_muted,
        p.accent,
        Elevation::None,
        p.is_dark,
    );
    // Cubierta elevada `bg_card` + `Elevation::Med`
    draw_surface_card(
        rects,
        book_rect,
        theme::RADIUS_ELEMENT,
        p.bg_card,
        p.border_emphasized,
        Elevation::Med,
        p.is_dark,
    );
    // Cinta marcapáginas de acento en el borde superior derecho
    rects.push(CanvasRect::rounded(
        br - 20.0,
        by + shift_y,
        br - 12.0,
        by + 24.0 + shift_y,
        2.0,
        p.accent,
    ));
    // Filetes tipográficos interiores
    let line_w = (br - bx) * 0.56;
    rects.push(CanvasRect::rounded(
        bx + 14.0,
        by + 20.0 + shift_y,
        bx + 14.0 + line_w * 0.45,
        by + 23.0 + shift_y,
        1.5,
        p.accent,
    ));
    for i in 0..3 {
        let ly = by + 36.0 + i as f32 * 16.0 + shift_y;
        let w_mul = if i == 2 { 0.65 } else { 1.0 };
        rects.push(CanvasRect::rounded(
            bx + 14.0,
            ly,
            bx + 14.0 + line_w * w_mul,
            ly + 2.5,
            1.25,
            p.border_emphasized,
        ));
    }

    let (title, subtitle, btn_label) = if reader.permission_granted {
        (
            "Tu biblioteca está vacía",
            "Añade tu primer PDF para empezar a leer.",
            "Añadir PDF",
        )
    } else {
        (
            "Acceso necesario",
            "Concede permiso para leer tus PDFs.",
            "Conceder acceso",
        )
    };
    texts.push(
        CanvasText::new(
            reader.win_w as f32 * 0.5,
            g.title_y + shift_y,
            theme::FONT_DISPLAY,
            p.text_primary,
            TextAlign::Center,
            true,
            title,
        )
        .with_tracking(theme::TRACKING_TIGHT),
    );
    texts.push(CanvasText::new(
        reader.win_w as f32 * 0.5,
        g.subtitle_y + shift_y,
        theme::FONT_BODY,
        p.text_secondary,
        TextAlign::Center,
        false,
        subtitle,
    ));
    let (l, t, r, b) = g.button;
    draw_astryx_button(
        rects,
        texts,
        (l, t + shift_y, r, b + shift_y),
        ButtonVariant::Primary,
        &p,
        theme::FONT_BODY,
        true,
        btn_label,
    );
}

/// Pega la portada escalada según el modo `fit` (Crop vs Fit) dentro del bitmap base.
#[allow(clippy::too_many_arguments)]
fn paste_thumb(
    dst: &mut [u8],
    dst_w: usize,
    thumb: &Bitmap,
    dx: i32,
    dy: i32,
    target_w: i32,
    target_h: i32,
    fit: LibraryCoverFit,
) {
    let dst_h = dst.len() / (dst_w * 4);
    if dst_w == 0 || dst_h == 0 || target_w <= 0 || target_h <= 0 {
        return;
    }
    let src_w = thumb.width as i64;
    let src_h = thumb.height as i64;
    if src_w <= 0 || src_h <= 0 {
        return;
    }

    match fit {
        LibraryCoverFit::Crop => {
            let scale = (target_w as f64 / src_w as f64).max(target_h as f64 / src_h as f64);
            let dw = (src_w as f64 * scale).max(1.0);
            let dh = (src_h as f64 * scale).max(1.0);
            let off_x = ((dw - target_w as f64) / 2.0).round();
            let off_y = ((dh - target_h as f64) / 2.0).round();

            let src_stride = src_w as usize * 4;
            for ty in 0..target_h {
                let py = dy + ty;
                if py < 0 || py as usize >= dst_h {
                    continue;
                }
                let sy = (((ty as f64 + off_y) + 0.5) / scale - 0.5).clamp(0.0, (src_h - 1) as f64);
                let y0 = sy.floor() as usize;
                let y1 = (y0 + 1).min(src_h as usize - 1);
                let fy = (sy - y0 as f64) as f32;
                let row0 = &thumb.data[y0 * src_stride..];
                let row1 = &thumb.data[y1 * src_stride..];

                for tx in 0..target_w {
                    let px = dx + tx;
                    if px < 0 || px as usize >= dst_w {
                        continue;
                    }
                    let sx =
                        (((tx as f64 + off_x) + 0.5) / scale - 0.5).clamp(0.0, (src_w - 1) as f64);
                    let x0 = sx.floor() as usize;
                    let x1 = (x0 + 1).min(src_w as usize - 1);
                    let fx = (sx - x0 as f64) as f32;

                    let o = (py as usize * dst_w + px as usize) * 4;
                    for c in 0..3usize {
                        let p00 = row0[x0 * 4 + c] as f32;
                        let p10 = row0[x1 * 4 + c] as f32;
                        let p01 = row1[x0 * 4 + c] as f32;
                        let p11 = row1[x1 * 4 + c] as f32;
                        let top = p00 + (p10 - p00) * fx;
                        let bottom = p01 + (p11 - p01) * fx;
                        let val = top + (bottom - top) * fy;
                        dst[o + c] = val.clamp(0.0, 255.0).round() as u8;
                    }
                    dst[o + 3] = 0xFF;
                }
            }
        }
        LibraryCoverFit::Fit => {
            let pad = 6.0f64;
            let avail_w = (target_w as f64 - 2.0 * pad).max(10.0);
            let avail_h = (target_h as f64 - 2.0 * pad).max(10.0);
            let scale = (avail_w / src_w as f64).min(avail_h / src_h as f64);
            let dw = (src_w as f64 * scale).max(1.0);
            let dh = (src_h as f64 * scale).max(1.0);
            let page_x0 = ((target_w as f64 - dw) / 2.0).round() as i32;
            let page_y0 = ((target_h as f64 - dh) / 2.0).round() as i32;
            let page_x1 = page_x0 + dw as i32;
            let page_y1 = page_y0 + dh as i32;

            let src_stride = src_w as usize * 4;
            for ty in page_y0..page_y1 {
                let py = dy + ty;
                if py < 0 || py as usize >= dst_h {
                    continue;
                }
                let sy =
                    (((ty - page_y0) as f64 + 0.5) / scale - 0.5).clamp(0.0, (src_h - 1) as f64);
                let y0 = sy.floor() as usize;
                let y1 = (y0 + 1).min(src_h as usize - 1);
                let fy = (sy - y0 as f64) as f32;
                let row0 = &thumb.data[y0 * src_stride..];
                let row1 = &thumb.data[y1 * src_stride..];

                for tx in page_x0..page_x1 {
                    let px = dx + tx;
                    if px < 0 || px as usize >= dst_w {
                        continue;
                    }

                    let sx = (((tx - page_x0) as f64 + 0.5) / scale - 0.5)
                        .clamp(0.0, (src_w - 1) as f64);
                    let x0 = sx.floor() as usize;
                    let x1 = (x0 + 1).min(src_w as usize - 1);
                    let fx = (sx - x0 as f64) as f32;

                    let is_edge =
                        tx == page_x0 || tx == page_x1 - 1 || ty == page_y0 || ty == page_y1 - 1;
                    let spine_mul = if tx - page_x0 < 6 {
                        1.0 - (0.24 * (1.0 - (tx - page_x0) as f32 / 6.0))
                    } else {
                        1.0
                    };

                    let o = (py as usize * dst_w + px as usize) * 4;
                    for c in 0..3usize {
                        let p00 = row0[x0 * 4 + c] as f32;
                        let p10 = row0[x1 * 4 + c] as f32;
                        let p01 = row1[x0 * 4 + c] as f32;
                        let p11 = row1[x1 * 4 + c] as f32;
                        let top = p00 + (p10 - p00) * fx;
                        let bottom = p01 + (p11 - p01) * fx;
                        let mut val = (top + (bottom - top) * fy) * spine_mul;
                        if is_edge {
                            val *= 0.88;
                        }
                        dst[o + c] = val.clamp(0.0, 255.0).round() as u8;
                    }
                    dst[o + 3] = 0xFF;
                }
            }
        }
    }
}
