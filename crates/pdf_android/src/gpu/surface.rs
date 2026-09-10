// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Ciclo de vida EGL/GLES2 del visor (Tarea 4.1): `struct Gpu` (contexto,
//! recursos, cachés y contadores), creación/descarte de surface, context y
//! programas (`new`, `create_display`, `with_display`, `make_resources`),
//! recreación por ventana (`recreate_surface`, `drop_surface*`), contadores
//! del ciclo de vida (Tarea 2.6), `clear`/`view_bg` y los helpers de FBO
//! (`create/destroy_fbo_with_tex`) y swap interval.
use android_activity::ndk::hardware_buffer_format::HardwareBufferFormat;
use android_activity::ndk::native_window::NativeWindow;
use log::{info, warn};

use crate::reader::Reader;
use pdf_core::FrameTimer;

use super::ffi as gl;
use super::pipeline::InkVert;
use super::shaders::{
    FS_INK_SRC, FS_OVERLAY_SRC, FS_TEX_SRC, InkProg, QuadProg, VS_INK_SRC, VS_QUAD_SRC, gl_str,
    link,
};
use super::textures::OverlayTex;
use super::{DryKey, OVL_BYTE_BUDGET, OvlBudget};

/// Contexto EGL + recursos GLES2 del visor, la biblioteca y el picker. La
/// surface se destruye y recrea SOLO con la ventana (resize/InitWindow/
/// TerminateWindow); desde la Tarea 2.7 vive TODA la vida de la ventana y ya
/// NO se suelta al entrar en Library/Picker (productor único: esos modos
/// presentan por el mismo EGL — el lock CPU quedó solo como fallback sin
/// EGL). El display y el contexto sobreviven entre surfaces. `Drop`
/// desconecta en orden inverso (patrón del spike, validado en TCL).
pub(crate) struct Gpu {
    pub(crate) dpy: gl::EGLDisplay,
    pub(crate) ctx: gl::EGLContext,
    pub(crate) cfg: gl::EGLConfig,
    pub(crate) surf: Option<gl::EGLSurface>,
    pub(crate) win_w: i32,
    pub(crate) win_h: i32,
    #[allow(dead_code)]
    pub(crate) front_buffer_active: bool,
    pub(crate) prog_tex: QuadProg,
    pub(crate) prog_ovl: QuadProg,
    pub(crate) prog_ink: InkProg,
    pub(crate) page_tex: u32,
    pub(crate) page_tex_w: i32,
    pub(crate) page_tex_h: i32,
    pub(crate) page_loaded: Option<(u32, f32, u32)>,
    pub(crate) vbo_quad: u32,
    pub(crate) vbo_ink: u32,
    /// Caché de texturas de overlay claveada por id de generación (ver
    /// `OverlayTex`); el presupuesto LRU por bytes vive en `ovl_budget`
    /// (misma fuente de verdad: un id residente en la caché está SIEMPRE en
    /// el budget y viceversa).
    pub(crate) ovl_cache: Vec<OverlayTex>,
    pub(crate) ovl_budget: OvlBudget,
    /// Textura dedicada del fade de apertura (`Reader::lib_fade`): snapshot
    /// de ventana COMPLETA que puede exceder `OVL_BYTE_BUDGET`, así que se
    /// sube una vez por transición y vive FUERA del presupuesto (misma
    /// categoría que `page_tex`). Some((id_del_snapshot, tex)); se libera al
    /// terminar el fade, al empezar otro o al soltar la surface
    /// (`drop_surface_only` — salir del visor a mitad de fade, Tarea 2.6).
    pub(crate) fade_tex: Option<(u64, u32)>,
    // --- Texturas dedicadas de los planos de la BIBLIOTECA y del PICKER
    // (Tarea 2.7: productor único EGL). Igual que `fade_tex`, son grandes
    // (cabecera o banda de contenido ~ hasta ventana completa; la banda con
    // margen de prefetch) y viven FUERA del presupuesto LRU de overlays
    // (`OVL_BYTE_BUDGET`). Cada una guarda la versión del bitmap que se subió
    // (bumps del Reader: rebuild estructural, re-band, splice, portadas
    // nuevas, re-render del picker): el present las re-sube SOLO cuando esa
    // versión cambia (una subida por frame de ~12 MB sería lenta). Se
    // liberan al volver al visor (`present_viewer` → `free_ui_planes`) y al
    // soltar la surface (`drop_surface_only`).
    pub(crate) lib_header_plane: Option<((u8, u64), u32)>,
    pub(crate) lib_band_plane: Option<((u8, u64), u32)>,
    pub(crate) picker_plane: Option<(u64, u32)>,

