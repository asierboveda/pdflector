// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Barra de herramientas del visor (ADR-012, ADR-022): tarjeta flotante
//! acoplada a un borde de la ventana — vertical en los laterales, horizontal
//! arriba/abajo — con Bolígrafo, Subrayador, Goma y Recorte | deshacer,
//! rehacer | cerrar. Cerrada queda un solo botón (el lápiz navega). Tocar de
//! nuevo el Bolígrafo o el Subrayador activos abre su popover de color (y
//! grosor del bolígrafo) hacia el interior de la ventana.
//!
//! La geometría es pura (`toolbar_layout`, `toolbar_popover_layout`) y la
//! comparten el render y el hit-test de `input`. Los bitmaps se generan con
//! Canvas+JNI solo cuando cambia el estado de la barra (`Reader::
//! invalidate_toolbar`) y se presentan como overlays cacheados: el trazo no
//! paga nada por la barra.

use super::{ButtonRect, CanvasLine, CanvasRect, draw_card_shadow, jni_canvas_bitmap};
use crate::annotations::{HIGHLIGHT_PALETTE, INK_PALETTE, INK_WIDTHS, PenMode};
use crate::reader::{Reader, viewer_bottom_chrome_h, viewer_top_chrome_h};
use crate::theme;
use pdf_core::{Bitmap, Color};

/// Borde de la ventana al que está acoplada la barra.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ToolbarDock {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl ToolbarDock {
    fn vertical(self) -> bool {
        matches!(self, ToolbarDock::Left | ToolbarDock::Right)
    }

    /// Borde más cercano al punto (x, y) de la ventana: donde se acopla la
    /// barra al soltarla tras arrastrarla.
    pub(crate) fn nearest(x: f32, y: f32, win_w: i32, win_h: i32) -> Self {
        let (w, h) = (win_w as f32, win_h as f32);
        [
            (x, ToolbarDock::Left),
            (w - x, ToolbarDock::Right),
            (y, ToolbarDock::Top),
            (h - y, ToolbarDock::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or(ToolbarDock::Left, |(_, dock)| dock)
    }
}

/// Botones de la barra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolbarButton {
    Pen,
    Highlighter,
    Eraser,
    Lasso,
    Undo,
    Redo,
    /// Cerrar (barra abierta) o abrir (barra cerrada).
    Toggle,
}

/// Popover que se abre junto a la barra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolbarPopover {
    /// Colores y grosores del bolígrafo.
    Pen,
    /// Colores del subrayador.
    Highlighter,
}

/// Celda de un popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PopoverCell {
    /// Índice en `INK_PALETTE` o `HIGHLIGHT_PALETTE`.
    Color(usize),
    /// Índice en `INK_WIDTHS`.
    Width(usize),
}

/// Lado de un botón (px; 48 dp a la densidad 2× de la TCL).
const BTN: f32 = 96.0;
/// Separación entre botones del mismo grupo.
const GAP: f32 = 4.0;
/// Relleno interior de la tarjeta.
const PAD: f32 = 8.0;
/// Hueco del separador entre grupos.
const SEP: f32 = 17.0;
/// Margen respecto al borde de la ventana.
const MARGIN: f32 = 16.0;
/// Radio de las esquinas de la tarjeta.
const CARD_R: f32 = 28.0;
/// Margen del bitmap alrededor de la tarjeta para la sombra.
const SHADOW: f32 = 10.0;
/// Celda del popover (swatch o muestra de grosor).
const CELL: f32 = 80.0;
/// Separación entre la barra y el popover.
const POPOVER_GAP: f32 = 12.0;
const GROUPS: [&[ToolbarButton]; 3] = [
    &[
        ToolbarButton::Pen,
        ToolbarButton::Highlighter,
        ToolbarButton::Eraser,
        ToolbarButton::Lasso,
    ],
    &[ToolbarButton::Undo, ToolbarButton::Redo],
    &[ToolbarButton::Toggle],
];
const CLOSED: [&[ToolbarButton]; 1] = [&[ToolbarButton::Toggle]];

/// Geometría de la barra en px de ventana.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolbarLayout {
    pub(crate) dock: ToolbarDock,
    /// Tarjeta completa (left, top, right, bottom).
    pub(crate) card: ButtonRect,
    pub(crate) buttons: Vec<(ToolbarButton, ButtonRect)>,
    /// Separadores entre grupos como segmentos (x0, y0, x1, y1).
    pub(crate) separators: Vec<ButtonRect>,
}

