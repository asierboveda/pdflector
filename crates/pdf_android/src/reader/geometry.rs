// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Geometría y layout de picker/biblioteca/visor (extraído de `reader.rs`, 2026-09-06): funciones libres `lib_*`, `grid_*`, `list_*`, `sheet_*`, `picker_*` y `viewer_*` — altos, rects y posiciones en px compartidos por el render (`draw`), el tap/arrastre (`input`) y el blit (`reader`).

use super::BookStatus;
use super::EmptyStateGeom;
use super::LibSort;
use super::Reader;
use crate::draw::ButtonRect;

/// Alto (px) de cada fila del picker, proporcional a la ventana.
pub(crate) fn picker_row_h(win_h: i32) -> i32 {
    (win_h / 26).max(48)
}

/// Alto (px) de la cabecera del picker (título + botones).
pub(crate) fn picker_header_h(win_h: i32) -> i32 {
    picker_row_h(win_h) * 3 / 2
}

/// Ancho (px) de los botones de la cabecera del picker.
pub(crate) fn picker_btn_w(win_w: i32) -> i32 {
    win_w / 4
}

/// Alto (px) de los botones de la cabecera del picker.
pub(crate) fn picker_btn_h(win_h: i32) -> i32 {
    picker_row_h(win_h) * 4 / 5
}

/// Nº de filas visibles en el picker (depende de si hay mensaje de estado).
pub(crate) fn picker_visible_rows(win_h: i32, has_status: bool) -> usize {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    ((win_h - picker_header_h(win_h) - status_h) / picker_row_h(win_h)).max(0) as usize
}

/// Alto (px) del área de la barra superior flotante de chrome del visor.
pub(crate) fn viewer_top_chrome_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.02).clamp(28.0, 48.0) + 68.0 + 12.0
}

/// Alto (px) del área de la barra inferior flotante de chrome del visor.
pub(crate) fn viewer_bottom_chrome_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.025).clamp(28.0, 48.0) + 76.0 + 16.0
}

/// Alto (px) del sheet de ajustes (B2: ajustado al contenido real ~42% de win_h).
pub(crate) fn sheet_h(win_h: i32) -> i32 {
    (win_h as f32 * 0.42).clamp(650.0, 950.0).round() as i32
}

/// Pad horizontal del sheet (px).
pub(crate) fn sheet_pad(win_w: i32) -> f32 {
    (win_w as f32 * 0.04).clamp(24.0, 56.0)
}

/// Alto (px) de los botones del sheet (S3: alto >= 48 px).
pub(crate) fn sheet_btn_h(_win_h: i32) -> f32 {
    50.0
}

/// Y del borde superior de la fila de temas del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_theme_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.18).clamp(90.0, 160.0)
}

/// Y del borde superior de la fila de navegación del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_nav_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.48).clamp(280.0, 430.0)
}

/// Y del borde superior de la fila de acciones del sheet (B2: distribuido uniforme).
pub(crate) fn sheet_act_y(win_h: i32) -> f32 {
    let sh = sheet_h(win_h) as f32;
    (sh * 0.78).clamp(470.0, 700.0)
}

/// Ancho (px) de cada botón de 3 por fila del sheet.
pub(crate) fn sheet_btn_w(win_w: i32) -> f32 {
    (win_w as f32 - 4.0 * sheet_pad(win_w)) / 3.0
}

/// Ancho (px) de cada botón de 4 por fila (temas) del sheet.
pub(crate) fn sheet_theme_btn_w(win_w: i32) -> f32 {
    (win_w as f32 - 5.0 * sheet_pad(win_w)) / 4.0
}

/// --- Rejilla 3×3 de la biblioteca (geometría compartida por render y tap) ---
/// Columnas de la rejilla de la biblioteca.
pub(crate) const GRID_COLS: usize = 3;

/// Pad exterior horizontal de la rejilla (px): margen con respiro estilo Apple Books.
pub(crate) fn grid_pad(win_w: i32) -> f32 {
    (win_w as f32 * 0.04).clamp(24.0, 60.0)
}

/// Separación entre celdas de la rejilla (px): respiro >= 3% de win_w.
pub(crate) fn grid_gap(win_w: i32) -> f32 {
    (win_w as f32 * 0.035).clamp(24.0, 56.0)
}

/// Inset de la portada dentro de la celda (px).
pub(crate) const GRID_CELL_PAD: f32 = 10.0;

