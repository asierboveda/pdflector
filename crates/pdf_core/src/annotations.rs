// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Vector annotations in page coordinates (Fase 3: annotations as a vector
//! overlay drawn on top of the cached page bitmap, never rasterized into it —
//! AGENTS.md §4.3).
//!
//! All geometry lives in **page coordinates**: PDF points (1/72 inch, f32),
//! the same space as `Document::page_size` (engine.rs), so annotations stay
//! glued to the page across zoom/scroll and are rendered as a separate vector
//! layer over the cached bitmap.
//!
//! # Serialization (serde / serde_json)
//!
//! `serde` (derive feature) + `serde_json` are required to persist annotations
//! in the SQLite sidecar store (Fase 3 store) and to export them (Fase 4).
//! Both are the de-facto standard serialization crates: dual-licensed
//! MIT/Apache-2.0 (compatible with free distribution, AGENTS.md §3), minimal
//! dependencies and actively maintained. The dependencies are added to
//! `Cargo.toml` by the coordinator; this module only assumes they are
//! available.
//!
//! The `to_string`/`from_str` round-trip is exact for finite coordinates
//! (serde_json cannot represent NaN/infinity, so keep coordinates finite).
//!
//! # Invariants
//!
//! - A `Stroke` needs ≥ 2 points: `Stroke::new` refuses to build one and
//!   `AnnotationSet::add` rejects degenerate strokes (`None`).
//! - `Rect` with negative width/height is normalized (corner re-anchored);
//!   both `Rect::new` and `AnnotationSet::add` enforce it.
//! - `for_page` returns annotations in insertion order, which is the draw
//!   (z) order — later strokes paint on top of earlier ones.
//! - ids are unique and assigned monotonically; they are serialized together
//!   with the set so `remove(id)` keeps working after a save/load round-trip
//!   (hand-edited JSON must keep `next_id` above every stored id).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// RGBA colour, one byte per channel (0–255; alpha 0 = fully transparent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Axis-aligned rectangle in page coordinates. `x/y` is the top-left corner,
/// `w/h` the extent; PDF pages grow down-right.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Builds a rect with non-negative `w`/`h`: a negative `w` (resp. `h`)
    /// re-anchors the rect at `x + w` (resp. `y + h`) and flips the sign, so
    /// the result spans the same area with a positive extent.
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }.normalized()
    }

    /// Returns a copy with `w >= 0` and `h >= 0`, re-anchoring the corner when
    /// a negative extent was given. Fields are public, so `new` can be
    /// bypassed; `AnnotationSet::add` re-applies this on stored rects.
    pub fn normalized(self) -> Self {
        let (x, w) = if self.w < 0.0 {
            (self.x + self.w, -self.w)
        } else {
            (self.x, self.w)
        };
        let (y, h) = if self.h < 0.0 {
            (self.y + self.h, -self.h)
        } else {
            (self.y, self.h)
        };
        Self { x, y, w, h }
    }
}

/// Freehand polyline (margin scribbles / mind maps). `points` are vertices in
/// page coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Polyline vertices, at least 2 (a line needs two ends).
    pub points: Vec<(f32, f32)>,
    /// Stroke width in page units (PDF points); clamped to ≥ 0.
    pub width: f32,
    pub color: Color,
}

impl Stroke {
    /// Builds a stroke, or `None` when `points.len() < 2` (degenerate).
    /// A negative `width` is clamped to 0.
    pub fn new(points: Vec<(f32, f32)>, width: f32, color: Color) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        Some(Self {
            points,
            width: width.max(0.0),
            color,
        })
    }

    /// A stroke is drawable iff it has ≥ 2 points. Public fields allow
    /// bypassing `new`, so `AnnotationSet::add` re-checks this.
    pub fn is_valid(&self) -> bool {
        self.points.len() >= 2
    }
}

/// Text highlight; one rect per covered text line (highlights follow line
/// boxes, they are not a single selection box).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub rects: Vec<Rect>,
    pub color: Color,
}

/// Smooths a polyline with Catmull-Rom interpolation (the classic pen-stroke
/// smoothing: each output segment between `points[i]` and `points[i+1]` is a
/// cubic blended from the four surrounding control points, so the curve
/// passes *exactly* through every vertex — it never cuts corners) and keeps
/// the first and last input points as hard endpoints.
///
/// Returns a `Vec` with `1 + (n-1)*segs` points for `segs >= 1`;
/// degenerate inputs (fewer than 2 points) are returned unchanged, so the
/// result is always drawable when the input is. Positions are page
/// coordinates (f32), the same space as the rest of the model. Repeated
/// points collapse to a zero-length segment that still interpolates the
/// vertex.
///
/// Catmull-Rom needs no per-point state — a pure function, so callers can
/// smooth on capture and store only the (smoothed) points, or store the raw
/// capture and smooth per draw; no allocations beyond the output vec.
pub fn smooth_polyline(points: &[(f32, f32)], segs: u32) -> Vec<(f32, f32)> {
    if points.len() < 2 || segs == 0 {
        return points.to_vec();
    }
    let n = points.len();
    // Out = first point + (n-1) segments × segs samples each.
    let mut out = Vec::with_capacity(1 + (n - 1) * segs as usize);
    out.push(points[0]);
    let segs_f = segs as f32;
    for i in 0..n - 1 {
        // Control window [p0, p1, p2, p3]; clamped at the stroke ends to
        // duplicate the endpoint (no extrapolation beyond the stroke).
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(n - 1)];
        for t in 1..=segs {
            let t = t as f32 / segs_f;
            // Catmull-Rom basis (uniform): p(t) over [p1, p2] is
            //   0.5 * ((2 P1) + (-P0 + P2) t + (2 P0 - 5 P1 + 4 P2 - P3) t² + (-P0 + 3 P1 - 3 P2 + P3) t³)
            // which interpolates p1 at t=0 and p2 at t=1 exactly.
            let t2 = t * t;
            let t3 = t2 * t;
            let x = 0.5
                * (2.0 * p1.0
                    + (-p0.0 + p2.0) * t
                    + (2.0 * p0.0 - 5.0 * p1.0 + 4.0 * p2.0 - p3.0) * t2
                    + (-p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0) * t3);
            let y = 0.5
                * (2.0 * p1.1
                    + (-p0.1 + p2.1) * t
                    + (2.0 * p0.1 - 5.0 * p1.1 + 4.0 * p2.1 - p3.1) * t2
                    + (-p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1) * t3);
            out.push((x, y));
        }
    }
    out
}

