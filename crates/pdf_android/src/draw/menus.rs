// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Menús desplegables del visor (⋯ View / ☰ Settings) y lista del
//! picker: ítems, geometría compartida con `input` y renders Canvas+JNI.

use crate::reader::{
    LibSort, LibraryCoverFit, LibraryGroupBy, PickRow, PickerKind, Reader, human_size,
    picker_btn_h, picker_btn_w, picker_header_h, picker_row_h, settings_menu_button_rect,
    truncate_name, view_menu_button_rect,
};
use crate::theme;
use pdf_core::Bitmap;

use super::{
    ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign, TextFont, draw_astryx_button,
    draw_surface_card, draw_token_pill, jni_text_bitmap,
};

/// Dibuja un encabezado de sección uppercase Astryx dentro de un popover.
fn draw_menu_section(
    texts: &mut Vec<CanvasText>,
    x: f32,
    y: f32,
    label: &str,
    p: &theme::ThemePalette,
) {
    texts.push(
        CanvasText::new(
            x,
            y + theme::FONT_LABEL_CAPS * 0.85,
            theme::FONT_LABEL_CAPS,
            p.text_secondary,
            TextAlign::Left,
            true,
            label,
        )
        .with_tracking(theme::TRACKING_CAPS),
    );
}

/// Dibuja una fila seleccionable de menú Astryx (resaltado tonal `accent_muted` + marca `✓`).
#[allow(clippy::too_many_arguments)]
fn draw_menu_row(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    ml: f32,
    mr: f32,
    pad_x: f32,
    y: f32,
    item_h: f32,
    label: &str,
    active: bool,
    p: &theme::ThemePalette,
) {
    if active {
        rects.push(CanvasRect::rounded(
            ml + pad_x - 6.0,
            y + 2.0,
            mr - pad_x + 6.0,
            y + item_h - 2.0,
            theme::RADIUS_ELEMENT,
            p.accent_muted,
        ));
        texts.push(CanvasText::new(
            mr - pad_x - 2.0,
            y + item_h * 0.68,
            theme::FONT_BODY,
            p.on_accent_muted,
            TextAlign::Right,
            true,
            "✓",
        ));
    }
    let fg = if active {
        p.on_accent_muted
    } else {
        p.text_primary
    };
    texts.push(CanvasText::new(
        ml + pad_x,
        y + item_h * 0.68,
        theme::FONT_BODY,
        fg,
        TextAlign::Left,
        active,
        label,
    ));
}

/// Hairline divisor horizontal dentro de un popover Astryx.
fn draw_menu_divider(
    rects: &mut Vec<CanvasRect>,
    ml: f32,
    mr: f32,
    pad_x: f32,
    y: f32,
    p: &theme::ThemePalette,
) {
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.border,
    ));
}

