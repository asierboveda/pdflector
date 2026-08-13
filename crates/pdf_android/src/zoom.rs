//! Blit rápido de la página: zoom por vecino-más-cercano en el propio blit,
//! SIN re-render de MuPDF durante el pinch.
//!
//! Módulo separado para que el "fast zoom" (escalar/recortar el bitmap ya
//! renderizado, pinch instantáneo) no toque `Reader::blit`. El render de
//! MuPDF cuesta ~20 ms y los Move del pinch llegan a 60-120 Hz; re-renderizar
//! en cada evento encola lag. La solución: `render_current_page` renderiza a
//! `cover × zoom` (nítido a la escala pedida, re-render único al soltar el
//! pinch) y `blit_fast` escala ese bitmap por un zoom RELATIVO al render
//! (`zoom / rendered_zoom`): durante el pinch (sin re-render) el factor es
//! mayor que 1 y escala el bitmap viejo por vecino-más-cercano en memoria
//! en unos pocos ms (coste ~memcpy de una pantalla); al soltar, tras el
//! re-render, el factor es 1.0 y el blit es 1:1 nítido, sin doble zoom.

use android_activity::ndk::native_window::NativeWindow;
use log::warn;
use pdf_core::Bitmap;

use crate::draw::{copy_region, fill_buffer};

/// Blit de fondo + página escalada + overlay con UN solo lock+present.
///
/// Contrato con `Reader::blit` / `render_current_page` (para el pinch
/// instantáneo):
/// - `bitmap` es el render de la página a escala `cover × rendered_zoom`
///   (nítido a esa escala; `rendered_zoom` = zoom del último render, 1.0 al
///   abrir o cambiar de página). El re-render solo ocurre al soltar el pinch.
/// - `zoom` es un multiplicador RELATIVO al render:
///   `tamaño destino = bitmap × zoom` (el caller pasa `zoom / rendered_zoom`;
///   1.0 = blit 1:1 del bitmap recién renderizado, sin doble zoom).
/// - `offset` es la esquina superior izquierda del bitmap YA escalado en
///   coordenadas de ventana; el caller ya encapsula el centrado del render
///   (`offset_x/y` de `render_current_page`) y el pan.
///
/// Mapeo origen→destino por vecino-más-cercano con aritmética entera:
/// `src = (dst_rel × src_dim) / dst_dim` (sin f32 ni divisiones por píxel).
/// A zoom == 1.0 el mapeo es la identidad: bit a bit, el mismo resultado que
/// el blit 1:1 previo a la partición.
///
/// Nota de firma (desviación deliberada del template `(window, bitmap, zoom)`):
/// el blit previo a la partición dibujaba fondo + página + overlay del botón
/// "Open" en el MISMO buffer con UN solo lock+present; dividirlo en dos locks
/// presentaría dos buffers por frame y el compositor mostraría el frame
/// anterior (contenido stale bajo el botón). Por eso el stub recibe además el
/// fondo (`bg`), el desplazamiento (`offset` = offset_x + pan_x/y) y el overlay
/// opcional del visor. Así el blit rápido reproduce el blit actual píxel a
/// píxel con un único present, con todos los inputs para el escalado.
pub fn blit_fast(
    window: &NativeWindow,
    bitmap: &Bitmap,
    zoom: f32,
    bg: [u8; 4],
    offset: (i32, i32),
    overlay: Option<&Bitmap>,
) {
    // El guard se cae al final del scope: ANativeWindow_unlockAndPost.
    let Ok(mut guard) = window.lock(None) else {
        warn!("ANativeWindow_lock failed");
        return;
    };
    let bpp = match guard.format().bytes_per_pixel() {
        Some(b) => b,
        None => {
            warn!(
                "buffer format without bytes_per_pixel: {:?}",
                guard.format()
            );
            return;
        }
    };
    let dst_w = guard.width();
    let dst_h = guard.height();
    let dst_stride = guard.stride(); // en píxeles
    let dst = guard.bits() as *mut u8;

    // Fondo (letterbox; rojo si el PDF no se abrió — lo decide el caller).
    fill_buffer(dst, dst_w, dst_h, dst_stride, bpp, bg);

    // Página escalada al zoom pedido (vecino-más-cercano, sin re-render).
    // `offset` = esquina superior izquierda del bitmap ya escalado; el blit
    // recorta los bordes que quedan fuera del buffer (zoom > 1 o pan).
    let (dx, dy) = offset;
    blit_scaled_nearest(dst, dst_w, dst_h, dst_stride, bpp, bitmap, dx, dy, zoom);

    // Overlay del visor (botón "Open"), esquina superior izquierda.
    if let Some(ov) = overlay {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, 0, 0);
    }
}

