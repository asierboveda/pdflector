// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Dibujo de la UI: submódulos por responsabilidad (blits, tinta, chrome del
//! visor, sheet, menús, biblioteca, overlays) + la infra Canvas+JNI que
//! comparten (aquí, en la raíz). `reader` decide QUÉ dibujar; `zoom`
//! reutiliza `primitives::fill_buffer`/`copy_region`.
//!
//! Submódulos:
//! - `primitives`: blits crudos del buffer (relleno, copias, RGB565).
//! - `chrome`: barras e indicadores del visor (página, goma).
//! - `toolbar`: barra de herramientas del visor y su popover.
//! - `sheet`: panel de ajustes deslizante.
//! - `menus`: picker de PDFs y menús ⋯/☰.
//! - `library`: pantalla de biblioteca (cabecera, zona, portadas).
//! - `overlays`: toast, panel IA, snapshot de fade.
//!
//! Los paths `crate::draw::X` del código previo se conservan con los
//! re-exports de abajo (consumidores: `reader`, `input`, `zoom`, `gpu`).

use jni::objects::JValue;
use jni::{JavaVM, jni_sig, jni_str};
use log::error;
use pdf_core::Bitmap;

mod ai_text;
mod chrome;
mod discover;
mod library;
mod menus;
mod overlays;
mod primitives;
mod sheet;
mod toolbar;
pub(crate) use ai_text::{AiLine, AiLineKind, format_ai_response};
pub(crate) use chrome::ButtonRect;
pub(crate) use chrome::render_eraser_cursor;
pub(crate) use chrome::render_page_badge;
pub(crate) use chrome::render_viewer_bottom_chrome;
pub(crate) use chrome::render_viewer_top_chrome;
pub(crate) use chrome::viewer_top_chrome_buttons;
pub(crate) use discover::draw_header_tabs;
pub(crate) use discover::render_discover_header;
pub(crate) use discover::render_discover_zone;
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
pub(crate) use overlays::AI_LINE_H;
pub(crate) use overlays::ai_panel_layout;
pub(crate) use overlays::compose_library_snapshot;
pub(crate) use overlays::render_ai_panel;
pub(crate) use overlays::render_toast;
pub(crate) use primitives::copy_region;
pub(crate) use primitives::fill_buffer;
pub(crate) use sheet::render_sheet;
pub(crate) use sheet::sheet_buttons;
pub(crate) use toolbar::PopoverCell;
pub(crate) use toolbar::ToolbarButton;
pub(crate) use toolbar::ToolbarDock;
pub(crate) use toolbar::ToolbarLayout;
pub(crate) use toolbar::ToolbarPopover;
pub(crate) use toolbar::render_toolbar;
pub(crate) use toolbar::render_toolbar_popover;
pub(crate) use toolbar::toolbar_layout;
pub(crate) use toolbar::toolbar_popover_layout;
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

/// Familia tipográfica nativa de Android (`android.graphics.Typeface`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TextFont {
    #[default]
    Sans,
    #[allow(dead_code)]
    Serif,
    #[allow(dead_code)]
    Mono,
}

/// Escala de elevación Astryx (`none | low | med | high`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Elevation {
    None,
    Low,
    Med,
    High,
}

