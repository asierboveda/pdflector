// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Recorte (ADR-013): seleccionar con un lazo trazos COMPLETOS de la página
//! actual y moverlos o escalarlos (0,5×–2×) dentro de la página.
//!
//! Se entra con la herramienta Recorte de la barra o manteniendo el botón
//! superior del lápiz. Máquina: `Lazo → Seleccionado → Mover/Escalar →
//! Confirmar/Cancelar`. El lazo, la caja y la vista previa viven en la capa
//! Wet; mientras se arrastra, la Dry se compone sin los trazos seleccionados
//! (`recorte_hidden_ids`). Al soltar, `AnnotationSet::transform_strokes`
//! aplica la transformación en una sola operación que conserva id, orden y
//! color; se guarda una vez y entra en el historial de deshacer. Cancelar o
//! perder la superficie conserva los trazos como estaban.

use super::Reader;
use crate::undo::AnnotationEdit;
use log::error;
use pdf_core::{MAX_STROKE_SCALE, MIN_STROKE_SCALE, Rect, UniformTransform, stroke_bounds};

/// Radio (px de pantalla) del asa de escala para el hit-test.
pub(crate) const HANDLE_HIT_PX: f32 = 40.0;
/// Margen (px de pantalla) alrededor de la caja que también la agarra.
pub(crate) const BOX_PAD_PX: f32 = 16.0;
/// Fracción del menor lado de la página que puede recorrer un movimiento.
const MAX_MOVE_FRACTION: f32 = 0.25;

/// Límites de la página en coordenadas de página (x0, y0, x1, y1).
type PageBounds = (f32, f32, f32, f32);

/// Estado de Recorte en curso.
#[derive(Clone, Debug)]
pub(crate) struct Recorte {
    /// Página de la selección (0-based).
    pub(crate) page: u32,
    /// Puntos del lazo en coordenadas de página (solo mientras se dibuja).
    pub(crate) lasso: Vec<(f32, f32)>,
    /// Trazos seleccionados, en orden z.
    pub(crate) ids: Vec<u64>,
    /// Caja de los trazos seleccionados (None mientras se dibuja el lazo).
    pub(crate) bounds: Option<Rect>,
    /// Movimiento o escala en curso.
    pub(crate) drag: Option<RecorteDrag>,
}

/// Arrastre de la selección.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecorteDrag {
    scaling: bool,
    /// Punto de partida en coordenadas de página.
    start: (f32, f32),
    /// Transformación de la vista previa.
    pub(crate) transform: UniformTransform,
}

impl Recorte {
    /// Caja que se muestra: la actual con la vista previa aplicada.
    pub(crate) fn shown_bounds(&self) -> Option<Rect> {
        let b = self.bounds?;
        Some(match self.drag {
            Some(d) => d.transform.apply_rect(b),
            None => b,
        })
    }
}

/// Movimiento de `d` acotado a `MAX_MOVE_FRACTION` del menor lado de la
/// página y sin sacar la caja de la página. Si la caja ya sobresalía, no la
/// mete de golpe: solo impide alejarla más.
fn move_transform(bounds: Rect, page: PageBounds, d: (f32, f32)) -> UniformTransform {
    let (x0, y0, x1, y1) = page;
    let max = MAX_MOVE_FRACTION * (x1 - x0).min(y1 - y0);
    let len = (d.0 * d.0 + d.1 * d.1).sqrt();
    let k = if len > max && len > 0.0 {
        max / len
    } else {
        1.0
    };
    let (dx, dy) = (d.0 * k, d.1 * k);
    let range = |lo: f32, hi: f32| (lo.min(0.0), hi.max(0.0));
    let (lx, hx) = range(x0 - bounds.x, x1 - (bounds.x + bounds.w));
    let (ly, hy) = range(y0 - bounds.y, y1 - (bounds.y + bounds.h));
    UniformTransform {
        origin: (bounds.x, bounds.y),
        scale: 1.0,
        dx: dx.clamp(lx, hx),
        dy: dy.clamp(ly, hy),
    }
}

/// Escala uniforme desde la esquina superior izquierda de la caja: la
/// esquina opuesta sigue al asa (`d` = desplazamiento del asa). Acotada a
/// 0,5×–2× y sin sobrepasar la página (sin forzar un encogimiento si ya
/// sobresalía).
fn scale_transform(bounds: Rect, page: PageBounds, d: (f32, f32)) -> UniformTransform {
    let (_, _, x1, y1) = page;
    let diag = (bounds.w, bounds.h);
    let norm = diag.0 * diag.0 + diag.1 * diag.1;
    let corner = (diag.0 + d.0, diag.1 + d.1);
    let s = if norm > f32::EPSILON {
        (corner.0 * diag.0 + corner.1 * diag.1) / norm
    } else {
        1.0
    };
    let fit = |space: f32, size: f32| {
        if size > f32::EPSILON {
            space / size
        } else {
            f32::INFINITY
        }
    };
    let fit = fit(x1 - bounds.x, bounds.w).min(fit(y1 - bounds.y, bounds.h));
    let s = s
        .min(fit.max(1.0))
        .clamp(MIN_STROKE_SCALE, MAX_STROKE_SCALE);
    UniformTransform {
        origin: (bounds.x, bounds.y),
        scale: s,
        dx: 0.0,
        dy: 0.0,
    }
}