impl ToolbarLayout {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        inside(self.card, x, y)
    }

    pub(crate) fn button_at(&self, x: f32, y: f32) -> Option<ToolbarButton> {
        self.buttons
            .iter()
            .find(|(_, r)| inside(*r, x, y))
            .map(|(b, _)| *b)
    }

    fn rect_of(&self, button: ToolbarButton) -> Option<ButtonRect> {
        self.buttons
            .iter()
            .find(|(b, _)| *b == button)
            .map(|(_, r)| *r)
    }
}

/// Geometría del popover en px de ventana.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopoverLayout {
    pub(crate) card: ButtonRect,
    pub(crate) cells: Vec<(PopoverCell, ButtonRect)>,
}

impl PopoverLayout {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        inside(self.card, x, y)
    }

    pub(crate) fn cell_at(&self, x: f32, y: f32) -> Option<PopoverCell> {
        self.cells
            .iter()
            .find(|(_, r)| inside(*r, x, y))
            .map(|(c, _)| *c)
    }
}

fn inside((l, t, r, b): ButtonRect, x: f32, y: f32) -> bool {
    x >= l && x < r && y >= t && y < b
}

/// Barra acoplada al borde `dock` y centrada a lo largo de él. Arriba y
/// abajo deja libre el espacio de las barras de chrome del visor. Cerrada
/// queda un solo botón (abrir).
pub(crate) fn toolbar_layout(
    win_w: i32,
    win_h: i32,
    dock: ToolbarDock,
    collapsed: bool,
) -> ToolbarLayout {
    let groups: &[&[ToolbarButton]] = if collapsed { &CLOSED } else { &GROUPS };
    let n: usize = groups.iter().map(|g| g.len()).sum();
    let gaps = (n - groups.len()) as f32;
    let length = 2.0 * PAD + n as f32 * BTN + gaps * GAP + (groups.len() - 1) as f32 * SEP;
    let cross = BTN + 2.0 * PAD;
    let (w, h) = (win_w as f32, win_h as f32);
    let (left, top) = match dock {
        ToolbarDock::Left => (MARGIN, ((h - length) / 2.0).max(MARGIN)),
        ToolbarDock::Right => (w - MARGIN - cross, ((h - length) / 2.0).max(MARGIN)),
        ToolbarDock::Top => (((w - length) / 2.0).max(MARGIN), viewer_top_chrome_h(win_h)),
        ToolbarDock::Bottom => (
            ((w - length) / 2.0).max(MARGIN),
            h - viewer_bottom_chrome_h(win_h) - cross,
        ),
    };
    let (left, top) = (left.round(), top.round());
    let vertical = dock.vertical();
    let card = if vertical {
        (left, top, left + cross, top + length)
    } else {
        (left, top, left + length, top + cross)
    };
    let mut buttons = Vec::with_capacity(n);
    let mut separators = Vec::with_capacity(groups.len());
    let mut m = if vertical { top } else { left } + PAD;
    for (gi, group) in groups.iter().enumerate() {
        if gi > 0 {
            let c = m + SEP / 2.0;
            separators.push(if vertical {
                (left + PAD + 16.0, c, left + cross - PAD - 16.0, c)
            } else {
                (c, top + PAD + 16.0, c, top + cross - PAD - 16.0)
            });
            m += SEP;
        }
        for (bi, &button) in group.iter().enumerate() {
            if bi > 0 {
                m += GAP;
            }
            buttons.push((
                button,
                if vertical {
                    (left + PAD, m, left + PAD + BTN, m + BTN)
                } else {
                    (m, top + PAD, m + BTN, top + PAD + BTN)
                },
            ));
            m += BTN;
        }
    }
    ToolbarLayout {
        dock,
        card,
        buttons,
        separators,
    }
}

/// Celdas del popover por filas.
fn popover_rows(kind: ToolbarPopover) -> Vec<Vec<PopoverCell>> {
    let colors = (0..INK_PALETTE.len()).map(PopoverCell::Color).collect();
    match kind {
        ToolbarPopover::Pen => vec![
            colors,
            (0..INK_WIDTHS.len()).map(PopoverCell::Width).collect(),
        ],
        ToolbarPopover::Highlighter => vec![colors],
    }
}