/// Ancho (px) de una celda de la rejilla según el número de columnas.
pub(crate) fn grid_cell_w(win_w: i32, cols: usize) -> f32 {
    let w = win_w as f32;
    let c = cols.max(1) as f32;
    (w - 2.0 * grid_pad(win_w) - (c - 1.0) * grid_gap(win_w)) / c
}

/// Multiplicador de escala según `cover_size`: 0 -> 0.85 (Pequeño), 1 -> 1.0 (Mediano), 2 -> 1.15 (Grande).
pub(crate) fn cover_size_multiplier(cover_size: u8) -> f32 {
    match cover_size {
        0 => 0.85,
        2 => 1.15,
        _ => 1.0,
    }
}

/// Ancho (px) del área de portada dentro de la celda.
pub(crate) fn grid_cover_w(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    (grid_cell_w(win_w, cols) - 2.0 * GRID_CELL_PAD) * cover_size_multiplier(cover_size)
}

/// Alto (px) del área de portada: proporción 2:3 (alto = ancho × 1.5), estilo
/// Apple Books, para TODAS las celdas (rejilla uniforme).
pub(crate) fn grid_cover_h(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    grid_cover_w(win_w, cols, cover_size) * 1.5
}

/// Alto (px) de la zona de texto de la celda: título (14sp) + autor (12sp) + barra progreso + padding.
pub(crate) fn grid_title_h(_win_w: i32) -> f32 {
    72.0
}

/// Alto (px) de una celda de la rejilla.
pub(crate) fn grid_cell_h(win_w: i32, cols: usize, cover_size: u8) -> f32 {
    grid_cover_h(win_w, cols, cover_size) + grid_title_h(win_w)
}

/// Alto (px) de una fila de la biblioteca en modo Lista.
pub(crate) fn list_row_h(_win_h: i32, cover_size: u8) -> f32 {
    (116.0 * cover_size_multiplier(cover_size)).max(96.0)
}

/// Separación vertical (px) entre filas en modo Lista.
pub(crate) fn list_row_gap() -> f32 {
    12.0
}

/// Rectángulo de una fila de la biblioteca en modo Lista.
pub(crate) fn list_row_rect(
    win_w: i32,
    rows_y0: i32,
    idx: usize,
    win_h: i32,
    cover_size: u8,
) -> (f32, f32, f32, f32) {
    let pad = grid_pad(win_w);
    let x = pad;
    let w = win_w as f32 - 2.0 * pad;
    let h = list_row_h(win_h, cover_size);
    let gap = list_row_gap();
    let y = rows_y0 as f32 + idx as f32 * (h + gap);
    (x, y, x + w, y + h)
}

// --- Biblioteca rediseñada: biblioteca PERSONAL premium (2026-08-XX) ---
//
// La biblioteca ya NO es un file manager: es una biblioteca personal de
// libros (estilo Apple Books/Kindle pero propio). Las PORTADAS mandan;
// el header es editorial (título grande + "＋ Add book" + campo de
// búsqueda); "Continue Reading" (carousel horizontal de tarjetas con
// portada grande, título, autor, barra de progreso, "Page X of Y" y acción
// "Read") es el punto de entrada; y "My Library" es la rejilla principal
// de portadas con título/autor/progreso y sus chips discretos de
// organización (sort/filter). Toda la geometría de abajo es COMPARTIDA
// por el render (`draw::render_library_zone` + `render_library_header`), el tap y el arrastre
// (`input`) y el pump de portadas (`Reader::pump_thumbs`).
/// Alto (px) de la CABECERA de la biblioteca: título "Library" grande y
/// negrita + botón "＋ Add book" a la derecha.
pub(crate) fn lib_header_h(win_h: i32) -> f32 {
    (win_h as f32 / 16.0).clamp(115.0, 135.0)
}

/// Centro Y (px) de la FILA DE BOTONES de la cabecera editorial ("＋ Añadir"
/// y los círculos de menú "⋯"/"☰"): la mitad del espacio libre bajo el
/// margen superior de 36 px. Compartido por el render y el tap para que el
/// hit-test y el dibujo coincidan exactamente.
pub(crate) fn lib_header_buttons_cy(win_h: i32) -> f32 {
    let header_h = lib_header_h(win_h);
    let top_pad = 36.0f32;
    top_pad + (header_h - top_pad) / 2.0
}

/// Diámetro (px) del círculo de los botones de menú de la cabecera: un
/// touch target de ~32 dp (≈ 65 px con densidad 2.0 de la TCL) — el tamaño
/// mínimo cómodo para un dedo.
pub(crate) fn header_menu_btn_d(win_w: i32) -> f32 {
    (win_w as f32 / 22.0).clamp(56.0, 72.0)
}

