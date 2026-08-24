// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

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
use pdf_core::{Annotated, Bitmap, Document, Highlight, Stroke, ViewTransform};

use crate::annotations::ToolKind;
use crate::persist;
use crate::reader::{
    AiPhase, GRID_CELL_PAD, GRID_COLS, Reader, entry_author, entry_title, grid_cell_h,
    grid_cell_rect, grid_cell_w, grid_cover_h, grid_cover_w, grid_pad, human_size, lib_chip_h,
    lib_chips, lib_cont_block_h, lib_cont_card_h, lib_cont_card_w, lib_cont_card_x,
    lib_cont_cover_h, lib_cont_cover_w, lib_content_y0, lib_empty_state_geom, lib_grid_y0,
    lib_header_h, lib_org_chip_h, lib_org_chips, lib_org_y, lib_search_h, lib_section_title_h,
    page_badge_size, picker_btn_h, picker_btn_w, picker_header_h, picker_row_h,
    picker_visible_rows, sheet_btn_h, sheet_btn_w, sheet_h, sheet_pad, sheet_row1_y, sheet_row2_y,
    title_from_name, truncate_name,
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
/// Copia un RECTÁNGULO de un bitmap RGBA (del mismo tamaño que la ventana) a
/// la MISMA posición del buffer destino — la versión "dirty rect" de
/// [`copy_region`] para el gesto: no toca los ~12 MB del resto del frame por
/// Move del boli (Fase C, comparativa saber-notes: la copia completa es el
/// coste dominante del blit por evento).
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
            for (o, px) in src_row.chunks_exact(4).enumerate() {
                dst_row[o * bpp..o * bpp + n].copy_from_slice(&px[..n]);
            }
        }
    }
}

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

/// Una página a blitear en el buffer (modo UNA HOJA: solo la página actual,
/// nunca una columna): bitmap cacheado + esquina superior izquierda en px de
/// ventana + zoom RELATIVO al render (1.0 = blit 1:1 nítido; > 1 durante el
/// pinch sin re-render, escala vecino-más-cercano del bitmap cacheado). Su
/// capa de anotaciones se pasa aparte (`PageAnnots`).
pub(crate) struct PageBlit<'a> {
    pub(crate) bitmap: &'a Bitmap,
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    pub(crate) zoom: f32,
}

/// Capa de anotaciones de una página visible: trazos y highlights guardados
/// (coordenadas de página, puntos PDF) + transformación página→ventana
/// (`× scale` + `(dx, dy)`), con `dx`/`dy`/`scale` EXACTAMENTE los del blit
/// de la página (`PageBlit`). Se dibuja como capa vectorial SOBRE el bitmap
/// ya bliteado — nunca se rasteriza dentro del bitmap cacheado (AGENTS.md
/// §4.3): así la anotación permanece nítida a cualquier zoom y el coste del
/// render es ∝ anotaciones visibles, no ∝ área de página. Los highlights se
/// dibujan DEBAJO de los trazos (orden de dibujo: rellenos primero).
pub(crate) struct PageAnnots<'a> {
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    /// px de ventana por punto PDF (cover × zoom).
    pub(crate) scale: f32,
    /// Highlights guardados de la página (subrayados, relleno translúcido).
    pub(crate) highlights: Vec<&'a Highlight>,
    /// Trazos guardados de la página, en orden de dibujo (z).
    pub(crate) strokes: Vec<&'a Stroke>,
}

/// Blit del visor (modo UNA HOJA) con UN solo lock+present: fondo + la página
/// actual (vecino-más-cercano para el zoom, recorte a la ventana — nunca otra
/// hoja: solo se dibuja `page`) + su capa de anotaciones (trazos Bresenham
/// sobre el bitmap) + los overlays del visor (indicador de página y sheet de
/// ajustes, cada uno con su posición). Es el equivalente de una página de
/// `zoom::blit_fast` (mismo contrato: fondo + página + overlays en el mismo
/// buffer, un único unlock_and_post — dividirlo en varios locks presentaría
/// varios buffers por frame y el compositor mostraría el frame anterior).
///
/// `page = None` si la página actual no tiene bitmap en la caché (render
/// fallido): solo fondo + overlays.
///
/// `dark` = modo oscuro activo: invierte los canales RGB de la página
/// (255 − v, la misma transformación que `pdf_core::dark::invert_bitmap`) en
/// el propio blit, píxel a píxel. La caché guarda SIEMPRE bitmaps normales;
/// materializar una copia invertida por página y frame sería memoria y GC
/// innecesarios (a zoom alto una página puede pesar cientos de MiB). Coste:
/// una pasada extra (~1-3 ms a pantalla completa).
///
/// Las ANOTACIONES no se invierten en modo oscuro: la tinta conserva su
/// color (decisión: la capa de anotaciones es independiente del modo de
/// visualización de la página, como el subrayado físico).
pub(crate) fn blit_page(
    window: &NativeWindow,
    bg: [u8; 4],
    dark: bool,
    page: Option<&PageBlit>,
    anns: Option<&PageAnnots>,
    sel: Option<(f32, f32, f32, f32)>,
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

    // La página actual, escalada al zoom relativo pedido y recortada a la
    // ventana (la posición la calcula `reader`: centrado cover + pan). Tras
    // ella, su capa de anotaciones (trazos y highlights sobre el bitmap, en
    // orden z) y el rect de selección en vivo/fijado (translúcido, ya
    // recortado a los bordes de la página por el caller).
    if let Some(page) = page {
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
        if let Some(layer) = anns {
            draw_annotations(dst, dst_w, dst_h, dst_stride, bpp, layer);
        }
        if let Some((l, t, r, b)) = sel {
            draw_sel_rect(dst, dst_w, dst_h, dst_stride, bpp, l, t, r, b);
        }
    }

    // Overlays del visor (indicador de página, sheet de ajustes), cada uno
    // con su esquina superior izquierda en px de ventana.
    for (ov, ox, oy) in overlays {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, *ox, *oy);
    }
}

/// Compone el frame de página (fondo + página actual + capa de anotaciones +
/// indicador "N / total") en un `Bitmap` RGBA8 del tamaño de la ventana: la
/// MISMA salida que un blit normal, pero en memoria propia en vez del
/// ANativeWindow. Lo usa `Reader` para la animación del sheet: el frame se
/// compone UNA vez al empezar a deslizar y cada frame de la animación solo lo
/// copia al buffer (`blit_composed`, memcpy ~1-2 ms) + el overlay del sheet,
/// en vez de re-blitear la página completa en cada paso (~25-40 ms/frame — la
/// causa del lag del sheet; ver `Reader::page_frame`).
///
/// `page = None` si la página actual no tiene bitmap en la caché: el frame
/// queda con solo fondo + indicador. `sel` es el rect de selección en px de
/// ventana (ya recortado a la página) que se dibuja como capa translúcida
/// sobre la página, igual que en `blit_page`.
//
// 8 parámetros posicionales de un blit (raw pointer + dimensiones + piezas):
// mismo patrón que `blit_page` y `copy_region`; se acepta el allow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_frame(
    w: i32,
    h: i32,
    bg: [u8; 4],
    dark: bool,
    page: Option<&PageBlit>,
    anns: Option<&PageAnnots>,
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
        if let Some(layer) = anns {
            draw_annotations(dst, fw, fh, fw, 4, layer);
        }
        if let Some((l, t, r, b)) = sel {
            draw_sel_rect(dst, fw, fh, fw, 4, l, t, r, b);
        }
    }
    if let Some((b, bx, by)) = badge {
        copy_region(dst, fw, fh, fw, 4, b, bx, by);
    }
    frame
}

/// Blit de un frame ya compuesto (`compose_frame`) + overlays al buffer con
/// UN solo lock+present. Cada overlay va con su esquina superior izquierda en
/// px de ventana. Uso: animación del sheet — copiar el frame es un memcpy
/// (~1-2 ms en la tablet), mucho más barato que re-blitear la página.
pub(crate) fn blit_composed(
    window: &NativeWindow,
    frame: &Bitmap,
    overlays: &[(&Bitmap, i32, i32)],
    blend_layer: Option<(&Bitmap, i32, i32)>,
) {
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
    // Frame completo (fondo + página + indicador) en la esquina (0,0).
    copy_region(dst, dst_w, dst_h, dst_stride, bpp, frame, 0, 0);
    // Capa de anotación EN CURSO (trazo/resaltador), con alfa-blend: va
    // sobre la página pero DEBAJO de los overlays opacos (menú, sheet...).
    if let Some((ov, ox, oy)) = blend_layer {
        copy_region_blend(dst, dst_w, dst_h, dst_stride, bpp, ov, ox, oy);
    }
    for (ov, ox, oy) in overlays {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, *ox, *oy);
    }
}

/// Igual que [`blit_composed`] pero SOLO repinta el rectángulo sucio
/// `dirty` (px de ventana) del frame — el resto de la ventana ya tiene el
/// contenido anterior (el blit anterior lo copió): ahorra la copia completa
/// de ~12 MB por Move del gesto (4-8 ms → ~0.05-0.2 ms). El blend del trazo
/// y los overlays opacos (FAB, barra, indicador — todos pequeños) se
/// repintan cada vez sobre su área.
///
/// Uso: SOLO cuando `tool_gesture` está activo y el sheet está cerrado (el
/// frame es la única fuente de verdad y la ventana ya la muestra).
pub(crate) fn blit_composed_dirty(
    window: &NativeWindow,
    frame: &Bitmap,
    overlays: &[(&Bitmap, i32, i32)],
    blend_layer: Option<(&Bitmap, i32, i32)>,
    dirty: (i32, i32, i32, i32),
) {
    // Lock con REGIÓN SUCIO: además de copiar solo el rect, se le pasa a
    // ANativeWindow_lock para que el compositor solo procese esa zona
    // (camino rápido del driver; en TCL alivia algo el lock vsync).
    let (x0, y0, x1, y1) = dirty;
    let mut dirty_rect = android_activity::ndk_sys::ARect {
        left: x0,
        top: y0,
        right: x1,
        bottom: y1,
    };
    let Ok(mut guard) = window.lock(Some(&mut dirty_rect)) else {
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
    if bpp == 4 {
        copy_region_rect(dst, dst_w, dst_h, dst_stride, bpp, frame, x0, y0, x1, y1);
    } else {
        // Formato no RGBA: sin dirty rect, copia completa (raro; el buffer
        // se fuerza a R8G8B8A8_UNORM).
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, frame, 0, 0);
    }
    if let Some((ov, ox, oy)) = blend_layer {
        copy_region_blend(dst, dst_w, dst_h, dst_stride, bpp, ov, ox, oy);
    }
    for (ov, ox, oy) in overlays {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, ov, *ox, *oy);
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
fn copy_region_blend(
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
            for (i, px) in src_row.chunks_exact(4).enumerate() {
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
            for (i, px) in src_row.chunks_exact(4).enumerate() {
                let o = i * bpp;
                dst_row[o..o + n].copy_from_slice(&px[..n]);
            }
        }
    }
}

