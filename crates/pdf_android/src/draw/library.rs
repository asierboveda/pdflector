// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pantalla de biblioteca: cabecera fija + zona scrolleable, filas de
//! chips/carousel, blit de la biblioteca y pegado de portadas cacheadas.

use crate::persist;
use crate::reader::{
    GRID_CELL_PAD, LibraryCoverFit, Reader, cover_size_multiplier, entry_author, entry_title,
    grid_cell_h, grid_cell_rect, grid_cell_w, grid_cover_h, grid_cover_w, grid_pad,
    header_menu_btn_d, lib_add_btn_w, lib_chip_h, lib_chips, lib_cont_card_h, lib_cont_card_w,
    lib_cont_card_x, lib_cont_cover_h, lib_cont_cover_w, lib_content_y0, lib_empty_state_geom,
    lib_grid_y0, lib_header_h, lib_org_chip_h, lib_org_chips, lib_search_h, list_row_gap,
    list_row_h, list_row_rect, picker_row_h, settings_menu_button_rect, title_from_name,
    truncate_name, view_menu_button_rect,
};
use crate::theme;
use android_activity::ndk::native_window::NativeWindow;
use log::warn;
use pdf_core::Bitmap;

use super::{
    CanvasRect, CanvasText, TextAlign, copy_region, draw_button, draw_card_shadow,
    draw_settings_menu, draw_view_menu, fill_buffer, jni_text_bitmap,
    primitives::copy_region_blend, settings_menu_geometry, view_menu_geometry,
};

/// Render de la ZONA FIJA de la biblioteca (cabecera editorial + campo de
/// búsqueda + franja de estado + overlay de menú si está abierto)
pub(crate) fn render_library_header(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h_fixed = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    );
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

    // Fondo del bloque fijo + hairline 1 px bajo la zona fija
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        h_fixed as f32,
        p.base_200,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        h_fixed as f32 - 1.0,
        w as f32,
        h_fixed as f32,
        p.base_300,
    ));

    // ---- CABECERA editorial: título "Biblioteca" + "＋ Añadir" ----
    let top_pad = 36.0f32; // margen seguro para la barra de estado de Android
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
    draw_button(
        &mut rects,
        &mut texts,
        btn_x,
        btn_y,
        btn_x + btn_w,
        btn_y + btn_h,
        p.base_100,
        p.base_300,
        p.base_content,
        theme::FONT_BODY,
        true,
        "＋ Añadir",
    );

    // ---- Botones de MENÚ: View "⋯" y Settings "☰" ----
    let d = header_menu_btn_d(w);
    let mut menu_glyph = |open: bool, l: f32, t: f32, r: f32, b: f32, ch: &str| {
        let (bg, border, fg) = if open {
            (p.primary, p.primary, p.primary_content)
        } else {
            (p.base_100, p.base_300, p.base_content)
        };
        rects.push(CanvasRect::rounded(l, t, r, b, d / 2.0, border));
        rects.push(CanvasRect::rounded(
            l + 1.0,
            t + 1.0,
            r - 1.0,
            b - 1.0,
            d / 2.0 - 1.0,
            bg,
        ));
        texts.push(CanvasText::new(
            (l + r) / 2.0,
            (t + b) / 2.0 + theme::FONT_BODY * 0.35,
            theme::FONT_BODY,
            fg,
            TextAlign::Center,
            false,
            ch.to_string(),
        ));
    };
    let (vl, vt, vr, vb) = view_menu_button_rect(w, reader.win_h);
    menu_glyph(reader.view_menu_open, vl, vt, vr, vb, "⋯");
    let (sl, st, sr, sb) = settings_menu_button_rect(w, reader.win_h);
    menu_glyph(reader.settings_menu_open, sl, st, sr, sb, "☰");

    // ---- CAMPO de búsqueda ----
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;
    let field_r = w as f32 - pad;
    rects.push(CanvasRect::rounded(
        pad,
        search_y,
        field_r,
        search_y + search_hh,
        search_hh / 2.0,
        p.base_300,
    ));
    rects.push(CanvasRect::rounded(
        pad + 1.0,
        search_y + 1.0,
        field_r - 1.0,
        search_y + search_hh - 1.0,
        (search_hh / 2.0 - 1.0).max(0.0),
        p.base_100,
    ));

    let (summary, has_filter) = search_summary(reader);
    if has_filter {
        texts.push(CanvasText::new(
            pad + 18.0,
            search_y + search_hh * 0.64,
            theme::FONT_BODY,
            p.base_content,
            TextAlign::Left,
            true,
            truncate_name(&summary, 28),
        ));
        let xw = search_hh - 8.0;
        let xx = field_r - 14.0 - xw;
        rects.push(CanvasRect::rounded(
            xx,
            search_y + 4.0,
            xx + xw,
            search_y + 4.0 + xw,
            xw / 2.0,
            p.base_300,
        ));
        texts.push(CanvasText::new(
            xx + xw / 2.0,
            search_y + 4.0 + xw * 0.64,
            theme::FONT_CAPTION,
            p.neutral_content,
            TextAlign::Center,
            false,
            "✕",
        ));
    } else {
        texts.push(CanvasText::new(
            pad + 18.0,
            search_y + search_hh * 0.64,
            theme::FONT_BODY,
            p.neutral_content,
            TextAlign::Left,
            false,
            "Buscar por título o carpeta...",
        ));
    }

    // ---- FRANJA de estado (si la hay) ----
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

    // Menús dropdown abiertos: dibujar el menú directamente sobre la cabecera
    if reader.view_menu_open {
        draw_view_menu(reader, &mut rects, &mut texts);
    } else if reader.settings_menu_open {
        draw_settings_menu(reader, &mut rects, &mut texts);
    }

    jni_text_bitmap(w, h_total, theme::TRANSPARENT, &rects, &texts)
}

