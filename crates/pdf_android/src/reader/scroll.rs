// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Modo de lectura CONTINUO (PR-01): junto al visor página a página (Tap),
//! el modo Scroll apila las páginas en vertical con un hueco entre ellas;
//! el arrastre con un dedo desplaza el documento y el indicador de página
//! sigue a la hoja que más ocupa la pantalla.
//!
//! El modo es una preferencia POR DOCUMENTO (`persist::BookProgress`).
//!
//! Integración con el visor (`impl Reader` al final del módulo):
//! - `page_screen_transform` devuelve en Scroll la transformación de cada
//!   hoja en su rect de la columna, así selección, tinta, resaltado, goma y
//!   Recorte siguen funcionando sobre la página actual (`self.page`), que
//!   es la más visible — o la que el lápiz toca (`scroll_focus_page_at`).
//! - `set_pan`/`begin_pan` (dedo, dos dedos, modo mano) mueven el offset
//!   vertical y el pan X en vez del pan de UNA hoja.
//! - El render reutiliza el worker y la `PageCache`: solo visibles +
//!   vecinas (`render_plan`), un lote a la vez, re-render al cambiar el zoom.

use log::info;
use serde::{Deserialize, Serialize};

use super::Reader;
use super::geometry::PageScreenTransform;
use super::redraw::budget_render_scale;
use crate::cache::{CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES};
use crate::view::{initial_scale_with_margin, viewport_margin_px};
use pdf_core::Document;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// testeable en host). Cada página ocupa una RANURA del alto de SU propio
/// tamaño (`sizes[p] · zoom`, PR-04: apaisadas y hojas raras a su tamaño);
/// las que aún no se conocen usan la página de referencia (`ref_w/ref_h`)
/// como provisional. Ranuras separadas por `gap`; la columna empieza y
/// acaba con `margin`. El desplazamiento vertical `off` (px) es la
/// distancia desde el inicio de la columna al borde superior de la ventana.
/// Un bitmap más alto que su ranura (tamaño aún provisional) se reduce para
/// caber en ella; uno más bajo se centra.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScrollLayout {
    pub(crate) win_w: f32,
    pub(crate) win_h: f32,
    /// Margen superior/inferior de la columna (px).
    pub(crate) margin: f32,
    /// Hueco entre páginas (px, no escala con el zoom).
    pub(crate) gap: f32,
    /// Tamaño (px) de la página de referencia a zoom 1 (contain): el
    /// provisional de las hojas cuyo tamaño aún no se conoce.
    pub(crate) ref_w: f32,
    pub(crate) ref_h: f32,
    /// Factor de zoom (1.0 = cada página en contain).
    pub(crate) zoom: f32,
    /// Número de páginas del documento.
    pub(crate) pages: u32,
    /// Tamaño (px a zoom 1, contain) de cada página ya conocida; None (o
    /// fuera del vector) = aún sin geometría → provisional de referencia.
    pub(crate) sizes: Arc<[Option<(f32, f32)>]>,
}

impl ScrollLayout {
    /// Alto de la ranura PROVISIONAL (página de referencia) al zoom actual.
    pub(crate) fn slot_h(&self) -> f32 {
        (self.ref_h * self.zoom).max(1.0)
    }

    /// Distancia entre bordes superiores de dos ranuras provisionales.
    pub(crate) fn pitch(&self) -> f32 {
        self.slot_h() + self.gap
    }

    /// Tamaño conocido (px a zoom 1) de `page`, si lo hay y es válido.
    fn known_size(&self, page: u32) -> Option<(f32, f32)> {
        self.sizes
            .get(page as usize)
            .copied()
            .flatten()
            .filter(|&(w, h)| w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0)
    }

    /// Tamaño (px a zoom 1) con el que `page` ocupa la columna: el suyo o
    /// el provisional de referencia.
    pub(crate) fn size0(&self, page: u32) -> (f32, f32) {
        self.known_size(page).unwrap_or((self.ref_w, self.ref_h))
    }

    /// Alto de la ranura de `page` al zoom actual.
    pub(crate) fn slot_h_of(&self, page: u32) -> f32 {
        match self.known_size(page) {
            Some((_, h)) => (h * self.zoom).max(1.0),
            None => self.slot_h(),
        }
    }

    /// Ranura + hueco de `page`.
    fn pitch_of(&self, page: u32) -> f32 {
        match self.known_size(page) {
            Some(_) => self.slot_h_of(page) + self.gap,
            None => self.pitch(),
        }
    }

