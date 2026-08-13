//! Dibujo a bajo nivel: blit de buffers (`fill_buffer`, `copy_*`, `rgb565`,
//! `copy_region`) y render de listas/botones con `android.graphics.Canvas` vía
//! JNI (`jni_text_bitmap`).
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): aquí vive TODO
//! el dibujo; `reader` decide QUÉ dibujar y `zoom` (blit rápido) reutiliza
//! `fill_buffer`/`copy_region`.

use android_activity::ndk::native_window::NativeWindow;
use jni::objects::JValue;
use jni::{JavaVM, jni_sig, jni_str};
use log::{error, warn};
use pdf_core::{Bitmap, Document, Stroke};

use crate::annotations::ActiveStroke;
use crate::reader::{
    Reader, human_size, lib_letter_index, lib_strip_cell_h, lib_strip_letter, lib_strip_w,
    normalize_letter, picker_btn_h, picker_btn_w, picker_header_h, picker_row_h,
    picker_visible_rows, truncate_name,
};
use crate::{
    COLOR_BTN_W_DIV, DARK_BTN_W_DIV, JUMP_BTN_W_DIV, OPEN_BTN_W_DIV, PENCIL_BTN_W_DIV,
    UNDO_BTN_W_DIV, VIEWER_BAR_H_DIV,
};

/// Rellena la zona visible del buffer (`w` píxeles por fila de `stride` píxeles)
/// con `color` RGBA8. bpp 4 y 2 usan relleno rápido; otros bpp, byte a byte.
pub(crate) fn fill_buffer(
    dst: *mut u8,
    w: usize,
    h: usize,
    stride: usize,
    bpp: usize,
    color: [u8; 4],
) {
    match bpp {
        4 => {
            let color = u32::from_ne_bytes(color);
            for y in 0..h {
                let row = unsafe {
                    std::slice::from_raw_parts_mut(dst.add(y * stride * 4) as *mut u32, w)
                };
                row.fill(color);
            }
        }
        2 => {
            let color = rgb565(color[0], color[1], color[2]);
            for y in 0..h {
                let row = unsafe {
                    std::slice::from_raw_parts_mut(dst.add(y * stride * 2) as *mut u16, w)
                };
                row.fill(color);
            }
        }
        _ => {
            let n = bpp.min(4);
            for y in 0..h {
                let row =
                    unsafe { std::slice::from_raw_parts_mut(dst.add(y * stride * bpp), w * bpp) };
                for px in row.chunks_exact_mut(bpp) {
                    px[..n].copy_from_slice(&color[..n]);
                }
            }
        }
    }
}

/// Copia una fila de píxeles RGBA8 (`src`, 4 bytes/px) a `dst` en el formato del
/// buffer (mismo número de píxeles en ambos). bpp 4 = copia directa (caso
/// normal tras forzar R8G8B8A8_UNORM); bpp 2 = conversión a RGB565.
fn copy_row_rgba_to(dst: &mut [u8], src: &[u8], bpp: usize) {
    match bpp {
        4 => dst.copy_from_slice(&src[..dst.len()]),
        2 => {
            for (out, px) in dst.chunks_exact_mut(2).zip(src.chunks_exact(4)) {
                out.copy_from_slice(&rgb565(px[0], px[1], px[2]).to_ne_bytes());
            }
        }
        _ => {
            let n = bpp.min(3);
            for (out, px) in dst.chunks_exact_mut(bpp).zip(src.chunks_exact(4)) {
                out[..n].copy_from_slice(&px[..n]);
            }
        }
    }
}

/// Conversión RGBA8 → RGB565 (formato `R5G6B5_UNORM` de Android, u16 little-endian).
fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// Copia la intersección de `src` (bitmap RGBA8) con la ventana del buffer
/// `dst` (formato `bpp` bytes/px), con la esquina superior-izquierda de `src`
/// en `(sx, sy)` px del buffer. Recorta los bordes fuera del buffer (zoom > 1,
/// pan o botones en los bordes).
//
// 8 parámetros posicionales de un blit (raw pointer + dimensiones): se acepta
// el allow en vez de empaquetarlos en una struct que solo se usaría aquí.
#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_region(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    src: &Bitmap,
    sx: i32,
    sy: i32,
) {
    let src_w = src.width as i32;
    let src_h = src.height as i32;
    let x0 = sx.max(0);
    let y0 = sy.max(0);
    let x1 = (sx + src_w).min(dst_w as i32);
    let y1 = (sy + src_h).min(dst_h as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let copy_w = (x1 - x0) as usize;
    let copy_h = (y1 - y0) as usize;
    // Origen dentro del bitmap (0 cuando el bitmap no sobresale).
    let src_ox = (x0 - sx) as usize;
    let src_oy = (y0 - sy) as usize;
    for y in 0..copy_h {
        let row_off = ((src_oy + y) * src_w as usize + src_ox) * 4;
        let src_row = &src.data[row_off..row_off + copy_w * 4];
        let dst_row = unsafe {
            std::slice::from_raw_parts_mut(
                dst.add(((y0 as usize + y) * dst_stride + x0 as usize) * bpp),
                copy_w * bpp,
            )
        };
        copy_row_rgba_to(dst_row, src_row, bpp);
    }
}

/// Una página de la columna (scroll vertical continuo) a blitear en el
/// buffer: índice de página + bitmap cacheado + esquina superior izquierda en
/// px de ventana + zoom RELATIVO al render (1.0 = blit 1:1 nítido; > 1 durante
/// el pinch sin re-render, escala vecino-más-cercano del bitmap cacheado). El
/// índice lo usa `blit_stacked` para emparejar la página con su capa de
/// anotaciones.
pub(crate) struct PageBlit<'a> {
    pub(crate) page: u32,
    pub(crate) bitmap: &'a Bitmap,
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    pub(crate) zoom: f32,
}