/// Render de la BANDA de contenido de la biblioteca (la zona scrolleable:
/// Continue Reading + My Library + rejilla o lista o empty state)
pub(crate) fn render_library_zone(
    reader: &Reader,
    band_origin: i32,
    band_h: i32,
) -> Option<Bitmap> {
    let w = reader.win_w;
    if w <= 0 || band_h <= 0 {
        return None;
    }
    let yof = -band_origin as f32; // contenido − origen de banda
    let p = reader.theme.palette();
    let pad = grid_pad(w);

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    if reader.library_list.is_empty() {
        let content_y0 = lib_content_y0(
            reader.win_h,
            reader.library.lib_search_open,
            reader.status.is_some(),
        );
        let shift = -(content_y0 as f32 + band_origin as f32);
        draw_empty_state(reader, &mut rects, &mut texts, shift);
    } else {
        // Continue reading carousel (si está habilitado y hay libros)
        if reader.lib_has_cont() {
            let section_y = yof + 8.0;
            texts.push(CanvasText::new(
                pad,
                section_y + theme::FONT_TITLE * 0.85,
                theme::FONT_TITLE,
                p.base_content,
                TextAlign::Left,
                true,
                "Seguir leyendo".to_string(),
            ));
            let cont_y0 = section_y + theme::FONT_TITLE + 12.0;
            let books = reader.lib_continue_reading();
            for (i, book) in books.iter().enumerate() {
                let card_w = lib_cont_card_w(w, reader.win_h);
                let card_h = lib_cont_card_h(reader.win_h);
                let cx = lib_cont_card_x(w, reader.win_h, i) - reader.library.lib_carousel_x;
                if cx + card_w < 0.0 || cx > w as f32 {
                    continue;
                }
                let cy = cont_y0;
                let cw = lib_cont_cover_w(reader.win_h);
                let chh = lib_cont_cover_h(reader.win_h);
                let cover_r = 12.0f32;

                draw_card_shadow(
                    &mut rects,
                    cx,
                    cy,
                    cx + card_w,
                    cy + card_h,
                    16.0,
                    p.is_dark,
                );
                rects.push(CanvasRect::rounded(
                    cx,
                    cy,
                    cx + card_w,
                    cy + card_h,
                    16.0,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    cx + 1.0,
                    cy + 1.0,
                    cx + card_w - 1.0,
                    cy + card_h - 1.0,
                    15.0,
                    p.base_100,
                ));

                let cover_x = cx + 16.0;
                let cover_y = cy + 16.0;
                draw_card_shadow(
                    &mut rects,
                    cover_x,
                    cover_y,
                    cover_x + cw,
                    cover_y + chh,
                    cover_r,
                    p.is_dark,
                );
                rects.push(CanvasRect::rounded(
                    cover_x,
                    cover_y,
                    cover_x + cw,
                    cover_y + chh,
                    cover_r,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    cover_x + 1.0,
                    cover_y + 1.0,
                    cover_x + cw - 1.0,
                    cover_y + chh - 1.0,
                    (cover_r - 1.0).max(0.0),
                    p.base_200,
                ));

                let tx = cover_x + cw + 16.0;
                let title_ts = theme::FONT_TITLE;
                texts.push(CanvasText::new(
                    tx,
                    cy + 34.0,
                    title_ts,
                    p.base_content,
                    TextAlign::Left,
                    true,
                    truncate_name(&book.name, 18),
                ));
                texts.push(CanvasText::new(
                    tx,
                    cy + 60.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    truncate_name(&book.author, 18),
                ));
                let bar_w = card_w - (tx - cx) - 20.0;
                let bar_y = cy + 86.0;
                rects.push(CanvasRect::rounded(
                    tx,
                    bar_y,
                    tx + bar_w,
                    bar_y + 4.0,
                    2.0,
                    p.base_300,
                ));
                if book.pct > 0.0 {
                    rects.push(CanvasRect::rounded(
                        tx,
                        bar_y,
                        tx + (bar_w * book.pct).clamp(4.0, bar_w),
                        bar_y + 4.0,
                        2.0,
                        p.primary,
                    ));
                }
                let page_info = format!(
                    "Pág. {} de {} · {:.0}%",
                    book.page + 1,
                    book.page_count.max(1),
                    book.pct * 100.0
                );
                texts.push(CanvasText::new(
                    tx,
                    bar_y + 20.0,
                    theme::FONT_CAPTION * 0.9,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    page_info,
                ));
            }
        }

        let grid_y0 = lib_grid_y0(w, reader.win_h, reader.lib_has_cont());
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
                    if !reader.hide_covers {
                        let cover_x0 = cx + (cell_w - cover_w) / 2.0;
                        let cover_y0 = cy + 4.0;
                        let cover_r = 12.0f32;
                        // Sombra visible multicapa detrás del marco 2:3
                        draw_card_shadow(
                            &mut rects,
                            cover_x0,
                            cover_y0,
                            cover_x0 + cover_w,
                            cover_y0 + cover_h,
                            cover_r,
                            p.is_dark,
                        );
                        // Marco 2:3 con fondo base-200 y borde 1px base-300
                        rects.push(CanvasRect::rounded(
                            cover_x0,
                            cover_y0,
                            cover_x0 + cover_w,
                            cover_y0 + cover_h,
                            cover_r,
                            p.base_300,
                        ));
                        rects.push(CanvasRect::rounded(
                            cover_x0 + 1.0,
                            cover_y0 + 1.0,
                            cover_x0 + cover_w - 1.0,
                            cover_y0 + cover_h - 1.0,
                            (cover_r - 1.0).max(0.0),
                            p.base_200,
                        ));
                        if reader.thumbs.peek(&entry.uri).is_none() {
                            texts.push(CanvasText::new(
                                cover_x0 + cover_w / 2.0,
                                cover_y0 + cover_h / 2.0 + title_ts * 0.35,
                                title_ts,
                                p.neutral_content,
                                TextAlign::Center,
                                true,
                                truncate_name(&entry_title(entry), 12),
                            ));
                        }
                        // Badge de progreso sobre la portada (SHOW PROGRESS)
                        let pct = persist::progress_for(
                            &reader.library.lib_books,
                            &reader.entry_path(entry),
                        )
                        .map(|bp| bp.pct())
                        .unwrap_or(0.0);
                        if reader.cover_progress && pct > 0.0 {
                            let pct_val = (pct * 100.0).round() as u32;
                            if pct_val > 0 {
                                let pct_str = format!("{pct_val}%");
                                let badge_w = 42.0f32;
                                let badge_h = 22.0f32;
                                let badge_r = cover_x0 + cover_w - 6.0;
                                let badge_b = cover_y0 + cover_h - 6.0;
                                let badge_l = badge_r - badge_w;
                                let badge_t = badge_b - badge_h;
                                rects.push(CanvasRect::rounded(
                                    badge_l, badge_t, badge_r, badge_b, 999.0, p.primary,
                                ));
                                texts.push(CanvasText::new(
                                    (badge_l + badge_r) / 2.0,
                                    badge_t + badge_h * 0.68,
                                    theme::FONT_CAPTION * 0.85,
                                    p.primary_content,
                                    TextAlign::Center,
                                    true,
                                    pct_str,
                                ));
                            }
                        }
                        // Título bajo el marco
                        let text_y0 = cy + 4.0 + cover_h + 10.0;
                        texts.push(CanvasText::new(
                            cx + GRID_CELL_PAD,
                            text_y0 + title_ts * 0.85,
                            title_ts,
                            p.base_content,
                            TextAlign::Left,
                            true,
                            truncate_name(&entry_title(entry), max_chars),
                        ));
                        // Autor
                        let author_y0 = text_y0 + 20.0;
                        texts.push(CanvasText::new(
                            cx + GRID_CELL_PAD,
                            author_y0 + theme::FONT_CAPTION * 0.85,
                            theme::FONT_CAPTION,
                            p.neutral_content,
                            TextAlign::Left,
                            false,
                            truncate_name(&entry_author(entry), max_chars),
                        ));
                        // Barra de progreso
                        let bar_y = author_y0 + 20.0;
                        let track_w = cell_w - 2.0 * GRID_CELL_PAD;
                        rects.push(CanvasRect::rounded(
                            cx + GRID_CELL_PAD,
                            bar_y,
                            cx + GRID_CELL_PAD + track_w,
                            bar_y + 4.0,
                            2.0,
                            p.base_300,
                        ));
                        if pct > 0.0 {
                            let fill_w = (track_w * pct).clamp(4.0, track_w);
                            rects.push(CanvasRect::rounded(
                                cx + GRID_CELL_PAD,
                                bar_y,
                                cx + GRID_CELL_PAD + fill_w,
                                bar_y + 4.0,
                                2.0,
                                p.primary,
                            ));
                        }
                    } else {
                        // Modo Rejilla con hide_covers
                        let card_r = 12.0f32;
                        draw_card_shadow(
                            &mut rects,
                            cx,
                            cy,
                            cx + cell_w,
                            cy + 100.0,
                            card_r,
                            p.is_dark,
                        );
                        rects.push(CanvasRect::rounded(
                            cx,
                            cy,
                            cx + cell_w,
                            cy + 100.0,
                            card_r,
                            p.base_300,
                        ));
                        rects.push(CanvasRect::rounded(
                            cx + 1.0,
                            cy + 1.0,
                            cx + cell_w - 1.0,
                            cy + 99.0,
                            card_r - 1.0,
                            p.base_100,
                        ));

                        texts.push(CanvasText::new(
                            cx + 12.0,
                            cy + 28.0,
                            title_ts,
                            p.base_content,
                            TextAlign::Left,
                            true,
                            truncate_name(&entry_title(entry), max_chars),
                        ));
                        texts.push(CanvasText::new(
                            cx + 12.0,
                            cy + 52.0,
                            theme::FONT_CAPTION,
                            p.neutral_content,
                            TextAlign::Left,
                            false,
                            truncate_name(&entry_author(entry), max_chars),
                        ));
                        let bar_y = cy + 72.0;
                        let track_w = cell_w - 24.0;
                        let pct = persist::progress_for(
                            &reader.library.lib_books,
                            &reader.entry_path(entry),
                        )
                        .map(|bp| bp.pct())
                        .unwrap_or(0.0);
                        rects.push(CanvasRect::rounded(
                            cx + 12.0,
                            bar_y,
                            cx + 12.0 + track_w,
                            bar_y + 4.0,
                            2.0,
                            p.base_300,
                        ));
                        if pct > 0.0 {
                            let fill_w = (track_w * pct).clamp(4.0, track_w);
                            rects.push(CanvasRect::rounded(
                                cx + 12.0,
                                bar_y,
                                cx + 12.0 + fill_w,
                                bar_y + 4.0,
                                2.0,
                                p.primary,
                            ));
                        }
                    }
                }
            }
        } else {
            // MODO LISTA: 1 columna, fila de ancho total
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
                let card_r = 14.0f32;

                draw_card_shadow(&mut rects, rx, ry, rx2, ry + row_h, card_r, p.is_dark);
                rects.push(CanvasRect::rounded(
                    rx,
                    ry,
                    rx2,
                    ry + row_h,
                    card_r,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    rx + 1.0,
                    ry + 1.0,
                    rx2 - 1.0,
                    ry + row_h - 1.0,
                    card_r - 1.0,
                    p.base_100,
                ));

                let pct =
                    persist::progress_for(&reader.library.lib_books, &reader.entry_path(entry))
                        .map(|bp| bp.pct())
                        .unwrap_or(0.0);

                let text_x = if !reader.hide_covers {
                    let cw = (60.0 * cover_size_multiplier(reader.cover_size)).round();
                    let ch = (90.0 * cover_size_multiplier(reader.cover_size)).round();
                    let cx = rx + 12.0;
                    let cy = ry + (row_h - ch) / 2.0;
                    let cover_r = 8.0f32;

                    draw_card_shadow(&mut rects, cx, cy, cx + cw, cy + ch, cover_r, p.is_dark);
                    rects.push(CanvasRect::rounded(
                        cx,
                        cy,
                        cx + cw,
                        cy + ch,
                        cover_r,
                        p.base_300,
                    ));
                    rects.push(CanvasRect::rounded(
                        cx + 1.0,
                        cy + 1.0,
                        cx + cw - 1.0,
                        cy + ch - 1.0,
                        (cover_r - 1.0).max(0.0),
                        p.base_200,
                    ));

                    if reader.thumbs.peek(&entry.uri).is_none() {
                        texts.push(CanvasText::new(
                            cx + cw / 2.0,
                            cy + ch / 2.0 + theme::FONT_CAPTION * 0.35,
                            theme::FONT_CAPTION * 0.9,
                            p.neutral_content,
                            TextAlign::Center,
                            true,
                            truncate_name(&entry_title(entry), 8),
                        ));
                    }
                    if reader.cover_progress && pct > 0.0 {
                        let pct_val = (pct * 100.0).round() as u32;
                        if pct_val > 0 {
                            let pct_str = format!("{pct_val}%");
                            let badge_w = 38.0f32;
                            let badge_h = 20.0f32;
                            let badge_r = cx + cw - 4.0;
                            let badge_b = cy + ch - 4.0;
                            let badge_l = badge_r - badge_w;
                            let badge_t = badge_b - badge_h;
                            rects.push(CanvasRect::rounded(
                                badge_l, badge_t, badge_r, badge_b, 999.0, p.primary,
                            ));
                            texts.push(CanvasText::new(
                                (badge_l + badge_r) / 2.0,
                                badge_t + badge_h * 0.68,
                                theme::FONT_CAPTION * 0.80,
                                p.primary_content,
                                TextAlign::Center,
                                true,
                                pct_str,
                            ));
                        }
                    }
                    cx + cw + 18.0
                } else {
                    rx + 20.0
                };

                let title_ts = theme::FONT_TITLE;
                let char_w = title_ts * 0.55;
                let max_chars = (((rx2 - text_x - 120.0) / char_w) as usize).max(8);

                texts.push(CanvasText::new(
                    text_x,
                    ry + 32.0,
                    title_ts,
                    p.base_content,
                    TextAlign::Left,
                    true,
                    truncate_name(&entry_title(entry), max_chars),
                ));

                texts.push(CanvasText::new(
                    text_x,
                    ry + 58.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    truncate_name(&entry_author(entry), max_chars),
                ));

                let bar_y = ry + 82.0;
                let bar_w = rx2 - text_x - 90.0;
                rects.push(CanvasRect::rounded(
                    text_x,
                    bar_y,
                    text_x + bar_w,
                    bar_y + 4.0,
                    2.0,
                    p.base_300,
                ));
                if pct > 0.0 {
                    let fill_w = (bar_w * pct).clamp(4.0, bar_w);
                    rects.push(CanvasRect::rounded(
                        text_x,
                        bar_y,
                        text_x + fill_w,
                        bar_y + 4.0,
                        2.0,
                        p.primary,
                    ));
                }
                let pct_text = format!("{:.0}%", pct * 100.0);
                texts.push(CanvasText::new(
                    rx2 - 20.0,
                    bar_y + 4.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Right,
                    true,
                    pct_text,
                ));
            }
        }

        // 5. Sin resultados con filtro activo (buscador con teclado o
        // filtros legacy conservados).
        if reader.library.lib_filtered.is_empty()
            && (!reader.library.lib_query.is_empty()
                || reader.library.lib_letter.is_some()
                || reader.library.lib_folder.is_some()
                || reader.library.lib_status.is_some())
        {
            texts.push(CanvasText::new(
                w as f32 / 2.0,
                yof + 24.0,
                theme::FONT_BODY,
                p.neutral_content,
                TextAlign::Center,
                false,
                "Sin resultados — toca ✕ para limpiar",
            ));
        }
    }

    jni_text_bitmap(w, band_h, p.base_200, &rects, &texts)
}