/// Escala `src` (RGBA8) por vecino-más-cercano a tamaño `src × zoom` y copia
/// el resultado al buffer con su esquina superior izquierda en `(dx, dy)` px,
/// recortando los bordes fuera de la ventana. Espejo de
/// `zoom::blit_scaled_nearest` (que es privado y no se puede reutilizar desde
/// aquí): misma fórmula, mismo estilo — véase su doc para el mapeo entero
/// `src = (dst_rel × src_dim) / dst_dim` con tabla x precalculada.
///
/// `dark` añade la inversión de canales RGB inline (ver `blit_page`).
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
    // por el caller; el overlay lo pinta `blit_page` después).
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
    // Camino 1:1 (`zoom == 1.0`, el reposo del visor: página renderizada a la
    // escala exacta, incluido el arrastre de la SELECCIÓN DE TEXTO — cada Move
    // re-blitea la página): el mapa vecino-más-cercano es la identidad
    // (`dw == src_w` ⇒ `src_x = x − dx`), así que se copia FILA a FILA sin
    // tabla x ni divisiones — evita el coste del bucle de vecino-más-cercano
    // en el camino que más veces se ejecuta (el lag del "cuadrado de
    // seleccionar" se medía también aquí; ver `Reader::blit`).
    if zoom == 1.0 {
        let src_ox = (x0 - i64::from(dx)) as usize; // 1:1: origen = destino
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
                        // Inversión RGB inline como XOR de u32 (255 − v):
                        // 0x00FF_FFFF invierte los bytes 0-2 (R, G, B) y deja
                        // el canal alfa (byte 3) intacto — la misma
                        // transformación que `pdf_core::dark::invert_bitmap`.
                        // Una pasada por u32 en vez de 4 accesos por byte
                        // (medido: 1,38 ms → ~0,4 ms a 2000×1200 en
                        // `blit` bench — el loop por bytes no auto-vectoriza
                        // con el slice por fila).
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
                // Mapeo x de vecino-más-cercano como accesos por u32:
                // `x_map[x]` ∈ [0, src_w) por construcción (división entera
                // truncada con dst_rel < dw), así que leer/escribir u32
                // directos evita el slice con bounds-check por píxel del
                // camino antiguo (medido: zoom 1,35 × 2000×1200 1,29 ms →
                // ~memcpy+mapa en el `blit` bench). El alfa (byte 3) no se
                // toca en oscuro (XOR 0x00FF_FFFF, ver camino 1:1).
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
pub(crate) fn draw_annotations(
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
    // Los highlights (rellenos translúcidos) se dibujan PRIMERO, debajo de
    // los trazos: el orden de dibujo es por capas (subrayado bajo la tinta).
    for h in &layer.highlights {
        draw_highlight(
            dst, dst_w, dst_h, dst_stride, bpp, h, scale, layer.dx, layer.dy,
        );
    }
    for s in &layer.strokes {
        draw_stroke(
            dst, dst_w, dst_h, dst_stride, bpp, &s.points, s.width, s.color, scale, layer.dx,
            layer.dy,
        );
    }
}

/// Dibuja un highlight (rect o rects en coordenadas de página) como relleno
/// translúcido (el alfa del color) sobre el bitmap, con la MISMA
/// transformación que los trazos (`pt × scale + (dx, dy)`): el subrayado
/// queda pegado al texto a cualquier zoom y nunca se rasteriza en el bitmap
/// cacheado (AGENTS.md §4.3). El color se pasa tal cual: las anotaciones no
/// se invierten en modo oscuro (ver `blit_page`).
#[allow(clippy::too_many_arguments)]
fn draw_highlight(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    h: &Highlight,
    scale: f32,
    dx: i32,
    dy: i32,
) {
    let color = [h.color.r, h.color.g, h.color.b, h.color.a];
    for r in &h.rects {
        let l = r.x * scale + dx as f32;
        let t = r.y * scale + dy as f32;
        let w = r.w * scale;
        let hh = r.h * scale;
        fill_rect(
            dst,
            dst_w,
            dst_h,
            dst_stride,
            bpp,
            l,
            t,
            l + w,
            t + hh,
            color,
        );
    }
}

/// Rellena un rectángulo (px de ventana, f32) con `color` RGBA8, recortado a
/// la ventana. Reutiliza `stamp` (alfa-blend en bpp 4, opaco en bpp 2/otros),
/// así que el relleno respeta la transparencia del color. Se usa para los
/// highlights (subrayados) y para el rect de selección (`fill_rect_bordered`).
#[allow(clippy::too_many_arguments)]
fn fill_rect(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    color: [u8; 4],
) {
    fill_rect_bordered(
        dst, dst_w, dst_h, dst_stride, bpp, x0, y0, x1, y1, color, None,
    );
}

/// Rellena un rectángulo (px de ventana, f32) recortado a la ventana; con
/// `border = Some(color)` dibuja además un borde de 2 px (píxeles a menos de
/// 2 del borde del rect) con ese color, sobre el relleno.
///
/// Camino RÁPIDO (bpp 4, el formato forzado del buffer): relleno por FILAS
/// con alfa-blend por LUT (`fill_rect_lut`, sin división ni llamada por
/// píxel) y el borde como 4 rectas finas sobre el relleno. Es el camino del
/// RECT DE SELECCIÓN, que se redibuja en cada Move del arrastre (~60-120 Hz):
/// el camino antiguo (un `stamp` por píxel con división por 255 en cada
/// canal) era la causa del lag del "cuadrado de seleccionar" en la tablet
/// (medible con el log de frame time de `Reader::blit`). bpp 2/otros
/// conservan el camino antiguo (raro: el buffer se fuerza a R8G8B8A8_UNORM).
#[allow(clippy::too_many_arguments)]
fn fill_rect_bordered(
    dst: *mut u8,
    dst_w: usize,
    dst_h: usize,
    dst_stride: usize,
    bpp: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    fill: [u8; 4],
    border: Option<[u8; 4]>,
) {
    // Enteros con recorte a la ventana: el rect se extiende de floor(x0) a
    // ceil(x1) (cubre el área f32 completa; el borde queda en la frontera).
    let ix0 = x0.floor().max(0.0) as i32;
    let iy0 = y0.floor().max(0.0) as i32;
    let ix1 = x1.ceil().min(dst_w as f32) as i32;
    let iy1 = y1.ceil().min(dst_h as f32) as i32;
    if ix1 <= ix0 || iy1 <= iy0 {
        return;
    }
    const BORDER_W: i32 = 2;
    if bpp == 4 {
        // Relleno interior (sin el anillo del borde) + borde como 4 rectas
        // finas encima. El blend sobre sí mismo es idempotente (las esquinas
        // se escriben dos veces con el mismo color → resultado exacto).
        fill_rect_lut(
            dst, dst_w, dst_h, dst_stride, ix0, iy0, ix1, iy1, fill, BORDER_W,
        );
        if let Some(bc) = border {
            fill_rect_lut(
                dst,
                dst_w,
                dst_h,
                dst_stride,
                ix0,
                iy0,
                ix1,
                iy0 + BORDER_W,
                bc,
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
                bc,
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
                bc,
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
                bc,
                0,
            );
        }
        return;
    }
    // bpp 2/otros: camino antiguo (un `stamp` por píxel).
    let disc = [(0, 0)];
    for y in iy0..iy1 {
        for x in ix0..ix1 {
            let edge = match border {
                Some(_) => {
                    x - ix0 < BORDER_W
                        || ix1 - 1 - x < BORDER_W
                        || y - iy0 < BORDER_W
                        || iy1 - 1 - y < BORDER_W
                }
                None => false,
            };
            let c = if edge { border.unwrap() } else { fill };
            stamp(dst, dst_w, dst_h, dst_stride, bpp, x, y, &disc, c);
        }
    }
}

/// Rellena un rectángulo ALINEADO a píxel entero `[x0,x1) × [y0,y1)` (ya
/// recortado por el caller) con alfa-blend RGBA8 por LUT:
/// `out = (c·a)/255 + (d·(255−a))/255` con 2 tablas de 256 entradas
/// construidas UNA vez por llamada — sin división por píxel (la división de
/// la fórmula exacta `(c·a + d·(255−a))/255` se aproxima con ±1 por canal:
/// imperceptible en overlays translúcidos como el rect de selección o los
/// highlights). Relleno por FILAS completas, sin llamada por píxel. `shrink`
/// resta ese nº de px a cada borde (el relleno interior del rect con borde).
/// Opaco (a=255) → copia directa por filas (el borde SEL_BORDER); a=0 → no-op.
//
// 7 parámetros posicionales de un blit (raw pointer + dimensiones): mismo
// patrón que `copy_region`; se acepta el allow en vez de empaquetarlos.
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
    // Tablas de blend (idénticas por canal, la fórmula solo depende de a).
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

/// Color del relleno del rect de selección (azul accent, alfa ~30 %: 77/255).
const SEL_FILL: [u8; 4] = [0x4D, 0xA3, 0xFF, 0x4D];
/// Color del borde del rect de selección (1-2 px, alfa completo).
const SEL_BORDER: [u8; 4] = [0x4D, 0xA3, 0xFF, 0xFF];

/// Dibuja el rect de selección en vivo/fijado (px de ventana, ya recortado a
/// los bordes de la página por `Reader::sel_screen_rect`): relleno
/// translúcido azul/accent (~30 %) con borde de 2 px, sobre la página (y las
/// anotaciones), antes de los overlays.
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
    fill_rect_bordered(
        dst,
        dst_w,
        dst_h,
        dst_stride,
        bpp,
        l,
        t,
        r,
        b,
        SEL_FILL,
        Some(SEL_BORDER),
    );
}

/// Transforma un trazo (coordenadas de página) a px de ventana
/// (`pt × scale + (dx, dy)`) y lo dibuja como polilínea Bresenham de grosor
/// `width × scale` px (mínimo 1 px). El color se pasa tal cual (RGBA8): las
/// anotaciones no se invierten en modo oscuro (ver `blit_page`).
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

