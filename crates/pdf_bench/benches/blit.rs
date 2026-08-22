// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Blit benchmark — per-frame drawing path of the Android viewer
//! (`pdf_android/src/draw.rs`).
//!
//! Why a mirror: `pdf_android` cannot build on host (`android-activity` is
//! Android-only), so the CPU-only blit primitives below are FAITHFUL COPIES
//! of `draw.rs` (`fill_buffer`, `rgb565`, `copy_row_rgba_to`, `copy_region`,
//! `blit_page_scaled`, `fill_rect_lut`, `fill_rect_bordered`,
//! `draw_sel_rect`, `compose_frame`, `blit_composed`). **Keep them in sync**
//! when optimizing `draw.rs` and re-run this bench to prove the change.
//!
//! What it measures — the per-frame cost in the viewer at a typical tablet
//! resolution (2000×1200 landscape):
//!
//!   blit/page_1to1_*          — `blit_page_scaled` at zoom 1.0 (the resting
//!                               path: scroll steps, page turns, selection
//!                               drag re-blits). bpp 4 light/dark and bpp 2
//!                               (RGB565 conversion; rare since the buffer is
//!                               forced to R8G8B8A8_UNORM).
//!   blit/page_zoom_*          — same page scaled by vecino-más-cercano at
//!                               zoom 1.35 (a pinch-gesture frame, no
//!                               re-render): the precomputed x-map path.
//!   blit/compose_frame_2k1k   — full frame composition (background fill +
//!                               page blit + selection rect + page badge)
//!                               into an RGBA8 bitmap: paid ONCE per sheet
//!                               open/slide.
//!   blit/blit_composed_2k1k   — the per-frame copy of the composed frame
//!                               into a window buffer + badge overlay
//!                               (memcpy path, sheet animation).
//!   blit/fill_buffer_2k1k     — letterbox background fill alone.
//!
//! The annotation layer (`draw_annotations`, Bresenham strokes) is NOT
//! mirrored: its cost is ∝ visible stroke points, not page area, and is the
//! subject of the `annotations` bench's acceptance criterion (200 strokes).
//!
//! Source content: real MuPDF renders of `large_document.pdf` page 0 at the
//! viewer's cover scale for 2000×1200 (≈849×1200 px) — same memory traffic a
//! real frame pays.
//!
//! Run with:
//!     cargo bench -p pdf_bench --bench blit
//! Results land in target/criterion/<group>/report/.

use std::path::PathBuf;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use pdf_core::Bitmap;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine, corpus_dir};

// ---------------------------------------------------------------------------
// MIRROR of pdf_android/src/draw.rs (CPU-only paths). Keep in sync with the
// real file — every optimization here must be applied there too.
// ---------------------------------------------------------------------------

