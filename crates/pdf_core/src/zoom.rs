// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Zoom support (Fase 1 B3): the immediate software-scaled (blurry) path and
//! the continuous-zoom → ladder-level mapping that drives the crisp re-render.
//!
//! Flow: on a zoom change the UI shows `scale_bitmap` output instantly (cheap,
//! bilinear, slightly soft) and asks the cache/prefetch for a crisp render at
//! `scale_for_level(scale_level_for_zoom(zoom))`; the cache's
//! `trim_to_scale_level` frees the bytes of the previous level so the new one
//! never has to fight for budget.

use crate::engine::{Bitmap, Error, Result};

/// Software bilinear scaler for RGBA8 bitmaps, row-major, 4 bytes/pixel.
///
/// Maps each target pixel to its centre in source-pixel coordinates
/// (`(t + 0.5) * src / dst - 0.5`) and bilinearly interpolates the four
/// surrounding source texels, clamping to the edge so out-of-bounds reads are
/// impossible. Deterministic: pure IEEE-754 f32 arithmetic with
/// round-to-nearest at the end, no platform-dependent fast-math.
///
/// This is the "immediate" path shown while the crisp re-render at the new
/// ladder level runs in the background; it trades a little sharpness for speed
/// and is never the final image.
pub fn scale_bitmap(src: &Bitmap, target_width: u32, target_height: u32) -> Result<Bitmap> {
    if src.width == 0 || src.height == 0 {
        return Err(Error::InvalidArgument(
            "source bitmap has zero width or height".to_string(),
        ));
    }
    if src.data.len() != src.width as usize * src.height as usize * 4 {
        return Err(Error::InvalidArgument(format!(
            "source buffer is {} bytes, expected {}x{} RGBA8 = {} bytes",
            src.data.len(),
            src.width,
            src.height,
            src.width as usize * src.height as usize * 4
        )));
    }
    if target_width == 0 || target_height == 0 {
        return Err(Error::InvalidArgument(format!(
            "target size {target_width}x{target_height} must be non-zero"
        )));
    }

    let src_w = src.width as f32;
    let src_h = src.height as f32;
    let sx_scale = src_w / target_width as f32;
    let sy_scale = src_h / target_height as f32;
    let max_x = src.width - 1;
    let max_y = src.height - 1;
    let src_row = src.width as usize * 4;

    let mut out = vec![0u8; target_width as usize * target_height as usize * 4];

    for ty in 0..target_height {
        let sy = ((ty as f32 + 0.5) * sy_scale - 0.5).clamp(0.0, max_y as f32);
        let y0 = sy.floor() as u32;
        let y1 = (y0 + 1).min(max_y);
        let fy = sy - y0 as f32;
        let row0 = y0 as usize * src_row;
        let row1 = y1 as usize * src_row;
        let out_row = ty as usize * target_width as usize * 4;

        for tx in 0..target_width {
            let sx = ((tx as f32 + 0.5) * sx_scale - 0.5).clamp(0.0, max_x as f32);
            let x0 = sx.floor() as u32;
            let x1 = (x0 + 1).min(max_x);
            let fx = sx - x0 as f32;
            let x0b = x0 as usize * 4;
            let x1b = x1 as usize * 4;
            let o = out_row + tx as usize * 4;

            for c in 0..4 {
                let p00 = src.data[row0 + x0b + c] as f32;
                let p10 = src.data[row0 + x1b + c] as f32;
                let p01 = src.data[row1 + x0b + c] as f32;
                let p11 = src.data[row1 + x1b + c] as f32;
                let top = p00 + (p10 - p00) * fx;
                let bottom = p01 + (p11 - p01) * fx;
                let v = top + (bottom - top) * fy;
                out[o + c] = v.round() as u8;
            }
        }
    }

    Ok(Bitmap {
        width: target_width,
        height: target_height,
        data: out,
    })
}