/// Botones del sheet de ajustes del visor (2 filas × 3): fila 1 = "← Library"
/// (biblioteca MediaStore) | Dark/Light (la etiqueta cambia con el modo) |
/// Search (biblioteca con la búsqueda lista para empezar, sin teclado = la
/// barra de filtros de la biblioteca); fila 2 = −10 | "N / total" (tap =
/// página siguiente) | +10. La geometría se comparte con `input::sheet_tap`
/// (mismas fórmulas `sheet_*` de `reader`).
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
    let row1: [&'static str; 3] = [
        "← Library",
        if reader.dark { "Light" } else { "Dark" },
        "Search",
    ];
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
/// `win_w × sheet_h(win_h)` (la mitad de la ventana): tarjeta deslizante desde
/// el borde superior con título, un asa central, los botones de `sheet_buttons`
/// ("← Library" / Dark-Light / Search y −10 / "N / total" / +10) y la pista de
/// cierre. Cacheado en `Reader::sheet_bitmap` (invalida al cambiar ventana,
/// página o modo oscuro; se libera al cerrar).
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

    // Tarjeta deslizable desde arriba: esquinas inferiores redondeadas (18 px)
    // y borde de 1 px.
    let card_r = 18.0f32;
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

    // Asa central (pista visual de "deslizable") + título de la tarjeta.
    let pad = sheet_pad(w);
    let handle_w = (w / 10).max(48) as f32;
    rects.push(CanvasRect::rounded(
        (w as f32 - handle_w) / 2.0,
        10.0,
        (w as f32 + handle_w) / 2.0,
        16.0,
        3.0,
        bar_border,
    ));
    texts.push(CanvasText::new(
        pad,
        26.0 + 14.0 * 0.85,
        14.0,
        btn_text,
        TextAlign::Left,
        true,
        "Settings",
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

    // Botones estilo píldora con acento dorado en los estados activos.
    let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
    for (label, (l, t, r, b)) in sheet_buttons(reader, w as f32, reader.win_h as f32) {
        let (fill, border, text_color) = match label {
            // "← Library" y "Search": acción principal → acento dorado.
            "← Library" | "Search" => (
                theme::ACCENT_AMBER_BG,
                theme::ACCENT_AMBER_BORDER,
                0xFF0B0D12,
            ),
            // Dark/Light: dorado SOLO cuando el modo oscuro está activo.
            "Dark" | "Light" => {
                if reader.dark {
                    (
                        theme::ACCENT_AMBER_BG,
                        theme::ACCENT_AMBER_BORDER,
                        0xFF0B0D12,
                    )
                } else {
                    (btn_bg, btn_border, btn_text)
                }
            }
            "N / total" => (badge_bg, badge_border, badge_text),
            _ => (btn_bg, btn_border, btn_text),
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
            true,
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

/// Renderiza la biblioteca MediaStore a un bitmap RGBA8 de tamaño de
/// ventana: la pantalla de biblioteca PERSONAL premium (estilo Apple
/// Books/Kindle pero propio) — las PORTADAS son lo principal, no un file
/// manager (sin rutas completas visibles, sin iconos PDF genéricos: siempre
/// portada real o placeholder elegante con el título).
///
/// Estructura FIJA (no scrollea): cabecera editorial (título "Library"
/// grande + botón "＋ Add book") + campo de búsqueda (+ panel de chips de
/// letra/carpeta si está abierto) + franja de estado (si la hay). Contenido
/// SCROLLABLE (desplazado `reader.lib_scroll` px): [Continue Reading:
/// carousel horizontal de tarjetas con portada 2:3 grande, título, autor,
/// barra de progreso, "Page X of Y · Z%" y botón Read] + [título "My
/// Library" + chips de organización (sort/filter) + rejilla 3×3 de portadas
/// con título, autor y barra fina de progreso] + EMPTY STATE si no hay PDFs.
///
/// El bitmap base (fondo, cabecera, campo de búsqueda, chips, tarjetas,
/// textos, barras de progreso, SOMBRAS y placeholders) se dibuja con
/// Canvas+JNI (`jni_text_bitmap`); las portadas CACHEADAS se pegan después
/// directamente sobre sus bytes RGBA (Canvas no pinta bitmaps): center-crop
/// vecino-más-cercano al área 2:3 (`grid_cover_w`×`grid_cover_h` /
/// `lib_cont_cover_*`), sin pasar por un lock de ventana.
/// Color de fondo de la biblioteca en RGBA (tema `LIB_BG` = 0xFF0B0D12):
/// lo usa `blit_library` como base del buffer (la zona fija y la banda se
/// copian encima; el sobrante queda del color de la biblioteca).
const LIB_BG_RGBA: [u8; 4] = [0x0B, 0x0D, 0x12, 0xFF];

/// Render de la ZONA FIJA de la biblioteca (cabecera editorial + campo de
/// búsqueda + franja de estado) a un bitmap de alto `content_y0`, origen =
/// borde superior de la ventana. La zona fija NO scrollea y se re-renderiza
/// solo cuando cambia la estructura (datos, filtros, panel de búsqueda,
/// estado, tamaño de ventana) — ver `Reader::rebuild_library`. Los chips del
/// panel de búsqueda (fila letras / fila carpetas) NO se dibujan aquí: son
/// filas horizontales móviles que `Reader` remienda encima (`render_search_chip_row`
/// + `splice_row`) para que su scroll horizontal no re-renderice la pantalla.
pub(crate) fn render_library_header(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = lib_content_y0(
        reader.win_h,
        reader.lib_search_open,
        reader.status.is_some(),
    );
    if w <= 0 || h <= 0 {
        return None;
    }
    let pad = grid_pad(w);
    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Fondo del bloque fijo (algo más claro que el contenido) + hairline
    // 1 px bajo la zona fija: separación editorial entre cabecera y lista.
    rects.push(CanvasRect::sharp(
        0.0,
        0.0,
        w as f32,
        h as f32,
        theme::LIB_HEADER_BG,
    ));
    rects.push(CanvasRect::sharp(
        0.0,
        h as f32 - 1.0,
        w as f32,
        h as f32,
        theme::LIB_HEADER_BORDER,
    ));

    // ---- CABECERA editorial: título "Library" + "＋ Add book" ----
    let btn_w = (w as f32 * 0.24).clamp(120.0, 220.0);
    let btn_h = (header_h * 0.5).clamp(36.0, 52.0);
    let btn_y = (header_h - btn_h) / 2.0;
    let btn_x = w as f32 - pad - btn_w;
    let title_ts = (header_h * 0.34).clamp(30.0, 44.0);
    texts.push(CanvasText::new(
        pad,
        header_h * 0.66,
        title_ts,
        theme::LIB_TEXT_PRIMARY,
        TextAlign::Left,
        true,
        "Library",
    ));
    draw_button(
        &mut rects,
        &mut texts,
        btn_x,
        btn_y,
        btn_x + btn_w,
        btn_y + btn_h,
        theme::LIB_ACCENT,
        theme::ACCENT_AMBER_BORDER,
        theme::LIB_ACCENT_DARK,
        btn_h * 0.42,
        true,
        "＋ Add book",
    );

    // ---- CAMPO de búsqueda (píldora; al tocarla abre el panel de chips) ----
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;
    let field_r = search_hh / 2.0;
    rects.push(CanvasRect::rounded(
        pad,
        search_y,
        w as f32 - pad,
        search_y + search_hh,
        field_r,
        theme::LIB_SEARCH_BORDER,
    ));
    rects.push(CanvasRect::rounded(
        pad + 1.0,
        search_y + 1.0,
        w as f32 - pad - 1.0,
        search_y + search_hh - 1.0,
        (field_r - 1.0).max(0.0),
        theme::LIB_SEARCH_BG,
    ));
    let (summary, has_filter) = search_summary(reader);
    if has_filter {
        // Resumen del filtro activo + "✕" (limpia filtros y cierra el panel).
        texts.push(CanvasText::new(
            pad + 14.0,
            search_y + search_hh * 0.64,
            13.0,
            theme::LIB_TEXT_PRIMARY,
            TextAlign::Left,
            false,
            summary,
        ));
        let xw = search_hh - 8.0;
        let xx = w as f32 - pad - 14.0 - xw;
        rects.push(CanvasRect::rounded(
            xx,
            search_y + 4.0,
            xx + xw,
            search_y + 4.0 + xw,
            xw / 2.0,
            theme::DARK_BTN_BG,
        ));
        texts.push(CanvasText::new(
            xx + xw / 2.0,
            search_y + 4.0 + xw * 0.64,
            12.0,
            theme::LIB_TEXT_SECONDARY,
            TextAlign::Center,
            false,
            "✕",
        ));
    } else {
        texts.push(CanvasText::new(
            pad + 14.0,
            search_y + search_hh * 0.64,
            13.0,
            theme::LIB_TEXT_MUTED,
            TextAlign::Left,
            false,
            "Search by title or folder",
        ));
    }

    // ---- FRANJA de estado (si la hay): ocupa el tramo final de la zona
    // fija (bajo el panel de búsqueda, sea panel abierto o cerrado). ----
    if let Some(status) = reader.status.as_deref() {
        let row_h = picker_row_h(reader.win_h) as f32;
        let status_top = h as f32 - row_h;
        rects.push(CanvasRect::sharp(
            0.0,
            status_top,
            w as f32,
            h as f32,
            theme::STATUS_BG,
        ));
        rects.push(CanvasRect::sharp(
            0.0,
            h as f32 - 1.0,
            w as f32,
            h as f32,
            theme::STATUS_BORDER,
        ));
        texts.push(CanvasText::new(
            pad,
            status_top + row_h * 0.62,
            row_h * 0.36,
            theme::STATUS_TEXT,
            TextAlign::Left,
            true,
            status,
        ));
    }

    jni_text_bitmap(w, h, theme::LIB_HEADER_BG, &rects, &texts)
}

/// Render de la BANDA de contenido de la biblioteca (la zona scrolleable:
/// Continue Reading + My Library + rejilla o empty state) a un bitmap de
/// alto `band_h` cuyas filas representan coordenadas de CONTENIDO en
/// [band_origin, band_origin + band_h). El scroll vertical NO se aplica
/// aquí: `blit_library` copia la banda al buffer desplazada por el scroll
/// (memcpy). Es el análogo del `page_frame` del visor para la biblioteca: el
/// render caro (Canvas+JNI) se paga una vez por banda, no por frame.
///
/// Las filas horizontales (tarjetas del carousel y chips de sort/filter) no
/// se dibujan aquí: `Reader` las remienda encima (`render_carousel_row` /
/// `render_org_chip_row` + `splice_row`) para que su scroll horizontal no
/// re-renderice la pantalla completa.
pub(crate) fn render_library_zone(
    reader: &Reader,
    band_origin: i32,
    band_h: i32,
) -> Option<Bitmap> {
    let w = reader.win_w;
    if w <= 0 || band_h <= 0 {
        return None;
    }
    let yof = -band_origin as f32; // contenido − origen de banda
    let pad = grid_pad(w);
    let has_cont = !reader.lib_continue_reading().is_empty();
    let cont_block_h = lib_cont_block_h(w, reader.win_h, has_cont);
    let section_title_h = lib_section_title_h(reader.win_h);

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    if reader.library_list.is_empty() {
        // EMPTY STATE (sin PDFs): centrado en la ventana; su geometría es en
        // px de ventana, así que se traslada a la banda (contenido −
        // content_y0 − band_origin).
        let content_y0 = lib_content_y0(
            reader.win_h,
            reader.lib_search_open,
            reader.status.is_some(),
        );
        let shift = -(content_y0 as f32 + band_origin as f32);
        draw_empty_state(reader, &mut rects, &mut texts, shift);
    } else {
        // 1. Sección "Continue Reading" (título estático; las tarjetas van
        //    en `render_carousel_row`, remendadas sobre la banda).
        texts.push(CanvasText::new(
            pad,
            yof + section_title_h * 0.72,
            11.0,
            theme::LIB_TEXT_MUTED,
            TextAlign::Left,
            true,
            "CONTINUE READING",
        ));

        // 2. Título de "My Library" (la rejilla principal).
        let my_lib_y = yof + cont_block_h + section_title_h * 0.72;
        texts.push(CanvasText::new(
            pad,
            my_lib_y,
            11.0,
            theme::LIB_TEXT_MUTED,
            TextAlign::Left,
            true,
            "MY LIBRARY",
        ));

        // 3. Etiquetas "SORT"/"FILTER" del bloque de organización (los chips
        //    van en `render_org_chip_row`).
        for (row, label_text) in ["SORT", "FILTER"].iter().enumerate() {
            let org_y0 = yof + lib_org_y(w, reader.win_h, has_cont, row);
            texts.push(CanvasText::new(
                pad,
                org_y0 + lib_org_chip_h(reader.win_h) * 0.74,
                9.0,
                theme::LIB_TEXT_MUTED,
                TextAlign::Left,
                false,
                *label_text,
            ));
        }

        // 4. REJILLA 3×3 (lista FILTRADA): solo las filas que intersectan la
        //    banda — celda a celda: sombra + portada/placeholder + título +
        //    autor + barra fina de progreso si empezado.
        let grid_y0 = lib_grid_y0(w, reader.win_h, has_cont);
        let cell_h = grid_cell_h(w);
        let cell_w = grid_cell_w(w);
        let cover_w = grid_cover_w(w);
        let cover_h = grid_cover_h(w);
        let title_ts = 13.0f32;
        let char_w = title_ts * 0.55;
        let max_chars = (((cell_w - 2.0 * GRID_CELL_PAD) / char_w) as usize).max(3);
        let row_first = (((band_origin as f32 - grid_y0) / cell_h).floor().max(0.0)) as usize;
        let row_last = (((band_origin + band_h) as f32 - grid_y0) / cell_h)
            .ceil()
            .max(0.0) as usize;
        for row in row_first..row_last {
            for col in 0..GRID_COLS {
                let Some(entry) = reader.grid_entry_at(row, col) else {
                    continue;
                };
                // Celda en coords de BANDA: contenido (grid_y0 + cy_rel) − origen.
                let (cx, cy_rel, _, _) = grid_cell_rect(w, 0, row, col);
                let cy = yof + grid_y0 + cy_rel;
                let cover_x0 = cx + (cell_w - cover_w) / 2.0;
                let cover_y0 = cy + 4.0;
                let cover_r = 14.0f32;
                rects.push(CanvasRect::rounded(
                    cover_x0 + 2.0,
                    cover_y0 + 3.0,
                    cover_x0 + 2.0 + cover_w,
                    cover_y0 + 3.0 + cover_h,
                    cover_r,
                    theme::LIB_COVER_SHADOW,
                ));
                if reader.thumbs.peek(&entry.uri).is_none() {
                    rects.push(CanvasRect::rounded(
                        cover_x0,
                        cover_y0,
                        cover_x0 + cover_w,
                        cover_y0 + cover_h,
                        cover_r,
                        theme::LIB_COVER_PLACEHOLDER,
                    ));
                    // Placeholder ELEGANTE con el título (nada de "…" ni de
                    // iconos PDF genéricos).
                    texts.push(CanvasText::new(
                        cover_x0 + cover_w / 2.0,
                        cover_y0 + cover_h / 2.0 + title_ts * 0.35,
                        title_ts,
                        theme::LIB_TEXT_SECONDARY,
                        TextAlign::Center,
                        true,
                        truncate_name(&entry_title(entry), 12),
                    ));
                }
                // Título (primario) + autor (muted).
                texts.push(CanvasText::new(
                    cx + GRID_CELL_PAD,
                    cy + 4.0 + cover_h + 12.0 + title_ts * 0.85,
                    title_ts,
                    theme::LIB_TEXT_PRIMARY,
                    TextAlign::Left,
                    false,
                    truncate_name(&entry_title(entry), max_chars),
                ));
                texts.push(CanvasText::new(
                    cx + GRID_CELL_PAD,
                    cy + 4.0 + cover_h + 12.0 + 21.0 + 10.0 * 0.85,
                    10.0,
                    theme::LIB_TEXT_MUTED,
                    TextAlign::Left,
                    false,
                    truncate_name(&entry_author(entry), max_chars),
                ));
                // Barra fina de progreso si el libro está empezado.
                if let Some(p) = persist::progress_for(&reader.lib_books, &reader.entry_path(entry))
                {
                    let bar_y = cy + 4.0 + cover_h + 12.0 + 36.0;
                    let track_w = cell_w - 2.0 * GRID_CELL_PAD;
                    rects.push(CanvasRect::rounded(
                        cx + GRID_CELL_PAD,
                        bar_y,
                        cx + GRID_CELL_PAD + track_w,
                        bar_y + 3.0,
                        1.5,
                        theme::LIB_PROGRESS_TRACK,
                    ));
                    let fill_w = (track_w * p.pct()).max(0.0);
                    if fill_w > 0.0 {
                        rects.push(CanvasRect::rounded(
                            cx + GRID_CELL_PAD,
                            bar_y,
                            cx + GRID_CELL_PAD + fill_w,
                            bar_y + 3.0,
                            1.5,
                            theme::LIB_ACCENT,
                        ));
                    }
                }
            }
        }

        // 5. Sin resultados con filtro activo (búsqueda o estado): aviso.
        if reader.lib_filtered.is_empty()
            && (reader.lib_letter.is_some()
                || reader.lib_folder.is_some()
                || reader.lib_status.is_some())
        {
            texts.push(CanvasText::new(
                w as f32 / 2.0,
                yof + cont_block_h + section_title_h + 24.0,
                13.0,
                theme::LIB_TEXT_MUTED,
                TextAlign::Center,
                false,
                "No matches — tap ✕ to clear",
            ));
        }
    }

    jni_text_bitmap(w, band_h, theme::LIB_BG, &rects, &texts)
}

/// Render de la fila HORIZONTAL del carousel de "Continue Reading" a un
/// bitmap (ancho = extensión total de las tarjetas, alto = tarjeta): el
/// arrastre horizontal mueve la copia de esta fila sobre la banda
/// (`splice_row` con `sx = -lib_carousel_x`), así que el canvas de la banda
/// no se re-renderiza por frame de scroll horizontal. Las portadas cacheadas
/// se pegan DENTRO de la fila (cada tarjeta en su posición), no en la banda.
pub(crate) fn render_carousel_row(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let books = reader.lib_continue_reading();
    let n = books.len();
    if n == 0 {
        return None;
    }
    let cw = lib_cont_cover_w(w);
    let chh = lib_cont_cover_h(w);
    let card_w = lib_cont_card_w(w);
    let card_h = lib_cont_card_h(w);
    let cover_r = 14.0f32;
    let row_w = (lib_cont_card_x(w, n - 1) + card_w + grid_pad(w)).ceil() as i32;
    let row_h = card_h.ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }

    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (i, book) in books.iter().enumerate() {
        let cx = lib_cont_card_x(w, i);
        // Tarjeta: borde + relleno (esquinas 16 px, más redondeada que la
        // rejilla) — la pieza "premium" del carousel.
        rects.push(CanvasRect::rounded(
            cx,
            0.0,
            cx + card_w,
            card_h,
            16.0,
            theme::LIB_CARD_BORDER,
        ));
        rects.push(CanvasRect::rounded(
            cx + 1.0,
            1.0,
            cx + card_w - 1.0,
            card_h - 1.0,
            15.0,
            theme::LIB_CARD_BG,
        ));
        let cover_x = cx + 10.0;
        let cover_y = 10.0;
        // Sombra sutil + portada (o placeholder elegante con el título).
        rects.push(CanvasRect::rounded(
            cover_x + 2.0,
            cover_y + 3.0,
            cover_x + 2.0 + cw,
            cover_y + 3.0 + chh,
            cover_r,
            theme::LIB_COVER_SHADOW,
        ));
        if reader.thumbs.peek(&book.path).is_none() {
            rects.push(CanvasRect::rounded(
                cover_x,
                cover_y,
                cover_x + cw,
                cover_y + chh,
                cover_r,
                theme::LIB_COVER_PLACEHOLDER,
            ));
            texts.push(CanvasText::new(
                cover_x + cw / 2.0,
                cover_y + chh / 2.0 + 7.0,
                13.0,
                theme::LIB_TEXT_SECONDARY,
                TextAlign::Center,
                true,
                truncate_name(&title_from_name(&book.name), 12),
            ));
        }
        // Texto de la tarjeta: título, autor, barra, meta y botón Read
        // (coordenadas relativas a la fila: la portada empieza en y = 10).
        let tx = cover_x;
        let ty = cover_y + chh;
        texts.push(CanvasText::new(
            tx,
            ty + 10.0 + 14.0 * 0.85,
            14.0,
            theme::LIB_TEXT_PRIMARY,
            TextAlign::Left,
            true,
            truncate_name(&title_from_name(&book.name), 16),
        ));
        texts.push(CanvasText::new(
            tx,
            ty + 10.0 + 22.0 + 10.0 * 0.85,
            10.0,
            theme::LIB_TEXT_MUTED,
            TextAlign::Left,
            false,
            truncate_name(&book.author, 16),
        ));
        // Barra de progreso (track + relleno dorado).
        let bar_y = ty + 10.0 + 38.0;
        rects.push(CanvasRect::rounded(
            tx,
            bar_y,
            tx + cw,
            bar_y + 3.0,
            1.5,
            theme::LIB_PROGRESS_TRACK,
        ));
        let fill_w = (cw * book.pct).max(0.0);
        if fill_w > 0.0 {
            rects.push(CanvasRect::rounded(
                tx,
                bar_y,
                tx + fill_w,
                bar_y + 3.0,
                1.5,
                theme::LIB_ACCENT,
            ));
        }
        // "Page X of Y · Z%"
        let meta = format!(
            "Page {} of {} · {:.0}%",
            book.page + 1,
            book.page_count,
            book.pct * 100.0
        );
        texts.push(CanvasText::new(
            tx,
            ty + 10.0 + 50.0 + 9.0 * 0.85,
            9.0,
            theme::LIB_TEXT_MUTED,
            TextAlign::Left,
            false,
            meta,
        ));
        // Acción clara "Read" (tocar la tarjeta también abre).
        let rbw = (cw * 0.52).clamp(64.0, 120.0);
        let rbh = 26.0;
        let rbx = tx + (cw - rbw) / 2.0;
        let rby = ty + 10.0 + 64.0;
        draw_button(
            &mut rects,
            &mut texts,
            rbx,
            rby,
            rbx + rbw,
            rby + rbh,
            theme::LIB_ACCENT,
            theme::ACCENT_AMBER_BORDER,
            theme::LIB_ACCENT_DARK,
            11.0,
            true,
            "Read",
        );
    }

    let mut out = jni_text_bitmap(row_w, row_h, theme::LIB_BG, &rects, &texts)?;
    // Portadas cacheadas DENTRO de la fila (sobre el placeholder de cada
    // tarjeta; la fila se re-renderiza al arrastrar, así que siempre van
    // pegadas a la posición actual del carousel).
    for (i, book) in books.iter().enumerate() {
        let Some(thumb) = reader.thumbs.peek(&book.path) else {
            continue;
        };
        let cover_x = (lib_cont_card_x(w, i) + 10.0).round() as i32;
        paste_thumb(
            &mut out.data,
            out.width as usize,
            thumb,
            cover_x,
            10,
            cw as i32,
            chh as i32,
        );
    }
    Some(out)
}

/// Render de la fila HORIZONTAL de chips del panel de BÚSQUEDA `row`
/// (0 = letras A-Z/#, 1 = carpetas) a un bitmap (ancho = extensión total de
/// los chips, alto = chip): se remienda sobre la cabecera con `splice_row`
/// (`sx = -lib_letters_x` / `-lib_folders_x`), sin re-renderizar la pantalla
/// al arrastrarla. Los chips se dibujan en sus coordenadas x GLOBALES (sin
/// scroll): el desplazamiento lo aplica el remiendo.
pub(crate) fn render_search_chip_row(reader: &Reader, row: usize) -> Option<Bitmap> {
    let chips = lib_chips(reader, row);
    if chips.is_empty() {
        return None;
    }
    // Ancho de la fila: última chip en coords GLOBALES (r + scroll) + margen.
    let scroll = if row == 0 {
        reader.lib_letters_x
    } else {
        reader.lib_folders_x
    };
    let row_w = chips
        .iter()
        .map(|(_, (_, _, r, _), _)| r + scroll)
        .fold(grid_pad(reader.win_w), f32::max)
        .ceil() as i32
        + grid_pad(reader.win_w) as i32;
    let row_h = lib_chip_h(reader.win_h).ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }
    let scroll = if row == 0 {
        reader.lib_letters_x
    } else {
        reader.lib_folders_x
    };
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (label, (l, t, r, b), active) in &chips {
        // `lib_chips` ya restó el scroll horizontal al rect: dibujar en la
        // fila su posición GLOBAL (l + scroll) y remendar con sx = -scroll
        // reproduce exactamente la posición en pantalla.
        let (gl, gr) = (l + scroll, r + scroll);
        let (fill, border, tc) = if *active {
            (
                theme::ACCENT_AMBER_BG,
                theme::ACCENT_AMBER_BORDER,
                0xFF0B0D12,
            )
        } else {
            (
                theme::DARK_BTN_BG,
                theme::DARK_BTN_BORDER,
                theme::DARK_BTN_TEXT,
            )
        };
        draw_button(
            &mut rects,
            &mut texts,
            gl,
            0.0,
            gr,
            b - t,
            fill,
            border,
            tc,
            (b - t) * 0.42,
            *active,
            label,
        );
    }
    // Fondo = el de la zona fija (los chips van sobre la cabecera).
    jni_text_bitmap(row_w, row_h, theme::LIB_HEADER_BG, &rects, &texts)
}

/// Render de la fila HORIZONTAL de chips de ORGANIZACIÓN `row` (0 = SORT,
/// 1 = FILTER) a un bitmap (ancho = extensión total, alto = chip): se
/// remienda sobre la banda con `splice_row` (`sx = -lib_sort_x` /
/// `-lib_filter_x`, `sy = lib_org_y(..) − band_origin`). Los chips se
/// dibujan en sus coordenadas x GLOBALES.
pub(crate) fn render_org_chip_row(reader: &Reader, row: usize) -> Option<Bitmap> {
    let chips = lib_org_chips(reader, row);
    if chips.is_empty() {
        return None;
    }
    let scroll = if row == 0 {
        reader.lib_sort_x
    } else {
        reader.lib_filter_x
    };
    let row_w = chips
        .iter()
        .map(|(_, (_, _, r, _), _)| r + scroll)
        .fold(grid_pad(reader.win_w), f32::max)
        .ceil() as i32
        + grid_pad(reader.win_w) as i32;
    let row_h = lib_org_chip_h(reader.win_h).ceil() as i32;
    if row_w <= 0 || row_h <= 0 {
        return None;
    }
    let scroll = if row == 0 {
        reader.lib_sort_x
    } else {
        reader.lib_filter_x
    };
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    for (label, (l, t, r, b), active) in &chips {
        let (gl, gr) = (l + scroll, r + scroll);
        let (fill, border, tc) = if *active {
            (
                theme::LIB_ACCENT,
                theme::ACCENT_AMBER_BORDER,
                theme::LIB_ACCENT_DARK,
            )
        } else {
            (
                theme::DARK_BTN_BG,
                theme::DARK_BTN_BORDER,
                theme::DARK_BTN_TEXT,
            )
        };
        draw_button(
            &mut rects,
            &mut texts,
            gl,
            0.0,
            gr,
            (b - t).max(1.0),
            fill,
            border,
            tc,
            (b - t) * 0.42,
            *active,
            label,
        );
    }
    jni_text_bitmap(row_w, row_h, theme::LIB_BG, &rects, &texts)
}

/// Copia la fila `row` (bitmap) sobre `dst` (zona fija o banda) con su
/// esquina superior izquierda en `(sx, sy)` px, recortada a los bordes de
/// `dst` (mismo contrato que `copy_region`). El scroll horizontal de la fila
/// se aplica como `sx = -scroll_x`.
pub(crate) fn splice_row(dst: &mut Bitmap, row: &Bitmap, sx: i32, sy: i32) {
    if dst.width == 0 || dst.height == 0 || row.width == 0 || row.height == 0 {
        return;
    }
    copy_region(
        dst.data.as_mut_ptr(),
        dst.width as usize,
        dst.height as usize,
        dst.width as usize,
        4,
        row,
        sx,
        sy,
    );
}

/// Pega las portadas CACHEADAS de la rejilla sobre la banda (celdas visibles
/// dentro de la banda, clave = content:// URI): se llama tras crear la banda
/// y cuando `pump_thumbs` cachea portadas nuevas — el placeholder del canvas
/// queda cubierto y NO se re-renderiza la pantalla (el pegado es memcpy por
/// celda). Las portadas del carousel viven DENTRO de su fila
/// (`render_carousel_row`), no en la banda.
pub(crate) fn paste_lib_thumbs(reader: &Reader, band: &mut Bitmap, band_origin: i32) {
    let w = reader.win_w;
    if w <= 0 || band.width == 0 || band.height == 0 {
        return;
    }
    let cell_w = grid_cell_w(w);
    let cover_w = grid_cover_w(w);
    let cover_h = grid_cover_h(w);
    let has_cont = reader.lib_has_cont();
    let grid_y0 = lib_grid_y0(w, reader.win_h, has_cont);
    let cell_h = grid_cell_h(w);
    let row_first = (((band_origin as f32 - grid_y0) / cell_h).floor().max(0.0)) as usize;
    let row_last = (((band_origin + band.height as i32) as f32 - grid_y0) / cell_h)
        .ceil()
        .max(0.0) as usize;
    for row in row_first..row_last {
        for col in 0..GRID_COLS {
            let Some(entry) = reader.grid_entry_at(row, col) else {
                continue;
            };
            let Some(thumb) = reader.thumbs.peek(&entry.uri) else {
                continue;
            };
            let (cx, cy_rel, _, _) = grid_cell_rect(w, 0, row, col);
            let cover_x0 = (cx + (cell_w - cover_w) / 2.0).round() as i32;
            let cover_y0 = (grid_y0 + cy_rel - band_origin as f32 + 4.0).round() as i32;
            paste_thumb(
                &mut band.data,
                band.width as usize,
                thumb,
                cover_x0,
                cover_y0,
                cover_w as i32,
                cover_h as i32,
            );
        }
    }
}

/// Blit de la biblioteca cacheadA (zona fija + banda de contenido) al buffer
/// del ANativeWindow con UN solo lock+present: copia la cabecera (0..content_y0)
/// y la banda desplazada por `scroll` + el aviso breve (toast), todo con
/// memcpy por filas. El scroll por frame NO re-renderiza nada (el parpadeo y
/// el lag del scroll de la biblioteca venían de re-renderizar la pantalla
/// entera por Canvas+JNI en cada Move; ver `render_library_header`/
/// `render_library_zone`).
pub(crate) fn blit_library(
    window: &NativeWindow,
    header: Option<&Bitmap>,
    band: Option<(&Bitmap, i32)>,
    content_y0: i32,
    scroll: f32,
    toast: Option<(&Bitmap, i32, i32)>,
) {
    let Ok(mut guard) = window.lock(None) else {
        warn!("ANativeWindow_lock failed");
        return;
    };
    let Some(bpp) = guard.format().bytes_per_pixel() else {
        warn!(
            "buffer format without bytes_per_pixel: {:?}",
            guard.format()
        );
        return;
    };
    let dst_w = guard.width();
    let dst_h = guard.height();
    let dst_stride = guard.stride(); // en píxeles
    let dst = guard.bits() as *mut u8;

    // Fondo (LIB_BG): el sobrante de la zona fija/banda queda del color de
    // la biblioteca (p. ej. bajo un contenido más corto que la ventana).
    fill_buffer(dst, dst_w, dst_h, dst_stride, bpp, LIB_BG_RGBA);

    if let Some(h) = header {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, h, 0, 0);
    }
    if let Some((band, origin)) = band {
        // Fila 0 de la banda (contenido-y = origin) se ve en pantalla en
        // `content_y0 − (scroll − origin)`: el scroll mueve solo la copia.
        let sy = content_y0 - (scroll as i32 - origin);
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, band, 0, sy);
    }
    // Aviso breve integrado en el MISMO present (antes un segundo lock+present
    // por frame durante el aviso).
    if let Some((t, tx, ty)) = toast {
        copy_region(dst, dst_w, dst_h, dst_stride, bpp, t, tx, ty);
    }
}
/// Resumen del filtro de BÚSQUEDA activo para el campo ("M" / "Download" /
/// "M · Download"): texto mostrable + si hay filtro (para el "✕").
fn search_summary(reader: &Reader) -> (String, bool) {
    let mut parts = Vec::new();
    if let Some(l) = reader.lib_letter {
        parts.push(l.to_string());
    }
    if let Some(f) = &reader.lib_folder {
        parts.push(f.trim_end_matches('/').to_string());
    }
    if parts.is_empty() {
        (String::new(), false)
    } else {
        (parts.join(" · "), true)
    }
}