/// Simplifica una polilínea con Douglas-Peucker (iterativo, sin recursión):
/// elimina puntos que se desvían menos de `epsilon` de la línea entre los
/// extremos del segmento (Fase C — pintado sin latencia: un trazo del dedo
/// de 100+ puntos se reduce a ~15 sin perder forma, y el rasterizado, la
/// serialización y la caché de capa pagan menos).
///
/// - `epsilon` en las mismas unidades que los puntos (coords de página, pt).
/// - Degenerado: < 2 puntos → devuelve igual; `epsilon <= 0` → devuelve
///   igual (no simplifica).
/// - Extremos SIEMPRE conservados (el primero y el último punto sobreviven).
/// - Determinista: mismo input → mismo output.
pub fn simplify_polyline(points: &[(f32, f32)], epsilon: f32) -> Vec<(f32, f32)> {
    if points.len() < 2 || epsilon <= 0.0 {
        return points.to_vec();
    }
    let mut stack: Vec<(usize, usize)> = vec![(0, points.len() - 1)];
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    while let Some((start, end)) = stack.pop() {
        if end <= start + 1 {
            continue;
        }
        let (ax, ay) = points[start];
        let (bx, by) = points[end];
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = dx * dx + dy * dy;
        let mut max_dist = 0.0f32;
        let mut max_idx = start;
        for (i, &(px, py)) in points.iter().enumerate().take(end).skip(start + 1) {
            let t = if len2 > 0.0 {
                (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (cx, cy) = (ax + t * dx, ay + t * dy);
            let d = ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt();
            if d > max_dist {
                max_dist = d;
                max_idx = i;
            }
        }
        if max_dist > epsilon && max_idx != start && max_idx != end {
            keep[max_idx] = true;
            stack.push((start, max_idx));
            stack.push((max_idx, end));
        }
    }
    points
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, p)| *p)
        .collect()
}

/// Note anchored to a page point (e.g. a margin position).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextNote {
    /// Anchor point in page coordinates.
    pub anchor: (f32, f32),
    pub text: String,
}

/// The three supported annotation kinds (Fase 3 scope).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Annotation {
    Stroke(Stroke),
    Highlight(Highlight),
    TextNote(TextNote),
}

/// One stored annotation: a unique id, the page it belongs to and its kind.
/// The id lives here (not inside the enum) so every variant shares the same
/// identity handling and `remove(id)` needs no per-variant match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotated {
    pub id: u64,
    pub page_idx: usize,
    pub kind: Annotation,
}

/// Collection of annotations indexed by page.
///
/// Backed by `HashMap<usize, Vec<Annotated>>`: per-page lookup is O(1), and
/// within a page the `Vec` keeps insertion order (= z-order for drawing).
/// `remove(id)` is a linear scan over all pages — fine for realistic
/// annotation counts; index by id if Fase 3 profiling ever demands it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnnotationSet {
    by_page: HashMap<usize, Vec<Annotated>>,
    /// Next id to hand out. Serialized so ids survive a save/load round-trip
    /// and `remove(id)` keeps working after reload. Hand-edited JSON must keep
    /// this above every stored id.
    next_id: u64,
}

impl AnnotationSet {
    /// Creates an empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `kind` to `page_idx`, assigning a fresh unique id.
    ///
    /// Returns `Some(id)` on success, `None` when the annotation is invalid
    /// (a `Stroke` with fewer than 2 points). Highlight rects are normalized
    /// to non-negative `w`/`h` before storing.
    pub fn add(&mut self, page_idx: usize, kind: Annotation) -> Option<u64> {
        if let Annotation::Stroke(s) = &kind
            && !s.is_valid()
        {
            return None;
        }
        let kind = match kind {
            Annotation::Highlight(h) => Annotation::Highlight(Highlight {
                rects: h.rects.into_iter().map(Rect::normalized).collect(),
                color: h.color,
            }),
            other => other,
        };
        let id = self.next_id;
        // saturating: a debug overflow panic here would be a bug in the data,
        // not in the logic; 2^64 ids is unreachable in practice.
        self.next_id = self.next_id.saturating_add(1);
        self.by_page
            .entry(page_idx)
            .or_default()
            .push(Annotated { id, page_idx, kind });
        Some(id)
    }

