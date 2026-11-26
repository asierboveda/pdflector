// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Capa Dry del modo Scroll (PR-01): la columna de páginas. Dibuja las hojas
//! VISIBLES de la caché, cada una en el rect que le da su transformación
//! (`Reader::page_screen_transform` en Scroll) y con su textura propia
//! (`PageSlots`: sin re-subidas por frame de desplazamiento), y encima sus
//! anotaciones consolidadas. Cada hoja usa la escala de SU bitmap
//! (`bitmap_quad`), así una hoja aún renderizada a otro zoom se estira sin
//! saltar hasta que llega su render nítido.
//!
//! El modo Tap sigue en `pipeline::render_dry` sin cambios.

use super::ffi as gl;
use super::shaders::mat3_scale_translate;
use super::surface::Gpu;
use crate::reader::Reader;
use pdf_core::Bitmap;

impl Gpu {
    /// Hornea la columna visible en la dry FBO. Devuelve true si dibujó al
    /// menos una hoja (mismo contrato que `render_dry`).
    pub(super) fn render_dry_scroll(&mut self, reader: &Reader) -> bool {
        if self.dry_fbo == 0 || self.dry_tex == 0 {
            return false;
        }
        let mut drew = false;
        unsafe {
            gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, self.dry_fbo);
            gl::glViewport(0, 0, self.win_w, self.win_h);
            let bg = self.view_bg(reader);
            self.clear(bg);
        }
        let hidden = reader.recorte_hidden_ids();
        for page in reader.scroll_visible_pages() {
            let Some(entry) = reader.cache.peek(page) else {
                continue; // aún sin render: queda el fondo (llega del worker)
            };
            let Some(transform) = reader.page_screen_transform(page) else {
                continue;
            };
            let (s, x, y) = transform.bitmap_quad(entry.full_w, entry.crop_x, entry.crop_y);
            let tex = self.page_slot_tex(page, &entry.bitmap);
            self.draw_page_quad(tex, &entry.bitmap, s, x.round(), y.round(), reader.dark);

            // Anotaciones consolidadas de la hoja (mismo dibujo que la dry
            // del modo Tap: resaltados debajo, trazos encima).
            let anns = reader.annotations.for_page(page as usize);
            for a in &anns {
                if let pdf_core::Annotation::Highlight(h) = &a.kind {
                    for r in &h.rects {
                        let r = r.normalized();
                        let (x0, y0) = transform.page_to_screen(r.x, r.y);
                        let (x1, y1) = transform.page_to_screen(r.x + r.w, r.y + r.h);
                        let rgba = [h.color.r, h.color.g, h.color.b, h.color.a];
                        self.draw_solid_quad(x0, y0, x1, y1, rgba);
                    }
                }
            }
            for a in &anns {
                if hidden.contains(&a.id) {
                    continue;
                }
                if let pdf_core::Annotation::Stroke(st) = &a.kind {
                    self.pts_scratch.clear();
                    for &(px, py) in &st.points {
                        self.pts_scratch.push(transform.page_to_screen(px, py));
                    }
                    let hw = (st.width * transform.scale() / 2.0).max(0.5);
                    let pts = std::mem::take(&mut self.pts_scratch);
                    self.draw_polyline_gpu(
                        &pts,
                        hw,
                        [st.color.r, st.color.g, st.color.b, st.color.a],
                    );
                    self.pts_scratch = pts;
                }
            }
            drew = true;
        }
        if !drew {
            unsafe { gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, 0) };
        }
        drew
    }

    /// Quad texturizado de una hoja: textura `tex` del bitmap `b` escalada
    /// `s` con la esquina en (x, y); inversión de color en modo oscuro.
    fn draw_page_quad(&mut self, tex: u32, b: &Bitmap, s: f32, x: f32, y: f32, dark: bool) {
        unsafe {
            gl::glUseProgram(self.prog_tex.prog);
            gl::glActiveTexture(gl::GL_TEXTURE0);
            gl::glBindTexture(gl::GL_TEXTURE_2D, tex);
            self.set_tex_filter();
            let m = mat3_scale_translate(s, s, x, y);
            gl::glUniformMatrix3fv(self.prog_tex.u_mvp, 1, 0, m.as_ptr());
            gl::glUniform2f(self.prog_tex.u_res, self.win_w as f32, self.win_h as f32);
            gl::glUniform1i(self.prog_tex.u_tex, 0);
            gl::glUniform1i(self.prog_tex.u_dark, if dark { 1 } else { 0 });

            let (x1, y1) = (b.width as f32, b.height as f32);
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
        }
    }
}
