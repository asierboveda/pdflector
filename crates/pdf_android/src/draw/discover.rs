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
    ButtonRect, ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign, TextFont,
    draw_astryx_button, draw_elevation_shadow, draw_inset_ring, draw_surface_card, draw_token_pill,
    jni_text_bitmap,
};

/// Texto de atribución requerido por las condiciones de interoperabilidad abierta de arXiv.
pub(crate) const ARXIV_ATTRIBUTION: &str =
    "Thank you to arXiv for use of its open access interoperability.";

/// Dibuja las pestañas principales compartidas de la cabecera: [Biblioteca] y [Descubrir]
/// como un control segmentado Astryx (`bg_muted` rail + pestaña activa elevada `bg_card`).
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

    // Rail contenedor segmentado Astryx alrededor de ambas pestañas (conservando los hitboxes exactos)
    let rail_l = tab_lib.0 - 4.0;
    let rail_t = tab_lib.1 - 4.0;
    let rail_r = tab_disc.2 + 4.0;
    let rail_b = tab_lib.3 + 4.0;
    draw_surface_card(
        rects,
        (rail_l, rail_t, rail_r, rail_b),
        theme::RADIUS_CONTAINER,
        p.bg_muted,
        p.border,
        Elevation::None,
        p.is_dark,
    );

    let render_tab = |rects: &mut Vec<CanvasRect>,
                      texts: &mut Vec<CanvasText>,
                      rect: ButtonRect,
                      is_active: bool,
                      label: &str| {
        let (left, top, right, bottom) = rect;
        let cy = top + (bottom - top) * 0.5 + theme::FONT_BODY * 0.34;
        if is_active {
            draw_surface_card(
                rects,
                rect,
                theme::RADIUS_ELEMENT,
                p.bg_card,
                p.border,
                Elevation::Low,
                p.is_dark,
            );
            // Barra de acento inferior centrada de 3 px
            let bar_w = ((right - left) * 0.34).clamp(24.0, 56.0);
            let cx = (left + right) * 0.5;
            rects.push(CanvasRect::rounded(
                cx - bar_w * 0.5,
                bottom - 4.5,
                cx + bar_w * 0.5,
                bottom - 1.5,
                1.5,
                p.accent,
            ));
            texts.push(
                CanvasText::new(
                    cx,
                    cy,
                    theme::FONT_BODY,
                    p.text_primary,
                    TextAlign::Center,
                    true,
                    label,
                )
                .with_tracking(theme::TRACKING_TIGHT),
            );
        } else {
            texts.push(CanvasText::new(
                (left + right) * 0.5,
                cy,
                theme::FONT_BODY,
                p.text_secondary,
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

/// Dibuja la fila de insignias semánticas Astryx de un paper:
/// `PREPRINT` (neutro `bg_muted`), `OPEN ACCESS` (semántico Teal) y categoría primaria (semántico Purple).
fn draw_paper_badges(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    start_x: f32,
    badge_y: f32,
    max_right: f32,
    primary_category: &str,
    p: &theme::ThemePalette,
) {
    let badge_h = 22.0f32;
    let mut bx = start_x;

    // 1. PREPRINT (neutro)
    let bw_prep = 72.0f32;
    draw_token_pill(
        rects,
        texts,
        (bx, badge_y, bx + bw_prep, badge_y + badge_h),
        p.bg_muted,
        Some(p.border),
        p.text_secondary,
        theme::FONT_MICRO,
        TextFont::Sans,
        theme::TRACKING_CAPS,
        true,
        "PREPRINT",
    );
    bx += bw_prep + 8.0;

    // 2. OPEN ACCESS (Astryx Teal semántico)
    let bw_oa = 96.0f32;
    draw_token_pill(
        rects,
        texts,
        (bx, badge_y, bx + bw_oa, badge_y + badge_h),
        p.teal_bg,
        None,
        p.teal_text,
        theme::FONT_MICRO,
        TextFont::Sans,
        theme::TRACKING_CAPS,
        true,
        "OPEN ACCESS",
    );
    bx += bw_oa + 8.0;

    // 3. Categoría primaria (Astryx Purple semántico)
    if !primary_category.is_empty() {
        let full_label = category_label(primary_category);
        let label_candidate = (full_label.chars().count() as f32 * 7.2 + 18.0).max(50.0);
        let (cat_str, bw_cat) = if bx + label_candidate <= max_right {
            (full_label, label_candidate)
        } else {
            let code_w = (primary_category.chars().count() as f32 * 7.8 + 18.0).max(46.0);
            (primary_category, code_w)
        };
        if bx + bw_cat <= max_right + 4.0 {
            draw_token_pill(
                rects,
                texts,
                (bx, badge_y, bx + bw_cat, badge_y + badge_h),
                p.purple_bg,
                None,
                p.purple_text,
                theme::FONT_MICRO,
                TextFont::Sans,
                0.02,
                true,
                cat_str,
            );
        }
    }
}

/// Dibuja una tarjeta de estado centrada (error, carga o vacío) en lenguaje Astryx.
#[allow(clippy::too_many_arguments)]
fn draw_discover_callout(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    win_w: i32,
    top_y: f32,
    tag: &str,
    tag_bg: u32,
    tag_fg: u32,
    border_col: u32,
    title: &str,
    body: &str,
    hint: Option<&str>,
    p: &theme::ThemePalette,
) {
    let center_x = win_w as f32 * 0.5;
    let card_w = (win_w as f32 - 2.0 * grid_pad(win_w)).clamp(260.0, 580.0);
    let card_l = center_x - card_w * 0.5;
    let card_r = center_x + card_w * 0.5;
    let card_h = if hint.is_some() { 196.0 } else { 166.0 };
    let card_b = top_y + card_h;

    draw_surface_card(
        rects,
        (card_l, top_y, card_r, card_b),
        theme::RADIUS_CONTAINER,
        p.bg_card,
        border_col,
        Elevation::Low,
        p.is_dark,
    );

    let tag_w = (tag.chars().count() as f32 * 8.2 + 24.0).max(76.0);
    draw_token_pill(
        rects,
        texts,
        (
            center_x - tag_w * 0.5,
            top_y + 22.0,
            center_x + tag_w * 0.5,
            top_y + 44.0,
        ),
        tag_bg,
        None,
        tag_fg,
        theme::FONT_MICRO,
        TextFont::Sans,
        theme::TRACKING_CAPS,
        true,
        tag,
    );

    texts.push(
        CanvasText::new(
            center_x,
            top_y + 84.0,
            theme::FONT_H1,
            p.text_primary,
            TextAlign::Center,
            true,
            title,
        )
        .with_tracking(theme::TRACKING_TIGHT),
    );
    texts.push(CanvasText::new(
        center_x,
        top_y + 116.0,
        theme::FONT_BODY,
        p.text_secondary,
        TextAlign::Center,
        false,
        body,
    ));
    if let Some(h) = hint {
        texts.push(CanvasText::new(
            center_x,
            top_y + 146.0,
            theme::FONT_CAPTION,
            p.text_secondary,
            TextAlign::Center,
            false,
            h,
        ));
    }

    texts.push(CanvasText::new(
        center_x,
        card_b + 30.0,
        theme::FONT_CAPTION,
        p.text_secondary,
        TextAlign::Center,
        false,
        ARXIV_ATTRIBUTION,
    ));
}

/// Render de la ZONA FIJA de Discover: cabecera Astryx (`bg_surface`) + sub-pestañas
/// tonales (Feed, Buscar, Áreas) + buscador rehundido o cabecera de ficha `[← Volver]`.
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

    match reader.discover.screen {
        DiscoverScreen::Detail => {
            let back_rect = disc_detail_back_rect(w, reader.win_h);
            draw_astryx_button(
                &mut rects,
                &mut texts,
                back_rect,
                ButtonVariant::Ghost,
                &p,
                theme::FONT_BODY,
                true,
                "← Volver",
            );

            let title_y =
                back_rect.1 + (back_rect.3 - back_rect.1) * 0.5 + theme::FONT_TITLE * 0.35;
            texts.push(
                CanvasText::new(
                    back_rect.2 + 20.0,
                    title_y,
                    theme::FONT_TITLE,
                    p.text_primary,
                    TextAlign::Left,
                    true,
                    "Ficha del paper",
                )
                .with_tracking(theme::TRACKING_TIGHT),
            );
        }
        DiscoverScreen::Feed | DiscoverScreen::Search | DiscoverScreen::Areas => {
            draw_header_tabs(
                &mut rects,
                &mut texts,
                w,
                reader.win_h,
                &reader.theme,
                UiMode::Discover,
            );

            // Sub-pestañas Astryx: tokens píldora tonales [Feed] [Buscar] [Áreas]
            for (tab, rect) in disc_subtabs_rect(w, reader.win_h) {
                let is_active = tab == reader.discover.active_tab;
                let label = match tab {
                    DiscoverTab::Feed => "Feed",
                    DiscoverTab::Search => "Buscar",
                    DiscoverTab::Areas => "Áreas",
                };
                let (fill, border, fg) = if is_active {
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
                    theme::FONT_BODY,
                    TextFont::Sans,
                    0.0,
                    is_active,
                    label,
                );
            }

            // Barra de búsqueda Astryx en pantalla Search
            if reader.discover.screen == DiscoverScreen::Search {
                let (input_r, clear_r, search_btn_r) = disc_search_rect(w, reader.win_h);
                let has_query = !reader.discover.query.is_empty();
                if has_query {
                    draw_inset_ring(
                        &mut rects,
                        input_r,
                        theme::RADIUS_ELEMENT,
                        p.accent,
                        p.bg_card,
                    );
                } else {
                    draw_surface_card(
                        &mut rects,
                        input_r,
                        theme::RADIUS_ELEMENT,
                        p.bg_body,
                        p.border_emphasized,
                        Elevation::None,
                        p.is_dark,
                    );
                }

                let input_cy = input_r.1 + (input_r.3 - input_r.1) * 0.5 + theme::FONT_BODY * 0.35;
                texts.push(CanvasText::new(
                    input_r.0 + 14.0,
                    input_cy,
                    theme::FONT_BODY,
                    if has_query {
                        p.text_accent
                    } else {
                        p.text_secondary
                    },
                    TextAlign::Left,
                    false,
                    "⌕",
                ));

                let query_text = if has_query {
                    reader.discover.query.as_str()
                } else {
                    "Buscar por título, autor o ID (ej. 2401.12345)…"
                };
                texts.push(CanvasText::new(
                    input_r.0 + 36.0,
                    input_cy,
                    theme::FONT_BODY,
                    if has_query {
                        p.text_primary
                    } else {
                        p.text_secondary
                    },
                    TextAlign::Left,
                    has_query,
                    query_text,
                ));

                if has_query {
                    draw_token_pill(
                        &mut rects,
                        &mut texts,
                        clear_r,
                        p.bg_muted,
                        Some(p.border),
                        p.text_secondary,
                        theme::FONT_CAPTION,
                        TextFont::Sans,
                        0.0,
                        true,
                        "✕",
                    );
                }

                draw_astryx_button(
                    &mut rects,
                    &mut texts,
                    search_btn_r,
                    ButtonVariant::Primary,
                    &p,
                    theme::FONT_BODY,
                    true,
                    "Buscar",
                );
            }
        }
    }

    // Franja de estado opcional
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
            p.text_accent,
            TextAlign::Left,
            true,
            msg,
        ));
    }

    jni_text_bitmap(w, h_fixed, p.bg_surface, &rects, &texts)
}