    // --- Contadores del ciclo de vida EGL/GLES (Tarea 2.6) ---
    // Acumuladores create/destroy por tipo de recurso, logueados en cada
    // `drop_surface_only` (resumen `gpu: lifecycle ...`). Tras N ciclos
    // Library→Viewer, surf y fbo deben leerse a delta 0 (create==destroy);
    // tex a delta == ovl_live (texturas vivas del LRU; la fade se libera en
    // el propio drop). No son AtomicU64 porque solo se tocan con `&mut self`
    // (un único hilo, el del bucle principal). page_tex, programas y VBOs se
    // crean UNA vez por contexto (make_resources) y NO cuentan: se liberan
    // con `eglDestroyContext` en el Drop.
    pub(crate) surf_created: u64,
    pub(crate) surf_destroyed: u64,
    pub(crate) surf_failed: u64,
    pub(crate) fbo_created: u64,
    pub(crate) fbo_destroyed: u64,
    pub(crate) tex_created: u64,
    pub(crate) tex_destroyed: u64,

    // --- Fase W1/W2/W3: Pipeline Dual FBO (Wet / Dry Ink) ---
    pub(crate) dry_fbo: u32,
    pub(crate) dry_tex: u32,
    pub(crate) dry_dirty: bool,
    pub(crate) dry_key: Option<DryKey>,

    pub(crate) wet_fbo: u32,
    pub(crate) wet_tex: u32,

    // Búfers de trabajo prealocados: cero alocaciones en el hot path (Fase W3)
    pub(crate) ink_scratch: Vec<InkVert>,
    pub(crate) pts_scratch: Vec<(f32, f32)>,
    pub(crate) current_swap_interval: i32,
    // --- Fase A1: instrumentación frame time (p95 a logcat, overhead ~ns) ---
    pub(crate) frame_timer: FrameTimer,
    pub(crate) presents: u64,
    pub(crate) last_present: Option<std::time::Instant>,
}

