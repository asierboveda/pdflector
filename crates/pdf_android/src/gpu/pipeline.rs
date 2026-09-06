// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Render y present del visor con arquitectura Dual FBO Wet/Dry (extraído
//! de `gpu/mod.rs`, Tarea 4.1): `render_dry` (página+anotaciones), `render_wet`
//! (trazo en vuelo, goma, selección), `present_viewer` (composición en fb0 +
//! overlays UI por frame + swap), primitivas de dibujo (quads sólidos,
//! polylines de tinta, quads texturizados a pantalla completa), el formato de
//! vértice `InkVert` y la copia de overlays `OverlayList`.
use log::info;

use crate::reader::Reader;
use pdf_core::Bitmap;

use super::DryKey;
use super::ffi as gl;
use super::shaders::mat3_scale_translate;
use super::surface::Gpu;

/// Vertex de tinta: posición en PANTALLA (px, y abajo) + offset perpendicular
/// con signo (AA por ancho) + semi-ancho del centro + centro del punto
/// (disco redondo de tapas/juntas) + RGBA8.
#[repr(C)]
pub(crate) struct InkVert {
    x: f32,
    y: f32,
    /// Offset perpendicular CON SIGNO (±hw) al centro de línea: `abs(d)` =
    /// distancia al centro en el fragment shader (AA por ancho).
    d: f32,
    /// Semi-ancho del trazo en px de pantalla (centro de la línea).
    hw: f32,
    /// Centro del punto (disco redondo de tapas/juntas; 0 en la cinta).
    cx: f32,
    cy: f32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

const INK_VERT_SIZE: usize = std::mem::size_of::<InkVert>(); // 28 bytes

impl Gpu {
    /// Dibuja un quad texturizado (prog_ovl, alpha uniforme) con una textura
    /// YA resuelta (`overlay_tex` o `fade_tex`). Coordenadas de pantalla; el
    /// pan NO se aplica (los overlays son UI fija).
    fn draw_tex_quad(&mut self, tex: u32, b: &Bitmap, x: i32, y: i32, alpha: f32) {
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
    /// Dibuja el bitmap de overlay `b` como quad texturizado, cacheándolo
    /// por su id de generación (ver `overlay_tex`).
    fn draw_bitmap(&mut self, b: &Bitmap, id: u64, x: i32, y: i32, alpha: f32) {
        let tex = self.overlay_tex(id, b);
        if tex == 0 {
            // Entrada mayor que el presupuesto: no se cachea ni se dibuja
            // (bindear la textura 0 pintaría un quad indefinido).
            return;
        }
        self.draw_tex_quad(tex, b, x, y, alpha);
    }
    fn draw_solid_quad(&mut self, l: f32, t: f32, r: f32, b: f32, rgba: [u8; 4]) {
        if !l.is_finite() || !t.is_finite() || !r.is_finite() || !b.is_finite() {
            return;
        }
        self.draw_ink_triangles(
            &[
                InkVert {
                    x: l,
                    y: t,
                    d: 0.0,
                    hw: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                },
                InkVert {
                    x: r,
                    y: t,
                    d: 0.0,
                    hw: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                },
                InkVert {
                    x: l,
                    y: b,
                    d: 0.0,
                    hw: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                },
                InkVert {
                    x: r,
                    y: b,
                    d: 0.0,
                    hw: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                },
            ],
            false,
            gl::GL_TRIANGLE_STRIP,
        );
    }
    fn draw_ink_triangles(&mut self, verts: &[InkVert], round: bool, mode: u32) {
        if verts.is_empty() {
            return;
        }
        unsafe {
            gl::glUseProgram(self.prog_ink.prog);
            let id = mat3_scale_translate(1.0, 1.0, 0.0, 0.0);
            gl::glUniformMatrix3fv(self.prog_ink.u_mvp, 1, 0, id.as_ptr());
            gl::glUniform2f(self.prog_ink.u_res, self.win_w as f32, self.win_h as f32);
            gl::glUniform1f(self.prog_ink.u_round, if round { 1.0 } else { 0.0 });
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
            gl::glEnableVertexAttribArray(self.prog_ink.a_d as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_d as u32,
                1,
                gl::GL_FLOAT,
                0,
                stride,
                8 as *const u8,
            );
            gl::glEnableVertexAttribArray(self.prog_ink.a_hw as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_hw as u32,
                1,
                gl::GL_FLOAT,
                0,
                stride,
                12 as *const u8,
            );
            gl::glEnableVertexAttribArray(self.prog_ink.a_center as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_center as u32,
                2,
                gl::GL_FLOAT,
                0,
                stride,
                16 as *const u8,
            );
            gl::glEnableVertexAttribArray(self.prog_ink.a_color as u32);
            gl::glVertexAttribPointer(
                self.prog_ink.a_color as u32,
                4,
                gl::GL_UNSIGNED_BYTE,
                1,
                stride,
                24 as *const u8,
            );
            gl::glDrawArrays(mode, 0, verts.len() as i32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_pos as u32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_d as u32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_hw as u32);
            gl::glDisableVertexAttribArray(self.prog_ink.a_center as u32);
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
        self.pts_scratch.clear();
        for (i, &(x, y)) in pts.iter().enumerate() {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            // Punto válido: se usa para el disco redondo (tapa y junta).
            self.pts_scratch.push((x, y));
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
            // Cinta: offset perpendicular CON SIGNO (±hw) para el AA por ancho.
            self.ink_scratch.push(InkVert {
                x: x + nx * hw,
                y: y + ny * hw,
                d: hw,
                hw,
                cx: x,
                cy: y,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            });
            self.ink_scratch.push(InkVert {
                x: x - nx * hw,
                y: y - ny * hw,
                d: -hw,
                hw,
                cx: x,
                cy: y,
                r: rgba[0],
                g: rgba[1],
                b: rgba[2],
                a: rgba[3],
            });
        }

        let ribbon = std::mem::take(&mut self.ink_scratch);
        if !ribbon.is_empty() {
            self.draw_ink_triangles(&ribbon, false, gl::GL_TRIANGLE_STRIP);
        }
        self.ink_scratch = ribbon;

        // Tapas Y JUNTAS REDONDAS: disco (quad + distancia radial en el FS)
        // en cada punto. Al unirse a la cinta, el trazo queda con extremos y
        // esquinas redondeados (nada de "cuadrados").
        self.ink_scratch.clear();
        for &(x, y) in &self.pts_scratch {
            let corners = [
                (x - hw, y - hw),
                (x + hw, y - hw),
                (x + hw, y + hw),
                (x - hw, y - hw),
                (x + hw, y + hw),
                (x - hw, y + hw),
            ];
            for &(px, py) in &corners {
                self.ink_scratch.push(InkVert {
                    x: px,
                    y: py,
                    d: 0.0,
                    hw,
                    cx: x,
                    cy: y,
                    r: rgba[0],
                    g: rgba[1],
                    b: rgba[2],
                    a: rgba[3],
                });
            }
        }
        if !self.ink_scratch.is_empty() {
            let caps = std::mem::take(&mut self.ink_scratch);
            self.draw_ink_triangles(&caps, true, gl::GL_TRIANGLES);
            self.ink_scratch = caps;
        }
    }
    /// Invalida explícitamente la capa base (Dry FBO) para forzar su re-render.
    #[allow(dead_code)]
    pub(crate) fn invalidate_dry(&mut self) {
        self.dry_dirty = true;
    }
    /// Dibuja una textura a pantalla completa en el framebuffer actualmente
    /// vinculado. `offset` (px de ventana, f32) traslada el quad de vértices:
    /// la capa Dry (página) lo recibe del pan del visor; la Wet (trazo en
    /// vuelo) y los overlays van sin offset (UI fija en coords de pantalla).
    fn draw_fullscreen_texture(&mut self, tex: u32, alpha: f32, offset: (f32, f32)) {
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
            // El offset traslada el quad de página: misma proyección (mvp
            // identidad de pantalla), solo desplazamos las coordenadas.
            let (dx, dy) = offset;
            let pos = [dx, dy, w + dx, dy, dx, h + dy, w + dx, h + dy];
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
    /// Renderiza la capa base persistente (Dry FBO): página + anotaciones SOLO.
    /// Se invoca ÚNICAMENTE cuando cambia la página, el zoom, las anotaciones o
    /// el dark (campos reales de la `DryKey` reducida). Los overlays de UI
    /// (chrome, sheet, toast, sel_menu, ai_panel, lib_fade, badges, cursor de
    /// goma) NO viven aquí: se dibujan por frame en `present_viewer` directos
    /// a fb0 (Fase 2).
    fn render_dry(&mut self, reader: &Reader) {
        if self.dry_fbo == 0 || self.dry_tex == 0 {
            return;
        }
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, self.dry_fbo);
            gl::glViewport(0, 0, self.win_w, self.win_h);

            let bg = self.view_bg(reader);
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
                // El pan NO se hornea aquí: lo aplica el quad de composición
                // en present_viewer (Tarea 2.3). Horneado + quad desplazado
                // duplicarían el pan (2×pan) en cada re-render de la dry.
                page_dx = ((reader.win_w as f32 - pw) / 2.0).round();
                page_dy = 0.0;

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

            // 3. (Sin overlays: la UI se dibuja por frame en present_viewer.)

            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
        }
    }
    /// Renderiza la capa transitoria (Wet FBO transparente): avance del
    /// trazo activo (tinta con remate + predicción Kalman, resaltador
    /// alineado), cursor de la goma y rect de selección (fill + borde). El
    /// FBO se limpia por COMPLETO en cada llamada (sin glScissor) y la
    /// textura se compone sobre la dry con offset (0,0): el trazo ya hornea
    /// su pan (página→pantalla) y el cursor/rect usan px de ventana.
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
                        let c = pdf_core::HIGHLIGHT_COLOR;
                        if g.hl_spans.is_empty() {
                            // Sin spans cacheados: bbox crudo ancla→cursor.
                            let cur = g.points.last().copied().unwrap_or(g.anchor);
                            let (x0, y0) = (g.anchor.0 * scale + dx, g.anchor.1 * scale + dy);
                            let (x1, y1) = (cur.0 * scale + dx, cur.1 * scale + dy);
                            self.draw_solid_quad(
                                x0.min(x1),
                                y0.min(y1),
                                x0.max(x1),
                                y0.max(y1),
                                [c.r, c.g, c.b, c.a],
                            );
                        } else {
                            // B3: rects tentativos alineados al texto (~10µs,
                            // sin I/O: spans pre-ordenados en el Down). Sin
                            // save hasta el Up.
                            let gesture = pdf_core::Gesture::Points(g.points.clone());
                            if let Some(hl) = pdf_core::highlight_under_gesture_sorted(
                                &g.hl_spans,
                                &gesture,
                                pdf_core::HIGHLIGHT_COLOR,
                            ) {
                                for r in &hl.rects {
                                    self.draw_solid_quad(
                                        r.x * scale + dx,
                                        r.y * scale + dy,
                                        (r.x + r.w) * scale + dx,
                                        (r.y + r.h) * scale + dy,
                                        [c.r, c.g, c.b, c.a],
                                    );
                                }
                            }
                        }
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
    /// - Capa Dry: se re-renderiza SOLO al cambiar página, zoom, anotaciones o
    ///   dark (los 4 campos de la `DryKey`). Durante la escritura activa, la
    ///   base NUNCA se limpia ni se re-renderiza (CERO parpadeo).
    /// - Capa Wet: capa transitoria en FBO transparente — avance del trazo
    ///   activo + predicción Kalman, cursor de la goma y rect de selección —
    ///   re-renderizada por frame SOLO mientras `has_wet` (trazo, goma o
    ///   selección activos).
    /// - Composición: compone `dry_fbo ⊕ wet_fbo` en el framebuffer 0 (la
    ///   ventana visible) y encima los overlays de UI por frame (no invalidan
    ///   la dry).
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
            ann_count: anns_count,
            dark: reader.dark,
        };

