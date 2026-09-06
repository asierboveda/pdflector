// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Primitivas de blit a bajo nivel sobre el buffer de ventana
//! (`fill_buffer`, `copy_region_rect`, `copy_region`, `copy_region_blend`
//! con alfa y conversión RGB565). Puntero + dimensión, patrón de `zoom.rs`.

use pdf_core::Bitmap;

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
            for (out, px) in dst
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                out.copy_from_slice(&rgb565(px[0], px[1], px[2]).to_ne_bytes());
            }
        }
        _ => {
            let n = bpp.min(3);
            for (out, px) in dst.chunks_exact_mut(bpp).zip(src.as_chunks::<4>().0) {
                out[..n].copy_from_slice(&px[..n]);
            }
        }
    }
}

/// Conversión RGBA8 → RGB565 (formato `R5G6B5_UNORM` de Android, u16 little-endian).
pub(super) fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// Copia la intersección de `src` (bitmap RGBA8) con la ventana del buffer
/// `dst` (formato `bpp` bytes/px), con la esquina superior-izquierda de `src`
/// en `(sx, sy)` px del buffer. Recorta los bordes fuera del buffer (zoom > 1,
/// pan o botones en los bordes).
//
// 8 parámetros posicionales de un blit (raw pointer + dimensiones): se acepta
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn copy_region_rect(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    src: &Bitmap,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) {
    debug_assert_eq!(
        src.width as usize, dst_w,
        "dirty rect exige bitmap del mismo tamaño"
    );
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(dst_w as i32).min(src.width as i32);
    let y1 = y1.min(dst_h as i32).min(src.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let copy_w = (x1 - x0) as usize;
    for y in y0..y1 {
        let row_off = (y as usize * src.width as usize + x0 as usize) * 4;
        let src_row = &src.data[row_off..row_off + copy_w * 4];
        let dst_row = unsafe {
            std::slice::from_raw_parts_mut(
                dst.add((y as usize * dst_stride + x0 as usize) * bpp),
                copy_w * bpp,
            )
        };
        if bpp == 4 {
            dst_row.copy_from_slice(&src_row[..copy_w * 4]);
        } else {
            let n = bpp.min(3);
            for (o, px) in src_row.as_chunks::<4>().0.iter().enumerate() {
                dst_row[o * bpp..o * bpp + n].copy_from_slice(&px[..n]);
            }
        }
    }
}

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

/// Copia un bitmap overlay a `(sx, sy)` respetando su canal ALPHA
/// (source-over por píxel): los píxeles con alfa 255 se copian directo (sin
/// blend), los de alfa 0 se saltan y los intermedios se funden sobre el
/// destino. Se usa para la capa temporal de anotación en curso, cuyo bitmap
/// es RGBA con fondo transparente y tinta translúcida (los overlays opacos
/// existentes —sheet, menú, aviso— se copian con `copy_region` directo).
///
/// Coste O(área del overlay) por píxel con blend — ∝ el bbox del trazo en
/// curso (típicamente un trozo de página), el presupuesto del requisito 5
/// (no re-blitear la página en cada Move del dedo).
//
// 8 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `copy_region`; se acepta el allow.
#[allow(clippy::too_many_arguments)]
pub(super) fn copy_region_blend(
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
        if bpp == 4 {
            for (i, px) in src_row.as_chunks::<4>().0.iter().enumerate() {
                let a = px[3];
                if a == 0 {
                    continue;
                }
                let o = i * 4;
                if a == 255 {
                    dst_row[o..o + 4].copy_from_slice(px);
                } else {
                    let inv = (255 - a) as u32;
                    for c in 0..3 {
                        dst_row[o + c] =
                            ((px[c] as u32 * a as u32 + dst_row[o + c] as u32 * inv) / 255) as u8;
                    }
                }
            }
        } else {
            // bpp != 4 (raro: el buffer se fuerza a RGBA): copia directa.
            let n = bpp.min(3);
            for (i, px) in src_row.as_chunks::<4>().0.iter().enumerate() {
                let o = i * bpp;
                dst_row[o..o + n].copy_from_slice(&px[..n]);
            }
        }
    }
}