/// Capa de anotaciones de una página visible: trazos (coordenadas de página,
/// puntos PDF) + transformación página→ventana (`× scale` + `(dx, dy)`), con
/// `dx`/`dy`/`scale` EXACTAMENTE los del blit de la página (`PageBlit`). Se
/// dibuja como capa vectorial SOBRE el bitmap ya bliteado — nunca se
/// rasteriza dentro del bitmap cacheado (AGENTS.md §4.3): así la anotación
/// permanece nítida a cualquier zoom y el coste del render es ∝ trazos
/// visibles, no ∝ área de página.
pub(crate) struct PageAnnots<'a> {
    pub(crate) page: u32,
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    /// px de ventana por punto PDF (cover × zoom).
    pub(crate) scale: f32,
    /// Trazos ya guardados de la página, en orden de dibujo (z).
    pub(crate) strokes: Vec<&'a Stroke>,
    /// Trazo en curso (modo dibujo), dibujado encima de los guardados.
    pub(crate) active: Option<&'a ActiveStroke>,
}

/// Blit de la columna de páginas apiladas (scroll vertical continuo) con UN
/// solo lock+present: fondo + cada página (vecino-más-cercano para el zoom,
/// recorte a la ventana) + la capa de anotaciones de cada página (trazos
/// Bresenham sobre su bitmap) + overlay de la barra superior. Es el
/// equivalente multi-página de `zoom::blit_fast` (mismo contrato: fondo +
/// página(s) + overlay en el mismo buffer, un único unlock_and_post —
/// dividirlo en varios locks presentaría varios buffers por frame y el
/// compositor mostraría el frame anterior).
///
/// `dark` = modo oscuro activo: invierte los canales RGB de cada página
/// (255 − v, la misma transformación que `pdf_core::dark::invert_bitmap`) en
/// el propio blit, píxel a píxel. La caché guarda SIEMPRE bitmaps normales;
/// materializar una copia invertida por página y frame sería memoria y GC
/// innecesarios (a zoom alto una página puede pesar cientos de MiB). Coste:
/// una pasada extra por página visible (~1-3 ms a pantalla completa).
///
/// Las ANOTACIONES no se invierten en modo oscuro: la tinta conserva su
/// color (decisión: la capa de anotaciones es independiente del modo de
/// visualización de la página, como el subrayado físico).
pub(crate) fn blit_stacked(
    window: &NativeWindow,
    bg: [u8; 4],
    dark: bool,
    pages: &[PageBlit],
    anns: &[PageAnnots],
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

    // Páginas en orden de documento: cada una escalada al zoom relativo
    // pedido y recortada a la ventana (las posiciones las calcula `reader`
    // con el layout de la columna: offset acumulado − scroll_y). Tras cada
    // página, su capa de anotaciones (trazos sobre el bitmap, en orden z).
    for page in pages {
        blit_page_scaled(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            bpp,
            page.bitmap,
            page.dx,
            page.dy,
            page.zoom,
            dark,
        );
        if let Some(layer) = anns.iter().find(|l| l.page == page.page) {
            draw_annotations(dst, dst_w, dst_h, dst_stride, bpp, layer);
        }
    }

    // Overlay del visor (barra superior), esquina superior izquierda.
    if let Some(ov) = overlay {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, 0, 0);
    }
}

/// Escala `src` (RGBA8) por vecino-más-cercano a tamaño `src × zoom` y copia
/// el resultado al buffer con su esquina superior izquierda en `(dx, dy)` px,
/// recortando los bordes fuera de la ventana. Espejo de
/// `zoom::blit_scaled_nearest` (que es privado y no se puede reutilizar desde
/// aquí): misma fórmula, mismo estilo — véase su doc para el mapeo entero
/// `src = (dst_rel × src_dim) / dst_dim` con tabla x precalculada.
///
/// `dark` añade la inversión de canales RGB inline (ver `blit_stacked`).
//
// 11 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `copy_region` y `blit_scaled_nearest`; se acepta el allow en vez
// de empaquetarlos en una struct que solo se usaría aquí.
#[allow(clippy::too_many_arguments)]
fn blit_page_scaled(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    src: &Bitmap,
    dx: i32,
    dy: i32,
    zoom: f32,
    dark: bool,
) {
    let src_w = src.width as i64;
    let src_h = src.height as i64;
    // Guardas: zoom inválido o destino degenerado → solo fondo (ya pintado
    // por el caller; el overlay lo pinta `blit_stacked` después).
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
                if dark {
                    for x in 0..vis_w {
                        let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                        let o = x * 4;
                        dst_row[o] = 255 - px[0];
                        dst_row[o + 1] = 255 - px[1];
                        dst_row[o + 2] = 255 - px[2];
                        dst_row[o + 3] = px[3];
                    }
                } else {
                    for x in 0..vis_w {
                        let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                        dst_row[x * 4..x * 4 + 4].copy_from_slice(px);
                    }
                }
            }
            2 => {
                for x in 0..vis_w {
                    let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                    let (r, g, b) = if dark {
                        (255 - px[0], 255 - px[1], 255 - px[2])
                    } else {
                        (px[0], px[1], px[2])
                    };
                    dst_row[x * 2..x * 2 + 2].copy_from_slice(&rgb565(r, g, b).to_ne_bytes());
                }
            }
            _ => {
                // Fallback: primeros `bpp` bytes de cada píxel RGBA8.
                let n = bpp.min(3);
                for x in 0..vis_w {
                    let px = &src_row[x_map[x] * 4..x_map[x] * 4 + 4];
                    let o = x * bpp;
                    if dark {
                        for c in 0..n {
                            dst_row[o + c] = 255 - px[c];
                        }
                    } else {
                        dst_row[o..o + n].copy_from_slice(&px[..n]);
                    }
                }
            }
        }
    }
}