        if self.dry_dirty || self.dry_key.is_none_or(|old| key.invalidates(&old)) {
            self.render_dry(reader);
            self.dry_dirty = false;
            self.dry_key = Some(key);
        }

        // 2. Renderizar capa Wet si hay capa transitoria que pintar: trazo de
        // herramienta, goma activa o selección (rect vivo/fijado). La
        // selección NO invalida la dry (fuera de la DryKey desde 2.0): vive
        // en la wet como el trazo — sin esto, el gesto Selecting (long-press
        // de dedo, sin tool_gesture ni erase_pt) nunca llegaría a render_wet
        // y el rect sería invisible en GPU (Tarea 2.5).
        let has_wet =
            reader.tool_gesture.is_some() || reader.erase_pt.is_some() || reader.sel.is_some();
        if has_wet {
            self.render_wet(reader);
        }

        // 3. Composición final en framebuffer 0 (la ventana visible)
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0);
            gl::glViewport(0, 0, self.win_w, self.win_h);

            // Limpiar fb0 con el fondo del visor: con pan ≠ 0 el quad de la
            // dry no cubre toda la pantalla y esas bandas deben ser del color
            // de fondo (no contenido indefinido tras el swap).
            self.clear(self.view_bg(reader));

            // Dibujar capa Dry base, trasladada por el pan del visor: la
            // página entera se desplaza con el dedo (offset en el quad de
            // vértices, misma proyección).
            self.draw_fullscreen_texture(self.dry_tex, 1.0, (reader.pan_x, reader.pan_y));

