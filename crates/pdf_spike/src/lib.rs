// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Spike Fase 0 (PLAN-PARIDAD-STYLUS-NATIVO §4): comparar en la TCL dos
//! pipelines de presentación con el MISMO bucle de input:
//!
//! - `SW`  (baseline, el de pdf_android hoy): `ANativeWindow_lock` + fill +
//!   `unlockAndPost`. SurfaceFlinger compone después (2-3 frames extra).
//! - `EGL` (candidato A.2): contexto EGL/GLES2 sobre el mismo ANativeWindow +
//!   `eglSwapBuffers`. Sin copia CPU de 12 MB por frame.
//!
//! Interacción:
//! - Arrastre (stylus o dedo): deja puntos negros → trailing lag medible con
//!   cámara lenta (protocolo §4.1).
//! - Tap corto: alterna SW ↔ EGL.
//! - Cualquier tecla: salir.
//!
//! Telemetría en logcat (tag `pdf_spike`):
//! - `spike_mode <SW|EGL>`           — cambio de pipeline
//! - `spike_input <us>`              — delta timestamp evento→procesado
//! - `spike_present <SW|EGL> <ms>`   — duración del present
//! - `spike_frame <SW|EGL> <dt_ms>`  — intervalo entre presents
//!
//! En EGL el trazo se visualiza como cambio de color por frame (el clear
//! lleva el nivel del canal R ligado a la x del último punto) — suficiente
//! para medir el frente de color en cámara lenta sin escribir shaders.

pub mod prediction;

use android_activity::InputStatus;
use android_activity::input::MotionAction;
use android_activity::ndk::native_window::NativeWindow;
use android_activity::{AndroidApp, MainEvent, PollEvent};
use log::{info, warn};
use std::time::Instant;

// ---------------------------------------------------------------- FFI EGL/GLES

mod egl {
    #![allow(non_snake_case)]
    use std::ffi::c_void;

    pub type EGLDisplay = *mut c_void;
    pub type EGLConfig = *mut c_void;
    pub type EGLContext = *mut c_void;
    pub type EGLSurface = *mut c_void;
    pub const EGL_NO_SURFACE: EGLSurface = std::ptr::null_mut();
    pub const EGL_NO_CONTEXT: EGLContext = std::ptr::null_mut();
    pub const EGL_SURFACE_TYPE: i32 = 0x3033;
    pub const EGL_WINDOW_BIT: i32 = 0x0004;
    pub const EGL_RED_SIZE: i32 = 0x3024;
    pub const EGL_GREEN_SIZE: i32 = 0x3023;
    pub const EGL_BLUE_SIZE: i32 = 0x3022;
    pub const EGL_ALPHA_SIZE: i32 = 0x3021;
    pub const EGL_RENDERABLE_TYPE: i32 = 0x3040;
    pub const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
    pub const EGL_NONE: i32 = 0x3038;
    pub const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
    pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;

    #[link(name = "EGL")]
    unsafe extern "C" {
        pub fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
        pub fn eglInitialize(dpy: EGLDisplay, major: *mut i32, minor: *mut i32) -> u32;
        pub fn eglTerminate(dpy: EGLDisplay) -> u32;
        pub fn eglChooseConfig(
            dpy: EGLDisplay,
            attribs: *const i32,
            configs: *mut EGLConfig,
            num: i32,
            out: *mut i32,
        ) -> u32;
        pub fn eglCreateWindowSurface(
            dpy: EGLDisplay,
            cfg: EGLConfig,
            win: *mut c_void,
            attrs: *const i32,
        ) -> EGLSurface;
        pub fn eglCreateContext(
            dpy: EGLDisplay,
            cfg: EGLConfig,
            share: EGLContext,
            attrs: *const i32,
        ) -> EGLContext;
        pub fn eglMakeCurrent(
            dpy: EGLDisplay,
            draw: EGLSurface,
            read: EGLSurface,
            ctx: EGLContext,
        ) -> u32;
        pub fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> u32;
        pub fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> u32;
        pub fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> u32;
    }

    #[link(name = "GLESv2")]
    unsafe extern "C" {
        pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
        pub fn glClear(mask: u32);
        pub fn glViewport(x: i32, y: i32, w: i32, h: i32);
    }
}

struct EglCtx {
    display: egl::EGLDisplay,
    surface: egl::EGLSurface,
    context: egl::EGLContext,
}