    /// Removes the annotation with `id`; returns whether it was present.
    /// Empty per-page buckets are pruned, so the map only holds pages that
    /// actually have annotations (keeps the set small in RAM).
    pub fn remove(&mut self, id: u64) -> bool {
        let mut removed = false;
        self.by_page.retain(|_, anns| {
            if removed {
                // keep every later bucket untouched: only the first match is
                // the target (ids are unique).
                return true;
            }
            match anns.iter().position(|a| a.id == id) {
                Some(pos) => {
                    anns.remove(pos);
                    removed = true;
                    // drop the bucket when it became empty (prune)
                    !anns.is_empty()
                }
                None => true,
            }
        });
        removed
    }

    /// All annotations of `page_idx`, in insertion (z) order.
    pub fn for_page(&self, page_idx: usize) -> Vec<&Annotated> {
        match self.by_page.get(&page_idx) {
            Some(anns) => anns.iter().collect(),
            None => Vec::new(),
        }
    }

    /// Total number of annotations across all pages.
    pub fn len(&self) -> usize {
        self.by_page.values().map(Vec::len).sum()
    }

    /// Whether the set holds no annotations. Defined as `len() == 0`, not map
    /// emptiness, so a set deserialized from hand-edited JSON with empty
    /// per-page buckets still reports empty correctly.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// All ids in the set, sorted ascending (sorted for deterministic output;
    /// the internal map has no order).
    pub fn ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.by_page.values().flatten().map(|a| a.id).collect();
        ids.sort_unstable();
        ids
    }
}

/// Squared distance from point `p` to the segment `a`–`b` (page coordinates).
/// Uses the closest point on the segment (clamped parameter t), so the eraser
/// hits mid-segment gaps the same way as a real wide stroke would.
fn dist2_point_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, py) = p;
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let denom = abx * abx + aby * aby;
    let t = if denom == 0.0 {
        0.0
    } else {
        (((px - a.0) * abx + (py - a.1) * aby) / denom).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * abx, a.1 + t * aby);
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy
}

/// Hit-test of the ERASER against a stroke: `true` when `pt` is within
/// `radius` (page points) of the polyline, measured per SEGMENT (distance
/// point→segment) plus half the stroke width — the effective hit radius is
/// `radius + width/2`, so a wide marker feels its ink.
///
/// Pure function (no model/store access): the caller resolves `Stroke` from
/// the set. Tests in [`tests`](self).
pub fn stroke_hit(stroke: &Stroke, pt: (f32, f32), radius: f32) -> bool {
    let r = radius + stroke.width / 2.0;
    let r2 = r * r;
    stroke
        .points
        .windows(2)
        .any(|w| dist2_point_segment(pt, w[0], w[1]) <= r2)
}

/// Hit-test of the ERASER against a highlight: `true` when `pt` lies inside
/// any highlight rect EXPANDED by `pad` page points in every direction (the
/// eraser feels a padded box, so a thin underline line is easy to hit).
///
/// Pure function; tests in [`tests`](self).
pub fn highlight_hit(h: &Highlight, pt: (f32, f32), pad: f32) -> bool {
    h.rects.iter().any(|r| {
        pt.0 >= r.x - pad && pt.0 <= r.x + r.w + pad && pt.1 >= r.y - pad && pt.1 <= r.y + r.h + pad
    })
}

/// Sliver de un rect recortado por la goma: trozos más cortos no se crean
/// (evita motas en el subrayado al pasar la goma por el borde).
const ERASE_MIN_SLIVER_PT: f32 = 2.0;

/// GOMA REAL sobre un trazo: elimina los vértices que caen dentro del círculo
/// de la goma (`radius` + `width/2` efectivos) o dentro del BARRIIDO entre la
/// posición anterior de la goma (`prev`) y la actual (`center`) — un barrido
/// continuo no deja islas entre pasadas. Los tramos contiguos NO tocados se
/// devuelven como trazos separados (una goma parte la línea en trozos); los
/// tramos degenerados (1 punto) se descartan.
///
/// Devuelve `None` si el trazo no fue tocado (el llamador no toca nada);
/// `Some(trozos)` si lo fue (puede ser vacío → eliminar el trazo entero).
/// Pura: el llamador decide el `remove`/`add` y la persistencia.
/// GOMA REAL sobre un trazo: recorta los SEGMENTOS que cruzan el círculo de la
/// goma (`radius` + `width/2` efectivos) en sus puntos de intersección — el
/// hueco es EXACTAMENTE el círculo, como una goma real, sin "mordiscos" de
/// vértices. El barrido entre la posición anterior (`prev`) y la actual
/// (`center`) se muestrea en círculos intermedios, así una pasada rápida corta
/// el trazo por todo el camino recorrido (sin islas entre frames). Los trozos
/// contiguos que sobreviven se devuelven como trazos separados; las motas
/// menores de 1.5 pt se descartan.
///
/// Devuelve `None` si el trazo no fue tocado; `Some(trozos)` si lo fue (puede
/// ser vacío → eliminar el trazo entero). Pura: el llamador decide el
/// `remove`/`add` y la persistencia.
pub fn split_stroke(
    stroke: &Stroke,
    center: (f32, f32),
    radius: f32,
    prev: Option<(f32, f32)>,
) -> Option<Vec<Stroke>> {
    let r = radius + stroke.width / 2.0;
    // Círculos de la goma: barrido muestreado (4) + posición actual.
    let mut centers: Vec<(f32, f32)> = Vec::new();
    if let Some(pr) = prev {
        for i in 1..=4 {
            let t = i as f32 / 4.0;
            centers.push((pr.0 + (center.0 - pr.0) * t, pr.1 + (center.1 - pr.1) * t));
        }
    }
    centers.push(center);

    // 1) Recortar cada segmento del trazo por TODOS los círculos.
    let mut segs: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for w in stroke.points.windows(2) {
        segs.push((w[0], w[1]));
    }
    let mut any_touched = false;
    for c in &centers {
        let mut next: Vec<((f32, f32), (f32, f32))> = Vec::new();
        for (a, b) in segs {
            let orig_len = seg_len(a, b);
            let pieces = clip_segment_by_circle(a, b, *c, r);
            if pieces.is_empty() {
                any_touched = true; // el segmento entero se comió
            } else {
                let kept_len: f32 = pieces.iter().map(|(x, y)| seg_len(*x, *y)).sum();
                if kept_len < orig_len - 1e-3 {
                    any_touched = true;
                }
                next.extend(pieces);
            }
        }
        segs = next;
    }
    if !any_touched {
        return None;
    }

    // 2) Unir trozos contiguos en polilíneas (runs) y descartar motas.
    const MIN_PIECE_PT: f32 = 1.5;
    let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    for (a, b) in segs {
        if seg_len(a, b) < MIN_PIECE_PT {
            continue; // mota: se descarta
        }
        if cur.is_empty() {
            cur = vec![a, b];
        } else if let Some(last) = cur.last()
            && dist2(*last, a) < 1e-3
        {
            cur.push(b);
        } else {
            runs.push(std::mem::take(&mut cur));
            cur = vec![a, b];
        }
    }
    if cur.len() >= 2 {
        runs.push(cur);
    }
    Some(
        runs.into_iter()
            .map(|points| Stroke {
                points,
                width: stroke.width,
                color: stroke.color,
            })
            .collect(),
    )
}