/// Render de la fila horizontal del carousel de "Continue Reading". Desde la
/// biblioteca minimalista (2026-08-25: rejilla + buscador, sección oculta)
/// ya no se splices; se conserva por si se reintroduce.
#[allow(dead_code)] // sección "Continue Reading" oculta por diseño
pub(crate) fn render_carousel_row(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let books = reader.lib_continue_reading();
    let n = books.len();
    if n == 0 {
        return None;
    }
    let cw = lib_cont_cover_w(reader.win_h);
    let chh = lib_cont_cover_h(reader.win_h);
    let card_w = lib_cont_card_w(w, reader.win_h);
    let card_h = lib_cont_card_h(reader.win_h);
    let cover_r = 12.0f32;
    let row_w = (lib_cont_card_x(w, reader.win_h, n - 1) + card_w + grid_pad(w)).ceil() as i32;
    let row_h = card_h.ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }
    let p = reader.theme.palette();

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (i, book) in books.iter().enumerate() {
        let cx = lib_cont_card_x(w, reader.win_h, i);
        // Sombra visible de la tarjeta horizontal (B4, G1)
        draw_card_shadow(&mut rects, cx, 0.0, cx + card_w, card_h, 16.0, p.is_dark);
        // Tarjeta: borde 1px base_300 + fondo base_100 (radio 16 px, B1, B4)
        rects.push(CanvasRect::rounded(
            cx,
            0.0,
            cx + card_w,
            card_h,
            16.0,
            p.base_300,
        ));
        rects.push(CanvasRect::rounded(
            cx + 1.0,
            1.0,
            cx + card_w - 1.0,
            card_h - 1.0,
            15.0,
            p.base_100,
        ));
        // Portada 2:3 a la izquierda (B4)
        let cover_x = cx + 16.0;
        let cover_y = 16.0;
        draw_card_shadow(
            &mut rects,
            cover_x,
            cover_y,
            cover_x + cw,
            cover_y + chh,
            cover_r,
            p.is_dark,
        );
        rects.push(CanvasRect::rounded(
            cover_x,
            cover_y,
            cover_x + cw,
            cover_y + chh,
            cover_r,
            p.base_300,
        ));
        rects.push(CanvasRect::rounded(
            cover_x + 1.0,
            cover_y + 1.0,
            cover_x + cw - 1.0,
            cover_y + chh - 1.0,
            (cover_r - 1.0).max(0.0),
            p.base_200,
        ));
        if reader.thumbs.peek(&book.path).is_none() {
            texts.push(CanvasText::new(
                cover_x + cw / 2.0,
                cover_y + chh / 2.0 + 7.0,
                theme::FONT_BODY,
                p.neutral_content,
                TextAlign::Center,
                true,
                truncate_name(&title_from_name(&book.name), 12),
            ));
        }
        // Textos a la derecha de la portada (B4)
        let tx = cover_x + cw + 20.0;
        let tw = card_w - (cw + 52.0);
        let max_chars = ((tw / 9.0) as usize).max(8);
        // Título (17sp negrita base-content)
        texts.push(CanvasText::new(
            tx,
            cover_y + 22.0,
            theme::FONT_TITLE,
            p.base_content,
            TextAlign::Left,
            true,
            truncate_name(&title_from_name(&book.name), max_chars),
        ));
        // Autor / carpeta (12sp neutral-content)
        texts.push(CanvasText::new(
            tx,
            cover_y + 46.0,
            theme::FONT_CAPTION,
            p.neutral_content,
            TextAlign::Left,
            false,
            truncate_name(&book.author, max_chars),
        ));
        // Barra de progreso (track 4px base-300, fill primary) con separación >= 10px del autor (B1)
        let bar_y = cover_y + 70.0;
        rects.push(CanvasRect::rounded(
            tx,
            bar_y,
            tx + tw,
            bar_y + 4.0,
            2.0,
            p.base_300,
        ));
        let fill_w = (tw * book.pct).clamp(4.0, tw);
        if book.pct > 0.0 {
            rects.push(CanvasRect::rounded(
                tx,
                bar_y,
                tx + fill_w,
                bar_y + 4.0,
                2.0,
                p.primary,
            ));
        }
        // Meta: "Pág. X de Y · Z%"
        let meta = format!(
            "Pág. {} de {} · {:.0}%",
            book.page + 1,
            book.page_count,
            book.pct * 100.0
        );
        texts.push(CanvasText::new(
            tx,
            bar_y + 20.0,
            theme::FONT_CAPTION,
            p.neutral_content,
            TextAlign::Left,
            false,
            meta,
        ));
        // Botón "Continuar" como PÍLDORA RELLENA primary con texto contraste (B4)
        let btn_w = tw.clamp(100.0, 140.0);
        let btn_h = 40.0;
        let btn_y = (card_h - 16.0 - btn_h).max(bar_y + 34.0);
        draw_button(
            &mut rects,
            &mut texts,
            tx,
            btn_y,
            tx + btn_w,
            btn_y + btn_h,
            p.primary,
            p.primary,
            p.primary_content,
            theme::FONT_BODY,
            true,
            "Continuar",
        );
    }

    let mut out = jni_text_bitmap(row_w, row_h, p.base_200, &rects, &texts)?;
    for (i, book) in books.iter().enumerate() {
        let Some(thumb) = reader.thumbs.peek(&book.path) else {
            continue;
        };
        let cover_x = (lib_cont_card_x(w, reader.win_h, i) + 16.0).round() as i32;
        paste_thumb(
            &mut out.data,
            out.width as usize,
            thumb,
            cover_x,
            16,
            cw as i32,
            chh as i32,
            reader.cover_fit,
        );
    }
    Some(out)
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
        let (fill, border, tc) = if *active {
            (p.primary, p.primary, p.primary_content)
        } else {
            (p.base_100, p.base_300, p.base_content)
        };
        draw_button(
            &mut rects,
            &mut texts,
            gl,
            0.0,
            gr,
            b - t,
            fill,
            border,
            tc,
            theme::FONT_BODY,
            *active,
            label,
        );
    }
    jni_text_bitmap(row_w, row_h, p.base_200, &rects, &texts)
}

