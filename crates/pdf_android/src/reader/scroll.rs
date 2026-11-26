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
        (self.ref_h * self.zoom).max(1.0)
    }

    /// Distancia entre los bordes superiores de dos páginas consecutivas.
    pub(crate) fn pitch(&self) -> f32 {
        self.slot_h() + self.gap
    }

    /// Alto total de la columna (márgenes incluidos).
    pub(crate) fn content_h(&self) -> f32 {
        if self.pages == 0 {
            return 0.0;
        }
        2.0 * self.margin + self.pages as f32 * self.pitch() - self.gap
    }

    /// Clampa `off` al rango válido; si la columna entera cabe en la
    /// ventana queda CENTRADA (offset negativo).
    pub(crate) fn clamp_offset(&self, off: f32) -> f32 {
        let content = self.content_h();
        if content <= self.win_h {
            return -(self.win_h - content) / 2.0;
        }
        if off.is_finite() {
            off.clamp(0.0, content - self.win_h)
        } else {
            0.0
        }
    }

    /// Offset (clampado) de una posición continua `pos` en unidades de
    /// página (`3.25` = un cuarto de hoja por debajo del borde de la 4ª).
    pub(crate) fn offset_for_pos(&self, pos: f32) -> f32 {
        self.clamp_offset(pos * self.pitch())
    }

    /// Posición continua (unidades de página) de un offset.
    pub(crate) fn pos_for_offset(&self, off: f32) -> f32 {
        off / self.pitch()
    }

    /// Borde superior (px de ventana) de la ranura de `page`.
    fn slot_top(&self, page: u32, off: f32) -> f32 {
        self.margin + page as f32 * self.pitch() - off
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
        let slot = self.slot_h();
        let (mut w, mut h) = (size0.0 * self.zoom, size0.1 * self.zoom);
        if h > slot && h > 0.0 {
            let k = slot / h;
            w *= k;
            h *= k;
        }
        let l = (self.win_w - w) / 2.0 + pan_x;
        let t = self.slot_top(page, off) + (slot - h) / 2.0;
        (l, t, l + w, t + h)
    }

    /// Rango (primera, última) de páginas cuya ranura cruza la ventana.
    pub(crate) fn visible_pages(&self, off: f32) -> Option<(u32, u32)> {
        if self.pages == 0 {
            return None;
        }
        let last = self.pages - 1;
        let pitch = self.pitch();
        // Primera ranura cuyo borde inferior queda por debajo de y = 0.
        let first = ((off - self.margin - self.slot_h()) / pitch).floor() + 1.0;
        let first = (first.max(0.0) as u32).min(last);
        let mut end = first;
        while end < last && self.slot_top(end + 1, off) < self.win_h {
            end += 1;
        }
        let top = self.slot_top(first, off);
        if top >= self.win_h || top + self.slot_h() <= 0.0 {
            return None;
        }
        Some((first, end))
    }

    /// Alto visible (px) de la ranura de `page`.
    fn visible_h(&self, page: u32, off: f32) -> f32 {
        let top = self.slot_top(page, off);
        ((top + self.slot_h()).min(self.win_h) - top.max(0.0)).max(0.0)
    }

    /// Página "actual": la que más alto visible ocupa (empate → la de
    /// arriba). Alimenta el indicador "N / total" y la persistencia.
    pub(crate) fn current_page(&self, off: f32) -> u32 {
        let Some((a, b)) = self.visible_pages(off) else {
            return 0;
        };
        let mut best = a;
        let mut best_h = self.visible_h(a, off);
        for p in a + 1..=b {
            let h = self.visible_h(p, off);
            if h > best_h {
                best = p;
                best_h = h;
            }
        }
        best
    }

    /// Página cuya ranura contiene la coordenada de pantalla `y` (None en
    /// los huecos y fuera de la columna).
    pub(crate) fn page_at_y(&self, y: f32, off: f32) -> Option<u32> {
        let c = y + off - self.margin;
        if c < 0.0 || self.pages == 0 {
            return None;
        }
        let page = (c / self.pitch()).floor() as u32;
        let inside = c - page as f32 * self.pitch() <= self.slot_h();
        (page < self.pages && inside).then_some(page)
    }

    /// Clamp del pan horizontal: con la hoja más ancha que la ventana se
    /// puede mover hasta sus bordes; si cabe, queda centrada (pan 0).
    pub(crate) fn clamp_pan_x(&self, pan: f32) -> f32 {
        let w = self.ref_w * self.zoom;
        if w <= self.win_w || !pan.is_finite() {
            return 0.0;
        }
        let half = (w - self.win_w) / 2.0;
        pan.clamp(-half, half)
    }
}