/// Popover junto a la barra, hacia el interior de la ventana, centrado en
/// la herramienta que lo abre y recortado a la ventana.
pub(crate) fn toolbar_popover_layout(
    win_w: i32,
    win_h: i32,
    toolbar: &ToolbarLayout,
    kind: ToolbarPopover,
) -> PopoverLayout {
    let anchor = match kind {
        ToolbarPopover::Pen => ToolbarButton::Pen,
        ToolbarPopover::Highlighter => ToolbarButton::Highlighter,
    };
    let (al, at, ar, ab) = toolbar.rect_of(anchor).unwrap_or(toolbar.card);
    let rows = popover_rows(kind);
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1);
    let w = 2.0 * PAD + cols as f32 * CELL;
    let h = 2.0 * PAD + rows.len() as f32 * CELL;
    let (cl, ct, cr, cb) = toolbar.card;
    let clamp_x = |x: f32| x.clamp(MARGIN, (win_w as f32 - w - MARGIN).max(MARGIN));
    let clamp_y = |y: f32| y.clamp(MARGIN, (win_h as f32 - h - MARGIN).max(MARGIN));
    let (left, top) = match toolbar.dock {
        ToolbarDock::Left => (cr + POPOVER_GAP, clamp_y((at + ab - h) / 2.0)),
        ToolbarDock::Right => (cl - POPOVER_GAP - w, clamp_y((at + ab - h) / 2.0)),
        ToolbarDock::Top => (clamp_x((al + ar - w) / 2.0), cb + POPOVER_GAP),
        ToolbarDock::Bottom => (clamp_x((al + ar - w) / 2.0), ct - POPOVER_GAP - h),
    };
    let (left, top) = (left.round(), top.round());
    let mut cells = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        // Filas más cortas centradas.
        let offset = (cols - row.len()) as f32 * CELL / 2.0;
        for (ci, &cell) in row.iter().enumerate() {
            let l = left + PAD + offset + ci as f32 * CELL;
            let t = top + PAD + ri as f32 * CELL;
            cells.push((cell, (l, t, l + CELL, t + CELL)));
        }
    }
    PopoverLayout {
        card: (left, top, left + w, top + h),
        cells,
    }
}