/// Dibuja la capa de anotaciones de una página en el buffer: cada trazo
/// guardado (orden z) y, encima, el trazo en curso. La transformación
/// página→ventana (`× scale + (dx, dy)`) es la misma del blit de la página
/// (ver `PageAnnots`); el grosor se escala igual: `width_px = width_pt × scale`.
/// Coste ∝ nº de puntos de los trazos visibles × área de la brocha — nunca
/// ∝ área de la página, como exige el presupuesto de Fase 1/2 (200 trazos
/// visibles sin degradar el frame time).
fn draw_annotations(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    layer: &PageAnnots,
) {
    let scale = layer.scale;
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    for s in &layer.strokes {
        draw_stroke(
            dst, dst_w, dst_h, dst_stride, bpp, &s.points, s.width, s.color, scale, layer.dx,
            layer.dy,
        );
    }
    if let Some(act) = layer.active {
        draw_stroke(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            bpp,
            &act.points,
            act.width,
            act.color,
            scale,
            layer.dx,
            layer.dy,
        );
    }
}

/// Transforma un trazo (coordenadas de página) a px de ventana
/// (`pt × scale + (dx, dy)`) y lo dibuja como polilínea Bresenham de grosor
/// `width × scale` px (mínimo 1 px). El color se pasa tal cual (RGBA8): las
/// anotaciones no se invierten en modo oscuro (ver `blit_stacked`).
//
// 10 parámetros posicionales de un blit (raw pointer + dimensiones + trazo):
// mismo patrón que `copy_region` y `blit_page_scaled`; se acepta el allow.
#[allow(clippy::too_many_arguments)]
fn draw_stroke(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    points: &[(f32, f32)],
    width_pt: f32,
    color: pdf_core::Color,
    scale: f32,
    dx: i32,
    dy: i32,
) {
    if points.len() < 2 {
        return;
    }
    let pts: Vec<(f32, f32)> = points
        .iter()
        .map(|&(px, py)| (px * scale + dx as f32, py * scale + dy as f32))
        .collect();
    let width_px = (width_pt * scale).max(1.0);
    draw_polyline(
        dst,
        dst_w,
        dst_h,
        dst_stride,
        bpp,
        &pts,
        width_px,
        [color.r, color.g, color.b, color.a],
    );
}

/// Dibuja una polilínea en el buffer (coordenadas de ventana, px) con
/// Bresenham por segmento y brocha circular de radio `width_px/2` (extremos
/// redondeados, juntas suaves). Respeta `bpp`/`stride` y recorta a la ventana.
///
/// - Cada segmento se recorta con Liang–Barsky a la ventana extendida por el
///   radio de la brocha: Bresenham no camina por fuera de la pantalla.
/// - La brocha (offsets del disco) se precalcula una vez por trazo; radio 1
///   (grosor ≤ 2 px, el caso normal) usa un solo píxel por punto de línea.
/// - bpp 4: alfa-blend del color sobre el bitmap (la tinta puede ser
///   translúcida); bpp 2 (RGB565): escritura opaca (blend en 565 no merece
///   la pena); otros bpp: primeros `bpp` bytes del color.
///
/// Coste ∝ nº de puntos × área del disco (∝ trazos visibles, ver
/// `draw_annotations`).
//
// 8 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `copy_region` y `blit_page_scaled`; se acepta el allow.
#[allow(clippy::too_many_arguments)]
fn draw_polyline(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    pts: &[(f32, f32)],
    width_px: f32,
    color: [u8; 4],
) {
    if pts.len() < 2 || dst_w == 0 || dst_h == 0 {
        return;
    }
    let r = ((width_px / 2.0).ceil().max(1.0)) as i32;
    // Brocha: offsets del disco de radio r (centro + vecinos). Radio 1 → solo
    // el centro (trazo fino = una línea de 1 px, sin solapamientos extra).
    let disc: Vec<(i32, i32)> = if r == 1 {
        vec![(0, 0)]
    } else {
        let r2 = (r as i64) * (r as i64);
        let mut d = Vec::with_capacity((2 * r as usize + 1).pow(2));
        for oy in -r..=r {
            for ox in -r..=r {
                if (ox as i64) * (ox as i64) + (oy as i64) * (oy as i64) <= r2 {
                    d.push((ox, oy));
                }
            }
        }
        d
    };
    let (xmin, ymin) = (-(r as f32), -(r as f32));
    let (xmax, ymax) = (dst_w as f32 + r as f32, dst_h as f32 + r as f32);
    let mut last = pts[0];
    for &p in &pts[1..] {
        // Liang–Barsky al rectángulo [−r, w+r) × [−r, h+r): descarta los
        // segmentos enteramente fuera de la ventana y acorta los parciales.
        if let Some(((x0, y0), (x1, y1))) = clip_segment(last, p, xmin, ymin, xmax, ymax) {
            bresenham(
                x0.round() as i32,
                y0.round() as i32,
                x1.round() as i32,
                y1.round() as i32,
                |x, y| stamp(dst, dst_w, dst_h, dst_stride, bpp, x, y, &disc, color),
            );
        }
        last = p;
    }
}