/// Render de la fila HORIZONTAL de chips de ORGANIZACIÓN `row` (0 = SORT, 1 = FILTER).
/// Render de una fila de chips de ORGANIZACIÓN (sort/filter) de la
/// biblioteca. Desde la biblioteca minimalista (2026-08-25) ya no se
/// splices; se conserva por si se reintroduce el bloque de organización.
#[allow(dead_code)] // organización (sort/filter) oculta por diseño
pub(crate) fn render_org_chip_row(reader: &Reader, row: usize) -> Option<Bitmap> {
    let chips = lib_org_chips(reader, row);
    if chips.is_empty() {
        return None;
    }
    let scroll = if row == 0 {
        reader.library.lib_sort_x
    } else {
        reader.library.lib_filter_x
    };
    let row_w = chips
        .iter()
        .map(|(_, (_, _, r, _), _)| r + scroll)
        .fold(grid_pad(reader.win_w), f32::max)
        .ceil() as i32
        + grid_pad(reader.win_w) as i32;
    let row_h = lib_org_chip_h(reader.win_h).ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }
    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (label, (l, t, r, b), active) in &chips {
        let (gl, gr) = (l + scroll, r + scroll);
        let (fill, border, tc) = if *active {
            (p.primary, p.primary, p.primary_content)
        } else {
            (p.base_100, p.base_300, p.base_content)
        };
        draw_button(
            &mut rects,
            &mut texts,
            gl,
            0.0,
            gr,
            (b - t).max(1.0),
            fill,
            border,
            tc,
            theme::FONT_BODY,
            *active,
            label,
        );
    }
    jni_text_bitmap(row_w, row_h, p.base_200, &rects, &texts)
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
    let has_cont = reader.lib_has_cont();
    let grid_y0 = lib_grid_y0(w, reader.win_h, has_cont);

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
                        let pct_val = (pct * 100.0).round() as u32;
                        if pct_val > 0 {
                            let (cx, cy_rel, _, _) =
                                grid_cell_rect(w, 0, row, col, cols, reader.cover_size);
                            let cover_x0 = cx + (cell_w - cover_w) / 2.0;
                            let cover_y0 = grid_y0 + cy_rel - band_origin as f32 + 4.0;
                            let pct_str = format!("{pct_val}%");
                            let badge_w = 42.0f32;
                            let badge_h = 22.0f32;
                            let badge_r = cover_x0 + cover_w - 6.0;
                            let badge_b = cover_y0 + cover_h - 6.0;
                            let badge_l = badge_r - badge_w;
                            let badge_t = badge_b - badge_h;
                            badge_rects.push(CanvasRect::rounded(
                                badge_l, badge_t, badge_r, badge_b, 999.0, p.primary,
                            ));
                            badge_texts.push(CanvasText::new(
                                (badge_l + badge_r) / 2.0,
                                badge_t + badge_h * 0.68,
                                theme::FONT_CAPTION * 0.85,
                                p.primary_content,
                                TextAlign::Center,
                                true,
                                pct_str,
                            ));
                        }
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
                    let pct_val = (pct * 100.0).round() as u32;
                    if pct_val > 0 {
                        let (rx, ry_rel, _, _) =
                            list_row_rect(w, 0, i, reader.win_h, reader.cover_size);
                        let cx = rx + 12.0;
                        let cy = grid_y0 + ry_rel - band_origin as f32 + (row_h - ch) / 2.0;
                        let pct_str = format!("{pct_val}%");
                        let badge_w = 38.0f32;
                        let badge_h = 20.0f32;
                        let badge_r = cx + cw - 4.0;
                        let badge_b = cy + ch - 4.0;
                        let badge_l = badge_r - badge_w;
                        let badge_t = badge_b - badge_h;
                        badge_rects.push(CanvasRect::rounded(
                            badge_l, badge_t, badge_r, badge_b, 999.0, p.primary,
                        ));
                        badge_texts.push(CanvasText::new(
                            (badge_l + badge_r) / 2.0,
                            badge_t + badge_h * 0.68,
                            theme::FONT_CAPTION * 0.80,
                            p.primary_content,
                            TextAlign::Center,
                            true,
                            pct_str,
                        ));
                    }
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
    // Buscador con TECLADO: el texto tecleado es el filtro principal.
    if !reader.library.lib_query.is_empty() {
        return (reader.library.lib_query.clone(), true);
    }
    // Filtros legacy por letra/carpeta (sin UI desde 2026-08-25).
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

