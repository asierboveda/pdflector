// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Escala inicial y encuadre visible de la página.
//!
//! Módulo separado para mantener la política de escala de apertura fuera del
//! camino de render de `Reader`.
//!
//! La escala inicial contiene la caja visible dentro del viewport, dejando
//! un margen fijo de interfaz y centrando la página en ambos ejes.

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
}
