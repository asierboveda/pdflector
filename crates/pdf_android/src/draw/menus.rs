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

use super::{CanvasRect, CanvasText, TextAlign, draw_button, draw_card_shadow, jni_text_bitmap};

/// Renderiza la lista del picker a un bitmap RGBA8 de tamaño de ventana.
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

    // Cabecera + línea divisoria
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        header_h as f32,
        p.base_100,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        header_h as f32 - 1.0,
        w as f32,
        header_h as f32,
        p.base_300,
    ));

    // Título según variante: fallback interno ("Abrir PDF") o selector de
    // añadir ("Selecciona PDF": elige cuál de TODOS los PDFs del sistema
    // pasa a la biblioteca curada).
    let selecting = reader.picker_kind == PickerKind::Select;
    texts.push(CanvasText::new(
        pad,
        header_h as f32 * 0.62,
        theme::FONT_TITLE,
        p.base_content,
        TextAlign::Left,
        true,
        if selecting {
            "Selecciona PDF"
        } else {
            "Abrir PDF"
        },
    ));

    let btn_y = (header_h - btn_h) as f32 / 2.0;
    let back_x = w as f32 - btn_w as f32 * 2.0 - 16.0;
    let rescan_x = w as f32 - btn_w as f32 - 8.0;
    let btn_ts = theme::FONT_CAPTION;

    if reader.doc.is_some() || selecting {
        draw_button(
            &mut rects,
            &mut texts,
            back_x,
            btn_y,
            back_x + btn_w as f32,
            btn_y + btn_h as f32,
            p.base_200,
            p.base_300,
            p.base_content,
            btn_ts,
            true,
            "Atrás",
        );
    }
    draw_button(
        &mut rects,
        &mut texts,
        rescan_x,
        btn_y,
        rescan_x + btn_w as f32,
        btn_y + btn_h as f32,
        p.primary,
        p.primary,
        p.primary_content,
        btn_ts,
        true,
        "Reescanear",
    );

    // Franja de estado
    let crumbs = if reader.picker_has_crumb() { row_h } else { 0 };
    let rows_y0 = header_h + status_h + crumbs;
    if let Some(status) = reader.status.as_deref() {
        rects.push(CanvasRect::sharp(
            0.0,
            header_h as f32,
            w as f32,
            rows_y0 as f32,
            p.status_bg(),
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            rows_y0 as f32 - 1.0,
            w as f32,
            rows_y0 as f32,
            p.status_border(),
        ));
        let ts = theme::FONT_CAPTION;
        texts.push(CanvasText::new(
            pad,
            header_h as f32 + row_h as f32 * 0.62,
            ts,
            p.status_text(),
            TextAlign::Left,
            true,
            status,
        ));
    }

    // Barra de BREADCRUMB del gestor de archivos (selector de añadir dentro
    // de una carpeta): muestra la ruta actual y, al tocarla, sube un nivel
    // (`picker_sel_up` en `input`).
    if reader.picker_has_crumb() {
        let y0 = header_h as f32 + status_h as f32;
        rects.push(CanvasRect::sharp(
            0.0,
            y0,
            w as f32,
            y0 + row_h as f32,
            p.base_200,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            y0 + row_h as f32 - 1.0,
            w as f32,
            y0 + row_h as f32,
            p.base_300,
        ));
        texts.push(CanvasText::new(
            pad,
            y0 + row_h as f32 * 0.66,
            theme::FONT_CAPTION,
            p.primary,
            TextAlign::Left,
            true,
            format!("⬆  {}", reader.sel_dir.join("/")),
        ));
    }

    // Filas del gestor: carpetas primero (📁), luego los PDFs del nivel
    // actual (📄). El fallback interno (`PickerKind::Files`) lee `pdf_list`.
    let rows: Vec<PickRow> = reader.picker_rows();
    let visible = reader.picker_visible();
    let row_ts = theme::FONT_BODY;
    for i in 0..visible {
        let r = reader.list_scroll + i;
        let (label, size_str) = if selecting {
            match rows.get(r) {
                Some(PickRow::Folder(name)) => (format!("📁  {name}"), String::new()),
                Some(PickRow::File(idx)) => {
                    let Some(e) = reader.select_list.get(*idx) else {
                        break;
                    };
                    let sz = if e.size > 0 {
                        human_size(e.size.max(0) as u64)
                    } else {
                        String::new()
                    };
                    (format!("📄  {}", truncate_name(&e.name, 48)), sz)
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
            let max_chars = (((w as f32 - pad * 3.0 - size_w) / char_w) as usize).max(1);
            (
                format!(
                    "📄  {} [{}]",
                    truncate_name(&entry.name, max_chars),
                    entry.source
                ),
                size_str,
            )
        };
        let y0 = (rows_y0 + (i as i32) * row_h) as f32;
        let bg = if i % 2 == 0 { p.base_100 } else { p.base_200 };
        rects.push(CanvasRect::sharp(0.0, y0, w as f32, y0 + row_h as f32, bg));
        rects.push(CanvasRect::sharp(
            0.0,
            y0 + row_h as f32 - 1.0,
            w as f32,
            y0 + row_h as f32,
            p.base_300,
        ));

        let is_folder = matches!(rows.get(r), Some(PickRow::Folder(_)));
        texts.push(CanvasText::new(
            pad,
            y0 + row_h as f32 * 0.64,
            row_ts,
            if is_folder { p.primary } else { p.base_content },
            TextAlign::Left,
            true,
            label,
        ));
        texts.push(CanvasText::new(
            w as f32 - pad,
            y0 + row_h as f32 * 0.64,
            theme::FONT_CAPTION,
            p.neutral_content,
            TextAlign::Right,
            false,
            size_str,
        ));
    }

    jni_text_bitmap(w, h, p.base_100, &rects, &texts)
}

/// Renderiza la biblioteca MediaStore a un bitmap RGBA8 de tamaño de
/// ventana: la pantalla de biblioteca PERSONAL premium (estilo Apple
/// Books/Kindle pero propio) — las PORTADAS son lo principal, no un file
/// manager (sin rutas completas visibles, sin iconos PDF genéricos: siempre
/// portada real o placeholder elegante con el título).
///
/// Estructura FIJA (no scrollea): cabecera editorial (título "Library"
/// grande + botón "＋ Add book") + campo de búsqueda (+ panel de chips de
/// letra/carpeta si está abierto) + franja de estado (si la hay). Contenido
/// SCROLLABLE (desplazado `reader.library.lib_scroll` px): [Continue Reading:
/// carousel horizontal de tarjetas con portada 2:3 grande, título, autor,
/// barra de progreso, "Page X of Y · Z%" y botón Read] + [título "My
/// Library" + chips de organización (sort/filter) + rejilla 3×3 de portadas
/// con título, autor y barra fina de progreso] + EMPTY STATE si no hay PDFs.
///
/// El bitmap base (fondo, cabecera, campo de búsqueda, chips, tarjetas,
/// textos, barras de progreso, SOMBRAS y placeholders) se dibuja con
/// Canvas+JNI (`jni_text_bitmap`); las portadas CACHEADAS se pegan después
/// directamente sobre sus bytes RGBA (Canvas no pinta bitmaps): center-crop
/// vecino-más-cercano al área 2:3 (`grid_cover_w`×`grid_cover_h` /
/// `lib_cont_cover_*`), sin pasar por un lock de ventana.
/// Items interactivos del menú View "⋯" (Readest ViewMenu).
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
    RecentShelf,
    GroupNone,
    GroupAuthor,
    SortTitle,
    SortAuthor,
    SortAdded,
    SortRead,
    SortProgress,
}