impl EglCtx {
    /// RGBA8888 + GLES2 sobre el ANativeWindow. Sin depth/stencil.
    unsafe fn new(win: &NativeWindow) -> Option<EglCtx> {
        unsafe {
            let dpy = egl::eglGetDisplay(std::ptr::null_mut());
            if dpy.is_null() {
                warn!("eglGetDisplay null");
                return None;
            }
            let (mut maj, mut min) = (0i32, 0i32);
            if egl::eglInitialize(dpy, &mut maj, &mut min) == 0 {
                warn!("eglInitialize failed");
                return None;
            }
            info!("EGL {}.{} initialized", maj, min);
            let attribs = [
                egl::EGL_SURFACE_TYPE,
                egl::EGL_WINDOW_BIT,
                egl::EGL_RED_SIZE,
                8,
                egl::EGL_GREEN_SIZE,
                8,
                egl::EGL_BLUE_SIZE,
                8,
                egl::EGL_ALPHA_SIZE,
                8,
                egl::EGL_RENDERABLE_TYPE,
                egl::EGL_OPENGL_ES2_BIT,
                egl::EGL_NONE,
            ];
            let mut cfg = [std::ptr::null_mut() as egl::EGLConfig; 1];
            let mut n = 0i32;
            if egl::eglChooseConfig(dpy, attribs.as_ptr(), cfg.as_mut_ptr(), 1, &mut n) == 0
                || n < 1
            {
                warn!("eglChooseConfig: {} configs", n);
                return None;
            }
            let surf = egl::eglCreateWindowSurface(
                dpy,
                cfg[0],
                win.ptr().as_ptr().cast(),
                std::ptr::null(),
            );
            if surf.is_null() {
                warn!("eglCreateWindowSurface failed");
                return None;
            }
            const CTX_ATTRS: [i32; 3] = [egl::EGL_CONTEXT_CLIENT_VERSION, 2, egl::EGL_NONE];
            let ctx = egl::eglCreateContext(dpy, cfg[0], egl::EGL_NO_CONTEXT, CTX_ATTRS.as_ptr());
            if ctx.is_null() {
                warn!("eglCreateContext failed");
                return None;
            }
            if egl::eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
                warn!("eglMakeCurrent failed");
                return None;
            }
            Some(EglCtx {
                display: dpy,
                surface: surf,
                context: ctx,
            })
        }
    }

    unsafe fn present(&mut self, w: i32, h: i32, clear: [f32; 4]) {
        unsafe {
            egl::glViewport(0, 0, w, h);
            egl::glClearColor(clear[0], clear[1], clear[2], clear[3]);
            egl::glClear(egl::GL_COLOR_BUFFER_BIT);
            egl::eglSwapBuffers(self.display, self.surface);
        }
    }
}

impl Drop for EglCtx {
    fn drop(&mut self) {
        unsafe {
            egl::eglMakeCurrent(
                self.display,
                egl::EGL_NO_SURFACE,
                egl::EGL_NO_SURFACE,
                egl::EGL_NO_CONTEXT,
            );
            egl::eglDestroySurface(self.display, self.surface);
            egl::eglDestroyContext(self.display, self.context);
            egl::eglTerminate(self.display);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Sw,
    Egl,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Sw => "SW",
            Mode::Egl => "EGL",
        }
    }
}

// ---------------------------------------------------------------- Spike

struct Spike {
    mode: Mode,
    last: Option<(f32, f32)>,
    moved: bool,
    last_present: Option<Instant>,
    egl: Option<EglCtx>,
    win_w: i32,
    win_h: i32,
}

impl Spike {
    fn new() -> Spike {
        Spike {
            mode: Mode::Sw,
            last: None,
            moved: false,
            last_present: None,
            egl: None,
            win_w: 0,
            win_h: 0,
        }
    }

    fn toggle(&mut self) {
        self.mode = if self.mode == Mode::Sw {
            Mode::Egl
        } else {
            Mode::Sw
        };
        info!("spike_mode {}", self.mode.name());
    }

    /// Fill blanco con banda que cambia por frame (SW): parpadeo visible en
    /// cámara lenta sin tocar los píxeles del trazo.
    fn present_sw(&mut self, win: &NativeWindow, pts: &[(f32, f32)]) {
        let Ok(mut guard) = win.lock(None) else {
            warn!("lock failed");
            return;
        };
        let Some(bpp) = guard.format().bytes_per_pixel() else {
            return;
        };
        // En ndk 0.9 width/height/stride/bpp ya son usize.
        let (w, h, stride, bpp) = (guard.width(), guard.height(), guard.stride(), bpp);
        let base = guard.bits() as *mut u8;
        unsafe {
            for y in 0..h {
                let row = base.add(y * stride * bpp);
                let px: [u8; 4] = [240, 240, 240, 255];
                for x in 0..w {
                    std::ptr::copy_nonoverlapping(px.as_ptr(), row.add(x * bpp), 4);
                }
            }
            for (x, y) in pts {
                let (xi, yi) = (*x as i32, *y as i32);
                for dy in -8i32..8 {
                    for dx in -8i32..8 {
                        let (px, py) = (xi + dx, yi + dy);
                        if (0..w as i32).contains(&px) && (0..h as i32).contains(&py) {
                            let off = (py as usize * stride + px as usize) * bpp;
                            std::ptr::copy_nonoverlapping(
                                [20u8, 20, 20, 255].as_ptr(),
                                base.add(off),
                                4,
                            );
                        }
                    }
                }
            }
        }
        drop(guard); // unlock_and_post
    }

