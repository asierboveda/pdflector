// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Dibujo de la UI: submódulos por responsabilidad (blits, tinta, chrome del
//! visor, sheet, menús, biblioteca, overlays) + la infra Canvas+JNI que
//! comparten (aquí, en la raíz). `reader` decide QUÉ dibujar; `zoom`
//! reutiliza `primitives::fill_buffer`/`copy_region`.
//!
//! Submódulos:
//! - `primitives`: blits crudos del buffer (relleno, copias, RGB565).
//! - `tinta`: rasterizador de trazos (brocha de disco + Liang–Barsky).
//! - `chrome`: barras e indicadores del visor (página, modo, goma).
//! - `sheet`: panel de ajustes deslizante.
//! - `menus`: picker de PDFs y menús ⋯/☰.
//! - `library`: pantalla de biblioteca (cabecera, zona, portadas).
//! - `overlays`: menú de selección, toast, panel IA, snapshot de fade.
//! Los paths `crate::draw::X` del código previo se conservan con los
//! re-exports de abajo (consumidores: `reader`, `input`, `zoom`, `gpu`).

use jni::objects::JValue;
use jni::{JavaVM, jni_sig, jni_str};
use log::error;
use pdf_core::Bitmap;

mod chrome;
mod library;
mod menus;
mod overlays;
mod primitives;
mod sheet;
mod tinta;
pub(crate) use chrome::ButtonRect;
pub(crate) use chrome::mode_badge_rect;
pub(crate) use chrome::render_eraser_cursor;
pub(crate) use chrome::render_mode_badge;
pub(crate) use chrome::render_page_badge;
pub(crate) use chrome::render_viewer_bottom_chrome;
pub(crate) use chrome::render_viewer_top_chrome;
pub(crate) use chrome::viewer_top_chrome_buttons;
pub(crate) use library::blit_library;
pub(crate) use library::paste_lib_thumbs;
pub(crate) use library::render_library_header;
pub(crate) use library::render_library_zone;
pub(crate) use library::render_search_chip_row;
pub(crate) use library::splice_row;
pub(crate) use menus::SettingsMenuItem;
pub(crate) use menus::ViewMenuItem;
pub(crate) use menus::draw_settings_menu;
pub(crate) use menus::draw_view_menu;
pub(crate) use menus::render_picker_list;
pub(crate) use menus::settings_menu_geometry;
pub(crate) use menus::view_menu_geometry;
pub(crate) use overlays::ai_panel_layout;
pub(crate) use overlays::compose_library_snapshot;
pub(crate) use overlays::render_ai_panel;
pub(crate) use overlays::render_sel_menu;
pub(crate) use overlays::render_toast;
pub(crate) use overlays::sel_menu_layout;
pub(crate) use primitives::copy_region;
pub(crate) use primitives::fill_buffer;
pub(crate) use sheet::render_sheet;
pub(crate) use sheet::sheet_buttons;
pub(crate) use tinta::draw_ink_segment_on_frame;
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

/// Dibuja sombra multicapa suave y visible (G1: alpha >= 0x30 en light, >= 0x60 en dark)
fn draw_card_shadow(
    rects: &mut Vec<CanvasRect>,
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    radius: f32,
    is_dark: bool,
) {
    let (s1, s2, s3) = if is_dark {
        (0x28000000, 0x48000000, 0x70000000)
    } else {
        (0x10000000, 0x20000000, 0x34000000)
    };
    rects.push(CanvasRect::rounded(
        l - 1.0,
        t + 3.0,
        r + 1.0,
        b + 7.0,
        radius + 2.0,
        s1,
    ));
    rects.push(CanvasRect::rounded(
        l,
        t + 2.0,
        r,
        b + 5.0,
        radius + 1.0,
        s2,
    ));
    rects.push(CanvasRect::rounded(
        l + 1.0,
        t + 1.0,
        r - 1.0,
        b + 3.0,
        radius,
        s3,
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
