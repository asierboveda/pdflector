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

use crate::reader::{
    GRID_CELL_PAD, GRID_COLS, Reader, grid_cell_h, grid_cell_rect, grid_cell_w, grid_cover_h,
    grid_cover_w, grid_pad, grid_rows_y0, grid_visible_rows, human_size, page_badge_size,
    picker_btn_h, picker_btn_w, picker_header_h, picker_row_h, picker_visible_rows, sheet_btn_h,
    sheet_btn_w, sheet_h, sheet_pad, sheet_row1_y, sheet_row2_y, truncate_name,
};
use crate::theme;

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
/// visibles, no ∝ área de página. Solo trazos YA GUARDADOS: el trazo en curso
/// (modo dibujo) se eliminó con la barra superior (2026-08-XX).
pub(crate) struct PageAnnots<'a> {
    pub(crate) page: u32,
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    /// px de ventana por punto PDF (cover × zoom).
    pub(crate) scale: f32,
    /// Trazos guardados de la página, en orden de dibujo (z).
    pub(crate) strokes: Vec<&'a Stroke>,
}

/// Blit de la columna de páginas apiladas (scroll vertical continuo) con UN
/// solo lock+present: fondo + cada página (vecino-más-cercano para el zoom,
/// recorte a la ventana) + la capa de anotaciones de cada página (trazos
/// Bresenham sobre su bitmap) + los overlays del visor (indicador de página y
/// sheet de ajustes, cada uno con su posición). Es el equivalente
/// multi-página de `zoom::blit_fast` (mismo contrato: fondo + página(s) +
/// overlays en el mismo buffer, un único unlock_and_post — dividirlo en
/// varios locks presentaría varios buffers por frame y el compositor
/// mostraría el frame anterior).
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
    overlays: &[(&Bitmap, i32, i32)],
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

    // Overlays del visor (indicador de página, sheet de ajustes), cada uno
    // con su esquina superior izquierda en px de ventana.
    for (ov, ox, oy) in overlays {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, *ox, *oy);
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextAlign {
    Left,
    Center,
    Right,
}

pub(crate) struct CanvasRect {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) right: f32,
    pub(crate) bottom: f32,
    pub(crate) rx: f32,
    pub(crate) ry: f32,
    pub(crate) color: u32,
}

impl CanvasRect {
    pub(crate) fn sharp(left: f32, top: f32, right: f32, bottom: f32, color: u32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
            rx: 0.0,
            ry: 0.0,
            color,
        }
    }

    pub(crate) fn rounded(
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        r: f32,
        color: u32,
    ) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
            rx: r,
            ry: r,
            color,
        }
    }
}

pub(crate) struct CanvasText {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) size: f32,
    pub(crate) color: u32,
    pub(crate) align: TextAlign,
    pub(crate) bold: bool,
    pub(crate) text: String,
}

impl CanvasText {
    pub(crate) fn new(
        x: f32,
        y: f32,
        size: f32,
        color: u32,
        align: TextAlign,
        bold: bool,
        text: impl Into<String>,
    ) -> Self {
        Self {
            x,
            y,
            size,
            color,
            align,
            bold,
            text: text.into(),
        }
    }
}

/// Helper para renderizar un botón estilo píldora con bordes redondeados y relleno.
#[allow(clippy::too_many_arguments)]
fn draw_button(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    fill_color: u32,
    border_color: u32,
    text_color: u32,
    ts: f32,
    bold: bool,
    label: &str,
) {
    let r = ((bottom - top) * 0.5).max(4.0);
    rects.push(CanvasRect::rounded(
        left,
        top,
        right,
        bottom,
        r,
        border_color,
    ));
    rects.push(CanvasRect::rounded(
        left + 1.0,
        top + 1.0,
        right - 1.0,
        bottom - 1.0,
        (r - 1.0).max(0.0),
        fill_color,
    ));
    let cx = left + (right - left) * 0.5;
    let cy = top + (bottom - top) * 0.5 + ts * 0.35;
    texts.push(CanvasText::new(
        cx,
        cy,
        ts,
        text_color,
        TextAlign::Center,
        bold,
        label,
    ));
}

