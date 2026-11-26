// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Input de la BIBLIOTECA: `library_tap` (cabecera, búsqueda, chips y
//! rejilla) y `library_down_zone` (qué fila arrastra en horizontal).

use crate::android_activity::AndroidApp;
use crate::draw::{SettingsMenuItem, ViewMenuItem, settings_menu_geometry, view_menu_geometry};
use crate::jni::launch_all_files_settings;
use crate::reader::{
    BookStatus, LibSort, LibraryCoverFit, LibraryGroupBy, LibraryViewMode, Reader, grid_cell_h,
    grid_cell_w, grid_gap, grid_pad, lib_add_btn_rect, lib_chip_h, lib_chips, lib_collection_chips,
    lib_content_y0, lib_empty_state_geom, lib_grid_y0, lib_header_h, lib_org_block_h,
    lib_org_chip_h, lib_org_chips, lib_search_chips_y0, lib_search_h, lib_search_panel_h,
    lib_tabs_rect, list_row_gap, list_row_h, settings_menu_button_rect, view_menu_button_rect,
};
use std::time::Instant;

/// Tap de la biblioteca (biblioteca personal premium): botón "＋ Add book"
/// de la cabecera, campo de búsqueda (toggle del panel de chips + "✕"),
/// chips del panel de búsqueda (fila 0 = letras A-Z/#, fila 1 = carpetas),
/// chips de organización (sort/filter) o celda de la rejilla (abre el libro).
/// La geometría DEBE reflejar exactamente la de `render_library_zone`
/// (mismas fórmulas: `lib_chips`, `lib_content_y0`, `lib_grid_cell_rect`,
/// `lib_org_chips`).
pub(super) fn library_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    // Pestaña Descubrir en la cabecera
    let (_tab_lib, tab_disc) = lib_tabs_rect(reader.win_w, reader.win_h);
    if x >= tab_disc.0 && x <= tab_disc.2 && y >= tab_disc.1 && y <= tab_disc.3 {
        reader.enter_discover(app);
        return;
    }

    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;

    let (vl, vt, vr, vb) = view_menu_button_rect(reader.win_w, reader.win_h);
    let (sl, st, sr, sb) = settings_menu_button_rect(reader.win_w, reader.win_h);
    let hit_view = x >= vl && x < vr && y >= vt && y < vb;
    let hit_settings = x >= sl && x < sr && y >= st && y < sb;

    // ViewMenu dropdown abierto: procesar tap en ítems o cerrar
    if reader.view_menu_open {
        let (_card_rect, items) = view_menu_geometry(reader.win_w, reader.win_h);
        let mut handled = false;
        for (item, rect) in items {
            if x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3 {
                match item {
                    ViewMenuItem::Grid => {
                        reader.view_mode = LibraryViewMode::Grid;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::List => {
                        reader.view_mode = LibraryViewMode::List;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::ColumnsAuto => {
                        reader.auto_columns = !reader.auto_columns;
                        reader.save_state();
                    }
                    ViewMenuItem::ColumnsDec => {
                        reader.auto_columns = false;
                        reader.columns = reader.columns.saturating_sub(1).clamp(1, 4);
                        reader.save_state();
                    }
                    ViewMenuItem::ColumnsInc => {
                        reader.auto_columns = false;
                        reader.columns = (reader.columns + 1).clamp(1, 4);
                        reader.save_state();
                    }
                    ViewMenuItem::CoverCrop => {
                        reader.cover_fit = LibraryCoverFit::Crop;
                        reader.hide_covers = false;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::CoverFit => {
                        reader.cover_fit = LibraryCoverFit::Fit;
                        reader.hide_covers = false;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::CoverHide => {
                        reader.hide_covers = !reader.hide_covers;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::GroupNone => {
                        reader.group_by = LibraryGroupBy::None;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::GroupAuthor => {
                        reader.group_by = LibraryGroupBy::Author;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortTitle => {
                        reader.library.lib_sort = LibSort::Title;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAuthor => {
                        reader.library.lib_sort = LibSort::Author;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAdded => {
                        reader.library.lib_sort = LibSort::RecentlyAdded;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortRead => {
                        reader.library.lib_sort = LibSort::RecentlyRead;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortProgress => {
                        reader.library.lib_sort = LibSort::Progress;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                }
                handled = true;
                break;
            }
        }
        if handled {
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        if hit_view {
            reader.view_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        reader.view_menu_open = false;
        reader.list_dirty = true;
        reader.redraw();
        return;
    }

    // SettingsMenu dropdown abierto: procesar tap en ítems o cerrar
    if reader.settings_menu_open {
        let (_card_rect, items) = settings_menu_geometry(reader.win_w, reader.win_h);
        let mut handled = false;
        for (item, rect) in items {
            if x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3 {
                match item {
                    SettingsMenuItem::CoverSizeSmall => {
                        reader.cover_size = 0;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverSizeMedium => {
                        reader.cover_size = 1;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverSizeLarge => {
                        reader.cover_size = 2;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverProgress => {
                        reader.cover_progress = !reader.cover_progress;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::ClearLibrary => {
                        let now = Instant::now();
                        if let Some(until) = reader.clear_confirm_until
                            && now <= until
                        {
                            reader.clear_confirm_until = None;
                            reader.clear_library(app);
                            return;
                        }
                        reader.clear_confirm_until = Some(now + std::time::Duration::from_secs(3));
                        reader.list_dirty = true;
                        reader.redraw();
                        return;
                    }
                }
                handled = true;
                break;
            }
        }
        if handled {
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        if hit_settings {
            reader.settings_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        reader.settings_menu_open = false;
        reader.list_dirty = true;
        reader.redraw();
        return;
    }
    for (label, (l, t, r, b), _, subject_row) in lib_collection_chips(reader) {
        if x >= l && x < r && y >= t && y < b {
            reader.lib_close_ime(app);
            if subject_row {
                reader.lib_select_master(true, if label == "Todas" { None } else { Some(label) });
            } else {
                reader.lib_select_master(label == "Máster", None);
            }
            return;
        }
    }

    if y < header_h {
        // "⋯" View: alterna su dropdown y cierra el de Settings.
        if hit_view {
            reader.view_menu_open = !reader.view_menu_open;
            reader.settings_menu_open = false;
            reader.list_dirty = true; // re-render de la cabecera (highlight)
            reader.redraw();
            return;
        }
        // "☰" Settings: alterna su dropdown y cierra el de View.
        if hit_settings {
            reader.settings_menu_open = !reader.settings_menu_open;
            reader.view_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        // Mismo rect que el render (`lib_add_btn_rect`).
        let (bl, bt, br, bb) = lib_add_btn_rect(reader.win_w, reader.win_h);
        if x >= bl && x < br && y >= bt && y < bb {
            reader.add_book(app);
        }
        return;
    }

    // CAMPO de búsqueda: la "✕" limpia el texto tecleado y cierra el
    // teclado; tocar el campo abre el TECLADO del sistema (`jni::ime_*`).
    if y < search_y + search_hh {
        let field_right = reader.win_w as f32 - grid_pad(reader.win_w);
        let has_filter = !reader.library.lib_query.is_empty()
            || reader.library.lib_letter.is_some()
            || reader.library.lib_folder.is_some();
        if has_filter {
            let xw = search_hh - 8.0;
            let xx = field_right - 14.0 - xw;
            if x >= xx && x < xx + xw {
                reader.lib_clear_search(app);
                return;
            }
        }
        reader.lib_open_keyboard(app);
        return;
    }

    // PANEL de búsqueda desplegado: fila 0 = letras A-Z/#, fila 1 = carpetas.
    // La zona usa la MISMA geometría que el render (`lib_search_chips_y0/1`,
    // donde `lib_chips` coloca los chips) y que `lib_down_zone`; antes usaba
    // `panel_top = header_h + search_h` (6 px por encima de la fila real), de
    // modo que un tap en el borde superior del panel caía fuera de los chips.
    let panel_top = lib_search_chips_y0(reader);
    let panel_h = lib_search_panel_h(reader.win_h, reader.library.lib_search_open);
    if reader.library.lib_search_open && y >= panel_top && y < panel_top + panel_h {
        let row = if y < lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) {
            0
        } else {
            1
        };
        for (label, (l, t, r, b), _active) in lib_chips(reader, row) {
            if x >= l && x < r && y >= t && y < b {
                if row == 0 {
                    if label == "All" {
                        reader.lib_set_letter(None);
                    } else {
                        reader.lib_set_letter(label.chars().next());
                    }
                } else if label == "All" {
                    reader.lib_set_folder(None);
                } else {
                    reader.lib_set_folder(Some(label.clone()));
                }
                return;
            }
        }
        return;
    }

    // Franja de estado: no es seleccionable.
    let content_y0 = lib_content_y0(reader) as f32;
    if y < content_y0 {
        return;
    }

    // Contenido scrolleable: pasar a coordenadas de CONTENIDO (y del Down +
    // scroll vertical).
    let yc = y - content_y0 + reader.library.lib_scroll;
    let win_w = reader.win_w;

    // EMPTY STATE: botón "Add PDF"/"Grant access" (misma geometría que el
    // render).
    if reader.library_list.is_empty() && !reader.library.lib_master {
        if let Some(g) = lib_empty_state_geom(reader) {
            let (l, t, r, b) = g.button;
            if x >= l && x < r && y >= t && y < b {
                if reader.permission_granted {
                    reader.add_book(app);
                } else {
                    reader.grant_pending = true;
                    launch_all_files_settings(app);
                }
            }
        }
        return;
    }

    // Título de "My Library": no seleccionable. Tras él, el bloque de
    // ORGANIZACIÓN (chips de sort/filter) antes de la rejilla.
    let grid_y0 = lib_grid_y0(win_w, reader.win_h);
    if yc < grid_y0 {
        let org_top = grid_y0 - lib_org_block_h(reader.win_h);
        if yc >= org_top {
            let row = if yc < org_top + lib_org_chip_h(reader.win_h) {
                0
            } else {
                1
            };
            for (label, (l, t, r, b), _active) in lib_org_chips(reader, row) {
                if x >= l && x < r && y >= t && y < b {
                    if row == 0 {
                        reader.lib_set_sort(match label.as_str() {
                            "Recently Read" | "Leídos" => LibSort::RecentlyRead,
                            "Title" | "Título" => LibSort::Title,
                            "Author" | "Autor" => LibSort::Author,
                            _ => LibSort::RecentlyAdded,
                        });
                    } else {
                        reader.lib_set_status(match label.as_str() {
                            "Reading" | "En lectura" => Some(BookStatus::Reading),
                            "Finished" | "Terminados" => Some(BookStatus::Finished),
                            "Unread" | "Por leer" => Some(BookStatus::Unread),
                            _ => None,
                        });
                    }
                    return;
                }
            }
        }
        return;
    }

    if reader.is_grid() {
        let cols = reader.effective_grid_cols();
        let row = ((yc - grid_y0) / grid_cell_h(win_w, cols, reader.cover_size)) as usize;
        let cell_w = grid_cell_w(win_w, cols);
        let pad = grid_pad(win_w);
        let col = ((x - pad) / (cell_w + grid_gap(win_w))).floor() as usize;
        if col < cols
            && let Some(entry) = reader.grid_entry_at(row, col)
        {
            let entry = entry.clone();
            if !reader.open_library_entry(app, &entry) {
                reader.status = Some(format!("Cannot open {}", entry.name));
                reader.list_dirty = true;
                reader.redraw();
            }
        }
    } else {
        let pad = grid_pad(win_w);
        if x >= pad && x < win_w as f32 - pad {
            let row_h = list_row_h(reader.win_h, reader.cover_size);
            let row_gap = list_row_gap();
            let total_row_h = row_h + row_gap;
            let rel_y = yc - grid_y0;
            if rel_y >= 0.0 {
                let idx = (rel_y / total_row_h).floor() as usize;
                let in_row_y = rel_y - idx as f32 * total_row_h;
                if in_row_y <= row_h
                    && let Some(entry) = reader.list_entry_at(idx)
                {
                    let entry = entry.clone();
                    if !reader.open_library_entry(app, &entry) {
                        reader.status = Some(format!("Cannot open {}", entry.name));
                        reader.list_dirty = true;
                        reader.redraw();
                    }
                }
            }
        }
    }
}

/// Zona de la biblioteca donde cayó el Down (qué arrastra en HORIZONTAL):
/// 0 = contenido (scroll vertical), 2 = fila de chips de LETRAS (búsqueda),
/// 3 = fila de chips de CARPETAS (búsqueda), 4 = fila de chips de SORT,
/// 5 = fila de chips de FILTER.
/// Misma geometría que `library_tap` y `render_library_zone`.
pub(super) fn library_down_zone(reader: &Reader, y: f32) -> u8 {
    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;
    if y < search_y + search_hh {
        return 0; // cabecera + campo de búsqueda: sin arrastre horizontal
    }
    // Panel de búsqueda desplegado: fila 0 = letras (2), fila 1 = carpetas (3).
    if reader.library.lib_search_open {
        let panel_top = search_y + search_hh + 6.0;
        let panel_h = lib_search_panel_h(reader.win_h, true);
        if y >= panel_top && y < panel_top + panel_h {
            return if y < lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) {
                2
            } else {
                3
            };
        }
    }
    // Contenido
    let content_y0 = lib_content_y0(reader) as f32;
    let yc = y - content_y0 + reader.library.lib_scroll;
    // Organización: fila SORT (4) / FILTER (5).
    let grid_y0 = lib_grid_y0(reader.win_w, reader.win_h);
    let org_top = grid_y0 - lib_org_block_h(reader.win_h);
    if yc >= org_top && yc < grid_y0 {
        return if yc < org_top + lib_org_chip_h(reader.win_h) {
            4
        } else {
            5
        };
    }
    0
}