    fn present(&mut self, win: Option<&NativeWindow>, pts: &[(f32, f32)]) {
        let t0 = Instant::now();
        match self.mode {
            Mode::Sw => {
                if let Some(w) = win {
                    self.present_sw(w, pts);
                }
            }
            Mode::Egl => {
                // Creación perezosa del contexto EGL.
                if self.egl.is_none()
                    && let Some(w) = win
                {
                    self.egl = unsafe { EglCtx::new(w) };
                    info!("spike_mode EGL ready");
                }
                if let Some(ctx) = self.egl.as_mut() {
                    // Nivel de rojo ligado a la x del último punto: frente
                    // de color medible en cámara lenta.
                    let last = pts.last().or(self.last.as_ref());
                    let r = last
                        .map(|(x, _)| (x / self.win_w.max(1) as f32).clamp(0.05, 0.95))
                        .unwrap_or(0.5);
                    unsafe { ctx.present(self.win_w, self.win_h, [r, 0.85, 0.2, 1.0]) };
                }
            }
        }
        let now = Instant::now();
        info!(
            "spike_present {} {:.2}",
            self.mode.name(),
            (now - t0).as_secs_f64() * 1000.0
        );
        if let Some(prev) = self.last_present {
            info!(
                "spike_frame {} {:.2}",
                self.mode.name(),
                (now - prev).as_secs_f64() * 1000.0
            );
        }
        self.last_present = Some(now);
    }
}

// ---------------------------------------------------------------- android_main

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("pdf_spike"),
    );
    info!("spike: inicio");
    let mut spike = Spike::new();
    let mut running = true;

    while running {
        app.poll_events(
            Some(std::time::Duration::from_millis(16)),
            |event| match event {
                PollEvent::Main(MainEvent::InitWindow { .. })
                | PollEvent::Main(MainEvent::WindowResized { .. }) => {
                    if let Some(w) = app.native_window() {
                        spike.win_w = w.width();
                        spike.win_h = w.height();
                        // Redibujar el fondo al (re)crear la ventana.
                        let w2 = w.clone();
                        spike.present(Some(&w2), &[]);
                    }
                    spike.last = None;
                }
                PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                    spike.egl = None;
                }
                PollEvent::Main(MainEvent::Destroy) => running = false,
                PollEvent::Main(MainEvent::InputAvailable) => {
                    if let Ok(mut iter) = app.input_events_iter() {
                        while iter.next(|ev| match ev {
                            android_activity::input::InputEvent::MotionEvent(m) => {
                                // Latencia de entrega evento→app: event_time usa
                                // CLOCK_MONOTONIC (no wall clock). Comparamos contra
                                // clock_gettime(CLOCK_MONOTONIC) vía Instant? Instant
                                // también es CLOCK_MONOTONIC en Android, pero sus
                                // instantes no son comparables con el epoch. Para
                                // no introducir una métrica errónea, el delta
                                // evento→procesado se obtiene con systrace/Perfetto
                                // (protocolo §4.2); aquí solo se marca el instante
                                // de procesado en el log.
                                let _ = m.event_time();

                                let Some(p) = m.pointers().next() else {
                                    return InputStatus::Handled;
                                };
                                let (x, y) = (p.x(), p.y());
                                match m.action() {
                                    MotionAction::Down => {
                                        spike.last = Some((x, y));
                                        spike.moved = false;
                                    }
                                    MotionAction::Move => {
                                        spike.moved = true;
                                        // History del evento: muestras a 240 Hz
                                        // agrupadas (mismo drain que pdf_android).
                                        let mut pts: Vec<(f32, f32)> = Vec::new();
                                        for hp in p.history() {
                                            pts.push((hp.x(), hp.y()));
                                        }
                                        pts.push((x, y));
                                        let win = app.native_window();
                                        spike.present(win.as_ref(), &pts);
                                    }
                                    MotionAction::Up => {
                                        if !spike.moved {
                                            spike.toggle();
                                        }
                                        spike.last = None;
                                    }
                                    MotionAction::Cancel => spike.last = None,
                                    _ => {}
                                }
                                InputStatus::Handled
                            }
                            android_activity::input::InputEvent::KeyEvent(_) => {
                                running = false;
                                InputStatus::Handled
                            }
                            _ => InputStatus::Unhandled,
                        }) {}
                    }
                }
                _ => {}
            },
        );
    }
    info!("spike: fin");
}