fn fill_buffer(dst: *mut u8, w: usize, h: usize, stride: usize, bpp: usize, color: [u8; 4]) {
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

fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

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

#[allow(clippy::too_many_arguments)]
fn copy_region(
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
        copy_row_rgba_to(dst_row, src_row, bpp);
    }
}

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
    if src_w <= 0 || src_h <= 0 || !zoom.is_finite() || zoom <= 0.0 {
        return;
    }
    let dw = (src_w as f64 * zoom as f64).round() as i64;
    let dh = (src_h as f64 * zoom as f64).round() as i64;
    if dw <= 0 || dh <= 0 {
        return;
    }
    let x0 = i64::from(dx.max(0));
    let y0 = i64::from(dy.max(0));
    let x1 = (i64::from(dx) + dw).min(dst_w as i64);
    let y1 = (i64::from(dy) + dh).min(dst_h as i64);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let vis_w = (x1 - x0) as usize;
    let src_row_bytes = src_w as usize * 4;
    if zoom == 1.0 {
        let src_ox = (x0 - i64::from(dx)) as usize;
        for y in 0..(y1 - y0) as usize {
            let dst_y = y0 + y as i64;
            let src_y = (dst_y - i64::from(dy)) as usize;
            let src_row =
                &src.data[src_y * src_row_bytes + src_ox * 4..(src_y + 1) * src_row_bytes];
            let dst_row = unsafe {
                std::slice::from_raw_parts_mut(
                    dst.add((dst_y as usize * dst_stride + x0 as usize) * bpp),
                    vis_w * bpp,
                )
            };
            match bpp {
                4 => {
                    if dark {
                        // XOR de u32 = inversión RGB (255 − v) sin tocar el
                        // alfa (0x00FF_FFFF excluye el byte 3). Espejo
                        // exacto de `draw.rs`; ver allí la justificación.
                        let src32 = unsafe {
                            std::slice::from_raw_parts(src_row.as_ptr() as *const u32, vis_w)
                        };
                        let dst32 = unsafe {
                            std::slice::from_raw_parts_mut(dst_row.as_mut_ptr() as *mut u32, vis_w)
                        };
                        for x in 0..vis_w {
                            dst32[x] = src32[x] ^ 0x00FF_FFFF;
                        }
                    } else {
                        dst_row.copy_from_slice(&src_row[..vis_w * 4]);
                    }
                }
                2 => {
                    for x in 0..vis_w {
                        let o = x * 4;
                        let (r, g, b) = if dark {
                            (255 - src_row[o], 255 - src_row[o + 1], 255 - src_row[o + 2])
                        } else {
                            (src_row[o], src_row[o + 1], src_row[o + 2])
                        };
                        dst_row[x * 2..x * 2 + 2].copy_from_slice(&rgb565(r, g, b).to_ne_bytes());
                    }
                }
                _ => {
                    let n = bpp.min(3);
                    for x in 0..vis_w {
                        let o = x * 4;
                        if dark {
                            for c in 0..n {
                                dst_row[x * bpp + c] = 255 - src_row[o + c];
                            }
                        } else {
                            dst_row[x * bpp..x * bpp + n].copy_from_slice(&src_row[o..o + n]);
                        }
                    }
                }
            }
        }
        return;
    }
    let x_map: Vec<usize> = (0..vis_w)
        .map(|x| (((x0 - i64::from(dx) + x as i64) * src_w) / dw) as usize)
        .collect();
    for y in 0..(y1 - y0) as usize {
        let dst_y = y0 + y as i64;
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
                // Mapeo x como accesos por u32 (espejo de `draw.rs`):
                // `x_map[x]` ∈ [0, src_w) por construcción (división entera
                // truncada), lecturas directas sin bounds-check por píxel.
                let src32 = src_row.as_ptr() as *const u32;
                let dst32 = dst_row.as_mut_ptr() as *mut u32;
                if dark {
                    for (x, &src_x) in x_map.iter().enumerate() {
                        unsafe { *dst32.add(x) = *src32.add(src_x) ^ 0x00FF_FFFF }
                    }
                } else {
                    for (x, &src_x) in x_map.iter().enumerate() {
                        unsafe { *dst32.add(x) = *src32.add(src_x) }
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

#[allow(clippy::too_many_arguments)]
fn fill_rect_lut(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: [u8; 4],
    shrink: i32,
) {
    let x0 = (x0 + shrink).max(0);
    let y0 = (y0 + shrink).max(0);
    let x1 = (x1 - shrink).min(dst_w as i32);
    let y1 = (y1 - shrink).min(dst_h as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let w = (x1 - x0) as usize;
    let a = color[3] as usize;
    if a == 255 {
        let px = u32::from_ne_bytes(color);
        for y in y0..y1 {
            let row = unsafe {
                std::slice::from_raw_parts_mut(
                    dst.add((y as usize * dst_stride + x0 as usize) * 4) as *mut u32,
                    w,
                )
            };
            row.fill(px);
        }
        return;
    }
    if a == 0 {
        return;
    }
    let t_src: [u8; 256] = std::array::from_fn(|c| ((c as u32 * a as u32) / 255) as u8);
    let t_dst: [u8; 256] = std::array::from_fn(|c| ((c as u32 * (255 - a) as u32) / 255) as u8);
    let (s0, s1, s2) = (
        t_src[color[0] as usize],
        t_src[color[1] as usize],
        t_src[color[2] as usize],
    );
    for y in y0..y1 {
        let row = unsafe {
            std::slice::from_raw_parts_mut(
                dst.add((y as usize * dst_stride + x0 as usize) * 4),
                w * 4,
            )
        };
        for px in row.chunks_exact_mut(4) {
            px[0] = s0 + t_dst[px[0] as usize];
            px[1] = s1 + t_dst[px[1] as usize];
            px[2] = s2 + t_dst[px[2] as usize];
        }
    }
}

const SEL_FILL: [u8; 4] = [0x4D, 0xA3, 0xFF, 0x4D];
const SEL_BORDER: [u8; 4] = [0x4D, 0xA3, 0xFF, 0xFF];

#[allow(clippy::too_many_arguments)]
fn draw_sel_rect(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    l: f32,
    t: f32,
    r: f32,
    b: f32,
) {
    // fill_rect_bordered with border, bpp 4 only (bench scope; draw.rs also
    // handles bpp 2/others via the slow per-pixel path, not hit in practice).
    let ix0 = l.floor().max(0.0) as i32;
    let iy0 = t.floor().max(0.0) as i32;
    let ix1 = r.ceil().min(dst_w as f32) as i32;
    let iy1 = b.ceil().min(dst_h as f32) as i32;
    if ix1 <= ix0 || iy1 <= iy0 {
        return;
    }
    const BORDER_W: i32 = 2;
    if bpp == 4 {
        fill_rect_lut(
            dst, dst_w, dst_h, dst_stride, ix0, iy0, ix1, iy1, SEL_FILL, BORDER_W,
        );
        fill_rect_lut(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            ix0,
            iy0,
            ix1,
            iy0 + BORDER_W,
            SEL_BORDER,
            0,
        );
        fill_rect_lut(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            ix0,
            iy1 - BORDER_W,
            ix1,
            iy1,
            SEL_BORDER,
            0,
        );
        fill_rect_lut(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            ix0,
            iy0,
            ix0 + BORDER_W,
            iy1,
            SEL_BORDER,
            0,
        );
        fill_rect_lut(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            ix1 - BORDER_W,
            iy0,
            ix1,
            iy1,
            SEL_BORDER,
            0,
        );
    }
}

// `blit_page`'s PageBlit / blit_composed pieces (see draw.rs for the exact
// geometry contracts).
struct PageBlit<'a> {
    bitmap: &'a Bitmap,
    dx: i32,
    dy: i32,
    zoom: f32,
}

#[allow(clippy::too_many_arguments)]
fn compose_frame(
    w: i32,
    h: i32,
    bg: [u8; 4],
    dark: bool,
    page: Option<&PageBlit>,
    sel: Option<(f32, f32, f32, f32)>,
    badge: Option<(&Bitmap, i32, i32)>,
) -> Bitmap {
    let (fw, fh) = (w.max(0) as usize, h.max(0) as usize);
    let mut frame = Bitmap {
        width: fw as u32,
        height: fh as u32,
        data: vec![0u8; fw * fh * 4],
    };
    let dst = frame.data.as_mut_ptr();
    fill_buffer(dst, fw, fh, fw, 4, bg);
    if let Some(page) = page {
        blit_page_scaled(
            dst,
            fw,
            fh,
            fw,
            4,
            page.bitmap,
            page.dx,
            page.dy,
            page.zoom,
            dark,
        );
        if let Some((l, t, r, b)) = sel {
            draw_sel_rect(dst, fw, fh, fw, 4, l, t, r, b);
        }
    }
    if let Some((b, bx, by)) = badge {
        copy_region(dst, fw, fh, fw, 4, b, bx, by);
    }
    frame
}

#[allow(clippy::too_many_arguments)]
fn blit_composed_to_buffer(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    frame: &Bitmap,
    overlays: &[(&Bitmap, i32, i32)],
) {
    copy_region(dst, dst_w, dst_h, dst_stride, bpp, frame, 0, 0);
    for (ov, ox, oy) in overlays {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, *ox, *oy);
    }
}

// ---------------------------------------------------------------------------
// Benchmark harness
// ---------------------------------------------------------------------------

/// Landscape window typical of a tablet in landscape (the target of this
/// bench; the TCL target is 1440×2200 portrait — same order of magnitude).
const WIN_W: usize = 2000;
const WIN_H: usize = 1200;
const BPP4: usize = 4;
const BPP2: usize = 2;
/// Pinch-gesture relative zoom (bitmap scaled by vecino-más-cercano, no
/// re-render): a frame right before the crisp re-render lands.
const GESTURE_ZOOM: f32 = 1.35;

fn large_path() -> PathBuf {
    corpus_dir().join("large_document.pdf")
}

fn build_engine() -> MupdfEngine {
    MupdfEngine::new().expect("failed to init mupdf")
}

/// The page bitmap the viewer would blit at rest: MuPDF render at the cover
/// scale for 2000×1200 (A4 → ≈849×1200 px, 4 MiB).
fn page_bitmap_cover() -> Bitmap {
    let engine = build_engine();
    let doc = engine.open(&large_path()).expect("open document");
    let scale = (WIN_H as f32) / 842.0; // A4 height @72dpi ≈ 842 px
    doc.render_page(0, scale).expect("render cover page")
}

/// A small overlay bitmap (page badge):
fn badge_bitmap() -> Bitmap {
    Bitmap {
        width: 64,
        height: 32,
        data: vec![0xFF; 64 * 32 * 4],
    }
}

fn bench_blit(c: &mut Criterion) {
    let page = page_bitmap_cover();
    let badge = badge_bitmap();
    // Letterbox background of the viewer (light theme).
    let bg: [u8; 4] = [0xF2, 0xF2, 0xF2, 0xFF];
    // Centered position of the page at rest (cover): dx = (WIN_W - w)/2.
    let dx = ((WIN_W as f32 - page.width as f32) / 2.0).round() as i32;
    let dy = 0i32;

    // A window-sized buffer with a realistic window stride (width, no pad —
    // tight; ANativeWindow may pad, only addressing arithmetic differs).
    let mut win_buf = vec![0u8; WIN_W * WIN_H * BPP4];
    let mut win_buf2 = vec![0u8; WIN_W * WIN_H * BPP2];

    let mut g = c.benchmark_group("blit");
    g.throughput(criterion::Throughput::Bytes((WIN_W * WIN_H * BPP4) as u64));

    // --- 1:1 resting blit (bpp 4, light) ---
    g.bench_function("page_1to1_bpp4_light_2k1k", |b| {
        b.iter(|| {
            let dst = win_buf.as_mut_ptr();
            blit_page_scaled(dst, WIN_W, WIN_H, WIN_W, BPP4, &page, dx, dy, 1.0, false);
            black_box(win_buf.iter().copied().sum::<u8>());
        });
    });
    // --- 1:1 resting blit (bpp 4, dark: per-pixel RGB inversion) ---
    g.bench_function("page_1to1_bpp4_dark_2k1k", |b| {
        b.iter(|| {
            blit_page_scaled(
                win_buf.as_mut_ptr(),
                WIN_W,
                WIN_H,
                WIN_W,
                BPP4,
                &page,
                dx,
                dy,
                1.0,
                true,
            );
            black_box(win_buf[0]);
        });
    });
    // --- 1:1 resting blit (bpp 2 → RGB565 conversion) ---
    g.bench_function("page_1to1_bpp2_2k1k", |b| {
        b.iter(|| {
            blit_page_scaled(
                win_buf2.as_mut_ptr(),
                WIN_W,
                WIN_H,
                WIN_W,
                BPP2,
                &page,
                dx,
                dy,
                1.0,
                false,
            );
            black_box(win_buf2[0]);
        });
    });
    // --- pinch frame (zoom 1.35, bpp 4, light): x-map + nearest-neighbor ---
    g.bench_function("page_zoom135_bpp4_light_2k1k", |b| {
        b.iter(|| {
            blit_page_scaled(
                win_buf.as_mut_ptr(),
                WIN_W,
                WIN_H,
                WIN_W,
                BPP4,
                &page,
                dx,
                dy,
                GESTURE_ZOOM,
                false,
            );
            black_box(win_buf.iter().copied().sum::<u8>());
        });
    });
    // --- pinch frame (zoom 1.35, bpp 4, dark) ---
    g.bench_function("page_zoom135_bpp4_dark_2k1k", |b| {
        b.iter(|| {
            blit_page_scaled(
                win_buf.as_mut_ptr(),
                WIN_W,
                WIN_H,
                WIN_W,
                BPP4,
                &page,
                dx,
                dy,
                GESTURE_ZOOM,
                true,
            );
            black_box(win_buf[0]);
        });
    });
    // --- background letterbox fill alone ---
    g.bench_function("fill_buffer_2k1k", |b| {
        b.iter(|| {
            fill_buffer(win_buf.as_mut_ptr(), WIN_W, WIN_H, WIN_W, BPP4, bg);
            black_box(win_buf[0]);
        });
    });
    g.finish();

    // --- full frame composition (sheet path: paid once) ---
    let pb = PageBlit {
        bitmap: &page,
        dx,
        dy,
        zoom: 1.0,
    };
    // Selection rect over the page (the compose called during sheet slide).
    let sel = (dx as f32 + 100.0, 200.0_f32, dx as f32 + 700.0, 900.0_f32);
    let badge_pos = (0i32, (WIN_H as i32) - 40);
    let mut g = c.benchmark_group("blit");
    g.throughput(criterion::Throughput::Bytes((WIN_W * WIN_H * BPP4) as u64));
    g.bench_function("compose_frame_2k1k", |b| {
        b.iter(|| {
            let frame = compose_frame(
                WIN_W as i32,
                WIN_H as i32,
                bg,
                false,
                Some(&pb),
                Some(sel),
                Some((&badge, badge_pos.0, badge_pos.1)),
            );
            black_box(frame.data.iter().copied().sum::<u8>());
        });
    });
    // --- per-frame copy of the composed frame + badge overlay (sheet anim) ---
    let frame = compose_frame(
        WIN_W as i32,
        WIN_H as i32,
        bg,
        false,
        Some(&pb),
        Some(sel),
        Some((&badge, badge_pos.0, badge_pos.1)),
    );
    let overlays = [(&badge, badge_pos.0, badge_pos.1)];
    g.bench_function("blit_composed_2k1k", |b| {
        b.iter(|| {
            blit_composed_to_buffer(
                win_buf.as_mut_ptr(),
                WIN_W,
                WIN_H,
                WIN_W,
                BPP4,
                &frame,
                &overlays,
            );
            black_box(win_buf.iter().copied().sum::<u8>());
        });
    });
    g.finish();
}

fn benches(c: &mut Criterion) {
    bench_blit(c);
}

criterion_group!(all, benches);
criterion_main!(all);