/// Dibuja el EMPTY STATE de la biblioteca (sin PDFs): ilustración simple de
/// un libro (portada + lomo + líneas de texto) + título + subtítulo + botón
/// ("Add PDF" si el permiso está concedido, "Grant access" si no — el botón
/// Grant abre los Ajustes de \"All files access\"). Geometría COMPARTIDA con
/// el tap (`reader::lib_empty_state_geom`). `shift_y`: traslada todas las
/// coordenadas Y (px de ventana → banda de contenido, ver
/// `render_library_zone`); 0 = cocina normal (geometría ya en ventana).
fn draw_empty_state(
    reader: &Reader,
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    shift_y: f32,
) {
    let Some(g) = lib_empty_state_geom(reader) else {
        return;
    };
    let (bx, by, br, bb) = g.book;
    // Portada (silueta oscura) + lomo a la izquierda + líneas de "texto".
    rects.push(CanvasRect::rounded(
        bx,
        by + shift_y,
        br,
        bb + shift_y,
        6.0,
        theme::LIB_COVER_PLACEHOLDER,
    ));
    rects.push(CanvasRect::sharp(
        bx - 8.0,
        by + 4.0 + shift_y,
        bx - 2.0,
        bb - 4.0 + shift_y,
        0xFF141922,
    ));
    let line_w = (br - bx) * 0.6;
    for i in 0..3 {
        let ly = by + 18.0 + i as f32 * 18.0 + shift_y;
        rects.push(CanvasRect::sharp(
            bx + 12.0,
            ly,
            bx + 12.0 + line_w,
            ly + 2.0,
            0xFF232B3A,
        ));
    }
    // Título + subtítulo + botón.
    let (title, subtitle, btn_label) = if reader.permission_granted {
        (
            "Your library is empty",
            "Add your first PDF to start reading.",
            "Add PDF",
        )
    } else {
        (
            "Access needed",
            "Grant \u{201c}All files access\u{201d} to read your PDFs.",
            "Grant access",
        )
    };
    texts.push(CanvasText::new(
        reader.win_w as f32 / 2.0,
        g.title_y + shift_y,
        18.0,
        theme::LIB_TEXT_PRIMARY,
        TextAlign::Center,
        true,
        title,
    ));
    texts.push(CanvasText::new(
        reader.win_w as f32 / 2.0,
        g.subtitle_y + shift_y,
        13.0,
        theme::LIB_TEXT_MUTED,
        TextAlign::Center,
        false,
        subtitle,
    ));
    let (l, t, r, b) = g.button;
    draw_button(
        rects,
        texts,
        l,
        t + shift_y,
        r,
        b + shift_y,
        theme::LIB_ACCENT,
        theme::ACCENT_AMBER_BORDER,
        theme::LIB_ACCENT_DARK,
        (b - t) * 0.42,
        true,
        btn_label,
    );
}

