// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pure rasterization of the annotation layer onto a page RGBA bitmap
//! (AGENTS.md §4.3: annotations are a vector overlay drawn over the cached
//! page bitmap, never baked into it). This module is the Fase 3/Android
//! bridge: `pdf_android` calls [`composite_annotations`] on the cached page
//! buffer every frame, with the current zoom/offset, before blitting.
//!
//! # Performance contract
//!
//! The rasterizer is tuned for the frame budget: **no allocation per pixel**
//! (no pixel buffers, no per-pixel Vecs — the only allocations are the
//! per-annotation screen rects, and none at all for the ink path), and the
//! two drawing primitives are chosen for speed:
//!
//! - **Highlight quads** (axis-aligned rects in page space — the per-line
//!   boxes of [`Highlight`]) are filled as spans: per scanline, per-pixel
//!   coverage is the 1-D overlap of the pixel's unit interval with the rect
//!   edge, so a quad with `w×h` screen pixels costs O(h+w) work, not O(w×h)
//!   per edge. Antialiasing is exact for the axis-aligned case (a 1-D
//!   coverage equals the covered area fraction).
//! - **Ink strokes** are polyline segments drawn as thick lines with a
//!   1-px antialiasing fringe: per segment only its screen bounding box is
//!   visited, coverage is `clamp((radius + 0.5 - d) , 0, 1)` for the
//!   distance `d` to the segment — a reasonable smooth edge without a
//!   full-scanline fill.
//!
//! Both paths blend source-over (porter-duff) into the existing buffer, so
//! the bitmap stays in place (no copy, no alpha pass) and the layer is
//! trivially re-rendered on every zoom/scroll change.
//!
//! Coordinates: everything enters in **page coordinates** (PDF points) and
//! is mapped through [`ViewTransform`] (zoom + translation) to pixel space.
//! The buffer writes are clamped to `width×height`; geometry outside is
//! skipped cheaply.

use crate::annotations::{Annotated, Annotation, Color, Rect, Stroke};

/// Maps page coordinates (PDF points, top-left origin, y down) to screen
/// pixels via uniform `zoom` (device px per point) and a translation.
/// `screen = page * zoom + offset`.
#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    /// Device pixels per PDF point (1.0 = 72 dpi).
    pub zoom: f32,
    /// Screen position of the page origin, in device pixels.
    pub offset_x: f32,
    /// Screen position of the page origin, in device pixels.
    pub offset_y: f32,
}

impl ViewTransform {
    /// Identity transform (zoom 1, origin at 0,0) — mainly for tests.
    pub const IDENTITY: Self = Self {
        zoom: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };

    /// Maps one page point to screen pixels.
    #[inline]
    pub fn page_to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.zoom + self.offset_x, y * self.zoom + self.offset_y)
    }
}

/// Rasterizes all annotations of one page onto `buf` (RGBA8, row-major,
/// `width * height` bytes). `anns` are the annotations of that page in z
/// order (later ones paint on top), e.g. `set.for_page(page)`.
///
/// Pure and infallible: `buf.len()` must equal `width * height * 4`
/// (`debug_assert`ed); no other failure mode exists, so the caller can call
/// it from the render path without error handling.
pub fn composite_annotations(
    buf: &mut [u8],
    width: u32,
    height: u32,
    anns: &[&Annotated],
    xform: &ViewTransform,
) {
    debug_assert_eq!(
        buf.len(),
        (width as usize) * (height as usize) * 4,
        "annotation layer buffer must be RGBA8 with exactly width*height*4 bytes"
    );
    if width == 0 || height == 0 || buf.is_empty() {
        return;
    }
    for a in anns {
        match &a.kind {
            Annotation::Highlight(hl) => {
                for r in &hl.rects {
                    fill_rect(buf, width, height, xform, r, hl.color);
                }
            }
            Annotation::Stroke(s) => draw_stroke(buf, width, height, xform, s),
            Annotation::TextNote(_) => {
                // Notes are rendered by the UI (they carry a string, not
                // geometry); nothing to paint in the layer.
            }
        }
    }
}