/// Dibuja rectángulos (rectos o redondeados) y textos (con alineación y negrita)
/// con `android.graphics.Canvas` vía JNI y devuelve el resultado como `Bitmap` RGBA8.
fn jni_text_bitmap(
    w: i32,
    h: i32,
    bg: u32,
    rects: &[CanvasRect],
    texts: &[CanvasText],
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

            // android.graphics.Paint.Align
            let align_class = env.find_class(jni_str!("android/graphics/Paint$Align"))?;
            let align_left = env
                .get_static_field(
                    &align_class,
                    jni_str!("LEFT"),
                    jni_sig!(sig = android.graphics.Paint::Align),
                )?
                .l()?;
            let align_center = env
                .get_static_field(
                    &align_class,
                    jni_str!("CENTER"),
                    jni_sig!(sig = android.graphics.Paint::Align),
                )?
                .l()?;
            let align_right = env
                .get_static_field(
                    &align_class,
                    jni_str!("RIGHT"),
                    jni_sig!(sig = android.graphics.Paint::Align),
                )?
                .l()?;

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

            // Rectángulos.
            for r in rects {
                env.call_method(
                    &paint,
                    jni_str!("setColor"),
                    jni_sig!(sig = (int) -> void),
                    &[JValue::Int(r.color as i32)],
                )?;
                if r.rx > 0.0 || r.ry > 0.0 {
                    env.call_method(
                        &canvas,
                        jni_str!("drawRoundRect"),
                        jni_sig!(sig = (float, float, float, float, float, float, android.graphics.Paint) -> void),
                        &[
                            JValue::Float(r.left),
                            JValue::Float(r.top),
                            JValue::Float(r.right),
                            JValue::Float(r.bottom),
                            JValue::Float(r.rx),
                            JValue::Float(r.ry),
                            JValue::Object(&paint),
                        ],
                    )?;
                } else {
                    env.call_method(
                        &canvas,
                        jni_str!("drawRect"),
                        jni_sig!(sig = (float, float, float, float, android.graphics.Paint) -> void),
                        &[
                            JValue::Float(r.left),
                            JValue::Float(r.top),
                            JValue::Float(r.right),
                            JValue::Float(r.bottom),
                            JValue::Object(&paint),
                        ],
                    )?;
                }
            }

            // Textos.
            for t in texts {
                env.call_method(
                    &paint,
                    jni_str!("setColor"),
                    jni_sig!(sig = (int) -> void),
                    &[JValue::Int(t.color as i32)],
                )?;
                env.call_method(
                    &paint,
                    jni_str!("setTextSize"),
                    jni_sig!(sig = (float) -> void),
                    &[JValue::Float(t.size)],
                )?;
                env.call_method(
                    &paint,
                    jni_str!("setFakeBoldText"),
                    jni_sig!(sig = (boolean) -> void),
                    &[JValue::Bool(t.bold)],
                )?;
                let align_obj = match t.align {
                    TextAlign::Left => &align_left,
                    TextAlign::Center => &align_center,
                    TextAlign::Right => &align_right,
                };
                env.call_method(
                    &paint,
                    jni_str!("setTextAlign"),
                    jni_sig!(sig = (android.graphics.Paint::Align) -> void),
                    &[JValue::Object(align_obj)],
                )?;
                let jstr = env.new_string(t.text.as_str())?;
                env.call_method(
                    &canvas,
                    jni_str!("drawText"),
                    jni_sig!(
                        sig = ("java.lang.String", float, float, android.graphics.Paint) -> void
                    ),
                    &[
                        JValue::Object(jstr.as_ref()),
                        JValue::Float(t.x),
                        JValue::Float(t.y),
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
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("jni_text_bitmap ({w}x{h}): {e}");
            None
        }
    }
}

/// Renderiza el indicador de página "N / total" (overlay abajo a la
/// izquierda, `page_badge_size`): un badge pequeño con el número actual y el
/// total. Cacheado en `Reader::page_badge` (se invalida al cambiar ventana,
/// página o modo oscuro); el tap en él avanza a la página siguiente
/// (`input::page_badge_tap` — decisión documentada: el indicador se puede
/// tocar como acceso rápido a la página siguiente).
pub(crate) fn render_page_badge(reader: &Reader) -> Option<Bitmap> {
    let (bw, bh) = page_badge_size(reader.win_w, reader.win_h);
    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    let label = format!("{} / {}", reader.page + 1, pages);
    let (bg, border, text) = if reader.dark {
        (
            theme::DARK_BADGE_BG,
            theme::DARK_BADGE_BORDER,
            theme::DARK_BADGE_TEXT,
        )
    } else {
        (
            theme::LIGHT_BADGE_BG,
            theme::LIGHT_BADGE_BORDER,
            theme::LIGHT_BADGE_TEXT,
        )
    };
    let mut rects = Vec::new();
    let mut texts = Vec::new();
    let r = 999.0f32;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, bw as f32, bh as f32, r, border,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        bw as f32 - 1.0,
        bh as f32 - 1.0,
        r,
        bg,
    ));
    let ts = 12.0f32;
    texts.push(CanvasText::new(
        bw as f32 / 2.0,
        bh as f32 * 0.5 + ts * 0.35,
        ts,
        text,
        TextAlign::Center,
        true,
        label,
    ));
    jni_text_bitmap(bw, bh, 0x00000000, &rects, &texts)
}