/// Recorte de segmento Liang–Barsky a la ventana `[xmin, xmax] × [ymin, ymax]`
/// (puede incluir un margen para la brocha). Devuelve `None` si el segmento
/// queda enteramente fuera. Aritmética en f32 con t ∈ [0, 1]: los extremos
/// recortados se redondean después, en `draw_polyline`.
fn clip_segment(
    p0: (f32, f32),
    p1: (f32, f32),
    xmin: f32,
    ymin: f32,
    xmax: f32,
    ymax: f32,
) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let mut t0 = 0.0f32;
    let mut t1 = 1.0f32;
    // p, q para cada borde (Liang–Barsky): t0 = entrada, t1 = salida.
    let edges = [
        (-dx, p0.0 - xmin), // x ≥ xmin  →  dx·t ≥ xmin − x0
        (dx, xmax - p0.0),  // x ≤ xmax
        (-dy, p0.1 - ymin), // y ≥ ymin
        (dy, ymax - p0.1),  // y ≤ ymax
    ];
    for (p, q) in edges {
        if p == 0.0 {
            if q < 0.0 {
                return None; // paralelo y fuera
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                // borde de entrada
                if t > t1 {
                    return None;
                }
                t0 = t0.max(t);
            } else {
                // borde de salida
                if t < t0 {
                    return None;
                }
                t1 = t1.min(t);
            }
        }
    }
    Some((
        (p0.0 + t0 * dx, p0.1 + t0 * dy),
        (p0.0 + t1 * dx, p0.1 + t1 * dy),
    ))
}

