// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Escala inicial y encuadre visible de la página.
//!
//! Módulo separado para mantener la política de escala de apertura fuera del
//! camino de render de `Reader`.
//!
//! La escala inicial contiene la caja visible dentro del viewport, dejando
//! un margen fijo de interfaz y centrando la página en ambos ejes.

use pdf_core::Bitmap;

/// Convert the fixed 12 dp interface margin to the window's physical pixels.
pub(crate) fn viewport_margin_px(density_dpi: u32) -> f32 {
    12.0 * density_dpi.max(1) as f32 / 160.0
}

/// Render scale for opening a document with its complete visible box in view.
pub(crate) fn initial_scale_with_margin(
    page_w: f32,
    page_h: f32,
    win_w: i32,
    win_h: i32,
    margin_px: f32,
) -> f32 {
    let margin = if margin_px.is_finite() {
        margin_px.max(0.0)
    } else {
        0.0
    };
    let available_w = (win_w as f32 - 2.0 * margin).max(1.0);
    let available_h = (win_h as f32 - 2.0 * margin).max(1.0);
    let contain = (available_w / page_w).min(available_h / page_h);
    if page_w.is_finite()
        && page_w > 0.0
        && page_h.is_finite()
        && page_h > 0.0
        && contain.is_finite()
        && contain > 0.0
    {
        contain
    } else {
        1.0
    }
}

/// Center the complete visible page in the window at its opening scale.
pub(crate) fn page_origin(
    page_w: f32,
    page_h: f32,
    scale: f32,
    win_w: i32,
    win_h: i32,
    _margin_px: f32,
) -> (f32, f32) {
    (
        (win_w as f32 - page_w * scale) / 2.0,
        (win_h as f32 - page_h * scale) / 2.0,
    )
}

/// Umbral de "blanco de papel": un píxel se considera margen si sus canales
/// R, G y B están TODOS por encima de este valor. El bitmap de página es
/// RGBA8 opaco (fondo 255,255,255, alfa ignorado); el contenido (texto,
/// trazos, imágenes) baja algún canal por debajo.
///
/// `dead_code` intencional: solo lo usa `crop_margins`, que hoy no tiene
/// caller (ver su `#[allow]`); se suprime aquí la advertencia de la constante
/// junto a la de la función para que el build del cdylib salga limpio.
#[allow(dead_code)]
const WHITE_THRESHOLD: u8 = 245;