/// Screen-space bounds of a page rect under `xform`, clamped to the bitmap.
/// Returns `None` when entirely outside.
#[inline]
fn screen_rect(xform: &ViewTransform, r: &Rect, w: f32, h: f32) -> Option<(f32, f32, f32, f32)> {
    let r = r.normalized();
    let (x0, y0) = xform.page_to_screen(r.x, r.y);
    let (x1, y1) = xform.page_to_screen(r.x + r.w, r.y + r.h);
    let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (y0, y1) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
    if x1 <= 0.0 || y1 <= 0.0 || x0 >= w || y0 >= h {
        return None;
    }
    Some((x0.max(0.0), y0.max(0.0), x1.min(w), y1.min(h)))
}

/// Fills one highlight quad (axis-aligned rect). Per scanline, coverage is
/// the exact 1-D overlap fraction of each pixel interval with the rect, so
/// edges get smoothly antialiased at O(h+w) total.
fn fill_rect(
    buf: &mut [u8],
    width: u32,
    height: u32,
    xform: &ViewTransform,
    r: &Rect,
    color: Color,
) {
    let (w, h) = (width as f32, height as f32);
    let Some((rx0, ry0, rx1, ry1)) = screen_rect(xform, r, w, h) else {
        return;
    };
    let y_start = ry0.floor() as i32;
    let y_end = (ry1 - 1.0).ceil() as i32 + 1;
    let x_start = rx0.floor() as i32;
    let x_end = (rx1 - 1.0).ceil() as i32 + 1;
    if y_end <= y_start || x_end <= x_start {
        return;
    }
    // Row start of the scanline buffer for the first covered row.
    let row0 = (y_start.max(0) as usize) * (width as usize);
    for y in y_start..y_end {
        if y < 0 || y as u32 >= height {
            continue;
        }
        let fy = y as f32;
        // Vertical coverage: overlap of [y, y+1) with [ry0, ry1).
        let cov_y = (ry1.min(fy + 1.0) - ry0.max(fy)).clamp(0.0, 1.0);
        if cov_y <= 0.0 {
            continue;
        }
        let row = row0 + (y as usize - y_start as usize) * (width as usize);
        for x in x_start..x_end {
            if x < 0 || x as u32 >= width {
                continue;
            }
            let fx = x as f32;
            let cov_x = (rx1.min(fx + 1.0) - rx0.max(fx)).clamp(0.0, 1.0);
            if cov_x <= 0.0 {
                continue;
            }
            blend_pixel(buf, (row + x as usize) * 4, color, cov_x * cov_y);
        }
    }
}

/// Draws one ink stroke as a thick polyline: each segment is rasterized over
/// its own padded bounding box with a 1-px antialiased fringe. Thickness and
/// colour come from the stroke model; alpha blends over the page bitmap.
/// One thick segment of an ink stroke, already mapped to screen pixels.
struct Seg {
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
}

/// Draws one ink stroke as a thick polyline: each segment is rasterized over
/// its own padded bounding box with a 1-px antialiased fringe. Thickness and
/// colour come from the stroke model; alpha blends over the page bitmap.
fn draw_stroke(buf: &mut [u8], width: u32, height: u32, xform: &ViewTransform, s: &Stroke) {
    if !s.is_valid() {
        return;
    }
    let (w, h) = (width as f32, height as f32);
    // Half-width in screen pixels, with at least 0.5 px of fringe room so a
    // hairlike stroke (width ~0) is still visible.
    let half = (s.width * xform.zoom * 0.5).max(0.5);
    let mut prev = s.points[0];
    for &p in &s.points[1..] {
        let (ax, ay) = xform.page_to_screen(prev.0, prev.1);
        let (bx, by) = xform.page_to_screen(p.0, p.1);
        draw_segment(buf, width, w, h, Seg { ax, ay, bx, by }, half, s.color);
        prev = p;
    }
}