/// Separación (px) entre el círculo de menú y su vecino ("8 dp gap").
pub(crate) fn header_menu_gap() -> f32 {
    16.0
}

/// Ancho (px) del botón "＋ Añadir" de la cabecera (compartido por el
/// render y la geometría de los menús ⋯/☰, que se alinean a su izquierda).
pub(crate) fn lib_add_btn_w(win_w: i32) -> f32 {
    (win_w as f32 * 0.18).clamp(110.0, 160.0)
}

/// Rectángulo (left, top, right, bottom) del botón de menú SETTINGS "☰":
/// círculo de `header_menu_btn_d` alineado a la IZQUIERDA del "＋ Añadir"
/// (que mantiene su anclaje a derecha con `grid_pad`) con `header_menu_gap`
/// de separación, centrado en el Y de la fila de botones. Los futuros
/// dropdowns cuelgan de su BORDE DERECHO (`dropdown-end`) para no cortarse
/// por la izquierda.
pub(crate) fn settings_menu_button_rect(win_w: i32, win_h: i32) -> (f32, f32, f32, f32) {
    let d = header_menu_btn_d(win_w);
    let pad = grid_pad(win_w);
    let add_x = win_w as f32 - pad - lib_add_btn_w(win_w); // borde izq del ＋
    let cy = lib_header_buttons_cy(win_h);
    (
        add_x - header_menu_gap() - d,
        cy - d / 2.0,
        add_x - header_menu_gap(),
        cy + d / 2.0,
    )
}

/// Rectángulo del botón de menú VIEW "⋯": a la IZQUIERDA del de settings,
/// con `header_menu_gap` de separación.
pub(crate) fn view_menu_button_rect(win_w: i32, win_h: i32) -> (f32, f32, f32, f32) {
    let (l, t, _, b) = settings_menu_button_rect(win_w, win_h);
    let d = header_menu_btn_d(win_w);
    (l - d - header_menu_gap(), t, l - header_menu_gap(), b)
}

/// Alto (px) del campo de búsqueda (fila fija bajo la cabecera).
pub(crate) fn lib_search_h() -> f32 {
    48.0
}

/// Alto (px) del panel de búsqueda desplegado (2 filas de chips: letras y
/// carpetas); 0 si el campo de búsqueda está cerrado.
pub(crate) fn lib_search_panel_h(win_h: i32, open: bool) -> f32 {
    if open {
        lib_chip_h(win_h) * 2.0 + 16.0
    } else {
        0.0
    }
}

/// Alto (px) de un chip del panel de búsqueda (letras/carpetas, >= 40 px).
pub(crate) fn lib_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 50.0).clamp(40.0, 46.0)
}

/// Y (px) del borde superior de la fila 0 (letras) del panel de búsqueda.
pub(crate) fn lib_search_chips_y0(reader: &Reader) -> f32 {
    lib_header_h(reader.win_h) + lib_search_h() + 6.0
}

/// Y (px) del borde superior de la fila 1 (carpetas) del panel de búsqueda.
pub(crate) fn lib_search_chips_y1(reader: &Reader) -> f32 {
    lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) + 8.0
}

/// Y (px) del borde superior del contenido scrolleable (cabecera + campo de
/// búsqueda + panel de chips si está abierto + franja de estado si la hay).
pub(crate) fn lib_content_y0(win_h: i32, search_open: bool, has_status: bool) -> i32 {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    (lib_header_h(win_h) + lib_search_h() + lib_search_panel_h(win_h, search_open)) as i32
        + status_h
}

/// Alto (px) de un título de sección ("CONTINUE READING"/"My Library").
pub(crate) fn lib_section_title_h(win_h: i32) -> f32 {
    (win_h as f32 / 64.0).clamp(24.0, 32.0)
}

/// Ancho (px) de la portada de una tarjeta de "Continue Reading" (2:3).
// sección "Continue Reading" oculta por diseño (2026-08-25)
pub(crate) fn lib_cont_cover_w(win_h: i32) -> f32 {
    lib_cont_cover_h(win_h) / 1.5
}

/// Alto (px) de la portada de una tarjeta (proporción 2:3).
// sección "Continue Reading" oculta por diseño (2026-08-25)
pub(crate) fn lib_cont_cover_h(win_h: i32) -> f32 {
    lib_cont_card_h(win_h) - 32.0
}

