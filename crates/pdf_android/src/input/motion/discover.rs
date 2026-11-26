// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Input de DESCUBRIR (arXiv): `discover_tap` (cabecera, sub-tabs, buscador,
//! tarjetas, áreas y ficha).

use crate::android_activity::AndroidApp;
use crate::reader::discover_categories::ARXIV_CATEGORIES;
use crate::reader::discover_state::{DiscoverScreen, DiscoverTab};
use crate::reader::{
    Reader, disc_card_action_rect, disc_card_gap, disc_card_h, disc_card_rect, disc_cat_row_rect,
    disc_content_y0, disc_detail_back_rect, disc_detail_layout, disc_more_btn_rect,
    disc_search_rect, disc_subtabs_rect, lib_tabs_rect,
};

/// Tap en la pantalla de Descubrir (arXiv Discover): cabecera (tabs Biblioteca/Descubrir,
/// sub-tabs Feed/Buscar/Áreas, buscador) y contenido (tarjetas de papers, áreas o ficha).
pub(super) fn discover_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let content_y0 = disc_content_y0(
        reader.win_h,
        reader.discover.screen,
        reader.status.is_some() || reader.discover.status_msg.is_some(),
    );

    // 1. Cabecera fija
    if y < content_y0 as f32 {
        // En Ficha: botón [← Volver]
        if reader.discover.screen == DiscoverScreen::Detail {
            let back_rect = disc_detail_back_rect(reader.win_w, reader.win_h);
            if x >= back_rect.0 && x <= back_rect.2 && y >= back_rect.1 && y <= back_rect.3 {
                reader.discover.screen = match reader.discover.active_tab {
                    DiscoverTab::Search => DiscoverScreen::Search,
                    _ => DiscoverScreen::Feed,
                };
                reader.discover.dirty = true;
                reader.redraw();
                return;
            }
        }

        // Pestañas principales: [Biblioteca] y [Descubrir]
        let (tab_lib, _tab_disc) = lib_tabs_rect(reader.win_w, reader.win_h);
        if x >= tab_lib.0 && x <= tab_lib.2 && y >= tab_lib.1 && y <= tab_lib.3 {
            reader.exit_discover(app);
            return;
        }

        // Sub-pestañas: [Feed] [Buscar] [Áreas]
        let subtabs = disc_subtabs_rect(reader.win_w, reader.win_h);
        for (tab, rect) in subtabs {
            if x >= rect.0 && x <= rect.2 && y >= rect.1 && y <= rect.3 {
                if tab != reader.discover.active_tab
                    || reader.discover.screen == DiscoverScreen::Detail
                {
                    reader.discover.active_tab = tab;
                    reader.discover.screen = match tab {
                        DiscoverTab::Feed => DiscoverScreen::Feed,
                        DiscoverTab::Search => DiscoverScreen::Search,
                        DiscoverTab::Areas => DiscoverScreen::Areas,
                    };
                    reader.discover.dirty = true;
                    if tab == DiscoverTab::Feed && reader.discover.feed_entries.is_empty() {
                        reader.discover_refresh_feed();
                    }
                    reader.redraw();
                }
                return;
            }
        }

        // Barra de búsqueda en pantalla Search
        if reader.discover.screen == DiscoverScreen::Search {
            let (input_r, clear_r, search_btn_r) = disc_search_rect(reader.win_w, reader.win_h);
            if !reader.discover.query.is_empty()
                && x >= clear_r.0
                && x <= clear_r.2
                && y >= clear_r.1
                && y <= clear_r.3
            {
                reader.discover.query.clear();
                crate::jni::ime_set_text(app, "");
                reader.discover.dirty = true;
                reader.redraw();
                return;
            }
            if x >= search_btn_r.0
                && x <= search_btn_r.2
                && y >= search_btn_r.1
                && y <= search_btn_r.3
            {
                let q = reader.discover.query.clone();
                reader.lib_close_ime(app);
                reader.discover_search(&q);
                return;
            }
            if x >= input_r.0 && x <= input_r.2 && y >= input_r.1 && y <= input_r.3 {
                reader.discover_open_keyboard(app);
                return;
            }
        }
        return;
    }

    // 2. Contenido scrolleable
    let scroll = reader.discover_scroll();
    let cy = y - content_y0 as f32 + scroll;

    match reader.discover.screen {
        DiscoverScreen::Feed | DiscoverScreen::Search => {
            let entries = if reader.discover.screen == DiscoverScreen::Feed {
                reader.discover.feed_entries.clone()
            } else {
                reader.discover.search_entries.clone()
            };

            for (idx, entry) in entries.iter().enumerate() {
                let card_rect = disc_card_rect(reader.win_w, idx);
                if x >= card_rect.0 && x <= card_rect.2 && cy >= card_rect.1 && cy <= card_rect.3 {
                    let action_rect = disc_card_action_rect(card_rect);
                    let hit_action = x >= action_rect.0
                        && x <= action_rect.2
                        && cy >= action_rect.1
                        && cy <= action_rect.3;

                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    if hit_action && is_downloading {
                        reader.discover.cancel();
                        reader.show_toast("Descarga cancelada");
                        reader.redraw();
                        return;
                    }

                    if hit_action && let Some(local_entry) = reader.find_arxiv_in_library(&entry.id)
                    {
                        reader.open_library_entry(app, &local_entry);
                        return;
                    }
                    // Tocar tarjeta o botón "Ficha": abrir ficha detallada
                    reader.discover.selected_entry = Some(entry.clone());
                    reader.discover.screen = DiscoverScreen::Detail;
                    reader.discover.detail_scroll = 0.0;
                    reader.discover.dirty = true;
                    reader.redraw();
                    return;
                }
            }

            // Botón "Cargar más"
            let has_more = if reader.discover.screen == DiscoverScreen::Feed {
                reader.discover.feed_has_more
            } else {
                reader.discover.search_has_more
            };
            if has_more {
                let last_bottom = 16.0 + entries.len() as f32 * (disc_card_h() + disc_card_gap());
                let more_rect = disc_more_btn_rect(reader.win_w, last_bottom);
                if x >= more_rect.0 && x <= more_rect.2 && cy >= more_rect.1 && cy <= more_rect.3 {
                    reader.discover_more();
                }
            }
        }
        DiscoverScreen::Areas => {
            for (idx, cat) in ARXIV_CATEGORIES.iter().enumerate() {
                let row_rect = disc_cat_row_rect(reader.win_w, idx);
                if x >= row_rect.0 && x <= row_rect.2 && cy >= row_rect.1 && cy <= row_rect.3 {
                    reader
                        .discover
                        .toggle_category(cat.code, reader.internal_dir.as_deref());
                    reader.discover_refresh_feed();
                    return;
                }
            }
        }
        DiscoverScreen::Detail => {
            if let Some(entry) = reader.discover.selected_entry.clone() {
                let (action_rect, _) = disc_detail_layout(reader.win_w, &entry);
                if x >= action_rect.0
                    && x <= action_rect.2
                    && cy >= action_rect.1
                    && cy <= action_rect.3
                {
                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    if is_downloading {
                        reader.discover.cancel();
                        reader.show_toast("Descarga cancelada");
                        reader.redraw();
                    } else if let Some(local_entry) = reader.find_arxiv_in_library(&entry.id) {
                        reader.open_library_entry(app, &local_entry);
                    } else {
                        let id = entry.id.clone();
                        reader.discover_download(&id, Some(entry));
                    }
                }
            }
        }
    }
}
