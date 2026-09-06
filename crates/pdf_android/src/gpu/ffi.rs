// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Declaraciones FFI de las APIs públicas de EGL + GLES2 (Khronos;
//! licencia de este fichero, no de los headers). Los crates de bindings
//! (`khronos-egl`) exigen pkg-config o `dlopen("libEGL.so.1")` — nombre que
//! no existe en Android (`libEGL.so`) — así que el spike validó este FFI
//! directo en la TCL.
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
pub const GL_TRIANGLES: u32 = 0x0004;
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
    pub fn eglGetError() -> u32;
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