fn argb(c: Color, a: u8) -> u32 {
    (a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

/// Mezcla opaca `t` de `a` hacia `b` (ARGB). Los iconos atenuados usan un
/// color opaco: con alfa, los extremos solapados de los segmentos se
/// verían más oscuros (efecto punteado).
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (ca, cb) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((ca as f32 + (cb as f32 - ca as f32) * t).round() as u32) << shift
    };
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

/// Fondo de tarjeta con sombra (coordenadas locales del bitmap).
fn card(rects: &mut Vec<CanvasRect>, w: f32, h: f32, p: &theme::ThemePalette) {
    let (l, t, r, b) = (SHADOW, SHADOW, w - SHADOW, h - SHADOW);
    draw_card_shadow(rects, l, t, r, b, CARD_R, p.is_dark);
    rects.push(CanvasRect::rounded(l, t, r, b, CARD_R, p.base_300));
    rects.push(CanvasRect::rounded(
        l + 1.0,
        t + 1.0,
        r - 1.0,
        b - 1.0,
        CARD_R - 1.0,
        p.base_100,
    ));
}

/// Círculo relleno (rect redondeado cuadrado).
fn circle(rects: &mut Vec<CanvasRect>, cx: f32, cy: f32, r: f32, color: u32) {
    rects.push(CanvasRect::rounded(
        cx - r,
        cy - r,
        cx + r,
        cy + r,
        r,
        color,
    ));
}

/// Polilínea como segmentos con extremos redondeados.
fn polyline(lines: &mut Vec<CanvasLine>, pts: &[(f32, f32)], width: f32, color: u32) {
    for w in pts.windows(2) {
        lines.push(CanvasLine::new(w[0], w[1], width, color));
    }
}

/// Chevron centrado en (cx, cy) apuntando hacia `toward` (vector unitario).
fn chevron(lines: &mut Vec<CanvasLine>, cx: f32, cy: f32, toward: (f32, f32), s: f32, color: u32) {
    let (dx, dy) = toward;
    let (px, py) = (-dy, dx);
    let tip = (cx + dx * s / 2.0, cy + dy * s / 2.0);
    let a = (cx - dx * s / 2.0 + px * s, cy - dy * s / 2.0 + py * s);
    let b = (cx - dx * s / 2.0 - px * s, cy - dy * s / 2.0 - py * s);
    polyline(lines, &[a, tip, b], 4.0, color);
}

/// Flecha curva de deshacer (↶) o, reflejada, de rehacer (↷).
fn history_arrow(lines: &mut Vec<CanvasLine>, cx: f32, cy: f32, mirror: bool, color: u32) {
    let sx = if mirror { -1.0 } else { 1.0 };
    let (ox, oy, r) = (cx + 2.0 * sx, cy + 6.0, 14.0);
    let mut pts = Vec::with_capacity(12);
    for i in 0..=10 {
        // De 180° (izquierda) a -30°, pasando por arriba.
        let a = std::f32::consts::PI * (1.0 - 1.1667 * i as f32 / 10.0);
        pts.push((ox + sx * r * a.cos(), oy - r * a.sin()));
    }
    polyline(lines, &pts, 4.0, color);
    let tip = (ox - sx * r, oy + 3.0);
    lines.push(CanvasLine::new(tip, (tip.0 - 8.0, tip.1 - 9.0), 4.0, color));
    lines.push(CanvasLine::new(tip, (tip.0 + 8.0, tip.1 - 9.0), 4.0, color));
}

/// Icono de herramienta; Bolígrafo y Subrayador llevan una raya del color
/// que dibujarán.
fn tool_icon(
    lines: &mut Vec<CanvasLine>,
    (cx, cy): (f32, f32),
    mode: PenMode,
    fg: u32,
    bg: u32,
    reader: &Reader,
) {
    match mode {
        PenMode::Ink => {
            lines.push(CanvasLine::new(
                (cx + 13.0, cy - 17.0),
                (cx - 5.0, cy + 1.0),
                8.0,
                fg,
            ));
            lines.push(CanvasLine::new(
                (cx - 5.0, cy + 1.0),
                (cx - 11.0, cy + 7.0),
                3.0,
                fg,
            ));
            lines.push(CanvasLine::new(
                (cx - 16.0, cy + 17.0),
                (cx + 16.0, cy + 17.0),
                5.0,
                argb(reader.ink_color, 255),
            ));
        }
        PenMode::Highlight => {
            lines.push(CanvasLine::new(
                (cx + 12.0, cy - 17.0),
                (cx - 3.0, cy - 2.0),
                13.0,
                fg,
            ));
            lines.push(CanvasLine::new(
                (cx - 3.0, cy - 2.0),
                (cx - 9.0, cy + 4.0),
                6.0,
                fg,
            ));
            lines.push(CanvasLine::new(
                (cx - 16.0, cy + 16.0),
                (cx + 16.0, cy + 16.0),
                8.0,
                argb(reader.highlight_color, 255),
            ));
        }
        PenMode::Eraser => {
            // Goma: píldora inclinada en contorno + línea de base.
            let (a, b) = ((cx + 9.0, cy - 13.0), (cx - 7.0, cy + 3.0));
            lines.push(CanvasLine::new(a, b, 20.0, fg));
            lines.push(CanvasLine::new(a, b, 13.0, bg));
            lines.push(CanvasLine::new(
                (cx - 1.0, cy - 3.0),
                (cx - 7.0, cy + 3.0),
                13.0,
                fg,
            ));
            lines.push(CanvasLine::new(
                (cx - 16.0, cy + 17.0),
                (cx + 16.0, cy + 17.0),
                3.0,
                fg,
            ));
        }
        PenMode::Lasso => {
            // Lazo: elipse discontinua con su cuerda.
            let (ex, ey, rx, ry) = (cx + 2.0, cy - 4.0, 17.0, 12.0);
            let n = 16;
            for i in (0..n).step_by(2) {
                let a0 = std::f32::consts::TAU * i as f32 / n as f32;
                let a1 = std::f32::consts::TAU * (i as f32 + 1.0) / n as f32;
                lines.push(CanvasLine::new(
                    (ex + rx * a0.cos(), ey + ry * a0.sin()),
                    (ex + rx * a1.cos(), ey + ry * a1.sin()),
                    3.5,
                    fg,
                ));
            }
            polyline(
                lines,
                &[
                    (cx - 8.0, cy + 7.0),
                    (cx - 12.0, cy + 13.0),
                    (cx - 8.0, cy + 18.0),
                ],
                3.5,
                fg,
            );
        }
    }
}

fn tool_of(button: ToolbarButton) -> Option<PenMode> {
    match button {
        ToolbarButton::Pen => Some(PenMode::Ink),
        ToolbarButton::Highlighter => Some(PenMode::Highlight),
        ToolbarButton::Eraser => Some(PenMode::Eraser),
        ToolbarButton::Lasso => Some(PenMode::Lasso),
        _ => None,
    }
}

/// Vector unitario hacia el borde de acoplamiento.
fn toward_edge(dock: ToolbarDock) -> (f32, f32) {
    match dock {
        ToolbarDock::Left => (-1.0, 0.0),
        ToolbarDock::Right => (1.0, 0.0),
        ToolbarDock::Top => (0.0, -1.0),
        ToolbarDock::Bottom => (0.0, 1.0),
    }
}

/// Grosor en px de pantalla con que se muestra cada grosor del boli.
fn width_sample_px(i: usize) -> f32 {
    [3.0, 6.0, 10.0][i.min(2)]
}

/// Índice del grosor actual en `INK_WIDTHS` (el más cercano).
fn width_index(width: f32) -> usize {
    INK_WIDTHS
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - width).abs().total_cmp(&(b.1 - width).abs()))
        .map_or(1, |(i, _)| i)
}