/// Renderiza la lista del picker de archivos en lenguaje Astryx.
pub(crate) fn render_picker_list(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    let row_h = picker_row_h(h);
    let header_h = picker_header_h(h);
    let status_h = if reader.status.is_some() { row_h } else { 0 };
    let btn_w = picker_btn_w(w);
    let btn_h = picker_btn_h(h);
    let pad = (w / 32).max(8) as f32;
    let p = reader.theme.palette();

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Cabecera elevada `bg_surface` + hairline `border`
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        header_h as f32,
        p.bg_surface,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        header_h as f32 - 1.0,
        w as f32,
        header_h as f32,
        p.border,
    ));

    let selecting = reader.picker_kind == PickerKind::Select;
    texts.push(
        CanvasText::new(
            pad,
            header_h as f32 * 0.62,
            theme::FONT_TITLE,
            p.text_primary,
            TextAlign::Left,
            true,
            if selecting {
                "Seleccionar PDF"
            } else {
                "Abrir PDF"
            },
        )
        .with_tracking(theme::TRACKING_TIGHT),
    );

    let btn_y = (header_h - btn_h) as f32 * 0.5;
    let back_x = w as f32 - btn_w as f32 * 2.0 - 16.0;
    let rescan_x = w as f32 - btn_w as f32 - 8.0;

    if reader.doc.is_some() || selecting {
        draw_astryx_button(
            &mut rects,
            &mut texts,
            (back_x, btn_y, back_x + btn_w as f32, btn_y + btn_h as f32),
            ButtonVariant::Secondary,
            &p,
            theme::FONT_CAPTION,
            true,
            "Atrás",
        );
    }
    draw_astryx_button(
        &mut rects,
        &mut texts,
        (
            rescan_x,
            btn_y,
            rescan_x + btn_w as f32,
            btn_y + btn_h as f32,
        ),
        ButtonVariant::Primary,
        &p,
        theme::FONT_CAPTION,
        true,
        "Reescanear",
    );

    // Franja de estado semántica
    let crumbs = if reader.picker_has_crumb() { row_h } else { 0 };
    let rows_y0 = header_h + status_h + crumbs;
    if let Some(status) = reader.status.as_deref() {
        rects.push(CanvasRect::sharp(
            0.0,
            header_h as f32,
            w as f32,
            (header_h + status_h) as f32,
            p.status_bg(),
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            header_h as f32,
            4.0,
            (header_h + status_h) as f32,
            p.accent,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            (header_h + status_h) as f32 - 1.0,
            w as f32,
            (header_h + status_h) as f32,
            p.status_border(),
        ));
        texts.push(CanvasText::new(
            pad,
            header_h as f32 + row_h as f32 * 0.62,
            theme::FONT_CAPTION,
            p.status_text(),
            TextAlign::Left,
            true,
            status,
        ));
    }

    // Barra de breadcrumb (tocar sube un nivel)
    if reader.picker_has_crumb() {
        let y0 = header_h as f32 + status_h as f32;
        rects.push(CanvasRect::sharp(
            0.0,
            y0,
            w as f32,
            y0 + row_h as f32,
            p.bg_muted,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            y0 + row_h as f32 - 1.0,
            w as f32,
            y0 + row_h as f32,
            p.border,
        ));
        texts.push(
            CanvasText::new(
                pad,
                y0 + row_h as f32 * 0.64,
                theme::FONT_CAPTION,
                p.text_accent,
                TextAlign::Left,
                true,
                format!("↑  /{}", reader.sel_dir.join("/")),
            )
            .with_font(TextFont::Mono),
        );
    }

    // Filas del gestor con badges tipográficos Astryx (`DIR` / `PDF`) en lugar de emojis
    let rows: Vec<PickRow> = reader.picker_rows();
    let visible = reader.picker_visible();
    let row_ts = theme::FONT_BODY;
    let badge_w = 40.0f32;
    let badge_h = 20.0f32;

    for i in 0..visible {
        let r = reader.list_scroll + i;
        let (is_folder, label, size_str) = if selecting {
            match rows.get(r) {
                Some(PickRow::Folder(name)) => (true, name.clone(), String::new()),
                Some(PickRow::File(idx)) => {
                    let Some(e) = reader.select_list.get(*idx) else {
                        break;
                    };
                    let sz = if e.size > 0 {
                        human_size(e.size.max(0) as u64)
                    } else {
                        String::new()
                    };
                    (false, truncate_name(&e.name, 48), sz)
                }
                None => break,
            }
        } else {
            let Some(entry) = reader.pdf_list.get(r) else {
                break;
            };
            let size_str = human_size(entry.size);
            let char_w = row_ts * 0.55;
            let size_w = size_str.chars().count() as f32 * char_w + pad;
            let max_chars =
                (((w as f32 - pad * 3.0 - badge_w - 14.0 - size_w) / char_w) as usize).max(1);
            (
                false,
                format!(
                    "{}  [{}]",
                    truncate_name(&entry.name, max_chars),
                    entry.source
                ),
                size_str,
            )
        };

        let y0 = (rows_y0 + (i as i32) * row_h) as f32;
        let bg = if i % 2 == 0 { p.bg_card } else { p.bg_body };
        rects.push(CanvasRect::sharp(0.0, y0, w as f32, y0 + row_h as f32, bg));
        rects.push(CanvasRect::sharp(
            0.0,
            y0 + row_h as f32 - 1.0,
            w as f32,
            y0 + row_h as f32,
            p.border,
        ));

        let bt = y0 + (row_h as f32 - badge_h) * 0.5;
        if is_folder {
            draw_token_pill(
                &mut rects,
                &mut texts,
                (pad, bt, pad + badge_w, bt + badge_h),
                p.accent_muted,
                None,
                p.on_accent_muted,
                theme::FONT_MICRO,
                TextFont::Mono,
                theme::TRACKING_CAPS,
                true,
                "DIR",
            );
        } else {
            draw_token_pill(
                &mut rects,
                &mut texts,
                (pad, bt, pad + badge_w, bt + badge_h),
                p.bg_muted,
                Some(p.border),
                p.text_secondary,
                theme::FONT_MICRO,
                TextFont::Mono,
                theme::TRACKING_CAPS,
                true,
                "PDF",
            );
        }

        texts.push(CanvasText::new(
            pad + badge_w + 12.0,
            y0 + row_h as f32 * 0.64,
            row_ts,
            if is_folder {
                p.text_accent
            } else {
                p.text_primary
            },
            TextAlign::Left,
            is_folder,
            label,
        ));
        if !size_str.is_empty() {
            texts.push(
                CanvasText::new(
                    w as f32 - pad,
                    y0 + row_h as f32 * 0.64,
                    theme::FONT_CAPTION,
                    p.text_secondary,
                    TextAlign::Right,
                    false,
                    size_str,
                )
                .with_font(TextFont::Mono),
            );
        }
    }

    jni_text_bitmap(w, h, p.bg_body, &rects, &texts)
}