/// Bounding box del contenido no-blanco de un `Bitmap` de página.
///
/// Devuelve `Some((left, top, right, bottom))` con el rectángulo mínimo que
/// contiene todo píxel no-blanco, con `right`/`bottom` EXCLUSIVOS
/// (ancho = `right - left`, alto = `bottom - top`); `None` si la página está
/// completamente en blanco o el bitmap está vacío/corrupto.
///
/// Coste O(ancho × alto), una pasada por píxel. No tiene llamador en la app:
/// el encuadre de página usa la caja visible del documento y no recorta por
/// contenido de píxeles.
///
/// `dead_code` intencional: API pública usada por tests unitarios (sin caller
/// todavía en la app). En un cdylib rustc la marca dead_code aunque sea `pub`;
/// se suprime la advertencia para mantener el build sin warnings.
///
/// Auditoría (fix C de speed, 2026-09-07): NINGÚN llamador consume bitmaps de
/// la `PageCache` por aquí (sus únicos usos son los tests de abajo con bitmaps
/// sintéticos). Si un futuro caller le pasara un bitmap CACHEADO (el crop
/// centrado a ventana de `CachedPage`), el bbox de márgenes saldría relativo
/// al CROP (la ventana), no a la página completa: habría que componer con
/// `full_w/full_h/crop_x/crop_y` o medir sobre un render full.
#[allow(dead_code)]
pub fn crop_margins(bitmap: &Bitmap) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = (bitmap.width as usize, bitmap.height as usize);
    // Invariante documentado en pdf_core (`data.len() == width * height * 4`);
    // el guard evita un panic por slice fuera de rango ante un bitmap corrupto.
    if w == 0 || h == 0 || bitmap.data.len() != w * h * 4 {
        return None;
    }

    let mut left = w;
    let mut top = h;
    let mut right = 0usize;
    let mut bottom = 0usize;

    for y in 0..h {
        let row = &bitmap.data[y * w * 4..(y + 1) * w * 4];
        for (x, px) in row.as_chunks::<4>().0.iter().enumerate() {
            // RGBA: los tres primeros canales son R, G, B (alfa se ignora).
            let is_white =
                px[0] >= WHITE_THRESHOLD && px[1] >= WHITE_THRESHOLD && px[2] >= WHITE_THRESHOLD;
            if !is_white {
                if x < left {
                    left = x;
                }
                right = right.max(x + 1);
                if y < top {
                    top = y;
                }
                bottom = y + 1;
            }
        }
    }

    // Sin ningún píxel no-blanco (right/bottom quedaron en 0): página en blanco.
    if right == 0 || bottom == 0 {
        None
    } else {
        Some((left as u32, top as u32, right as u32, bottom as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La página cabe dentro del viewport con el margen y centrado definidos.
    #[test]
    fn initial_scale_contains_page_inside_margin() {
        let scale = initial_scale_with_margin(595.0, 842.0, 1280, 800, 24.0);
        let expected = ((1280.0_f32 - 48.0) / 595.0).min((800.0_f32 - 48.0) / 842.0);
        assert_eq!(scale, expected);
        assert!(scale * 595.0 <= 1280.0 - 48.0);
        assert!(scale * 842.0 <= 800.0 - 48.0);
    }

    /// El origen de contain deja el mismo margen a ambos lados del eje.
    #[test]
    fn initial_origin_centers_page_inside_margin() {
        let scale = initial_scale_with_margin(1440.0, 810.0, 1440, 2200, 24.0);
        let (x, y) = page_origin(1440.0, 810.0, scale, 1440, 2200, 24.0);
        let right = 1440.0 - (x + 1440.0 * scale);
        let bottom = 2200.0 - (y + 810.0 * scale);
        assert!((x - right).abs() <= 0.01);
        assert!((y - bottom).abs() <= 0.01);
        assert!(x >= 24.0 && y >= 24.0);
    }

    /// División por cero / NaN / negativos → fallback 1.0, nunca ∞/NaN.
    #[test]
    fn initial_scale_clamps_bad_page_sizes() {
        assert_eq!(initial_scale_with_margin(0.0, 842.0, 1280, 800, 24.0), 1.0);
        assert_eq!(initial_scale_with_margin(595.0, 0.0, 1280, 800, 24.0), 1.0);
        assert_eq!(
            initial_scale_with_margin(f32::NAN, 842.0, 1280, 800, 24.0),
            1.0
        );
        assert_eq!(
            initial_scale_with_margin(-595.0, 842.0, 1280, 800, 24.0),
            1.0
        );
        assert_eq!(
            initial_scale_with_margin(f32::INFINITY, 842.0, 1280, 800, 24.0),
            1.0
        );
    }

    #[test]
    fn viewport_margin_converts_dp_using_display_density() {
        assert_eq!(viewport_margin_px(160), 12.0);
        assert_eq!(viewport_margin_px(320), 24.0);
    }

    /// crop_margins: recorta márgenes blancos en un bitmap sintético.
    #[test]
    fn crop_margins_finds_content_bbox() {
        // 10×8 con un rectángulo de contenido no-blanco en x∈[3,7), y∈[2,6).
        let mut data = vec![255u8; 10 * 8 * 4];
        for y in 2..6 {
            for x in 3..7 {
                let i = (y * 10 + x) * 4;
                data[i] = 0;
                data[i + 1] = 0;
                data[i + 2] = 0;
            }
        }
        let bmp = Bitmap {
            width: 10,
            height: 8,
            data,
        };
        assert_eq!(crop_margins(&bmp), Some((3, 2, 7, 6)));
    }

    /// crop_margins: página en blanco → None.
    #[test]
    fn crop_margins_blank_page_returns_none() {
        let bmp = Bitmap {
            width: 4,
            height: 4,
            data: vec![255u8; 4 * 4 * 4],
        };
        assert_eq!(crop_margins(&bmp), None);
    }

    /// crop_margins: bitmap vacío/corrupto → None sin panic.
    #[test]
    fn crop_margins_corrupt_bitmap_returns_none() {
        let bmp = Bitmap {
            width: 4,
            height: 4,
            data: vec![0u8; 3],
        };
        assert_eq!(crop_margins(&bmp), None);
        let bmp = Bitmap {
            width: 0,
            height: 4,
            data: vec![],
        };
        assert_eq!(crop_margins(&bmp), None);
    }
}