/// Rasterizes one thick segment (screen pixels, already transform-mapped)
/// with half-width `r` and a 1-px antialiased edge.
#[inline]
fn draw_segment(buf: &mut [u8], width: u32, w: f32, h: f32, seg: Seg, r: f32, color: Color) {
    let dx = seg.bx - seg.ax;
    let dy = seg.by - seg.ay;
    let len2 = dx * dx + dy * dy;
    // Degenerate segment (two identical points) still draws a dot:
    // `point_segment_distance` clamps the projection to t=0 and measures
    // the distance to the single point.
    // Segment bounding box padded by the radius + 1-px AA fringe.
    let pad = r + 1.0;
    let x0 = seg.ax.min(seg.bx) - pad;
    let x1 = seg.ax.max(seg.bx) + pad;
    let y0 = seg.ay.min(seg.by) - pad;
    let y1 = seg.ay.max(seg.by) + pad;
    if x1 <= 0.0 || y1 <= 0.0 || x0 >= w || y0 >= h {
        return;
    }
    let x_start = x0.floor().max(0.0) as usize;
    let x_end = (x1.ceil().min(w)) as usize;
    let y_start = y0.floor().max(0.0) as usize;
    let y_end = (y1.ceil().min(h)) as usize;
    if y_end <= y_start || x_end <= x_start {
        return;
    }

    for y in y_start..y_end {
        let fy = y as f32 + 0.5;
        let row = y * (width as usize);
        for x in x_start..x_end {
            let px = x as f32 + 0.5;
            let d = point_segment_distance(px, fy, seg.ax, seg.ay, dx, dy, len2);
            // 1-px fringe: full coverage inside the core, linear falloff on
            // the outer pixel strip.
            let cov = ((r + 0.5 - d).clamp(0.0, 1.0)).min(1.0);
            if cov <= 0.0 {
                continue;
            }
            blend_pixel(buf, (row + x) * 4, color, cov);
        }
    }
}