/// Alto (px) de la tarjeta horizontal (~15% de win_h).
pub(crate) fn lib_cont_card_h(win_h: i32) -> f32 {
    (win_h as f32 * 0.15).clamp(240.0, 330.0)
}

/// Ancho (px) de la tarjeta horizontal.
pub(crate) fn lib_cont_card_w(win_w: i32, _win_h: i32) -> f32 {
    (win_w as f32 * 0.52).clamp(440.0, 640.0)
}

/// Separación horizontal entre tarjetas del carousel (px).
pub(crate) fn lib_cont_gap() -> f32 {
    18.0
}

/// X (px) en coords de CONTENIDO de la tarjeta `i` del carousel (sin el
/// scroll horizontal aplicado).
pub(crate) fn lib_cont_card_x(win_w: i32, win_h: i32, i: usize) -> f32 {
    grid_pad(win_w) + i as f32 * (lib_cont_card_w(win_w, win_h) + lib_cont_gap())
}

/// Alto (px) del bloque de "Continue Reading" (título de sección + fila de
/// tarjetas) en coords de contenido; 0 si no hay libros en curso.
pub(crate) fn lib_cont_block_h(_win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    if !has_cont {
        0.0
    } else {
        lib_section_title_h(win_h) + lib_cont_card_h(win_h) + 16.0
    }
}

/// --- Organización de "My Library" (sort + filter, chips discretos) ---
/// Alto (px) de un chip de organización (>= 40 px).
pub(crate) fn lib_org_chip_h(win_h: i32) -> f32 {
    (win_h as f32 / 50.0).clamp(40.0, 46.0)
}

/// Separación entre las filas de chips de sort y filter (px).
pub(crate) fn lib_org_gap() -> f32 {
    10.0
}

/// Alto (px) del bloque de organización (2 filas: sort + filter).
pub(crate) fn lib_org_block_h(win_h: i32) -> f32 {
    lib_org_chip_h(win_h) * 2.0 + lib_org_gap() + 6.0
}

/// Ancho (px) reservado para la etiqueta discreta de cada fila ("SORT" /
/// "FILTER"), antes de los chips.
pub(crate) fn lib_org_label_w() -> f32 {
    54.0
}

/// Y (px) del borde superior de la fila de organización `row` (0 = sort,
/// 1 = filter) en coords de CONTENIDO (bajo el título de "My Library").
pub(crate) fn lib_org_y(win_w: i32, win_h: i32, has_cont: bool, row: usize) -> f32 {
    lib_grid_y0(win_w, win_h, has_cont) - lib_org_block_h(win_h)
        + row as f32 * (lib_org_chip_h(win_h) + lib_org_gap())
}

/// Y (px) del borde superior de la REJILLA o LISTA en coords de CONTENIDO.
/// Si `has_cont` es true (estantería de recientes activa con libros),
/// deja espacio para el carousel Continue Reading.
pub(crate) fn lib_grid_y0(win_w: i32, win_h: i32, has_cont: bool) -> f32 {
    if has_cont {
        lib_cont_block_h(win_w, win_h, true) + 16.0
    } else {
        8.0
    }
}

/// Ancho (px) de un chip del panel de búsqueda según el nº de caracteres de
/// su etiqueta (los de carpetas llevan la ruta, p. ej. "Download/").
pub(crate) fn lib_chip_w(win_w: i32, chars: usize) -> f32 {
    (10.0 + chars as f32 * 7.0).clamp(40.0, (win_w / 3) as f32)
}

/// Ancho fijo (px) de los chips de letras (etiquetas de 1-3 caracteres).
pub(crate) fn lib_letter_chip_w(win_w: i32) -> f32 {
    (win_w as f32 / 26.0).clamp(40.0, 56.0)
}

