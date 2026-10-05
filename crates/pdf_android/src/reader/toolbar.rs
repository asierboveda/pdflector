// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Barra de herramientas del visor: hit-test (`toolbar_hit`), taps
//! (`toolbar_tap`), popover de color/grosor de la herramienta activa,
//! apertura/cierre (cerrada = el lápiz navega, ADR-012), arrastre con
//! pulsación larga y acoplamiento al borde más cercano (ADR-022), y
//! materialización de sus bitmaps (`materialize_toolbar`). La geometría y el
//! dibujo viven en `draw::toolbar`.

use super::{Reader, UiMode};
use crate::annotations::{HIGHLIGHT_PALETTE, INK_PALETTE, INK_WIDTHS, PenMode};
use crate::draw::{
    ButtonRect, PopoverCell, ToolbarButton, ToolbarDock, ToolbarLayout, ToolbarPopover,
    render_toolbar, render_toolbar_popover, toolbar_layout, toolbar_popover_layout,
};

impl Reader {
    /// ¿Se muestra la barra? Solo en el visor con un documento abierto.
    fn toolbar_active(&self) -> bool {
        self.mode == UiMode::Viewer && self.doc.is_some()
    }

    fn current_toolbar_layout(&self) -> ToolbarLayout {
        toolbar_layout(
            self.win_w,
            self.win_h,
            self.toolbar_dock,
            self.toolbar_collapsed,
        )
    }

    /// Tarjeta de la barra en px de ventana (None si no se muestra).
    pub(crate) fn toolbar_card(&self) -> Option<ButtonRect> {
        self.toolbar_active()
            .then(|| self.current_toolbar_layout().card)
    }

    /// Marca la barra (y el popover abierto) para re-renderizar.
    pub(crate) fn invalidate_toolbar(&mut self) {
        self.toolbar_bitmap = None;
        self.toolbar_popover_bitmap = None;
        self.mark_repaint();
    }

    /// Cierra el popover de la barra si está abierto.
    pub(crate) fn close_toolbar_popover(&mut self) {
        if self.toolbar_popover.take().is_some() {
            self.invalidate_toolbar();
        }
    }

    /// ¿Un Down en (x, y) pertenece a la barra? `Some(true)` sobre la
    /// tarjeta (tap o, con pulsación larga, arrastre); `Some(false)` con el
    /// popover abierto fuera de la tarjeta (el tap lo cierra sin dibujar);
    /// `None` si no es de la barra.
    pub(crate) fn toolbar_hit(&self, x: f32, y: f32) -> Option<bool> {
        if !self.toolbar_active() || self.sheet_progress > 0.0 {
            return None;
        }
        if self.current_toolbar_layout().contains(x, y) {
            Some(true)
        } else if self.toolbar_popover.is_some() {
            Some(false)
        } else {
            None
        }
    }

    /// Tap (dedo o lápiz) sobre la barra o con su popover abierto.
    pub(crate) fn toolbar_tap(&mut self, x: f32, y: f32) {
        if !self.toolbar_active() {
            return;
        }
        let layout = self.current_toolbar_layout();
        if let Some(kind) = self.toolbar_popover {
            let popover = toolbar_popover_layout(self.win_w, self.win_h, &layout, kind);
            if let Some(cell) = popover.cell_at(x, y) {
                self.pick_popover_cell(kind, cell);
                self.close_toolbar_popover();
                return;
            }
            let button = layout.button_at(x, y);
            self.close_toolbar_popover();
            let same_tool = matches!(
                (button, kind),
                (Some(ToolbarButton::Pen), ToolbarPopover::Pen)
                    | (
                        Some(ToolbarButton::Highlighter),
                        ToolbarPopover::Highlighter
                    )
            );
            if button.is_none() || same_tool || popover.contains(x, y) {
                return;
            }
        }
        let Some(button) = layout.button_at(x, y) else {
            return;
        };
        match button {
            ToolbarButton::Pen => self.tap_tool(PenMode::Ink, ToolbarPopover::Pen),
            ToolbarButton::Highlighter => {
                self.tap_tool(PenMode::Highlight, ToolbarPopover::Highlighter)
            }
            ToolbarButton::Eraser => self.set_pen_mode(PenMode::Eraser),
            ToolbarButton::Lasso => self.set_pen_mode(PenMode::Lasso),
            ToolbarButton::Undo => self.undo_annotation(),
            ToolbarButton::Redo => self.redo_annotation(),
            ToolbarButton::Toggle => self.set_toolbar_collapsed(!self.toolbar_collapsed),
        }
    }

