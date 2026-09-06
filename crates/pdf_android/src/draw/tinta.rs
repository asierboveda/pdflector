// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Rasterizador de la tinta: tramo en vivo (`draw_ink_segment_on_frame`)
//! y polilínea (`draw_polyline_pub`) con brocha de disco, recorte
//! Liang–Barsky y Bresenham — el MISMO rasterizador para trazos vivos y
//! guardados.

use pdf_core::Bitmap;

use super::{
    primitives::rgb565,
};

/// Pinta UN segmento de tinta (tramo incremental en vivo, patrón
/// Xournal++/GoodNotes) directamente sobre un bitmap RGBA del tamaño de la
/// ventana (`frame`): transforma los dos puntos de página a pantalla con
/// `scale/dx/dy` (el MISMO rasterizador que los trazos guardados, para que
/// la tinta en vivo sea idéntica a la definitiva) y dibuja el tramo con
/// brocha redondeada. Devuelve el bbox del tramo en px de ventana (dirty
/// rect para el blit).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_ink_segment_on_frame(
    frame: &mut Bitmap,
    p0: (f32, f32),
    p1: (f32, f32),
    width_pt: f32,
    color: pdf_core::Color,
    scale: f32,
    dx: i32,
    dy: i32,
) -> Option<(i32, i32, i32, i32)> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let a = (p0.0 * scale + dx as f32, p0.1 * scale + dy as f32);
    let b = (p1.0 * scale + dx as f32, p1.1 * scale + dy as f32);
    let width_px = (width_pt * scale).max(1.0);
    let pad = (width_px / 2.0).ceil() + 1.0;
    let pts = [a, b];
    let mut disc = [(0i32, 0i32); INK_DISC_CAP];
    // Brocha de 1 px si la capacidad no alcanza (escala extrema).
    let dn = ink_disc_for(width_px, &mut disc).unwrap_or(1);
    let dst = frame.data.as_mut_ptr();
    draw_polyline(
        dst,
        w,
        h,
        w,
        4,
        &pts,
        &disc[..dn],
        [color.r, color.g, color.b, color.a],
    );
    let (x0, y0) = (a.0.min(b.0) - pad, a.1.min(b.1) - pad);
    let (x1, y1) = (a.0.max(b.0) + pad, a.1.max(b.1) + pad);
    Some((
        x0.floor().max(0.0) as i32,
        y0.floor().max(0.0) as i32,
        x1.ceil().min(w as f32) as i32,
        y1.ceil().min(h as f32) as i32,
    ))
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
/// Radio de una brocha (los offsets se generan ordenados por fila desde −r,
/// así que el primer offset lleva `ox = −r`; brocha vacía → radio 0).
fn disc_r(disc: &[(i32, i32)]) -> i32 {
    disc.first().map_or(0, |&(ox, _)| -ox)
}

/// Capacidad de offsets del disco de radio `r` (peor caso, disco completo).
pub(crate) const INK_DISC_CAP: usize = 160;

fn disc_cap(r: i32) -> usize {
    ((std::f32::consts::PI * (r as f32 + 0.5).powi(2)).ceil() as usize).max(1)
}

/// Construye la brocha circular de radio `r` px (offsets del disco, centrada
/// en el origen) en el buffer FIJO `out`. Devuelve el número de offsets
/// escritos. Sin allocs: los callers usan un array en stack
/// (`[(i32, i32); INK_DISC_CAP]` cubre radio ≤ 7 px, el preset más grueso a
/// escala ~2 px/pt). Radio ≤ 1 → solo el centro (trazo fino = 1 px).
fn ink_disc(r: i32, out: &mut [(i32, i32)]) -> usize {
    if r <= 1 {
        out[0] = (0, 0);
        1
    } else {
        let r2 = (r as i64) * (r as i64);
        let mut k = 0usize;
        for oy in -r..=r {
            for ox in -r..=r {
                if (ox as i64) * (ox as i64) + (oy as i64) * (oy as i64) <= r2 {
                    out[k] = (ox, oy);
                    k += 1;
                }
            }
        }
        k
    }
}

/// Rellena `disc` con la brocha de tinta para un ancho `width_px` (radio
/// `⌈width_px/2⌉`, mínimo 1). Devuelve el nº de offsets escritos, o `None`
/// si la capacidad no alcanza (escala extrema: el caller degrada a brocha de
/// 1 px — nunca pánico).
pub(crate) fn ink_disc_for(width_px: f32, disc: &mut [(i32, i32)]) -> Option<usize> {
    let r = ((width_px / 2.0).ceil().max(1.0)) as i32;
    if disc.len() < disc_cap(r) {
        return None;
    }
    Some(ink_disc(r, disc))
}

#[allow(clippy::too_many_arguments)]
fn draw_polyline(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    pts: &[(f32, f32)],
    disc: &[(i32, i32)],
    color: [u8; 4],
) {
    draw_polyline_pub(dst, dst_w, dst_h, dst_stride, bpp, pts, disc, color)
}

/// Envoltorio público interno de [`draw_polyline`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_polyline_pub(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    pts: &[(f32, f32)],
    disc: &[(i32, i32)],
    color: [u8; 4],
) {
    if pts.len() < 2 || dst_w == 0 || dst_h == 0 {
        return;
    }
    // Brocha PRECALCULADA por el llamador (offsets del disco; ver `ink_disc`):
    // sin allocs por llamada. Radio 1 (grosor ≤ 2 px, el caso normal) es un
    // solo píxel por punto de línea.
    let r = disc_r(disc);
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
                |x, y| stamp(dst, dst_w, dst_h, dst_stride, bpp, x, y, disc, color),
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