/// Escala `src` (RGBA8) por vecino-más-cercano a tamaño `src × zoom` y copia
/// el resultado al buffer con su esquina superior izquierda en `(dx, dy)` px.
/// Recorta los bordes fuera de la ventana (zoom > 1, pan). Formato del buffer:
/// `bpp` bytes/px (4 = copia directa, 2 = conversión a RGB565, resto =
/// primeros `bpp` bytes de cada píxel, como `copy_region`).
///
/// Fórmula: `dest_w = src.width × zoom` (`zoom` es el factor RELATIVO al
/// render; `src.width` ya incluye el `rendered_zoom` del caller), y el
/// mapeo origen→destino por vecino-más-cercano
/// con aritmética entera `src = (dst_rel × src_dim) / dst_dim`, con
/// `dst_rel ∈ [0, dest_dim)`. La tabla de mapeo x se precalcula una vez por
/// blit (una división por columna), así el bucle interior no tiene f32 ni
/// divisiones: coste ~memcpy de una pantalla (unos pocos ms en la tablet).
//
// 9 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `draw::copy_region`; se acepta el allow en vez de empaquetarlos
// en una struct que solo se usaría aquí.
#[allow(clippy::too_many_arguments)]
fn blit_scaled_nearest(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    src: &Bitmap,
    dx: i32,
    dy: i32,
    zoom: f32,
) {
    let src_w = src.width as i64;
    let src_h = src.height as i64;
    // Guardas: zoom inválido o destino degenerado → solo fondo (ya pintado
    // por el caller; el overlay lo pinta `blit_fast` después).
    if src_w <= 0 || src_h <= 0 || !zoom.is_finite() || zoom <= 0.0 {
        return;
    }
    // Tamaño destino = bitmap × zoom (redondeo al píxel más cercano).
    let dw = (src_w as f64 * zoom as f64).round() as i64;
    let dh = (src_h as f64 * zoom as f64).round() as i64;
    if dw <= 0 || dh <= 0 {
        return;
    }
    // Recorte a la ventana (mismo criterio que `copy_region`).
    let x0 = i64::from(dx.max(0));
    let y0 = i64::from(dy.max(0));
    let x1 = (i64::from(dx) + dw).min(dst_w as i64);
    let y1 = (i64::from(dy) + dh).min(dst_h as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let vis_w = (x1 - x0) as usize;
    let src_row_bytes = src_w as usize * 4;
    // Mapeo x precalculado (vecino-más-cercano entero): `src_x = dst_rel ×
    // src_w / dw` con `dst_rel = x - dx ∈ [0, dw)`, lo que garantiza
    // `src_x ∈ [0, src_w)`. Una división por columna en vez de por píxel.
    let x_map: Vec<usize> = (0..vis_w)
        .map(|x| (((x0 - i64::from(dx) + x as i64) * src_w) / dw) as usize)
        .collect();
    for y in 0..(y1 - y0) as usize {
        let dst_y = y0 + y as i64;
        // Igual garantía de cota que x: `dst_rel_y ∈ [0, dh)` ⇒ `src_y ∈ [0, src_h)`.
        let src_y = (((dst_y - i64::from(dy)) * src_h) / dh) as usize;
        let src_row = &src.data[src_y * src_row_bytes..(src_y + 1) * src_row_bytes];
        let dst_row = unsafe {
            std::slice::from_raw_parts_mut(
                dst.add((dst_y as usize * dst_stride + x0 as usize) * bpp),
                vis_w * bpp,
            )
        };
        match bpp {
            4 => {
                for x in 0..vis_w {
                    let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                    dst_row[x * 4..x * 4 + 4].copy_from_slice(px);
                }
            }
            2 => {
                for x in 0..vis_w {
                    let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                    let c = rgb565(px[0], px[1], px[2]).to_ne_bytes();
                    dst_row[x * 2..x * 2 + 2].copy_from_slice(&c);
                }
            }
            _ => {
                // Fallback: primeros `bpp` bytes de cada píxel RGBA8.
                let n = bpp.min(3);
                for x in 0..vis_w {
                    let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                    dst_row[x * bpp..x * bpp + n].copy_from_slice(&px[..n]);
                }
            }
        }
    }
}

/// Conversión RGBA8 → RGB565 (formato `R5G6B5_UNORM` de Android, u16
/// little-endian). Duplicada aquí (misma fórmula que `draw::rgb565`, que es
/// privada) para no tocar draw.rs.
fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}