/// Variantes semánticas de botón Astryx.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ButtonVariant {
    Primary,
    Secondary,
    Tonal,
    Ghost,
    Danger,
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
    pub(crate) font: TextFont,
    pub(crate) letter_spacing: f32,
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
            font: TextFont::Sans,
            letter_spacing: 0.0,
            text: text.into(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_font(mut self, font: TextFont) -> Self {
        self.font = font;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_tracking(mut self, em: f32) -> Self {
        self.letter_spacing = em;
        self
    }
}

/// Segmento recto con extremos redondeados (iconos de la barra de
/// herramientas): de (x0, y0) a (x1, y1), `width` px, color ARGB.
pub(crate) struct CanvasLine {
    pub(crate) x0: f32,
    pub(crate) y0: f32,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
    pub(crate) width: f32,
    pub(crate) color: u32,
}

impl CanvasLine {
    pub(crate) fn new(p0: (f32, f32), p1: (f32, f32), width: f32, color: u32) -> Self {
        Self {
            x0: p0.0,
            y0: p0.1,
            x1: p1.0,
            y1: p1.1,
            width,
            color,
        }
    }
}

/// Sombra graduada Astryx (`--shadow-low`, `--shadow-med`, `--shadow-high`).
/// En claro usa tinte azul-pizarra frío `rgba(5, 54, 89, α)`; en oscuro, negro profundo.
pub(crate) fn draw_elevation_shadow(
    rects: &mut Vec<CanvasRect>,
    (l, t, r, b): ButtonRect,
    radius: f32,
    elevation: Elevation,
    is_dark: bool,
) {
    let tint: u32 = if is_dark { 0x0000_0000 } else { 0x0005_3659 };
    let layer = |alpha: u32| -> u32 { (alpha << 24) | tint };
    match elevation {
        Elevation::None => {}
        Elevation::Low => {
            let (a1, a2) = if is_dark { (0x24, 0x3C) } else { (0x0A, 0x16) };
            rects.push(CanvasRect::rounded(
                l,
                t + 1.5,
                r,
                b + 3.5,
                radius + 1.0,
                layer(a1),
            ));
            rects.push(CanvasRect::rounded(
                l + 0.5,
                t + 1.0,
                r - 0.5,
                b + 2.0,
                radius,
                layer(a2),
            ));
        }
        Elevation::Med => {
            let (a1, a2, a3) = if is_dark {
                (0x1E, 0x38, 0x58)
            } else {
                (0x0B, 0x14, 0x22)
            };
            rects.push(CanvasRect::rounded(
                l - 1.5,
                t + 3.0,
                r + 1.5,
                b + 8.0,
                radius + 3.0,
                layer(a1),
            ));
            rects.push(CanvasRect::rounded(
                l - 0.5,
                t + 2.0,
                r + 0.5,
                b + 5.0,
                radius + 1.5,
                layer(a2),
            ));
            rects.push(CanvasRect::rounded(
                l + 0.5,
                t + 1.0,
                r - 0.5,
                b + 2.5,
                radius,
                layer(a3),
            ));
        }
        Elevation::High => {
            let (a1, a2, a3) = if is_dark {
                (0x28, 0x48, 0x6C)
            } else {
                (0x0E, 0x1A, 0x2C)
            };
            rects.push(CanvasRect::rounded(
                l - 3.0,
                t + 5.0,
                r + 3.0,
                b + 14.0,
                radius + 4.0,
                layer(a1),
            ));
            rects.push(CanvasRect::rounded(
                l - 1.5,
                t + 3.0,
                r + 1.5,
                b + 9.0,
                radius + 2.0,
                layer(a2),
            ));
            rects.push(CanvasRect::rounded(
                l,
                t + 1.5,
                r,
                b + 4.0,
                radius + 0.5,
                layer(a3),
            ));
        }
    }
}

/// Superficie Astryx (tarjeta, popover o diálogo) con elevación opcional y borde hairline 1 px.
#[allow(dead_code)]
pub(crate) fn draw_surface_card(
    rects: &mut Vec<CanvasRect>,
    rect: ButtonRect,
    radius: f32,
    fill: u32,
    border: u32,
    elevation: Elevation,
    is_dark: bool,
) {
    let (l, t, r, b) = rect;
    draw_elevation_shadow(rects, rect, radius, elevation, is_dark);
    rects.push(CanvasRect::rounded(l, t, r, b, radius, border));
    rects.push(CanvasRect::rounded(
        l + 1.0,
        t + 1.0,
        r - 1.0,
        b - 1.0,
        (radius - 1.0).max(0.0),
        fill,
    ));
}

/// Anillo de foco/selección Astryx (`--shadow-inset-selected` 2 px).
#[allow(dead_code)]
pub(crate) fn draw_inset_ring(
    rects: &mut Vec<CanvasRect>,
    (l, t, r, b): ButtonRect,
    radius: f32,
    ring_color: u32,
    inner_fill: u32,
) {
    rects.push(CanvasRect::rounded(l, t, r, b, radius, ring_color));
    rects.push(CanvasRect::rounded(
        l + 2.0,
        t + 2.0,
        r - 2.0,
        b - 2.0,
        (radius - 2.0).max(0.0),
        inner_fill,
    ));
}

/// Botón Astryx con radio de elemento (`--radius-element`, no píldora por defecto) y variantes semánticas.
#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) fn draw_astryx_button(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    (l, t, r, b): ButtonRect,
    variant: ButtonVariant,
    p: &crate::theme::ThemePalette,
    ts: f32,
    bold: bool,
    label: &str,
) {
    let radius = crate::theme::RADIUS_ELEMENT;
    let (fill, border, fg) = match variant {
        ButtonVariant::Primary => (p.accent, p.accent, p.on_accent),
        ButtonVariant::Secondary => (p.bg_card, p.border_emphasized, p.text_primary),
        ButtonVariant::Tonal => (p.accent_muted, p.accent_muted, p.on_accent_muted),
        ButtonVariant::Ghost => (p.bg_muted, p.border, p.text_primary),
        ButtonVariant::Danger => (p.error_bg, p.error_border, p.error_text),
    };
    rects.push(CanvasRect::rounded(l, t, r, b, radius, border));
    rects.push(CanvasRect::rounded(
        l + 1.0,
        t + 1.0,
        r - 1.0,
        b - 1.0,
        (radius - 1.0).max(0.0),
        fill,
    ));
    let cx = l + (r - l) * 0.5;
    let cy = t + (b - t) * 0.5 + ts * 0.35;
    texts.push(CanvasText::new(
        cx,
        cy,
        ts,
        fg,
        TextAlign::Center,
        bold,
        label,
    ));
}