    /// Distancia (px, sin margen ni offset) del inicio de la columna al
    /// borde superior de la ranura de `page`.
    fn top0(&self, page: u32) -> f32 {
        (0..page.min(self.pages)).map(|p| self.pitch_of(p)).sum()
    }

    /// Ranura que contiene la coordenada de columna `c` (px desde el borde
    /// superior de la hoja 0, sin margen): (página, su `top0`). Antes de la
    /// columna → la 0; después → la última.
    fn slot_at(&self, c: f32) -> (u32, f32) {
        let mut top = 0.0;
        for p in 0..self.pages {
            let next = top + self.pitch_of(p);
            if c < next || p + 1 == self.pages {
                return (p, top);
            }
            top = next;
        }
        (0, 0.0)
    }

    /// Alto total de la columna (márgenes incluidos).
    pub(crate) fn content_h(&self) -> f32 {
        if self.pages == 0 {
            return 0.0;
        }
        2.0 * self.margin + self.top0(self.pages) - self.gap
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
    /// página (`3.25` = un cuarto de la ranura de la 4ª por debajo de su
    /// borde superior).
    pub(crate) fn offset_for_pos(&self, pos: f32) -> f32 {
        if !pos.is_finite() || self.pages == 0 {
            return self.clamp_offset(0.0);
        }
        let k = (pos.floor().max(0.0) as u32).min(self.pages - 1);
        let frac = pos - k as f32;
        self.clamp_offset(self.top0(k) + frac * self.pitch_of(k))
    }

    /// Posición continua (unidades de página) de un offset.
    pub(crate) fn pos_for_offset(&self, off: f32) -> f32 {
        let (k, top) = self.slot_at(off);
        k as f32 + (off - top) / self.pitch_of(k)
    }

    /// Borde superior (px de ventana) de la ranura de `page`.
    fn slot_top(&self, page: u32, off: f32) -> f32 {
        self.margin + self.top0(page) - off
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
        let slot = self.slot_h_of(page);
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
        // Primera ranura cuyo borde inferior queda por debajo de y = 0.
        let mut first = last;
        let mut first_top = self.slot_top(last, off);
        let mut top = self.margin - off;
        for p in 0..self.pages {
            if top + self.slot_h_of(p) > 0.0 {
                first = p;
                first_top = top;
                break;
            }
            top += self.pitch_of(p);
        }
        if first_top >= self.win_h || first_top + self.slot_h_of(first) <= 0.0 {
            return None;
        }
        let mut end = first;
        let mut next_top = first_top + self.pitch_of(first);
        while end < last && next_top < self.win_h {
            end += 1;
            next_top += self.pitch_of(end);
        }
        Some((first, end))
    }

    /// Alto visible (px) de la ranura de `page`.
    fn visible_h(&self, page: u32, off: f32) -> f32 {
        let top = self.slot_top(page, off);
        ((top + self.slot_h_of(page)).min(self.win_h) - top.max(0.0)).max(0.0)
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
        let (page, top) = self.slot_at(c);
        let inside = c - top <= self.slot_h_of(page) && c < top + self.pitch_of(page);
        inside.then_some(page)
    }

    /// Ancho (px a zoom 1) de la hoja más ancha conocida de la columna.
    fn column_w(&self) -> f32 {
        self.sizes
            .iter()
            .flatten()
            .map(|&(w, _)| w)
            .filter(|w| w.is_finite())
            .fold(self.ref_w, f32::max)
    }

    /// Clamp del pan horizontal: con la hoja más ancha que la ventana se
    /// puede mover hasta sus bordes; si cabe, queda centrada (pan 0).
    pub(crate) fn clamp_pan_x(&self, pan: f32) -> f32 {
        let w = self.column_w() * self.zoom;
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
/// dentro de la hoja `r` escala con su ranura; en el hueco (que no escala)
/// se conserva tal cual. En X es la fórmula del pinch del modo Tap con la
/// columna centrada: `left(z) = (win_w − ancho·z)/2 + pan`.
pub(crate) fn anchor_zoom(
    from: &ScrollLayout,
    to: &ScrollLayout,
    anchor: (f32, f32),
    off0: f32,
    pan_x0: f32,
) -> (f32, f32) {
    let (ax, ay) = anchor;
    let c0 = (off0 + ay - from.margin).max(0.0);
    let (k, top_k) = from.slot_at(c0);
    let r = c0 - top_k;
    let (h0, h1) = (from.slot_h_of(k), to.slot_h_of(k));
    let r1 = if r <= h0 { r * h1 / h0 } else { h1 + (r - h0) };
    let off1 = to.clamp_offset(to.top0(k) + r1 + to.margin - ay);

    let left0 = (from.win_w - from.column_w() * from.zoom) / 2.0 + pan_x0;
    let q = (ax - left0) / from.zoom.max(1e-4);
    let left1 = ax - q * to.zoom;
    let pan1 = to.clamp_pan_x(left1 - (to.win_w - to.column_w() * to.zoom) / 2.0);
    (off1, pan1)
}

/// Offset que mantiene fija en pantalla la página `page` cuando el layout
/// cambia de `old` a `new` (llega el tamaño real de otra hoja): lo que
/// crece o mengua por encima desplaza el offset lo mismo y la hoja (la
/// actual) no salta. Un cambio por debajo no mueve nada.
pub(crate) fn reanchor_offset(old: &ScrollLayout, new: &ScrollLayout, off: f32, page: u32) -> f32 {
    new.clamp_offset(off + new.top0(page) - old.top0(page))
}

/// ¿El bitmap residente (renderizado a `entry_zoom`) está obsoleto frente
/// al zoom que el worker produciría ahora (`expected_zoom`)? Tolerancia del
/// 5% (el redondeo a píxeles enteros del render no cuenta como cambio).
pub(crate) fn entry_is_stale(entry_zoom: f32, expected_zoom: f32) -> bool {
    if !entry_zoom.is_finite() || entry_zoom <= 0.0 || expected_zoom <= 0.0 {
        return true;
    }
    (entry_zoom - expected_zoom).abs() / expected_zoom > 0.05
}

/// Posición fina a persistir (`BookProgress::scroll_frac`): la posición
/// continua relativa a la página actual (la del indicador).
pub(crate) fn scroll_frac_for(pos: f32, current: u32) -> f32 {
    pos - current as f32
}

/// Posición continua a restaurar desde (página, `scroll_frac`) guardados:
/// un frac corrupto (no finito o fuera de ±1 hoja) cae al borde de la
/// página.
pub(crate) fn resume_scroll_pos(page: u32, frac: f32) -> f32 {
    let frac = if frac.is_finite() && frac.abs() <= 1.0 {
        frac
    } else {
        0.0
    };
    (page as f32 + frac).max(0.0)
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
    // Bytes de cada hoja a zoom 1 según SU tamaño (PR-04: una apaisada
    // pesa la mitad que una vertical).
    let area = |p: u32| {
        let (w, h) = layout.size0(p);
        w * h * 4.0
    };
    let visible_area: f32 = visible.iter().map(|&p| area(p)).sum();
    let mut zoom = layout.zoom;
    if visible_area * zoom * zoom > budget {
        // Zoom máximo al que caben TODAS las visibles (nunca por encima del
        // pedido; nunca por debajo de un mínimo razonable).
        zoom = (budget / visible_area).sqrt().min(zoom).max(0.25);
    }

    let mut pages = visible;
    let mut used = visible_area * zoom * zoom;
    let mut neighbours = Vec::with_capacity(2);
    if b + 1 < layout.pages {
        neighbours.push(b + 1);
    }
    if a > 0 {
        neighbours.push(a - 1);
    }
    for p in neighbours {
        let bytes = area(p) * zoom * zoom;
        if pages.len() < max_entries && used + bytes <= budget {
            pages.push(p);
            used += bytes;
        }
    }
    (pages, zoom)
}

/// Quietud del pinch (ms) a partir de la cual se re-renderiza al zoom
/// actual con los dedos aún en pantalla (mismo criterio que el debounce
/// del modo Tap en `tick`).
const SCROLL_PINCH_SETTLE: Duration = Duration::from_millis(350);

impl Reader {
    /// ¿Documento abierto en modo continuo?
    pub(crate) fn is_scroll_mode(&self) -> bool {
        self.reading_mode == ReadingMode::Scroll
    }

    /// Layout de la columna al zoom actual. None fuera del modo Scroll, sin
    /// documento o mientras no se conoce la página de referencia (entonces
    /// el visor se comporta como en Tap con la página actual).
    pub(crate) fn scroll_layout(&self) -> Option<ScrollLayout> {
        self.scroll_layout_at(self.zoom)
    }

    fn scroll_layout_at(&self, zoom: f32) -> Option<ScrollLayout> {
        if !self.is_scroll_mode() || self.win_w <= 0 || self.win_h <= 0 {
            return None;
        }
        let pages = self.doc.as_ref()?.page_count();
        let (pw, ph) = self.scroll_ref_pts?;
        let margin = viewport_margin_px(self.density_dpi);
        let fit = initial_scale_with_margin(pw, ph, self.win_w, self.win_h, margin);
        Some(ScrollLayout {
            win_w: self.win_w as f32,
            win_h: self.win_h as f32,
            margin,
            gap: margin,
            ref_w: pw * fit,
            ref_h: ph * fit,
            zoom,
            pages,
            sizes: self.scroll_sizes_now(margin),
        })
    }

    /// Tamaños por página para el layout: la caché si corresponde a esta
    /// ventana y densidad; si no (resize aún sin refrescar), recalculados.
    fn scroll_sizes_now(&self, margin: f32) -> Arc<[Option<(f32, f32)>]> {
        if self.scroll_sizes_key == (self.win_w, self.win_h, self.density_dpi) {
            return self.scroll_sizes.clone();
        }
        Arc::from(self.scroll_sizes_px(margin))
    }

    fn scroll_sizes_px(&self, margin: f32) -> Vec<Option<(f32, f32)>> {
        self.scroll_geoms
            .iter()
            .map(|g| {
                let b = g.as_ref()?.visible_bounds;
                let (pw, ph) = (b.width(), b.height());
                let fit = initial_scale_with_margin(pw, ph, self.win_w, self.win_h, margin);
                (pw > 0.0 && ph > 0.0 && fit > 0.0).then_some((pw * fit, ph * fit))
            })
            .collect()
    }

    /// Recalcula la caché de tamaños (geometría nueva, resize).
    pub(crate) fn scroll_refresh_sizes(&mut self) {
        let margin = viewport_margin_px(self.density_dpi);
        self.scroll_sizes = Arc::from(self.scroll_sizes_px(margin));
        self.scroll_sizes_key = (self.win_w, self.win_h, self.density_dpi);
    }

    /// Documento nuevo: olvida las geometrías de la columna.
    pub(crate) fn scroll_reset_geometry(&mut self) {
        self.scroll_geoms.clear();
        self.scroll_refresh_sizes();
    }

    /// Geometría conocida de `page` (la de la columna, o la caché corta).
    fn scroll_geometry(&self, page: u32) -> Option<pdf_core::engine::PageGeometry> {
        self.scroll_geoms
            .get(page as usize)
            .copied()
            .flatten()
            .or_else(|| self.page_geometry_cache.get(page))
    }

    /// Registra la geometría de `page` (llega con los renders del worker):
    /// la primera conocida fija la página de referencia (provisional de las
    /// demás) y cada una da a su hoja su propia ranura (PR-04). Si la
    /// columna cambia de forma, la hoja actual queda ANCLADA en pantalla
    /// (`reanchor_offset`): lo que estaba a la vista no salta.
    pub(crate) fn scroll_note_geometry(&mut self, page: u32) {
        let Some(g) = self.page_geometry_cache.get(page) else {
            return;
        };
        let b = g.visible_bounds;
        if b.width() <= 0.0 || b.height() <= 0.0 {
            return;
        }
        if self.scroll_ref_pts.is_none() {
            self.scroll_ref_pts = Some((b.width(), b.height()));
        }
        let pages = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0) as usize;
        if (page as usize) >= pages {
            return;
        }
        if self.scroll_geoms.len() != pages {
            self.scroll_geoms.resize(pages, None);
        }
        let known = self.scroll_geoms[page as usize];
        self.scroll_geoms[page as usize] = Some(g);
        if known.is_some_and(|k| k.visible_bounds == b) {
            return; // mismo tamaño: la columna no cambia
        }
        let old = self.scroll_layout();
        self.scroll_refresh_sizes();
        if let (Some(old), Some(new)) = (old, self.scroll_layout())
            && old != new
        {
            let off0 = old.offset_for_pos(self.scroll_pos);
            let off1 = reanchor_offset(&old, &new, off0, self.page);
            self.scroll_pos = new.pos_for_offset(off1);
            self.gesture.shift_pan_origin(off1 - off0);
            self.mark_repaint();
            if let Some(g) = self.gpu.as_mut() {
                g.invalidate_dry();
            }
        }
    }

    /// Transformación de `page` en su rect de la columna (None si la hoja
    /// no tiene geometría conocida o no hay layout).
    pub(crate) fn scroll_page_transform(&self, page: u32) -> Option<PageScreenTransform> {
        let layout = self.scroll_layout()?;
        let geometry = self.scroll_geometry(page)?;
        let size0 = layout.size0(page);
        if size0.0 <= 0.0 || size0.1 <= 0.0 {
            return None;
        }
        let off = layout.offset_for_pos(self.scroll_pos);
        PageScreenTransform::from_screen_rect(
            geometry,
            layout.page_rect(page, size0, off, self.pan_x),
        )
    }

    /// Páginas de la columna que cruzan la ventana (vacío fuera de Scroll).
    pub(crate) fn scroll_visible_pages(&self) -> Vec<u32> {
        self.scroll_layout()
            .and_then(|l| l.visible_pages(l.offset_for_pos(self.scroll_pos)))
            .map(|(a, b)| (a..=b).collect())
            .unwrap_or_default()
    }

    /// Componente vertical de la `DryKey`: en Scroll el offset de la
    /// columna (cada frame de desplazamiento re-hornea la dry), en Tap el
    /// pan de la hoja.
    pub(crate) fn dry_pan_y(&self) -> i32 {
        match self.scroll_layout() {
            Some(l) => -(l.offset_for_pos(self.scroll_pos).round() as i32),
            None => self.pan_y.round() as i32,
        }
    }

    /// Pan de partida de un arrastre en Scroll: (pan X, −offset), mismo
    /// convenio que `set_pan` (el contenido sigue al dedo: arrastrar hacia
    /// abajo reduce el offset y deja ver lo de arriba).
    pub(crate) fn scroll_begin_pan(&self) -> (f32, f32) {
        let off = self
            .scroll_layout()
            .map(|l| l.offset_for_pos(self.scroll_pos))
            .unwrap_or(0.0);
        (self.pan_x, -off)
    }

    /// `set_pan` del modo Scroll: `x` es el pan horizontal y `y = −offset`.
    pub(crate) fn scroll_set_pan(&mut self, x: f32, y: f32) {
        let Some(layout) = self.scroll_layout() else {
            return;
        };
        self.pan_x = layout.clamp_pan_x(x);
        let off = layout.clamp_offset(-y);
        self.scroll_pos = layout.pos_for_offset(off);
        self.scroll_moved(&layout, off);
    }

    /// Un dedo toca la pantalla: la inercia se detiene en seco.
    pub(crate) fn scroll_touch_down(&mut self) {
        self.scroll_stop_fling();
    }

    /// Detiene la inercia (toque, salto, zoom, cambio de modo).
    pub(crate) fn scroll_stop_fling(&mut self) {
        self.scroll_fling.touch_down();
        self.scroll_fling_at = None;
    }

    /// Muestra (`event_time` ns, y px) de un arrastre de un dedo en Scroll.
    pub(crate) fn scroll_track_drag(&mut self, t_ns: i64, y: f32) {
        if self.is_scroll_mode() {
            self.scroll_fling.track(t_ns, y);
        }
    }

    /// Fin del arrastre de un dedo: si la suelta fue rápida la columna
    /// sigue deslizando (`scroll_fling_tick` en cada tick).
    pub(crate) fn scroll_release_drag(&mut self, t_ns: i64, y: f32) {
        if self.is_scroll_mode() && self.scroll_fling.release(t_ns, y) {
            self.scroll_fling_at = Some(Instant::now());
            self.mark_repaint();
        }
    }

    /// Frame de inercia (desde `tick`): avanza la física el tiempo real
    /// transcurrido (acotado: un tick tardío no pega un salto).
    pub(crate) fn scroll_fling_tick(&mut self, now: Instant) {
        let Some(at) = self.scroll_fling_at else {
            return;
        };
        let dt = now.saturating_duration_since(at).as_secs_f32().min(0.05);
        self.scroll_fling_at = Some(now);
        let Some(layout) = self.scroll_layout() else {
            self.scroll_stop_fling();
            return;
        };
        let off = layout.offset_for_pos(self.scroll_pos);
        match self.scroll_fling.frame(&layout, off, dt) {
            Some(next) => self.scroll_set_pan(self.pan_x, -next),
            None => self.scroll_fling_at = None,
        }
        if !self.scroll_fling.is_active() {
            self.scroll_fling_at = None;
        }
    }

    /// Salto a `page` en Scroll (badge, ±10, "N / total"): la hoja queda
    /// arriba (o lo más arriba que permita el final del documento).
    pub(crate) fn scroll_to_page(&mut self, page: u32) {
        self.scroll_stop_fling();
        match self.scroll_layout() {
            Some(layout) => {
                let off = layout.offset_for_pos(page as f32);
                self.scroll_pos = layout.pos_for_offset(off);
                self.scroll_moved(&layout, off);
            }
            None => {
                self.scroll_pos = page as f32;
                self.page = page;
                self.cache.set_protected(page);
                self.page_badge = None;
                self.sheet_bitmap = None;
                self.mark_state_dirty();
                self.scroll_ensure_rendered();
                self.mark_repaint();
            }
        }
    }

    /// Tras mover la columna: la página actual sigue a la más visible, se
    /// piden las hojas que faltan y se marca la posición para persistir.
    fn scroll_moved(&mut self, layout: &ScrollLayout, off: f32) {
        let cur = layout.current_page(off);
        if cur != self.page {
            self.page = cur;
            self.cache.set_protected(cur);
            self.page_badge = None; // el indicador "N / total" cambia
            self.sheet_bitmap = None;
        }
        self.mark_state_dirty();
        self.scroll_ensure_rendered();
        self.mark_repaint();
    }

    /// El lápiz o el long-press actúan sobre la hoja que TOCAN: si `y` cae
    /// en otra página visible de la columna, esa pasa a ser la actual (sin
    /// mover la vista) para que la tinta, el resaltado o la selección usen
    /// su transformación.
    pub(crate) fn scroll_focus_page_at(&mut self, y: f32) {
        let Some(layout) = self.scroll_layout() else {
            return;
        };
        let off = layout.offset_for_pos(self.scroll_pos);
        if let Some(p) = layout.page_at_y(y, off)
            && p != self.page
        {
            self.page = p;
            self.cache.set_protected(p);
            self.page_badge = None;
            self.sheet_bitmap = None;
        }
    }

    /// Zoom del modo Scroll anclado en `anchor` (px de ventana): conserva el
    /// punto de documento bajo los dedos (`anchor_zoom`).
    pub(super) fn scroll_apply_zoom(&mut self, zoom: f32, anchor: (f32, f32)) {
        self.scroll_stop_fling();
        let (Some(from), Some(to)) = (
            self.scroll_layout_at(self.zoom),
            self.scroll_layout_at(zoom),
        ) else {
            self.zoom = zoom;
            return;
        };
        let off0 = from.offset_for_pos(self.scroll_pos);
        let (off1, pan1) = anchor_zoom(&from, &to, anchor, off0, self.pan_x);
        self.zoom = zoom;
        self.pan_x = pan1;
        self.scroll_pos = to.pos_for_offset(off1);
        self.scroll_moved(&to, off1);
    }

    /// ¿Hay que (re)renderizar `page` al zoom de render `z`? Sí si no está
    /// en caché o si su bitmap quedó a otra escala (zoom anterior).
    fn scroll_needs_render(&self, page: u32, z: f32) -> bool {
        let Some(entry) = self.cache.peek(page) else {
            return true;
        };
        let Some(g) = self.scroll_geometry(page) else {
            return false; // sin geometría no se puede medir: no insistir
        };
        let (pw, ph) = (g.visible_bounds.width(), g.visible_bounds.height());
        let margin = viewport_margin_px(self.density_dpi);
        let fit = initial_scale_with_margin(pw, ph, self.win_w, self.win_h, margin);
        if fit <= 0.0 || pw <= 0.0 {
            return false;
        }
        let entry_zoom = entry.full_w as f32 / (pw * fit);
        let expected = budget_render_scale(fit, z, pw, ph) / fit;
        entry_is_stale(entry_zoom, expected)
    }

    /// Pide al worker las hojas visibles (+ vecinas) que faltan o están a
    /// otro zoom. Un lote a la vez (no se preempta el que está en vuelo) y
    /// nada durante un pinch en movimiento.
    pub(crate) fn scroll_ensure_rendered(&mut self) {
        if self.render_rx.is_some() {
            return;
        }
        self.scroll_launch_needed();
    }

    fn scroll_launch_needed(&mut self) {
        if !self.is_scroll_mode() || self.doc.is_none() {
            return;
        }
        if self
            .last_pinch_move
            .is_some_and(|t| t.elapsed() < SCROLL_PINCH_SETTLE)
        {
            return;
        }
        let Some(layout) = self.scroll_layout() else {
            // Sin página de referencia: basta la actual (trae su geometría).
            if self.cache.peek(self.page).is_none() {
                self.launch_render(vec![self.page], self.zoom, false);
            }
            return;
        };
        let off = layout.offset_for_pos(self.scroll_pos);
        let (plan, z) = render_plan(
            &layout,
            off,
            self.page,
            CACHE_BYTE_BUDGET,
            CACHE_MAX_ENTRIES,
        );
        let needs: Vec<u32> = plan
            .into_iter()
            .filter(|&p| self.scroll_needs_render(p, z))
            .collect();
        if !needs.is_empty() {
            self.launch_render(needs, z, false);
        }
    }

    /// Fin del pinch en Scroll: render nítido de las visibles al zoom final
    /// (preempta el lote en vuelo, como el sharp del modo Tap).
    pub(super) fn scroll_zoom_settled(&mut self) {
        self.scroll_launch_needed();
        info!("zoom {:.3} (scroll)", self.zoom);
        self.save_state();
        self.mark_repaint();
    }

    /// Posición continua de apertura de `page`: la fina guardada en
    /// `library.json` si el registro apunta a esa misma página.
    pub(crate) fn resume_scroll_at(&mut self, page: u32) {
        let path = self.doc_path.clone().unwrap_or_default();
        let (mode, frac) = crate::persist::reading_view_for(&self.library.lib_books, &path);
        let same_page = crate::persist::progress_for(&self.library.lib_books, &path)
            .is_some_and(|b| b.page == page);
        self.reading_mode = mode;
        self.scroll_pos = resume_scroll_pos(page, if same_page { frac } else { 0.0 });
    }

    /// Cambia el modo de lectura del documento abierto (sheet). La posición
    /// se conserva: Tap → Scroll deja la hoja actual arriba; Scroll → Tap
    /// muestra la hoja más visible. Persiste en el acto.
    pub(crate) fn set_reading_mode(&mut self, mode: ReadingMode) {
        if self.reading_mode == mode {
            return;
        }
        self.reading_mode = mode;
        self.scroll_stop_fling();
        self.scroll_pos = self.page as f32;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.pinch = None;
        self.fallback_page = None;
        self.scroll_note_geometry(self.page);
        if let Some(layout) = self.scroll_layout() {
            let off = layout.offset_for_pos(self.scroll_pos);
            self.scroll_pos = layout.pos_for_offset(off);
            self.page = layout.current_page(off);
        }
        self.cache.set_protected(self.page);
        self.page_badge = None;
        self.sheet_bitmap = None;
        if let Some(g) = self.gpu.as_mut() {
            g.invalidate_dry();
        }
        info!("reading mode {:?}", mode);
        self.save_state();
        self.redraw();
    }
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
            sizes: Arc::from(Vec::new()),
        }
    }

