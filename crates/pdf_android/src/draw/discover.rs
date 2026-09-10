// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pantalla de Descubrir (arXiv Discover): cabecera fija (tabs + subtabs + buscador),
//! banda scrolleable con feed de papers, resultados de búsqueda, ficha de detalle
//! y selector de áreas temáticas.

use crate::reader::discover_categories::{ARXIV_CATEGORIES, category_label};
use crate::reader::discover_state::{DiscoverPhase, DiscoverScreen, DiscoverTab};
use crate::reader::{
    Reader, UiMode, disc_card_action_rect, disc_card_gap, disc_card_h, disc_card_pad,
    disc_card_rect, disc_cat_row_h, disc_cat_row_rect, disc_content_y0, disc_detail_back_rect,
    disc_detail_layout, disc_more_btn_rect, disc_search_rect, disc_subtabs_rect, grid_pad,
    lib_tabs_rect, wrap_text_chars,
};
use crate::theme;
use pdf_core::Bitmap;

use super::{
    ButtonRect, CanvasRect, CanvasText, TextAlign, draw_button, draw_card_shadow, jni_text_bitmap,
};

/// Texto de atribución requerido por las condiciones de interoperabilidad abierta de arXiv.
pub(crate) const ARXIV_ATTRIBUTION: &str =
    "Thank you to arXiv for use of its open access interoperability.";

/// Dibuja las pestañas principales compartidas de la cabecera: [Biblioteca] y [Descubrir].
pub(crate) fn draw_header_tabs(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    win_w: i32,
    win_h: i32,
    theme: &theme::AppTheme,
    active_mode: UiMode,
) {
    let (tab_lib, tab_disc) = lib_tabs_rect(win_w, win_h);
    let p = theme.palette();

    let mut render_tab = |rects: &mut Vec<CanvasRect>,
                          texts: &mut Vec<CanvasText>,
                          rect: ButtonRect,
                          is_active: bool,
                          label: &str| {
        let (left, top, right, bottom) = rect;
        let r = ((bottom - top) * 0.5).max(6.0);
        let cy = top + (bottom - top) * 0.5 + theme::FONT_BODY * 0.35;
        if is_active {
            draw_card_shadow(rects, left, top, right, bottom, r, p.is_dark);
            rects.push(CanvasRect::rounded(left, top, right, bottom, r, p.base_300));
            rects.push(CanvasRect::rounded(
                left + 1.0,
                top + 1.0,
                right - 1.0,
                bottom - 1.0,
                r - 1.0,
                p.base_100,
            ));
            texts.push(CanvasText::new(
                (left + right) / 2.0,
                cy,
                theme::FONT_BODY,
                p.base_content,
                TextAlign::Center,
                true,
                label,
            ));
        } else {
            rects.push(CanvasRect::rounded(left, top, right, bottom, r, p.base_300));
            rects.push(CanvasRect::rounded(
                left + 1.0,
                top + 1.0,
                right - 1.0,
                bottom - 1.0,
                r - 1.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                (left + right) / 2.0,
                cy,
                theme::FONT_BODY,
                p.neutral_content,
                TextAlign::Center,
                false,
                label,
            ));
        }
    };

    render_tab(
        rects,
        texts,
        tab_lib,
        active_mode == UiMode::Library,
        "Biblioteca",
    );
    render_tab(
        rects,
        texts,
        tab_disc,
        active_mode == UiMode::Discover,
        "Descubrir",
    );
}