/// Algoritmo de línea de Bresenham (octantes enteros, sin f32): invoca
/// `plot(x, y)` para cada píxel del segmento, incluidos ambos extremos.
fn bresenham<F: FnMut(i32, i32)>(mut x0: i32, mut y0: i32, x1: i32, y1: i32, mut plot: F) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        plot(x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Estampa la brocha (offsets del disco) centrada en `(x, y)` en el buffer,
/// recortando los píxeles fuera de la ventana. Escritura por bpp: 4 =
/// alfa-blend RGBA8 (la tinta respeta la transparencia del `Color`), 2 =
/// RGB565 opaco, otros = primeros `bpp` bytes del color.
//
// 9 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `copy_region` y `blit_page_scaled`; se acepta el allow.
#[allow(clippy::too_many_arguments)]
fn stamp(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    x: i32,
    y: i32,
    disc: &[(i32, i32)],
    color: [u8; 4],
) {
    for &(ox, oy) in disc {
        let px = x + ox;
        let py = y + oy;
        if px < 0 || py < 0 || px >= dst_w as i32 || py >= dst_h as i32 {
            continue;
        }
        let p = unsafe { dst.add((py as usize * dst_stride + px as usize) * bpp) };
        match bpp {
            4 => {
                let a = color[3] as u32;
                if a == 255 {
                    unsafe {
                        *p = color[0];
                        *p.add(1) = color[1];
                        *p.add(2) = color[2];
                    }
                } else if a > 0 {
                    // alfa-blend: dst = src·a + dst·(255−a) / 255
                    unsafe {
                        for (i, &c) in color[..3].iter().enumerate() {
                            let d = *p.add(i) as u32;
                            *p.add(i) = ((c as u32 * a + d * (255 - a)) / 255) as u8;
                        }
                    }
                }
            }
            2 => unsafe {
                let v = rgb565(color[0], color[1], color[2]).to_ne_bytes();
                *p = v[0];
                *p.add(1) = v[1];
            },
            _ => {
                let n = bpp.min(3);
                unsafe { std::ptr::copy_nonoverlapping(color.as_ptr(), p, n) };
            }
        }
    }
}

/// Dibuja rectángulos y textos (fuente del sistema, antialiasing) con
/// `android.graphics.Canvas` vía JNI y devuelve el resultado como `Bitmap`
/// RGBA8. Orden de dibujo: fondo → rects → textos.
///
/// `rects` = (left, top, right, bottom, color ARGB); `texts` =
/// (x, baseline_y, text_size_px, color ARGB, texto). Colores en 0xAARRGGBB.
///
/// `JavaVM::singleton()` ya está inicializado por android-activity (misma
/// versión de jni) y el hilo `android_main` está attachado por la glue, así
/// que `attach_current_thread` es un no-op barato. Los nombres y firmas se
/// pasan con `jni_str!`/`jni_sig!` (el API seguro de jni 0.22 no acepta
/// `&str`).
fn jni_text_bitmap(
    w: i32,
    h: i32,
    bg: u32,
    rects: &[(f32, f32, f32, f32, u32)],
    texts: &[(f32, f32, f32, u32, String)],
) -> Option<Bitmap> {
    let vm = JavaVM::singleton().ok()?;
    let n = w as usize * h as usize;
    let res: jni::errors::Result<Bitmap> = vm.attach_current_thread(|env| {
        env.with_local_frame(64, |env| {
            // android.graphics.Bitmap.Config.ARGB_8888
            let config_class = env.find_class(jni_str!("android/graphics/Bitmap$Config"))?;
            let config = env
                .get_static_field(
                    &config_class,
                    jni_str!("ARGB_8888"),
                    jni_sig!(sig = android.graphics.Bitmap::Config),
                )?
                .l()?;
            let bitmap_class = env.find_class(jni_str!("android/graphics/Bitmap"))?;
            let bmp = env
                .call_static_method(
                    &bitmap_class,
                    jni_str!("createBitmap"),
                    jni_sig!(
                        sig = (int, int, android.graphics.Bitmap::Config) -> android.graphics.Bitmap
                    ),
                    &[JValue::Int(w), JValue::Int(h), JValue::Object(&config)],
                )?
                .l()?;
            let canvas_class = env.find_class(jni_str!("android/graphics/Canvas"))?;
            let canvas = env.new_object(
                &canvas_class,
                jni_sig!(sig = (android.graphics.Bitmap) -> void),
                &[JValue::Object(&bmp)],
            )?;
            let paint_class = env.find_class(jni_str!("android/graphics/Paint"))?;
            let paint = env.new_object(&paint_class, jni_sig!(sig = () -> void), &[])?;
            env.call_method(
                &paint,
                jni_str!("setAntiAlias"),
                jni_sig!(sig = (boolean) -> void),
                &[JValue::Bool(true)],
            )?;

            // Fondo.
            env.call_method(
                &paint,
                jni_str!("setColor"),
                jni_sig!(sig = (int) -> void),
                &[JValue::Int(bg as i32)],
            )?;
            env.call_method(
                &canvas,
                jni_str!("drawRect"),
                jni_sig!(sig = (float, float, float, float, android.graphics.Paint) -> void),
                &[
                    JValue::Float(0.0),
                    JValue::Float(0.0),
                    JValue::Float(w as f32),
                    JValue::Float(h as f32),
                    JValue::Object(&paint),
                ],
            )?;
            // Rectángulos (cabecera, botones, filas, estado).
            for &(l, t, r, b, color) in rects {
                env.call_method(
                    &paint,
                    jni_str!("setColor"),
                    jni_sig!(sig = (int) -> void),
                    &[JValue::Int(color as i32)],
                )?;
                env.call_method(
                    &canvas,
                    jni_str!("drawRect"),
                    jni_sig!(sig = (float, float, float, float, android.graphics.Paint) -> void),
                    &[
                        JValue::Float(l),
                        JValue::Float(t),
                        JValue::Float(r),
                        JValue::Float(b),
                        JValue::Object(&paint),
                    ],
                )?;
            }
            // Textos: un JString por texto, liberado tras dibujar (la tabla de
            // refs locales del frame no debe crecer con el nº de textos).
            for (x, y, size, color, text) in texts {
                env.call_method(
                    &paint,
                    jni_str!("setColor"),
                    jni_sig!(sig = (int) -> void),
                    &[JValue::Int(*color as i32)],
                )?;
                env.call_method(
                    &paint,
                    jni_str!("setTextSize"),
                    jni_sig!(sig = (float) -> void),
                    &[JValue::Float(*size)],
                )?;
                let jstr = env.new_string(text.as_str())?;
                env.call_method(
                    &canvas,
                    jni_str!("drawText"),
                    jni_sig!(
                        sig = ("java.lang.String", float, float, android.graphics.Paint) -> void
                    ),
                    &[
                        JValue::Object(jstr.as_ref()),
                        JValue::Float(*x),
                        JValue::Float(*y),
                        JValue::Object(&paint),
                    ],
                )?;
                env.delete_local_ref(jstr);
            }
            // getPixels (ARGB int) → nuestros bytes RGBA8.
            let mut px = vec![0i32; n];
            let jarr = env.new_int_array(n)?;
            env.call_method(
                &bmp,
                jni_str!("getPixels"),
                jni_sig!(sig = ([int], int, int, int, int, int, int) -> void),
                &[
                    JValue::Object(jarr.as_ref()),
                    JValue::Int(0),
                    JValue::Int(w),
                    JValue::Int(0),
                    JValue::Int(0),
                    JValue::Int(w),
                    JValue::Int(h),
                ],
            )?;
            jarr.get_region(env, 0, &mut px)?;
            let mut data = Vec::with_capacity(n * 4);
            for p in px {
                data.extend_from_slice(&[
                    (p >> 16) as u8,
                    (p >> 8) as u8,
                    p as u8,
                    (p >> 24) as u8,
                ]);
            }
            Ok(Bitmap {
                width: w as u32,
                height: h as u32,
                data,
            })
        })
    });
    match res {
        Ok(bmp) => Some(bmp),
        Err(e) => {
            // Si el error fue una excepción Java, queda pendiente en el JVM:
            // limpiarla para no envenenar llamadas JNI posteriores.
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("jni_text_bitmap ({w}x{h}): {e}");
            None
        }
    }
}

/// Renderiza la barra superior del visor a un bitmap RGBA8 de tamaño
/// `win_w × (win_h / VIEWER_BAR_H_DIV)`: botón "Open" (izquierda), saltos
/// −10/+10 (a los lados de la zona central), indicador de página "N / total"
/// (centrado en su zona; tap = página siguiente), botón ✏️ (modo dibujo,
/// ámbar activo), botón ● (color del trazo) y undo ↶ (quitar último trazo), y
/// toggle "Dark"/"Light" (derecha, ámbar cuando el modo oscuro está activo).
///
/// Sustituye al antiguo botón "Open" suelto: es UN único overlay opaco
/// bliteado en (0,0) por `zoom::blit_fast`, así el indicador y los botones
/// no requieren tocar `zoom.rs`. Se cachea por tamaño de ventana y se
/// invalida al cambiar de página, alternar el modo oscuro, el modo dibujo o
/// el color del trazo (ver `Reader::redraw`/`toggle_dark`/`toggle_draw_mode`/
/// `cycle_stroke_color`). La geometría DEBE coincidir con las zonas de tap de
/// `input.rs` (mismas divisiones de la ventana).
pub(crate) fn render_viewer_bar(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = (reader.win_h / VIEWER_BAR_H_DIV).max(1);
    let pad = (h / 6).max(2) as f32;
    let ts = h as f32 * 0.5;
    let hh = h as f32;

    let open_w = w / OPEN_BTN_W_DIV;
    let pencil_w = w / PENCIL_BTN_W_DIV;
    let color_w = w / COLOR_BTN_W_DIV;
    let undo_w = w / UNDO_BTN_W_DIV;
    let dark_w = w / DARK_BTN_W_DIV;
    let jump_w = w / JUMP_BTN_W_DIV;
    // Bordes de la zona central del indicador (entre los grupos de botones).
    let left_end = open_w + pencil_w + color_w + undo_w + jump_w;
    let right_start = w - dark_w - jump_w;

    let mut rects: Vec<(f32, f32, f32, f32, u32)> = Vec::new();
    let mut texts: Vec<(f32, f32, f32, u32, String)> = Vec::new();

    // Botón con fondo `color` y etiqueta centrada (heurística ts*0.4/carácter).
    let mut button = |rect: (f32, f32, f32, f32), color: u32, label: &str| {
        rects.push((rect.0, rect.1, rect.2, rect.3, color));
        let cx = rect.0 + (rect.2 - rect.0) * 0.5 - ts * 0.4 * label.chars().count() as f32;
        texts.push((
            cx,
            rect.1 + (rect.3 - rect.1) * 0.66,
            ts,
            0xFFFFFFFF,
            label.to_string(),
        ));
    };

    // Open (azul) a la izquierda; modo dibujo ✏️ (ámbar activo) a su derecha.
    button(
        (pad, pad, open_w as f32 - pad, hh - pad),
        0xFF3A5A8C,
        "Open",
    );
    let pen_color = if reader.draw_mode {
        0xFF8C6A3A
    } else {
        0xFF3A4A5A
    };
    button(
        (
            open_w as f32 + pad,
            pad,
            (open_w + pencil_w) as f32 - pad,
            hh - pad,
        ),
        pen_color,
        "✏️",
    );
    // Undo ↶ (quitar el último trazo de la página actual).
    button(
        (
            (open_w + pencil_w + color_w) as f32 + pad,
            pad,
            (open_w + pencil_w + color_w + undo_w) as f32 - pad,
            hh - pad,
        ),
        0xFF3A4A5A,
        "↶",
    );
    // Salto −10 (gris) a la izquierda del indicador; +10 a su derecha.
    button(
        (
            (left_end - jump_w) as f32 + pad,
            pad,
            left_end as f32 - pad,
            hh - pad,
        ),
        0xFF3A4A5A,
        "-10",
    );
    button(
        (
            right_start as f32 + pad,
            pad,
            (right_start + jump_w) as f32 - pad,
            hh - pad,
        ),
        0xFF3A4A5A,
        "+10",
    );
    let dark_color = if reader.dark { 0xFF8C6A3A } else { 0xFF3A5A8C };
    button(
        ((w - dark_w) as f32 + pad, pad, w as f32 - pad, hh - pad),
        dark_color,
        if reader.dark { "Light" } else { "Dark" },
    );

    // Botón ● (color del trazo) e indicador "N / total": se dibujan DESPUÉS
    // del último uso de la closure `button` (su borrow de `rects`/`texts`
    // termina ahí). El ● necesita color de texto propio (el color actual de
    // la tinta) y el indicador se centra en su zona (entre los grupos de
    // botones) para no solaparse con ellos.
    let color_rect = (
        (open_w + pencil_w) as f32 + pad,
        pad,
        (open_w + pencil_w + color_w) as f32 - pad,
        hh - pad,
    );
    rects.push((
        color_rect.0,
        color_rect.1,
        color_rect.2,
        color_rect.3,
        0xFF3A4A5A,
    ));
    let sc = reader.stroke_color;
    let argb = ((sc.a as u32) << 24) | ((sc.r as u32) << 16) | ((sc.g as u32) << 8) | sc.b as u32;
    texts.push((
        color_rect.0 + (color_rect.2 - color_rect.0) * 0.5 - ts * 0.4,
        color_rect.1 + (color_rect.3 - color_rect.1) * 0.66,
        ts,
        argb,
        "●".to_string(),
    ));

    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let label = format!("{} / {}", reader.page + 1, pages);
    let zone_cx = (left_end as f32 + right_start as f32) * 0.5;
    let cx = zone_cx - ts * 0.55 * label.chars().count() as f32;
    texts.push((cx, hh * 0.66, ts, 0xFFF0F0F0, label));

    // Fondo de la barra: negro puro en modo oscuro (se funde con la página).
    let bg = if reader.dark { 0xFF000000 } else { 0xFF262626 };
    jni_text_bitmap(w, h, bg, &rects, &texts)
}

/// Renderiza la lista del picker a un bitmap RGBA8 de tamaño de ventana:
/// cabecera con título y botones (Back/Rescan), franja de estado opcional y
/// las filas de PDFs visibles. La geometría DEBE coincidir con `picker_tap`
/// (mismas fórmulas de layout).
pub(crate) fn render_picker_list(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    let row_h = picker_row_h(h);
    let header_h = picker_header_h(h);
    let status_h = if reader.status.is_some() { row_h } else { 0 };
    let btn_w = picker_btn_w(w);
    let btn_h = picker_btn_h(h);
    let pad = (w / 32).max(8) as f32;

    let mut rects: Vec<(f32, f32, f32, f32, u32)> = Vec::new();
    let mut texts: Vec<(f32, f32, f32, u32, String)> = Vec::new();

    // Cabecera: fondo + título.
    rects.push((0.0, 0.0, w as f32, header_h as f32, 0xFF2A2A2A));
    let title_ts = row_h as f32 * 0.5;
    texts.push((
        pad,
        header_h as f32 * 0.68,
        title_ts,
        0xFFF0F0F0,
        "Open PDF".to_string(),
    ));

    // Botones de la cabecera (derecha): Back (solo con PDF abierto) y Rescan.
    let btn_y = (header_h - btn_h) as f32 / 2.0;
    let back_x = w as f32 - btn_w as f32 * 2.0 - 16.0;
    let rescan_x = w as f32 - btn_w as f32 - 8.0;
    if reader.doc.is_some() {
        rects.push((
            back_x,
            btn_y,
            back_x + btn_w as f32,
            btn_y + btn_h as f32,
            0xFF3A4A5A,
        ));
        let ts = btn_h as f32 * 0.4;
        texts.push((
            back_x + btn_w as f32 * 0.5 - ts * 1.6,
            btn_y + btn_h as f32 * 0.64,
            ts,
            0xFFFFFFFF,
            "Back".to_string(),
        ));
    }
    rects.push((
        rescan_x,
        btn_y,
        rescan_x + btn_w as f32,
        btn_y + btn_h as f32,
        0xFF3A5A8C,
    ));
    let ts = btn_h as f32 * 0.4;
    texts.push((
        rescan_x + btn_w as f32 * 0.5 - ts * 2.4,
        btn_y + btn_h as f32 * 0.64,
        ts,
        0xFFFFFFFF,
        "Rescan".to_string(),
    ));

    // Franja de estado (errores, mensaje de arranque sin PDF).
    let rows_y0 = header_h + status_h;
    if let Some(status) = reader.status.as_deref() {
        rects.push((0.0, header_h as f32, w as f32, rows_y0 as f32, 0xFF3A1E1E));
        let ts = row_h as f32 * 0.36;
        texts.push((
            pad,
            header_h as f32 + row_h as f32 * 0.64,
            ts,
            0xFFFFC9C9,
            status.to_string(),
        ));
    }

    // Filas de PDFs visibles (alternando fondo).
    let visible = picker_visible_rows(h, reader.status.is_some());
    let row_ts = row_h as f32 * 0.42;
    for i in 0..visible {
        let r = reader.list_scroll + i;
        let Some(entry) = reader.pdf_list.get(r) else {
            break;
        };
        let y0 = (rows_y0 + (i as i32) * row_h) as f32;
        let bg = if i % 2 == 0 { 0xFF232323 } else { 0xFF282828 };
        rects.push((0.0, y0, w as f32, y0 + row_h as f32, bg));
        // Nombre (+ etiqueta de origen), truncado; tamaño a la derecha.
        let size_str = human_size(entry.size);
        let char_w = row_ts * 0.58;
        let size_w = size_str.chars().count() as f32 * char_w + pad;
        let max_chars = (((w as f32 - pad * 3.0 - size_w) / char_w) as usize).max(1);
        let label = format!(
            "{} [{}]",
            truncate_name(&entry.name, max_chars),
            entry.source
        );
        texts.push((pad, y0 + row_h as f32 * 0.68, row_ts, 0xFFF0F0F0, label));
        texts.push((
            w as f32 - pad - size_w,
            y0 + row_h as f32 * 0.68,
            row_ts,
            0xFF9A9A9A,
            size_str,
        ));
    }

    jni_text_bitmap(w, h, 0xFF1E1E1E, &rects, &texts)
}

/// Rectángulo de botón (left, top, right, bottom) en px.
pub(crate) type ButtonRect = (f32, f32, f32, f32);

/// Botones de la cabecera de la biblioteca, construidos de derecha a
/// izquierda: Rescan (siempre), Grant (si falta el permiso, API 30+) y Back
/// (si hay un PDF abierto). Compartida por render y tap para que la geometría
/// no pueda divergir.
pub(crate) fn library_buttons(
    reader: &Reader,
    win_w: f32,
    btn_w: f32,
    btn_h: f32,
    btn_y: f32,
) -> Vec<(&'static str, ButtonRect)> {
    let mut out = Vec::new();
    let mut right = win_w - 8.0;
    for label in ["Rescan", "Grant", "Back"] {
        let show = match label {
            "Rescan" => true,
            "Grant" => !reader.permission_granted && reader.sdk_int >= 30,
            _ => reader.doc.is_some(),
        };
        if !show {
            continue;
        }
        let x0 = right - btn_w;
        out.push((label, (x0, btn_y, right, btn_y + btn_h)));
        right = x0 - 8.0;
    }
    out
}

/// Renderiza la biblioteca MediaStore a un bitmap RGBA8 de tamaño de ventana:
/// cabecera con título ("Library", con la letra activa del filtro si lo hay)
/// y botones (Back/Grant/Rescan), franja de estado opcional, filas con NOMBRE
/// (primera línea) y CARPETA (segunda, más pequeña) y, en el borde derecho,
/// la TIRA DE LETRAS del índice (A-Z + '#', `lib_strip_*`): tocar una celda
/// filtra la lista por la letra inicial normalizada, repetir la activa la
/// desactiva (`input::library_tap`); las celdas sin entradas se atenúan y la
/// activa se resalta en ámbar. Reutiliza `jni_text_bitmap` (Canvas+JNI del
/// picker). La geometría DEBE coincidir con `library_tap`.
pub(crate) fn render_library_list(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    let row_h = picker_row_h(h);
    let header_h = picker_header_h(h);
    let status_h = if reader.status.is_some() { row_h } else { 0 };
    let btn_w = picker_btn_w(w);
    let btn_h = picker_btn_h(h);
    let btn_y = (header_h - btn_h) as f32 / 2.0;
    let pad = (w / 32).max(8) as f32;
    // Tira de letras (índice A-Z + '#') en el borde derecho: reservar su
    // ancho para que filas y tamaño no queden ocultos bajo ella.
    let strip_w = lib_strip_w(w);
    let list_w = w - strip_w;

    let mut rects: Vec<(f32, f32, f32, f32, u32)> = Vec::new();
    let mut texts: Vec<(f32, f32, f32, u32, String)> = Vec::new();

    // Cabecera: fondo + título (la letra activa del filtro, si lo hay).
    rects.push((0.0, 0.0, w as f32, header_h as f32, 0xFF2A2A2A));
    let title_ts = row_h as f32 * 0.5;
    let title = match reader.library_filter {
        Some(l) => format!("Library · {}", l.to_ascii_uppercase()),
        None => "Library".to_string(),
    };
    texts.push((pad, header_h as f32 * 0.68, title_ts, 0xFFF0F0F0, title));

    // Botones de la cabecera (derecha): Rescan, Grant (si falta permiso), Back.
    for (label, (l, t, r, b)) in
        library_buttons(reader, w as f32, btn_w as f32, btn_h as f32, btn_y)
    {
        let color = if label == "Rescan" {
            0xFF3A5A8C
        } else {
            0xFF3A4A5A
        };
        rects.push((l, t, r, b, color));
        let ts = btn_h as f32 * 0.4;
        // Centrado aproximado por caracteres (misma heurística que el picker).
        let cx = l + (r - l) * 0.5 - ts * 0.4 * label.len() as f32;
        texts.push((
            cx,
            t + btn_h as f32 * 0.64,
            ts,
            0xFFFFFFFF,
            label.to_string(),
        ));
    }

    // Franja de estado (permiso, error, avisos).
    let rows_y0 = header_h + status_h;
    if let Some(status) = reader.status.as_deref() {
        rects.push((0.0, header_h as f32, w as f32, rows_y0 as f32, 0xFF3A1E1E));
        let ts = row_h as f32 * 0.36;
        texts.push((
            pad,
            header_h as f32 + row_h as f32 * 0.64,
            ts,
            0xFFFFC9C9,
            status.to_string(),
        ));
    }

    // Filas visibles de la lista FILTRADA (nombre, carpeta, tamaño a la
    // derecha — dentro de la zona de lista, dejando la tira libre).
    let visible = picker_visible_rows(h, reader.status.is_some());
    let row_ts = row_h as f32 * 0.40;
    let sub_ts = row_h as f32 * 0.26;
    for i in 0..visible {
        let r = reader.list_scroll + i;
        let Some(entry) = reader.library_entry_at(r) else {
            break;
        };
        let y0 = (rows_y0 + (i as i32) * row_h) as f32;
        let bg = if i % 2 == 0 { 0xFF232323 } else { 0xFF282828 };
        rects.push((0.0, y0, list_w as f32, y0 + row_h as f32, bg));
        let size_str = human_size(entry.size.max(0) as u64);
        let char_w = row_ts * 0.58;
        let size_w = size_str.chars().count() as f32 * char_w + pad;
        let max_chars = (((list_w as f32 - pad * 3.0 - size_w) / char_w) as usize).max(1);
        texts.push((
            pad,
            y0 + row_h as f32 * 0.62,
            row_ts,
            0xFFF0F0F0,
            truncate_name(&entry.name, max_chars),
        ));
        texts.push((
            list_w as f32 - pad - size_w,
            y0 + row_h as f32 * 0.62,
            row_ts,
            0xFF9A9A9A,
            size_str,
        ));
        // Carpeta (RELATIVE_PATH, p. ej. "Download/" o "Document/Mates/3S/").
        let folder = if entry.folder.is_empty() {
            "(root)".to_string()
        } else {
            entry.folder.clone()
        };
        texts.push((
            pad,
            y0 + row_h as f32 * 0.88,
            sub_ts,
            0xFF9A9A9A,
            truncate_name(&folder, max_chars),
        ));
    }

    // Tira de letras (índice): columna opaca sobre el borde derecho de la
    // zona de filas. Las celdas sin entradas se atenúan; la activa se
    // resalta en ámbar. La presencia se calcula sobre la lista COMPLETA
    // (aunque haya filtro) para poder saltar a cualquier letra: tocar una
    // celda cambia el filtro y repetir la activa lo quita.
    rects.push((
        list_w as f32,
        rows_y0 as f32,
        w as f32,
        h as f32,
        0xFF1A1A1A,
    ));
    let cell_h = lib_strip_cell_h(h, rows_y0);
    let strip_ts = (cell_h * 0.55).min(row_h as f32 * 0.30);
    let mut present = [false; 27];
    for e in &reader.library_list {
        present[lib_letter_index(normalize_letter(&e.name))] = true;
    }
    for (i, has) in present.iter().enumerate() {
        let cy0 = rows_y0 as f32 + i as f32 * cell_h;
        let ch = lib_strip_letter(i);
        let active = reader.library_filter == Some(ch.to_ascii_lowercase());
        if active {
            rects.push((list_w as f32, cy0, w as f32, cy0 + cell_h, 0xFF8C6A3A));
        }
        let color = if active {
            0xFFFFE0B0
        } else if *has {
            0xFFF0F0F0
        } else {
            0xFF505050
        };
        let cx = list_w as f32 + strip_w as f32 * 0.5 - strip_ts * 0.32;
        texts.push((cx, cy0 + cell_h * 0.68, strip_ts, color, ch.to_string()));
    }

    jni_text_bitmap(w, h, 0xFF1E1E1E, &rects, &texts)
}