/// Chips del panel de BÚSQUEDA de la biblioteca, fila `row` (0 = letras
/// A-Z/#, 1 = carpetas): etiqueta + rect en px de VENTANA (con el scroll
/// horizontal de la fila ya aplicado) + si el chip está ACTIVO. Geometría
/// COMPARTIDA por `draw::render_library_zone` + `render_library_header` e `input::library_tap`. Es la
/// búsqueda SIN teclado presentada como un campo de búsqueda: el teclado
/// del sistema no entrega texto al backend native-activity de
/// android-activity (ver la cabecera del módulo), así que el filtro es por
/// inicial (A-Z/#) y por carpeta, con [All] al frente de cada fila.
pub(crate) fn lib_chips(reader: &Reader, row: usize) -> Vec<(String, ButtonRect, bool)> {
    let win_w = reader.win_w;
    let gap = grid_gap(win_w);
    let x0 = grid_pad(win_w);
    let y = if row == 0 {
        lib_search_chips_y0(reader)
    } else {
        lib_search_chips_y1(reader)
    };
    let scroll = if row == 0 {
        reader.library.lib_letters_x
    } else {
        reader.library.lib_folders_x
    };
    let mut out = Vec::new();
    let mut x = x0;
    let mut push =
        |label: String, chars: usize, active: bool, out: &mut Vec<(String, ButtonRect, bool)>| {
            let w = if row == 0 {
                lib_letter_chip_w(win_w)
            } else {
                lib_chip_w(win_w, chars)
            };
            let l = x - scroll;
            let r = l + w;
            out.push((label, (l, y, r, y + lib_chip_h(reader.win_h)), active));
            x += w + gap;
        };
    if row == 0 {
        push(
            "All".to_string(),
            3,
            reader.library.lib_letter.is_none(),
            &mut out,
        );
        for c in 'A'..='Z' {
            push(
                c.to_string(),
                1,
                reader.library.lib_letter == Some(c),
                &mut out,
            );
        }
        push(
            "#".to_string(),
            1,
            reader.library.lib_letter == Some('#'),
            &mut out,
        );
    } else {
        push(
            "All".to_string(),
            3,
            reader.library.lib_folder.is_none(),
            &mut out,
        );
        for f in reader.lib_folders() {
            let active = reader.library.lib_folder.as_deref() == Some(f.as_str());
            let n = f.chars().count();
            push(f, n, active, &mut out);
        }
    }
    out
}

/// Ancho total (px) de la fila de chips `row` (para el clamp del scroll
/// horizontal).
pub(crate) fn lib_chips_row_w(reader: &Reader, row: usize) -> f32 {
    let chips = lib_chips(reader, row);
    match chips.last() {
        Some((_, (_, _, r, _), _)) => *r + grid_pad(reader.win_w),
        None => grid_pad(reader.win_w),
    }
}

/// Chips de ORGANIZACIÓN de "My Library", fila `row` (0 = sort, 1 = filter):
/// etiqueta + rect en px de VENTANA (con el scroll vertical de la página y
/// el horizontal de la fila aplicados) + si el chip está ACTIVO. Geometría
/// COMPARTIDA por `draw::render_library_zone` + `render_library_header` e `input::library_tap`. Los
/// chips empiezan tras la etiqueta discreta de la fila ("SORT"/"FILTER",
/// `lib_org_label_w`, que dibuja el render y no es tappable).
pub(crate) fn lib_org_chips(reader: &Reader, row: usize) -> Vec<(String, ButtonRect, bool)> {
    let win_w = reader.win_w;
    let gap = 8.0;
    let x0 = grid_pad(win_w) + lib_org_label_w();
    let scroll = if row == 0 {
        reader.library.lib_sort_x
    } else {
        reader.library.lib_filter_x
    };
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let y0 = content_y0 - reader.library.lib_scroll
        + lib_org_y(win_w, reader.win_h, reader.lib_has_cont(), row);
    let chip_h = lib_org_chip_h(reader.win_h);
    let mut out = Vec::new();
    let mut x = x0;
    for (label, active) in lib_org_row(reader, row) {
        let w = (28.0 + label.chars().count() as f32 * 8.5).clamp(56.0, (win_w / 4) as f32);
        let l = x - scroll;
        out.push((label.to_string(), (l, y0, l + w, y0 + chip_h), active));
        x += w + gap;
    }
    out
}

/// Etiquetas + estado activo de la fila de organización `row` (0 = sort,
/// 1 = filter).
fn lib_org_row(reader: &Reader, row: usize) -> Vec<(&'static str, bool)> {
    if row == 0 {
        vec![
            (
                "Recientes",
                reader.library.lib_sort == LibSort::RecentlyAdded,
            ),
            ("Leídos", reader.library.lib_sort == LibSort::RecentlyRead),
            ("Título", reader.library.lib_sort == LibSort::Title),
            ("Autor", reader.library.lib_sort == LibSort::Author),
        ]
    } else {
        vec![
            ("Todos", reader.library.lib_status.is_none()),
            (
                "En lectura",
                reader.library.lib_status == Some(BookStatus::Reading),
            ),
            (
                "Terminados",
                reader.library.lib_status == Some(BookStatus::Finished),
            ),
            (
                "Por leer",
                reader.library.lib_status == Some(BookStatus::Unread),
            ),
        ]
    }
}