    /// Elige Bolígrafo o Subrayador; si ya estaba elegido abre su popover
    /// de color (y grosor).
    fn tap_tool(&mut self, mode: PenMode, popover: ToolbarPopover) {
        if self.pen_mode == mode {
            self.toolbar_popover = Some(popover);
            self.invalidate_toolbar();
        } else {
            self.set_pen_mode(mode);
        }
    }

    /// Abre o cierra la barra. Cerrada, el lápiz navega: se cancela el gesto
    /// en curso y la selección de Recorte; la herramienta guardada no cambia.
    fn set_toolbar_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            self.cancel_tool_gesture();
            self.clear_recorte();
        }
        self.toolbar_collapsed = collapsed;
        self.persist_tool_state();
        self.invalidate_toolbar();
    }

    /// Aplica la celda del popover y persiste el estado.
    fn pick_popover_cell(&mut self, kind: ToolbarPopover, cell: PopoverCell) {
        match (kind, cell) {
            (ToolbarPopover::Pen, PopoverCell::Color(i)) => self.ink_color = INK_PALETTE[i],
            (ToolbarPopover::Pen, PopoverCell::Width(i)) => self.ink_width = INK_WIDTHS[i],
            (ToolbarPopover::Highlighter, PopoverCell::Color(i)) => {
                self.highlight_color = HIGHLIGHT_PALETTE[i]
            }
            (ToolbarPopover::Highlighter, PopoverCell::Width(_)) => return,
        }
        self.persist_tool_state();
        self.invalidate_toolbar();
    }

    /// Pulsación larga sobre la barra: empieza a arrastrarla.
    pub(crate) fn begin_toolbar_drag(&mut self, x: f32, y: f32) {
        self.close_toolbar_popover();
        self.toolbar_drag = Some(((x, y), (x, y)));
        self.mark_repaint();
    }

    /// La barra sigue al dedo o lápiz durante el arrastre.
    pub(crate) fn update_toolbar_drag(&mut self, x: f32, y: f32) {
        if let Some((_, cur)) = self.toolbar_drag.as_mut() {
            *cur = (x, y);
            self.mark_repaint();
        }
    }

    /// Al soltar, la barra se acopla al borde más cercano al punto.
    pub(crate) fn end_toolbar_drag(&mut self, x: f32, y: f32) {
        if self.toolbar_drag.take().is_none() {
            return;
        }
        let dock = ToolbarDock::nearest(x, y, self.win_w, self.win_h);
        if dock != self.toolbar_dock {
            self.toolbar_dock = dock;
            self.persist_tool_state();
        }
        self.invalidate_toolbar();
    }

    /// Arrastre cancelado (segundo dedo, Cancel del sistema): la barra
    /// vuelve a su sitio.
    pub(crate) fn cancel_toolbar_drag(&mut self) {
        if self.toolbar_drag.take().is_some() {
            self.mark_repaint();
        }
    }

    /// Desplazamiento de la barra durante el arrastre (px).
    pub(crate) fn toolbar_drag_offset(&self) -> (i32, i32) {
        self.toolbar_drag.map_or((0, 0), |((sx, sy), (cx, cy))| {
            ((cx - sx).round() as i32, (cy - sy).round() as i32)
        })
    }

    /// Genera los bitmaps de la barra y del popover que falten (antes de
    /// presentar el frame; sin coste si no cambió nada).
    pub(crate) fn materialize_toolbar(&mut self) {
        if !self.toolbar_active() {
            return;
        }
        if self.toolbar_bitmap.is_none() {
            self.toolbar_bitmap = render_toolbar(self);
            self.toolbar_id = self.next_ovl_id();
        }
        if let Some(kind) = self.toolbar_popover
            && self.toolbar_popover_bitmap.is_none()
        {
            self.toolbar_popover_bitmap = render_toolbar_popover(self, kind);
            self.toolbar_popover_id = self.next_ovl_id();
        }
    }
}