/// Items interactivos del menú Vista "⋯".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewMenuItem {
    Grid,
    List,
    ColumnsAuto,
    ColumnsDec,
    ColumnsInc,
    CoverCrop,
    CoverFit,
    CoverHide,
    GroupNone,
    GroupAuthor,
    SortTitle,
    SortAuthor,
    SortAdded,
    SortRead,
    SortProgress,
}

/// Geometría compartida del menú Vista "⋯" (coords en px de ventana).
/// Devuelve el rect del card del menú y los rects de cada ítem interactivo.
#[allow(clippy::type_complexity)]
pub(crate) fn view_menu_geometry(
    win_w: i32,
    win_h: i32,
) -> (
    (f32, f32, f32, f32),
    Vec<(ViewMenuItem, (f32, f32, f32, f32))>,
) {
    let (_vl, _vt, vr, vb) = view_menu_button_rect(win_w, win_h);
    let menu_w = 380.0f32.min(win_w as f32 - 32.0);
    let menu_r = vr;
    let menu_l = menu_r - menu_w;
    let menu_t = vb + 8.0f32;
    let mut items = Vec::new();

    let mut y = menu_t + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;
    let pad_x = 16.0f32;

    // 1. MODO DE VISTA
    y += sec_h;
    items.push((
        ViewMenuItem::Grid,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::List,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 2. COLUMNAS
    y += sec_h;
    let btn_step_w = 44.0f32;
    let auto_w = 84.0f32;
    items.push((
        ViewMenuItem::ColumnsAuto,
        (
            menu_l + pad_x,
            y + 2.0,
            menu_l + pad_x + auto_w,
            y + item_h - 2.0,
        ),
    ));
    items.push((
        ViewMenuItem::ColumnsDec,
        (
            menu_r - pad_x - 2.0 * btn_step_w - 44.0,
            y + 2.0,
            menu_r - pad_x - btn_step_w - 44.0,
            y + item_h - 2.0,
        ),
    ));
    items.push((
        ViewMenuItem::ColumnsInc,
        (
            menu_r - pad_x - btn_step_w,
            y + 2.0,
            menu_r - pad_x,
            y + item_h - 2.0,
        ),
    ));
    y += item_h + hr_h;

    // 3. PORTADAS
    y += sec_h;
    items.push((
        ViewMenuItem::CoverCrop,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::CoverFit,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::CoverHide,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 4. AGRUPAR POR
    y += sec_h;
    items.push((
        ViewMenuItem::GroupNone,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::GroupAuthor,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 5. ORDENAR POR
    y += sec_h;
    items.push((
        ViewMenuItem::SortTitle,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::SortAuthor,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::SortAdded,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::SortRead,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        ViewMenuItem::SortProgress,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + 12.0;

    let menu_b = y;
    ((menu_l, menu_t, menu_r, menu_b), items)
}

/// Dibuja las primitivas del popover Vista (⋯) en lenguaje Astryx (`bg_popover` + `Elevation::High`).
pub(crate) fn draw_view_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    let (card_rect, _items) = view_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, _mb) = card_rect;
    let p = reader.theme.palette();

    draw_surface_card(
        rects,
        card_rect,
        theme::RADIUS_CARD,
        p.bg_popover,
        p.border_emphasized,
        Elevation::High,
        p.is_dark,
    );

    let pad_x = 16.0f32;
    let mut y = mt + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;

    // 1. MODO DE VISTA
    draw_menu_section(texts, ml + pad_x, y, "MODO DE VISTA", &p);
    y += sec_h;
    for (label, active) in [
        ("Cuadrícula", reader.is_grid()),
        ("Lista", !reader.is_grid()),
    ] {
        draw_menu_row(rects, texts, ml, mr, pad_x, y, item_h, label, active, &p);
        y += item_h;
    }
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 2. COLUMNAS
    draw_menu_section(texts, ml + pad_x, y, "COLUMNAS", &p);
    y += sec_h;
    draw_astryx_button(
        rects,
        texts,
        (ml + pad_x, y + 2.0, ml + pad_x + 84.0, y + item_h - 2.0),
        if reader.auto_columns {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        },
        &p,
        theme::FONT_CAPTION,
        true,
        "Auto",
    );

    let step_variant = if reader.auto_columns {
        ButtonVariant::Ghost
    } else {
        ButtonVariant::Secondary
    };
    let step_r = mr - pad_x;
    draw_astryx_button(
        rects,
        texts,
        (step_r - 132.0, y + 2.0, step_r - 88.0, y + item_h - 2.0),
        step_variant,
        &p,
        theme::FONT_BODY,
        true,
        "−",
    );
    texts.push(
        CanvasText::new(
            step_r - 66.0,
            y + item_h * 0.68,
            theme::FONT_BODY,
            if reader.auto_columns {
                p.text_secondary
            } else {
                p.text_primary
            },
            TextAlign::Center,
            true,
            format!("{}", reader.columns.clamp(1, 4)),
        )
        .with_font(TextFont::Mono),
    );
    draw_astryx_button(
        rects,
        texts,
        (step_r - 44.0, y + 2.0, step_r, y + item_h - 2.0),
        step_variant,
        &p,
        theme::FONT_BODY,
        true,
        "+",
    );
    y += item_h;
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 3. PORTADAS
    draw_menu_section(texts, ml + pad_x, y, "PORTADAS", &p);
    y += sec_h;
    for (label, active) in [
        (
            "Recortar",
            reader.cover_fit == LibraryCoverFit::Crop && !reader.hide_covers,
        ),
        (
            "Completa",
            reader.cover_fit == LibraryCoverFit::Fit && !reader.hide_covers,
        ),
        ("Ocultar portadas", reader.hide_covers),
    ] {
        draw_menu_row(rects, texts, ml, mr, pad_x, y, item_h, label, active, &p);
        y += item_h;
    }
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 4. AGRUPAR POR
    draw_menu_section(texts, ml + pad_x, y, "AGRUPAR POR", &p);
    y += sec_h;
    for (label, active) in [
        ("Sin agrupar", reader.group_by == LibraryGroupBy::None),
        ("Autor", reader.group_by == LibraryGroupBy::Author),
    ] {
        draw_menu_row(rects, texts, ml, mr, pad_x, y, item_h, label, active, &p);
        y += item_h;
    }
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 5. ORDENAR POR
    draw_menu_section(texts, ml + pad_x, y, "ORDENAR POR", &p);
    y += sec_h;
    for (label, active) in [
        ("Título", reader.library.lib_sort == LibSort::Title),
        ("Autor", reader.library.lib_sort == LibSort::Author),
        (
            "Fecha añadido",
            reader.library.lib_sort == LibSort::RecentlyAdded,
        ),
        (
            "Última lectura",
            reader.library.lib_sort == LibSort::RecentlyRead,
        ),
        ("Progreso", reader.library.lib_sort == LibSort::Progress),
    ] {
        draw_menu_row(rects, texts, ml, mr, pad_x, y, item_h, label, active, &p);
        y += item_h;
    }
}

/// Items interactivos del menú Ajustes "☰".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsMenuItem {
    CoverSizeSmall,
    CoverSizeMedium,
    CoverSizeLarge,
    CoverProgress,
    ClearLibrary,
}

/// Geometría compartida del menú Ajustes "☰" (coords en px de ventana).
#[allow(clippy::type_complexity)]
pub(crate) fn settings_menu_geometry(
    win_w: i32,
    win_h: i32,
) -> (
    (f32, f32, f32, f32),
    Vec<(SettingsMenuItem, (f32, f32, f32, f32))>,
) {
    let (_sl, _st, sr, sb) = settings_menu_button_rect(win_w, win_h);
    let menu_w = 380.0f32.min(win_w as f32 - 32.0);
    let menu_r = sr;
    let menu_l = menu_r - menu_w;
    let menu_t = sb + 8.0f32;
    let mut items = Vec::new();

    let mut y = menu_t + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;
    let pad_x = 16.0f32;

    // 1. TAMAÑO DE PORTADA
    y += sec_h;
    items.push((
        SettingsMenuItem::CoverSizeSmall,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        SettingsMenuItem::CoverSizeMedium,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h;
    items.push((
        SettingsMenuItem::CoverSizeLarge,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 2. PROGRESO
    y += sec_h;
    items.push((
        SettingsMenuItem::CoverProgress,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 3. ADMINISTRAR BIBLIOTECA
    y += sec_h;
    items.push((
        SettingsMenuItem::ClearLibrary,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 4. ACERCA DE (no interactivo)
    y += sec_h;
    y += 34.0 + 26.0 + 24.0 + 12.0;

    let menu_b = y;
    ((menu_l, menu_t, menu_r, menu_b), items)
}

/// Dibuja las primitivas del popover Ajustes (☰) en lenguaje Astryx.
pub(crate) fn draw_settings_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    let (card_rect, _items) = settings_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, _mb) = card_rect;
    let p = reader.theme.palette();

    draw_surface_card(
        rects,
        card_rect,
        theme::RADIUS_CARD,
        p.bg_popover,
        p.border_emphasized,
        Elevation::High,
        p.is_dark,
    );

    let pad_x = 16.0f32;
    let mut y = mt + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;

    // 1. TAMAÑO DE PORTADA
    draw_menu_section(texts, ml + pad_x, y, "TAMAÑO DE PORTADA", &p);
    y += sec_h;
    for (label, active) in [
        ("Pequeño", reader.cover_size == 0),
        ("Mediano", reader.cover_size == 1),
        ("Grande", reader.cover_size == 2),
    ] {
        draw_menu_row(rects, texts, ml, mr, pad_x, y, item_h, label, active, &p);
        y += item_h;
    }
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 2. PROGRESO
    draw_menu_section(texts, ml + pad_x, y, "PROGRESO", &p);
    y += sec_h;
    draw_menu_row(
        rects,
        texts,
        ml,
        mr,
        pad_x,
        y,
        item_h,
        "Mostrar porcentaje de lectura",
        reader.cover_progress,
        &p,
    );
    y += item_h;
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 3. ADMINISTRAR BIBLIOTECA (semántica Danger Astryx)
    draw_menu_section(texts, ml + pad_x, y, "ADMINISTRAR BIBLIOTECA", &p);
    y += sec_h;
    {
        let confirming = reader
            .clear_confirm_until
            .map(|until| std::time::Instant::now() <= until)
            .unwrap_or(false);
        let clear_label = if confirming {
            "¿Vaciar? Toca otra vez para confirmar"
        } else {
            "Vaciar biblioteca"
        };
        if confirming {
            draw_surface_card(
                rects,
                (
                    ml + pad_x - 6.0,
                    y + 2.0,
                    mr - pad_x + 6.0,
                    y + item_h - 2.0,
                ),
                theme::RADIUS_ELEMENT,
                p.error_bg,
                p.error_border,
                Elevation::None,
                p.is_dark,
            );
        }
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            theme::FONT_BODY,
            p.error_text,
            TextAlign::Left,
            confirming,
            clear_label,
        ));
        y += item_h;
    }
    draw_menu_divider(rects, ml, mr, pad_x, y, &p);
    y += hr_h;

    // 4. ACERCA DE
    draw_menu_section(texts, ml + pad_x, y, "ACERCA DE", &p);
    y += sec_h;
    texts.push(
        CanvasText::new(
            ml + pad_x,
            y + theme::FONT_BODY * 0.85,
            theme::FONT_BODY,
            p.text_primary,
            TextAlign::Left,
            true,
            format!("PDFLector v{}", env!("CARGO_PKG_VERSION")),
        )
        .with_font(TextFont::Mono),
    );
    y += 28.0;
    texts.push(CanvasText::new(
        ml + pad_x,
        y + theme::FONT_CAPTION * 0.85,
        theme::FONT_CAPTION,
        p.text_secondary,
        TextAlign::Left,
        false,
        "Código abierto · AGPL-3.0",
    ));
    y += 24.0;
    texts.push(CanvasText::new(
        ml + pad_x,
        y + theme::FONT_MICRO * 0.85,
        theme::FONT_MICRO,
        p.text_secondary,
        TextAlign::Left,
        false,
        crate::draw::discover::ARXIV_ATTRIBUTION,
    ));
}
