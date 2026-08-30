// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pipeline de presentación GPU (Fase 2 de PLAN-PARIDAD-STYLUS-NATIVO,
//! ADR-006): contexto EGL/GLES2 sobre el ANativeWindow del visor. La página
//! renderizada por MuPDF se sube como textura SOLO cuando cambia (cambio de
//! página o re-render nítido); la tinta (trazos guardados + gesto en curso +
//! tramo predicho) se dibuja como geometría vectorial (quads con AA analítico
//! en el fragment shader); los overlays (chrome, toolbar, sheet, menús,
//! badge, toast, cursor de goma) son quads texturizados de los bitmaps
//! Canvas+JNI que `draw::render_*` ya genera. Present con `eglSwapBuffers`
//! (spike 1: p50 0.17 ms vs 3.75 ms del lock+post).
//!
//! Solo el modo VISOR presenta por GPU. La biblioteca y el picker siguen por
//! SW (`ANativeWindow_lock`): el ciclo de vida destruye la surface EGL antes
//! de ese lock y la recrea al volver al visor (lock y swap NUNCA coexisten en
//! el mismo frame).
//!
//! FFI EGL/GLES2 propio (declaraciones de las APIs públicas de Khronos;
//! licencia de este fichero, no de los headers): los crates de bindings
//! (`khronos-egl`) exigen pkg-config o `dlopen("libEGL.so.1")` — nombre que
//! no existe en Android (`libEGL.so`) — así que el spike ya validó este FFI
//! directo en la TCL.

use android_activity::ndk::native_window::NativeWindow;
use log::{info, warn};

use crate::reader::Reader;
use pdf_core::Bitmap;

// ------------------------------------------------------------------ FFI EGL

pub mod ffi {
    #![allow(non_snake_case)]
    use std::ffi::c_void;

    pub type EGLDisplay = *mut c_void;
    pub type EGLConfig = *mut c_void;
    pub type EGLContext = *mut c_void;
    pub type EGLSurface = *mut c_void;
    pub type GLhandle = u32;

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
    #[allow(dead_code)]
    pub const EGL_FRONT_BUFFER_AUTO_REFRESH_ANDROID: i32 = 0x314C;
    #[allow(dead_code)]
    pub const EGL_TRUE: u32 = 1;
    #[allow(dead_code)]
    pub const EGL_FALSE: u32 = 0;

    // GLES2 constants (public Khronos values).
    pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
    pub const GL_TRUE: u32 = 1;
    pub const GL_FLOAT: u32 = 0x1406;
    pub const GL_BLEND: u32 = 0x0BE2;
    pub const GL_ONE: u32 = 1;
    pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
    pub const GL_TEXTURE_2D: u32 = 0x0DE1;
    pub const GL_TEXTURE0: u32 = 0x84C0;
    pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
    pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
    pub const GL_TEXTURE_WRAP_R: u32 = 0x8072;
    pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
    pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
    pub const GL_LINEAR: u32 = 0x2601;
    pub const GL_CLAMP_TO_EDGE: u32 = 0x812F;
    pub const GL_RGBA: u32 = 0x1908;
    pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
    pub const GL_ARRAY_BUFFER: u32 = 0x8892;
    pub const GL_STREAM_DRAW: u32 = 0x88E0;
    pub const GL_TRIANGLE_STRIP: u32 = 0x0005;
    pub const GL_FRAGMENT_SHADER: u32 = 0x8B30;
    pub const GL_VERTEX_SHADER: u32 = 0x8B31;
    pub const GL_COMPILE_STATUS: u32 = 0x8B81;
    pub const GL_LINK_STATUS: u32 = 0x8B82;
    pub const GL_FRAMEBUFFER: u32 = 0x8D40;
    pub const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    pub const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    #[allow(dead_code)]
    pub const GL_SCISSOR_TEST: u32 = 0x0C11;
    #[link(name = "EGL")]
    unsafe extern "C" {
        pub fn glBlendFuncSeparate(sfactor: u32, dfactor: u32, alpha_s: u32, alpha_d: u32);

        pub fn glEnable(cap: u32);

        pub fn glDisable(cap: u32);

        pub fn glDeleteProgram(p: GLhandle);

        pub fn glGenBuffers(n: i32, out: *mut u32);

        pub fn glDeleteShader(sh: GLhandle);

        pub fn glGetProgramiv(p: GLhandle, pname: u32, out: *mut i32);

        pub fn glLinkProgram(p: GLhandle);

        pub fn glAttachShader(p: GLhandle, sh: GLhandle);

        pub fn glCreateProgram() -> GLhandle;

        pub fn glGetShaderiv(sh: GLhandle, pname: u32, out: *mut i32);

        pub fn glCompileShader(sh: GLhandle);

        pub fn glShaderSource(sh: GLhandle, count: i32, src: *const *const u8, len: *const i32);

        pub fn glCreateShader(ty: u32) -> GLhandle;

        pub fn glGetString(name: u32) -> *const u8;

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
        pub fn eglSwapInterval(dpy: EGLDisplay, interval: i32) -> u32;
        #[allow(dead_code)]
        pub fn eglSurfaceAttrib(
            dpy: EGLDisplay,
            surface: EGLSurface,
            attribute: i32,
            value: i32,
        ) -> u32;
        pub fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> u32;
        pub fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> u32;
    }

    #[link(name = "GLESv2")]
    unsafe extern "C" {
        pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
        pub fn glClear(mask: u32);
        pub fn glViewport(x: i32, y: i32, w: i32, h: i32);
        #[allow(dead_code)]
        pub fn glFlush();
        pub fn glUseProgram(p: GLhandle);
        pub fn glGetUniformLocation(p: GLhandle, name: *const u8) -> i32;
        pub fn glGetAttribLocation(p: GLhandle, name: *const u8) -> i32;
        pub fn glUniform1i(loc: i32, v: i32);
        pub fn glUniform1f(loc: i32, v: f32);
        pub fn glUniform2f(loc: i32, x: f32, y: f32);
        pub fn glUniformMatrix3fv(loc: i32, n: i32, transpose: u8, v: *const f32);
        pub fn glGenTextures(n: i32, out: *mut u32);
        pub fn glDeleteTextures(n: i32, t: *const u32);
        pub fn glBindTexture(target: u32, t: u32);
        pub fn glTexParameteri(target: u32, pname: u32, v: i32);
        pub fn glTexImage2D(
            target: u32,
            level: i32,
            internal: i32,
            w: i32,
            h: i32,
            border: i32,
            fmt: u32,
            ty: u32,
            data: *const u8,
        );
        pub fn glTexSubImage2D(
            target: u32,
            level: i32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            fmt: u32,
            ty: u32,
            data: *const u8,
        );
        pub fn glActiveTexture(unit: u32);
        pub fn glBindBuffer(target: u32, b: u32);
        pub fn glBufferData(target: u32, size: isize, data: *const u8, usage: u32);
        pub fn glVertexAttribPointer(
            index: u32,
            size: i32,
            ty: u32,
            norm: u8,
            stride: i32,
            ptr: *const u8,
        );
        pub fn glEnableVertexAttribArray(index: u32);
        pub fn glDisableVertexAttribArray(index: u32);
        pub fn glDrawArrays(mode: u32, first: i32, count: i32);
        pub fn glGenFramebuffers(n: i32, framebuffers: *mut u32);
        pub fn glDeleteFramebuffers(n: i32, framebuffers: *const u32);
        pub fn glBindFramebuffer(target: u32, framebuffer: u32);
        pub fn glFramebufferTexture2D(
            target: u32,
            attachment: u32,
            textarget: u32,
            texture: u32,
            level: i32,
        );
        pub fn glCheckFramebufferStatus(target: u32) -> u32;
        #[allow(dead_code)]
        pub fn glScissor(x: i32, y: i32, width: i32, height: i32);
    }
}