/// Renderiza la barra; devuelve el bitmap y su posición en la ventana.
pub(crate) fn render_toolbar(reader: &Reader) -> Option<(Bitmap, i32, i32)> {
    let layout = toolbar_layout(
        reader.win_w,
        reader.win_h,
        reader.toolbar_dock,
        reader.toolbar_collapsed,
    );
    let p = reader.theme.palette();
    let (cl, ct, cr, cb) = layout.card;
    let (ox, oy) = (cl - SHADOW, ct - SHADOW);
    let (w, h) = (cr - cl + 2.0 * SHADOW, cb - ct + 2.0 * SHADOW);
    let mut rects = Vec::with_capacity(32);
    let mut lines = Vec::with_capacity(64);
    card(&mut rects, w, h, &p);
    for &(x0, y0, x1, y1) in &layout.separators {
        lines.push(CanvasLine::new(
            (x0 - ox, y0 - oy),
            (x1 - ox, y1 - oy),
            2.0,
            p.base_300,
        ));
    }
    let fg = p.base_content;
    let dim = mix(fg, p.base_100, 0.7);
    for &(button, (l, t, r, b)) in &layout.buttons {
        let (l, t, r, b) = (l - ox, t - oy, r - ox, b - oy);
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        if let Some(mode) = tool_of(button) {
            let selected = reader.pen_mode == mode;
            let bg = if selected { p.base_300 } else { p.base_100 };
            if selected {
                rects.push(CanvasRect::rounded(l, t, r, b, 20.0, bg));
            }
            let color = if selected { p.primary } else { fg };
            tool_icon(&mut lines, (cx, cy), mode, color, bg, reader);
            continue;
        }
        match button {
            ToolbarButton::Undo => {
                let c = if reader.undo.can_undo() { fg } else { dim };
                history_arrow(&mut lines, cx, cy, false, c);
            }
            ToolbarButton::Redo => {
                let c = if reader.undo.can_redo() { fg } else { dim };
                history_arrow(&mut lines, cx, cy, true, c);
            }
            ToolbarButton::Toggle => {
                let edge = toward_edge(layout.dock);
                if reader.toolbar_collapsed {
                    // Cerrada: herramienta guardada atenuada (el lápiz
                    // navega) y chevron hacia el interior para abrir.
                    let (sx, sy) = (edge.0 * 8.0, edge.1 * 8.0);
                    tool_icon(
                        &mut lines,
                        (cx + sx, cy + sy),
                        reader.pen_mode,
                        dim,
                        p.base_100,
                        reader,
                    );
                    chevron(
                        &mut lines,
                        cx - sx * 3.6,
                        cy - sy * 3.6,
                        (-edge.0, -edge.1),
                        7.0,
                        fg,
                    );
                } else {
                    chevron(&mut lines, cx, cy, edge, 12.0, fg);
                }
            }
            _ => {}
        }
    }
    let bmp = jni_canvas_bitmap(
        w.round() as i32,
        h.round() as i32,
        theme::TRANSPARENT,
        &rects,
        &lines,
        &[],
    )?;
    Some((bmp, ox.round() as i32, oy.round() as i32))
}