impl Reader {
    /// ¿Hay trazos seleccionados (sin lazo en curso)?
    pub(crate) fn recorte_selected(&self) -> bool {
        self.recorte.as_ref().is_some_and(|r| r.bounds.is_some())
    }

    /// Trazos que la capa Dry no debe pintar: los que se están moviendo o
    /// escalando (la vista previa los dibuja en Wet).
    pub(crate) fn recorte_hidden_ids(&self) -> &[u64] {
        match self.recorte.as_ref() {
            Some(r) if r.drag.is_some() => &r.ids,
            _ => &[],
        }
    }

    /// Down del lápiz: empieza un lazo en la página actual.
    pub(crate) fn begin_lasso(&mut self, sx: f32, sy: f32) -> bool {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return false;
        };
        self.clear_recorte();
        self.last_stylus_time = Some(std::time::Instant::now());
        self.recorte = Some(Recorte {
            page: self.page,
            lasso: vec![pt],
            ids: Vec::new(),
            bounds: None,
            drag: None,
        });
        self.mark_repaint();
        true
    }

    /// Move del lápiz: extiende el lazo.
    pub(crate) fn update_lasso(&mut self, sx: f32, sy: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        if let Some(r) = self.recorte.as_mut()
            && r.bounds.is_none()
        {
            r.lasso.push(pt);
            self.mark_repaint();
        }
    }

    /// Up del lápiz: cierra el lazo y selecciona los trazos que toca. Un
    /// lazo que no toca ninguno no deja selección.
    pub(crate) fn end_lasso(&mut self) {
        self.last_stylus_time = Some(std::time::Instant::now());
        let Some(mut r) = self.recorte.take() else {
            return;
        };
        let lasso = std::mem::take(&mut r.lasso);
        let ids = self.annotations.strokes_in_lasso(r.page as usize, &lasso);
        let bounds = ids
            .iter()
            .filter_map(|&id| match &self.annotations.find(id)?.kind {
                pdf_core::Annotation::Stroke(s) => stroke_bounds(s),
                _ => None,
            })
            .reduce(|a, b| {
                let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
                let (x1, y1) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
                Rect::new(x0, y0, x1 - x0, y1 - y0)
            });
        if let Some(bounds) = bounds {
            r.ids = ids;
            r.bounds = Some(bounds);
            self.recorte = Some(r);
        }
        self.mark_repaint();
    }

    /// ¿Agarra el punto de pantalla la selección? `Some(true)` = asa de
    /// escala, `Some(false)` = caja (mover), `None` = fuera.
    pub(crate) fn recorte_hit(&self, sx: f32, sy: f32) -> Option<bool> {
        let r = self.recorte.as_ref()?;
        let b = r.bounds?;
        let t = self.page_screen_transform(r.page)?;
        let (l, top) = t.page_to_screen(b.x, b.y);
        let (right, bottom) = t.page_to_screen(b.x + b.w, b.y + b.h);
        let (hx, hy) = (sx - right, sy - bottom);
        if hx * hx + hy * hy <= HANDLE_HIT_PX * HANDLE_HIT_PX {
            return Some(true);
        }
        let inside = sx >= l - BOX_PAD_PX
            && sx <= right + BOX_PAD_PX
            && sy >= top - BOX_PAD_PX
            && sy <= bottom + BOX_PAD_PX;
        inside.then_some(false)
    }

    /// Empieza a mover (o escalar con el asa) la selección. La Dry se
    /// recompone sin los trazos seleccionados mientras dure.
    pub(crate) fn begin_recorte_drag(&mut self, sx: f32, sy: f32, scaling: bool) -> bool {
        let Some(start) = self.screen_to_page(sx, sy) else {
            return false;
        };
        let Some(r) = self.recorte.as_mut() else {
            return false;
        };
        let Some(b) = r.bounds else {
            return false;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        r.drag = Some(RecorteDrag {
            scaling,
            start,
            transform: UniformTransform {
                origin: (b.x, b.y),
                ..UniformTransform::IDENTITY
            },
        });
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.invalidate_dry();
        }
        self.mark_repaint();
        true
    }

    /// Move durante el arrastre: actualiza la vista previa (sin escribir).
    pub(crate) fn update_recorte_drag(&mut self, sx: f32, sy: f32) {
        let Some(cur) = self.screen_to_page(sx, sy) else {
            return;
        };
        let Some(page) = self.recorte.as_ref().map(|r| r.page) else {
            return;
        };
        let Some(bounds) = self.page_screen_transform(page).map(|t| t.page_bounds()) else {
            return;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        if let Some(r) = self.recorte.as_mut()
            && let Some(b) = r.bounds
            && let Some(drag) = r.drag.as_mut()
        {
            let d = (cur.0 - drag.start.0, cur.1 - drag.start.1);
            drag.transform = if drag.scaling {
                scale_transform(b, bounds, d)
            } else {
                move_transform(b, bounds, d)
            };
            self.mark_repaint();
        }
    }

    /// Up: confirma el movimiento/escala en una sola operación atómica,
    /// lo guarda y lo apunta en el historial. La selección sigue activa
    /// para repetir.
    pub(crate) fn end_recorte_drag(&mut self) {
        self.last_stylus_time = Some(std::time::Instant::now());
        let Some(r) = self.recorte.as_mut() else {
            return;
        };
        let Some(drag) = r.drag.take() else {
            return;
        };
        let t = drag.transform;
        let moved = t.scale != 1.0 || t.dx != 0.0 || t.dy != 0.0;
        if moved {
            let (page, ids) = (r.page as usize, r.ids.clone());
            match self.annotations.transform_strokes(page, &ids, &t) {
                Ok(before) => {
                    let after = ids
                        .iter()
                        .filter_map(|&id| self.annotations.find(id).cloned())
                        .collect();
                    if let Some(r) = self.recorte.as_mut() {
                        r.bounds = r.bounds.map(|b| t.apply_rect(b));
                    }
                    self.record_annotation_edit(AnnotationEdit::changed(before, after));
                    self.save_annotations();
                }
                Err(e) => error!("recorte transform: {e}"),
            }
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.invalidate_dry();
        }
        self.mark_repaint();
    }

    /// Arrastre cancelado (segundo dedo, Cancel): los trazos no cambian.
    pub(crate) fn cancel_recorte_drag(&mut self) {
        if let Some(r) = self.recorte.as_mut()
            && r.drag.take().is_some()
        {
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.invalidate_dry();
            }
            self.mark_repaint();
        }
    }

    /// Descarta el lazo o la selección (sin modificar trazos).
    pub(crate) fn clear_recorte(&mut self) {
        let Some(r) = self.recorte.take() else {
            return;
        };
        if r.drag.is_some()
            && let Some(gpu) = self.gpu.as_mut()
        {
            gpu.invalidate_dry();
        }
        if self.window.is_some() {
            self.mark_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: PageBounds = (0.0, 0.0, 600.0, 800.0);

    #[test]
    fn move_is_limited_to_a_quarter_of_the_short_side() {
        let b = Rect::new(200.0, 200.0, 50.0, 50.0);
        let t = move_transform(b, PAGE, (400.0, 0.0));
        assert!((t.dx - 150.0).abs() < 1e-3); // 25 % de 600
        assert_eq!(t.dy, 0.0);
    }

    #[test]
    fn move_keeps_the_box_inside_the_page() {
        let b = Rect::new(10.0, 700.0, 50.0, 50.0);
        let t = move_transform(b, PAGE, (-40.0, 100.0));
        assert_eq!(t.dx, -10.0);
        assert_eq!(t.dy, 50.0);
        // Una caja que ya sobresalía no salta al empezar.
        let out = Rect::new(-20.0, 100.0, 50.0, 50.0);
        let t = move_transform(out, PAGE, (0.0, 0.0));
        assert_eq!((t.dx, t.dy), (0.0, 0.0));
    }

    #[test]
    fn scale_follows_the_handle_within_limits() {
        let b = Rect::new(100.0, 100.0, 100.0, 50.0);
        assert!((scale_transform(b, PAGE, (0.0, 0.0)).scale - 1.0).abs() < 1e-5);
        assert!((scale_transform(b, PAGE, (100.0, 50.0)).scale - 2.0).abs() < 1e-5);
        assert_eq!(
            scale_transform(b, PAGE, (500.0, 500.0)).scale,
            MAX_STROKE_SCALE
        );
        assert_eq!(
            scale_transform(b, PAGE, (-90.0, -45.0)).scale,
            MIN_STROKE_SCALE
        );
    }

    #[test]
    fn scale_does_not_grow_past_the_page() {
        let b = Rect::new(500.0, 100.0, 80.0, 40.0);
        let t = scale_transform(b, PAGE, (80.0, 40.0));
        assert!((t.scale - 100.0 / 80.0).abs() < 1e-5);
    }
}