/// Distance from pixel center `(px,py)` to the segment `a→b` (precomputed
/// `dx/dy` and squared length): the projection is clamped to the segment
/// extent, so points beyond the ends measure the distance to the nearer
/// endpoint. `sqrt` runs only when the squared distance is within the
/// (padded) bounding box already visited — cheap and exact enough for a
/// 1-px antialiased edge.
#[inline]
fn point_segment_distance(px: f32, py: f32, ax: f32, ay: f32, dx: f32, dy: f32, len2: f32) -> f32 {
    let t = if len2 > 0.0 {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cx = ax + t * dx;
    let cy = ay + t * dy;
    let d2 = (px - cx) * (px - cx) + (py - cy) * (py - cy);
    d2.sqrt()
}

/// Source-over blend of `color` (with coverage `cov` in [0,1]) at byte
/// offset `i`. No allocation, no branches beyond the alpha check.
#[inline]
fn blend_pixel(buf: &mut [u8], i: usize, color: Color, cov: f32) {
    // a: final alpha in [0,1] combining the annotation alpha and coverage.
    let a = color.a as f32 / 255.0 * cov;
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let (sr, sg, sb) = (color.r as f32, color.g as f32, color.b as f32);
    let (dr, dg, db) = (buf[i] as f32, buf[i + 1] as f32, buf[i + 2] as f32);
    buf[i] = (sr * a + dr * inv) as u8;
    buf[i + 1] = (sg * a + dg * inv) as u8;
    buf[i + 2] = (sb * a + db * inv) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{Annotation, AnnotationSet, Color, Highlight, Stroke, TextNote};

    const GRAY: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    fn buffer(w: u32, h: u32) -> Vec<u8> {
        // Opaque black background: alpha channel 255 (the page bitmap is
        // opaque RGB; the overlay must never touch the alpha byte).
        let mut v = vec![0u8; (w * h * 4) as usize];
        for px in v.chunks_exact_mut(4) {
            px[3] = 255;
        }
        v
    }

    fn pixel(buf: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let i = ((y * w + x) * 4) as usize;
        (buf[i], buf[i + 1], buf[i + 2], buf[i + 3])
    }

    #[test]
    fn highlight_fills_core_pixels_and_blends_alpha() {
        // Yellow marker, 50% alpha, over black background at identity zoom.
        let color = Color {
            r: 255,
            g: 240,
            b: 0,
            a: 128,
        };
        let mut set = AnnotationSet::new();
        set.add(
            0,
            Annotation::Highlight(Highlight {
                rects: vec![Rect::new(2.0, 3.0, 4.0, 2.0)],
                color,
            }),
        )
        .expect("add");
        let anns = set.for_page(0);
        let mut buf = buffer(16, 16);
        composite_annotations(&mut buf, 16, 16, &anns, &ViewTransform::IDENTITY);
        let (r, g, b, a) = pixel(&buf, 16, 4, 4);
        // Interior of the rect: coverage 1, alpha 128/255 ≈ 0.502 → dst
        // stays 0 (black), so out ≈ src*0.502.
        assert!(r > 110 && r < 145, "r={r}");
        assert!(g > 100 && g < 140, "g={g}");
        assert!(b < 20, "b={b}");
        assert_eq!(a, 255, "page bitmap alpha is untouched (opaque buffer)");

        // Outside the rect the buffer is untouched (still opaque black).
        assert_eq!(pixel(&buf, 16, 0, 0), (0, 0, 0, 255));
    }

    #[test]
    fn transform_maps_zoom_and_offset() {
        let mut set = AnnotationSet::new();
        set.add(
            0,
            Annotation::Highlight(Highlight {
                rects: vec![Rect::new(1.0, 1.0, 2.0, 2.0)],
                color: Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            }),
        )
        .expect("add");
        let anns = set.for_page(0);
        let xform = ViewTransform {
            zoom: 3.0,
            offset_x: 5.0,
            offset_y: 7.0,
        };
        let mut buf = buffer(20, 20);
        composite_annotations(&mut buf, 20, 20, &anns, &xform);
        // Page rect (1..3) × zoom 3 + offset → screen (8..14, 10..16).
        // Interior screen pixel (10, 12) must be painted red; one outside
        // (0,0) untouched. Screen edge (8,10) is exactly the rect corner.
        assert_eq!(pixel(&buf, 20, 10, 12), (255, 0, 0, 255));
        assert_eq!(pixel(&buf, 20, 0, 0), (0, 0, 0, 255));
        assert_eq!(pixel(&buf, 20, 8, 10), (255, 0, 0, 255));
    }

    #[test]
    fn stroke_draws_a_thick_line_with_aa_edge() {
        let mut set = AnnotationSet::new();
        // Horizontal stroke centered at page y=5.5 (between pixel rows, so
        // the 1-px AA fringe lands on real pixels), width 4 pt → radius 2 px
        // at zoom 1, painted red over the black page.
        set.add(
            0,
            Annotation::Stroke(
                Stroke::new(
                    vec![(1.0, 5.5), (11.0, 5.5)],
                    4.0,
                    Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                )
                .expect("stroke"),
            ),
        )
        .expect("add");
        let anns = set.for_page(0);
        let mut buf = buffer(20, 20);
        composite_annotations(&mut buf, 20, 20, &anns, &ViewTransform::IDENTITY);
        // Core: rows 4..6 fully inside the band (center 5.5 ± 2).
        for y in 4..7 {
            let (r, _, _, _) = pixel(&buf, 20, 5, y);
            assert_eq!(r, 255, "core row {y} fully covered → red");
        }
        // Antialiased fringe: row 3 is 2 px from the center = the band edge,
        // so the pixel (centered at 3.5) is half covered.
        let (r3, _, _, _) = pixel(&buf, 20, 5, 3);
        let (r0, _, _, _) = pixel(&buf, 20, 5, 0);
        assert!(r3 > 0 && r3 < 255, "fringe partially covered: r3={r3}");
        assert_eq!(r0, 0, "far row untouched: r0={r0}");
    }

    #[test]
    fn clamped_buffer_ignores_offscreen_geometry() {
        let mut set = AnnotationSet::new();
        // Rect fully offscreen left.
        set.add(
            0,
            Annotation::Highlight(Highlight {
                rects: vec![Rect::new(-10.0, -10.0, 5.0, 5.0)],
                color: GRAY,
            }),
        )
        .expect("add");
        set.add(
            0,
            Annotation::Stroke(
                Stroke::new(vec![(-5.0, 0.0), (-1.0, 0.0)], 2.0, GRAY).expect("stroke"),
            ),
        )
        .expect("add");
        let anns = set.for_page(0);
        let mut buf = buffer(8, 8);
        composite_annotations(&mut buf, 8, 8, &anns, &ViewTransform::IDENTITY);
        assert_eq!(&buf[..], buffer(8, 8).as_slice(), "nothing painted");
    }

    #[test]
    fn text_notes_do_not_paint() {
        let mut set = AnnotationSet::new();
        set.add(
            0,
            Annotation::TextNote(TextNote {
                anchor: (1.0, 1.0),
                text: "hola".to_string(),
            }),
        )
        .expect("add");
        set.add(
            0,
            Annotation::Highlight(Highlight {
                rects: vec![],
                color: GRAY,
            }),
        )
        .expect("add");
        let anns = set.for_page(0);
        let mut buf = buffer(8, 8);
        composite_annotations(&mut buf, 8, 8, &anns, &ViewTransform::IDENTITY);
        assert_eq!(&buf[..], buffer(8, 8).as_slice(), "notes paint nothing");
    }
}
