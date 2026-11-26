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
    ButtonRect, ButtonVariant, CanvasRect, CanvasText, Elevation, TextAlign, TextFont,
    draw_astryx_button, draw_surface_card, draw_token_pill, jni_text_bitmap,
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

/// Dibuja una fila seleccionable de menú Astryx sobre su hitbox `rect`
/// (resaltado tonal `accent_muted` + marca `✓`).
fn draw_menu_row(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    rect: ButtonRect,
    label: &str,
    active: bool,
    p: &theme::ThemePalette,
) {
    let (l, t, r, b) = rect;
    let baseline = t + (b - t) * 0.68;
    if active {
        let (hl, ht, hr, hb) = menu_row_highlight(rect);
        rects.push(CanvasRect::rounded(
            hl,
            ht,
            hr,
            hb,
            theme::RADIUS_ELEMENT,
            p.accent_muted,
        ));
        texts.push(CanvasText::new(
            r - 2.0,
            baseline,
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
        l,
        baseline,
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
            p.status_border(),
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

impl ViewMenuItem {
    /// Texto del ítem (glifo en los botones de columnas).
    fn label(self) -> &'static str {
        match self {
            Self::Grid => "Cuadrícula",
            Self::List => "Lista",
            Self::ColumnsAuto => "Auto",
            Self::ColumnsDec => "−",
            Self::ColumnsInc => "+",
            Self::CoverCrop => "Recortar",
            Self::CoverFit => "Completa",
            Self::CoverHide => "Ocultar portadas",
            Self::GroupNone => "Sin agrupar",
            Self::GroupAuthor => "Autor",
            Self::SortTitle => "Título",
            Self::SortAuthor => "Autor",
            Self::SortAdded => "Fecha añadido",
            Self::SortRead => "Última lectura",
            Self::SortProgress => "Progreso",
        }
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

impl SettingsMenuItem {
    /// Texto de la fila (`ClearLibrary` cambia al pedir confirmación).
    fn label(self, confirming_clear: bool) -> &'static str {
        match self {
            Self::CoverSizeSmall => "Pequeño",
            Self::CoverSizeMedium => "Mediano",
            Self::CoverSizeLarge => "Grande",
            Self::CoverProgress => "Mostrar porcentaje de lectura",
            Self::ClearLibrary if confirming_clear => "¿Vaciar? Toca otra vez para confirmar",
            Self::ClearLibrary => "Vaciar biblioteca",
        }
    }
}

// ---------------------------------------------------------------------
// Layout único de los popovers (fuente común de dibujo e hit-test)
// ---------------------------------------------------------------------

/// Métricas Astryx compartidas por los popovers Vista y Ajustes (px).
const MENU_W: f32 = 380.0;
const MENU_PAD_X: f32 = 16.0;
const MENU_PAD_Y: f32 = 12.0;
const MENU_ITEM_H: f32 = 38.0;
const MENU_SEC_H: f32 = 24.0;
const MENU_HR_H: f32 = 8.0;
/// Alto del bloque no interactivo "ACERCA DE" bajo su encabezado.
const MENU_ABOUT_H: f32 = 34.0 + 26.0 + 24.0;

/// Sección de un popover: título, `top` (y del encabezado), `bottom` (y del
/// divisor inferior) e ítems interactivos con su hitbox.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MenuSection<I> {
    pub(crate) title: &'static str,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) items: Vec<(I, ButtonRect)>,
}

/// Layout completo de un popover: card + secciones. Lo producen
/// `view_menu_layout` / `settings_menu_layout`; `input` hace hit-test con
/// sus ítems (vía `*_menu_geometry`) y el dibujo pinta SOBRE esos mismos
/// rects, así tocar y ver nunca divergen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MenuLayout<I> {
    pub(crate) card: ButtonRect,
    pub(crate) sections: Vec<MenuSection<I>>,
}

impl<I: Copy> MenuLayout<I> {
    /// Ítems interactivos en orden, con el rect usado para el hit-test.
    pub(crate) fn items(&self) -> Vec<(I, ButtonRect)> {
        self.sections
            .iter()
            .flat_map(|s| s.items.iter().copied())
            .collect()
    }
}

/// Constructor incremental de un `MenuLayout` anclado bajo su botón.
struct MenuBuilder<I> {
    l: f32,
    t: f32,
    r: f32,
    y: f32,
    sections: Vec<MenuSection<I>>,
}

impl<I> MenuBuilder<I> {
    /// Card alineado a la derecha del botón `(_, _, r, b)`, 8 px por debajo.
    fn new(win_w: i32, (_, _, r, b): ButtonRect) -> Self {
        let menu_w = MENU_W.min(win_w as f32 - 32.0);
        let t = b + 8.0;
        Self {
            l: r - menu_w,
            t,
            r,
            y: t + MENU_PAD_Y,
            sections: Vec::new(),
        }
    }

    /// Abre una sección (divisor tras la anterior + encabezado).
    fn section(&mut self, title: &'static str) {
        if let Some(prev) = self.sections.last_mut() {
            prev.bottom = self.y;
            self.y += MENU_HR_H;
        }
        self.sections.push(MenuSection {
            title,
            top: self.y,
            bottom: self.y,
            items: Vec::new(),
        });
        self.y += MENU_SEC_H;
    }

    /// Añade un ítem con hitbox explícito a la sección actual.
    fn item(&mut self, item: I, rect: ButtonRect) {
        if let Some(sec) = self.sections.last_mut() {
            sec.items.push((item, rect));
        }
    }

    /// Fila de ancho completo (menos el padding lateral).
    fn row(&mut self, item: I) {
        let rect = (
            self.l + MENU_PAD_X,
            self.y,
            self.r - MENU_PAD_X,
            self.y + MENU_ITEM_H,
        );
        self.item(item, rect);
        self.y += MENU_ITEM_H;
    }

    /// Cierra el layout tras `extra_h` px de contenido no interactivo.
    fn finish(mut self, extra_h: f32) -> MenuLayout<I> {
        self.y += extra_h;
        if let Some(last) = self.sections.last_mut() {
            last.bottom = self.y;
        }
        MenuLayout {
            card: (self.l, self.t, self.r, self.y + MENU_PAD_Y),
            sections: self.sections,
        }
    }
}

/// Layout del menú Vista "⋯" (coords en px de ventana).
pub(crate) fn view_menu_layout(win_w: i32, win_h: i32) -> MenuLayout<ViewMenuItem> {
    let mut b = MenuBuilder::new(win_w, view_menu_button_rect(win_w, win_h));

    b.section("MODO DE VISTA");
    b.row(ViewMenuItem::Grid);
    b.row(ViewMenuItem::List);

    // Columnas: [Auto] a la izquierda, [−] n [+] a la derecha.
    b.section("COLUMNAS");
    let (l, r, y) = (b.l + MENU_PAD_X, b.r - MENU_PAD_X, b.y);
    let (t, bt) = (y + 2.0, y + MENU_ITEM_H - 2.0);
    b.item(ViewMenuItem::ColumnsAuto, (l, t, l + 84.0, bt));
    b.item(ViewMenuItem::ColumnsDec, (r - 132.0, t, r - 88.0, bt));
    b.item(ViewMenuItem::ColumnsInc, (r - 44.0, t, r, bt));
    b.y += MENU_ITEM_H;

    b.section("PORTADAS");
    b.row(ViewMenuItem::CoverCrop);
    b.row(ViewMenuItem::CoverFit);
    b.row(ViewMenuItem::CoverHide);

    b.section("AGRUPAR POR");
    b.row(ViewMenuItem::GroupNone);
    b.row(ViewMenuItem::GroupAuthor);

    b.section("ORDENAR POR");
    for item in [
        ViewMenuItem::SortTitle,
        ViewMenuItem::SortAuthor,
        ViewMenuItem::SortAdded,
        ViewMenuItem::SortRead,
        ViewMenuItem::SortProgress,
    ] {
        b.row(item);
    }
    b.finish(0.0)
}

/// Layout del menú Ajustes "☰" (coords en px de ventana).
pub(crate) fn settings_menu_layout(win_w: i32, win_h: i32) -> MenuLayout<SettingsMenuItem> {
    let mut b = MenuBuilder::new(win_w, settings_menu_button_rect(win_w, win_h));

    b.section("TAMAÑO DE PORTADA");
    b.row(SettingsMenuItem::CoverSizeSmall);
    b.row(SettingsMenuItem::CoverSizeMedium);
    b.row(SettingsMenuItem::CoverSizeLarge);

    b.section("PROGRESO");
    b.row(SettingsMenuItem::CoverProgress);

    b.section("ADMINISTRAR BIBLIOTECA");
    b.row(SettingsMenuItem::ClearLibrary);

    // ACERCA DE (no interactivo)
    b.section("ACERCA DE");
    b.finish(MENU_ABOUT_H)
}

/// Geometría del menú Vista "⋯" para el hit-test de `input`: el card y los
/// rects de cada ítem interactivo, tomados del mismo layout que se dibuja.
#[allow(clippy::type_complexity)]
pub(crate) fn view_menu_geometry(
    win_w: i32,
    win_h: i32,
) -> (
    (f32, f32, f32, f32),
    Vec<(ViewMenuItem, (f32, f32, f32, f32))>,
) {
    let layout = view_menu_layout(win_w, win_h);
    (layout.card, layout.items())
}

/// Geometría del menú Ajustes "☰" para el hit-test de `input` (mismo
/// layout que se dibuja).
#[allow(clippy::type_complexity)]
pub(crate) fn settings_menu_geometry(
    win_w: i32,
    win_h: i32,
) -> (
    (f32, f32, f32, f32),
    Vec<(SettingsMenuItem, (f32, f32, f32, f32))>,
) {
    let layout = settings_menu_layout(win_w, win_h);
    (layout.card, layout.items())
}

/// Resaltado tonal de una fila seleccionada a partir de su hitbox.
fn menu_row_highlight((l, t, r, b): ButtonRect) -> ButtonRect {
    (l - 6.0, t + 2.0, r + 6.0, b - 2.0)
}

/// Estado del menú Vista que decide qué ítems se marcan como activos.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ViewMenuState {
    pub(crate) grid: bool,
    pub(crate) auto_columns: bool,
    pub(crate) columns: u32,
    pub(crate) cover_fit: LibraryCoverFit,
    pub(crate) hide_covers: bool,
    pub(crate) group_by: LibraryGroupBy,
    pub(crate) sort: LibSort,
}

impl ViewMenuState {
    fn from_reader(reader: &Reader) -> Self {
        Self {
            grid: reader.is_grid(),
            auto_columns: reader.auto_columns,
            columns: reader.columns,
            cover_fit: reader.cover_fit,
            hide_covers: reader.hide_covers,
            group_by: reader.group_by,
            sort: reader.library.lib_sort,
        }
    }

    fn is_active(&self, item: ViewMenuItem) -> bool {
        match item {
            ViewMenuItem::Grid => self.grid,
            ViewMenuItem::List => !self.grid,
            ViewMenuItem::ColumnsAuto => self.auto_columns,
            ViewMenuItem::ColumnsDec | ViewMenuItem::ColumnsInc => !self.auto_columns,
            ViewMenuItem::CoverCrop => self.cover_fit == LibraryCoverFit::Crop && !self.hide_covers,
            ViewMenuItem::CoverFit => self.cover_fit == LibraryCoverFit::Fit && !self.hide_covers,
            ViewMenuItem::CoverHide => self.hide_covers,
            ViewMenuItem::GroupNone => self.group_by == LibraryGroupBy::None,
            ViewMenuItem::GroupAuthor => self.group_by == LibraryGroupBy::Author,
            ViewMenuItem::SortTitle => self.sort == LibSort::Title,
            ViewMenuItem::SortAuthor => self.sort == LibSort::Author,
            ViewMenuItem::SortAdded => self.sort == LibSort::RecentlyAdded,
            ViewMenuItem::SortRead => self.sort == LibSort::RecentlyRead,
            ViewMenuItem::SortProgress => self.sort == LibSort::Progress,
        }
    }
}

/// Estado del menú Ajustes que decide qué ítems se marcan como activos.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SettingsMenuState {
    pub(crate) cover_size: u8,
    pub(crate) cover_progress: bool,
    pub(crate) confirming_clear: bool,
}