use ffi as gl;

/// Vertex de tinta: posición en PANTALLA (px, y abajo) + semi-grosor del
/// trazo en px de pantalla (para el AA del fragment) + RGBA8.
#[repr(C)]
struct InkVert {
    x: f32,
    y: f32,
    hw: f32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

const INK_VERT_SIZE: usize = std::mem::size_of::<InkVert>(); // 16 bytes

// ---------------------------------------------------------------- Shaders

const VS_QUAD_SRC: &[u8] = b"
attribute vec2 aPos;
attribute vec2 aUV;
uniform mat3 uMvp;
uniform vec2 uRes;
varying vec2 vUV;
void main() {
    vec2 px = (uMvp * vec3(aPos, 1.0)).xy;
    vec2 ndc = vec2(2.0 * px.x / uRes.x - 1.0, 1.0 - 2.0 * px.y / uRes.y);
    gl_Position = vec4(ndc, 0.0, 1.0);
    vUV = aUV;
}
\0";

const FS_TEX_SRC: &[u8] = b"
precision mediump float;
uniform sampler2D uTex;
uniform bool uDark;
varying vec2 vUV;
void main() {
    vec4 c = texture2D(uTex, vUV);
    if (uDark) {
        gl_FragColor = vec4(1.0 - c.r, 1.0 - c.g, 1.0 - c.b, 1.0);
    } else {
        gl_FragColor = vec4(c.r, c.g, c.b, 1.0);
    }
}
\0";

const FS_OVERLAY_SRC: &[u8] = b"
precision mediump float;
uniform sampler2D uTex;
uniform float uAlpha;
varying vec2 vUV;
void main() {
    vec4 c = texture2D(uTex, vUV);
    gl_FragColor = vec4(c.rgb * uAlpha, c.a * uAlpha);
}
\0";

const VS_INK_SRC: &[u8] = b"
attribute vec2 aPos;
attribute float aHw;
attribute vec4 aColor;
uniform mat3 uMvp;
uniform vec2 uRes;
varying float vHw;
varying vec4 vColor;
void main() {
    vec2 px = (uMvp * vec3(aPos, 1.0)).xy;
    vec2 ndc = vec2(2.0 * px.x / uRes.x - 1.0, 1.0 - 2.0 * px.y / uRes.y);
    gl_Position = vec4(ndc, 0.0, 1.0);
    vHw = aHw;
    vColor = aColor;
}
\0";

const FS_INK_SRC: &[u8] = b"
precision mediump float;
varying float vHw;
varying vec4 vColor;
void main() {
    gl_FragColor = vec4(vColor.rgb * vColor.a, vColor.a);
}
\0";

struct QuadProg {
    prog: u32,
    a_pos: i32,
    a_uv: i32,
    u_mvp: i32,
    u_res: i32,
    u_tex: i32,
    /// Solo el programa de página: `uDark` (inversión de RGB en el shader);
    /// -1 en el de overlays.
    u_dark: i32,
    u_alpha: i32,
}

struct InkProg {
    prog: u32,
    a_pos: i32,
    a_hw: i32,
    a_color: i32,
    u_mvp: i32,
    u_res: i32,
}

fn compile(typ: u32, src: &[u8]) -> Option<u32> {
    unsafe {
        let sh = gl::glCreateShader(typ);
        let ptr: *const u8 = src.as_ptr();
        gl::glShaderSource(sh, 1, &ptr, std::ptr::null());
        gl::glCompileShader(sh);
        let mut ok = 0;
        gl::glGetShaderiv(sh, gl::GL_COMPILE_STATUS, &mut ok);
        if ok == gl::GL_TRUE as i32 {
            Some(sh)
        } else {
            gl::glDeleteShader(sh);
            None
        }
    }
}

fn link(vs: &[u8], fs: &[u8]) -> Option<u32> {
    unsafe {
        let v = compile(gl::GL_VERTEX_SHADER, vs)?;
        let f = compile(gl::GL_FRAGMENT_SHADER, fs)?;
        let p = gl::glCreateProgram();
        gl::glAttachShader(p, v);
        gl::glAttachShader(p, f);
        gl::glLinkProgram(p);
        gl::glDeleteShader(v);
        gl::glDeleteShader(f);
        let mut ok = 0;
        gl::glGetProgramiv(p, gl::GL_LINK_STATUS, &mut ok);
        if ok == gl::GL_TRUE as i32 {
            Some(p)
        } else {
            gl::glDeleteProgram(p);
            None
        }
    }
}

/// MVP 3×3 por columnas (column-major, como espera GL): `screen = M · [x, y, 1]`
/// — escala sx/sy + traslación tx/ty (px de pantalla).
fn mat3_scale_translate(sx: f32, sy: f32, tx: f32, ty: f32) -> [f32; 9] {
    [
        sx, 0.0, 0.0, //
        0.0, sy, 0.0, //
        tx, ty, 1.0,
    ]
}

fn gl_str(ptr: *const u8) -> String {
    unsafe {
        let mut n = 0usize;
        while *ptr.add(n) != 0 {
            n += 1;
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr, n)).into_owned()
    }
}

// ---------------------------------------------------------------- Gpu

/// Contexto EGL + recursos GLES2 del visor. La surface se destruye y recrea
/// con la ventana (resize/TerminateWindow, o al pasar a modos SW); el display
/// y el contexto sobreviven entre surfaces. `Drop` desconecta en orden
/// inverso (patrón del spike, validado en TCL).
pub(crate) struct Gpu {
    dpy: gl::EGLDisplay,
    ctx: gl::EGLContext,
    cfg: gl::EGLConfig,
    surf: Option<gl::EGLSurface>,
    win_w: i32,
    win_h: i32,
    #[allow(dead_code)]
    front_buffer_active: bool,
    prog_tex: QuadProg,
    prog_ovl: QuadProg,
    prog_ink: InkProg,
    page_tex: u32,
    page_tex_w: i32,
    page_tex_h: i32,
    page_loaded: Option<(u32, f32, u32)>,
    vbo_quad: u32,
    vbo_ink: u32,
    ovl_cache: Vec<OverlayTex>,

    // --- Fase W1/W2/W3: Pipeline Dual FBO (Wet / Dry Ink) ---
    dry_fbo: u32,
    dry_tex: u32,
    dry_dirty: bool,
    dry_key: Option<DryKey>,

    wet_fbo: u32,
    wet_tex: u32,

    // Búfers de trabajo prealocados: cero alocaciones en el hot path (Fase W3)
    ink_scratch: Vec<InkVert>,
    pts_scratch: Vec<(f32, f32)>,
    current_swap_interval: i32,
}