/// Render de la ZONA FIJA de Discover: cabecera editorial + sub-pestañas (Feed, Buscar, Áreas)
/// + barra de búsqueda si está en `DiscoverScreen::Search`, o cabecera con [← Volver] si está en `Detail`.
pub(crate) fn render_discover_header(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h_fixed = disc_content_y0(
        reader.win_h,
        reader.discover.screen,
        reader.status.is_some() || reader.discover.status_msg.is_some(),
    );
    if w <= 0 || h_fixed <= 0 {
        return None;
    }

    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Fondo del bloque fijo + línea divisoria inferior
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

    match reader.discover.screen {
        DiscoverScreen::Detail => {
            // Cabecera de Ficha: botón [← Volver] y título
            let back_rect = disc_detail_back_rect(w, reader.win_h);
            draw_card_shadow(
                &mut rects,
                back_rect.0,
                back_rect.1,
                back_rect.2,
                back_rect.3,
                8.0,
                p.is_dark,
            );
            draw_button(
                &mut rects,
                &mut texts,
                back_rect.0,
                back_rect.1,
                back_rect.2,
                back_rect.3,
                p.base_100,
                p.base_300,
                p.base_content,
                theme::FONT_BODY,
                true,
                "← Volver",
            );

            let title_y =
                back_rect.1 + (back_rect.3 - back_rect.1) * 0.5 + theme::FONT_TITLE * 0.35;
            texts.push(CanvasText::new(
                back_rect.2 + 20.0,
                title_y,
                theme::FONT_TITLE,
                p.base_content,
                TextAlign::Left,
                true,
                "Ficha del paper",
            ));
        }
        DiscoverScreen::Feed | DiscoverScreen::Search | DiscoverScreen::Areas => {
            // Pestañas principales de cabecera: [Biblioteca] y [Descubrir]
            draw_header_tabs(
                &mut rects,
                &mut texts,
                w,
                reader.win_h,
                &reader.theme,
                UiMode::Discover,
            );

            // Sub-pestañas: [Feed] [Buscar] [Áreas]
            let subtabs = disc_subtabs_rect(w, reader.win_h);
            for (tab, rect) in subtabs {
                let is_active = tab == reader.discover.active_tab;
                let label = match tab {
                    DiscoverTab::Feed => "Feed",
                    DiscoverTab::Search => "Buscar",
                    DiscoverTab::Areas => "Áreas",
                };

                let (l, t, r, b) = rect;
                let rad = ((b - t) * 0.5).max(4.0);
                if is_active {
                    draw_card_shadow(&mut rects, l, t, r, b, rad, p.is_dark);
                    draw_button(
                        &mut rects,
                        &mut texts,
                        l,
                        t,
                        r,
                        b,
                        p.primary,
                        p.primary,
                        p.primary_content,
                        theme::FONT_BODY,
                        true,
                        label,
                    );
                } else {
                    draw_button(
                        &mut rects,
                        &mut texts,
                        l,
                        t,
                        r,
                        b,
                        p.base_100,
                        p.base_300,
                        p.base_content,
                        theme::FONT_BODY,
                        false,
                        label,
                    );
                }
            }

            // Barra de búsqueda en pantalla Search
            if reader.discover.screen == DiscoverScreen::Search {
                let (input_r, clear_r, search_btn_r) = disc_search_rect(w, reader.win_h);
                let rad = ((input_r.3 - input_r.1) * 0.5).min(10.0);
                rects.push(CanvasRect::rounded(
                    input_r.0, input_r.1, input_r.2, input_r.3, rad, p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    input_r.0 + 1.0,
                    input_r.1 + 1.0,
                    input_r.2 - 1.0,
                    input_r.3 - 1.0,
                    rad - 1.0,
                    p.base_100,
                ));

                let query_text = if reader.discover.query.is_empty() {
                    "Buscar por título, autor o id (ej. 2401.12345)…"
                } else {
                    &reader.discover.query
                };
                let query_color = if reader.discover.query.is_empty() {
                    p.neutral_content
                } else {
                    p.base_content
                };

                let input_cy = input_r.1 + (input_r.3 - input_r.1) * 0.5 + theme::FONT_BODY * 0.35;
                texts.push(CanvasText::new(
                    input_r.0 + 14.0,
                    input_cy,
                    theme::FONT_BODY,
                    query_color,
                    TextAlign::Left,
                    false,
                    query_text,
                ));

                // Botón limpiar '✕' si hay consulta
                if !reader.discover.query.is_empty() {
                    let cw = clear_r.2 - clear_r.0;
                    let ch = clear_r.3 - clear_r.1;
                    let crad = ((ch * 0.5).min(cw * 0.5)).max(4.0);
                    rects.push(CanvasRect::rounded(
                        clear_r.0, clear_r.1, clear_r.2, clear_r.3, crad, p.base_200,
                    ));
                    let clear_cy = clear_r.1 + ch * 0.5 + theme::FONT_CAPTION * 0.35;
                    texts.push(CanvasText::new(
                        (clear_r.0 + clear_r.2) / 2.0,
                        clear_cy,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        true,
                        "✕",
                    ));
                }

                // Botón Buscar
                draw_card_shadow(
                    &mut rects,
                    search_btn_r.0,
                    search_btn_r.1,
                    search_btn_r.2,
                    search_btn_r.3,
                    rad,
                    p.is_dark,
                );
                draw_button(
                    &mut rects,
                    &mut texts,
                    search_btn_r.0,
                    search_btn_r.1,
                    search_btn_r.2,
                    search_btn_r.3,
                    p.primary,
                    p.primary,
                    p.primary_content,
                    theme::FONT_BODY,
                    true,
                    "Buscar",
                );
            }
        }
    }

    // Franja de status opcional abajo
    let status = reader
        .status
        .as_deref()
        .or(reader.discover.status_msg.as_deref());
    if let Some(msg) = status {
        let status_y = h_fixed as f32 - 10.0;
        texts.push(CanvasText::new(
            grid_pad(w),
            status_y,
            theme::FONT_CAPTION,
            p.primary,
            TextAlign::Left,
            true,
            msg,
        ));
    }

    jni_text_bitmap(w, h_fixed, p.base_200, &rects, &texts)
}