/// Geometría compartida del menú View "⋯" (coords en px de ventana).
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

    // 4. DESTACADOS
    items.push((
        ViewMenuItem::RecentShelf,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 5. AGRUPAR POR
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

    // 6. ORDENAR POR
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

/// Dibuja las primitivas del dropdown ViewMenu (⋯) en coords absolutas de ventana.
pub(crate) fn draw_view_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    let (card_rect, _items) = view_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, mb) = card_rect;
    let p = reader.theme.palette();

    // Sombra multinivel (shadow-2xl)
    draw_card_shadow(rects, ml, mt, mr, mb, 16.0, p.is_dark);

    // Fondo base-100 + borde base-300
    rects.push(CanvasRect::rounded(ml, mt, mr, mb, 16.0, p.base_300));
    rects.push(CanvasRect::rounded(
        ml + 1.0,
        mt + 1.0,
        mr - 1.0,
        mb - 1.0,
        15.0,
        p.base_100,
    ));

    let pad_x = 16.0f32;
    let sec_ts = theme::FONT_CAPTION * 0.95; // 11sp
    let body_ts = theme::FONT_BODY; // 14sp

    let mut y = mt + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;

    // 1. MODO DE VISTA
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "MODO DE VISTA".to_string(),
    ));
    y += sec_h;
    for (_item, label, active) in [
        (ViewMenuItem::Grid, "Cuadrícula (Grid)", reader.is_grid()),
        (ViewMenuItem::List, "Lista (List)", !reader.is_grid()),
    ] {
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            label.to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 2. COLUMNAS
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "COLUMNAS".to_string(),
    ));
    y += sec_h;
    // Botón Auto
    let (auto_fill, auto_border, auto_fg) = if reader.auto_columns {
        (p.primary, p.primary, p.primary_content)
    } else {
        (p.base_200, p.base_300, p.base_content)
    };
    draw_button(
        rects,
        texts,
        ml + pad_x,
        y + 2.0,
        ml + pad_x + 84.0,
        y + item_h - 2.0,
        auto_fill,
        auto_border,
        auto_fg,
        theme::FONT_CAPTION,
        true,
        "Auto",
    );

    // Stepper − N +
    let step_fg = if reader.auto_columns {
        p.neutral_content
    } else {
        p.base_content
    };
    let step_bg = if reader.auto_columns {
        p.base_100
    } else {
        p.base_200
    };
    let step_bdr = p.base_300;
    let step_r = mr - pad_x;
    draw_button(
        rects,
        texts,
        step_r - 132.0,
        y + 2.0,
        step_r - 88.0,
        y + item_h - 2.0,
        step_bg,
        step_bdr,
        step_fg,
        body_ts,
        true,
        "−",
    );
    let num_str = format!("{}", reader.columns.clamp(1, 4));
    texts.push(CanvasText::new(
        step_r - 66.0,
        y + item_h * 0.68,
        body_ts,
        step_fg,
        TextAlign::Center,
        true,
        num_str,
    ));
    draw_button(
        rects,
        texts,
        step_r - 44.0,
        y + 2.0,
        step_r,
        y + item_h - 2.0,
        step_bg,
        step_bdr,
        step_fg,
        body_ts,
        true,
        "+",
    );
    y += item_h;
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 3. PORTADAS
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "PORTADAS".to_string(),
    ));
    y += sec_h;
    for (label, active) in [
        (
            "Recortar (Crop)",
            reader.cover_fit == LibraryCoverFit::Crop && !reader.hide_covers,
        ),
        (
            "Completa (Fit)",
            reader.cover_fit == LibraryCoverFit::Fit && !reader.hide_covers,
        ),
        ("Ocultar portadas", reader.hide_covers),
    ] {
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            label.to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 4. DESTACADOS
    {
        let active = reader.recent_shelf_enabled;
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            "Mostrar lectura reciente".to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 5. AGRUPAR POR
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "AGRUPAR POR".to_string(),
    ));
    y += sec_h;
    for (label, active) in [
        ("Ninguno (Libros)", reader.group_by == LibraryGroupBy::None),
        ("Autor", reader.group_by == LibraryGroupBy::Author),
    ] {
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            label.to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 6. ORDENAR POR
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "ORDENAR POR".to_string(),
    ));
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
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            label.to_string(),
        ));
        y += item_h;
    }
}