/// Pega la portada escalada (center-crop, vecino-más-cercano) dentro del
/// bitmap base de la rejilla: escala la miniatura hasta RELLENAR el área 2:3
/// y centra el recorte (el sobrante se recorta, nunca letterbox); fuera de
/// las esquinas redondeadas de 12 px el píxel no se escribe y queda visible
/// el fondo con la sombra ya pintada en el bitmap base.
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

    // Center-crop (scale-to-fill): escala hasta rellenar el área 2:3 y centra
    // el recorte con `offset_x`/`offset_y`; el sobrante se recorta — nunca
    // letterbox. Mapeo de texel con BILINEAR en f32 (interpola los 4 texels
    // vecinos): a escalas ~1,5-1,8× (la portada de 240 px cabe en una celda
    // de ~365 px) el vecino-más-cercano se veía blocky/„estirado". Coste ~1 ms
    // por portada, pagado UNA vez por portada al cachearla/pegarla — nunca
    // por frame.
    let scale = (target_w as f64 / src_w as f64).max(target_h as f64 / src_h as f64);
    let dw = (src_w as f64 * scale) as f64;
    let dh = (src_h as f64 * scale) as f64;
    if dw < 1.0 || dh < 1.0 {
        return;
    }
    let offset_x = ((dw - target_w as f64) / 2.0).max(0.0);
    let offset_y = ((dh - target_h as f64) / 2.0).max(0.0);

    let r = 12.0f32;
    let r_sq = r * r;
    let tw_f = target_w as f32;
    let th_f = target_h as f32;

    // Tabla de origen x por columna destino (precisión f64, texel → centro):
    // `sx = ((tx + 0.5) * dw + offset_x - 0.5) / src_w` — misma convención
    // de centrado que `scale_bitmap` (bilinear de pdf_core, en su unidad).
    let mut xmap: Vec<(usize, usize, f32)> = Vec::with_capacity(target_w as usize);
    for tx in 0..target_w {
        let sx = ((tx as f64 + 0.5) * dw + offset_x - 0.5) / src_w as f64;
        let src_x = sx.max(0.0).min((src_w - 1) as f64);
        let x0 = src_x.floor() as usize;
        let x1 = (x0 + 1).min(src_w as usize - 1);
        xmap.push((x0, x1, (src_x - x0 as f64) as f32));
    }

    let src_stride = src_w as usize * 4;
    for ty in 0..target_h {
        let py = dy + ty;
        if py < 0 || py as usize >= dst_h {
            continue;
        }
        // Texel origen y con fracción (misma convención que x).
        let sy = ((ty as f64 + 0.5) * dh + offset_y - 0.5) / src_h as f64;
        let sy_c = sy.max(0.0).min((src_h - 1) as f64);
        let y0 = sy_c.floor() as usize;
        let y1 = (y0 + 1).min(src_h as usize - 1);
        let fy = (sy_c - y0 as f64) as f32;
        let row0 = &thumb.data[y0 * src_stride..];
        let row1 = &thumb.data[y1 * src_stride..];

        let fy = fy;
        for tx in 0..target_w {
            let px = dx + tx;
            if px < 0 || px as usize >= dst_w {
                continue;
            }

            let fx = tx as f32 + 0.5;
            let fy_px = ty as f32 + 0.5;
            // Esquinas redondeadas: fuera del radio el píxel no se escribe
            // (queda visible el fondo con la sombra del bitmap base).
            let (is_corner, dist_sq) = if fx < r && fy_px < r {
                let cdx = r - fx;
                let cdy = r - fy_px;
                (true, cdx * cdx + cdy * cdy)
            } else if fx > tw_f - r && fy_px < r {
                let cdx = fx - (tw_f - r);
                let cdy = r - fy_px;
                (true, cdx * cdx + cdy * cdy)
            } else if fx < r && fy_px > th_f - r {
                let cdx = r - fx;
                let cdy = fy_px - (th_f - r);
                (true, cdx * cdx + cdy * cdy)
            } else if fx > tw_f - r && fy_px > th_f - r {
                let cdx = fx - (tw_f - r);
                let cdy = fy_px - (th_f - r);
                (true, cdx * cdx + cdy * cdy)
            } else {
                (false, 0.0)
            };
            if is_corner && dist_sq > r_sq {
                continue;
            }

            let (x0, x1, fx) = xmap[tx as usize];
            let o = (py as usize * dst_w + px as usize) * 4;
            for c in 0..4usize {
                // Bilinear: top = p00 + (p10-p00)*fx; bottom = p01 + (p11-p01)*fx;
                // out = top + (bottom-top)*fy.
                let p00 = row0[x0 * 4 + c] as f32;
                let p10 = row0[x1 * 4 + c] as f32;
                let p01 = row1[x0 * 4 + c] as f32;
                let p11 = row1[x1 * 4 + c] as f32;
                let top = p00 + (p10 - p00) * fx;
                let bottom = p01 + (p11 - p01) * fx;
                dst[o + c] = (top + (bottom - top) * fy).round() as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------
// Selección de texto: menú flotante (Copiar/Subrayar/IA) y aviso breve
// ---------------------------------------------------------------------
//
// Geometría COMPARTIDA entre el render (Canvas+JNI) y el tap de `input`
// (`Reader::sel_menu.buttons`): el menú se coloca cerca del rect de
// selección (centrado en su x, encima si hay sitio y si no debajo), siempre
// dentro de la ventana.

/// Layout del menú de selección calculado por `sel_menu_layout`: rect del
/// menú en px de ventana (left, top, right, bottom) + botones (etiqueta +
/// rect en px de ventana). Lo consumen `render_sel_menu` (dibujo) y
/// `Reader::open_sel_menu` (estado para el tap de `input`).
pub(crate) struct SelMenuLayout {
    pub(crate) rect: (f32, f32, f32, f32),
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
}

/// Layout del menú de selección: rectángulo del menú en px de ventana
/// (`rect`) + botones (etiqueta + rect en px de ventana), compartido por
/// `render_sel_menu` y el tap de `input::sel_menu_tap`. `None` sin selección
/// fijada. Decisión documentada: el menú "flota" cerca del rect (anclado a
/// su centro x), arriba si cabe y si no debajo — nunca tapa el rect si se
/// puede evitar, y nunca se sale de la ventana.
pub(crate) fn sel_menu_layout(reader: &Reader) -> Option<SelMenuLayout> {
    let (sl, st, sr, sb) = reader.sel_screen_rect()?;
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    let pad = 10.0f32;
    let gap = 8.0f32;
    let bw = (win_w / 6.0).clamp(64.0, 120.0);
    let bh = (win_h / 28.0).clamp(36.0, 48.0);
    let menu_w = 3.0 * bw + 2.0 * gap + 2.0 * pad;
    let menu_h = bh + 2.0 * pad;
    let cx = (sl + sr) / 2.0;
    let x = (cx - menu_w / 2.0).clamp(8.0, (win_w - menu_w - 8.0).max(8.0));
    // Encima del rect si cabe (margen de 12 px); si no, debajo.
    let mut y = st - menu_h - 12.0;
    if y < 8.0 {
        y = sb + 12.0;
    }
    let y = y.clamp(8.0, (win_h - menu_h - 8.0).max(8.0));
    let mut buttons = Vec::with_capacity(3);
    for (i, label) in ["Copiar", "Subrayar", "IA"].into_iter().enumerate() {
        let bx = x + pad + i as f32 * (bw + gap);
        buttons.push((label, (bx, y + pad, bx + bw, y + pad + bh)));
    }
    Some(SelMenuLayout {
        rect: (x, y, x + menu_w, y + menu_h),
        buttons,
    })
}

/// Renderiza el menú de selección (tarjeta oscura redondeada + 3 botones
/// píldora: Copiar, Subrayar, IA) a un bitmap RGBA8 del tamaño del menú con
/// fondo transparente, usando la MISMA geometría que `sel_menu_layout` (las
/// coordenadas de los botones se desplazan al origen local del bitmap). "IA"
/// va atenuado: es el hueco visual que rellenará la Parte 2 (otro agente).
pub(crate) fn render_sel_menu(reader: &Reader) -> Option<Bitmap> {
    let layout = sel_menu_layout(reader)?;
    let (mx, my, mrx, mry) = layout.rect;
    let w = (mrx - mx) as i32;
    let h = (mry - my) as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    // Tarjeta: rect redondeado oscuro semitransparente (borde + relleno).
    let r = (h as f32) * 0.5;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, w as f32, h as f32, r, 0xFF232B3A,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        r,
        0xE6101216,
    ));
    for (label, (l, t, rr, b)) in &layout.buttons {
        let (fill, border, text_color) = match *label {
            "Subrayar" => (
                theme::ACCENT_AMBER_BG,
                theme::ACCENT_AMBER_BORDER,
                0xFF0B0D12,
            ),
            // Hueco visual de la Parte 2 (IA): atenuado, sin acción aún.
            "IA" => (0xFF161B26, 0xFF2A3444, theme::LIB_TEXT_MUTED),
            _ => (
                theme::DARK_BTN_BG,
                theme::DARK_BTN_BORDER,
                theme::DARK_BTN_TEXT,
            ),
        };
        draw_button(
            &mut rects,
            &mut texts,
            *l - mx,
            *t - my,
            *rr - mx,
            *b - my,
            fill,
            border,
            text_color,
            (*b - *t) * 0.38,
            true,
            label,
        );
    }
    jni_text_bitmap(w, h, 0x00000000, &rects, &texts)
}

/// Renderiza el aviso breve ("copied", "highlighted", "no text", ...) como
/// badge pequeño centrado sobre el indicador de página (mismo estilo que
/// `render_page_badge`). Cacheado en `Reader::toast_bitmap`; se expira en
/// `Reader::tick` a los `TOAST_MS`.
pub(crate) fn render_toast(reader: &Reader) -> Option<Bitmap> {
    let msg = reader.toast.as_ref()?.0.clone();
    let (bw, bh) = ((reader.win_w / 6).max(140), (reader.win_h / 60).max(30));
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
        msg,
    ));
    jni_text_bitmap(bw, bh, 0x00000000, &rects, &texts)
}

