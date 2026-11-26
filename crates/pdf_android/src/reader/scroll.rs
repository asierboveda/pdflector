// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Modo de lectura CONTINUO (PR-01): junto al visor página a página (Tap),
//! el modo Scroll apila las páginas en vertical con un hueco entre ellas;
//! el arrastre con un dedo desplaza el documento y el indicador de página
//! sigue a la hoja que más ocupa la pantalla.
//!
//! El modo es una preferencia POR DOCUMENTO (`persist::BookProgress`).

use serde::{Deserialize, Serialize};

/// Modo de lectura del visor (por documento, persistido en `library.json`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReadingMode {
    /// Página a página: tap izquierda/derecha pasa de hoja (comportamiento
    /// histórico; default de los registros viejos sin el campo).
    #[default]
    Tap,
    /// Continuo: páginas apiladas en vertical, arrastre = desplazamiento.
    Scroll,
}

/// Geometría PURA de la columna de páginas del modo Scroll (sin `Reader`,
/// testeable en host). Todas las páginas ocupan una RANURA del mismo alto
/// (`ref_h · zoom`, la página de referencia a zoom 1 en contain) separadas
/// por `gap`; la columna empieza y acaba con `margin`. El desplazamiento
/// vertical `off` (px) es la distancia desde el inicio de la columna al
/// borde superior de la ventana. Una página más alta que la ranura (PDF de
/// tamaños mixtos) se reduce para caber en ella; una más baja se centra.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollLayout {
    pub(crate) win_w: f32,
    pub(crate) win_h: f32,
    /// Margen superior/inferior de la columna (px).
    pub(crate) margin: f32,
    /// Hueco entre páginas (px, no escala con el zoom).
    pub(crate) gap: f32,
    /// Tamaño (px) de la página de referencia a zoom 1 (contain).
    pub(crate) ref_w: f32,
    pub(crate) ref_h: f32,
    /// Factor de zoom (1.0 = página de referencia en contain).
    pub(crate) zoom: f32,
    /// Número de páginas del documento.
    pub(crate) pages: u32,
}

impl ScrollLayout {
    /// Alto de una ranura de página (px) al zoom actual.
    pub(crate) fn slot_h(&self) -> f32 {
        0.0
    }

    /// Distancia entre los bordes superiores de dos páginas consecutivas.
    pub(crate) fn pitch(&self) -> f32 {
        1.0
    }

    /// Alto total de la columna (márgenes incluidos).
    pub(crate) fn content_h(&self) -> f32 {
        0.0
    }

    /// Clampa `off` al rango válido; si la columna entera cabe en la
    /// ventana queda CENTRADA (offset negativo).
    pub(crate) fn clamp_offset(&self, off: f32) -> f32 {
        off
    }

    /// Offset (clampado) de una posición continua `pos` en unidades de
    /// página (`3.25` = un cuarto de hoja por debajo del borde de la 4ª).
    pub(crate) fn offset_for_pos(&self, pos: f32) -> f32 {
        pos
    }

    /// Posición continua (unidades de página) de un offset.
    pub(crate) fn pos_for_offset(&self, off: f32) -> f32 {
        off
    }

    /// Rect en pantalla (l, t, r, b) de `page` cuyo tamaño a zoom 1 en
    /// contain es `size0`, con el offset `off` y el pan horizontal `pan_x`.
    pub(crate) fn page_rect(
        &self,
        page: u32,
        size0: (f32, f32),
        off: f32,
        pan_x: f32,
    ) -> (f32, f32, f32, f32) {
        let _ = (page, size0, off, pan_x);
        (0.0, 0.0, 0.0, 0.0)
    }

    /// Rango (primera, última) de páginas cuya ranura cruza la ventana.
    pub(crate) fn visible_pages(&self, off: f32) -> Option<(u32, u32)> {
        let _ = off;
        None
    }

    /// Página "actual": la que más alto visible ocupa (empate → la de
    /// arriba). Alimenta el indicador "N / total" y la persistencia.
    pub(crate) fn current_page(&self, off: f32) -> u32 {
        let _ = off;
        0
    }

    /// Página cuya ranura contiene la coordenada de pantalla `y` (None en
    /// los huecos y fuera de la columna).
    pub(crate) fn page_at_y(&self, y: f32, off: f32) -> Option<u32> {
        let _ = (y, off);
        None
    }

    /// Clamp del pan horizontal: con la hoja más ancha que la ventana se
    /// puede mover hasta sus bordes; si cabe, queda centrada (pan 0).
    pub(crate) fn clamp_pan_x(&self, pan: f32) -> f32 {
        pan
    }
}