/// Clave de invalidación de la capa base persistente (Dry FBO).
#[derive(Clone, Copy, Debug, PartialEq)]
struct DryKey {
    page: u32,
    zoom_bits: u32,
    pan_x: i32,
    pan_y: i32,
    ann_count: usize,
    dark: bool,
    chrome_visible: bool,
    toolbar_open: bool,
    sheet_progress_bits: u32,
    has_toast: bool,
}

struct OverlayTex {
    key_ptr: *const u8,
    _keep: Vec<u8>,
    tex: u32,
}

impl Gpu {
    /// Crea display + contexto (una vez por proceso) y la surface para `win`.
    pub(crate) unsafe fn new(
        win: &android_activity::ndk::native_window::NativeWindow,
    ) -> Option<Self> {
        unsafe {
            Self::create_display()
                .and_then(|(dpy, cfg, ctx)| Self::with_display(dpy, cfg, ctx, win))
        }
    }

    unsafe fn create_display() -> Option<(gl::EGLDisplay, gl::EGLConfig, gl::EGLContext)> {
        unsafe {
            let dpy = gl::eglGetDisplay(std::ptr::null_mut());
            if dpy.is_null() {
                warn!("eglGetDisplay failed");
                return None;
            }
            let (mut maj, mut min) = (0i32, 0i32);
            if gl::eglInitialize(dpy, &mut maj, &mut min) == 0 {
                warn!("eglInitialize failed");
                return None;
            }
            let attribs = [
                gl::EGL_SURFACE_TYPE,
                gl::EGL_WINDOW_BIT,
                gl::EGL_RED_SIZE,
                8,
                gl::EGL_GREEN_SIZE,
                8,
                gl::EGL_BLUE_SIZE,
                8,
                gl::EGL_ALPHA_SIZE,
                8,
                gl::EGL_RENDERABLE_TYPE,
                gl::EGL_OPENGL_ES2_BIT,
                gl::EGL_NONE,
            ];
            let mut cfg = [std::ptr::null_mut() as gl::EGLConfig; 1];
            let mut n = 0i32;
            if gl::eglChooseConfig(dpy, attribs.as_ptr(), cfg.as_mut_ptr(), 1, &mut n) == 0 || n < 1
            {
                warn!("eglChooseConfig: {n} configs");
                return None;
            }
            let ctx_attrs = [gl::EGL_CONTEXT_CLIENT_VERSION, 2, gl::EGL_NONE];
            let ctx = gl::eglCreateContext(dpy, cfg[0], gl::EGL_NO_CONTEXT, ctx_attrs.as_ptr());
            if ctx.is_null() {
                warn!("eglCreateContext failed");
                return None;
            }
            Some((dpy, cfg[0], ctx))
        }
    }

    unsafe fn with_display(
        dpy: gl::EGLDisplay,
        cfg: gl::EGLConfig,
        ctx: gl::EGLContext,
        win: &android_activity::ndk::native_window::NativeWindow,
    ) -> Option<Self> {
        unsafe {
            let surf =
                gl::eglCreateWindowSurface(dpy, cfg, win.ptr().as_ptr().cast(), std::ptr::null());
            if surf.is_null() {
                warn!("eglCreateWindowSurface failed");
                return None;
            }
            if gl::eglMakeCurrent(dpy, surf, surf, ctx) == 0 {
                warn!("eglMakeCurrent failed");
                gl::eglDestroySurface(dpy, surf);
                return None;
            }
            gl::eglSwapInterval(dpy, 1);
            let mut gpu = Self::make_resources(dpy, cfg, ctx, surf)?;
            gpu.win_w = win.width();
            gpu.win_h = win.height();
            gl::glViewport(0, 0, gpu.win_w, gpu.win_h);

            let (dry_fbo, dry_tex) = Self::create_fbo_with_tex(gpu.win_w, gpu.win_h);
            let (wet_fbo, wet_tex) = Self::create_fbo_with_tex(gpu.win_w, gpu.win_h);
            gpu.dry_fbo = dry_fbo;
            gpu.dry_tex = dry_tex;
            gpu.dry_dirty = true;
            gpu.wet_fbo = wet_fbo;
            gpu.wet_tex = wet_tex;

            info!(
                "gpu: EGL/GLES2 ready {}x{} renderer {} (Dual FBO Wet/Dry active)",
                gpu.win_w,
                gpu.win_h,
                gl_str(gl::glGetString(0x1F01)) // GL_RENDERER
            );
            Some(gpu)
        }
    }

    unsafe fn make_resources(
        dpy: gl::EGLDisplay,
        cfg: gl::EGLConfig,
        ctx: gl::EGLContext,
        surf: gl::EGLSurface,
    ) -> Option<Self> {
        unsafe {
            gl::glDisable(0x0B71); // GL_DEPTH_TEST
            gl::glEnable(gl::GL_BLEND);
            gl::glBlendFuncSeparate(
                gl::GL_ONE,
                gl::GL_ONE_MINUS_SRC_ALPHA,
                gl::GL_ONE,
                gl::GL_ONE_MINUS_SRC_ALPHA,
            );

            let p = link(VS_QUAD_SRC, FS_TEX_SRC)?;
            let prog_tex = QuadProg {
                prog: p,
                a_pos: gl::glGetAttribLocation(p, c"aPos".as_ptr()),
                a_uv: gl::glGetAttribLocation(p, c"aUV".as_ptr()),
                u_mvp: gl::glGetUniformLocation(p, c"uMvp".as_ptr()),
                u_res: gl::glGetUniformLocation(p, c"uRes".as_ptr()),
                u_tex: gl::glGetUniformLocation(p, c"uTex".as_ptr()),
                u_dark: gl::glGetUniformLocation(p, c"uDark".as_ptr()),
                u_alpha: -1,
            };
            let p = link(VS_QUAD_SRC, FS_OVERLAY_SRC)?;
            let prog_ovl = QuadProg {
                prog: p,
                a_pos: gl::glGetAttribLocation(p, c"aPos".as_ptr()),
                a_uv: gl::glGetAttribLocation(p, c"aUV".as_ptr()),
                u_mvp: gl::glGetUniformLocation(p, c"uMvp".as_ptr()),
                u_res: gl::glGetUniformLocation(p, c"uRes".as_ptr()),
                u_tex: gl::glGetUniformLocation(p, c"uTex".as_ptr()),
                u_dark: -1,
                u_alpha: gl::glGetUniformLocation(p, c"uAlpha".as_ptr()),
            };
            let p = link(VS_INK_SRC, FS_INK_SRC)?;
            let prog_ink = InkProg {
                prog: p,
                a_pos: gl::glGetAttribLocation(p, c"aPos".as_ptr()),
                a_hw: gl::glGetAttribLocation(p, c"aHw".as_ptr()),
                a_color: gl::glGetAttribLocation(p, c"aColor".as_ptr()),
                u_mvp: gl::glGetUniformLocation(p, c"uMvp".as_ptr()),
                u_res: gl::glGetUniformLocation(p, c"uRes".as_ptr()),
            };
            let mut vbo_quad = 0u32;
            gl::glGenBuffers(1, &mut vbo_quad);
            let mut vbo_ink = 0u32;
            gl::glGenBuffers(1, &mut vbo_ink);
            let mut page_tex = 0u32;
            gl::glGenTextures(1, &mut page_tex);
            Some(Self {
                dpy,
                ctx,
                cfg,
                surf: Some(surf),
                win_w: 0,
                win_h: 0,
                front_buffer_active: false,
                prog_tex,
                prog_ovl,
                prog_ink,
                page_tex,
                page_tex_w: 0,
                page_tex_h: 0,
                page_loaded: None,
                vbo_quad,
                vbo_ink,
                ovl_cache: Vec::new(),
                dry_fbo: 0,
                dry_tex: 0,
                dry_dirty: true,
                dry_key: None,
                wet_fbo: 0,
                wet_tex: 0,
                ink_scratch: Vec::with_capacity(2048),
                pts_scratch: Vec::with_capacity(1024),
                current_swap_interval: 1,
            })
        }
    }