/// Distancia entre dos puntos (euclídea).
fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// Longitud de un segmento.
fn seg_len(a: (f32, f32), b: (f32, f32)) -> f32 {
    dist2(a, b).sqrt()
}

/// Interpolación lineal entre `a` y `b` en `t` (0..1).
fn lerp(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Recorta el segmento `a→b` con el CÍRCULO (c, r): devuelve los trozos del
/// segmento que quedan FUERA del círculo (intersección exacta punto→círculo,
/// https://paulbourke.net/geometry/circlesphere/); vacío si el segmento está
/// completamente dentro.
fn clip_segment_by_circle(
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    r: f32,
) -> Vec<((f32, f32), (f32, f32))> {
    let d = (b.0 - a.0, b.1 - a.1);
    let f = (a.0 - c.0, a.1 - c.1);
    let a2 = d.0 * d.0 + d.1 * d.1;
    if a2 <= 0.0 {
        return Vec::new();
    }
    let bb = 2.0 * (f.0 * d.0 + f.1 * d.1);
    let cc = f.0 * f.0 + f.1 * f.1 - r * r;
    let disc = bb * bb - 4.0 * a2 * cc;
    if disc <= 0.0 {
        // Sin intersección: ¿el punto medio dentro del círculo?
        let m = lerp(a, b, 0.5);
        if (m.0 - c.0).powi(2) + (m.1 - c.1).powi(2) <= r * r {
            Vec::new() // el segmento entero está dentro
        } else {
            vec![(a, b)] // entero fuera
        }
    } else {
        let t1 = ((-bb - disc.sqrt()) / (2.0 * a2)).clamp(0.0, 1.0);
        let t2 = ((-bb + disc.sqrt()) / (2.0 * a2)).clamp(0.0, 1.0);
        let mut out = Vec::new();
        if t1 > 0.0 {
            out.push((a, lerp(a, b, t1)));
        }
        if t2 < 1.0 {
            out.push((lerp(a, b, t2), b));
        }
        out
    }
}

/// GOMA REAL sobre un subrayado: cada rect cuya expansión `pad` contiene el
/// punto de la goma — o es cruzado por el BARRIO de la goma entre `prev` y
/// `center` (muestreo del segmento) — se parte en dos (izquierda/derecha del
/// corte); los trozos menores de `ERASE_MIN_SLIVER_PT` se descartan. Los
/// rects no tocados se conservan. `None` = no tocado; `Some(rects)` = rects
/// restantes (puede ser vacío → eliminar el highlight). Pura.
pub fn trim_highlight(
    h: &Highlight,
    center: (f32, f32),
    pad: f32,
    prev: Option<(f32, f32)>,
) -> Option<Vec<Rect>> {
    let mut out: Vec<Rect> = Vec::new();
    let mut any = false;
    for r in &h.rects {
        let hit_at = |p: (f32, f32)| -> bool {
            p.0 >= r.x - pad && p.0 <= r.x + r.w + pad && p.1 >= r.y - pad && p.1 <= r.y + r.h + pad
        };
        // Punto actual o, si existe barrido, la primera muestra del segmento
        // prev→center que toca la caja (una goma rápida no puede "saltar"
        // por encima de una línea de subrayado entre dos frames).
        let mut sweep_cut: Option<f32> = None;
        if let Some(pr) = prev {
            const SWEEP_SAMPLES: usize = 8;
            for i in 1..=SWEEP_SAMPLES {
                let t = i as f32 / SWEEP_SAMPLES as f32;
                let s = (pr.0 + (center.0 - pr.0) * t, pr.1 + (center.1 - pr.1) * t);
                if hit_at(s) {
                    sweep_cut = Some(s.0);
                    break;
                }
            }
        }
        if !hit_at(center) && sweep_cut.is_none() {
            out.push(*r);
            continue;
        }
        any = true;
        // Recortar al rect real (un toque en el pad exterior corta en el borde).
        let cut_x = sweep_cut.unwrap_or(center.0).clamp(r.x, r.x + r.w);
        let left = Rect::new(r.x, r.y, cut_x - r.x, r.h);
        let right = Rect::new(cut_x, r.y, r.x + r.w - cut_x, r.h);
        if left.w >= ERASE_MIN_SLIVER_PT {
            out.push(left);
        }
        if right.w >= ERASE_MIN_SLIVER_PT {
            out.push(right);
        }
    }
    if any { Some(out) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color() -> Color {
        Color {
            r: 255,
            g: 0,
            b: 0,
            a: 200,
        }
    }

    fn stroke() -> Stroke {
        Stroke::new(
            vec![(10.0, 20.0), (30.5, 40.25), (50.0, 60.0)],
            2.5,
            color(),
        )
        .expect("valid stroke")
    }

    fn highlight() -> Highlight {
        Highlight {
            rects: vec![
                Rect::new(10.0, 20.0, 100.0, 12.0),
                Rect::new(10.0, 34.0, 80.0, 12.0),
            ],
            color: color(),
        }
    }

    fn text_note() -> TextNote {
        TextNote {
            anchor: (5.0, 5.0),
            text: "revisar §3".to_string(),
        }
    }

    /// serde_json round-trips `Annotation` exactly (struct equality) and the
    /// produced JSON is byte-identical to the expected externally-tagged form.
    fn assert_exact_round_trip(ann: &Annotation, expected_json: &str) {
        let json = serde_json::to_string(ann).expect("serialize");
        assert_eq!(json, expected_json);
        let back: Annotation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&back, ann);
    }

    #[test]
    fn stroke_serde_round_trip_is_exact() {
        assert_exact_round_trip(
            &Annotation::Stroke(stroke()),
            r#"{"Stroke":{"points":[[10.0,20.0],[30.5,40.25],[50.0,60.0]],"width":2.5,"color":{"r":255,"g":0,"b":0,"a":200}}}"#,
        );
    }

    #[test]
    fn highlight_serde_round_trip_is_exact() {
        assert_exact_round_trip(
            &Annotation::Highlight(highlight()),
            r#"{"Highlight":{"rects":[{"x":10.0,"y":20.0,"w":100.0,"h":12.0},{"x":10.0,"y":34.0,"w":80.0,"h":12.0}],"color":{"r":255,"g":0,"b":0,"a":200}}}"#,
        );
    }

    #[test]
    fn text_note_serde_round_trip_is_exact() {
        assert_exact_round_trip(
            &Annotation::TextNote(text_note()),
            r#"{"TextNote":{"anchor":[5.0,5.0],"text":"revisar §3"}}"#,
        );
    }

    #[test]
    fn set_serde_round_trip_preserves_ids_and_pages() {
        let mut set = AnnotationSet::new();
        let ids = [
            set.add(0, Annotation::Stroke(stroke())).expect("add"),
            set.add(1, Annotation::Stroke(stroke())).expect("add"),
            set.add(1, Annotation::Highlight(highlight())).expect("add"),
            set.add(2, Annotation::TextNote(text_note())).expect("add"),
        ];
        let json = serde_json::to_string(&set).expect("serialize");
        let mut back: AnnotationSet = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, set);
        assert_eq!(back.len(), 4);
        for id in ids {
            assert!(back.remove(id), "id {id} should be present after reload");
        }
        assert!(back.is_empty());
        // the empty set round-trips too
        let empty_json = serde_json::to_string(&back).expect("serialize");
        let empty_back: AnnotationSet = serde_json::from_str(&empty_json).expect("deserialize");
        assert_eq!(empty_back, back);
    }

    #[test]
    fn for_page_returns_only_that_page_in_insertion_order() {
        let mut set = AnnotationSet::new();
        let page0 = set.add(0, Annotation::Stroke(stroke())).expect("add");
        let note = set.add(1, Annotation::TextNote(text_note())).expect("add");
        let hl = set.add(1, Annotation::Highlight(highlight())).expect("add");

        let page1 = set.for_page(1);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].id, note);
        assert!(matches!(page1[0].kind, Annotation::TextNote(_)));
        assert_eq!(page1[1].id, hl);
        assert!(matches!(page1[1].kind, Annotation::Highlight(_)));

        let page0_anns = set.for_page(0);
        assert_eq!(page0_anns.len(), 1);
        assert_eq!(page0_anns[0].id, page0);

        assert!(set.for_page(99).is_empty());
    }

    #[test]
    fn remove_deletes_only_the_target_id() {
        let mut set = AnnotationSet::new();
        let a = set.add(0, Annotation::Stroke(stroke())).expect("add");
        let b = set.add(1, Annotation::TextNote(text_note())).expect("add");

        assert!(set.remove(a));
        assert!(
            !set.remove(a),
            "second remove of the same id must be a no-op"
        );
        assert_eq!(set.len(), 1);
        assert!(set.for_page(0).is_empty());
        assert_eq!(set.for_page(1)[0].id, b);
        assert_eq!(set.ids(), vec![b]);
    }

    #[test]
    fn add_assigns_unique_ids() {
        let mut set = AnnotationSet::new();
        let mut ids = vec![
            set.add(0, Annotation::Stroke(stroke())).expect("add"),
            set.add(1, Annotation::TextNote(text_note())).expect("add"),
        ];
        for page in 0..5 {
            ids.push(
                set.add(page, Annotation::Highlight(highlight()))
                    .expect("add"),
            );
        }
        assert_eq!(set.len(), ids.len());
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "id {id} assigned twice");
        }
        // ids() is sorted; it must cover exactly the assigned ids.
        ids.sort_unstable();
        assert_eq!(set.ids(), ids);
    }

    #[test]
    fn stroke_requires_at_least_two_points() {
        // constructor refuses degenerate polylines
        assert!(Stroke::new(vec![], 2.0, color()).is_none());
        assert!(Stroke::new(vec![(1.0, 1.0)], 2.0, color()).is_none());
        assert!(Stroke::new(vec![(0.0, 0.0), (1.0, 1.0)], 2.0, color()).is_some());

        // public fields bypass `new`; `add` still rejects the degenerate stroke
        let mut set = AnnotationSet::new();
        let raw = Annotation::Stroke(Stroke {
            points: vec![(1.0, 1.0)],
            width: 2.0,
            color: color(),
        });
        assert!(set.add(0, raw).is_none());
        assert!(set.is_empty());

        // negative width is clamped to 0, not rejected
        let s = Stroke::new(vec![(0.0, 0.0), (1.0, 1.0)], -3.0, color()).expect("valid");
        assert_eq!(s.width, 0.0);
    }

    #[test]
    fn smooth_polyline_keeps_endpoints_and_interpolates() {
        let pts = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)];
        // A colinear flat line stays exactly on the line; the first point is
        // the hard start, then 3 segments × 4 samples each.
        let s = smooth_polyline(&pts, 4);
        assert_eq!(s.first(), Some(&(0.0, 0.0)));
        assert_eq!(s.last(), Some(&(30.0, 0.0)));
        assert_eq!(s.len(), 1 + 3 * 4);
        for (x, y) in &s {
            assert!((0.0..=30.0).contains(x));
            assert!(y.abs() < 1e-5, "flat line stays flat, got y={y}");
        }

        // A corner is properly rounded: the curve passes *exactly* through
        // the vertex control point (Catmull-Rom interpolates the controls
        // at t=1 for each inner segment) and then climbs upwards.
        let corner = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        let s = smooth_polyline(&corner, 8);
        // out = [P0] + seg0 (8 pts ending at P1) + seg1 (8 pts ending at P2).
        assert_eq!(s.len(), 17);
        assert_eq!(s[8], (10.0, 0.0), "curve interpolates the vertex");
        // After the vertex the stroke climbs: y > 0 on the first sample of
        // the second segment, and the endpoint is exactly the last vertex.
        let after = s[9];
        assert!(
            after.1 > 0.0 && s[16] == (10.0, 10.0),
            "climb into the corner: {after:?}"
        );
        for p in &s {
            assert!(p.0 <= 12.0 && p.1 <= 12.0, "overshoot bound at {p:?}");
        }
    }

    #[test]
    fn smooth_polyline_passthrough_for_degenerate_input() {
        assert_eq!(smooth_polyline(&[], 4), Vec::<(f32, f32)>::new());
        assert_eq!(smooth_polyline(&[(1.0, 1.0)], 4), vec![(1.0, 1.0)]);
        // segs = 0 means no interpolation at all.
        let pts = vec![(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)];
        assert_eq!(smooth_polyline(&pts, 0), pts);
    }

    #[test]
    fn simplify_keeps_endpoints_and_reduces_collinear_points() {
        // Línea recta larga con muchos puntos: DP elimina todos los
        // intermedios (desviación ~0 < epsilon).
        let pts: Vec<(f32, f32)> = (0..50).map(|i| (i as f32 * 2.0, i as f32)).collect();
        let out = simplify_polyline(&pts, 1.5);
        assert_eq!(out.len(), 2, "solo extremos en línea recta");
        assert_eq!(out[0], pts[0]);
        assert_eq!(out[1], *pts.last().unwrap());
    }

    #[test]
    fn simplify_keeps_sharp_corners() {
        // Esquina en ángulo recto: el vértice se desvía mucho → se conserva.
        let pts = vec![
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (10.0, 20.0),
        ];
        let out = simplify_polyline(&pts, 0.5);
        assert_eq!(out.len(), 3, "extremos + esquina");
        assert!(
            out.contains(&(10.0, 0.0)) || out.contains(&(10.0, 10.0)),
            "el vértice de la esquina"
        );
        // El punto (5,0) colineal se elimina.
        assert!(!out.contains(&(5.0, 0.0)));
    }

    #[test]
    fn simplify_respects_epsilon_threshold() {
        // Punto con desviación 1.0: conservado con epsilon 0.5, eliminado
        // con epsilon 2.0.
        let pts = vec![(0.0, 0.0), (5.0, 1.0), (10.0, 0.0)];
        assert_eq!(simplify_polyline(&pts, 0.5).len(), 3);
        assert_eq!(simplify_polyline(&pts, 2.0).len(), 2);
    }

    #[test]
    fn simplify_degenerate_inputs_are_noops() {
        assert_eq!(simplify_polyline(&[], 1.0), Vec::<(f32, f32)>::new());
        assert_eq!(simplify_polyline(&[(1.0, 1.0)], 1.0), vec![(1.0, 1.0)]);
        // epsilon <= 0: sin simplificación.
        let pts = vec![(0.0, 0.0), (5.0, 1.0), (10.0, 0.0)];
        assert_eq!(simplify_polyline(&pts, 0.0), pts);
        assert_eq!(simplify_polyline(&pts, -1.0), pts);
    }

    #[test]
    fn simplify_deterministic_same_input_same_output() {
        let pts: Vec<(f32, f32)> = (0..100)
            .map(|i| {
                let x = i as f32 * 0.7;
                let y = (i as f32 * 0.3).sin() * 5.0;
                (x, y)
            })
            .collect();
        let a = simplify_polyline(&pts, 1.5);
        let b = simplify_polyline(&pts, 1.5);
        assert_eq!(a, b);
    }

    #[test]
    fn rect_normalizes_negative_extents() {
        let cases = [
            (
                Rect::new(10.0, 20.0, -5.0, 10.0),
                Rect {
                    x: 5.0,
                    y: 20.0,
                    w: 5.0,
                    h: 10.0,
                },
            ),
            (
                Rect::new(10.0, 20.0, 5.0, -10.0),
                Rect {
                    x: 10.0,
                    y: 10.0,
                    w: 5.0,
                    h: 10.0,
                },
            ),
            (
                Rect::new(10.0, 20.0, -5.0, -10.0),
                Rect {
                    x: 5.0,
                    y: 10.0,
                    w: 5.0,
                    h: 10.0,
                },
            ),
            (
                Rect::new(10.0, 20.0, 5.0, 10.0),
                Rect {
                    x: 10.0,
                    y: 20.0,
                    w: 5.0,
                    h: 10.0,
                },
            ),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }

        // `add` normalizes highlight rects even when built raw via public fields
        let mut set = AnnotationSet::new();
        let raw = Annotation::Highlight(Highlight {
            rects: vec![Rect {
                x: 10.0,
                y: 20.0,
                w: -5.0,
                h: -10.0,
            }],
            color: color(),
        });
        set.add(0, raw).expect("add");
        match &set.for_page(0)[0].kind {
            Annotation::Highlight(h) => {
                assert_eq!(
                    h.rects[0],
                    Rect {
                        x: 5.0,
                        y: 10.0,
                        w: 5.0,
                        h: 10.0
                    }
                );
            }
            other => panic!("expected a highlight, got {other:?}"),
        }
    }

    // --- Eraser hit-tests (pure functions; see `stroke_hit`/`highlight_hit`) ---

    #[test]
    fn stroke_hit_hits_midpoint_of_a_segment() {
        let s = Stroke::new(vec![(0.0, 0.0), (100.0, 0.0)], 2.0, color()).unwrap();
        // Mid-segment point, 2 pt away: within radius 8 + width/2 = 9.
        assert!(stroke_hit(&s, (50.0, 2.0), 8.0));
        // 12 pt away: outside the same effective radius.
        assert!(!stroke_hit(&s, (50.0, 12.0), 8.0));
    }

    #[test]
    fn stroke_hit_hits_endpoint_and_uses_width() {
        let s = Stroke::new(vec![(0.0, 0.0), (10.0, 0.0)], 10.0, color()).unwrap();
        // Endpoint hit with radius 0 thanks to the width (10/2 = 5 > 0).
        assert!(stroke_hit(&s, (0.0, 4.0), 0.0));
        assert!(!stroke_hit(&s, (0.0, 6.0), 0.0));
    }

    #[test]
    fn stroke_hit_ignores_far_points_of_polyline() {
        let s = Stroke::new(vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 2.0, color()).unwrap();
        // Near the second segment, far from the first one.
        assert!(stroke_hit(&s, (10.0, 5.0), 8.0));
        // Far from both segments.
        assert!(!stroke_hit(&s, (50.0, 50.0), 8.0));
    }

    #[test]
    fn highlight_hit_respects_expansion_pad() {
        let h = Highlight {
            rects: vec![Rect::new(10.0, 20.0, 100.0, 12.0)],
            color: color(),
        };
        // Inside the rect.
        assert!(highlight_hit(&h, (50.0, 26.0), 4.0));
        // Inside the 4 pt expansion, outside the raw rect: hits.
        assert!(highlight_hit(&h, (10.0, 16.0), 4.0));
        assert!(highlight_hit(&h, (8.0, 21.0), 4.0));
        // Beyond the pad: misses.
        assert!(!highlight_hit(&h, (10.0, 15.0), 4.0));
        assert!(!highlight_hit(&h, (5.0, 21.0), 4.0));
    }

    #[test]
    fn highlight_hit_any_rect_of_multi_line() {
        let h = Highlight {
            rects: vec![
                Rect::new(10.0, 20.0, 100.0, 12.0),
                Rect::new(10.0, 40.0, 80.0, 12.0),
            ],
            color: color(),
        };
        assert!(highlight_hit(&h, (20.0, 45.0), 0.0)); // second line
        assert!(!highlight_hit(&h, (20.0, 35.0), 0.0)); // between lines
    }

    // --- Eraser trimming (the real-gum split; see `split_stroke`/`trim_highlight`) ---

    #[test]
    fn split_stroke_none_when_not_touched() {
        let s = Stroke::new(vec![(0.0, 0.0), (50.0, 0.0)], 2.0, color()).unwrap();
        assert!(split_stroke(&s, (25.0, 40.0), 8.0, None).is_none());
    }

    #[test]
    fn split_stroke_cuts_middle_leaving_two_pieces() {
        let s = Stroke::new(
            vec![
                (0.0, 0.0),
                (10.0, 0.0),
                (20.0, 0.0),
                (30.0, 0.0),
                (40.0, 0.0),
            ],
            2.0,
            color(),
        )
        .unwrap();
        // Goma en x=20: elimina los vértices 20 (y vecinos a <= 9 pt: 10 y 30
        // están a 10 pt -> fuera; con radio 8 + width 1 = 9, quedan 10 y 30
        // fuera por 1 pt).
        let parts = split_stroke(&s, (20.0, 0.0), 8.0, None).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].points, vec![(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(parts[1].points, vec![(30.0, 0.0), (40.0, 0.0)]);
    }

    #[test]
    fn split_stroke_wide_gum_erases_whole_short_stroke() {
        let s = Stroke::new(vec![(0.0, 0.0), (10.0, 0.0)], 2.0, color()).unwrap();
        let parts = split_stroke(&s, (5.0, 0.0), 8.0, None).unwrap();
        assert!(parts.is_empty()); // todos los vértices dentro del círculo
    }

    #[test]
    fn split_stroke_sweep_removes_points_between_passes() {
        // El barrido (5,0)→(15,0) recorre el trazo: borra el vértice (10,0)
        // aunque NO está en ningún círculo (radio efectivo 2 pt). Los
        // extremos (0,0),(2,0) y (18,0),(20,0) quedan como dos trozos.
        let s = Stroke::new(
            vec![
                (0.0, 0.0),
                (2.0, 0.0),
                (10.0, 0.0),
                (18.0, 0.0),
                (20.0, 0.0),
            ],
            2.0,
            color(),
        )
        .unwrap();
        let parts = split_stroke(&s, (15.0, 0.0), 1.0, Some((5.0, 0.0))).unwrap();
        assert_eq!(parts.len(), 2);
        // El recorte por intersección conserva la parte hasta el borde del
        // primer círculo del barrido (x=5.5: círculo en 7.5, radio 2) y la
        // parte final desde x=17 (círculo en 15, radio 2) — las motas
        // intermedias se descartan.
        assert_eq!(parts[0].points, vec![(0.0, 0.0), (2.0, 0.0), (5.5, 0.0)]);
        assert_eq!(parts[1].points, vec![(18.0, 0.0), (20.0, 0.0)]);
        // Sin barrido y con el círculo LEJOS de todos los segmentos
        // (y=10, radio efectivo 2) → `None` (nada que recortar).
        assert!(split_stroke(&s, (15.0, 10.0), 1.0, None).is_none());
    }

    #[test]
    fn trim_highlight_splits_touched_line_at_eraser() {
        let h = Highlight {
            rects: vec![Rect::new(10.0, 20.0, 100.0, 12.0)],
            color: color(),
        };
        let rects = trim_highlight(&h, (50.0, 26.0), 4.0, None).unwrap();
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect::new(10.0, 20.0, 40.0, 12.0));
        assert_eq!(rects[1], Rect::new(50.0, 20.0, 60.0, 12.0));
    }

    #[test]
    fn trim_highlight_keeps_untouched_lines_and_handles_edges() {
        let h = Highlight {
            rects: vec![
                Rect::new(10.0, 20.0, 100.0, 12.0),
                Rect::new(10.0, 40.0, 80.0, 12.0),
            ],
            color: color(),
        };
        // Toca la segunda línea: la primera se conserva intacta.
        let rects = trim_highlight(&h, (20.0, 45.0), 4.0, None).unwrap();
        assert!(rects.contains(&Rect::new(10.0, 20.0, 100.0, 12.0)));
        assert_eq!(rects.len(), 3); // izquierda + derecha del corte + línea 1
        // Toca a 1 pt del borde IZQUIERDO: el trozo izquierdo es un sliver
        // (< 2 pt) y se descarta; el derecho queda casi completo (99 pt).
        let rects = trim_highlight(&h, (11.0, 26.0), 4.0, None).unwrap();
        assert_eq!(rects.len(), 2);
        assert!(rects.contains(&Rect::new(10.0, 40.0, 80.0, 12.0)));
        assert!(rects.contains(&Rect::new(11.0, 20.0, 99.0, 12.0)));
    }

    #[test]
    fn trim_highlight_none_when_not_touched() {
        let h = Highlight {
            rects: vec![Rect::new(10.0, 20.0, 100.0, 12.0)],
            color: color(),
        };
        assert!(trim_highlight(&h, (10.0, 60.0), 4.0, None).is_none());
    }

    #[test]
    fn trim_highlight_sweep_hits_when_center_jumped_over() {
        // La goma se mueve rápido: el CENTRO cae fuera del rect expandido
        // (x=120 > 114) pero el barrido prev→center CRUZA la caja → corta.
        let h = Highlight {
            rects: vec![Rect::new(10.0, 20.0, 100.0, 12.0)],
            color: color(),
        };
        // El barrido CRUZA la caja por la primera muestra (t=1/8 → x=19.375),
        // así que el rect se parte en dos por ese x.
        let rects = trim_highlight(&h, (120.0, 26.0), 4.0, Some((5.0, 26.0))).unwrap();
        assert_eq!(rects.len(), 2);
        assert!(rects.contains(&Rect::new(10.0, 20.0, 9.375, 12.0)));
        assert!(rects.contains(&Rect::new(19.375, 20.0, 90.625, 12.0)));
        // Sin barrido, el mismo centro NO toca nada.
        assert!(trim_highlight(&h, (120.0, 26.0), 4.0, None).is_none());
    }
}