/// Zoom anclado del modo Scroll: (offset, pan_x) a zoom `to.zoom` que deja
/// fijo bajo `anchor` (px de ventana) el punto de documento que estaba bajo
/// él a `from.zoom` con `off0`/`pan_x0`. Resultado clampado.
pub(crate) fn anchor_zoom(
    from: &ScrollLayout,
    to: &ScrollLayout,
    anchor: (f32, f32),
    off0: f32,
    pan_x0: f32,
) -> (f32, f32) {
    let _ = (from, to, anchor);
    (off0, pan_x0)
}

/// Plan de render del modo Scroll: páginas a pedir al worker (la actual
/// primero, luego el resto de visibles por cercanía y las vecinas que
/// quepan) y el zoom de render. Las visibles caben SIEMPRE en
/// `budget_bytes` (y en `max_entries`): si al zoom pedido no caben, se
/// renderizan a un zoom menor (más suave, pero ninguna hoja visible en
/// blanco ni evicciones en bucle entre ellas).
pub(crate) fn render_plan(
    layout: &ScrollLayout,
    off: f32,
    current: u32,
    budget_bytes: usize,
    max_entries: usize,
) -> (Vec<u32>, f32) {
    let _ = (layout, off, current, budget_bytes, max_entries);
    (Vec::new(), 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: usize = 1024 * 1024;

    /// Tablet en vertical: 1200×2000, margen y hueco de 20 px, página de
    /// referencia 1160×1640 (A4 a contain por ancho), 10 páginas.
    fn tablet(zoom: f32) -> ScrollLayout {
        ScrollLayout {
            win_w: 1200.0,
            win_h: 2000.0,
            margin: 20.0,
            gap: 20.0,
            ref_w: 1160.0,
            ref_h: 1640.0,
            zoom,
            pages: 10,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn column_metrics_stack_pages_with_gap_and_margins() {
        let l = tablet(1.0);
        assert!(close(l.slot_h(), 1640.0));
        assert!(close(l.pitch(), 1660.0));
        assert!(close(l.content_h(), 20.0 + 10.0 * 1660.0 - 20.0 + 20.0));
        let z = tablet(2.0);
        assert!(close(z.slot_h(), 3280.0));
        assert!(close(z.pitch(), 3300.0));
    }

    #[test]
    fn offsets_are_clamped_to_the_column() {
        let l = tablet(1.0);
        let max = l.content_h() - 2000.0;
        assert!(close(l.clamp_offset(-50.0), 0.0));
        assert!(close(l.clamp_offset(1e9), max));
        assert!(close(l.clamp_offset(700.0), 700.0));
        assert!(close(l.offset_for_pos(0.0), 0.0));
        assert!(close(l.offset_for_pos(3.0), 3.0 * 1660.0));
        assert!(close(l.offset_for_pos(9.9), max));
        assert!(close(l.pos_for_offset(3.0 * 1660.0 + 415.0), 3.25));
        // Documento corto: la columna entera cabe → centrada.
        let short = ScrollLayout { pages: 1, ..l };
        let c = short.clamp_offset(123.0);
        assert!(close(c, -(2000.0 - short.content_h()) / 2.0));
        assert!(close(short.clamp_offset(-999.0), c));
    }

    #[test]
    fn page_rects_follow_offset_zoom_and_pan() {
        let l = tablet(1.0);
        let a4 = (1160.0, 1640.0);
        let r = l.page_rect(0, a4, 0.0, 0.0);
        assert!(close(r.0, 20.0) && close(r.1, 20.0) && close(r.2, 1180.0) && close(r.3, 1660.0));
        let r1 = l.page_rect(1, a4, 0.0, 0.0);
        assert!(close(r1.1, 20.0 + 1660.0));
        let r1s = l.page_rect(1, a4, 1660.0, 0.0);
        assert!(close(r1s.1, 20.0));
        // Zoom 2 + pan: más ancha que la ventana, desplazada en X.
        let z = tablet(2.0);
        let rz = z.page_rect(0, a4, 0.0, 100.0);
        assert!(close(rz.0, (1200.0 - 2320.0) / 2.0 + 100.0));
        assert!(close(rz.2 - rz.0, 2320.0) && close(rz.3 - rz.1, 3280.0));
        // Página más alta que la ranura (tamaños mixtos): se reduce a la ranura.
        let tall = l.page_rect(2, (1000.0, 2000.0), 0.0, 0.0);
        assert!(close(tall.3 - tall.1, 1640.0));
        assert!(close(tall.2 - tall.0, 820.0));
        assert!(close(tall.0, (1200.0 - 820.0) / 2.0));
        // Más baja (apaisada): centrada en su ranura.
        let wide = l.page_rect(0, (1160.0, 820.0), 0.0, 0.0);
        assert!(close(wide.1, 20.0 + 410.0));
    }

    #[test]
    fn visible_pages_are_those_crossing_the_window() {
        let l = tablet(1.0);
        assert_eq!(l.visible_pages(0.0), Some((0, 1)));
        assert_eq!(l.visible_pages(1660.0), Some((1, 2)));
        let end = l.clamp_offset(1e9);
        assert_eq!(l.visible_pages(end), Some((8, 9)));
        let none = ScrollLayout { pages: 0, ..l };
        assert_eq!(none.visible_pages(0.0), None);
    }

    #[test]
    fn current_page_is_the_one_mostly_in_view() {
        let l = tablet(1.0);
        assert_eq!(l.current_page(0.0), 0);
        // Página 0 visible 660 px, página 1 visible 1320 px.
        assert_eq!(l.current_page(1000.0), 1);
        assert_eq!(l.current_page(l.clamp_offset(1e9)), 9);
        // Empate exacto → la de arriba.
        let tie = ScrollLayout {
            win_h: 1000.0,
            margin: 0.0,
            gap: 0.0,
            ref_h: 1000.0,
            ..l
        };
        assert_eq!(tie.current_page(500.0), 0);
    }

    #[test]
    fn page_at_y_maps_screen_rows_to_pages() {
        let l = tablet(1.0);
        assert_eq!(l.page_at_y(10.0, 0.0), None); // margen superior
        assert_eq!(l.page_at_y(100.0, 0.0), Some(0));
        assert_eq!(l.page_at_y(1670.0, 0.0), None); // hueco entre 0 y 1
        assert_eq!(l.page_at_y(1700.0, 0.0), Some(1));
        assert_eq!(l.page_at_y(100.0, 1660.0), Some(1));
    }

    #[test]
    fn horizontal_pan_is_clamped_to_the_page_edges() {
        assert!(close(tablet(1.0).clamp_pan_x(300.0), 0.0));
        let z = tablet(2.0);
        assert!(close(z.clamp_pan_x(1000.0), 560.0));
        assert!(close(z.clamp_pan_x(-1000.0), -560.0));
        assert!(close(z.clamp_pan_x(123.0), 123.0));
    }

    #[test]
    fn pinch_keeps_the_document_point_under_the_fingers() {
        let (a, b) = (tablet(1.0), tablet(2.0));
        let (off, pan) = anchor_zoom(&a, &b, (600.0, 1000.0), 0.0, 0.0);
        // 980 px dentro de la página 0 a zoom 1 → 1960 px a zoom 2.
        assert!(close(off, 20.0 + 1960.0 - 1000.0));
        assert!(close(pan, 0.0));
        let (_, pan) = anchor_zoom(&a, &b, (300.0, 1000.0), 0.0, 0.0);
        assert!(close(pan, 300.0));
        // Y de vuelta a zoom 1: mismo punto, pan recentrado.
        let (off_back, pan_back) = anchor_zoom(&b, &a, (600.0, 1000.0), off, 0.0);
        assert!(close(off_back, 0.0));
        assert!(close(pan_back, 0.0));
        // Anclaje en la segunda página con offset previo.
        let (off2, _) = anchor_zoom(&a, &b, (600.0, 500.0), 1660.0, 0.0);
        // y=500 → página 1, 480 px dentro → 960 px a zoom 2.
        assert!(close(20.0 + 3300.0 - off2 + 960.0, 500.0));
    }

    #[test]
    fn render_plan_prefers_visible_pages_then_neighbours() {
        let l = tablet(1.0);
        let (pages, z) = render_plan(&l, 1000.0, 1, 48 * MIB, 5);
        assert_eq!(pages, vec![1, 0, 2]);
        assert!(close(z, 1.0));
        let (pages, _) = render_plan(&l, 1660.0 * 4.0, 4, 48 * MIB, 5);
        assert_eq!(pages[0], 4);
        assert!(pages.contains(&5) && pages.contains(&3));
        assert!(pages.len() <= 5);
    }

    #[test]
    fn render_plan_lowers_zoom_so_visible_pages_fit_the_budget() {
        let z = tablet(2.0);
        // A zoom 2 una página pesa ~29 MiB: dos visibles no caben en 48.
        let off = 3300.0 - 1000.0; // borde entre las páginas 0 y 1
        let (pages, rz) = render_plan(&z, off, 0, 48 * MIB, 5);
        assert_eq!(pages, vec![0, 1]);
        assert!(rz < 2.0 && rz >= 1.0);
        let bytes = 1160.0 * 1640.0 * rz * rz * 4.0 * 2.0;
        assert!(bytes <= (48 * MIB) as f32);
        // Una sola visible a zoom 2: cabe tal cual.
        let (pages, rz) = render_plan(&z, 400.0, 0, 48 * MIB, 5);
        assert_eq!(pages, vec![0]);
        assert!(close(rz, 2.0));
    }
}