/// Ancho total (px) de la fila de organización `row` (para el clamp del
/// scroll horizontal).
pub(crate) fn lib_org_row_w(reader: &Reader, row: usize) -> f32 {
    match lib_org_chips(reader, row).last() {
        Some((_, (_, _, r, _), _)) => *r + grid_pad(reader.win_w),
        None => grid_pad(reader.win_w),
    }
}

pub(crate) fn lib_empty_state_geom(reader: &Reader) -> Option<EmptyStateGeom> {
    if !reader.library_list.is_empty() {
        return None;
    }
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let ctop = content_y0 - reader.library.lib_scroll;
    let h = reader.win_h as f32;
    let cy = ctop + (h - ctop) * 0.40;
    let (bw, bh) = (96.0f32, 128.0f32);
    let bx = (reader.win_w as f32 - bw) / 2.0;
    let by = cy;
    let title_y = by + bh + 34.0;
    let subtitle_y = title_y + 26.0;
    let (bw2, bh2) = (172.0f32, 44.0f32);
    let bx2 = (reader.win_w as f32 - bw2) / 2.0;
    let by2 = subtitle_y + 20.0;
    Some(EmptyStateGeom {
        book: (bx, by, bx + bw, by + bh),
        title_y,
        subtitle_y,
        button: (bx2, by2, bx2 + bw2, by2 + bh2),
    })
}

/// Nº de filas de celdas visibles en la biblioteca (cabecera + franja de
/// estado restan de la ventana; mínimo 1 fila para que siempre haya algo).
#[allow(dead_code)] // geometría pre-rediseño; la biblioteca usa `lib_visible_grid_rows` (px)
pub(crate) fn grid_visible_rows(win_w: i32, win_h: i32, has_status: bool) -> usize {
    let status_h = if has_status { picker_row_h(win_h) } else { 0 };
    let usable = (win_h - picker_header_h(win_h) - status_h) as f32;
    (usable / grid_cell_h(win_w, GRID_COLS, 1)).floor().max(1.0) as usize
}

/// Y del borde superior de la zona de rejilla (cabecera + franja de estado).
#[allow(dead_code)] // geometría pre-rediseño; la biblioteca usa `lib_content_y0` (px)
pub(crate) fn grid_rows_y0(win_h: i32, has_status: bool) -> i32 {
    picker_header_h(win_h) + if has_status { picker_row_h(win_h) } else { 0 }
}

/// Rectángulo (left, top, right, bottom) en px de ventana de la celda
/// `(row, col)` de la rejilla (compartido por `draw::render_library_zone` + `render_library_header` e
/// `input::library_tap`).
pub(crate) fn grid_cell_rect(
    win_w: i32,
    rows_y0: i32,
    row: usize,
    col: usize,
    cols: usize,
    cover_size: u8,
) -> (f32, f32, f32, f32) {
    let x = grid_pad(win_w) + col as f32 * (grid_cell_w(win_w, cols) + grid_gap(win_w));
    let y = rows_y0 as f32 + row as f32 * grid_cell_h(win_w, cols, cover_size);
    (
        x,
        y,
        x + grid_cell_w(win_w, cols),
        y + grid_cell_h(win_w, cols, cover_size),
    )
}

/// Tamaño fijo del indicador de página "N / total" (overlay abajo a la
/// izquierda). Ancho ~1/8 de ventana, alto ~1/60 (≈ 150×33 px en la tablet).
pub(crate) fn page_badge_size(win_w: i32, win_h: i32) -> (i32, i32) {
    ((win_w / 8).max(110), (win_h / 60).max(30))
}

/// Rectángulo (left, top, right, bottom) en px de ventana del indicador de
/// página (compartido por el blit y el tap de `input`).
pub(crate) fn page_badge_rect(win_w: i32, win_h: i32) -> (i32, i32, i32, i32) {
    let (bw, bh) = page_badge_size(win_w, win_h);
    let pad = (win_w / 96).max(8);
    (pad, win_h - bh - pad, pad + bw, win_h - pad)
}

/// Formatea un tamaño de fichero (B/KB/MB) para la lista.
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Trunca un nombre a `max_chars` caracteres añadiendo "…" si hace falta
/// (Canvas no hace ellipsis automática; la anchura por carácter es una
/// estimación).
pub(crate) fn truncate_name(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}