    /// (Re)crea la surface para una ventana (resize / re-init del visor).
    pub(crate) fn recreate_surface(&mut self, win: &NativeWindow) {
        unsafe {
            self.drop_surface_only();
            let surf = gl::eglCreateWindowSurface(
                self.dpy,
                self.cfg,
                win.ptr().as_ptr().cast(),
                std::ptr::null(),
            );
            if surf.is_null() {
                warn!("eglCreateWindowSurface (recreate) failed");
                return;
            }
            if gl::eglMakeCurrent(self.dpy, surf, surf, self.ctx) == 0 {
                warn!("eglMakeCurrent (recreate) failed");
                return;
            }
            gl::eglSwapInterval(self.dpy, 1);
            self.surf = Some(surf);
            self.win_w = win.width();
            self.win_h = win.height();
            gl::glViewport(0, 0, self.win_w, self.win_h);
            self.page_loaded = None;

            // Recrear FBOs con la nueva resolución
            Self::destroy_fbo_with_tex(self.dry_fbo, self.dry_tex);
            Self::destroy_fbo_with_tex(self.wet_fbo, self.wet_tex);
            let (dry_fbo, dry_tex) = Self::create_fbo_with_tex(self.win_w, self.win_h);
            let (wet_fbo, wet_tex) = Self::create_fbo_with_tex(self.win_w, self.win_h);
            self.dry_fbo = dry_fbo;
            self.dry_tex = dry_tex;
            self.dry_dirty = true;
            self.dry_key = None;
            self.wet_fbo = wet_fbo;
            self.wet_tex = wet_tex;
        }
    }

    /// Suelta la surface (sin tocar el contexto).
    pub(crate) fn drop_surface(&mut self) {
        if self.surf.is_some() {
            unsafe { self.drop_surface_only() };
        }
    }

    unsafe fn drop_surface_only(&mut self) {
        unsafe {
            Self::destroy_fbo_with_tex(self.dry_fbo, self.dry_tex);
            Self::destroy_fbo_with_tex(self.wet_fbo, self.wet_tex);
            self.dry_fbo = 0;
            self.dry_tex = 0;
            self.wet_fbo = 0;
            self.wet_tex = 0;
            self.dry_dirty = true;
            self.dry_key = None;

            if let Some(s) = self.surf.take() {
                gl::eglMakeCurrent(
                    self.dpy,
                    gl::EGL_NO_SURFACE,
                    gl::EGL_NO_SURFACE,
                    gl::EGL_NO_CONTEXT,
                );
                gl::eglDestroySurface(self.dpy, s);
            }
        }
    }

    pub(crate) fn has_surface(&self) -> bool {
        self.surf.is_some()
    }

    fn clear(&mut self, rgba: [u8; 4]) {
        unsafe {
            gl::glClearColor(
                rgba[0] as f32 / 255.0,
                rgba[1] as f32 / 255.0,
                rgba[2] as f32 / 255.0,
                rgba[3] as f32 / 255.0,
            );
            gl::glClear(gl::GL_COLOR_BUFFER_BIT);
        }
    }

    // ------------------------------------------------- página como textura

    pub(crate) fn upload_page_if_needed(&mut self, page: u32, rendered_zoom: f32, bmp: &Bitmap) {
        if self.page_loaded == Some((page, rendered_zoom, bmp.data.len() as u32)) {
            return;
        }
        unsafe {
            gl::glBindTexture(gl::GL_TEXTURE_2D, self.page_tex);
            if self.page_tex_w != bmp.width as i32 || self.page_tex_h != bmp.height as i32 {
                gl::glTexImage2D(
                    gl::GL_TEXTURE_2D,
                    0,
                    gl::GL_RGBA as i32,
                    bmp.width as i32,
                    bmp.height as i32,
                    0,
                    gl::GL_RGBA,
                    gl::GL_UNSIGNED_BYTE,
                    bmp.data.as_ptr(),
                );
                self.page_tex_w = bmp.width as i32;
                self.page_tex_h = bmp.height as i32;
            } else {
                gl::glTexSubImage2D(
                    gl::GL_TEXTURE_2D,
                    0,
                    0,
                    0,
                    bmp.width as i32,
                    bmp.height as i32,
                    gl::GL_RGBA,
                    gl::GL_UNSIGNED_BYTE,
                    bmp.data.as_ptr(),
                );
            }
        }
        self.page_loaded = Some((page, rendered_zoom, bmp.data.len() as u32));
    }

    fn set_tex_filter(&self) {
        unsafe {
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MIN_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MAG_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_S,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_T,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_R,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
        }
    }

    // ------------------------------------------------- quads y overlays