/// Token / Badge Astryx (`--radius-full`) con relleno tonal semántico y tracking tipográfico.
#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) fn draw_token_pill(
    rects: &mut Vec<CanvasRect>,
    texts: &mut Vec<CanvasText>,
    (l, t, r, b): ButtonRect,
    fill: u32,
    border: Option<u32>,
    fg: u32,
    ts: f32,
    font: TextFont,
    tracking: f32,
    bold: bool,
    label: &str,
) {
    let rad = ((b - t) * 0.5).max(crate::theme::RADIUS_INNER);
    if let Some(bdr) = border {
        rects.push(CanvasRect::rounded(l, t, r, b, rad, bdr));
        rects.push(CanvasRect::rounded(
            l + 1.0,
            t + 1.0,
            r - 1.0,
            b - 1.0,
            (rad - 1.0).max(0.0),
            fill,
        ));
    } else {
        rects.push(CanvasRect::rounded(l, t, r, b, rad, fill));
    }
    let cx = l + (r - l) * 0.5;
    let cy = t + (b - t) * 0.5 + ts * 0.35;
    texts.push(
        CanvasText::new(cx, cy, ts, fg, TextAlign::Center, bold, label)
            .with_font(font)
            .with_tracking(tracking),
    );
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
    jni_canvas_bitmap(w, h, bg, rects, &[], texts)
}

/// Como `jni_text_bitmap`, con segmentos antialiasados de extremos
/// redondeados entre los rectángulos y los textos. Orden de dibujo: fondo →
/// rects → líneas → textos.
fn jni_canvas_bitmap(
    w: i32,
    h: i32,
    bg: u32,
    rects: &[CanvasRect],
    lines: &[CanvasLine],
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

            // Líneas (el estilo del Paint no afecta a drawLine: siempre traza).
            if !lines.is_empty() {
                let cap_class = env.find_class(jni_str!("android/graphics/Paint$Cap"))?;
                let cap_round = env
                    .get_static_field(
                        &cap_class,
                        jni_str!("ROUND"),
                        jni_sig!(sig = android.graphics.Paint::Cap),
                    )?
                    .l()?;
                env.call_method(
                    &paint,
                    jni_str!("setStrokeCap"),
                    jni_sig!(sig = (android.graphics.Paint::Cap) -> void),
                    &[JValue::Object(&cap_round)],
                )?;
                for l in lines {
                    env.call_method(
                        &paint,
                        jni_str!("setColor"),
                        jni_sig!(sig = (int) -> void),
                        &[JValue::Int(l.color as i32)],
                    )?;
                    env.call_method(
                        &paint,
                        jni_str!("setStrokeWidth"),
                        jni_sig!(sig = (float) -> void),
                        &[JValue::Float(l.width)],
                    )?;
                    env.call_method(
                        &canvas,
                        jni_str!("drawLine"),
                        jni_sig!(sig = (float, float, float, float, android.graphics.Paint) -> void),
                        &[
                            JValue::Float(l.x0),
                            JValue::Float(l.y0),
                            JValue::Float(l.x1),
                            JValue::Float(l.y1),
                            JValue::Object(&paint),
                        ],
                    )?;
                }
            }

            // Textos (tipografía Astryx: Sans / Serif / Mono + letter-spacing con estado cacheado).
            if !texts.is_empty() {
                let tf_class = env.find_class(jni_str!("android/graphics/Typeface"))?;
                let tf_sans = env
                    .get_static_field(
                        &tf_class,
                        jni_str!("SANS_SERIF"),
                        jni_sig!(sig = android.graphics.Typeface),
                    )?
                    .l()?;
                let tf_serif = env
                    .get_static_field(
                        &tf_class,
                        jni_str!("SERIF"),
                        jni_sig!(sig = android.graphics.Typeface),
                    )?
                    .l()?;
                let tf_mono = env
                    .get_static_field(
                        &tf_class,
                        jni_str!("MONOSPACE"),
                        jni_sig!(sig = android.graphics.Typeface),
                    )?
                    .l()?;
                let mut cur_font: Option<TextFont> = None;
                let mut cur_spacing: f32 = 0.0;

                for t in texts {
                    if cur_font != Some(t.font) {
                        let tf_obj = match t.font {
                            TextFont::Sans => &tf_sans,
                            TextFont::Serif => &tf_serif,
                            TextFont::Mono => &tf_mono,
                        };
                        let prev_tf = env
                            .call_method(
                                &paint,
                                jni_str!("setTypeface"),
                                jni_sig!(sig = (android.graphics.Typeface) -> android.graphics.Typeface),
                                &[JValue::Object(tf_obj)],
                            )?
                            .l()?;
                        env.delete_local_ref(prev_tf);
                        cur_font = Some(t.font);
                    }
                    if (t.letter_spacing - cur_spacing).abs() > f32::EPSILON {
                        env.call_method(
                            &paint,
                            jni_str!("setLetterSpacing"),
                            jni_sig!(sig = (float) -> void),
                            &[JValue::Float(t.letter_spacing)],
                        )?;
                        cur_spacing = t.letter_spacing;
                    }
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
            error!("jni_canvas_bitmap ({w}x{h}): {e}");
            None
        }
    }
}