/// Renderiza el popover abierto; devuelve el bitmap y su posición.
pub(crate) fn render_toolbar_popover(
    reader: &Reader,
    kind: ToolbarPopover,
) -> Option<(Bitmap, i32, i32)> {
    let toolbar = toolbar_layout(
        reader.win_w,
        reader.win_h,
        reader.toolbar_dock,
        reader.toolbar_collapsed,
    );
    let layout = toolbar_popover_layout(reader.win_w, reader.win_h, &toolbar, kind);
    let p = reader.theme.palette();
    let (cl, ct, cr, cb) = layout.card;
    let (ox, oy) = (cl - SHADOW, ct - SHADOW);
    let (w, h) = (cr - cl + 2.0 * SHADOW, cb - ct + 2.0 * SHADOW);
    let mut rects = Vec::with_capacity(32);
    let mut lines = Vec::new();
    card(&mut rects, w, h, &p);
    let (palette, current) = match kind {
        ToolbarPopover::Pen => (&INK_PALETTE, reader.ink_color),
        ToolbarPopover::Highlighter => (&HIGHLIGHT_PALETTE, reader.highlight_color),
    };
    for &(cell, (l, t, r, b)) in &layout.cells {
        let (l, t, r, b) = (l - ox, t - oy, r - ox, b - oy);
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        match cell {
            PopoverCell::Color(i) => {
                let color = palette[i];
                if color == current {
                    circle(&mut rects, cx, cy, 31.0, p.primary);
                    circle(&mut rects, cx, cy, 27.0, p.base_100);
                }
                // Contorno: la tinta oscura se distingue también en temas oscuros.
                circle(&mut rects, cx, cy, 25.0, p.base_300);
                circle(&mut rects, cx, cy, 23.0, argb(color, 255));
            }
            PopoverCell::Width(i) => {
                if i == width_index(reader.ink_width) {
                    rects.push(CanvasRect::rounded(
                        l + 4.0,
                        t + 4.0,
                        r - 4.0,
                        b - 4.0,
                        20.0,
                        p.base_300,
                    ));
                }
                lines.push(CanvasLine::new(
                    (cx - 20.0, cy),
                    (cx + 20.0, cy),
                    width_sample_px(i),
                    argb(reader.ink_color, 255),
                ));
            }
        }
    }
    let bmp = jni_canvas_bitmap(
        w.round() as i32,
        h.round() as i32,
        theme::TRANSPARENT,
        &rects,
        &lines,
        &[],
    )?;
    Some((bmp, ox.round() as i32, oy.round() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKS: [ToolbarDock; 4] = [
        ToolbarDock::Left,
        ToolbarDock::Right,
        ToolbarDock::Top,
        ToolbarDock::Bottom,
    ];
    const WINDOWS: [(i32, i32); 2] = [(2200, 1440), (1440, 2200)];

    fn within(inner: ButtonRect, outer: ButtonRect) -> bool {
        inner.0 >= outer.0 && inner.1 >= outer.1 && inner.2 <= outer.2 && inner.3 <= outer.3
    }

    #[test]
    fn every_dock_fits_both_orientations_without_overlapping_buttons() {
        for (w, h) in WINDOWS {
            for dock in DOCKS {
                let l = toolbar_layout(w, h, dock, false);
                assert_eq!(l.buttons.len(), 7);
                assert_eq!(l.separators.len(), 2);
                let window = (0.0, 0.0, w as f32, h as f32);
                assert!(within(l.card, window), "{dock:?} {w}x{h}");
                for pair in l.buttons.windows(2) {
                    let (a, b) = (pair[0].1, pair[1].1);
                    let apart = if dock.vertical() {
                        a.3 <= b.1
                    } else {
                        a.2 <= b.0
                    };
                    assert!(apart, "{:?} overlaps {:?}", pair[0].0, pair[1].0);
                }
                for &(_, r) in &l.buttons {
                    assert!(within(r, l.card));
                    assert!(r.2 - r.0 >= 96.0 && r.3 - r.1 >= 96.0); // 48 dp
                }
            }
        }
    }

    #[test]
    fn top_and_bottom_docks_leave_the_viewer_chrome_free() {
        for (w, h) in WINDOWS {
            let top = toolbar_layout(w, h, ToolbarDock::Top, false);
            assert!(top.card.1 >= viewer_top_chrome_h(h));
            let bottom = toolbar_layout(w, h, ToolbarDock::Bottom, false);
            assert!(bottom.card.3 <= h as f32 - viewer_bottom_chrome_h(h));
        }
    }

    #[test]
    fn hit_test_maps_button_centres_and_misses_outside() {
        let l = toolbar_layout(2200, 1440, ToolbarDock::Bottom, false);
        for &(button, (bl, bt, br, bb)) in &l.buttons {
            assert_eq!(l.button_at((bl + br) / 2.0, (bt + bb) / 2.0), Some(button));
        }
        assert_eq!(l.button_at(l.card.0 - 1.0, l.card.1 + 50.0), None);
        assert!(l.contains(l.card.0 + 2.0, l.card.1 + 2.0));
        assert_eq!(l.button_at(l.card.0 + 2.0, l.card.1 + 2.0), None);
    }

    #[test]
    fn closed_toolbar_has_a_single_toggle_button() {
        for dock in DOCKS {
            let l = toolbar_layout(2200, 1440, dock, true);
            assert_eq!(l.buttons.len(), 1);
            assert_eq!(l.buttons[0].0, ToolbarButton::Toggle);
            assert!(l.separators.is_empty());
        }
    }

    #[test]
    fn popover_opens_towards_the_inside_and_stays_in_the_window() {
        for (w, h) in WINDOWS {
            for dock in DOCKS {
                let bar = toolbar_layout(w, h, dock, false);
                for kind in [ToolbarPopover::Pen, ToolbarPopover::Highlighter] {
                    let pop = toolbar_popover_layout(w, h, &bar, kind);
                    assert!(within(pop.card, (0.0, 0.0, w as f32, h as f32)));
                    let clear = match dock {
                        ToolbarDock::Left => pop.card.0 > bar.card.2,
                        ToolbarDock::Right => pop.card.2 < bar.card.0,
                        ToolbarDock::Top => pop.card.1 > bar.card.3,
                        ToolbarDock::Bottom => pop.card.3 < bar.card.1,
                    };
                    assert!(clear, "{dock:?} {kind:?} {w}x{h}");
                    let (_, (l, t, r, b)) = pop.cells[0];
                    assert_eq!(
                        pop.cell_at((l + r) / 2.0, (t + b) / 2.0),
                        Some(PopoverCell::Color(0))
                    );
                }
            }
        }
        let bar = toolbar_layout(2200, 1440, ToolbarDock::Left, false);
        let pen = toolbar_popover_layout(2200, 1440, &bar, ToolbarPopover::Pen);
        assert_eq!(pen.cells.len(), INK_PALETTE.len() + INK_WIDTHS.len());
        let hl = toolbar_popover_layout(2200, 1440, &bar, ToolbarPopover::Highlighter);
        assert_eq!(hl.cells.len(), HIGHLIGHT_PALETTE.len());
    }

    #[test]
    fn nearest_dock_follows_the_closest_edge() {
        assert_eq!(
            ToolbarDock::nearest(50.0, 700.0, 2200, 1440),
            ToolbarDock::Left
        );
        assert_eq!(
            ToolbarDock::nearest(2150.0, 700.0, 2200, 1440),
            ToolbarDock::Right
        );
        assert_eq!(
            ToolbarDock::nearest(1100.0, 40.0, 2200, 1440),
            ToolbarDock::Top
        );
        assert_eq!(
            ToolbarDock::nearest(1100.0, 1400.0, 2200, 1440),
            ToolbarDock::Bottom
        );
    }

    #[test]
    fn width_index_picks_the_nearest_preset() {
        assert_eq!(width_index(INK_WIDTHS[0]), 0);
        assert_eq!(width_index(2.1), 1);
        assert_eq!(width_index(10.0), 2);
    }
}
