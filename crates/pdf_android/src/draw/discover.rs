// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pantalla de Descubrir (arXiv Discover): cabecera fija (tabs + subtabs + buscador),
//! banda scrolleable con feed de papers, resultados de búsqueda, ficha de detalle
//! y selector de áreas temáticas.

use crate::reader::discover_categories::{ARXIV_CATEGORIES, category_label};
use crate::reader::discover_state::{DiscoverPhase, DiscoverScreen, DiscoverTab};
use crate::reader::{
    Reader, UiMode, disc_card_action_rect, disc_card_gap, disc_card_h, disc_card_pad,
    disc_card_rect, disc_card_w, disc_cat_row_h, disc_cat_row_rect, disc_content_y0,
    disc_detail_action_rect, disc_detail_back_rect, disc_more_btn_rect, disc_search_rect,
    disc_subtabs_rect, grid_pad, lib_tabs_rect,
};
use crate::theme;
use pdf_core::Bitmap;

use super::{ButtonRect, CanvasRect, CanvasText, TextAlign, draw_button, jni_text_bitmap};

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

    let render_tab = |rects: &mut Vec<CanvasRect>,
                      texts: &mut Vec<CanvasText>,
                      rect: ButtonRect,
                      is_active: bool,
                      label: &str| {
        let (left, top, right, bottom) = rect;
        let r = ((bottom - top) * 0.5).max(6.0);
        if is_active {
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
                top + (bottom - top) * 0.68,
                theme::FONT_BODY,
                p.base_content,
                TextAlign::Center,
                true,
                label,
            ));
        } else {
            texts.push(CanvasText::new(
                (left + right) / 2.0,
                top + (bottom - top) * 0.68,
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

/// Helper simple para envolver texto en líneas según ancho aproximado en caracteres.
fn wrap_text_chars(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in trimmed.split_whitespace() {
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
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
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

            let title_y = back_rect.1 + (back_rect.3 - back_rect.1) * 0.72;
            texts.push(CanvasText::new(
                back_rect.2 + 24.0,
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
                    rects.push(CanvasRect::rounded(l, t, r, b, rad, p.primary));
                    texts.push(CanvasText::new(
                        (l + r) / 2.0,
                        t + (b - t) * 0.68,
                        theme::FONT_BODY,
                        p.primary_content,
                        TextAlign::Center,
                        true,
                        label,
                    ));
                } else {
                    rects.push(CanvasRect::rounded(l, t, r, b, rad, p.base_100));
                    texts.push(CanvasText::new(
                        (l + r) / 2.0,
                        t + (b - t) * 0.68,
                        theme::FONT_BODY,
                        p.base_content,
                        TextAlign::Center,
                        false,
                        label,
                    ));
                }
            }

            // Barra de búsqueda en pantalla Search
            if reader.discover.screen == DiscoverScreen::Search {
                let (input_r, clear_r, search_btn_r) = disc_search_rect(w, reader.win_h);
                // Campo de texto
                let rad = 8.0f32;
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
                    "Buscar por título, autor, id (ej. 2401.12345)…"
                } else {
                    &reader.discover.query
                };
                let query_color = if reader.discover.query.is_empty() {
                    p.neutral_content
                } else {
                    p.base_content
                };

                texts.push(CanvasText::new(
                    input_r.0 + 14.0,
                    input_r.1 + (input_r.3 - input_r.1) * 0.68,
                    theme::FONT_BODY,
                    query_color,
                    TextAlign::Left,
                    false,
                    query_text,
                ));

                // Botón limpiar '✕' si hay consulta
                if !reader.discover.query.is_empty() {
                    texts.push(CanvasText::new(
                        (clear_r.0 + clear_r.2) / 2.0,
                        clear_r.1 + (clear_r.3 - clear_r.1) * 0.70,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        true,
                        "✕",
                    ));
                }

                // Botón Buscar
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
        let status_y = h_fixed as f32 - 12.0;
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

    let pad = disc_card_pad(w);

    match reader.discover.screen {
        DiscoverScreen::Feed | DiscoverScreen::Search => {
            let entries = if reader.discover.screen == DiscoverScreen::Feed {
                &reader.discover.feed_entries
            } else {
                &reader.discover.search_entries
            };

            let is_loading = reader.discover.phase == DiscoverPhase::Loading;

            if entries.is_empty() {
                // Estado vacío / cargando
                let center_x = w as f32 / 2.0;
                let center_y = (band_h as f32 / 3.0).max(60.0);

                if is_loading {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_TITLE,
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
                        center_y + 32.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Consultando export.arxiv.org/api/query",
                    ));
                } else if reader.discover.screen == DiscoverScreen::Feed {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_TITLE,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        "No hay papers en las categorías seleccionadas",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 32.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Selecciona áreas de interés en la pestaña Áreas",
                    ));
                } else {
                    texts.push(CanvasText::new(
                        center_x,
                        center_y,
                        theme::FONT_TITLE,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        "Escribe un término o pega un ID de arXiv",
                    ));
                    texts.push(CanvasText::new(
                        center_x,
                        center_y + 32.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "Ejemplos: 2401.12345, ti:attention, au:vaswani, cat:cs.AI",
                    ));
                }

                // Atribución de interoperabilidad obligatoria de arXiv
                texts.push(CanvasText::new(
                    center_x,
                    center_y + 80.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Center,
                    false,
                    ARXIV_ATTRIBUTION,
                ));
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
                    let r = 10.0f32;

                    // Fondo y borde de la tarjeta
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

                    // Badge [Preprint]
                    let mut bx = card_l + 16.0;
                    let bw_prep = 68.0f32;
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_prep,
                        badge_y + badge_h,
                        4.0,
                        p.neutral,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_prep / 2.0,
                        badge_y + 15.0,
                        theme::FONT_LABEL_CAPS,
                        p.neutral_content,
                        TextAlign::Center,
                        true,
                        "PREPRINT",
                    ));
                    bx += bw_prep + 8.0;

                    // Badge [Open Access]
                    let bw_oa = 84.0f32;
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_oa,
                        badge_y + badge_h,
                        4.0,
                        p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_oa / 2.0,
                        badge_y + 15.0,
                        theme::FONT_LABEL_CAPS,
                        p.primary,
                        TextAlign::Center,
                        true,
                        "OPEN ACCESS",
                    ));
                    bx += bw_oa + 8.0;
                    // Badge categoría primaria
                    if !entry.primary_category.is_empty() {
                        let label = category_label(&entry.primary_category);
                        let bw_cat = (label.chars().count() as f32 * 7.5 + 16.0).max(50.0);
                        rects.push(CanvasRect::rounded(
                            bx,
                            badge_y,
                            bx + bw_cat,
                            badge_y + badge_h,
                            4.0,
                            p.base_200,
                        ));
                        texts.push(CanvasText::new(
                            bx + bw_cat / 2.0,
                            badge_y + 15.0,
                            theme::FONT_LABEL_CAPS,
                            p.base_content,
                            TextAlign::Center,
                            true,
                            label,
                        ));
                    }

                    // Título (truncado a ~95 caracteres)
                    let title_text = if entry.title.chars().count() > 95 {
                        let s: String = entry.title.chars().take(92).collect();
                        format!("{s}…")
                    } else {
                        entry.title.clone()
                    };
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        top_in_band + 62.0,
                        theme::FONT_TITLE,
                        p.base_content,
                        TextAlign::Left,
                        true,
                        title_text,
                    ));

                    // Autores
                    let authors_str = entry.authors.join(", ");
                    let authors_text = if authors_str.chars().count() > 80 {
                        let s: String = authors_str.chars().take(77).collect();
                        format!("{s}…")
                    } else {
                        authors_str
                    };
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        top_in_band + 90.0,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        authors_text,
                    ));

                    // Resumen (snippet de ~130 chars)
                    let summary_str: String = entry
                        .summary
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let summary_snippet = if summary_str.chars().count() > 130 {
                        let s: String = summary_str.chars().take(127).collect();
                        format!("{s}…")
                    } else {
                        summary_str
                    };
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        top_in_band + 120.0,
                        theme::FONT_CAPTION,
                        p.base_content,
                        TextAlign::Left,
                        false,
                        summary_snippet,
                    ));

                    // Fecha publicada abajo a la izquierda
                    let pub_date = entry
                        .published
                        .split('T')
                        .next()
                        .unwrap_or(&entry.published);
                    texts.push(CanvasText::new(
                        card_l + 16.0,
                        bot_in_band - 20.0,
                        theme::FONT_CAPTION,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        format!("Publicado: {pub_date}  |  arXiv:{}", entry.id),
                    ));

                    // Botón de acción en la esquina inferior derecha
                    let action_rect =
                        disc_card_action_rect((card_l, top_in_band, card_r, bot_in_band));

                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());

                    let is_in_library = reader
                        .library_list
                        .iter()
                        .any(|b| b.name.contains(&entry.id));

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
                            "Ficha",
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
                            "Cargar más"
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
                let r = 8.0f32;

                let is_selected = reader.discover.selected_cats.iter().any(|c| c == cat.code);

                // Fondo de la fila
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

                // Código y etiqueta
                texts.push(CanvasText::new(
                    row_l + 16.0,
                    top_in_band + 24.0,
                    theme::FONT_BODY,
                    p.base_content,
                    TextAlign::Left,
                    true,
                    cat.code,
                ));
                texts.push(CanvasText::new(
                    row_l + 100.0,
                    top_in_band + 24.0,
                    theme::FONT_BODY,
                    p.base_content,
                    TextAlign::Left,
                    false,
                    cat.label_es,
                ));
                texts.push(CanvasText::new(
                    row_l + 100.0,
                    top_in_band + 42.0,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    cat.group,
                ));

                // Chip de estado de selección a la derecha
                let chip_w = 90.0f32;
                let chip_h = 32.0f32;
                let chip_r = row_r - 14.0;
                let chip_l = chip_r - chip_w;
                let chip_t = top_in_band + (bot_in_band - top_in_band - chip_h) / 2.0;
                let chip_b = chip_t + chip_h;

                if is_selected {
                    rects.push(CanvasRect::rounded(
                        chip_l, chip_t, chip_r, chip_b, 4.0, p.primary,
                    ));
                    texts.push(CanvasText::new(
                        (chip_l + chip_r) / 2.0,
                        chip_t + 21.0,
                        theme::FONT_LABEL_CAPS,
                        p.primary_content,
                        TextAlign::Center,
                        true,
                        "✓ ACTIVA",
                    ));
                } else {
                    rects.push(CanvasRect::rounded(
                        chip_l, chip_t, chip_r, chip_b, 4.0, p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        (chip_l + chip_r) / 2.0,
                        chip_t + 21.0,
                        theme::FONT_LABEL_CAPS,
                        p.neutral_content,
                        TextAlign::Center,
                        false,
                        "+ AÑADIR",
                    ));
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
                let card_w = disc_card_w(w);
                let top_in_band = 16.0 - band_origin as f32;

                // Fila de insignias
                let badge_y = top_in_band + 8.0;
                let mut bx = pad;
                let bw_prep = 68.0f32;
                rects.push(CanvasRect::rounded(
                    bx,
                    badge_y,
                    bx + bw_prep,
                    badge_y + 22.0,
                    4.0,
                    p.neutral,
                ));
                texts.push(CanvasText::new(
                    bx + bw_prep / 2.0,
                    badge_y + 15.0,
                    theme::FONT_LABEL_CAPS,
                    p.neutral_content,
                    TextAlign::Center,
                    true,
                    "PREPRINT",
                ));
                bx += bw_prep + 8.0;

                let bw_oa = 84.0f32;
                rects.push(CanvasRect::rounded(
                    bx,
                    badge_y,
                    bx + bw_oa,
                    badge_y + 22.0,
                    4.0,
                    p.base_200,
                ));
                texts.push(CanvasText::new(
                    bx + bw_oa / 2.0,
                    badge_y + 15.0,
                    theme::FONT_LABEL_CAPS,
                    p.primary,
                    TextAlign::Center,
                    true,
                    "OPEN ACCESS",
                ));
                bx += bw_oa + 8.0;

                if !entry.primary_category.is_empty() {
                    let bw_cat = (entry.primary_category.len() as f32 * 8.0 + 16.0).max(50.0);
                    rects.push(CanvasRect::rounded(
                        bx,
                        badge_y,
                        bx + bw_cat,
                        badge_y + 22.0,
                        4.0,
                        p.base_200,
                    ));
                    texts.push(CanvasText::new(
                        bx + bw_cat / 2.0,
                        badge_y + 15.0,
                        theme::FONT_LABEL_CAPS,
                        p.base_content,
                        TextAlign::Center,
                        true,
                        &entry.primary_category,
                    ));
                }

                // Título completo (multilínea)
                let max_chars = ((card_w / (theme::FONT_TITLE * 0.52)).floor() as usize).max(20);
                let title_lines = wrap_text_chars(&entry.title, max_chars);
                let mut cur_y = badge_y + 44.0;
                for line in &title_lines {
                    texts.push(CanvasText::new(
                        pad,
                        cur_y,
                        theme::FONT_TITLE,
                        p.base_content,
                        TextAlign::Left,
                        true,
                        line,
                    ));
                    cur_y += 26.0;
                }

                // Autores
                cur_y += 6.0;
                let authors_lines = wrap_text_chars(&entry.authors.join(", "), max_chars + 10);
                for line in &authors_lines {
                    texts.push(CanvasText::new(
                        pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.neutral_content,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 22.0;
                }

                // Metadatos adicionales (fechas, ID, DOI)
                cur_y += 10.0;
                let pub_date = entry
                    .published
                    .split('T')
                    .next()
                    .unwrap_or(&entry.published);
                let up_date = entry.updated.split('T').next().unwrap_or(&entry.updated);
                let meta_str = format!(
                    "ID: arXiv:{}  |  Publicado: {}  |  Actualizado: {}",
                    entry.id, pub_date, up_date
                );
                texts.push(CanvasText::new(
                    pad,
                    cur_y,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
                    false,
                    meta_str,
                ));

                // Botón de acción (Descargar / Descargando / Leer)
                cur_y += 20.0;
                let action_btn = disc_detail_action_rect(w, cur_y);

                let is_downloading =
                    reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());

                let is_in_library = reader
                    .library_list
                    .iter()
                    .any(|b| b.name.contains(&entry.id));

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

                // Resumen / Abstract completo
                cur_y += 70.0;
                texts.push(CanvasText::new(
                    pad,
                    cur_y,
                    theme::FONT_BODY,
                    p.base_content,
                    TextAlign::Left,
                    true,
                    "Resumen (Abstract):",
                ));

                cur_y += 22.0;
                let abstract_lines = wrap_text_chars(&entry.summary, max_chars + 12);
                for line in &abstract_lines {
                    texts.push(CanvasText::new(
                        pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.base_content,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 20.0;
                }

                // Atribución requerida por arXiv
                cur_y += 30.0;
                texts.push(CanvasText::new(
                    pad,
                    cur_y,
                    theme::FONT_CAPTION,
                    p.neutral_content,
                    TextAlign::Left,
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