/// Rectángulo de botón (left, top, right, bottom) en px.
pub(crate) type ButtonRect = (f32, f32, f32, f32);

/// Botones del sheet de ajustes del visor (2 filas × 3): fila 1 = Back |
/// Open | Dark/Light (la etiqueta del tercero cambia con el modo); fila 2 =
/// −10 | "N / total" (tap = página siguiente) | +10. La geometría se comparte
/// con `input::sheet_tap` (mismas fórmulas `sheet_*` de `reader`).
pub(crate) fn sheet_buttons(
    reader: &Reader,
    win_w: f32,
    win_h: f32,
) -> Vec<(&'static str, ButtonRect)> {
    let pad = sheet_pad(win_w as i32);
    let bw = sheet_btn_w(win_w as i32);
    let bh = sheet_btn_h(win_h as i32);
    let r1 = sheet_row1_y(win_h as i32);
    let r2 = sheet_row2_y(win_h as i32);
    let mut out = Vec::with_capacity(6);
    let row1: [&'static str; 3] = ["Back", "Open", if reader.dark { "Light" } else { "Dark" }];
    for (i, label) in row1.into_iter().enumerate() {
        let x0 = pad + i as f32 * (bw + pad);
        out.push((label, (x0, r1, x0 + bw, r1 + bh)));
    }
    for (i, label) in ["-10", "N / total", "+10"].into_iter().enumerate() {
        let x0 = pad + i as f32 * (bw + pad);
        out.push((label, (x0, r2, x0 + bw, r2 + bh)));
    }
    out
}

/// Renderiza el sheet de ajustes del visor a un bitmap RGBA8 de tamaño
/// `win_w × sheet_h(win_h)` (la mitad de la ventana): panel deslizante desde
/// el borde superior con Back (biblioteca), Open (picker), Dark/Light, saltos
/// −10/+10 y el indicador de página. Cacheado en `Reader::sheet_bitmap`
/// (invalida al cambiar ventana, página o modo oscuro; se libera al cerrar).
pub(crate) fn render_sheet(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = sheet_h(reader.win_h);
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    let (bar_bg, bar_border, btn_bg, btn_border, btn_text, badge_bg, badge_border, badge_text) =
        if reader.dark {
            (
                theme::DARK_BAR_BG,
                theme::DARK_BAR_BORDER,
                theme::DARK_BTN_BG,
                theme::DARK_BTN_BORDER,
                theme::DARK_BTN_TEXT,
                theme::DARK_BADGE_BG,
                theme::DARK_BADGE_BORDER,
                theme::DARK_BADGE_TEXT,
            )
        } else {
            (
                theme::LIGHT_BAR_BG,
                theme::LIGHT_BAR_BORDER,
                theme::LIGHT_BTN_BG,
                theme::LIGHT_BTN_BORDER,
                theme::LIGHT_BTN_TEXT,
                theme::LIGHT_BADGE_BG,
                theme::LIGHT_BADGE_BORDER,
                theme::LIGHT_BADGE_TEXT,
            )
        };

    // Card deslizable desde arriba: esquinas inferiores redondeadas (16px) y borde de 1px.
    let card_r = 16.0f32;
    rects.push(CanvasRect::rounded(
        0.0, -16.0, w as f32, h as f32, card_r, bar_border,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        -16.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        card_r - 1.0,
        bar_bg,
    ));

    // Etiqueta "SETTINGS" en mayúsculas (11sp) en la esquina superior izquierda.
    let pad = sheet_pad(w);
    texts.push(CanvasText::new(
        pad,
        20.0 + 11.0 * 0.85,
        11.0,
        theme::LIB_TEXT_SECONDARY,
        TextAlign::Left,
        true,
        "SETTINGS",
    ));
    texts.push(CanvasText::new(
        pad,
        h as f32 - 16.0,
        11.0,
        theme::LIB_TEXT_MUTED,
        TextAlign::Left,
        false,
        "Swipe up or tap outside to close",
    ));

    // Botones estilo píldora.
    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    for (label, (l, t, r, b)) in sheet_buttons(reader, w as f32, reader.win_h as f32) {
        let (fill, border, text_color, bold) = match label {
            "Dark" | "Light" => {
                if reader.dark {
                    (
                        theme::ACCENT_AMBER_BG,
                        theme::ACCENT_AMBER_BORDER,
                        0xFF0B0D12,
                        true,
                    )
                } else {
                    (btn_bg, btn_border, btn_text, true)
                }
            }
            "N / total" => (badge_bg, badge_border, badge_text, true),
            _ => (btn_bg, btn_border, btn_text, true),
        };
        let label_str = if label == "N / total" {
            format!("{} / {}", reader.page + 1, pages)
        } else {
            label.to_string()
        };
        draw_button(
            &mut rects,
            &mut texts,
            l,
            t,
            r,
            b,
            fill,
            border,
            text_color,
            (b - t) * 0.38,
            bold,
            &label_str,
        );
    }

    jni_text_bitmap(w, h, 0x00000000, &rects, &texts)
}

/// Renderiza la lista del picker a un bitmap RGBA8 de tamaño de ventana.
pub(crate) fn render_picker_list(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    let row_h = picker_row_h(h);
    let header_h = picker_header_h(h);
    let status_h = if reader.status.is_some() { row_h } else { 0 };
    let btn_w = picker_btn_w(w);
    let btn_h = picker_btn_h(h);
    let pad = (w / 32).max(8) as f32;

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Cabecera + línea divisoria
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        header_h as f32,
        theme::LIB_HEADER_BG,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        header_h as f32 - 1.5,
        w as f32,
        header_h as f32,
        theme::LIB_HEADER_BORDER,
    ));

    let title_ts = row_h as f32 * 0.48;
    texts.push(CanvasText::new(
        pad,
        header_h as f32 * 0.62,
        title_ts,
        theme::LIB_TEXT_PRIMARY,
        TextAlign::Left,
        true,
        "Open PDF",
    ));

    let btn_y = (header_h - btn_h) as f32 / 2.0;
    let back_x = w as f32 - btn_w as f32 * 2.0 - 16.0;
    let rescan_x = w as f32 - btn_w as f32 - 8.0;
    let btn_ts = btn_h as f32 * 0.42;

    if reader.doc.is_some() {
        draw_button(
            &mut rects,
            &mut texts,
            back_x,
            btn_y,
            back_x + btn_w as f32,
            btn_y + btn_h as f32,
            theme::DARK_BTN_BG,
            theme::DARK_BTN_BORDER,
            theme::DARK_BTN_TEXT,
            btn_ts,
            true,
            "Back",
        );
    }
    draw_button(
        &mut rects,
        &mut texts,
        rescan_x,
        btn_y,
        rescan_x + btn_w as f32,
        btn_y + btn_h as f32,
        theme::ACCENT_BLUE_BG,
        theme::ACCENT_BLUE_BORDER,
        0xFFFFFFFF,
        btn_ts,
        true,
        "Rescan",
    );

    // Franja de estado
    let rows_y0 = header_h + status_h;
    if let Some(status) = reader.status.as_deref() {
        rects.push(CanvasRect::sharp(
            0.0,
            header_h as f32,
            w as f32,
            rows_y0 as f32,
            theme::STATUS_BG,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            rows_y0 as f32 - 1.0,
            w as f32,
            rows_y0 as f32,
            theme::STATUS_BORDER,
        ));
        let ts = row_h as f32 * 0.36;
        texts.push(CanvasText::new(
            pad,
            header_h as f32 + row_h as f32 * 0.62,
            ts,
            theme::STATUS_TEXT,
            TextAlign::Left,
            true,
            status,
        ));
    }

    // Filas de PDFs
    let visible = picker_visible_rows(h, reader.status.is_some());
    let row_ts = row_h as f32 * 0.40;
    for i in 0..visible {
        let r = reader.list_scroll + i;
        let Some(entry) = reader.pdf_list.get(r) else {
            break;
        };
        let y0 = (rows_y0 + (i as i32) * row_h) as f32;
        let bg = if i % 2 == 0 {
            theme::LIB_ROW_EVEN
        } else {
            theme::LIB_ROW_ODD
        };
        rects.push(CanvasRect::sharp(0.0, y0, w as f32, y0 + row_h as f32, bg));
        rects.push(CanvasRect::sharp(
            0.0,
            y0 + row_h as f32 - 1.0,
            w as f32,
            y0 + row_h as f32,
            theme::LIB_ROW_BORDER,
        ));

        let size_str = human_size(entry.size);
        let char_w = row_ts * 0.55;
        let size_w = size_str.chars().count() as f32 * char_w + pad;
        let max_chars = (((w as f32 - pad * 3.0 - size_w) / char_w) as usize).max(1);
        let label = format!(
            "📄  {} [{}]",
            truncate_name(&entry.name, max_chars),
            entry.source
        );
        texts.push(CanvasText::new(
            pad,
            y0 + row_h as f32 * 0.64,
            row_ts,
            theme::LIB_TEXT_PRIMARY,
            TextAlign::Left,
            true,
            label,
        ));
        texts.push(CanvasText::new(
            w as f32 - pad,
            y0 + row_h as f32 * 0.64,
            row_ts,
            theme::LIB_TEXT_MUTED,
            TextAlign::Right,
            false,
            size_str,
        ));
    }

    jni_text_bitmap(w, h, theme::LIB_BG, &rects, &texts)
}