/// Zoom anclado del modo Scroll: (offset, pan_x) a zoom `to.zoom` que deja
/// fijo bajo `anchor` (px de ventana) el punto de documento que estaba bajo
/// él a `from.zoom` con `off0`/`pan_x0`. Resultado clampado.
///
/// En Y el punto se expresa como (ranura `k`, distancia `r` dentro de ella):
/// dentro de la hoja `r` escala con el zoom; en el hueco (que no escala) se
/// conserva tal cual. En X es la fórmula del pinch del modo Tap con la hoja
/// centrada: `left(z) = (win_w − ref_w·z)/2 + pan`.
pub(crate) fn anchor_zoom(
    from: &ScrollLayout,
    to: &ScrollLayout,
    anchor: (f32, f32),
    off0: f32,
    pan_x0: f32,
) -> (f32, f32) {
    let (ax, ay) = anchor;
    let c0 = (off0 + ay - from.margin).max(0.0);
    let k = (c0 / from.pitch()).floor();
    let r = c0 - k * from.pitch();
    let r1 = if r <= from.slot_h() {
        r * to.slot_h() / from.slot_h()
    } else {
        to.slot_h() + (r - from.slot_h())
    };
    let off1 = to.clamp_offset(k * to.pitch() + r1 + to.margin - ay);

    let left0 = (from.win_w - from.ref_w * from.zoom) / 2.0 + pan_x0;
    let q = (ax - left0) / from.zoom.max(1e-4);
    let left1 = ax - q * to.zoom;
    let pan1 = to.clamp_pan_x(left1 - (to.win_w - to.ref_w * to.zoom) / 2.0);
    (off1, pan1)
}

/// ¿El bitmap residente (renderizado a `entry_zoom`) está obsoleto frente
/// al zoom que el worker produciría ahora (`expected_zoom`)? Tolerancia del
/// 5% (el redondeo a píxeles enteros del render no cuenta como cambio).
pub(crate) fn entry_is_stale(entry_zoom: f32, expected_zoom: f32) -> bool {
    let _ = (entry_zoom, expected_zoom);
    false
}

/// Margen de seguridad sobre el presupuesto de la caché: el recorte idle
/// (`PageCache::trim_to_budget`) no debe expulsar ninguna hoja del plan.
const PLAN_BUDGET_FRACTION: f32 = 0.9;

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
    let Some((a, b)) = layout.visible_pages(off) else {
        return (Vec::new(), layout.zoom);
    };
    let mut visible: Vec<u32> = (a..=b).collect();
    visible.sort_by_key(|&p| (p as i64 - current as i64).abs());
    visible.truncate(max_entries.max(1));

    let budget = budget_bytes as f32 * PLAN_BUDGET_FRACTION;
    let bytes_at = |z: f32| layout.ref_w * layout.ref_h * z * z * 4.0;
    let mut zoom = layout.zoom;
    let n = visible.len() as f32;
    if n * bytes_at(zoom) > budget {
        // Zoom máximo al que caben TODAS las visibles (nunca por encima del
        // pedido; nunca por debajo de un mínimo razonable).
        zoom = (budget / (n * bytes_at(1.0))).sqrt().min(zoom).max(0.25);
    }

    let mut pages = visible;
    let mut neighbours = Vec::with_capacity(2);
    if b + 1 < layout.pages {
        neighbours.push(b + 1);
    }
    if a > 0 {
        neighbours.push(a - 1);
    }
    for p in neighbours {
        let fits_bytes = (pages.len() + 1) as f32 * bytes_at(zoom) <= budget;
        if pages.len() < max_entries && fits_bytes {
            pages.push(p);
        }
    }
    (pages, zoom)
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
    fn stale_entries_are_those_at_another_zoom() {
        assert!(!entry_is_stale(1.0, 1.0));
        assert!(!entry_is_stale(1.99, 2.0)); // redondeo del render
        assert!(entry_is_stale(1.0, 2.0));
        assert!(entry_is_stale(2.0, 1.0));
        assert!(entry_is_stale(0.0, 1.0));
        assert!(entry_is_stale(f32::NAN, 1.0));
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