    fn draw_bitmap(&mut self, b: &Bitmap, x: i32, y: i32, alpha: f32) {
        let tex = self.overlay_tex(b);
        unsafe {
            gl::glActiveTexture(gl::GL_TEXTURE0);
            gl::glBindTexture(gl::GL_TEXTURE_2D, tex);
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MIN_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MAG_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_S,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_T,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_R,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glUseProgram(self.prog_ovl.prog);
            let m = mat3_scale_translate(1.0, 1.0, 0.0, 0.0);
            gl::glUniformMatrix3fv(self.prog_ovl.u_mvp, 1, 0, m.as_ptr());
            gl::glUniform2f(self.prog_ovl.u_res, self.win_w as f32, self.win_h as f32);
            gl::glUniform1i(self.prog_ovl.u_tex, 0);
            gl::glUniform1f(self.prog_ovl.u_alpha, alpha);

            let (bw, bh) = (b.width as f32, b.height as f32);
            let (x0, y0) = (x as f32, y as f32);
            let (x1, y1) = (x0 + bw, y0 + bh);
            let uv = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let pos = [x0, y0, x1, y0, x0, y1, x1, y1];
            let mut verts: [f32; 24] = [0.0; 24];
            for i in 0..4 {
                verts[i * 6] = pos[i * 2];
                verts[i * 6 + 1] = pos[i * 2 + 1];
                verts[i * 6 + 2] = uv[i * 2];
                verts[i * 6 + 3] = uv[i * 2 + 1];
            }
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, self.vbo_quad);
            gl::glBufferData(
                gl::GL_ARRAY_BUFFER,
                std::mem::size_of_val(&verts) as isize,
                verts.as_ptr() as *const u8,
                gl::GL_STREAM_DRAW,
            );
            gl::glEnableVertexAttribArray(self.prog_ovl.a_pos as u32);
            gl::glVertexAttribPointer(
                self.prog_ovl.a_pos as u32,
                2,
                gl::GL_FLOAT,
                0,
                24,
                std::ptr::null(),
            );
            gl::glEnableVertexAttribArray(self.prog_ovl.a_uv as u32);
            gl::glVertexAttribPointer(
                self.prog_ovl.a_uv as u32,
                2,
                gl::GL_FLOAT,
                0,
                24,
                8 as *const u8,
            );
            gl::glDrawArrays(gl::GL_TRIANGLE_STRIP, 0, 4);
            gl::glDisableVertexAttribArray(self.prog_ovl.a_pos as u32);
            gl::glDisableVertexAttribArray(self.prog_ovl.a_uv as u32);
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, 0);
        }
    }

    fn overlay_tex(&mut self, b: &Bitmap) -> u32 {
        let key = b.data.as_ptr();
        if let Some(o) = self.ovl_cache.iter().find(|o| o.key_ptr == key) {
            return o.tex;
        }
        let mut tex = 0u32;
        unsafe {
            gl::glGenTextures(1, &mut tex);
            gl::glBindTexture(gl::GL_TEXTURE_2D, tex);
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MIN_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MAG_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_S,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_T,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexImage2D(
                gl::GL_TEXTURE_2D,
                0,
                gl::GL_RGBA as i32,
                b.width as i32,
                b.height as i32,
                0,
                gl::GL_RGBA,
                gl::GL_UNSIGNED_BYTE,
                b.data.as_ptr(),
            );
        }
        self.ovl_cache.push(OverlayTex {
            key_ptr: key,
            _keep: b.data.clone(),
            tex,
        });
        if self.ovl_cache.len() > 8 {
            let old = self.ovl_cache.remove(0);
            unsafe {
                gl::glDeleteTextures(1, &old.tex);
            }
        }
        tex
    }

    fn draw_solid_quad(&mut self, l: f32, t: f32, r: f32, b: f32, rgba: [u8; 4]) {
        if !l.is_finite() || !t.is_finite() || !r.is_finite() || !b.is_finite() {
            return;
        }
        self.draw_ink_triangles(&[
            InkVert {
                x: l,
                y: t,
                hw: 0.0,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            },
            InkVert {
                x: r,
                y: t,
                hw: 0.0,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            },
            InkVert {
                x: l,
                y: b,
                hw: 0.0,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            },
            InkVert {
                x: r,
                y: b,
                hw: 0.0,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            },
        ]);
    }

    fn draw_ink_triangles(&mut self, verts: &[InkVert]) {
        if verts.is_empty() {
            return;
        }
        unsafe {
            gl::glUseProgram(self.prog_ink.prog);
            let id = mat3_scale_translate(1.0, 1.0, 0.0, 0.0);
            gl::glUniformMatrix3fv(self.prog_ink.u_mvp, 1, 0, id.as_ptr());
            gl::glUniform2f(self.prog_ink.u_res, self.win_w as f32, self.win_h as f32);
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, self.vbo_ink);
            gl::glBufferData(
                gl::GL_ARRAY_BUFFER,
                std::mem::size_of_val(verts) as isize,
                verts.as_ptr() as *const u8,
                gl::GL_STREAM_DRAW,
            );
            let stride = INK_VERT_SIZE as i32;
            gl::glEnableVertexAttribArray(self.prog_ink.a_pos as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_pos as u32,
                2,
                gl::GL_FLOAT,
                0,
                stride,
                std::ptr::null(),
            );
            gl::glEnableVertexAttribArray(self.prog_ink.a_hw as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_hw as u32,
                1,
                gl::GL_FLOAT,
                0,
                stride,
                8 as *const u8,
            );
            gl::glEnableVertexAttribArray(self.prog_ink.a_color as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_color as u32,
                4,
                gl::GL_UNSIGNED_BYTE,
                1,
                stride,
                12 as *const u8,
            );
            gl::glDrawArrays(gl::GL_TRIANGLE_STRIP, 0, verts.len() as i32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_pos as u32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_hw as u32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_color as u32);
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, 0);
        }
    }

    /// Trazo como TRIANGLE_STRIP: genera vértices directamente sobre `self.ink_scratch`
    /// para cero alocaciones en heap en el hot path.
    fn draw_polyline_gpu(&mut self, pts: &[(f32, f32)], hw: f32, rgba: [u8; 4]) {
        if !hw.is_finite() || hw <= 0.0 {
            return;
        }
        if pts.len() < 2 {
            if let Some(&(x, y)) = pts.first()
                && x.is_finite()
                && y.is_finite()
            {
                self.draw_solid_quad(x - hw, y - hw, x + hw, y + hw, rgba);
            }
            return;
        }

        self.ink_scratch.clear();
        for (i, &(x, y)) in pts.iter().enumerate() {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            let prev = if i > 0 { pts[i - 1] } else { (x, y) };
            let next = if i + 1 < pts.len() {
                pts[i + 1]
            } else {
                (x, y)
            };
            let prev = if prev.0.is_finite() && prev.1.is_finite() {
                prev
            } else {
                (x, y)
            };
            let next = if next.0.is_finite() && next.1.is_finite() {
                next
            } else {
                (x, y)
            };
            let (dx, dy) = (next.0 - prev.0, next.1 - prev.1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-3);
            let (nx, ny) = (-dy / len, dx / len);
            self.ink_scratch.push(InkVert {
                x: x + nx * hw,
                y: y + ny * hw,
                hw,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            });
            self.ink_scratch.push(InkVert {
                x: x - nx * hw,
                y: y - ny * hw,
                hw,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            });
        }

        if !self.ink_scratch.is_empty() {
            let verts = std::mem::take(&mut self.ink_scratch);
            self.draw_ink_triangles(&verts);
            self.ink_scratch = verts;
        }
    }

    // ------------------------------------------------- FBO helpers y Pipeline Wet/Dry

    /// Invalida explícitamente la capa base (Dry FBO) para forzar su re-render.
    #[allow(dead_code)]
    pub(crate) fn invalidate_dry(&mut self) {
        self.dry_dirty = true;
    }

    /// Fija el swap interval de EGL si difiere del actual (0 = inmediato, 1 = 120 Hz vsync).
    pub(crate) fn set_swap_interval(&mut self, interval: i32) {
        if self.current_swap_interval == interval {
            return;
        }
        unsafe {
            gl::eglSwapInterval(self.dpy, interval);
            self.current_swap_interval = interval;
        }
    }

    /// Crea un Framebuffer Object (FBO) con textura RGBA8 adjunta del tamaño dado.
    unsafe fn create_fbo_with_tex(w: i32, h: i32) -> (u32, u32) {
        if w <= 0 || h <= 0 {
            return (0, 0);
        }
        unsafe {
            let mut fbo = 0u32;
            let mut tex = 0u32;
            gl::glGenFramebuffers(1, &mut fbo);
            gl::glGenTextures(1, &mut tex);

            gl::glBindTexture(gl::GL_TEXTURE_2D, tex);
            gl::glTexImage2D(
                gl::GL_TEXTURE_2D,
                0,
                gl::GL_RGBA as i32,
                w,
                h,
                0,
                gl::GL_RGBA,
                gl::GL_UNSIGNED_BYTE,
                std::ptr::null(),
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MIN_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MAG_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_S,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_T,
                gl::GL_CLAMP_TO_EDGE as i32,
            );

            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, fbo);
            gl::glFramebufferTexture2D(
                gl::GL_FRAMEBUFFER,
                gl::GL_COLOR_ATTACHMENT0,
                gl::GL_TEXTURE_2D,
                tex,
                0,
            );

            let status = gl::glCheckFramebufferStatus(gl::GL_FRAMEBUFFER);
            if status != gl::GL_FRAMEBUFFER_COMPLETE {
                warn!("glCheckFramebufferStatus failed: 0x{:x}", status);
            }

            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
            gl::glBindTexture(gl::GL_TEXTURE_2D, 0);

            (fbo, tex)
        }
    }

    unsafe fn destroy_fbo_with_tex(fbo: u32, tex: u32) {
        unsafe {
            if fbo != 0 {
                gl::glDeleteFramebuffers(1, &fbo);
            }
            if tex != 0 {
                gl::glDeleteTextures(1, &tex);
            }
        }
    }

    /// Dibuja una textura a pantalla completa en el framebuffer actualmente vinculado.
    fn draw_fullscreen_texture(&mut self, tex: u32, alpha: f32) {
        if tex == 0 {
            return;
        }
        unsafe {
            gl::glActiveTexture(gl::GL_TEXTURE0);
            gl::glBindTexture(gl::GL_TEXTURE_2D, tex);
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MIN_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_MAG_FILTER,
                gl::GL_LINEAR as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_S,
                gl::GL_CLAMP_TO_EDGE as i32,
            );
            gl::glTexParameteri(
                gl::GL_TEXTURE_2D,
                gl::GL_TEXTURE_WRAP_T,
                gl::GL_CLAMP_TO_EDGE as i32,
            );

            gl::glUseProgram(self.prog_ovl.prog);
            let m = mat3_scale_translate(1.0, 1.0, 0.0, 0.0);
            gl::glUniformMatrix3fv(self.prog_ovl.u_mvp, 1, 0, m.as_ptr());
            gl::glUniform2f(self.prog_ovl.u_res, self.win_w as f32, self.win_h as f32);
            gl::glUniform1i(self.prog_ovl.u_tex, 0);
            gl::glUniform1f(self.prog_ovl.u_alpha, alpha);

            let (w, h) = (self.win_w as f32, self.win_h as f32);
            let pos = [0.0f32, 0.0, w, 0.0, 0.0, h, w, h];
            // Invertimos Y en el UV porque el FBO de OpenGL guarda el frame invertido verticalmente respecto a ventana
            let uv = [0.0f32, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
            let mut verts: [f32; 24] = [0.0; 24];
            for i in 0..4 {
                verts[i * 6] = pos[i * 2];
                verts[i * 6 + 1] = pos[i * 2 + 1];
                verts[i * 6 + 2] = uv[i * 2];
                verts[i * 6 + 3] = uv[i * 2 + 1];
            }
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, self.vbo_quad);
            gl::glBufferData(
                gl::GL_ARRAY_BUFFER,
                std::mem::size_of_val(&verts) as isize,
                verts.as_ptr() as *const u8,
                gl::GL_STREAM_DRAW,
            );
            gl::glEnableVertexAttribArray(self.prog_ovl.a_pos as u32);
            gl::glVertexAttribPointer(
                self.prog_ovl.a_pos as u32,
                2,
                gl::GL_FLOAT,
                0,
                24,
                std::ptr::null(),
            );
            gl::glEnableVertexAttribArray(self.prog_ovl.a_uv as u32);
            gl::glVertexAttribPointer(
                self.prog_ovl.a_uv as u32,
                2,
                gl::GL_FLOAT,
                0,
                24,
                8 as *const u8,
            );
            gl::glDrawArrays(gl::GL_TRIANGLE_STRIP, 0, 4);
            gl::glDisableVertexAttribArray(self.prog_ovl.a_pos as u32);
            gl::glDisableVertexAttribArray(self.prog_ovl.a_uv as u32);
            gl::glBindBuffer(gl::GL_ARRAY_BUFFER, 0);
        }
    }

    /// Renderiza la capa base persistente (Dry FBO).
    /// Se invoca ÚNICAMENTE cuando cambia la página, zoom, pan, anotaciones o UI.
    fn render_dry(&mut self, reader: &Reader) {
        if self.dry_fbo == 0 || self.dry_tex == 0 {
            return;
        }
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, self.dry_fbo);
            gl::glViewport(0, 0, self.win_w, self.win_h);

            let p = reader.theme.palette();
            let bg = if reader.doc.is_none() {
                crate::theme::ERROR_BG_RGBA
            } else {
                p.rgba_bg()
            };
            self.clear(bg);

            // 1. Dibujar página PDF
            let mut page_drawn = false;
            let mut scale = 1.0f32;
            let (mut page_dx, mut page_dy) = (0.0f32, 0.0f32);
            if let Some(bmp) = reader
                .cache
                .peek(reader.page)
                .or_else(|| reader.fallback_page.and_then(|pg| reader.cache.peek(pg)))
            {
                let blit_zoom = if reader.rendered_zoom.is_finite() && reader.rendered_zoom > 0.0 {
                    reader.zoom / reader.rendered_zoom
                } else {
                    1.0
                };
                let pw = bmp.width as f32 * blit_zoom;
                self.upload_page_if_needed(reader.page, reader.rendered_zoom, bmp);
                page_dx = ((reader.win_w as f32 - pw) / 2.0 + reader.pan_x).round();
                page_dy = reader.pan_y.round();

                gl::glUseProgram(self.prog_tex.prog);
                gl::glActiveTexture(gl::GL_TEXTURE0);
                gl::glBindTexture(gl::GL_TEXTURE_2D, self.page_tex);
                self.set_tex_filter();
                let m = mat3_scale_translate(blit_zoom, blit_zoom, page_dx, page_dy);
                gl::glUniformMatrix3fv(self.prog_tex.u_mvp, 1, 0, m.as_ptr());
                gl::glUniform2f(self.prog_tex.u_res, self.win_w as f32, self.win_h as f32);
                gl::glUniform1i(self.prog_tex.u_tex, 0);
                gl::glUniform1i(self.prog_tex.u_dark, if reader.dark { 1 } else { 0 });

                let (x1, y1) = (bmp.width as f32, bmp.height as f32);
                let pos = [0.0f32, 0.0, x1, 0.0, 0.0, y1, x1, y1];
                let uv = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
                let mut verts = [0.0f32; 24];
                for i in 0..4 {
                    verts[i * 6] = pos[i * 2];
                    verts[i * 6 + 1] = pos[i * 2 + 1];
                    verts[i * 6 + 2] = uv[i * 2];
                    verts[i * 6 + 3] = uv[i * 2 + 1];
                }
                gl::glBindBuffer(gl::GL_ARRAY_BUFFER, self.vbo_quad);
                gl::glBufferData(
                    gl::GL_ARRAY_BUFFER,
                    std::mem::size_of_val(&verts) as isize,
                    verts.as_ptr() as *const u8,
                    gl::GL_STREAM_DRAW,
                );
                gl::glEnableVertexAttribArray(self.prog_tex.a_pos as u32);
                gl::glVertexAttribPointer(
                    self.prog_tex.a_pos as u32,
                    2,
                    gl::GL_FLOAT,
                    0,
                    24,
                    std::ptr::null(),
                );
                gl::glEnableVertexAttribArray(self.prog_tex.a_uv as u32);
                gl::glVertexAttribPointer(
                    self.prog_tex.a_uv as u32,
                    2,
                    gl::GL_FLOAT,
                    0,
                    24,
                    8 as *const u8,
                );
                gl::glDrawArrays(gl::GL_TRIANGLE_STRIP, 0, 4);
                gl::glDisableVertexAttribArray(self.prog_tex.a_pos as u32);
                gl::glDisableVertexAttribArray(self.prog_tex.a_uv as u32);
                gl::glBindBuffer(gl::GL_ARRAY_BUFFER, 0);

                page_drawn = true;
            }

            // 2. Dibujar anotaciones consolidadas
            if page_drawn {
                if let Some((pw, ph)) = reader.page_size_pt(reader.page) {
                    scale = crate::view::initial_scale(pw, ph, reader.win_w, reader.win_h)
                        * reader.zoom;
                }
                let dx = page_dx;
                let dy = page_dy;
                let anns = reader.annotations.for_page(reader.page as usize);
                for a in &anns {
                    if let pdf_core::Annotation::Highlight(h) = &a.kind {
                        for r in &h.rects {
                            let r = r.normalized();
                            let (x0, y0) = (r.x * scale + dx, r.y * scale + dy);
                            let (x1, y1) = ((r.x + r.w) * scale + dx, (r.y + r.h) * scale + dy);
                            let rgba = [h.color.r, h.color.g, h.color.b, h.color.a];
                            self.draw_solid_quad(x0, y0, x1, y1, rgba);
                        }
                    }
                }
                for a in &anns {
                    if let pdf_core::Annotation::Stroke(s) = &a.kind {
                        self.pts_scratch.clear();
                        for &(x, y) in &s.points {
                            self.pts_scratch.push((x * scale + dx, y * scale + dy));
                        }
                        let hw = (s.width * scale / 2.0).max(0.5);
                        let pts = std::mem::take(&mut self.pts_scratch);
                        self.draw_polyline_gpu(
                            &pts,
                            hw,
                            [s.color.r, s.color.g, s.color.b, s.color.a],
                        );
                        self.pts_scratch = pts;
                    }
                }
            }

            // 3. Overlays de UI (badges, chrome, toolbar, toast)
            let mut ovl = OverlayList::new();
            OverlayList::collect_viewer(reader, &mut ovl);
            for (b, x, y) in ovl.items {
                self.draw_bitmap(b, x, y, 1.0);
            }

            if reader.sheet_progress > 0.0
                && let Some(s) = reader.sheet_bitmap.as_ref()
            {
                let slide = (crate::reader::sheet_h(reader.win_h) as f32
                    * (1.0 - reader.sheet_progress))
                    .round() as i32;
                self.draw_bitmap(s, 0, -slide, 1.0);
            }

            if let Some((started, snap)) = &reader.lib_fade {
                let t = started.elapsed().as_secs_f32();
                let alpha = (1.0 - t / crate::LIB_FADE_MS).clamp(0.0, 1.0);
                if alpha > 0.0 {
                    self.draw_bitmap(snap, 0, 0, alpha);
                }
            }

            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
        }
    }

    /// Renderiza la capa de tinta en vuelo (Wet FBO transparente) con glScissor acotado.
    fn render_wet(&mut self, reader: &Reader) {
        if self.wet_fbo == 0 || self.wet_tex == 0 {
            return;
        }
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, self.wet_fbo);
            gl::glViewport(0, 0, self.win_w, self.win_h);

            // Limpieza a transparente puro
            gl::glClearColor(0.0, 0.0, 0.0, 0.0);
            gl::glClear(gl::GL_COLOR_BUFFER_BIT);

            if let Some(g) = reader.tool_gesture.as_ref() {
                let mut scale = 1.0f32;
                if let Some((pw, ph)) = reader.page_size_pt(reader.page) {
                    scale = crate::view::initial_scale(pw, ph, reader.win_w, reader.win_h)
                        * reader.zoom;
                }
                let blit_zoom = if reader.rendered_zoom.is_finite() && reader.rendered_zoom > 0.0 {
                    reader.zoom / reader.rendered_zoom
                } else {
                    1.0
                };
                let pw = reader
                    .cache
                    .peek(reader.page)
                    .map(|b| b.width as f32 * blit_zoom)
                    .unwrap_or(0.0);
                let dx = ((reader.win_w as f32 - pw) / 2.0 + reader.pan_x).round();
                let dy = reader.pan_y.round();

                match g.tool {
                    crate::annotations::ToolKind::Ink => {
                        self.pts_scratch.clear();
                        for &(x, y) in &g.ink_pts {
                            self.pts_scratch.push((x * scale + dx, y * scale + dy));
                        }
                        let w =
                            crate::prediction::pressure_width(reader.ink_width, g.last_pressure());
                        let hw = (w * scale / 2.0).max(0.5);
                        let pts = std::mem::take(&mut self.pts_scratch);
                        self.draw_polyline_gpu(
                            &pts,
                            hw,
                            [
                                reader.ink_color.r,
                                reader.ink_color.g,
                                reader.ink_color.b,
                                reader.ink_color.a,
                            ],
                        );
                        self.pts_scratch = pts;

                        // Remate M_last -> posición actual
                        if let (Some(m_last), Some(&last)) = (g.prev_mid, g.points.last())
                            && m_last != last
                        {
                            let a = (m_last.0 * scale + dx, m_last.1 * scale + dy);
                            let b = (last.0 * scale + dx, last.1 * scale + dy);
                            self.draw_polyline_gpu(
                                &[a, b],
                                hw,
                                [
                                    reader.ink_color.r,
                                    reader.ink_color.g,
                                    reader.ink_color.b,
                                    reader.ink_color.a,
                                ],
                            );
                        }

                        // Proyección Kalman
                        if let Some((px, py)) = g.predicted_pt {
                            let start_pt = g
                                .prev_mid
                                .or_else(|| g.points.last().copied())
                                .unwrap_or(g.anchor);
                            if start_pt != (px, py) {
                                let a = (start_pt.0 * scale + dx, start_pt.1 * scale + dy);
                                let b = (px * scale + dx, py * scale + dy);
                                self.draw_polyline_gpu(
                                    &[a, b],
                                    hw,
                                    [
                                        reader.ink_color.r,
                                        reader.ink_color.g,
                                        reader.ink_color.b,
                                        reader.ink_color.a,
                                    ],
                                );
                            }
                        }
                    }
                    crate::annotations::ToolKind::Highlight => {
                        let cur = g.points.last().copied().unwrap_or(g.anchor);
                        let (x0, y0) = (g.anchor.0 * scale + dx, g.anchor.1 * scale + dy);
                        let (x1, y1) = (cur.0 * scale + dx, cur.1 * scale + dy);
                        let c = pdf_core::HIGHLIGHT_COLOR;
                        self.draw_solid_quad(
                            x0.min(x1),
                            y0.min(y1),
                            x0.max(x1),
                            y0.max(y1),
                            [c.r, c.g, c.b, c.a],
                        );
                    }
                    _ => {}
                }
            }

            // Cursor de goma si aplica
            if let Some((ex, ey)) = reader.erase_pt {
                let r = reader.erase_r_px;
                let rgba = [0x88u8, 0x88, 0x88, 0x66];
                self.pts_scratch.clear();
                for i in 0..=36 {
                    let ang = i as f32 * std::f32::consts::TAU / 36.0;
                    self.pts_scratch
                        .push((ex + ang.cos() * r, ey + ang.sin() * r));
                }
                let pts = std::mem::take(&mut self.pts_scratch);
                self.draw_polyline_gpu(&pts, 1.5, rgba);
                self.pts_scratch = pts;
            }

            // Rect de selección (fill + borde) en px de ventana
            if let Some((l, t, r, b)) = reader.sel_screen_rect() {
                self.draw_solid_quad(l, t, r, b, crate::theme::SEL_FILL_RGBA);
                self.draw_sel_border(l, t, r, b);
            }

            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
        }
    }

    /// Present completo del visor por GPU con arquitectura Dual FBO (Wet/Dry).
    ///
    /// - Capa Dry: se re-renderiza SOLO al cambiar página, zoom, tema o al soltar el lápiz.
    ///   Durante la escritura activa, la base NUNCA se limpia ni se re-renderiza (CERO parpadeo).
    /// - Capa Wet: dibuja únicamente el avance del trazo activo + predicción Kalman en FBO transparente.
    /// - Composición: compone `dry_fbo ⊕ wet_fbo` en el framebuffer 0 (la ventana visible).
    pub(crate) fn present_viewer(&mut self, reader: &Reader) {
        let t0 = std::time::Instant::now();
        if !self.has_surface() {
            return;
        }

        // 1. Comprobar si la capa Dry (base persistente) está sucia
        let anns_count = reader.annotations.for_page(reader.page as usize).len();
        let key = DryKey {
            page: reader.page,
            zoom_bits: reader.zoom.to_bits(),
            pan_x: reader.pan_x.round() as i32,
            pan_y: reader.pan_y.round() as i32,
            ann_count: anns_count,
            dark: reader.dark,
            chrome_visible: reader.chrome_visible,
            toolbar_open: reader.toolbar_open,
            sheet_progress_bits: (reader.sheet_progress * 100.0).round() as u32,
            has_toast: reader.toast.is_some(),
        };

        if self.dry_dirty || self.dry_key != Some(key) {
            self.render_dry(reader);
            self.dry_dirty = false;
            self.dry_key = Some(key);
        }

        // 2. Renderizar capa Wet si hay trazo o goma activa
        let has_wet = reader.tool_gesture.is_some() || reader.erase_pt.is_some();
        if has_wet {
            self.render_wet(reader);
        }

        // 3. Composición final en framebuffer 0 (la ventana visible)
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
            gl::glViewport(0, 0, self.win_w, self.win_h);

            // Dibujar capa Dry base
            self.draw_fullscreen_texture(self.dry_tex, 1.0);

            // Componer encima la capa Wet transparente con premultiplied alpha
            if has_wet {
                self.draw_fullscreen_texture(self.wet_tex, 1.0);
            }
        }

        // 4. Conmutación dinámica de swap interval:
        // Durante trazo activo (Wet Ink), eglSwapInterval(0) despacha inmediatamente
        // al display sin bloqueo de VSYNC (< 4 ms). En commit/reposo, eglSwapInterval(1) a 120 Hz.
        let target_swap_interval = if has_wet { 0 } else { 1 };
        self.set_swap_interval(target_swap_interval);

        let Some(surf) = self.surf else { return };
        let swap_t0 = std::time::Instant::now();
        let ok = unsafe { gl::eglSwapBuffers(self.dpy, surf) != 0 };
        let swap_ms = swap_t0.elapsed().as_secs_f64() * 1000.0;
        info!(
            "gl_present {}x{}: {:.2} ms (swap {:.2} ms, {})",
            reader.win_w,
            reader.win_h,
            t0.elapsed().as_secs_f64() * 1000.0,
            swap_ms,
            if ok { "ok" } else { "FAIL" }
        );
    }

    fn draw_sel_border(&mut self, l: f32, t: f32, r: f32, b: f32) {
        let w = 2.0f32;
        let c = crate::theme::SEL_BORDER_RGBA;
        self.draw_solid_quad(l, t, r, t + w, c);
        self.draw_solid_quad(l, b - w, r, b, c);
        self.draw_solid_quad(l, t, l + w, b, c);
        self.draw_solid_quad(r - w, t, r, b, c);
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.drop_surface_only();
            gl::eglDestroyContext(self.dpy, self.ctx);
            gl::eglTerminate(self.dpy);
        }
    }
}