            // Componer encima la capa Wet transparente con premultiplied alpha.
            // La Wet se compone SIEMPRE con offset (0,0): el trazo en vuelo ya
            // hornea su pan al transformar página→pantalla y el cursor de goma
            // / rect de selección están en px de ventana.
            if has_wet {
                self.draw_fullscreen_texture(self.wet_tex, 1.0, (0.0, 0.0));
            }
        }

        // 3b. Overlays de UI directamente a fb0 (por frame): chrome, sheet,
        // toast, sel_menu, ai_panel, lib_fade, badges, cursor de goma. Nunca
        // invalidan la dry (Fase 2): la dry cachea página + anotaciones, esto
        // es UI viva.
        // Cada (bitmap, id) lleva el id de generación que el Reader asignó en
        // su último re-render (`Reader::ovl_seq`): el hit de la caché de
        // texturas es por id, nunca por puntero (ABA, Tarea 2.4).
        let mut ovl = OverlayList::new();
        OverlayList::collect_viewer(reader, &mut ovl);
        for (b, id, x, y) in ovl.items {
            // El pan NO se aplica a los overlays: son UI fija en coords de pantalla.
            self.draw_bitmap(b, id, x, y, 1.0);
        }

        if reader.sheet_progress > 0.0
            && let Some(s) = reader.sheet_bitmap.as_ref()
        {
            let slide = (crate::reader::sheet_h(reader.win_h) as f32
                * (1.0 - reader.sheet_progress))
                .round() as i32;
            self.draw_bitmap(s, reader.sheet_id, 0, -slide, 1.0);
        }

        if let Some((started, snap)) = &reader.lib_fade {
            let t = started.elapsed().as_secs_f32();
            let alpha = (1.0 - t / crate::LIB_FADE_MS).clamp(0.0, 1.0);
            if alpha > 0.0 {
                let tex = self.fade_tex(reader.lib_fade_id, snap);
                self.draw_tex_quad(tex, snap, 0, 0, alpha);
            } else {
                self.free_fade_tex(); // fade expirado: liberar la textura grande
            }
        } else {
            self.free_fade_tex(); // sin fade activo: no retener el snapshot
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
        // Fase A1: p95 del intervalo entre presents (1 línea / 120 presents;
        // overhead ~ns: 1 Instant + push en anillo prealocado de 600).
        if ok {
            let now = std::time::Instant::now();
            if let Some(prev) = self.last_present.replace(now) {
                self.frame_timer.push(now - prev);
            }
            self.presents += 1;
            if self.presents.is_multiple_of(120)
                && let Some(p95) = self.frame_timer.p95()
            {
                info!(
                    "frame p95={:.1}ms ({} frames)",
                    p95.as_secs_f64() * 1000.0,
                    self.presents
                );
            }
        }
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

/// Copia de la lista de overlays del visor (mismos bitmaps y posiciones que
/// la rama Viewer de `Reader::blit`) — los (bitmap, id, x, y) que la GPU sube
/// como quads texturizados. El `id` es el de generación del bitmap
/// (`Reader::ovl_seq`, campo `<overlay>_id`): la caché de texturas se clavea
/// por él (ABA resuelto, Tarea 2.4).
struct OverlayList<'a> {
    items: Vec<(&'a Bitmap, u64, i32, i32)>,
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
            out.items.push((tb, reader.toast_id, tx, ty));
        }
        if reader.chrome_visible {
            if let Some(top) = reader.chrome_top_bitmap.as_ref() {
                out.items.push((top, reader.chrome_top_id, 0, 0));
            }
            if let Some(bot) = reader.chrome_bottom_bitmap.as_ref() {
                out.items.push((
                    bot,
                    reader.chrome_bottom_id,
                    0,
                    reader.win_h - bot.height as i32,
                ));
            }
        } else if let Some(b) = reader.page_badge.as_ref() {
            let (bx, by, _, _) = crate::reader::page_badge_rect(reader.win_w, reader.win_h);
            out.items.push((b, reader.page_badge_id, bx, by));
        }
        if !reader.chrome_visible
            && let Some(mb) = reader.mode_badge.as_ref()
        {
            let (bx, by, _, _) = crate::draw::mode_badge_rect(reader.win_w, reader.win_h);
            out.items.push((mb, reader.mode_badge_id, bx, by));
        }
        if let Some(menu) = reader.sel_menu.as_ref() {
            out.items
                .push((&menu.bitmap, reader.sel_menu_id, menu.x, menu.y));
        }
        if let Some(panel) = reader.ai_panel.as_ref() {
            out.items
                .push((&panel.bitmap, reader.ai_panel_id, panel.x, panel.y));
        }
        if let Some(eb) = reader.eraser_cursor.as_ref()
            && let Some((ex, ey)) = reader.erase_pt
        {
            out.items.push((
                eb,
                reader.eraser_cursor_id,
                ex as i32 - (eb.width as i32) / 2,
                ey as i32 - (eb.height as i32) / 2,
            ));
        }
    }
}