/// Maps a continuous zoom factor to the nearest ladder level
/// (`scale_for_level(level) == 2^level`, see `cache::scale_for_level`).
///
/// Policy: `level = max(0, ceil(log2(zoom)))`. Ceiling guarantees the crisp
/// re-render is rendered at `scale >= zoom` — never an upscale of the blurry
/// buffer, which would compound the softness. The `max(0, ·)` clamps
/// out-zooming (zoom < 1) to the cheap 72 dpi baseline (level 0).
///
/// Invalid input (zoom <= 0, or NaN) is not a meaningful continuous zoom; it
/// is clamped to level 0 (baseline) rather than panicking — callers that must
/// distinguish can validate `zoom` beforehand.
pub fn scale_level_for_zoom(zoom: f32) -> u32 {
    // `zoom > 0.0` is false for NaN too, which is exactly the intent.
    if zoom <= 0.0 || zoom.is_nan() {
        return 0;
    }
    let level = zoom.log2().ceil();
    level.max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Bitmap {
        Bitmap {
            width: w,
            height: h,
            data: rgba.repeat(w as usize * h as usize),
        }
    }

    /// 4x4 RGBA checkerboard of 2x2 blocks: top-left/bottom-right white,
    /// top-right/bottom-left black. Downscaling 2x must land exactly on block
    /// centres, so each output texel equals its block colour exactly.
    fn checkerboard_4x4() -> Bitmap {
        let mut data = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let white = ((x / 2) + (y / 2)) % 2 == 0;
                let i = (y * 4 + x) * 4;
                data[i..i + 4].fill(if white { 255 } else { 0 });
            }
        }
        Bitmap {
            width: 4,
            height: 4,
            data,
        }
    }

    #[test]
    fn scale_2x_down_of_checkerboard_is_exact() {
        let out = scale_bitmap(&checkerboard_4x4(), 2, 2).expect("scale");
        assert_eq!((out.width, out.height), (2, 2));
        assert_eq!(out.data.len(), 2 * 2 * 4);
        let px = |x: usize, y: usize| out.data[(y * 2 + x) * 4];
        assert_eq!(px(0, 0), 255);
        assert_eq!(px(1, 0), 0);
        assert_eq!(px(0, 1), 0);
        assert_eq!(px(1, 1), 255);
    }

    #[test]
    fn scale_2x_down_of_horizontal_gradient_interpolates() {
        // Columns 0, 85, 170, 255; centres of the two output columns sample
        // exactly halfway between columns -> 42.5 and 212.5, rounded to 43/213.
        let src = Bitmap {
            width: 4,
            height: 1,
            data: [0u8, 85, 170, 255]
                .into_iter()
                .flat_map(|c| [c; 4])
                .collect(),
        };
        let out = scale_bitmap(&src, 2, 1).expect("scale");
        assert_eq!(out.width, 2);
        assert_eq!(out.data[0], 43);
        assert_eq!(out.data[4], 213);
    }

    #[test]
    fn scale_identity_is_exact() {
        // 1:1 scale must reproduce the source exactly (no sub-pixel shift).
        let src = checkerboard_4x4();
        let out = scale_bitmap(&src, 4, 4).expect("scale");
        assert_eq!(out.data, src.data);
    }

    #[test]
    fn scale_odd_sizes_no_panic() {
        let mut src = solid(5, 3, [7, 8, 9, 10]);
        for (i, b) in src.data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(3);
        }
        for (w, h) in [(7u32, 11u32), (11, 7), (1, 1), (13, 2)] {
            let out = scale_bitmap(&src, w, h).expect("scale");
            assert_eq!((out.width, out.height), (w, h));
            assert_eq!(out.data.len(), w as usize * h as usize * 4);
        }
    }

    #[test]
    fn scale_is_deterministic() {
        let src = checkerboard_4x4();
        let a = scale_bitmap(&src, 7, 5).expect("scale");
        let b = scale_bitmap(&src, 7, 5).expect("scale");
        assert_eq!(a.data, b.data);
        assert_eq!((a.width, a.height), (b.width, b.height));
    }

    #[test]
    fn scale_uniform_source_stays_uniform() {
        let src = solid(3, 2, [10, 20, 30, 40]);
        let out = scale_bitmap(&src, 9, 7).expect("scale");
        assert!(out.data.chunks_exact(4).all(|p| p == [10, 20, 30, 40]));
    }

    #[test]
    fn scale_rejects_invalid_input() {
        assert!(scale_bitmap(&solid(0, 2, [0; 4]), 2, 2).is_err());
        assert!(scale_bitmap(&solid(2, 0, [0; 4]), 2, 2).is_err());
        // Truncated buffer: invariant `data.len() == w*h*4` violated.
        let truncated = Bitmap {
            width: 2,
            height: 2,
            data: vec![0; 4], // 4 bytes instead of 16
        };
        assert!(scale_bitmap(&truncated, 2, 2).is_err());
        assert!(scale_bitmap(&solid(2, 2, [0; 4]), 0, 2).is_err());
        assert!(scale_bitmap(&solid(2, 2, [0; 4]), 2, 0).is_err());
    }

    #[test]
    fn zoom_level_maps_powers_of_two_exactly() {
        assert_eq!(scale_level_for_zoom(1.0), 0);
        assert_eq!(scale_level_for_zoom(2.0), 1);
        assert_eq!(scale_level_for_zoom(4.0), 2);
        assert_eq!(scale_level_for_zoom(8.0), 3);
    }

    #[test]
    fn zoom_level_rounds_up_so_rerender_never_upscales() {
        assert_eq!(scale_level_for_zoom(1.1), 1);
        assert_eq!(scale_level_for_zoom(1.5), 1);
        assert_eq!(scale_level_for_zoom(2.1), 2);
        assert_eq!(scale_level_for_zoom(3.0), 2);
        assert_eq!(scale_level_for_zoom(7.9), 3);
    }

    #[test]
    fn zoom_level_clamps_out_zooming_to_baseline() {
        assert_eq!(scale_level_for_zoom(0.99), 0);
        assert_eq!(scale_level_for_zoom(0.5), 0);
        assert_eq!(scale_level_for_zoom(0.25), 0);
    }

    #[test]
    fn zoom_level_clamps_invalid_input_to_baseline() {
        assert_eq!(scale_level_for_zoom(0.0), 0);
        assert_eq!(scale_level_for_zoom(-1.0), 0);
        assert_eq!(scale_level_for_zoom(f32::NAN), 0);
    }

    #[test]
    fn zoom_level_scale_never_below_zoom() {
        // The core promise: `scale_for_level(level) >= zoom` for every zoom >= 1.
        let mut z = 1.0f32;
        while z <= 64.0 {
            let level = scale_level_for_zoom(z);
            assert!(
                crate::scale_for_level(level) >= z,
                "zoom {z} -> level {level} renders at {}",
                crate::scale_for_level(level)
            );
            z += 0.01;
        }
    }
}