/// Copia de la lista de overlays del visor (mismos bitmaps y posiciones que
/// la rama Viewer de `Reader::blit`) — los (bitmap, x, y) que la GPU sube
/// como quads texturizados.
struct OverlayList<'a> {
    items: Vec<(&'a Bitmap, i32, i32)>,
}

impl<'a> OverlayList<'a> {
    fn new() -> Self {
        Self {
            items: Vec::with_capacity(8),
        }
    }

    /// Réplica EXACTA del orden/posiciones de `Reader::blit` (rama Viewer).
    fn collect_viewer(reader: &'a Reader, out: &mut Self) {
        if let Some(tb) = reader.toast_bitmap.as_ref() {
            let (_, by, _, _) = crate::reader::page_badge_rect(reader.win_w, reader.win_h);
            let tx = (reader.win_w - tb.width as i32) / 2;
            let ty = by - tb.height as i32 - 8;
            out.items.push((tb, tx, ty));
        }
        if reader.chrome_visible {
            if let Some(top) = reader.chrome_top_bitmap.as_ref() {
                out.items.push((top, 0, 0));
            }
            if let Some(bot) = reader.chrome_bottom_bitmap.as_ref() {
                out.items.push((bot, 0, reader.win_h - bot.height as i32));
            }
        } else if let Some(b) = reader.page_badge.as_ref() {
            let (bx, by, _, _) = crate::reader::page_badge_rect(reader.win_w, reader.win_h);
            out.items.push((b, bx, by));
        }
        if !reader.chrome_visible
            && let Some(mb) = reader.mode_badge.as_ref()
        {
            let (bx, by, _, _) = crate::draw::mode_badge_rect(reader.win_w, reader.win_h);
            out.items.push((mb, bx, by));
        }
        if let Some(tb) = reader.toolbar_bitmap.as_ref() {
            let (tx, ty, _, _) = crate::draw::toolbar_rect(reader.win_w, reader.win_h);
            out.items.push((tb, tx as i32, ty as i32));
        }
        if let Some(menu) = reader.sel_menu.as_ref() {
            out.items.push((&menu.bitmap, menu.x, menu.y));
        }
        if let Some(panel) = reader.ai_panel.as_ref() {
            out.items.push((&panel.bitmap, panel.x, panel.y));
        }
        if let Some(eb) = reader.eraser_cursor.as_ref()
            && let Some((ex, ey)) = reader.erase_pt
        {
            out.items.push((
                eb,
                ex as i32 - (eb.width as i32) / 2,
                ey as i32 - (eb.height as i32) / 2,
            ));
        }
    }
}
