// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Subida y caché de texturas (extraído de `gpu/mod.rs`, Tarea 4.1):
//! textura de página (`upload_page_if_needed`), caché LRU de overlays con
//! presupuesto por bytes (`OverlayTex`, `overlay_tex`, `delete_ovl_tex` —
//! wiring de `ovl_budget`), textura dedicada del fade de apertura
//! (`fade_tex`, `free_fade_tex`) y borrado contabilizado (`delete_texture`).
use log::info;

use pdf_core::Bitmap;

use super::ffi as gl;
use super::surface::Gpu;

/// Entrada de la caché de texturas de overlay: `id` = generación del bitmap
/// (id monótono asignado por el Reader en cada re-render, ver
/// `Reader::ovl_seq`) y `tex` = nombre GL. La política LRU por bytes vive en
/// `OvlBudget`, que usa el MISMO `id` como clave de unión y devuelve los ids
/// evictados para que `overlay_tex` borre sus texturas. El bitmap NO se
/// clona aquí: quien posee el id posee el bitmap (ABA del puntero resuelto).
pub(crate) struct OverlayTex {
    id: u64,
    tex: u32,
}

impl Gpu {
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
    pub(crate) fn set_tex_filter(&self) {
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
    /// Sube `b` como textura RGBA8 y devuelve el nombre GL (sin cachear).
    /// Lo comparten `overlay_tex` (caché LRU con presupuesto) y `fade_tex`
    /// (textura dedicada del fade, que puede exceder el presupuesto). `kind`
    /// etiqueta el contador de ciclo de vida en logcat ("ovl" | "fade").
    fn upload_texture(&mut self, b: &Bitmap, kind: &str) -> u32 {
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
        self.tex_created += 1;
        info!(
            "gpu: tex create {tex} {}x{} kind={kind} (created={})",
            b.width, b.height, self.tex_created
        );
        tex
    }
    /// Textura (cacheada) del bitmap de overlay `b`. El `id` lo asigna el
    /// Reader en CADA re-render del overlay (`Reader::ovl_seq` → campo
    /// `<overlay>_id`): la clave NUNCA es el puntero de `Bitmap::data` —
    /// cuando el allocator reusa la dirección de un bitmap ya liberado
    /// (ABA), el hit por puntero devolvía la textura del contenido ANTERIOR.
    /// El bitmap NO se clona aquí: quien posee el id posee el bitmap. El
    /// presupuesto LRU por bytes (`ovl_budget`, OVL_BYTE_BUDGET) evicta del
    /// frente y devuelve los ids evictados → `delete_ovl_tex` (glDeleteTextures).
    pub(crate) fn overlay_tex(&mut self, id: u64, b: &Bitmap) -> u32 {
        if let Some(o) = self.ovl_cache.iter().find(|o| o.id == id) {
            self.ovl_budget.touch(id); // hit: pasa a MRU del presupuesto
            return o.tex;
        }
        let bytes = (b.width as usize) * (b.height as usize) * 4;
        let evicted = self.ovl_budget.insert(id, bytes);
        if !self.ovl_budget.contains(id) {
            // No cabe ni vaciando la caché (bytes > presupuesto): se borran
            // las texturas evictadas y no se sube nada (draw_bitmap dibuja 0).
            for vid in evicted {
                self.delete_ovl_tex(vid);
            }
            return 0;
        }
        for vid in evicted {
            self.delete_ovl_tex(vid);
        }
        let tex = self.upload_texture(b, "ovl");
        self.ovl_cache.push(OverlayTex { id, tex });
        tex
    }
    /// Borra la textura del overlay `vid` (evicción LRU del presupuesto).
    fn delete_ovl_tex(&mut self, vid: u64) {
        if let Some(pos) = self.ovl_cache.iter().position(|o| o.id == vid) {
            let old = self.ovl_cache.swap_remove(pos);
            self.delete_texture(old.tex, "ovl");
        }
    }
    /// Textura dedicada del fade de apertura (`Reader::lib_fade`): el
    /// snapshot es de ventana COMPLETA y puede exceder `OVL_BYTE_BUDGET`, así
    /// que no pasa por `ovl_budget` (misma categoría que `page_tex`). Se sube
    /// una vez por transición: id nuevo por snapshot (`lib_fade_id`), reuso
    /// mientras el fade siga vivo y liberación al terminar o al reemplazarlo.
    pub(crate) fn fade_tex(&mut self, id: u64, b: &Bitmap) -> u32 {
        if let Some((old, tex)) = self.fade_tex
            && old == id
        {
            return tex;
        }
        let new = self.upload_texture(b, "fade");
        if let Some((_, tex)) = self.fade_tex.replace((id, new)) {
            self.delete_texture(tex, "fade");
        }
        new
    }
    /// Libera la textura del fade (transición terminada, sin fade activo o
    /// surface soltada — el fade se re-subirá con id nuevo si hace falta).
    pub(crate) fn free_fade_tex(&mut self) {
        if let Some((_, tex)) = self.fade_tex.take() {
            self.delete_texture(tex, "fade");
        }
    }
    /// Borra una textura standalone (overlay LRU o fade) con contador y log.
    fn delete_texture(&mut self, tex: u32, kind: &str) {
        unsafe {
            gl::glDeleteTextures(1, &tex);
        }
        self.tex_destroyed += 1;
        info!(
            "gpu: tex destroy {tex} kind={kind} (destroyed={})",
            self.tex_destroyed
        );
    }
}
