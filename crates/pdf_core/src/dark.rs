//! Dark mode support (PLAN Fase 2): page negation for a black background with
//! light text.
//!
//! The dark-mode bitmap is derived from the cached page bitmap, so it never
//! touches the render cache nor the engine: `invert_bitmap` is a pure
//! per-pixel transform over the RGBA8 buffer.

use crate::engine::Bitmap;

/// Inverts the RGB channels of a page bitmap (`255 - v`), leaving the alpha
/// channel intact. Used for dark mode: a white page (255,255,255,255) becomes
/// black with opaque alpha, and dark text becomes light.
///
/// Pure and deterministic: same dimensions, same row-major RGBA8 layout as
/// `src`; the buffer is copied, the source is untouched. Alpha is preserved
/// byte-for-byte because the alpha channel is not part of the colour negation
/// (a fully opaque page stays fully opaque).
pub fn invert_bitmap(src: &Bitmap) -> Bitmap {
    let mut data = src.data.clone();
    for px in data.chunks_exact_mut(4) {
        px[0] = 255 - px[0];
        px[1] = 255 - px[1];
        px[2] = 255 - px[2];
        // px[3] (alpha) intentionally untouched.
    }
    Bitmap {
        width: src.width,
        height: src.height,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(w: u32, h: u32, data: Vec<u8>) -> Bitmap {
        assert_eq!(data.len(), w as usize * h as usize * 4);
        Bitmap {
            width: w,
            height: h,
            data,
        }
    }

    /// Fully opaque black page inverts to fully opaque white.
    #[test]
    fn black_becomes_white() {
        // Black, fully opaque (RGB 0, alpha 255).
        let mut data = vec![0u8; 3 * 2 * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        let src = rgba(3, 2, data);
        let out = invert_bitmap(&src);
        assert_eq!((out.width, out.height), (3, 2));
        assert!(
            out.data
                .chunks_exact(4)
                .all(|px| px == [255, 255, 255, 255])
        );
    }

    /// Fully opaque white page inverts to fully opaque black.
    #[test]
    fn white_becomes_black() {
        let src = rgba(2, 2, vec![255u8; 2 * 2 * 4]);
        let out = invert_bitmap(&src);
        assert!(out.data.chunks_exact(4).all(|px| px[..3] == [0, 0, 0]));
        assert!(out.data.chunks_exact(4).all(|px| px[3] == 255));
    }

    /// Alpha must pass through untouched, whatever its value.
    #[test]
    fn alpha_is_preserved() {
        let alphas = [0u8, 1, 127, 254, 255];
        let mut data = Vec::new();
        for a in alphas {
            data.extend_from_slice(&[10, 20, 30, a]);
        }
        let src = rgba(5, 1, data);
        let out = invert_bitmap(&src);
        for (i, a) in alphas.iter().enumerate() {
            let px = &out.data[i * 4..i * 4 + 4];
            assert_eq!([px[0], px[1], px[2]], [245, 235, 225], "RGB inverted");
            assert_eq!(px[3], *a, "alpha {a} must be untouched");
        }
    }

    /// Inverting twice must reproduce the source exactly (and the source
    /// buffer must be left untouched).
    #[test]
    fn invert_is_idempotent() {
        let data: Vec<u8> = (0..7 * 5 * 4)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(7))
            .collect();
        let src = rgba(7, 5, data.clone());
        let roundtrip = invert_bitmap(&invert_bitmap(&src));
        assert_eq!(roundtrip.data, data);
        assert_eq!(src.data, data, "source must not be mutated");
    }
}