/// Dibuja el EMPTY STATE de la biblioteca (sin PDFs): ilustración simple de
/// un libro + título + subtítulo + botón Readest ("Añadir PDF" o "Conceder permiso").
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
    // Portada + lomo + líneas de "texto"
    rects.push(CanvasRect::rounded(
        bx,
        by + shift_y,
        br,
        bb + shift_y,
        8.0,
        p.cover_placeholder(),
    ));
    rects.push(CanvasRect::sharp(
        bx - 8.0,
        by + 4.0 + shift_y,
        bx - 2.0,
        bb - 4.0 + shift_y,
        p.base_300,
    ));
    let line_w = (br - bx) * 0.6;
    for i in 0..3 {
        let ly = by + 18.0 + i as f32 * 18.0 + shift_y;
        rects.push(CanvasRect::sharp(
            bx + 12.0,
            ly,
            bx + 12.0 + line_w,
            ly + 2.0,
            p.base_300,
        ));
    }
    // Título + subtítulo + botón
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
    texts.push(CanvasText::new(
        reader.win_w as f32 / 2.0,
        g.title_y + shift_y,
        theme::FONT_DISPLAY,
        p.base_content,
        TextAlign::Center,
        true,
        title,
    ));
    texts.push(CanvasText::new(
        reader.win_w as f32 / 2.0,
        g.subtitle_y + shift_y,
        theme::FONT_BODY,
        p.neutral_content,
        TextAlign::Center,
        false,
        subtitle,
    ));
    let (l, t, r, b) = g.button;
    draw_button(
        rects,
        texts,
        l,
        t + shift_y,
        r,
        b + shift_y,
        p.primary,
        p.primary,
        p.primary_content,
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