// ---------------------------------------------------------------------
// Panel de "Preguntar a la IA" (Parte 2): tarjeta flotante con respuesta
// ---------------------------------------------------------------------
//
// Misma filosofía que el menú de selección: geometría COMPARTIDA entre el
// render (Canvas+JNI) y el tap de `input` (`Reader::ai_panel.buttons`). El
// panel es una tarjeta centrada (horizontal y verticalmente) con cabecera
// (título + botones ✕/▲/▼) y cuerpo de texto envuelto en líneas. El texto
// se envuelve AQUÍ en Rust: se estima el ancho de cada carácter en ~0.52 ×
// tamaño de fuente (alfabeto latino; no hay medición real de glifos vía
// JNI) y cada línea es un `CanvasText`; las líneas fuera de la ventana de
// scroll se saltan, así que el recorte del cuerpo es gratis. Decisiones:
//
// - El alto del panel se AJUSTA al texto: si cabe entero (≤ 55 % de la
//   ventana) no hay scroll; si desborda, el cuerpo se limita y aparecen
//   los botones ▲/▼ (scroll por línea, `Reader::ai_scroll`).
// - La cabecera siempre muestra el botón ✕ (cerrar); un tap FUERA del panel
//   también lo cierra (`input::ai_panel_tap`).
// - El error (sin red / key inválida / error del proveedor) se muestra en
//   el MISMO panel, en rojo, con el mensaje de `AiError` (Display).