impl SettingsMenuState {
    fn from_reader(reader: &Reader) -> Self {
        Self {
            cover_size: reader.cover_size,
            cover_progress: reader.cover_progress,
            confirming_clear: reader
                .clear_confirm_until
                .map(|until| std::time::Instant::now() <= until)
                .unwrap_or(false),
        }
    }

    fn is_active(&self, item: SettingsMenuItem) -> bool {
        match item {
            SettingsMenuItem::CoverSizeSmall => self.cover_size == 0,
            SettingsMenuItem::CoverSizeMedium => self.cover_size == 1,
            SettingsMenuItem::CoverSizeLarge => self.cover_size == 2,
            SettingsMenuItem::CoverProgress => self.cover_progress,
            SettingsMenuItem::ClearLibrary => self.confirming_clear,
        }
    }
}

/// Card del popover + encabezados de sección + divisores entre secciones.
fn draw_menu_frame<I>(
    layout: &MenuLayout<I>,
    p: &theme::ThemePalette,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    let (ml, _mt, mr, _mb) = layout.card;
    draw_surface_card(
        rects,
        layout.card,
        theme::RADIUS_CARD,
        p.bg_popover,
        p.border_emphasized,
        Elevation::High,
        p.is_dark,
    );
    let n = layout.sections.len();
    for (i, s) in layout.sections.iter().enumerate() {
        draw_menu_section(texts, ml + MENU_PAD_X, s.top, s.title, p);
        if i + 1 < n {
            draw_menu_divider(rects, ml, mr, MENU_PAD_X, s.bottom, p);
        }
    }
}