/// Render de la BANDA SCROLLEABLE de Discover: feed de papers, resultados de búsqueda,
/// lista de áreas / categorías o ficha detallada del paper seleccionado.
pub(crate) fn render_discover_zone(
    reader: &Reader,
    band_origin: i32,
    band_h: i32,
) -> Option<Bitmap> {
    let w = reader.win_w;
    if w <= 0 || band_h <= 0 {
        return None;
    }

    let p = reader.theme.palette();
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    match reader.discover.screen {
        DiscoverScreen::Feed | DiscoverScreen::Search => {
            let entries = if reader.discover.screen == DiscoverScreen::Feed {
                &reader.discover.feed_entries
            } else {
                &reader.discover.search_entries
            };

            let is_loading = reader.discover.phase == DiscoverPhase::Loading;

            if entries.is_empty() {
                // Estado vacío / cargando / error
                let center_x = w as f32 / 2.0;
                let center_y = (band_h as f32 * 0.35).clamp(80.0, 200.0);

                if let DiscoverPhase::Error(err) = &reader.discover.phase {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_DISPLAY,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        "No se pudo conectar con arXiv",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 36.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        err,
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 64.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Comprueba tu conexión a internet e inténtalo de nuevo.",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 110.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                } else if is_loading {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_DISPLAY,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        if reader.discover.screen == DiscoverScreen::Feed {
                            "Cargando papers de arXiv…"
                        } else {
                            "Buscando en arXiv…"
                        },
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 36.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Consultando export.arxiv.org/api/query",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 64.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Descargando resúmenes y metadatos de preprints",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 110.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                } else if reader.discover.screen == DiscoverScreen::Feed {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_DISPLAY,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        "Tu feed de arXiv está vacío",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 36.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Selecciona áreas de interés en la pestaña Áreas para ver novedades.",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 90.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                } else {
                    if reader.discover.query.is_empty() {
                        texts.push(CanvasText::new(
                            center_x,
                            center_y,
                            theme::FONT_DISPLAY,
                            p.base_content,
                            TextAlign::Center,
                            true,
                            "Explora millones de papers en arXiv",
                        ));
                        texts.push(CanvasText::new(
                            center_x,
                            center_y + 36.0,
                            theme::FONT_BODY,
                            p.neutral_content,
                            TextAlign::Center,
                            false,
                            "Busca por término, autor o identificador de arXiv.",
                        ));
                        texts.push(CanvasText::new(
                            center_x,
                            center_y + 64.0,
                            theme::FONT_CAPTION,
                            p.neutral_content,
                            TextAlign::Center,
                            false,
                            "Ejemplos: 2401.12345, attention, vaswani, cs.AI",
                        ));
                    } else {
                        texts.push(CanvasText::new(
                            center_x,
                            center_y,
                            theme::FONT_DISPLAY,
                            p.base_content,
                            TextAlign::Center,
                            true,
                            "Sin resultados encontrados",
                        ));
                        texts.push(CanvasText::new(
                            center_x,
                            center_y + 36.0,
                            theme::FONT_BODY,
                            p.neutral_content,
                            TextAlign::Center,
                            false,
                            format!("No se encontraron papers para «{}»", reader.discover.query),
                        ));
                        texts.push(CanvasText::new(
                            center_x,
                            center_y + 64.0,
                            theme::FONT_CAPTION,
                            p.neutral_content,
                            TextAlign::Center,
                            false,
                            "Intenta con términos más generales o un identificador arXiv directo.",
                        ));
                    }

                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 100.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                }
            } else {
                // Lista de tarjetas de papers
                for (idx, entry) in entries.iter().enumerate() {
                    let card_rect = disc_card_rect(w, idx);
                    let top_in_band = card_rect.1 - band_origin as f32;
                    let bot_in_band = card_rect.3 - band_origin as f32;

                    // Salto si la tarjeta está fuera de la banda visible
                    if bot_in_band < 0.0 || top_in_band > band_h as f32 {
                        continue;
                    }

                    let card_l = card_rect.0;
                    let card_r = card_rect.2;
                    let card_w = card_r - card_l;
                    let r = 12.0f32;

                    // Sombra visible multicapa + fondo y borde redondeado
                    draw_card_shadow(
                        &mut rects,
                        card_l,
                        top_in_band,
                        card_r,
                        bot_in_band,
                        r,
                        p.is_dark,
                    );
                    rects.push(CanvasRect::rounded(
                        card_l,
                        top_in_band,
                        card_r,
                        bot_in_band,
                        r,
                        p.base_300,
                    ));
                    rects.push(CanvasRect::rounded(
                        card_l + 1.0,
                        top_in_band + 1.0,
                        card_r - 1.0,
                        bot_in_band - 1.0,
                        r - 1.0,
                        p.base_100,
                    ));

                    // Fila de insignias (Preprint / Open Access / Categoría)
                    let badge_y = top_in_band + 14.0;
                    let badge_h = 22.0f32;
                    let badge_rad = badge_h * 0.5;
                    let badge_text_cy = badge_y + badge_h * 0.5 + theme::FONT_LABEL_CAPS * 0.35;

                    // Badge [Preprint]
                    let mut bx = card_l + 16.0;
                    let bw_prep = 68.0f32;
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_prep,
                        badge_y + badge_h,
                        badge_rad,
                        p.base_300,
                    ));
                    rects.push(CanvasRect::rounded(
                        bx + 1.0,
                        badge_y + 1.0,
                        bx + bw_prep - 1.0,
                        badge_y + badge_h - 1.0,
                        badge_rad - 1.0,
                        p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_prep / 2.0,
                        badge_text_cy,
                        theme::FONT_LABEL_CAPS,
                        p.neutral_content,
                        TextAlign::Center,
                        true,
                        "PREPRINT",
                    ));
                    bx += bw_prep + 8.0;

                    // Badge [Open Access]
                    let bw_oa = 88.0f32;
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_oa,
                        badge_y + badge_h,
                        badge_rad,
                        p.base_300,
                    ));
                    rects.push(CanvasRect::rounded(
                        bx + 1.0,
                        badge_y + 1.0,
                        bx + bw_oa - 1.0,
                        badge_y + badge_h - 1.0,
                        badge_rad - 1.0,
                        p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_oa / 2.0,
                        badge_text_cy,
                        theme::FONT_LABEL_CAPS,
                        p.primary,
                        TextAlign::Center,
                        true,
                        "OPEN ACCESS",
                    ));
                    bx += bw_oa + 8.0;

                    // Badge categoría primaria
                    if !entry.primary_category.is_empty() {
                        let full_label = category_label(&entry.primary_category);
                        let label_candidate =
                            (full_label.chars().count() as f32 * 7.5 + 16.0).max(46.0);
                        let (cat_str, bw_cat) = if bx + label_candidate <= card_r - 16.0 {
                            (full_label, label_candidate)
                        } else {
                            let code_w = (entry.primary_category.chars().count() as f32 * 8.0
                                + 16.0)
                                .max(44.0);
                            (entry.primary_category.as_str(), code_w)
                        };
                        rects.push(CanvasRect::rounded(
                            bx,
                            badge_y,
                            bx + bw_cat,
                            badge_y + badge_h,
                            badge_rad,
                            p.base_300,
                        ));
                        rects.push(CanvasRect::rounded(
                            bx + 1.0,
                            badge_y + 1.0,
                            bx + bw_cat - 1.0,
                            badge_y + badge_h - 1.0,
                            badge_rad - 1.0,
                            p.base_200,
                        ));
                        texts.push(CanvasText::new(
                            bx + bw_cat / 2.0,
                            badge_text_cy,
                            theme::FONT_LABEL_CAPS,
                            p.base_content,
                            TextAlign::Center,
                            true,
                            cat_str,
                        ));
                    }

                    // Título adaptativo (1 o 2 líneas)
                    let title_max_chars =
                        (((card_w - 32.0) / (theme::FONT_TITLE * 0.52)).floor() as usize).max(20);
                    let title_lines = wrap_text_chars(&entry.title, title_max_chars);
                    let has_two_title_lines = title_lines.len() > 1;

                    if has_two_title_lines {
                        texts.push(CanvasText::new(
                            card_l + 16.0,
                            top_in_band + 58.0,
                            theme::FONT_TITLE,
                            p.base_content,
                            TextAlign::Left,
                            true,
                            &title_lines[0],
                        ));
                        let line1_text = if title_lines.len() > 2 {
                            format!("{}…", title_lines[1].trim_end())
                        } else {
                            title_lines[1].clone()
                        };
                        texts.push(CanvasText::new(
                            card_l + 16.0,
                            top_in_band + 78.0,
                            theme::FONT_TITLE,
                            p.base_content,
                            TextAlign::Left,
                            true,
                            line1_text,
                        ));
                    } else if let Some(first_line) = title_lines.first() {
                        texts.push(CanvasText::new(
                            card_l + 16.0,
                            top_in_band + 60.0,
                            theme::FONT_TITLE,
                            p.base_content,
                            TextAlign::Left,
                            true,
                            first_line,
                        ));
                    }

                    // Autores
                    let authors_y = if has_two_title_lines {
                        top_in_band + 100.0
                    } else {
                        top_in_band + 84.0
                    };
                    let authors_max =
                        (((card_w - 32.0) / (theme::FONT_BODY * 0.52)).floor() as usize).max(20);
                    let authors_str = entry.authors.join(", ");
                    let authors_display = if authors_str.chars().count() > authors_max {
                        format!(
                            "{}…",
                            authors_str
                                .chars()
                                .take(authors_max.saturating_sub(1))
                                .collect::<String>()
                        )
                    } else {
                        authors_str
                    };
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        authors_y,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        authors_display,
                    ));

                    // Resumen / snippet
                    let summary_clean: String = entry
                        .summary
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let caption_max =
                        (((card_w - 32.0) / (theme::FONT_CAPTION * 0.52)).floor() as usize).max(25);
                    let summary_lines = wrap_text_chars(&summary_clean, caption_max);

                    if has_two_title_lines {
                        if let Some(first_sum) = summary_lines.first() {
                            let text = if summary_lines.len() > 1 {
                                format!("{first_sum}…")
                            } else {
                                first_sum.clone()
                            };
                            texts.push(CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 122.0,
                                theme::FONT_CAPTION,
                                p.base_content,
                                TextAlign::Left,
                                false,
                                text,
                            ));
                        }
                    } else {
                        if let Some(line0) = summary_lines.first() {
                            texts.push(CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 108.0,
                                theme::FONT_CAPTION,
                                p.base_content,
                                TextAlign::Left,
                                false,
                                line0,
                            ));
                        }
                        if summary_lines.len() > 1 {
                            let line1 = if summary_lines.len() > 2 {
                                format!("{}…", summary_lines[1])
                            } else {
                                summary_lines[1].clone()
                            };
                            texts.push(CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 126.0,
                                theme::FONT_CAPTION,
                                p.base_content,
                                TextAlign::Left,
                                false,
                                line1,
                            ));
                        }
                    }

                    // Fecha publicada y ID en la parte inferior izquierda
                    let pub_date = entry
                        .published
                        .split('T')
                        .next()
                        .unwrap_or(&entry.published);
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        bot_in_band - 26.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        format!("{pub_date}  •  arXiv:{}", entry.id),
                    ));

                    // Botón de acción en la esquina inferior derecha
                    let action_rect =
                        disc_card_action_rect((card_l, top_in_band, card_r, bot_in_band));

                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    let is_in_library = reader.is_arxiv_in_library(&entry.id);

                    draw_card_shadow(
                        &mut rects,
                        action_rect.0,
                        action_rect.1,
                        action_rect.2,
                        action_rect.3,
                        6.0,
                        p.is_dark,
                    );
                    if is_downloading {
                        let dl_label = if reader.discover.download_bytes > 0 {
                            format!(
                                "{:.1} MB",
                                reader.discover.download_bytes as f64 / 1_000_000.0
                            )
                        } else {
                            "Descargando…".to_string()
                        };
                        draw_button(
                            &mut rects,
                            &mut texts,
                            action_rect.0,
                            action_rect.1,
                            action_rect.2,
                            action_rect.3,
                            p.base_200,
                            p.primary,
                            p.primary,
                            theme::FONT_BODY,
                            true,
                            &dl_label,
                        );
                    } else if is_in_library {
                        draw_button(
                            &mut rects,
                            &mut texts,
                            action_rect.0,
                            action_rect.1,
                            action_rect.2,
                            action_rect.3,
                            p.primary,
                            p.primary,
                            p.primary_content,
                            theme::FONT_BODY,
                            true,
                            "Leer",
                        );
                    } else {
                        draw_button(
                            &mut rects,
                            &mut texts,
                            action_rect.0,
                            action_rect.1,
                            action_rect.2,
                            action_rect.3,
                            p.base_100,
                            p.base_300,
                            p.base_content,
                            theme::FONT_BODY,
                            true,
                            "Ver ficha →",
                        );
                    }
                }

                // Botón "Cargar más" si hay más páginas
                let has_more = if reader.discover.screen == DiscoverScreen::Feed {
                    reader.discover.feed_has_more
                } else {
                    reader.discover.search_has_more
                };

                let last_bottom = 16.0 + entries.len() as f32 * (disc_card_h() + disc_card_gap());
                let more_y = last_bottom - band_origin as f32;

                if has_more && more_y > -50.0 && more_y < band_h as f32 + 50.0 {
                    let more_rect = disc_more_btn_rect(w, more_y);
                    draw_card_shadow(
                        &mut rects,
                        more_rect.0,
                        more_rect.1,
                        more_rect.2,
                        more_rect.3,
                        10.0,
                        p.is_dark,
                    );
                    draw_button(
                        &mut rects,
                        &mut texts,
                        more_rect.0,
                        more_rect.1,
                        more_rect.2,
                        more_rect.3,
                        p.base_100,
                        p.base_300,
                        p.base_content,
                        theme::FONT_BODY,
                        true,
                        if is_loading {
                            "Cargando…"
                        } else {
                            "Cargar más papers"
                        },
                    );
                }

                // Atribución de interoperabilidad arXiv al final
                let attr_y = more_y + 80.0;
                if attr_y > 0.0 && attr_y < band_h as f32 + 40.0 {
                    texts.push(CanvasText::new(
                        w as f32 / 2.0,
                        attr_y,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                }
            }
        }
        DiscoverScreen::Areas => {
            // Lista de áreas / categorías temáticas de arXiv con selector
            for (idx, cat) in ARXIV_CATEGORIES.iter().enumerate() {
                let row_rect = disc_cat_row_rect(w, idx);
                let top_in_band = row_rect.1 - band_origin as f32;
                let bot_in_band = row_rect.3 - band_origin as f32;

                if bot_in_band < 0.0 || top_in_band > band_h as f32 {
                    continue;
                }

                let row_l = row_rect.0;
                let row_r = row_rect.2;
                let r = 10.0f32;

                let is_selected = reader.discover.selected_cats.iter().any(|c| c == cat.code);

                // Sombra y tarjeta de la fila
                draw_card_shadow(
                    &mut rects,
                    row_l,
                    top_in_band,
                    row_r,
                    bot_in_band,
                    r,
                    p.is_dark,
                );
                let border_col = if is_selected { p.primary } else { p.base_300 };
                rects.push(CanvasRect::rounded(
                    row_l,
                    top_in_band,
                    row_r,
                    bot_in_band,
                    r,
                    border_col,
                ));
                rects.push(CanvasRect::rounded(
                    row_l + 1.0,
                    top_in_band + 1.0,
                    row_r - 1.0,
                    bot_in_band - 1.0,
                    r - 1.0,
                    p.base_100,
                ));

                // Pastilla / pill con el código de categoría a la izquierda
                let code_h = 26.0f32;
                let code_w = (cat.code.chars().count() as f32 * 8.0 + 16.0).max(52.0);
                let code_t = top_in_band + (bot_in_band - top_in_band - code_h) / 2.0;
                let code_b = code_t + code_h;
                let code_l = row_l + 14.0;
                let code_r = code_l + code_w;
                let code_rad = code_h * 0.5;

                rects.push(CanvasRect::rounded(
                    code_l, code_t, code_r, code_b, code_rad, border_col,
                ));
                rects.push(CanvasRect::rounded(
                    code_l + 1.0,
                    code_t + 1.0,
                    code_r - 1.0,
                    code_b - 1.0,
                    code_rad - 1.0,
                    p.base_200,
                ));
                let code_text_color = if is_selected {
                    p.primary
                } else {
                    p.base_content
                };
                let code_cy = code_t + code_h * 0.5 + theme::FONT_LABEL_CAPS * 0.35;
                texts.push(CanvasText::new(
                    (code_l + code_r) / 2.0,
                    code_cy,
                    theme::FONT_LABEL_CAPS,
                    code_text_color,
                    TextAlign::Center,
                    true,
                    cat.code,
                ));

                // Nombre y grupo temático
                let label_x = code_r + 14.0;
                texts.push(CanvasText::new(
                    label_x,
                    top_in_band + 21.0,
                    theme::FONT_BODY,
                    p.base_content,
                    TextAlign::Left,
                    is_selected,
                    cat.label_es,
                ));
                texts.push(CanvasText::new(
                    label_x,
                    top_in_band + 39.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    cat.group,
                ));

                // Chip de estado de selección a la derecha
                let chip_w = 94.0f32;
                let chip_h = 32.0f32;
                let chip_r = row_r - 14.0;
                let chip_l = chip_r - chip_w;
                let chip_t = top_in_band + (bot_in_band - top_in_band - chip_h) / 2.0;
                let chip_b = chip_t + chip_h;

                if is_selected {
                    draw_button(
                        &mut rects,
                        &mut texts,
                        chip_l,
                        chip_t,
                        chip_r,
                        chip_b,
                        p.primary,
                        p.primary,
                        p.primary_content,
                        theme::FONT_LABEL_CAPS,
                        true,
                        "✓ ACTIVA",
                    );
                } else {
                    draw_button(
                        &mut rects,
                        &mut texts,
                        chip_l,
                        chip_t,
                        chip_r,
                        chip_b,
                        p.base_100,
                        p.base_300,
                        p.neutral_content,
                        theme::FONT_LABEL_CAPS,
                        false,
                        "+ AÑADIR",
                    );
                }
            }

            // Atribución al final de categorías
            let total_cats_h = 14.0 + ARXIV_CATEGORIES.len() as f32 * disc_cat_row_h();
            let attr_y = total_cats_h - band_origin as f32 + 30.0;
            if attr_y > 0.0 && attr_y < band_h as f32 + 40.0 {
                texts.push(CanvasText::new(
                    w as f32 / 2.0,
                    attr_y,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Center,
                    false,
                    ARXIV_ATTRIBUTION,
                ));
            }
        }
        DiscoverScreen::Detail => {
            // Pantalla de Ficha detallada de un paper
            if let Some(entry) = &reader.discover.selected_entry {
                let pad = disc_card_pad(w);
                let card_w = (w as f32 - 2.0 * pad).max(200.0);
                let inner_pad = 16.0f32;
                let inner_w = (card_w - 2.0 * inner_pad).max(180.0);

                let max_chars = ((inner_w / (theme::FONT_TITLE * 0.52)).floor() as usize).max(20);
                let title_lines = wrap_text_chars(&entry.title, max_chars);
                let authors_lines = wrap_text_chars(&entry.authors.join(", "), max_chars + 8);
                let abstract_lines = wrap_text_chars(&entry.summary, max_chars + 10);

                let hero_top = 16.0f32 - band_origin as f32;
                let badge_y = hero_top + 16.0;
                let title_y = badge_y + 36.0;
                let authors_y = title_y + title_lines.len() as f32 * 26.0 + 8.0;
                let divider_y = authors_y + authors_lines.len() as f32 * 22.0 + 10.0;
                let meta_y = divider_y + 18.0;

                let (action_btn_layout, _) = disc_detail_layout(w, entry);
                let action_btn = (
                    action_btn_layout.0,
                    action_btn_layout.1 - band_origin as f32,
                    action_btn_layout.2,
                    action_btn_layout.3 - band_origin as f32,
                );
                let hero_bot = action_btn.3 + 20.0;

                let r = 14.0f32;

                // Tarjeta 1: Metadatos y acción (Hero card)
                draw_card_shadow(
                    &mut rects,
                    pad,
                    hero_top,
                    pad + card_w,
                    hero_bot,
                    r,
                    p.is_dark,
                );
                rects.push(CanvasRect::rounded(
                    pad,
                    hero_top,
                    pad + card_w,
                    hero_bot,
                    r,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    pad + 1.0,
                    hero_top + 1.0,
                    pad + card_w - 1.0,
                    hero_bot - 1.0,
                    r - 1.0,
                    p.base_100,
                ));

                // Fila de insignias dentro de la tarjeta hero
                let badge_h = 22.0f32;
                let badge_rad = badge_h * 0.5;
                let badge_text_cy = badge_y + badge_h * 0.5 + theme::FONT_LABEL_CAPS * 0.35;
                let mut bx = pad + inner_pad;

                let bw_prep = 68.0f32;
                rects.push(CanvasRect::rounded(
                    bx,
                    badge_y,
                    bx + bw_prep,
                    badge_y + badge_h,
                    badge_rad,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    bx + 1.0,
                    badge_y + 1.0,
                    bx + bw_prep - 1.0,
                    badge_y + badge_h - 1.0,
                    badge_rad - 1.0,
                    p.base_200,
                ));
                texts.push(CanvasText::new(
                    bx + bw_prep / 2.0,
                    badge_text_cy,
                    theme::FONT_LABEL_CAPS,
                    p.neutral_content,
                    TextAlign::Center,
                    true,
                    "PREPRINT",
                ));
                bx += bw_prep + 8.0;

                let bw_oa = 88.0f32;
                rects.push(CanvasRect::rounded(
                    bx,
                    badge_y,
                    bx + bw_oa,
                    badge_y + badge_h,
                    badge_rad,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    bx + 1.0,
                    badge_y + 1.0,
                    bx + bw_oa - 1.0,
                    badge_y + badge_h - 1.0,
                    badge_rad - 1.0,
                    p.base_200,
                ));
                texts.push(CanvasText::new(
                    bx + bw_oa / 2.0,
                    badge_text_cy,
                    theme::FONT_LABEL_CAPS,
                    p.primary,
                    TextAlign::Center,
                    true,
                    "OPEN ACCESS",
                ));
                bx += bw_oa + 8.0;

                if !entry.primary_category.is_empty() {
                    let full_label = category_label(&entry.primary_category);
                    let label_candidate =
                        (full_label.chars().count() as f32 * 7.5 + 16.0).max(46.0);
                    let (cat_str, bw_cat) = if bx + label_candidate <= pad + card_w - inner_pad {
                        (full_label, label_candidate)
                    } else {
                        let code_w =
                            (entry.primary_category.chars().count() as f32 * 8.0 + 16.0).max(44.0);
                        (entry.primary_category.as_str(), code_w)
                    };
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_cat,
                        badge_y + badge_h,
                        badge_rad,
                        p.base_300,
                    ));
                    rects.push(CanvasRect::rounded(
                        bx + 1.0,
                        badge_y + 1.0,
                        bx + bw_cat - 1.0,
                        badge_y + badge_h - 1.0,
                        badge_rad - 1.0,
                        p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_cat / 2.0,
                        badge_text_cy,
                        theme::FONT_LABEL_CAPS,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        cat_str,
                    ));
                }

                // Título multilínea
                let mut cur_y = title_y;
                for line in &title_lines {
                    texts.push(CanvasText::new(
                        pad + inner_pad,
                        cur_y,
                        theme::FONT_TITLE,
                        p.base_content,
                        TextAlign::Left,
                        true,
                        line,
                    ));
                    cur_y += 26.0;
                }

                // Autores multilínea
                cur_y = authors_y;
                for line in &authors_lines {
                    texts.push(CanvasText::new(
                        pad + inner_pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 22.0;
                }

                // Hairline divisoria
                rects.push(CanvasRect::sharp(
                    pad + inner_pad,
                    divider_y,
                    pad + card_w - inner_pad,
                    divider_y + 1.0,
                    p.base_300,
                ));

                // Metadatos
                let pub_date = entry
                    .published
                    .split('T')
                    .next()
                    .unwrap_or(&entry.published);
                let up_date = entry.updated.split('T').next().unwrap_or(&entry.updated);
                let meta_str = format!(
                    "ID: arXiv:{}  •  Publicado: {}  •  Actualizado: {}",
                    entry.id, pub_date, up_date
                );
                texts.push(CanvasText::new(
                    pad + inner_pad,
                    meta_y,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    meta_str,
                ));

                // Botón de acción (Descargar / Descargando / Leer) usando el layout unificado
                let is_downloading =
                    reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                let is_in_library = reader.is_arxiv_in_library(&entry.id);

                draw_card_shadow(
                    &mut rects,
                    action_btn.0,
                    action_btn.1,
                    action_btn.2,
                    action_btn.3,
                    10.0,
                    p.is_dark,
                );
                if is_downloading {
                    let dl_label = if reader.discover.download_bytes > 0 {
                        format!(
                            "Descargando ({:.1} MB)…",
                            reader.discover.download_bytes as f64 / 1_000_000.0
                        )
                    } else {
                        "Descargando PDF…".to_string()
                    };
                    draw_button(
                        &mut rects,
                        &mut texts,
                        action_btn.0,
                        action_btn.1,
                        action_btn.2,
                        action_btn.3,
                        p.base_200,
                        p.primary,
                        p.primary,
                        theme::FONT_BODY,
                        true,
                        &dl_label,
                    );
                } else if is_in_library {
                    draw_button(
                        &mut rects,
                        &mut texts,
                        action_btn.0,
                        action_btn.1,
                        action_btn.2,
                        action_btn.3,
                        p.primary,
                        p.primary,
                        p.primary_content,
                        theme::FONT_BODY,
                        true,
                        "Abrir en el lector",
                    );
                } else {
                    draw_button(
                        &mut rects,
                        &mut texts,
                        action_btn.0,
                        action_btn.1,
                        action_btn.2,
                        action_btn.3,
                        p.primary,
                        p.primary,
                        p.primary_content,
                        theme::FONT_BODY,
                        true,
                        "⬇ Descargar PDF",
                    );
                }

                // Tarjeta 2: Resumen / Abstract
                let abstract_top = hero_bot + 16.0;
                let abstract_header_y = abstract_top + 28.0;
                let abstract_body_y = abstract_header_y + 24.0;
                let abstract_bot = abstract_body_y + abstract_lines.len() as f32 * 22.0 + 20.0;

                draw_card_shadow(
                    &mut rects,
                    pad,
                    abstract_top,
                    pad + card_w,
                    abstract_bot,
                    r,
                    p.is_dark,
                );
                rects.push(CanvasRect::rounded(
                    pad,
                    abstract_top,
                    pad + card_w,
                    abstract_bot,
                    r,
                    p.base_300,
                ));
                rects.push(CanvasRect::rounded(
                    pad + 1.0,
                    abstract_top + 1.0,
                    pad + card_w - 1.0,
                    abstract_bot - 1.0,
                    r - 1.0,
                    p.base_100,
                ));

                texts.push(CanvasText::new(
                    pad + inner_pad,
                    abstract_header_y,
                    theme::FONT_TITLE,
                    p.base_content,
                    TextAlign::Left,
                    true,
                    "Resumen (Abstract)",
                ));

                rects.push(CanvasRect::sharp(
                    pad + inner_pad,
                    abstract_header_y + 10.0,
                    pad + card_w - inner_pad,
                    abstract_header_y + 11.0,
                    p.base_300,
                ));

                cur_y = abstract_body_y;
                for line in &abstract_lines {
                    texts.push(CanvasText::new(
                        pad + inner_pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.base_content,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 22.0;
                }

                // Atribución requerida por arXiv
                let attr_y = abstract_bot + 36.0;
                texts.push(CanvasText::new(
                    w as f32 / 2.0,
                    attr_y,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Center,
                    false,
                    ARXIV_ATTRIBUTION,
                ));
            } else {
                texts.push(CanvasText::new(
                    w as f32 / 2.0,
                    60.0,
                    theme::FONT_BODY,
                    p.neutral_content,
                    TextAlign::Center,
                    false,
                    "Ningún paper seleccionado",
                ));
            }
        }
    }

    jni_text_bitmap(w, band_h, p.base_200, &rects, &texts)
}