/// Layout del panel de IA calculado por `ai_panel_layout`: rect del panel en
/// px de ventana + botones (✕ siempre; ▲/▼ solo si desborda) + las líneas
/// envueltas del texto actual + conteos de scroll. Lo consumen
/// `render_ai_panel` (dibujo) y `Reader::rebuild_ai_panel` (estado para el
/// tap de `input`).
pub(crate) struct AiLayout {
    pub(crate) rect: (f32, f32, f32, f32),
    pub(crate) buttons: Vec<(&'static str, ButtonRect)>,
    pub(crate) lines: Vec<String>,
    pub(crate) scroll: usize,
    pub(crate) visible: usize,
    pub(crate) scrollable: bool,
}

/// Envuelve un texto en líneas de ≤ `max_chars` caracteres, respetando los
/// saltos de línea del texto (párrafos) y cortando palabras más largas que
/// la línea. `max_chars` es una ESTIMACIÓN (caracteres por línea, latino):
/// suficiente para un panel de lectura; la medición real de glifos exigiría
/// JNI (`Paint.measureText`), que se evita a propósito (cambios mínimos).
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new()); // línea en blanco: separa párrafos
            continue;
        }
        let mut cur = String::new();
        for word in para.split(' ') {
            if word.chars().count() > max_chars {
                // Palabra más larga que la línea: cortar en pedazos.
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                let mut rest = word;
                while rest.chars().count() > max_chars {
                    let cut = rest
                        .char_indices()
                        .nth(max_chars)
                        .map(|(i, _)| i)
                        .unwrap_or(rest.len());
                    let (a, b) = rest.split_at(cut);
                    out.push(a.to_string());
                    rest = b;
                }
                if !rest.is_empty() {
                    cur = rest.to_string();
                }
                continue;
            }
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.chars().count() + 1 + word.chars().count() <= max_chars {
                cur.push(' ');
                cur.push_str(word);
            } else {
                out.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        out.push(cur);
    }
    out
}

/// Layout del panel de IA: tarjeta centrada (margen 16 px, ancho ≤ 560 px),
/// cabecera fija de 44 px (título + botones a la derecha) y cuerpo cuyo alto
/// se ajusta al texto: si cabe entero, panel corto SIN scroll; si desborda,
/// cuerpo limitado a ~55 % de la ventana y botones ▲/▼. El scroll actual se
/// lee de `Reader::ai_panel` (0 al abrir) y se devuelve recortado al rango
/// válido para que `Reader::rebuild_ai_panel` lo guarde. None si la ventana
/// es demasiado pequeña para un panel.
pub(crate) fn ai_panel_layout(reader: &Reader) -> Option<AiLayout> {
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;
    if win_w < 200.0 || win_h < 200.0 {
        return None;
    }
    // Constantes del panel (las MISMAS en layout y render).
    let ts = 13.0f32; // cuerpo
    let line_h = (ts * 1.5).round(); // 20 px
    let pad = 14.0f32;
    let header_h = 44.0f32;
    let btn = 30.0f32;
    let gap = 6.0f32;
    let margin = 16.0f32;
    let panel_w = (win_w - 2.0 * margin).clamp(200.0, 560.0);
    let max_body_h = (win_h * 0.55).clamp(120.0, 340.0);
    // Envolver el texto (estimación de caracteres por línea para latino).
    let content_w = panel_w - 2.0 * pad;
    let max_chars = ((content_w / (ts * 0.52)).floor() as usize).max(8);
    let mut lines = wrap_text(&reader.ai_text, max_chars);
    if lines.is_empty() {
        lines.push("…".to_string()); // defensa: texto vacío
    }
    let total = lines.len();
    let visible = ((max_body_h / line_h).floor() as usize).max(1);
    let scrollable = total > visible;
    let body_h = if scrollable {
        max_body_h
    } else {
        (total as f32 * line_h).max(line_h)
    };
    let panel_h = header_h + body_h + pad;
    let x = (win_w - panel_w) / 2.0;
    let y = ((win_h - panel_h) / 2.0).max(8.0);
    // Botones de la cabecera, alineados a la derecha: [▲][▼][×] (▲/▼ solo
    // si `scrollable`). Misma geometría para render y tap.
    let mut buttons: Vec<(&'static str, ButtonRect)> = Vec::with_capacity(3);
    let btn_top = y + (header_h - btn) / 2.0;
    let btn_bottom = btn_top + btn;
    let mut bx = x + panel_w - pad - btn; // el más a la derecha: ✕
    buttons.push(("×", (bx, btn_top, bx + btn, btn_bottom)));
    if scrollable {
        bx -= btn + gap;
        buttons.push(("▼", (bx, btn_top, bx + btn, btn_bottom)));
        bx -= btn + gap;
        buttons.push(("▲", (bx, btn_top, bx + btn, btn_bottom)));
    }
    // Recortar el scroll actual al rango válido (0..total−visible).
    let max_scroll = total.saturating_sub(visible);
    let scroll = reader
        .ai_panel
        .as_ref()
        .map(|p| p.scroll)
        .unwrap_or(0)
        .min(max_scroll);
    Some(AiLayout {
        rect: (x, y, x + panel_w, y + panel_h),
        buttons,
        lines,
        scroll,
        visible,
        scrollable,
    })
}

/// Renderiza el panel de IA (tarjeta oscura redondeada + cabecera con título
/// y botones + cuerpo con las líneas VISIBLES, saltando las que quedan fuera
/// de la ventana de scroll) a un bitmap RGBA8 del tamaño del panel con fondo
/// transparente. Título y color del cuerpo según la fase (`AiPhase`):
/// Asking = "Preguntando a la IA…" en gris (estado transitorio), Answer =
/// texto claro, Error = "IA — error" con texto rojizo (`STATUS_*`).
pub(crate) fn render_ai_panel(reader: &Reader) -> Option<Bitmap> {
    let layout = ai_panel_layout(reader)?;
    let (mx, my, mrx, mry) = layout.rect;
    let w = (mrx - mx) as i32;
    let h = (mry - my) as i32;
    if w <= 0 || h <= 0 {
        return None;
    }
    let ts = 13.0f32;
    let line_h = (ts * 1.5).round();
    let pad = 14.0f32;
    let header_h = 44.0f32;
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    // Tarjeta: rect redondeado oscuro semitransparente (borde + relleno).
    let r = 16.0f32;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, w as f32, h as f32, r, 0xFF232B3A,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        w as f32 - 1.0,
        h as f32 - 1.0,
        (r - 1.0).max(0.0),
        0xF2101216,
    ));
    // Divisor bajo la cabecera.
    rects.push(CanvasRect::sharp(
        1.0,
        header_h,
        w as f32 - 1.0,
        header_h + 1.0,
        0xFF2A3444,
    ));
    // Cabecera: título según la fase + botones ✕/▲/▼ (draw_button).
    let (title, body_color) = match reader.ai_phase {
        AiPhase::Asking => ("Preguntando a la IA…", theme::LIB_TEXT_MUTED),
        AiPhase::Answer => ("Preguntar a la IA", theme::DARK_BTN_TEXT),
        AiPhase::Error => ("IA — error", theme::STATUS_TEXT),
    };
    texts.push(CanvasText::new(
        pad,
        header_h * 0.5 + 14.0 * 0.35,
        14.0,
        theme::DARK_BTN_TEXT,
        TextAlign::Left,
        true,
        title,
    ));
    for (label, (l, t, rr, b)) in &layout.buttons {
        draw_button(
            &mut rects,
            &mut texts,
            *l - mx,
            *t - my,
            *rr - mx,
            *b - my,
            theme::DARK_BTN_BG,
            theme::DARK_BTN_BORDER,
            theme::DARK_BTN_TEXT,
            14.0,
            false,
            label,
        );
    }
    // Cuerpo: solo las líneas visibles [scroll, scroll+visible); el resto se
    // salta (recorte del cuerpo sin bitmap intermedio).
    let body_top = header_h + pad;
    for (i, line) in layout
        .lines
        .iter()
        .enumerate()
        .skip(layout.scroll)
        .take(layout.visible)
    {
        let y = body_top + (i - layout.scroll) as f32 * line_h + ts * 0.9;
        texts.push(CanvasText::new(
            pad,
            y,
            ts,
            body_color,
            TextAlign::Left,
            false,
            line.clone(),
        ));
    }
    jni_text_bitmap(w, h, 0x00000000, &rects, &texts)
}

// ---------------------------------------------------------------------------
// Barra de herramientas de anotación (Fase 3.5: resaltador + boli)
// ---------------------------------------------------------------------------

/// Tamaño (ancho, alto) en px de la barra de herramientas del visor: píldora
/// centrada horizontalmente pegada al borde superior, con 5 botones
/// (Resaltar / Boli / ↶ / ● / ━ / →) — 6 botones. El ancho se adapta a la ventana
/// (los botones no se hacen gigantes en tablets muy anchas ni se desbordan en
/// ventanas estrechas); el alto es fijo (52 px, cómodo para el lápiz).
pub(crate) fn toolbar_size(win_w: i32, _win_h: i32) -> (i32, i32) {
    const GAP: i32 = 6;
    const INNER: i32 = 12;
    const BTN_H: i32 = 52;
    let bw = ((win_w - 2 * INNER - 5 * GAP) / 6).clamp(80, 130);
    (6 * bw + 5 * GAP + 2 * INNER, BTN_H)
}

/// Rect (left, top, right, bottom) en px de ventana de la barra de
/// herramientas (compartida por el render, el blit y los taps de `input`).
pub(crate) fn toolbar_rect(win_w: i32, win_h: i32) -> (f32, f32, f32, f32) {
    let (tw, th) = toolbar_size(win_w, win_h);
    let x0 = (win_w - tw) / 2;
    (x0 as f32, 12.0f32, (x0 + tw) as f32, (12.0 + th as f32))
}

/// Rect del botón flotante "✎" (toggle de la barra), esquina superior derecha
/// del visor (compartido por el render, el blit y los taps).
pub(crate) fn tool_fab_rect(win_w: i32, _win_h: i32) -> (f32, f32, f32, f32) {
    (win_w as f32 - 56.0, 14.0, win_w as f32 - 12.0, 58.0)
}

/// Botones de la barra de herramientas (izquierda → derecha): "Resaltar" y
/// "Boli" (activan la herramienta; el chip activo se dibuja con el acento
/// dorado), "↶" (deshacer el último trazo de la sesión, oculto cuando no
/// hay nada que deshacer), "●" (cicla el color del boli; se dibuja con el
/// color actual) y "→" (volver a modo navegación y cerrar la barra). La
/// geometría se comparte con `input::toolbar_tap`.
pub(crate) fn toolbar_buttons(
    _reader: &Reader,
    win_w: i32,
    win_h: i32,
) -> Vec<(&'static str, ButtonRect)> {
    let (tw, th) = toolbar_size(win_w, win_h);
    let (l, t, _, _) = toolbar_rect(win_w, win_h);
    let gap = 6.0f32;
    let inner = 12.0f32;
    let bw = (tw as f32 - 2.0 * inner - 5.0 * gap) / 6.0;
    let mut out = Vec::with_capacity(6);
    for (i, label) in ["Resaltar", "Boli", "↶", "●", "━", "→"]
        .into_iter()
        .enumerate()
    {
        let x0 = l + inner + i as f32 * (bw + gap);
        out.push((label, (x0, t, x0 + bw, t + th as f32)));
    }
    out
}