/// Dibuja el popover Vista (⋯) sobre `layout` (los mismos rects del hit-test).
pub(crate) fn draw_view_menu_layout(
    layout: &MenuLayout<ViewMenuItem>,
    st: &ViewMenuState,
    p: &theme::ThemePalette,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    draw_menu_frame(layout, p, rects, texts);
    let step_variant = if st.auto_columns {
        ButtonVariant::Ghost
    } else {
        ButtonVariant::Secondary
    };
    let (mut dec, mut inc) = (None, None);
    for (item, rect) in layout.items() {
        match item {
            ViewMenuItem::ColumnsAuto => draw_astryx_button(
                rects,
                texts,
                rect,
                if st.auto_columns {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
                p,
                theme::FONT_CAPTION,
                true,
                item.label(),
            ),
            ViewMenuItem::ColumnsDec | ViewMenuItem::ColumnsInc => {
                if item == ViewMenuItem::ColumnsDec {
                    dec = Some(rect);
                } else {
                    inc = Some(rect);
                }
                draw_astryx_button(
                    rects,
                    texts,
                    rect,
                    step_variant,
                    p,
                    theme::FONT_BODY,
                    true,
                    item.label(),
                );
            }
            _ => draw_menu_row(rects, texts, rect, item.label(), st.is_active(item), p),
        }
    }
    // Contador de columnas centrado entre [−] y [+].
    if let (Some(d), Some(i)) = (dec, inc) {
        let row_t = d.1 - 2.0;
        texts.push(
            CanvasText::new(
                (d.2 + i.0) * 0.5,
                row_t + MENU_ITEM_H * 0.68,
                theme::FONT_BODY,
                if st.auto_columns {
                    p.text_secondary
                } else {
                    p.text_primary
                },
                TextAlign::Center,
                true,
                format!("{}", st.columns.clamp(1, 4)),
            )
            .with_font(TextFont::Mono),
        );
    }
}

/// Dibuja las primitivas del popover Vista (⋯) en lenguaje Astryx (`bg_popover` + `Elevation::High`).
pub(crate) fn draw_view_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    draw_view_menu_layout(
        &view_menu_layout(reader.win_w, reader.win_h),
        &ViewMenuState::from_reader(reader),
        &reader.theme.palette(),
        rects,
        texts,
    );
}