/// Botones de la cabecera de la biblioteca.
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
/// rejilla de 3 columnas (`GRID_COLS`) donde cada celda muestra la PORTADA
/// (página 1, perezosa — ver `Reader::thumbs`/`pump_thumbs`) y el título
/// debajo (1-2 líneas truncadas). Las celdas sin portada todavía cargada se
/// pintan con un placeholder (rect gris + "…") que se sustituye cuando la
/// portada llega a la caché (el re-render lo dispara `pump_thumbs`).
///
/// El bitmap base (fondo, cabecera, estado, títulos y placeholders) se
/// dibuja con Canvas+JNI (`jni_text_bitmap`); las portadas CACHEADAS se
/// pegan después directamente sobre sus bytes RGBA (Canvas no pinta
/// bitmaps): escala vecino-más-cercano al ancho del área de portada
/// (`grid_cover_w`), sin pasar por un lock de ventana (el bitmap es nuestro).
pub(crate) fn render_library_grid(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    let header_h = picker_header_h(h);
    let btn_w = picker_btn_w(w);
    let btn_h = picker_btn_h(h);
    let btn_y = (header_h - btn_h) as f32 / 2.0;
    let pad = grid_pad(w);
    let rows_y0 = grid_rows_y0(h, reader.status.is_some());

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // 1. Cabecera + borde inferior (mismo layout que la lista).
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        header_h as f32,
        theme::LIB_HEADER_BG,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        header_h as f32 - 1.5,
        w as f32,
        header_h as f32,
        theme::LIB_HEADER_BORDER,
    ));

    let title_ts = picker_row_h(h) as f32 * 0.48;
    texts.push(CanvasText::new(
        pad,
        header_h as f32 * 0.62,
        title_ts,
        theme::LIB_TEXT_PRIMARY,
        TextAlign::Left,
        true,
        "Library",
    ));

    let btn_ts = btn_h as f32 * 0.42;
    for (label, (l, t, r, b)) in
        library_buttons(reader, w as f32, btn_w as f32, btn_h as f32, btn_y)
    {
        let (fill, border) = if label == "Rescan" {
            (theme::ACCENT_BLUE_BG, theme::ACCENT_BLUE_BORDER)
        } else if label == "Grant" {
            (theme::ACCENT_AMBER_BG, theme::ACCENT_AMBER_BORDER)
        } else {
            (theme::DARK_BTN_BG, theme::DARK_BTN_BORDER)
        };
        draw_button(
            &mut rects, &mut texts, l, t, r, b, fill, border, 0xFFFFFFFF, btn_ts, true, label,
        );
    }

    // 2. Franja de estado.
    if let Some(status) = reader.status.as_deref() {
        rects.push(CanvasRect::sharp(
            0.0,
            header_h as f32,
            w as f32,
            rows_y0 as f32,
            theme::STATUS_BG,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            rows_y0 as f32 - 1.0,
            w as f32,
            rows_y0 as f32,
            theme::STATUS_BORDER,
        ));
        let ts = picker_row_h(h) as f32 * 0.36;
        texts.push(CanvasText::new(
            pad,
            header_h as f32 + picker_row_h(h) as f32 * 0.62,
            ts,
            theme::STATUS_TEXT,
            TextAlign::Left,
            true,
            status,
        ));
    }

    // 3. Celdas visibles (rejilla 3 × N): placeholder de portada + título.
    let cell_w = grid_cell_w(w);
    let cell_h = grid_cell_h(w);
    let cover_w = grid_cover_w(w);
    let cover_h = grid_cover_h(w);
    let title_ts = 14.0f32;
    let char_w = title_ts * 0.55;
    let max_chars = (((cell_w - 2.0 * GRID_CELL_PAD) / char_w) as usize).max(3);
    let visible = grid_visible_rows(w, h, reader.status.is_some());
    for row in 0..visible {
        for col in 0..GRID_COLS {
            let r = reader.list_scroll + row;
            let Some(entry) = reader.grid_entry_at(r, col) else {
                continue;
            };
            let (cx, cy, _, _) = grid_cell_rect(w, rows_y0, row, col);
            let bg = if (row + col) % 2 == 0 {
                theme::LIB_ROW_EVEN
            } else {
                theme::LIB_ROW_ODD
            };
            rects.push(CanvasRect::sharp(cx, cy, cx + cell_w, cy + cell_h, bg));

            // Área de portada: placeholder mientras no hay thumbnail (esquinas redondeadas 12px + borde 1px LIB_ROW_BORDER).
            let cover_x0 = cx + (cell_w - cover_w) / 2.0;
            let cover_y0 = cy + 4.0;
            if reader.thumbs.peek(&entry.uri).is_none() {
                let pr = 12.0f32;
                rects.push(CanvasRect::rounded(
                    cover_x0,
                    cover_y0,
                    cover_x0 + cover_w,
                    cover_y0 + cover_h,
                    pr,
                    theme::LIB_ROW_BORDER,
                ));
                rects.push(CanvasRect::rounded(
                    cover_x0 + 1.0,
                    cover_y0 + 1.0,
                    cover_x0 + cover_w - 1.0,
                    cover_y0 + cover_h - 1.0,
                    (pr - 1.0).max(0.0),
                    theme::LIB_ROW_EVEN,
                ));
                texts.push(CanvasText::new(
                    cover_x0 + cover_w / 2.0,
                    cover_y0 + cover_h / 2.0 + title_ts * 0.35,
                    title_ts * 1.6,
                    theme::LIB_TEXT_MUTED,
                    TextAlign::Center,
                    true,
                    "…",
                ));
            }

            // Título DEBAJO de la portada: 14sp, 1 línea con puntos suspensivos, LIB_TEXT_SECONDARY.
            let title_text = truncate_name(&entry.name, max_chars);
            texts.push(CanvasText::new(
                cx + GRID_CELL_PAD,
                cy + cover_h + 6.0 + title_ts * 0.85,
                title_ts,
                theme::LIB_TEXT_SECONDARY,
                TextAlign::Left,
                false,
                title_text,
            ));
        }
    }

    let mut out = jni_text_bitmap(w, h, theme::LIB_BG, &rects, &texts)?;

    // 4. Pegar las portadas CACHEADAS sobre el bitmap base (scale-to-fill, redondeadas 12px, borde 1px LIB_ROW_BORDER).
    for row in 0..visible {
        for col in 0..GRID_COLS {
            let r = reader.list_scroll + row;
            let Some(entry) = reader.grid_entry_at(r, col) else {
                continue;
            };
            let Some(thumb) = reader.thumbs.peek(&entry.uri) else {
                continue;
            };
            let (cx, cy, _, _) = grid_cell_rect(w, rows_y0, row, col);
            let cover_x0 = (cx + (cell_w - cover_w) / 2.0).round() as i32;
            let cover_y0 = (cy + 4.0).round() as i32;
            paste_thumb(
                &mut out.data,
                out.width as usize,
                thumb,
                cover_x0,
                cover_y0,
                cover_w as i32,
                cover_h as i32,
            );
        }
    }

    Some(out)
}