// Logs de transición + contadores acumulados (campos `surf_*/fbo_*/tex_*`).
// Cada `drop_surface_only` cierra con `log_lifecycle`. Desde la Tarea 2.7 la
// surface ya NO se suelta en las transiciones Library→Viewer (productor
// único EGL), así que los drop/create solo ocurren con ventanas NUEVAS:
// tras N ciclos Library→Viewer los contadores deben quedar INALTERADOS
// (delta 0 por ciclo, sin recrear nada). tex a delta == ovl_live en cada
// drop (el LRU de overlays es vivo por diseño y acotado por
// `OVL_BYTE_BUDGET`). Un delta que crezca con los ciclos = leak.

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
            let mut gpu = match Self::make_resources(dpy, cfg, ctx, surf) {
                Some(g) => g,
                None => {
                    // Sin `Gpu` no hay `Drop` que libere la surface ya creada
                    // (el `?` anterior la dejaba huérfana): destruirla aquí
                    // (Tarea 2.6, defensivo e idempotente).
                    warn!("make_resources failed: releasing EGL surface");
                    gl::eglDestroySurface(dpy, surf);
                    return None;
                }
            };
            gpu.win_w = win.width();
            gpu.win_h = win.height();
            gl::glViewport(0, 0, gpu.win_w, gpu.win_h);
            gpu.note_surface_created(gpu.win_w, gpu.win_h);

            let (dry_fbo, dry_tex) = Self::create_fbo_with_tex(gpu.win_w, gpu.win_h);
            let (wet_fbo, wet_tex) = Self::create_fbo_with_tex(gpu.win_w, gpu.win_h);
            gpu.dry_fbo = dry_fbo;
            gpu.dry_tex = dry_tex;
            gpu.dry_dirty = true;
            gpu.wet_fbo = wet_fbo;
            gpu.wet_tex = wet_tex;
            gpu.note_fbos_created();

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
                a_d: gl::glGetAttribLocation(p, c"aD".as_ptr()),
                a_hw: gl::glGetAttribLocation(p, c"aHw".as_ptr()),
                a_center: gl::glGetAttribLocation(p, c"aCenter".as_ptr()),
                a_color: gl::glGetAttribLocation(p, c"aColor".as_ptr()),
                u_mvp: gl::glGetUniformLocation(p, c"uMvp".as_ptr()),
                u_res: gl::glGetUniformLocation(p, c"uRes".as_ptr()),
                u_round: gl::glGetUniformLocation(p, c"uRound".as_ptr()),
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
                ovl_budget: OvlBudget::new(OVL_BYTE_BUDGET),
                fade_tex: None,
                lib_header_plane: None,
                lib_band_plane: None,
                picker_plane: None,
                dry_fbo: 0,
                dry_tex: 0,
                dry_dirty: true,
                dry_key: None,
                wet_fbo: 0,
                wet_tex: 0,
                ink_scratch: Vec::with_capacity(2048),
                pts_scratch: Vec::with_capacity(1024),
                current_swap_interval: 1,
                frame_timer: FrameTimer::new(),
                presents: 0,
                last_present: None,
                surf_created: 0,
                surf_destroyed: 0,
                surf_failed: 0,
                fbo_created: 0,
                fbo_destroyed: 0,
                tex_created: 0,
                tex_destroyed: 0,
            })
        }
    }
    /// (Re)crea la surface para una ventana NUEVA (resize / re-init; cada
    /// `ANativeWindow` nuevo invalida la surface EGL previa, ligada a la
    /// ventana anterior). Desde la Tarea 2.7 ya NO se llama en las
    /// transiciones Library→Viewer: la surface vive toda la vida de la
    /// ventana (productor único EGL).
    pub(crate) fn recreate_surface(&mut self, win: &NativeWindow) {
        unsafe {
            // Re-forzar la geometría RGBA8888 como en `set_window` (si la
            // ventana se usó con un lock CPU del fallback sin EGL, el create
            // puede fallar con EGL_BAD_ALLOC (0x3003) si el formato quedó
            // distinto; forzarlo es la defensa documentada).
            if let Err(e) =
                win.set_buffers_geometry(0, 0, Some(HardwareBufferFormat::R8G8B8A8_UNORM))
            {
                warn!("set_buffers_geometry (recreate): {e}");
            }
            // Destrucción PREVIA idempotente (Tarea 2.6): suelta la surface y
            // los FBOs dry/wet si existe versión previa — y las texturas
            // grandes (fade + planos de biblioteca/picker) si quedaron vivas
            // — ANTES de crear nada nuevo. Ninguna creación llega aquí con un
            // recurso del mismo tipo sin liberar.
            self.drop_surface_only();
            let surf = gl::eglCreateWindowSurface(
                self.dpy,
                self.cfg,
                win.ptr().as_ptr().cast(),
                std::ptr::null(),
            );
            if surf.is_null() {
                self.surf_failed += 1;
                warn!(
                    "eglCreateWindowSurface (recreate) failed: eglGetError=0x{:x} (failure #{})",
                    gl::eglGetError(),
                    self.surf_failed
                );
                return;
            }
            if gl::eglMakeCurrent(self.dpy, surf, surf, self.ctx) == 0 {
                // La surface recién creada no puede quedar huérfana: sin este
                // destroy, cada intento fallido filtraba una EGLSurface
                // (nadie la guardaba ni la destruía) — 0x3003 acumulado.
                warn!(
                    "eglMakeCurrent (recreate) failed: eglGetError=0x{:x}",
                    gl::eglGetError()
                );
                gl::eglDestroySurface(self.dpy, surf);
                return;
            }
            gl::eglSwapInterval(self.dpy, 1);
            self.surf = Some(surf);
            self.win_w = win.width();
            self.win_h = win.height();
            gl::glViewport(0, 0, self.win_w, self.win_h);
            self.page_loaded = None;
            self.note_surface_created(self.win_w, self.win_h);

            // Recrear FBOs con la nueva resolución. `drop_surface_only` (al
            // inicio) ya destruyó los previos; los destroy siguientes quedan
            // como defensa idempotente (0,0 → no-op) por si una ruta futura
            // creara FBOs sin pasar por él.
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
            self.note_fbos_created();
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
            // Destrucción de los FBOs dry/wet con contador y log (un evento
            // por par FBO+textura vivo; (0,0) → no-op, sin log).
            let had_dry = self.dry_fbo != 0 || self.dry_tex != 0;
            let had_wet = self.wet_fbo != 0 || self.wet_tex != 0;
            Self::destroy_fbo_with_tex(self.dry_fbo, self.dry_tex);
            Self::destroy_fbo_with_tex(self.wet_fbo, self.wet_tex);
            if had_dry {
                self.fbo_destroyed += 1;
            }
            if had_wet {
                self.fbo_destroyed += 1;
            }
            if had_dry || had_wet {
                info!(
                    "gpu: fbo destroy dry={}/{} wet={}/{} (destroyed={})",
                    self.dry_fbo, self.dry_tex, self.wet_fbo, self.wet_tex, self.fbo_destroyed
                );
            }
            self.dry_fbo = 0;
            self.dry_tex = 0;
            self.wet_fbo = 0;
            self.wet_tex = 0;
            self.dry_dirty = true;
            self.dry_key = None;

            // Fade abandonado a mitad de transición (se sale del visor antes
            // de que expire): liberar YA la textura grande (ventana completa,
            // hasta ~13 MB) en vez de retenerla hasta el próximo frame de
            // visor o el Drop del Gpu — fix Tarea 2.6 (minor diferido de la
            // revisión 2.4). Al volver a abrir un libro el fade se re-subirá
            // con id de snapshot nuevo.
            self.free_fade_tex();

            // Planos de la biblioteca/picker (texturas grandes, fuera del
            // LRU): la ventana desaparece, su contenido no volverá a
            // pintarse — liberarlas aquí (Tarea 2.7).
            self.free_ui_planes();

            if let Some(s) = self.surf.take() {
                let unbind = gl::eglMakeCurrent(
                    self.dpy,
                    gl::EGL_NO_SURFACE,
                    gl::EGL_NO_SURFACE,
                    gl::EGL_NO_CONTEXT,
                );
                let gone = gl::eglDestroySurface(self.dpy, s);
                if gone != 0 {
                    self.note_surface_destroyed();
                }
                if unbind == 0 || gone == 0 {
                    warn!(
                        "drop_surface: unbind={} destroy={} err=0x{:x}",
                        unbind,
                        gone,
                        gl::eglGetError()
                    );
                }
            }
            // Resumen de contadores: punto de control por ciclo. surf y fbo
            // deben quedar a delta 0; tex a delta == ovl_live (ver
            // `log_lifecycle`).
            self.log_lifecycle();
        }
    }
    pub(crate) fn has_surface(&self) -> bool {
        self.surf.is_some()
    }
    fn note_surface_created(&mut self, w: i32, h: i32) {
        self.surf_created += 1;
        info!(
            "gpu: surface create {}x{} (created={})",
            w, h, self.surf_created
        );
    }
    fn note_surface_destroyed(&mut self) {
        self.surf_destroyed += 1;
        info!("gpu: surface drop (destroyed={})", self.surf_destroyed);
    }
    fn note_fbos_created(&mut self) {
        self.fbo_created += 2; // dry + wet: un par FBO+textura por cada una
        info!(
            "gpu: fbo create dry={}/{} wet={}/{} (created={})",
            self.dry_fbo, self.dry_tex, self.wet_fbo, self.wet_tex, self.fbo_created
        );
    }
    /// Resumen de contadores (deltas create−destroy por tipo) en cada drop.
    fn log_lifecycle(&self) {
        let surf = self.surf_created as i64 - self.surf_destroyed as i64;
        let fbo = self.fbo_created as i64 - self.fbo_destroyed as i64;
        let tex = self.tex_created as i64 - self.tex_destroyed as i64;
        info!(
            "gpu: lifecycle surf={surf} fbo={fbo} tex={tex} ovl_live={} surf_failed={} \
             (create/destroy surf={}/{} fbo={}/{} tex={}/{})",
            self.ovl_cache.len(),
            self.surf_failed,
            self.surf_created,
            self.surf_destroyed,
            self.fbo_created,
            self.fbo_destroyed,
            self.tex_created,
            self.tex_destroyed,
        );
    }
    pub(crate) fn clear(&mut self, rgba: [u8; 4]) {
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
    /// Fondo del visor (tema; rojo de error sin documento) para los clears.
    /// La dry FBO y el fb0 comparten color: con el pan aplicado al quad de
    /// composición, las bandas que el desplazamiento deja al descubierto en
    /// fb0 son del mismo tono que el fondo horneado en la dry.
    pub(crate) fn view_bg(&self, reader: &Reader) -> [u8; 4] {
        if reader.doc.is_none() {
            crate::theme::ERROR_BG_RGBA
        } else {
            reader.theme.palette().rgba_bg()
        }
    }
    /// Invalida y limpia el estado de renderizado del documento (FBO dry, clave
    /// de dry, textura cargada y fade). Debe llamarse al cambiar de documento
    /// o al salir del visor para evitar que el visor muestre la página del
    /// documento anterior si las claves coinciden (p. ej. pág 0 / zoom 1.0).
    pub(crate) fn reset_document(&mut self, bg: [u8; 4]) {
        self.dry_dirty = true;
        self.dry_key = None;
        self.page_loaded = None;
        self.free_fade_tex();
        if self.dry_fbo != 0 {
            unsafe {
                gl::glBindFramebuffer(gl::GL_FRAMEBUFFER, self.dry_fbo);
                gl::glViewport(0, 0, self.win_w, self.win_h);
                self.clear(bg);
            }
        }
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