/// Dibuja el popover Ajustes (☰) sobre `layout` (los mismos rects del hit-test).
pub(crate) fn draw_settings_menu_layout(
    layout: &MenuLayout<SettingsMenuItem>,
    st: &SettingsMenuState,
    p: &theme::ThemePalette,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    draw_menu_frame(layout, p, rects, texts);
    for (item, rect) in layout.items() {
        let label = item.label(st.confirming_clear);
        if item != SettingsMenuItem::ClearLibrary {
            draw_menu_row(rects, texts, rect, label, st.is_active(item), p);
            continue;
        }
        // Vaciar biblioteca: semántica Danger Astryx.
        if st.confirming_clear {
            draw_surface_card(
                rects,
                menu_row_highlight(rect),
                theme::RADIUS_ELEMENT,
                p.error_bg,
                p.error_border,
                Elevation::None,
                p.is_dark,
            );
        }
        texts.push(CanvasText::new(
            rect.0,
            rect.1 + (rect.3 - rect.1) * 0.68,
            theme::FONT_BODY,
            p.error_text,
            TextAlign::Left,
            st.confirming_clear,
            label,
        ));
    }

    // ACERCA DE: bloque informativo bajo el encabezado de la última sección.
    let Some(about) = layout.sections.last() else {
        return;
    };
    let x = layout.card.0 + MENU_PAD_X;
    let mut y = about.top + MENU_SEC_H;
    texts.push(
        CanvasText::new(
            x,
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
        x,
        y + theme::FONT_CAPTION * 0.85,
        theme::FONT_CAPTION,
        p.text_secondary,
        TextAlign::Left,
        false,
        "Código abierto · AGPL-3.0",
    ));
    y += 24.0;
    texts.push(CanvasText::new(
        x,
        y + theme::FONT_MICRO * 0.85,
        theme::FONT_MICRO,
        p.text_secondary,
        TextAlign::Left,
        false,
        crate::draw::discover::ARXIV_ATTRIBUTION,
    ));
}