/// Render de la BANDA SCROLLEABLE de Discover sobre `bg_body`: feed de papers,
/// resultados de búsqueda, lista de áreas temáticas o ficha detallada.
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
                let top_y = (band_h as f32 * 0.24).clamp(36.0, 140.0);

                if let DiscoverPhase::Error(err) = &reader.discover.phase {
                    draw_discover_callout(
                        &mut rects,
                        &mut texts,
                        w,
                        top_y,
                        "ERROR DE RED",
                        p.error_bg,
                        p.error_text,
                        p.error_border,
                        "No se pudo conectar con arXiv",
                        err,
                        Some("Comprueba tu conexión a internet e inténtalo de nuevo."),
                        &p,
                    );
                } else if is_loading {
                    let title = if reader.discover.screen == DiscoverScreen::Feed {
                        "Cargando novedades de arXiv…"
                    } else {
                        "Buscando en arXiv…"
                    };
                    draw_discover_callout(
                        &mut rects,
                        &mut texts,
                        w,
                        top_y,
                        "SINCRONIZANDO",
                        p.accent_muted,
                        p.on_accent_muted,
                        p.border,
                        title,
                        "Obteniendo resúmenes y metadatos de preprints.",
                        None,
                        &p,
                    );
                } else if reader.discover.screen == DiscoverScreen::Feed {
                    draw_discover_callout(
                        &mut rects,
                        &mut texts,
                        w,
                        top_y,
                        "ARXIV FEED",
                        p.teal_bg,
                        p.teal_text,
                        p.border,
                        "Tu feed de arXiv está vacío",
                        "Elige tus disciplinas en la pestaña Áreas para recibir novedades.",
                        None,
                        &p,
                    );
                } else if reader.discover.query.is_empty() {
                    draw_discover_callout(
                        &mut rects,
                        &mut texts,
                        w,
                        top_y,
                        "ARXIV BUSCAR",
                        p.purple_bg,
                        p.purple_text,
                        p.border,
                        "Explora millones de papers en arXiv",
                        "Busca por tema, autor o identificador de arXiv.",
                        Some("Ejemplos: 2401.12345 · attention · vaswani · cs.AI"),
                        &p,
                    );
                } else {
                    let msg = format!("No hay resultados para «{}».", reader.discover.query);
                    draw_discover_callout(
                        &mut rects,
                        &mut texts,
                        w,
                        top_y,
                        "SIN RESULTADOS",
                        p.bg_muted,
                        p.text_secondary,
                        p.border,
                        "Sin coincidencias",
                        &msg,
                        Some("Prueba con términos más amplios o un identificador de arXiv."),
                        &p,
                    );
                }
            } else {
                // Lista de tarjetas de papers Astryx
                for (idx, entry) in entries.iter().enumerate() {
                    let card_rect = disc_card_rect(w, idx);
                    let top_in_band = card_rect.1 - band_origin as f32;
                    let bot_in_band = card_rect.3 - band_origin as f32;

                    if bot_in_band < 0.0 || top_in_band > band_h as f32 {
                        continue;
                    }

                    let card_l = card_rect.0;
                    let card_r = card_rect.2;
                    let card_w = card_r - card_l;
                    let card_band = (card_l, top_in_band, card_r, bot_in_band);

                    draw_surface_card(
                        &mut rects,
                        card_band,
                        theme::RADIUS_CONTAINER,
                        p.bg_card,
                        p.border,
                        Elevation::Low,
                        p.is_dark,
                    );

                    // Fila de badges semánticos Astryx
                    draw_paper_badges(
                        &mut rects,
                        &mut texts,
                        card_l + 16.0,
                        top_in_band + 14.0,
                        card_r - 16.0,
                        &entry.primary_category,
                        &p,
                    );

                    // Título adaptativo (1 o 2 líneas) con tracking ajustado
                    let title_max_chars =
                        (((card_w - 32.0) / (theme::FONT_TITLE * 0.52)).floor() as usize).max(20);
                    let title_lines = wrap_text_chars(&entry.title, title_max_chars);
                    let has_two_title_lines = title_lines.len() > 1;

                    if has_two_title_lines {
                        texts.push(
                            CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 58.0,
                                theme::FONT_TITLE,
                                p.text_primary,
                                TextAlign::Left,
                                true,
                                &title_lines[0],
                            )
                            .with_tracking(theme::TRACKING_TIGHT),
                        );
                        let line1_text = if title_lines.len() > 2 {
                            format!("{}…", title_lines[1].trim_end())
                        } else {
                            title_lines[1].clone()
                        };
                        texts.push(
                            CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 78.0,
                                theme::FONT_TITLE,
                                p.text_primary,
                                TextAlign::Left,
                                true,
                                line1_text,
                            )
                            .with_tracking(theme::TRACKING_TIGHT),
                        );
                    } else if let Some(first_line) = title_lines.first() {
                        texts.push(
                            CanvasText::new(
                                card_l + 16.0,
                                top_in_band + 60.0,
                                theme::FONT_TITLE,
                                p.text_primary,
                                TextAlign::Left,
                                true,
                                first_line,
                            )
                            .with_tracking(theme::TRACKING_TIGHT),
                        );
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
                        p.text_secondary,
                        TextAlign::Left,
                        false,
                        authors_display,
                    ));

                    // Snippet del resumen
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
                                p.text_primary,
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
                                p.text_primary,
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
                                p.text_primary,
                                TextAlign::Left,
                                false,
                                line1,
                            ));
                        }
                    }

                    // Metadatos monoespaciados en la esquina inferior izquierda
                    let pub_date = entry
                        .published
                        .split('T')
                        .next()
                        .unwrap_or(&entry.published);
                    texts.push(
                        CanvasText::new(
                            card_l + 16.0,
                            bot_in_band - 26.0,
                            theme::FONT_CAPTION,
                            p.text_secondary,
                            TextAlign::Left,
                            false,
                            format!("{pub_date}  ·  arXiv:{}", entry.id),
                        )
                        .with_font(TextFont::Mono),
                    );

                    // Botón de acción semántico Astryx en la esquina inferior derecha
                    let action_rect = disc_card_action_rect(card_band);
                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    let is_in_library = reader.is_arxiv_in_library(&entry.id);

                    if is_downloading {
                        let dl_label = if reader.discover.download_bytes > 0 {
                            format!(
                                "{:.1} MB",
                                reader.discover.download_bytes as f64 / 1_000_000.0
                            )
                        } else {
                            "Descargando…".to_string()
                        };
                        draw_astryx_button(
                            &mut rects,
                            &mut texts,
                            action_rect,
                            ButtonVariant::Tonal,
                            &p,
                            theme::FONT_BODY,
                            true,
                            &dl_label,
                        );
                    } else if is_in_library {
                        draw_astryx_button(
                            &mut rects,
                            &mut texts,
                            action_rect,
                            ButtonVariant::Primary,
                            &p,
                            theme::FONT_BODY,
                            true,
                            "Leer",
                        );
                    } else {
                        draw_astryx_button(
                            &mut rects,
                            &mut texts,
                            action_rect,
                            ButtonVariant::Secondary,
                            &p,
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
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        more_rect,
                        ButtonVariant::Secondary,
                        &p,
                        theme::FONT_BODY,
                        true,
                        if is_loading {
                            "Cargando…"
                        } else {
                            "Cargar más papers"
                        },
                    );
                }

                // Atribución requerida por arXiv
                let attr_y = more_y + 80.0;
                if attr_y > 0.0 && attr_y < band_h as f32 + 40.0 {
                    texts.push(CanvasText::new(
                        w as f32 * 0.5,
                        attr_y,
                        theme::FONT_CAPTION,
                        p.text_secondary,
                        TextAlign::Center,
                        false,
                        ARXIV_ATTRIBUTION,
                    ));
                }
            }
        }
        DiscoverScreen::Areas => {
            for (idx, cat) in ARXIV_CATEGORIES.iter().enumerate() {
                let row_rect = disc_cat_row_rect(w, idx);
                let top_in_band = row_rect.1 - band_origin as f32;
                let bot_in_band = row_rect.3 - band_origin as f32;

                if bot_in_band < 0.0 || top_in_band > band_h as f32 {
                    continue;
                }

                let row_l = row_rect.0;
                let row_r = row_rect.2;
                let row_band = (row_l, top_in_band, row_r, bot_in_band);
                let is_selected = reader.discover.selected_cats.iter().any(|c| c == cat.code);

                if is_selected {
                    draw_elevation_shadow(
                        &mut rects,
                        row_band,
                        theme::RADIUS_CONTAINER,
                        Elevation::Low,
                        p.is_dark,
                    );
                    draw_inset_ring(
                        &mut rects,
                        row_band,
                        theme::RADIUS_CONTAINER,
                        p.accent,
                        p.bg_card,
                    );
                } else {
                    draw_surface_card(
                        &mut rects,
                        row_band,
                        theme::RADIUS_CONTAINER,
                        p.bg_card,
                        p.border,
                        Elevation::None,
                        p.is_dark,
                    );
                }

                // Token monoespaciado del código de categoría
                let code_h = 26.0f32;
                let code_w = (cat.code.chars().count() as f32 * 8.2 + 18.0).max(56.0);
                let code_t = top_in_band + (bot_in_band - top_in_band - code_h) * 0.5;
                let code_l = row_l + 14.0;
                let code_r = code_l + code_w;

                draw_token_pill(
                    &mut rects,
                    &mut texts,
                    (code_l, code_t, code_r, code_t + code_h),
                    if is_selected {
                        p.accent_muted
                    } else {
                        p.bg_muted
                    },
                    if is_selected { None } else { Some(p.border) },
                    if is_selected {
                        p.on_accent_muted
                    } else {
                        p.text_secondary
                    },
                    theme::FONT_LABEL_CAPS,
                    TextFont::Mono,
                    0.0,
                    true,
                    cat.code,
                );

                // Nombre y grupo temático
                let label_x = code_r + 14.0;
                texts.push(
                    CanvasText::new(
                        label_x,
                        top_in_band + 22.0,
                        theme::FONT_BODY,
                        p.text_primary,
                        TextAlign::Left,
                        is_selected,
                        cat.label_es,
                    )
                    .with_tracking(theme::TRACKING_TIGHT),
                );
                texts.push(CanvasText::new(
                    label_x,
                    top_in_band + 40.0,
                    theme::FONT_CAPTION,
                    p.text_secondary,
                    TextAlign::Left,
                    false,
                    cat.group,
                ));

                // Acción derecha Astryx
                let chip_w = 96.0f32;
                let chip_h = 32.0f32;
                let chip_r = row_r - 14.0;
                let chip_l = chip_r - chip_w;
                let chip_t = top_in_band + (bot_in_band - top_in_band - chip_h) * 0.5;
                let chip_rect = (chip_l, chip_t, chip_r, chip_t + chip_h);

                if is_selected {
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        chip_rect,
                        ButtonVariant::Tonal,
                        &p,
                        theme::FONT_CAPTION,
                        true,
                        "✓ Activa",
                    );
                } else {
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        chip_rect,
                        ButtonVariant::Secondary,
                        &p,
                        theme::FONT_CAPTION,
                        false,
                        "+ Añadir",
                    );
                }
            }

            let total_cats_h = 14.0 + ARXIV_CATEGORIES.len() as f32 * disc_cat_row_h();
            let attr_y = total_cats_h - band_origin as f32 + 30.0;
            if attr_y > 0.0 && attr_y < band_h as f32 + 40.0 {
                texts.push(CanvasText::new(
                    w as f32 * 0.5,
                    attr_y,
                    theme::FONT_CAPTION,
                    p.text_secondary,
                    TextAlign::Center,
                    false,
                    ARXIV_ATTRIBUTION,
                ));
            }
        }
        DiscoverScreen::Detail => {
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

                // Tarjeta 1: Metadatos y CTA (Hero Astryx)
                draw_surface_card(
                    &mut rects,
                    (pad, hero_top, pad + card_w, hero_bot),
                    theme::RADIUS_CONTAINER,
                    p.bg_card,
                    p.border,
                    Elevation::Low,
                    p.is_dark,
                );

                draw_paper_badges(
                    &mut rects,
                    &mut texts,
                    pad + inner_pad,
                    badge_y,
                    pad + card_w - inner_pad,
                    &entry.primary_category,
                    &p,
                );

                let mut cur_y = title_y;
                for line in &title_lines {
                    texts.push(
                        CanvasText::new(
                            pad + inner_pad,
                            cur_y,
                            theme::FONT_TITLE,
                            p.text_primary,
                            TextAlign::Left,
                            true,
                            line,
                        )
                        .with_tracking(theme::TRACKING_TIGHT),
                    );
                    cur_y += 26.0;
                }

                cur_y = authors_y;
                for line in &authors_lines {
                    texts.push(CanvasText::new(
                        pad + inner_pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.text_secondary,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 22.0;
                }

                rects.push(CanvasRect::sharp(
                    pad + inner_pad,
                    divider_y,
                    pad + card_w - inner_pad,
                    divider_y + 1.0,
                    p.border,
                ));

                let pub_date = entry
                    .published
                    .split('T')
                    .next()
                    .unwrap_or(&entry.published);
                let up_date = entry.updated.split('T').next().unwrap_or(&entry.updated);
                let meta_str = format!(
                    "arXiv:{}  ·  Publicado {}  ·  Actualizado {}",
                    entry.id, pub_date, up_date
                );
                texts.push(
                    CanvasText::new(
                        pad + inner_pad,
                        meta_y,
                        theme::FONT_CAPTION,
                        p.text_secondary,
                        TextAlign::Left,
                        false,
                        meta_str,
                    )
                    .with_font(TextFont::Mono),
                );

                let is_downloading =
                    reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                let is_in_library = reader.is_arxiv_in_library(&entry.id);

                if is_downloading {
                    let dl_label = if reader.discover.download_bytes > 0 {
                        format!(
                            "Descargando ({:.1} MB)…",
                            reader.discover.download_bytes as f64 / 1_000_000.0
                        )
                    } else {
                        "Descargando PDF…".to_string()
                    };
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        action_btn,
                        ButtonVariant::Tonal,
                        &p,
                        theme::FONT_BODY,
                        true,
                        &dl_label,
                    );
                } else if is_in_library {
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        action_btn,
                        ButtonVariant::Primary,
                        &p,
                        theme::FONT_BODY,
                        true,
                        "Abrir en el lector",
                    );
                } else {
                    draw_astryx_button(
                        &mut rects,
                        &mut texts,
                        action_btn,
                        ButtonVariant::Primary,
                        &p,
                        theme::FONT_BODY,
                        true,
                        "⬇ Descargar PDF",
                    );
                }

                // Tarjeta 2: Resumen con kicker uppercase Astryx
                let abstract_top = hero_bot + 16.0;
                let abstract_header_y = abstract_top + 26.0;
                let abstract_body_y = abstract_header_y + 26.0;
                let abstract_bot = abstract_body_y + abstract_lines.len() as f32 * 22.0 + 20.0;

                draw_surface_card(
                    &mut rects,
                    (pad, abstract_top, pad + card_w, abstract_bot),
                    theme::RADIUS_CONTAINER,
                    p.bg_card,
                    p.border,
                    Elevation::Low,
                    p.is_dark,
                );

                texts.push(
                    CanvasText::new(
                        pad + inner_pad,
                        abstract_header_y,
                        theme::FONT_LABEL_CAPS,
                        p.text_secondary,
                        TextAlign::Left,
                        true,
                        "RESUMEN",
                    )
                    .with_tracking(theme::TRACKING_CAPS),
                );

                rects.push(CanvasRect::sharp(
                    pad + inner_pad,
                    abstract_header_y + 10.0,
                    pad + card_w - inner_pad,
                    abstract_header_y + 11.0,
                    p.border,
                ));

                cur_y = abstract_body_y;
                for line in &abstract_lines {
                    texts.push(CanvasText::new(
                        pad + inner_pad,
                        cur_y,
                        theme::FONT_BODY,
                        p.text_primary,
                        TextAlign::Left,
                        false,
                        line,
                    ));
                    cur_y += 22.0;
                }

                let attr_y = abstract_bot + 36.0;
                texts.push(CanvasText::new(
                    w as f32 * 0.5,
                    attr_y,
                    theme::FONT_CAPTION,
                    p.text_secondary,
                    TextAlign::Center,
                    false,
                    ARXIV_ATTRIBUTION,
                ));
            } else {
                texts.push(CanvasText::new(
                    w as f32 * 0.5,
                    60.0,
                    theme::FONT_BODY,
                    p.text_secondary,
                    TextAlign::Center,
                    false,
                    "Ningún paper seleccionado.",
                ));
            }
        }
    }

    jni_text_bitmap(w, band_h, p.bg_body, &rects, &texts)
}
