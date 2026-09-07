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

/// Recorte CENTRADO de un bitmap RGBA8 (row-major, 4 B/px) al tamaño máximo
/// `max_w × max_h` (píxeles): devuelve `(bitmap, (crop_x, crop_y))` con el
/// bitmap recortado y el ORIGEN del recorte en píxeles del render original.
///
/// - Dims resultantes: `min(src.width, max_w) × min(src.height, max_h)` — si
///   el bitmap ya cabe en el límite es un NO-OP (misma imagen, origen (0,0);
///   el bitmap se clona porque la API toma `&Bitmap`).
/// - Centrado exacto: `crop_x = (src.width − w) / 2` (división entera — con
///   exceso impar el píxel extra sobrante queda en el lado final; el origen
///   devuelto es SIEMPRE el offset real de la primera columna/fila copiada).
/// - Píxeles: copia literal (sin interpolación ni filtro) — el resultado es
///   EXACTAMENTE la región `[crop_y, crop_y + h) × [crop_x, crop_x + w)` del
///   source, de modo que `crop.pixel(x, y) == src.pixel(x + crop_x, y + crop_y)`.
///
/// Precondición documentada (invariante de pdf_core): `data.len() == width *
/// height * 4` — todo bitmap que entra viene de `render_page`/`scale_bitmap`,
/// que lo garantizan; el recorte no añade un canal de error (contrato simple
/// del worker, que solo recorta renders OK).
///
/// Uso (fix de residency del visor): el worker renderiza la página a cover y
/// recorta aquí al tamaño de VENTANA (`(min(bw, win_w), min(bh, win_h))`); los
/// consumidores que convierten pantalla → píxeles del render (transición
/// fast→sharp del pinch, selección → imagen) compensan `(crop_x, crop_y)`
/// para volver a la cuadrícula del render FULL — ver `CachedPage` en
/// pdf_android (`cache.rs`).
pub fn crop_centered(bmp: &Bitmap, max_w: u32, max_h: u32) -> (Bitmap, (u32, u32)) {
    let w = bmp.width.min(max_w);
    let h = bmp.height.min(max_h);
    let crop_x = (bmp.width - w) / 2;
    let crop_y = (bmp.height - h) / 2;
    if w == bmp.width && h == bmp.height {
        return (bmp.clone(), (0, 0)); // ya cabe: no-op (origen 0)
    }
    let src_row = bmp.width as usize * 4;
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for row in crop_y..crop_y + h {
        let start = row as usize * src_row + crop_x as usize * 4;
        out.extend_from_slice(&bmp.data[start..start + w as usize * 4]);
    }
    (
        Bitmap {
            width: w,
            height: h,
            data: out,
        },
        (crop_x, crop_y),
    )
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
        assert!(
            out.data
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [10, 20, 30, 40])
        );
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

    /// Bitmap w×h con un RGBA único por píxel — `(x, y, x^y, 255)` — de modo
    /// que cada píxel del source es identificable y un crop solo puede acertar
    /// si copia EXACTAMENTE la región correcta (pixel-exactness).
    fn pixel_indexed(w: u32, h: u32) -> Bitmap {
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[
                    (x & 0xff) as u8,
                    (y & 0xff) as u8,
                    ((x ^ y) & 0xff) as u8,
                    255,
                ]);
            }
        }
        Bitmap {
            width: w,
            height: h,
            data,
        }
    }

    /// crop_centered: exceso PAR en ambos ejes → origen exactamente centrado
    /// y píxeles idénticos a la región origen del bitmap original.
    #[test]
    fn crop_centered_even_excess_is_centered_with_exact_pixels() {
        let src = pixel_indexed(10, 8); // excesos 10−4=6 y 8−4=4 (pares)
        let (crop, origin) = crop_centered(&src, 4, 4);
        assert_eq!(origin, (3, 2));
        assert_eq!((crop.width, crop.height), (4, 4));
        for y in 0..4 {
            for x in 0..4 {
                let c = ((y * 4 + x) * 4) as usize;
                let s = (((y + 2) * 10 + (x + 3)) * 4) as usize;
                assert_eq!(&crop.data[c..c + 4], &src.data[s..s + 4]);
            }
        }
    }

    /// crop_centered: exceso IMPAR (9−4=5, 7−3=4) → el origen redondea a la
    /// baja ((diff/2) entero) y los píxeles siguen siendo exactos.
    #[test]
    fn crop_centered_odd_excess_rounds_origin_down() {
        let src = pixel_indexed(9, 7);
        let (crop, origin) = crop_centered(&src, 4, 3);
        assert_eq!(origin, (2, 2)); // 5/2 y 4/2 truncados
        assert_eq!((crop.width, crop.height), (4, 3));
        for y in 0..3 {
            for x in 0..4 {
                let c = ((y * 4 + x) * 4) as usize;
                let s = (((y + 2) * 9 + (x + 2)) * 4) as usize;
                assert_eq!(&crop.data[c..c + 4], &src.data[s..s + 4]);
            }
        }
    }

    /// crop_centered: un solo eje desborda (el otro cabe) → el crop es el
    /// bitmap completo en el eje que cabe (origen 0) y centrado en el otro.
    #[test]
    fn crop_centered_clips_only_the_overflowing_axis() {
        let src = pixel_indexed(7, 5);
        // Solo X desborda (7 > 5): origen X = (7−5)/2 = 1, Y sin recorte.
        let (crop, origin) = crop_centered(&src, 5, 5);
        assert_eq!(origin, (1, 0));
        assert_eq!((crop.width, crop.height), (5, 5));
        // Solo Y desborda (5 > 3): origen Y = (5−3)/2 = 1, X sin recorte.
        let (crop, origin) = crop_centered(&src, 7, 3);
        assert_eq!(origin, (0, 1));
        assert_eq!((crop.width, crop.height), (7, 3));
    }

    /// crop_centered: el bitmap ya cabe → no-op (mismos píxeles, origen 0),
    /// también cuando el límite pedido es MAYOR que el bitmap.
    #[test]
    fn crop_centered_noop_when_it_fits() {
        let src = pixel_indexed(6, 4);
        let (crop, origin) = crop_centered(&src, 6, 4);
        assert_eq!(origin, (0, 0));
        assert_eq!((crop.width, crop.height), (6, 4));
        assert_eq!(crop.data, src.data);
        let (crop, origin) = crop_centered(&src, 10, 9); // límite mayor
        assert_eq!(origin, (0, 0));
        assert_eq!((crop.width, crop.height), (6, 4));
        assert_eq!(crop.data, src.data);
    }

    /// crop_centered: origen EXACTO en píxeles del render original (contrato
    /// del worker: los consumidores convierten pantalla→bitmap compensando
    /// crop_x/crop_y para llegar a los mismos píxeles del render full).
    #[test]
    fn crop_centered_origin_matches_source_pixel_grid() {
        let src = pixel_indexed(11, 9);
        let (crop, (ox, oy)) = crop_centered(&src, 5, 5);
        assert_eq!((ox, oy), (3, 2));
        // El píxel del crop en (0,0) es el píxel del source en el origen.
        assert_eq!(
            &crop.data[0..4],
            &src.data[((oy * 11 + ox) * 4) as usize..((oy * 11 + ox) * 4) as usize + 4]
        );
        // Y el ÚLTIMO píxel del crop es el origen + dims − 1 del source.
        let c = ((4 * 5 + 4) * 4) as usize;
        let s = (((oy + 4) * 11 + (ox + 4)) * 4) as usize;
        assert_eq!(&crop.data[c..c + 4], &src.data[s..s + 4]);
    }
}
