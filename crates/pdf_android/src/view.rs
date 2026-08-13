// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Escala inicial de apertura de documento: política "cover" (pantalla completa).
//!
//! Módulo separado para que otro agente pueda cambiar la política de escala de
//! apertura SIN tocar `Reader::render_current_page`. La partición de `lib.rs`
//! (2026-08-13) aísla aquí la única decisión de escala que hoy vive inline en
//! el render.
//!
//! ## Cover vs contain (por qué `max` y no `min`)
//!
//! Al abrir un documento queremos aprovechar TODO el espacio de la tablet:
//! con *contain* (`min`) la página cabe entera pero, si su proporción no
//! coincide con la de la ventana, quedan barras (letterbox) a los lados o
//! arriba/abajo. Con *cover* (`max`) la página LLENA ancho y alto y el exceso
//! se recorta por los bordes — en la práctica, los márgenes de la página.
//! Es un recorte geométrico por proporción; NO analiza píxeles (para recortar
//! los márgenes blancos reales está `crop_margins`, bonus aparte).

use pdf_core::Bitmap;

/// Render scale for opening a document so the page fills the screen.
/// Política "cover": la página llena la ventana en ancho Y alto; el sobrante
/// (márgenes) se recorta. Sin letterbox.
///
/// Fórmula: `scale = max(win_w / page_w, win_h / page_h)` con `win_*` como f32
/// (el doble cast de `i32` a `f32` es intencional: sin él, la división entera
/// truncaría). El zoom continuo se multiplica DESPUÉS en el caller.
///
/// Clamp de seguridad: si `page_w` o `page_h` no son finitos positivos (p. ej.
/// página corrupta con tamaño 0, que daría división por cero → +∞, o NaN) el
/// cociente deja de ser un número finito > 0 y se devuelve 1.0 (escala neutra,
/// 1 px = 1 pt, 72 dpi) en vez de propagar ∞/NaN al render.
pub fn initial_scale(page_w: f32, page_h: f32, win_w: i32, win_h: i32) -> f32 {
    // Cover: llenar la pantalla recortando el exceso. `max` en vez del `min`
    // de contain. El chequeo del resultado cubre la división por cero
    // (`page_w == 0.0` → +∞) y los tamaños NaN/negativos: no hace falta
    // comprobar `page_w`/`page_h` por separado.
    let cover = (win_w as f32 / page_w).max(win_h as f32 / page_h);
    if cover.is_finite() && cover > 0.0 {
        cover
    } else {
        1.0
    }
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
/// Coste O(ancho × alto), una sola pasada por píxel. Pensada para llamarse
/// UNA vez al abrir/cambiar de página y cachear el rect — NUNCA en el camino
/// de render o scroll (60 fps, presupuesto < 16,6 ms/frame). La política
/// "cover" de `initial_scale` ya recorta por los bordes geométricamente; este
/// rect permite recortar además los MÁRGENES BLANCOS reales (páginas
/// escaneadas con margen ancho de editorial) cuando el caller lo integre en
/// Fase 2/6.
///
/// `dead_code` intencional (2026-08-13): API pública sin caller todavía — la
/// pide la tarea "pantalla completa al abrir" para que otro agente/el
/// coordinador la use luego. En un cdylib rustc la marca dead_code aunque sea
/// `pub`; se suprime la advertencia para mantener el build sin warnings, y se
/// elimina el `#[allow]` cuando el primer caller la consuma.
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
        for (x, px) in row.chunks_exact(4).enumerate() {
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

    /// Cobertura: sin letterbox — la escala es `max`, no `min`.
    #[test]
    fn initial_scale_is_cover_not_contain() {
        // Página A4 en vertical (595×842) en ventana apaisada 1280×800:
        // contain daría min(2.15, 0.95) = 0.95 (barras arriba/abajo);
        // cover debe dar max(2.15, 0.95) = 2.15 (recorta los lados).
        let scale = initial_scale(595.0, 842.0, 1280, 800);
        let contain = (1280.0_f32 / 595.0).min(800.0_f32 / 842.0);
        let cover = (1280.0_f32 / 595.0).max(800.0_f32 / 842.0);
        assert_eq!(scale, cover);
        assert!(scale > contain);
    }

    /// La escala llena ancho Y alto (según la dimensión limitante).
    #[test]
    fn initial_scale_fills_both_dimensions() {
        let scale = initial_scale(595.0, 842.0, 1280, 800);
        assert!(scale * 595.0 >= 1280.0 - 0.5); // ancho cubierto
        assert!(scale * 842.0 >= 800.0 - 0.5); // alto cubierto
    }

    /// División por cero / NaN / negativos → fallback 1.0, nunca ∞/NaN.
    #[test]
    fn initial_scale_clamps_bad_page_sizes() {
        assert_eq!(initial_scale(0.0, 842.0, 1280, 800), 1.0);
        assert_eq!(initial_scale(595.0, 0.0, 1280, 800), 1.0);
        assert_eq!(initial_scale(f32::NAN, 842.0, 1280, 800), 1.0);
        assert_eq!(initial_scale(-595.0, 842.0, 1280, 800), 1.0);
        assert_eq!(initial_scale(f32::INFINITY, 842.0, 1280, 800), 1.0);
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
