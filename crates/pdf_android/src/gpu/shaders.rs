// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Código GLSL de los shaders (página/dark, overlay con alpha y tinta con AA
//! analítico), structs de programas compilados (`QuadProg`, `InkProg`) y
//! helpers de compilación/link (`compile`, `link`, `gl_str`, `mat3_scale_translate`).
//! Extraído de `gpu/mod.rs` (Fase 2, Tarea 4.1).
use super::ffi as gl;

// ---------------------------------------------------------------- Shaders

pub(crate) const VS_QUAD_SRC: &[u8] = b"
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

pub(crate) const FS_TEX_SRC: &[u8] = b"
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

pub(crate) const FS_OVERLAY_SRC: &[u8] = b"
precision mediump float;
uniform sampler2D uTex;
uniform float uAlpha;
varying vec2 vUV;
void main() {
    vec4 c = texture2D(uTex, vUV);
    gl_FragColor = vec4(c.rgb * uAlpha, c.a * uAlpha);
}
\0";

pub(crate) const VS_INK_SRC: &[u8] = b"
attribute vec2 aPos;
attribute float aD;
attribute float aHw;
attribute vec2 aCenter;
attribute vec4 aColor;
uniform mat3 uMvp;
uniform vec2 uRes;
varying vec2 vPos;
varying float vD;
varying float vHw;
varying vec2 vCenter;
varying vec4 vColor;
void main() {
    vec2 px = (uMvp * vec3(aPos, 1.0)).xy;
    vec2 ndc = vec2(2.0 * px.x / uRes.x - 1.0, 1.0 - 2.0 * px.y / uRes.y);
    gl_Position = vec4(ndc, 0.0, 1.0);
    vPos = px;
    vD = aD;
    vHw = aHw;
    vCenter = aCenter;
    vColor = aColor;
}
\0";

pub(crate) const FS_INK_SRC: &[u8] = b"
precision mediump float;
uniform float uRound;
varying vec2 vPos;
varying float vD;
varying float vHw;
varying vec2 vCenter;
varying vec4 vColor;
void main() {
    float aa = 1.0;
    float alpha;
    if (vHw <= 0.0) {
        alpha = 1.0;
    } else if (uRound > 0.5) {
        // Disco redondo: distancia desde el fragmento al centro del punto.
        alpha = 1.0 - smoothstep(vHw - aa, vHw, distance(vPos, vCenter));
    } else {
        // Cinta: distancia perpendicular al centro de la linea (AA por ancho).
        alpha = 1.0 - smoothstep(vHw - aa, vHw, abs(vD));
    }
    gl_FragColor = vec4(vColor.rgb * vColor.a * alpha, vColor.a * alpha);
}
\0";

pub(crate) struct QuadProg {
    pub(crate) prog: u32,
    pub(crate) a_pos: i32,
    pub(crate) a_uv: i32,
    pub(crate) u_mvp: i32,
    pub(crate) u_res: i32,
    pub(crate) u_tex: i32,
    /// Solo el programa de página: `uDark` (inversión de RGB en el shader);
    /// -1 en el de overlays.
    pub(crate) u_dark: i32,
    pub(crate) u_alpha: i32,
}

pub(crate) struct InkProg {
    pub(crate) prog: u32,
    pub(crate) a_pos: i32,
    pub(crate) a_d: i32,
    pub(crate) a_hw: i32,
    pub(crate) a_center: i32,
    pub(crate) a_color: i32,
    pub(crate) u_mvp: i32,
    pub(crate) u_res: i32,
    pub(crate) u_round: i32,
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

pub(crate) fn link(vs: &[u8], fs: &[u8]) -> Option<u32> {
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
pub(crate) fn mat3_scale_translate(sx: f32, sy: f32, tx: f32, ty: f32) -> [f32; 9] {
    [
        sx, 0.0, 0.0, //
        0.0, sy, 0.0, //
        tx, ty, 1.0,
    ]
}

pub(crate) fn gl_str(ptr: *const u8) -> String {
    unsafe {
        let mut n = 0usize;
        while *ptr.add(n) != 0 {
            n += 1;
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(ptr, n)).into_owned()
    }
}