/// Renderiza el dropdown ViewMenu (⋯) completo a un bitmap RGBA8.
#[allow(dead_code)]
pub(crate) fn render_view_menu(reader: &Reader) -> Option<Bitmap> {
    let (card_rect, _items) = view_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, mb) = card_rect;
    let mw = (mr - ml).ceil() as i32;
    let mh = (mb - mt).ceil() as i32;
    if mw <= 0 || mh <= 0 {
        return None;
    }
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    draw_view_menu(reader, &mut rects, &mut texts);
    for r in &mut rects {
        r.left -= ml;
        r.right -= ml;
        r.top -= mt;
        r.bottom -= mt;
    }
    for t in &mut texts {
        t.x -= ml;
        t.y -= mt;
    }
    jni_text_bitmap(mw, mh, theme::TRANSPARENT, &rects, &texts)
}

/// Items interactivos del menú Settings "☰" (Readest SettingsMenu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsMenuItem {
    RecentShelf,
    CoverSizeSmall,
    CoverSizeMedium,
    CoverSizeLarge,
    CoverProgress,
    ClearLibrary,
}

/// Geometría compartida del menú Settings "☰" (coords en px de ventana).
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

    // 1. RECENTLY READ
    y += sec_h;
    items.push((
        SettingsMenuItem::RecentShelf,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 2. COVER SIZE
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

    // 3. SHOW PROGRESS
    y += sec_h;
    items.push((
        SettingsMenuItem::CoverProgress,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 4. MANAGE LIBRARY
    y += sec_h;
    items.push((
        SettingsMenuItem::ClearLibrary,
        (menu_l + pad_x, y, menu_r - pad_x, y + item_h),
    ));
    y += item_h + hr_h;

    // 5. ABOUT
    y += sec_h;
    // About is non-interactive
    y += 34.0 + 26.0 + 12.0;

    let menu_b = y;
    ((menu_l, menu_t, menu_r, menu_b), items)
}

/// Dibuja las primitivas del dropdown SettingsMenu (☰) en coords absolutas de ventana.
pub(crate) fn draw_settings_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    let (card_rect, _items) = settings_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, mb) = card_rect;
    let p = reader.theme.palette();

    // Sombra multinivel (shadow-2xl)
    draw_card_shadow(rects, ml, mt, mr, mb, 16.0, p.is_dark);

    // Fondo base-100 + borde base-300
    rects.push(CanvasRect::rounded(ml, mt, mr, mb, 16.0, p.base_300));
    rects.push(CanvasRect::rounded(
        ml + 1.0,
        mt + 1.0,
        mr - 1.0,
        mb - 1.0,
        15.0,
        p.base_100,
    ));

    let pad_x = 16.0f32;
    let sec_ts = theme::FONT_CAPTION * 0.95; // 11sp
    let body_ts = theme::FONT_BODY; // 14sp

    let mut y = mt + 12.0;
    let item_h = 38.0f32;
    let sec_h = 24.0f32;
    let hr_h = 8.0f32;

    // 1. RECENTLY READ
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "LECTURA RECIENTE".to_string(),
    ));
    y += sec_h;
    {
        let active = reader.recent_shelf_enabled;
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            "Mostrar lectura reciente".to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 2. COVER SIZE
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "TAMAÑO DE PORTADA".to_string(),
    ));
    y += sec_h;
    for (label, active) in [
        ("Pequeño (Small)", reader.cover_size == 0),
        ("Mediano (Medium)", reader.cover_size == 1),
        ("Grande (Large)", reader.cover_size == 2),
    ] {
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            label.to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 3. SHOW PROGRESS
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "PROGRESO".to_string(),
    ));
    y += sec_h;
    {
        let active = reader.cover_progress;
        if active {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                p.base_200,
            ));
            texts.push(CanvasText::new(
                mr - pad_x - 4.0,
                y + item_h * 0.68,
                body_ts,
                p.primary,
                TextAlign::Right,
                true,
                "✓".to_string(),
            ));
        }
        let fg = if active { p.primary } else { p.base_content };
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            fg,
            TextAlign::Left,
            active,
            "Mostrar porcentaje de lectura".to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 4. MANAGE LIBRARY
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "ADMINISTRAR BIBLIOTECA".to_string(),
    ));
    y += sec_h;
    {
        let confirming = reader
            .clear_confirm_until
            .map(|until| std::time::Instant::now() <= until)
            .unwrap_or(false);
        let clear_label = if confirming {
            "¿Vaciar? Toca de nuevo para confirmar"
        } else {
            "Vaciar biblioteca"
        };
        let danger_color = if p.is_dark { 0xFFFF6B6B } else { 0xFFD32F2F };
        if confirming {
            rects.push(CanvasRect::rounded(
                ml + pad_x - 4.0,
                y + 2.0,
                mr - pad_x + 4.0,
                y + item_h - 2.0,
                8.0,
                if p.is_dark { 0x33FF6B6B } else { 0x22D32F2F },
            ));
        }
        texts.push(CanvasText::new(
            ml + pad_x,
            y + item_h * 0.68,
            body_ts,
            danger_color,
            TextAlign::Left,
            confirming,
            clear_label.to_string(),
        ));
        y += item_h;
    }
    rects.push(CanvasRect::sharp(
        ml + pad_x,
        y + 3.0,
        mr - pad_x,
        y + 4.0,
        p.base_300,
    ));
    y += hr_h;

    // 5. ABOUT
    texts.push(CanvasText::new(
        ml + pad_x,
        y + sec_ts * 0.85,
        sec_ts,
        p.neutral_content,
        TextAlign::Left,
        true,
        "ACERCA DE".to_string(),
    ));
    y += sec_h;
    texts.push(CanvasText::new(
        ml + pad_x,
        y + theme::FONT_CAPTION * 1.1 * 0.85,
        theme::FONT_CAPTION * 1.1,
        p.base_content,
        TextAlign::Left,
        true,
        format!("PDFLector v{}", env!("CARGO_PKG_VERSION")),
    ));
    y += 28.0;
    texts.push(CanvasText::new(
        ml + pad_x,
        y + theme::FONT_CAPTION * 0.9 * 0.85,
        theme::FONT_CAPTION * 0.9,
        p.neutral_content,
        TextAlign::Left,
        false,
        "Open source · AGPL-3.0".to_string(),
    ));
}

/// Renderiza el dropdown SettingsMenu (☰) completo a un bitmap RGBA8.
#[allow(dead_code)]
pub(crate) fn render_settings_menu(reader: &Reader) -> Option<Bitmap> {
    let (card_rect, _items) = settings_menu_geometry(reader.win_w, reader.win_h);
    let (ml, mt, mr, mb) = card_rect;
    let mw = (mr - ml).ceil() as i32;
    let mh = (mb - mt).ceil() as i32;
    if mw <= 0 || mh <= 0 {
        return None;
    }
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    draw_settings_menu(reader, &mut rects, &mut texts);
    for r in &mut rects {
        r.left -= ml;
        r.right -= ml;
        r.top -= mt;
        r.bottom -= mt;
    }
    for t in &mut texts {
        t.x -= ml;
        t.y -= mt;
    }
    jni_text_bitmap(mw, mh, theme::TRANSPARENT, &rects, &texts)
}