/// Dibuja las primitivas del popover Ajustes (☰) en lenguaje Astryx.
pub(crate) fn draw_settings_menu(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
) {
    draw_settings_menu_layout(
        &settings_menu_layout(reader.win_w, reader.win_h),
        &SettingsMenuState::from_reader(reader),
        &reader.theme.palette(),
        rects,
        texts,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::AppTheme;

    /// Ventanas típicas de la tablet (retrato y apaisado).
    const SIZES: [(i32, i32); 4] = [(1600, 2560), (2560, 1600), (1200, 1920), (1920, 1200)];

    fn inside((x, y): (f32, f32), (l, t, r, b): ButtonRect) -> bool {
        x >= l && x <= r && y >= t && y <= b
    }

    fn contains((ol, ot, or, ob): ButtonRect, (l, t, r, b): ButtonRect) -> bool {
        l >= ol && t >= ot && r <= or && b <= ob
    }

    fn bounds(r: &CanvasRect) -> ButtonRect {
        (r.left, r.top, r.right, r.bottom)
    }

    /// Desplaza todo el layout: si el dibujo recalculase su propia
    /// geometría en vez de usar la recibida, dejaría de coincidir.
    fn shifted<I: Copy>(layout: &MenuLayout<I>, dx: f32, dy: f32) -> MenuLayout<I> {
        let mv = |(l, t, r, b): ButtonRect| (l + dx, t + dy, r + dx, b + dy);
        MenuLayout {
            card: mv(layout.card),
            sections: layout
                .sections
                .iter()
                .map(|s| MenuSection {
                    title: s.title,
                    top: s.top + dy,
                    bottom: s.bottom + dy,
                    items: s.items.iter().map(|&(i, r)| (i, mv(r))).collect(),
                })
                .collect(),
        }
    }

    fn view_state_activating(item: ViewMenuItem) -> ViewMenuState {
        let mut st = ViewMenuState {
            columns: 3,
            ..Default::default()
        };
        match item {
            ViewMenuItem::Grid => st.grid = true,
            ViewMenuItem::List => st.grid = false,
            ViewMenuItem::ColumnsAuto => st.auto_columns = true,
            ViewMenuItem::ColumnsDec | ViewMenuItem::ColumnsInc => st.auto_columns = false,
            ViewMenuItem::CoverCrop => st.cover_fit = LibraryCoverFit::Crop,
            ViewMenuItem::CoverFit => st.cover_fit = LibraryCoverFit::Fit,
            ViewMenuItem::CoverHide => st.hide_covers = true,
            ViewMenuItem::GroupNone => st.group_by = LibraryGroupBy::None,
            ViewMenuItem::GroupAuthor => st.group_by = LibraryGroupBy::Author,
            ViewMenuItem::SortTitle => st.sort = LibSort::Title,
            ViewMenuItem::SortAuthor => st.sort = LibSort::Author,
            ViewMenuItem::SortAdded => st.sort = LibSort::RecentlyAdded,
            ViewMenuItem::SortRead => st.sort = LibSort::RecentlyRead,
            ViewMenuItem::SortProgress => st.sort = LibSort::Progress,
        }
        st
    }

    fn settings_state_activating(item: SettingsMenuItem) -> SettingsMenuState {
        let mut st = SettingsMenuState {
            cover_size: 9,
            ..Default::default()
        };
        match item {
            SettingsMenuItem::CoverSizeSmall => st.cover_size = 0,
            SettingsMenuItem::CoverSizeMedium => st.cover_size = 1,
            SettingsMenuItem::CoverSizeLarge => st.cover_size = 2,
            SettingsMenuItem::CoverProgress => st.cover_progress = true,
            SettingsMenuItem::ClearLibrary => st.confirming_clear = true,
        }
        st
    }

    #[test]
    fn menu_geometry_for_input_is_the_drawn_layout() {
        for (w, h) in SIZES {
            let v = view_menu_layout(w, h);
            assert_eq!(view_menu_geometry(w, h), (v.card, v.items()));
            let s = settings_menu_layout(w, h);
            assert_eq!(settings_menu_geometry(w, h), (s.card, s.items()));
        }
    }

    #[test]
    fn view_menu_draws_every_item_on_its_hit_rect() {
        let p = AppTheme::DefaultLight.palette();
        for (w, h) in SIZES {
            let layout = shifted(&view_menu_layout(w, h), 37.0, 53.0);
            let items = layout.items();
            assert_eq!(items.len(), 15, "el menú Vista tiene 15 ítems");
            for &(item, rect) in &items {
                let st = view_state_activating(item);
                let (mut rects, mut texts) = (Vec::new(), Vec::new());
                draw_view_menu_layout(&layout, &st, &p, &mut rects, &mut texts);
                assert!(
                    texts.iter().any(|t| inside((t.x, t.y), rect)),
                    "{item:?}: ninguna etiqueta dibujada dentro de su hitbox {rect:?}"
                );
                let want = match item {
                    ViewMenuItem::ColumnsAuto
                    | ViewMenuItem::ColumnsDec
                    | ViewMenuItem::ColumnsInc => rect,
                    _ => menu_row_highlight(rect),
                };
                assert!(
                    rects.iter().any(|r| bounds(r) == want),
                    "{item:?}: no se dibuja el rect {want:?} derivado de su hitbox"
                );
            }
        }
    }

    #[test]
    fn settings_menu_draws_every_item_on_its_hit_rect() {
        let p = AppTheme::DefaultLight.palette();
        for (w, h) in SIZES {
            let layout = shifted(&settings_menu_layout(w, h), -41.0, 29.0);
            let items = layout.items();
            assert_eq!(items.len(), 5, "el menú Ajustes tiene 5 ítems");
            for &(item, rect) in &items {
                let st = settings_state_activating(item);
                let (mut rects, mut texts) = (Vec::new(), Vec::new());
                draw_settings_menu_layout(&layout, &st, &p, &mut rects, &mut texts);
                assert!(
                    texts.iter().any(|t| inside((t.x, t.y), rect)),
                    "{item:?}: ninguna etiqueta dibujada dentro de su hitbox {rect:?}"
                );
                let want = menu_row_highlight(rect);
                assert!(
                    rects.iter().any(|r| bounds(r) == want),
                    "{item:?}: no se dibuja el resaltado {want:?} derivado de su hitbox"
                );
            }
        }
    }

    #[test]
    fn menus_fit_in_window_and_draw_inside_their_card() {
        let p = AppTheme::DefaultLight.palette();
        for (w, h) in SIZES {
            let win = (0.0, 0.0, w as f32, h as f32);

            let v = view_menu_layout(w, h);
            assert!(
                contains(win, v.card),
                "{w}x{h}: menú Vista fuera de la ventana"
            );
            assert_eq!(v.sections.len(), 5, "{w}x{h}: secciones del menú Vista");
            for (item, r) in v.items() {
                assert!(contains(v.card, r), "{w}x{h}: {item:?} fuera del card");
            }
            let (mut rects, mut texts) = (Vec::new(), Vec::new());
            draw_view_menu_layout(&v, &ViewMenuState::default(), &p, &mut rects, &mut texts);
            assert!(!texts.is_empty(), "{w}x{h}: el menú Vista no dibuja texto");
            for t in &texts {
                assert!(
                    inside((t.x, t.y), v.card),
                    "{w}x{h}: texto {:?} fuera del card",
                    t.text
                );
            }

            let s = settings_menu_layout(w, h);
            assert!(
                contains(win, s.card),
                "{w}x{h}: menú Ajustes fuera de la ventana"
            );
            assert_eq!(s.sections.len(), 4, "{w}x{h}: secciones del menú Ajustes");
            for (item, r) in s.items() {
                assert!(contains(s.card, r), "{w}x{h}: {item:?} fuera del card");
            }
            let (mut rects, mut texts) = (Vec::new(), Vec::new());
            draw_settings_menu_layout(
                &s,
                &SettingsMenuState::default(),
                &p,
                &mut rects,
                &mut texts,
            );
            assert!(
                !texts.is_empty(),
                "{w}x{h}: el menú Ajustes no dibuja texto"
            );
            for t in &texts {
                assert!(
                    inside((t.x, t.y), s.card),
                    "{w}x{h}: texto {:?} fuera del card",
                    t.text
                );
            }
        }
    }
}