    const A4: (f32, f32) = (1160.0, 1640.0);
    const LAND: (f32, f32) = (1160.0, 820.0);

    /// Columna de tamaños MIXTOS (PR-04): 0 A4, 1 apaisada, 2 aún sin
    /// tamaño (provisional = referencia A4), 3 A4.
    fn mixed(zoom: f32) -> ScrollLayout {
        with_sizes(zoom, &[Some(A4), Some(LAND), None, Some(A4)])
    }

    fn with_sizes(zoom: f32, sizes: &[Option<(f32, f32)>]) -> ScrollLayout {
        ScrollLayout {
            pages: sizes.len() as u32,
            sizes: Arc::from(sizes.to_vec()),
            ..tablet(zoom)
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
    fn scroll_position_round_trips_through_page_and_frac() {
        assert!(close(scroll_frac_for(3.25, 3), 0.25));
        // La página actual puede ser la siguiente (más visible): frac < 0.
        assert!(close(scroll_frac_for(3.8, 4), -0.2));
        assert!(close(resume_scroll_pos(3, 0.25), 3.25));
        assert!(close(resume_scroll_pos(4, -0.2), 3.8));
        assert!(close(resume_scroll_pos(4, f32::NAN), 4.0));
        assert!(close(resume_scroll_pos(4, 7.0), 4.0));
        assert!(close(resume_scroll_pos(0, -0.5), 0.0)); // nunca antes del inicio
        assert!(close(resume_scroll_pos(5, 0.0), 5.0));
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
        assert!((1.0..2.0).contains(&rz));
        let bytes = 1160.0 * 1640.0 * rz * rz * 4.0 * 2.0;
        assert!(bytes <= (48 * MIB) as f32);
        // Una sola visible a zoom 2: cabe tal cual.
        let (pages, rz) = render_plan(&z, 400.0, 0, 48 * MIB, 5);
        assert_eq!(pages, vec![0]);
        assert!(close(rz, 2.0));
    }

    #[test]
    fn mixed_pages_get_their_own_slot_heights() {
        let l = mixed(1.0);
        // Pitches 1660, 840 (apaisada), 1660 (provisional), 1660.
        assert!(close(
            l.content_h(),
            40.0 + 1660.0 + 840.0 + 1660.0 + 1660.0 - 20.0
        ));
        let r1 = l.page_rect(1, LAND, 0.0, 0.0);
        assert!(close(r1.1, 20.0 + 1660.0), "{r1:?}"); // sin centrar en 1640
        assert!(close(r1.3 - r1.1, 820.0) && close(r1.2 - r1.0, 1160.0));
        let r2 = l.page_rect(2, A4, 0.0, 0.0);
        assert!(close(r2.1, 20.0 + 1660.0 + 840.0), "{r2:?}");
        let r3 = l.page_rect(3, A4, 0.0, 0.0);
        assert!(close(r3.1, 20.0 + 1660.0 + 840.0 + 1660.0), "{r3:?}");
        // A zoom 2 cada ranura escala con su página; el hueco no.
        let z = mixed(2.0);
        let z1 = z.page_rect(1, LAND, 0.0, 0.0);
        assert!(close(z1.1, 20.0 + 3300.0) && close(z1.3 - z1.1, 1640.0));
        let z3 = z.page_rect(3, A4, 0.0, 0.0);
        assert!(close(z3.1, 20.0 + 3300.0 + 1660.0 + 3300.0), "{z3:?}");
    }

    #[test]
    fn visible_current_and_hit_pages_follow_mixed_heights() {
        let l = mixed(1.0);
        assert_eq!(l.visible_pages(0.0), Some((0, 1)));
        // off 1500: hoja 0 asoma 160 px, la apaisada 820, la 2 980.
        assert_eq!(l.visible_pages(1500.0), Some((0, 2)));
        assert_eq!(l.current_page(1500.0), 2);
        assert_eq!(l.page_at_y(1700.0, 0.0), Some(1));
        assert_eq!(l.page_at_y(2510.0, 0.0), None); // hueco tras la apaisada
        assert_eq!(l.page_at_y(2600.0, 0.0), Some(2));
    }

    #[test]
    fn positions_round_trip_through_mixed_heights() {
        let l = mixed(1.0);
        assert!(close(l.offset_for_pos(1.5), 1660.0 + 420.0));
        assert!(close(l.pos_for_offset(1660.0 + 420.0), 1.5));
        assert!(close(l.offset_for_pos(2.0), 2500.0));
        assert!(close(l.pos_for_offset(2500.0 + 830.0), 2.5));
        let max = l.content_h() - 2000.0;
        assert!(close(l.offset_for_pos(3.0), max));
        // Reanudar (página, frac) guardados cae en el mismo punto.
        let pos = resume_scroll_pos(1, scroll_frac_for(1.5, 1));
        assert!(close(l.offset_for_pos(pos), 2080.0));
    }

    #[test]
    fn learning_a_page_size_keeps_the_current_page_anchored() {
        // La hoja 0 aún no se conoce (provisional 1640); la actual es la 2.
        let old = with_sizes(1.0, &[None, Some(A4), Some(A4), Some(A4), Some(A4)]);
        let off = 3320.0;
        assert_eq!(old.current_page(off), 2);
        let before = old.page_rect(2, A4, off, 0.0);
        // Llega la hoja 0: es apaisada → todo lo de debajo sube 820 px.
        let new = with_sizes(1.0, &[Some(LAND), Some(A4), Some(A4), Some(A4), Some(A4)]);
        // Sin reanclar, la hoja actual saltaría 820 px hacia arriba.
        assert!(close(new.page_rect(2, A4, off, 0.0).1, before.1 - 820.0));
        let off1 = reanchor_offset(&old, &new, off, 2);
        let after = new.page_rect(2, A4, off1, 0.0);
        assert!(close(before.1, after.1), "{before:?} → {after:?}");
        assert_eq!(new.current_page(off1), 2);
        // Un cambio POR DEBAJO de la actual no mueve la vista.
        let below = with_sizes(1.0, &[None, Some(A4), Some(A4), Some(A4), Some(LAND)]);
        assert!(close(reanchor_offset(&old, &below, off, 2), off));
    }

    #[test]
    fn pinch_anchor_uses_each_page_height() {
        let (a, b) = (mixed(1.0), mixed(2.0));
        // y=1000 con la apaisada arriba: 140 px dentro de la hoja 2.
        let (off, _) = anchor_zoom(&a, &b, (600.0, 1000.0), 1660.0, 0.0);
        assert!(close(off, 3300.0 + 1660.0 + 280.0 + 20.0 - 1000.0), "{off}");
    }

    #[test]
    fn render_plan_budgets_each_page_by_its_own_size() {
        // Apaisadas a zoom 2: ~14,5 MiB por hoja, dos visibles caben en 48.
        let l = with_sizes(2.0, &[Some(LAND); 10]);
        let (pages, rz) = render_plan(&l, 0.0, 0, 48 * MIB, 5);
        assert_eq!(&pages[..2], &[0, 1]);
        assert!(close(rz, 2.0), "{rz}");
    }
}