/// Pega la portada escalada (scale-to-fill, vecino-más-cercano) dentro del bitmap
/// base de la rejilla, ajustando el crop al área de la portada con esquinas
/// redondeadas de 12px y borde de 1px `LIB_ROW_BORDER`.
fn paste_thumb(
    dst: &mut [u8],
    dst_w: usize,
    thumb: &Bitmap,
    dx: i32,
    dy: i32,
    target_w: i32,
    target_h: i32,
) {
    let dst_h = dst.len() / (dst_w * 4);
    if dst_w == 0 || dst_h == 0 || target_w <= 0 || target_h <= 0 {
        return;
    }
    let src_w = thumb.width as i64;
    let src_h = thumb.height as i64;
    if src_w <= 0 || src_h <= 0 {
        return;
    }

    // Scale-to-fill (sin letterbox)
    let scale = (target_w as f64 / src_w as f64).max(target_h as f64 / src_h as f64);
    let dw = (src_w as f64 * scale).round() as i64;
    let dh = (src_h as f64 * scale).round() as i64;
    if dw <= 0 || dh <= 0 {
        return;
    }
    let offset_x = (dw - target_w as i64) / 2;
    let offset_y = (dh - target_h as i64) / 2;

    let border_color: [u8; 4] = [0x1E, 0x25, 0x30, 0xFF]; // theme::LIB_ROW_BORDER
    let r = 12.0f32;
    let r_sq = r * r;
    let inner_r_sq = (r - 1.0) * (r - 1.0);

    let tw_f = target_w as f32;
    let th_f = target_h as f32;

    for ty in 0..target_h {
        let py = dy + ty;
        if py < 0 || py as usize >= dst_h {
            continue;
        }
        let src_y = (((ty as i64 + offset_y) * src_h) / dh).clamp(0, src_h - 1) as usize;
        let srow = &thumb.data[src_y * src_w as usize * 4..];

        for tx in 0..target_w {
            let px = dx + tx;
            if px < 0 || px as usize >= dst_w {
                continue;
            }

            let fx = tx as f32 + 0.5;
            let fy = ty as f32 + 0.5;

            let (is_corner, dist_sq) = if fx < r && fy < r {
                let cdx = r - fx;
                let cdy = r - fy;
                (true, cdx * cdx + cdy * cdy)
            } else if fx > tw_f - r && fy < r {
                let cdx = fx - (tw_f - r);
                let cdy = r - fy;
                (true, cdx * cdx + cdy * cdy)
            } else if fx < r && fy > th_f - r {
                let cdx = r - fx;
                let cdy = fy - (th_f - r);
                (true, cdx * cdx + cdy * cdy)
            } else if fx > tw_f - r && fy > th_f - r {
                let cdx = fx - (tw_f - r);
                let cdy = fy - (th_f - r);
                (true, cdx * cdx + cdy * cdy)
            } else {
                (false, 0.0)
            };

            let is_edge = tx == 0 || tx == target_w - 1 || ty == 0 || ty == target_h - 1;

            if is_corner && dist_sq > r_sq {
                continue;
            }

            let d_offset = (py as usize * dst_w + px as usize) * 4;

            if (is_corner && dist_sq >= inner_r_sq) || (!is_corner && is_edge) {
                dst[d_offset..d_offset + 4].copy_from_slice(&border_color);
            } else {
                let src_x = (((tx as i64 + offset_x) * src_w) / dw).clamp(0, src_w - 1) as usize;
                dst[d_offset..d_offset + 4].copy_from_slice(&srow[src_x * 4..src_x * 4 + 4]);
            }
        }
    }
}