/// Renderiza la barra de herramientas del visor a un bitmap RGBA8 (Canvas+
/// JNI): píldora con borde, 5 botones con su estado (herramienta activa con
/// el acento dorado, ↶ atenuado sin trazos de esta sesión) y el círculo del
/// color actual del boli en el botón "●". Cacheado en `Reader::toolbar_bitmap`
/// (se invalida al alternar tool/color/modo oscuro o ventana).
pub(crate) fn render_toolbar(reader: &Reader) -> Option<Bitmap> {
    let (tw, th) = toolbar_size(reader.win_w, reader.win_h);
    let (l, t, _, _) = toolbar_rect(reader.win_w, reader.win_h);
    let (bar_bg, bar_border, btn_bg, btn_border, btn_text) = if reader.dark {
        (
            theme::DARK_BAR_BG,
            theme::DARK_BAR_BORDER,
            theme::DARK_BTN_BG,
            theme::DARK_BTN_BORDER,
            theme::DARK_BTN_TEXT,
        )
    } else {
        (
            theme::LIGHT_BAR_BG,
            theme::LIGHT_BAR_BORDER,
            theme::LIGHT_BTN_BG,
            theme::LIGHT_BTN_BORDER,
            theme::LIGHT_BTN_TEXT,
        )
    };
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();

    // Píldora de la barra: borde + relleno (esquinas totalmente redondeadas).
    let r = (th as f32) / 2.0;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, tw as f32, th as f32, r, bar_border,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        tw as f32 - 1.0,
        th as f32 - 1.0,
        (r - 1.0).max(0.0),
        bar_bg,
    ));

    let ts = 15.0f32;
    let has_undo = !reader.session_ids.is_empty();
    for (label, (bl, bt, br, bb)) in toolbar_buttons(reader, reader.win_w, reader.win_h).into_iter()
    {
        let bx = bl - l;
        let bw = br - bl;
        let active = (label == "Resaltar" && reader.tool == ToolKind::Highlight)
            || (label == "Boli" && reader.tool == ToolKind::Ink);
        let (fill, border, text_color) = if active {
            (
                theme::ACCENT_AMBER_BG,
                theme::ACCENT_AMBER_BORDER,
                0xFF0B0D12,
            )
        } else if label == "↶" && !has_undo {
            // Deshacer sin trazos de esta sesión: atenuado (no pulsable).
            (btn_bg, btn_border, theme::LIB_TEXT_MUTED)
        } else {
            (btn_bg, btn_border, btn_text)
        };
        let btop = bt - t;
        let bh = bb - bt;
        draw_button(
            &mut rects,
            &mut texts,
            bx + 2.0,
            btop + 2.0,
            bx + bw - 2.0,
            btop + bh - 2.0,
            fill,
            border,
            text_color,
            ts,
            true,
            if label == "●" { "" } else { label },
        );
        // Botón "●": además del botón, un círculo del color actual del boli.
        if label == "●" {
            let c = &reader.ink_color;
            // 0xAARRGGBB con alfa opaco: Android Paint.setColor lo necesita
            // completo (alpha 0 dejaría el círculo invisible).
            let ink = u32::from_be_bytes([0xFF, c.r, c.g, c.b]);
            let cx = bx + bw / 2.0;
            let cy = btop + bh / 2.0;
            rects.push(CanvasRect::rounded(
                cx - 9.0,
                cy - 9.0,
                cx + 9.0,
                cy + 9.0,
                999.0,
                ink,
            ));
        }
        // Botón "━": línea horizontal con grosor = ink_width actual.
        if label == "━" {
            let c = &reader.ink_color;
            let ink = u32::from_be_bytes([0xFF, c.r, c.g, c.b]);
            let cx = bx + bw / 2.0;
            let cy = btop + bh / 2.0;
            // Mapear pt a px preview: 1.0→3px, 2.5→5px, 4.0→7px, 7.0→10px
            let h = (reader.ink_width * 1.3).clamp(2.0, 10.0);
            let w = bw * 0.6;
            rects.push(CanvasRect::rounded(
                cx - w / 2.0,
                cy - h / 2.0,
                cx + w / 2.0,
                cy + h / 2.0,
                h / 2.0,
                ink,
            ));
        }
    }
    jni_text_bitmap(tw, th, 0x00000000, &rects, &texts)
}

/// Renderiza el botón flotante de toggle de la barra (esquina superior
/// derecha): "✎" cuando la barra está oculta, "✕" cuando está abierta.
/// Cacheado en `Reader::tool_fab` (invalida al togglear o cambiar ventana).
pub(crate) fn render_tool_fab(reader: &Reader) -> Option<Bitmap> {
    let (fw, fh) = (44i32, 44i32);
    let (bar_bg, bar_border, btn_bg, btn_border, btn_text) = if reader.dark {
        (
            theme::DARK_BAR_BG,
            theme::DARK_BAR_BORDER,
            theme::DARK_BTN_BG,
            theme::DARK_BTN_BORDER,
            theme::DARK_BTN_TEXT,
        )
    } else {
        (
            theme::LIGHT_BAR_BG,
            theme::LIGHT_BAR_BORDER,
            theme::LIGHT_BTN_BG,
            theme::LIGHT_BTN_BORDER,
            theme::LIGHT_BTN_TEXT,
        )
    };
    let mut rects: Vec<CanvasRect> = Vec::new();
    let mut texts: Vec<CanvasText> = Vec::new();
    let r = (fh as f32) / 2.0;
    rects.push(CanvasRect::rounded(
        0.0, 0.0, fw as f32, fh as f32, r, bar_border,
    ));
    rects.push(CanvasRect::rounded(
        1.0,
        1.0,
        fw as f32 - 1.0,
        fh as f32 - 1.0,
        (r - 1.0).max(0.0),
        bar_bg,
    ));
    draw_button(
        &mut rects,
        &mut texts,
        2.0,
        2.0,
        fw as f32 - 2.0,
        fh as f32 - 2.0,
        btn_bg,
        btn_border,
        btn_text,
        16.0,
        true,
        if reader.toolbar_open { "✕" } else { "✎" },
    );
    jni_text_bitmap(fw, fh, 0x00000000, &rects, &texts)
}

/// Bounding box en px de ventana de una anotación (todos sus puntos), con la
/// transformación `xform` ya aplicada: (min_x, min_y, max_x, max_y). Se usa
/// para dimensionar el bitmap de la capa temporal del trazo en curso.
fn ann_screen_bbox(ann: &Annotated, xform: &ViewTransform) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut acc = |x: f32, y: f32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };
    match &ann.kind {
        pdf_core::Annotation::Stroke(s) => {
            for &(x, y) in &s.points {
                let (px, py) = xform.page_to_screen(x, y);
                acc(px, py);
            }
        }
        pdf_core::Annotation::Highlight(h) => {
            for r in &h.rects {
                let (a, b) = xform.page_to_screen(r.x, r.y);
                let (c, d) = xform.page_to_screen(r.x + r.w, r.y + r.h);
                acc(a.min(c), b.min(d));
                acc(a.max(c), b.max(d));
            }
        }
        pdf_core::Annotation::TextNote(_) => return None,
    }
    if !min_x.is_finite() {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

/// Rasteriza la capa temporal de la anotación EN CURSO (trazo del boli o
/// rect del resaltador) en un bitmap RGBA del tamaño de su bbox de pantalla
/// (fondo transparente), usando `pdf_core::overlay::composite_annotations`
/// — el rasterizador vectorial de la capa de anotaciones de pdf_core (fill
/// de quads por scanline + trazos gruesos con AA de 1 px, sin allocaciones
/// por píxel). Devuelve `(bitmap, x, y)` para copiarlo con alfa-blend sobre
/// el frame (coste ∝ bbox del trazo, no ∝ página: el requisito 5 de
/// rendimiento — el visor NO re-blitea la página por evento de Move).
pub(crate) fn raster_tool_layer(
    win_w: i32,
    win_h: i32,
    xform: ViewTransform,
    ann: &Annotated,
    pad: f32,
) -> Option<(Bitmap, i32, i32)> {
    let (min_x, min_y, max_x, max_y) = ann_screen_bbox(ann, &xform)?;
    let l = (min_x - pad).floor().max(0.0) as i32;
    let t = (min_y - pad).floor().max(0.0) as i32;
    let r = (max_x + pad).ceil().min(win_w as f32) as i32;
    let b = (max_y + pad).ceil().min(win_h as f32) as i32;
    if r <= l || b <= t {
        return None;
    }
    let (w, h) = ((r - l) as usize, (b - t) as usize);
    let mut data = vec![0u8; w * h * 4]; // transparente: el trazo se funde al copiar
    let layer_xform = ViewTransform {
        zoom: xform.zoom,
        offset_x: xform.offset_x - l as f32,
        offset_y: xform.offset_y - t as f32,
    };
    let anns = [ann];
    // Variante alpha (Fase C): el bitmap del trazo EN CURSO debe
    // marcar alpha en los píxeles pintados — `composite_annotations` a secas
    // deja alpha=0 (diseñado para el bitmap opaco de página) y el blend las
    // saltaría: trazo invisible (bug 2026-08-24, ver overlay.rs).
    pdf_core::composite_annotations_alpha(&mut data, w as u32, h as u32, &anns, &layer_xform);
    Some((
        Bitmap {
            width: w as u32,
            height: h as u32,
            data,
        },
        l,
        t,
    ))
}

/// Snapshot de la pantalla de BIBLIOTECA (zona fija + banda actual) a un
/// bitmap RGBA8 del tamaño de la ventana: se captura justo antes de abrir un
/// libro y el visor lo funde sobre la página durante `LIB_FADE_MS`
/// (transición visual al abrir; ver `blit_lib_fade`). Puro memcpy por filas.
pub(crate) fn compose_library_snapshot(reader: &Reader) -> Option<Bitmap> {
    let w = reader.win_w;
    let h = reader.win_h;
    if w <= 0 || h <= 0 {
        return None;
    }
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.lib_search_open,
        reader.status.is_some(),
    );
    let mut out = Bitmap {
        width: w as u32,
        height: h as u32,
        data: vec![0u8; w as usize * h as usize * 4],
    };
    // Fondo base (LIB_BG) y planos cacheados.
    let dst = out.data.as_mut_ptr();
    fill_buffer(dst, w as usize, h as usize, w as usize, 4, LIB_BG_RGBA);
    if let Some(header) = reader.lib_header.as_ref() {
        copy_region(dst, w as usize, h as usize, w as usize, 4, header, 0, 0);
    }
    if let Some((band, origin)) = reader.lib_band.as_ref() {
        let sy = content_y0 - (reader.lib_scroll as i32 - *origin);
        copy_region(dst, w as usize, h as usize, w as usize, 4, band, 0, sy);
    }
    Some(out)
}

/// Funde sobre el buffer (un segundo lock+present TRANSITORIO) el snapshot
/// de la biblioteca capturado al abrir un libro: `out = snap·α + buf·(1−α)`
/// en RGB (el alfa del buffer se conserva). Alfa decreciente → la página
/// aparece bajo la portada de la biblioteca (transición de apertura). Se
/// aplica solo durante `LIB_FADE_MS` (~12 frames) justo después del blit de
/// la página.
pub(crate) fn blit_lib_fade(window: &NativeWindow, snap: &Bitmap, alpha: u8) {
    let Ok(mut guard) = window.lock(None) else {
        return;
    };
    let Some(bpp) = guard.format().bytes_per_pixel() else {
        return;
    };
    let dst_w = guard.width();
    let dst_h = guard.height();
    let dst_stride = guard.stride();
    let dst = guard.bits() as *mut u8;
    if bpp != 4 || snap.width != dst_w as u32 || snap.height != dst_h as u32 {
        // Formato inesperado: sin mezcla (seguridad; el visor fuerza RGBA8).
        let _ = bpp;
        return;
    }
    let a = alpha as usize;
    if a == 0 {
        return;
    }
    let inv = 255 - a;
    for y in 0..dst_h as usize {
        let row_dst = unsafe {
            std::slice::from_raw_parts_mut(dst.add(y * dst_stride * 4), dst_w as usize * 4)
        };
        let row_snap = &snap.data[y * dst_w as usize * 4..(y + 1) * dst_w as usize * 4];
        for px in 0..dst_w as usize {
            let d = px * 4;
            for c in 0..3usize {
                row_dst[d + c] =
                    ((row_snap[d + c] as usize * a + row_dst[d + c] as usize * inv) / 255) as u8;
            }
            // alfa del buffer intacto.
        }
    }
}
