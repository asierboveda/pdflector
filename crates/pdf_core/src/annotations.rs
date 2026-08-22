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
}
