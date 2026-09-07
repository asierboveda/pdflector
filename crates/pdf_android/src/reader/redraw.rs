// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pipeline de render del visor y la biblioteca (extraído de `reader.rs`, 2026-09-06): `redraw`, `ensure_pages_rendered`, `blit` y los blits de la biblioteca en dos planos (`rebuild_library*`, `splice_*`, `lib_band_covers`), transformaciones página→px (`page_doc_size_px`, `budget_scale`, `centered_base`, `page_size_pt`) y el worker actor de render asíncrono (`WorkerMsg`/`WorkerReq`/`WorkerCmd`/`RenderWorker`, `render_worker_req`, `launch_render`, `start/stop_render_worker`, `poll_render`, `render_in_flight_for`).

use super::Reader;
use super::UiMode;
use super::geometry::grid_cell_h;
use super::geometry::lib_content_y0;
use super::geometry::lib_search_chips_y0;
use super::geometry::lib_search_chips_y1;
use super::geometry::list_row_h;
use crate::draw::blit_library;
use crate::draw::paste_lib_thumbs;
use crate::draw::render_eraser_cursor;
use crate::draw::render_library_header;
use crate::draw::render_library_zone;
use crate::draw::render_mode_badge;
use crate::draw::render_page_badge;
use crate::draw::render_picker_list;
use crate::draw::render_search_chip_row;
use crate::draw::render_sheet;
use crate::draw::render_toast;
use crate::draw::render_viewer_bottom_chrome;
use crate::draw::render_viewer_top_chrome;
use crate::draw::splice_row;
use crate::theme;
use crate::view::initial_scale;
use crate::zoom::blit_fast;
use log::error;
use log::info;
use log::warn;
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::{Bitmap, Document, RenderEngine};
use std::time::Instant;

/// Mensaje del worker de render asíncrono: el bitmap cacheable a la escala
/// pedida (`target_zoom` = factor de zoom con el que se renderizó, la "escala
/// efectiva" = cover × target_zoom) + metadatos del render. El bitmap NO es
/// el render full: es su recorte a la ventana del worker — X CENTRADO +
/// Y ALINEADO ARRIBA (`crop_rect`, fix de residency — el full a cover pesa
/// 27,4 MiB en landscape y dejaba 1 solo residente en la caché; el recorte
/// deja ≤ ~12,7 MiB → ~3 residentes). `full_w/full_h` son las dims del
/// render FULL (lo que se dibujó antes de recortar) y `crop_x/crop_y` el
/// origen del recorte dentro de él: los consumidores que convierten pantalla
/// ↔ píxeles del render (transición fast→sharp del pinch, selección →
/// imagen, capa de anotaciones/tinta) compensan el origen.
pub(crate) struct WorkerMsg {
    seq: u64,
    page: u32,
    bitmap: Bitmap,
    target_zoom: f32,
    full_w: u32,
    full_h: u32,
    crop_x: u32,
    crop_y: u32,
}

/// Petición de render al worker actor: páginas a la escala pedida, ventana
/// congelada del cover y canal de respuesta propio por lote. Port de F3.1
/// (`mejora_zoom`): el hilo drena comandos entre páginas (preemption por
/// `seq`) en `render_worker_req`.
struct WorkerReq {
    seq: u64,
    pages: Vec<u32>,
    target_zoom: f32,
    clamp_level: bool,
    win_w: i32,
    win_h: i32,
    reply: std::sync::mpsc::Sender<WorkerMsg>,
}

/// Comandos del worker actor: render de la última petición o parada limpia.
enum WorkerCmd {
    Render(WorkerReq),
    Stop,
}

/// Controlador del worker actor de render (F3.1): canal de comandos + join
/// handle. El hilo retiene su propio `MupdfDocument` (MuPDF no es Send) y
/// muere solo con `Stop`. Sustituye al hilo-por-zoom anterior, que disparaba
/// el contador de hilos y reabría el PDF en cada gesto.
pub(crate) struct RenderWorker {
    tx: std::sync::mpsc::Sender<WorkerCmd>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// Ejecuta `req` en el worker: renderiza las páginas a la escala pedida y
/// envía cada bitmap por `req.reply`. ENTRE páginas drena el canal: una
/// petición con `seq` mayor sustituye a la actual y `Stop` sale del actor.
/// Errores por página: best-effort (drop silencioso; el UI muestra fallback).
fn render_worker_req(
    doc: &MupdfDocument,
    rx: &std::sync::mpsc::Receiver<WorkerCmd>,
    req: WorkerReq,
) {
    let target = if req.clamp_level {
        let level = pdf_core::scale_level_for_zoom(req.target_zoom).min(1);
        2f32.powi(level as i32)
    } else {
        req.target_zoom
    };
    for page in req.pages {
        // Preemption: un seq posterior ya lanzado anula las páginas restantes.
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                WorkerCmd::Stop => return,
                WorkerCmd::Render(newer) => {
                    if newer.seq > req.seq {
                        render_worker_req(doc, rx, newer);
                        return;
                    }
                    // seq <= req.seq: lote viejo encolado, ignorar.
                }
            }
        }
        if let Ok((pw, ph)) = doc.page_size(page) {
            let cover = initial_scale(pw, ph, req.win_w, req.win_h);
            // Presupuesto: el bitmap debe caber en la caché (misma regla que
            // `Reader::budget_scale` — duplicada aquí porque el worker no
            // tiene acceso a `self`).
            let mut scale = cover * target;
            let px_pdf = pw as f64 * ph as f64;
            let max_px = crate::cache::CACHE_BYTE_BUDGET as f64 / 4.0;
            while scale > 0.001 && px_pdf * scale as f64 * scale as f64 > max_px {
                scale *= 0.5;
            }
            let target_eff = if cover > 0.0 { scale / cover } else { 1.0 };
            if let Ok(bmp) = doc.render_page(page, scale) {
                let (full_w, full_h) = (bmp.width, bmp.height);
                // Crop a la ventana (fix raíz de residency): el render full a
                // cover excede la ventana en al menos un eje y pesa hasta
                // 27,4 MiB en landscape (2200×3112) — solo cabía 1 residente
                // en la caché de 48 MiB y cada turno re-renderizaba (~115 ms).
                // Recortado a (min(bw, win_w), min(bh, win_h)) queda ≤ ~12,7
                // MiB → ~3 residentes y turnos sin re-render. Origen: X
                // CENTRADO (compensa el centrado X del blit) e Y = 0 (el blit
                // alinea ARRIBA en Y — un crop centrado en Y mostraría la
                // franja central de la página en vez de la superior). La
                // composición NO cambia: a blit_zoom == 1 el crop reproduce
                // los píxeles del render full (ver render_dry); los
                // metadatos permiten a los consumidores (pinch fast→sharp,
                // sel_image) volver a la cuadrícula del render full.
                let (bitmap, (crop_x, crop_y)) = if req.win_w > 0 && req.win_h > 0 {
                    let (cw, ch) = (
                        bmp.width.min(req.win_w as u32),
                        bmp.height.min(req.win_h as u32),
                    );
                    if cw == bmp.width && ch == bmp.height {
                        (bmp, (0, 0)) // ya cabe en la ventana: sin recorte
                    } else {
                        let x = (bmp.width - cw) / 2;
                        (pdf_core::crop_rect(&bmp, x, 0, cw, ch), (x, 0))
                    }
                } else {
                    (bmp, (0, 0))
                };
                let _ = req.reply.send(WorkerMsg {
                    seq: req.seq,
                    page,
                    bitmap,
                    target_zoom: target_eff,
                    full_w,
                    full_h,
                    crop_x,
                    crop_y,
                });
            }
        }
    }
}

impl Reader {
    /// Redibuja: re-render si cambió página, zoom o tamaño de ventana, y blit.
    pub(crate) fn redraw(&mut self) {
        let (w, h) = match self.window.as_ref() {
            Some(win) => (win.width(), win.height()),
            None => return,
        };
        if w <= 0 || h <= 0 {
            return;
        }
        if w != self.win_w || h != self.win_h {
            self.win_w = w;
            self.win_h = h;
            self.bitmap = None; // lista del picker → re-render
            self.library.lib_header = None; // zona fija de la biblioteca: tamaño nuevo
            self.library.lib_band = None; // banda de contenido: tamaño nuevo
            self.cache.clear(); // nueva escala cover → los bitmaps viejos no sirven
            self.list_dirty = true;
            self.page_badge = None;
            self.chrome_top_bitmap = None;
            self.chrome_bottom_bitmap = None;
            self.sheet_bitmap = None;
        }
        match self.mode {
            UiMode::Viewer => {
                // Modo UNA HOJA: la página actual + vecinas se garantizan de
                // forma ASÍNCRONA (worker, patrón de `goto_page`): si alguna
                // falta y no hay ya un lote en vuelo, se lanza — el render
                // síncrono aquí congelaba el UI cada `RedrawNeeded` del
                // sistema (medido: 3 páginas × 40-120 ms, repetido).
                let needs = {
                    let n = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                    let lo = self.page.saturating_sub(1);
                    let hi = (self.page + 1).min(n.saturating_sub(1));
                    (lo..=hi)
                        .filter(|&p| self.cache.peek(p).is_none())
                        .collect::<Vec<u32>>()
                };
                if !needs.is_empty() && self.render_rx.is_none() {
                    self.launch_render(needs, self.rendered_zoom, false);
                }
                // Chrome del visor y sheet de ajustes
                if self.chrome_visible {
                    if self.chrome_top_bitmap.is_none() {
                        self.chrome_top_bitmap = render_viewer_top_chrome(self);
                        self.chrome_top_id = self.next_ovl_id();
                    }
                    if self.chrome_bottom_bitmap.is_none() {
                        self.chrome_bottom_bitmap = render_viewer_bottom_chrome(self);
                        self.chrome_bottom_id = self.next_ovl_id();
                    }
                }
                if self.doc.is_some() && self.page_badge.is_none() {
                    self.page_badge = render_page_badge(self);
                    self.page_badge_id = self.next_ovl_id();
                }
                if self.sheet_progress > 0.0 && self.sheet_bitmap.is_none() {
                    self.sheet_bitmap = render_sheet(self);
                    self.sheet_id = self.next_ovl_id();
                }
                if self.sheet_progress <= 0.0 {
                    self.sheet_bitmap = None;
                }
            }
            UiMode::Picker => {
                // Clamp del scroll si la lista menguó (rescan/cancel) o cambió
                // la ventana (picker: filas de `picker_row_h`).
                let max_scroll = self.picker_len().saturating_sub(self.picker_visible());
                if self.list_scroll > max_scroll {
                    self.list_scroll = max_scroll;
                }
                if self.list_dirty
                    && let Some(bmp) = render_picker_list(self)
                {
                    self.bitmap = Some(bmp);
                    // Generación nueva del bitmap de la lista: la textura GPU
                    // dedicada se re-subirá en el present (Tarea 2.7).
                    self.picker_bmp_ver += 1;
                    self.offset_x = 0;
                    self.offset_y = 0;
                    self.list_dirty = false;
                }
            }
            UiMode::Library => {
                // Clamp del scroll VERTICAL (px) si el contenido menguó
                // (filtro, rescan, ventana): cabecera + campo de búsqueda +
                // franja de estado son fijas; el contenido (Continue Reading
                // + My Library) scrollea bajo ellas. Los scrolls
                // HORIZONTALES (carousel, panel de búsqueda y organización)
                // se clampean igual contra su ancho total.
                let max_v = self.lib_max_scroll();
                if self.library.lib_scroll > max_v {
                    self.library.lib_scroll = max_v;
                }
                self.library.lib_carousel_x =
                    self.library.lib_carousel_x.min(self.lib_cont_max_x());
                self.library.lib_letters_x =
                    self.library.lib_letters_x.min(self.lib_chips_max_x(0));
                self.library.lib_folders_x =
                    self.library.lib_folders_x.min(self.lib_chips_max_x(1));
                self.library.lib_sort_x = self.library.lib_sort_x.min(self.lib_org_max_x(0));
                self.library.lib_filter_x = self.library.lib_filter_x.min(self.lib_org_max_x(1));

                if self.list_dirty {
                    // Cambio ESTRUCTURAL (datos/filtros/sort/search/estado/
                    // ventana/entrada): re-renderizar cabecera + banda de
                    // contenido + filas horizontales + portadas. Es el
                    // render CARO (Canvas+JNI), pagado una vez por cambio,
                    // nunca por frame de scroll.
                    self.rebuild_library();
                } else if let Some(zone) = self.library.lib_row_dirty {
                    // Solo una fila HORIZONTAL se arrastró (carousel o
                    // chips): re-renderizar ESA fila y remendarla sobre su
                    // contenedor — barato (área pequeña), sin tocar el resto.
                    self.rebuild_library_row(zone);
                } else if !self.lib_band_covers() {
                    // El scroll salió de la banda actual: re-bandear
                    // (render de la banda en la nueva posición; cabecera
                    // intacta).
                    self.rebuild_library_band();
                }
            }
        }
        if self.window.is_some() {
            self.blit();
        }
    }

    // ---------------------------------------------------------------------
    // Biblioteca: render CACHEADO en dos planos (zona fija + banda de
    // contenido). El scroll vertical por frame es un memcpy (blit_library),
    // no un re-render Canvas+JNI (~20-60 ms): el mismo patrón que
    // compose_frame/blit_composed del visor aplicado a la biblioteca
    // (2026-08-22, fix del lag/parpadeo del scroll reportado).
    // ---------------------------------------------------------------------
    /// Rebuild COMPLETO de la biblioteca: cabecera (zona fija) + banda de
    /// contenido + filas horizontales + portadas. Se llama solo cuando
    /// cambia la ESTRUCTURA (datos, filtros, sort, panel de búsqueda,
    /// status, ventana, entrada), nunca por frame de scroll.
    fn rebuild_library(&mut self) {
        self.list_dirty = false;
        self.library.lib_row_dirty = None;
        // 1) Zona fija (cabecera editorial + campo de búsqueda + panel +
        //    franja de estado).
        self.library.lib_header = render_library_header(self);
        // Generación nueva del plano de cabecera: la textura GPU dedicada se
        // re-subirá en el próximo present (Tarea 2.7; ver `lib_header_ver`).
        self.library.lib_header_ver += 1;
        // 2) Banda de contenido en la posición actual del scroll.
        self.rebuild_library_band();
        // 3) Filas horizontales dentro de sus contenedores (carousel,
        //    chips del panel de búsqueda y de organización).
        self.splice_library_rows();
    }

    /// Re-renderiza SOLO la banda de contenido (sin tocar la cabecera): se
    /// llama al entrar/salir de una banda (scroll lejos del rango actual) o
    /// al rebuild completo. El render es Canvas+JNI UNA vez por banda; el
    /// scroll dentro de la banda es memcpy.
    fn rebuild_library_band(&mut self) {
        let content_y0 = lib_content_y0(
            self.win_h,
            self.library.lib_search_open,
            self.status.is_some(),
        );
        let viewport = (self.win_h - content_y0).max(0);
        let content_h = self.lib_content_h() as i32;
        let margin = if self.is_grid() {
            let cols = self.effective_grid_cols();
            grid_cell_h(self.win_w, cols, self.cover_size) as i32
        } else {
            list_row_h(self.win_h, self.cover_size) as i32
        };
        let band_h = (viewport + 2 * margin).min(content_h.max(viewport));
        let band_origin = ((self.library.lib_scroll as i32) - margin)
            .max(0)
            .min((content_h - band_h).max(0));
        if let Some(bmp) = render_library_zone(self, band_origin, band_h) {
            let mut band = bmp;
            paste_lib_thumbs(self, &mut band, band_origin);
            self.library.lib_band = Some((band, band_origin));
            // Generación nueva de la banda: la textura GPU dedicada se
            // re-subirá en el próximo present (Tarea 2.7; ver
            // `lib_band_ver`).
            self.library.lib_band_ver += 1;
            self.splice_band_rows();
        } else {
            self.library.lib_band = None;
        }
    }

    /// ¿La banda actual cubre la ventana de contenido con el scroll actual?
    /// false → hay que re-bandear (render de la banda en la nueva posición).
    fn lib_band_covers(&self) -> bool {
        match &self.library.lib_band {
            None => false,
            Some((bmp, origin)) => {
                let content_y0 = lib_content_y0(
                    self.win_h,
                    self.library.lib_search_open,
                    self.status.is_some(),
                );
                let viewport = (self.win_h - content_y0).max(0);
                let s = self.library.lib_scroll as i32;
                s >= *origin && s + viewport <= *origin + bmp.height as i32
            }
        }
    }

    /// Re-renderiza SOLO la fila horizontal `zone` (pequeña, Canvas+JNI
    /// barato) y la remienda sobre su contenedor: el arrastre horizontal del
    /// carousel o de chips no re-renderiza la pantalla completa.
    fn rebuild_library_row(&mut self, zone: u8) {
        self.library.lib_row_dirty = None;
        match zone {
            2 | 3 => {
                // Chips del panel de búsqueda → cabecera (zona fija).
                let row = render_search_chip_row(self, (zone - 2) as usize);
                let x = if zone == 2 {
                    self.library.lib_letters_x as i32
                } else {
                    self.library.lib_folders_x as i32
                };
                let y = if zone == 2 {
                    lib_search_chips_y0(self)
                } else {
                    lib_search_chips_y1(self)
                };
                if let (Some(row), Some(h)) = (row, self.library.lib_header.as_mut()) {
                    splice_row(h, &row, -x, y as i32);
                    // Cabecera mutada in-place: nueva generación de su plano
                    // (Tarea 2.7 — la textura GPU se re-subirá).
                    self.library.lib_header_ver += 1;
                }
            }
            // Zonas 1 (carousel), 4 y 5 (sort/filter) ya no existen en la
            // biblioteca minimalista: sin filas que remendar.
            _ => {}
        }
    }

    /// Remienda todas las filas horizontales sobre sus contenedores
    /// (cabecera: chips de búsqueda; banda: carousel + chips de organización).
    /// Se llama tras un rebuild completo (los contenedores acaban de
    /// renderizarse SIN las filas, que se leen de `lib_*_x`).
    fn splice_library_rows(&mut self) {
        let letters_row = if self.library.lib_search_open {
            render_search_chip_row(self, 0)
        } else {
            None
        };
        let folders_row = if self.library.lib_search_open {
            render_search_chip_row(self, 1)
        } else {
            None
        };
        let lx = self.library.lib_letters_x as i32;
        let fx = self.library.lib_folders_x as i32;
        let cy0 = lib_search_chips_y0(self) as i32;
        let cy1 = lib_search_chips_y1(self) as i32;
        if let Some(header) = self.library.lib_header.as_mut() {
            let mut spliced = false;
            if let Some(row) = letters_row {
                splice_row(header, &row, -lx, cy0);
                spliced = true;
            }
            if let Some(row) = folders_row {
                splice_row(header, &row, -fx, cy1);
                spliced = true;
            }
            if spliced {
                // Cabecera mutada in-place: nueva generación de su plano
                // (Tarea 2.7 — la textura GPU se re-subirá).
                self.library.lib_header_ver += 1;
            }
        }
        self.splice_band_rows();
    }

    /// Remienda las filas horizontales de la BANDA sobre la banda actual.
    /// Biblioteca MINIMALISTA (estilo Readest): NO hay carousel de Continue
    /// Reading ni chips de sort/filter, así que no se remienda ninguna fila
    /// en la banda (solo los chips del panel de BÚSQUEDA, que viven en la
    /// cabecera fija).
    pub(crate) fn splice_band_rows(&mut self) {
        // Sin filas horizontales en la banda (carousel/organización ocultos).
    }

    /// Tamaño de la página `page` en px de ventana a zoom 1 (cover × puntos
    /// PDF): las dimensiones que el usuario ve con el factor 1.0, base del
    /// centrado y del anclaje del pinch. Equivale a `bitmap_cached.width /
    /// rendered_zoom` (los bitmaps se renderizan a cover × rendered_zoom);
    /// se calcula de la página para no depender de un hit de caché.
    pub(crate) fn page_doc_size_px(&self, page: u32) -> (f32, f32) {
        let Some(doc) = self.doc.as_ref() else {
            return (0.0, 0.0);
        };
        let Ok((pw, ph)) = doc.page_size(page) else {
            return (0.0, 0.0);
        };
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        (pw * cover, ph * cover)
    }

    /// Escala límite por PRESUPUESTO de píxeles: el bitmap de la página
    /// (`pw×ph` pt) a esa escala no debe superar la caché (~48 MiB). Reduce la
    /// escala pedida por mitades hasta caber; nunca devuelve 0. Evita el
    /// render de cientos de MB al abrir con un zoom alto guardado (la
    /// "pillada") tanto en el worker como en el render síncrono del arranque.
    #[allow(dead_code)] // regla documentada; el worker la duplica
    fn budget_scale(&self, pw: f32, ph: f32, scale: f32) -> f32 {
        let max_px = crate::cache::CACHE_BYTE_BUDGET as f64 / 4.0;
        let mut s = scale.max(0.001);
        let px_pdf = pw as f64 * ph as f64;
        while px_pdf * s as f64 * s as f64 > max_px {
            s *= 0.5;
            if s <= 0.01 {
                break;
            }
        }
        s
    }

    /// Esquina superior izquierda del bitmap escalado para centrado
    /// horizontal: `base(z) = (win − doc·z) / 2` (px de zoom 1), la misma
    /// fórmula que `blit` usa para `dx` sin pan. Lineal en `z`; en el
    /// anclaje Y la base es 0 (el borde superior de la página actual está
    /// fijo en el borde superior del viewport — modo UNA HOJA, sin scroll).
    pub(crate) fn centered_base(win: i32, doc: f32, z: f32) -> f32 {
        (win as f32 - doc * z) / 2.0
    }

    /// Tamaño de página en puntos PDF (`None` si no hay documento o falla).
    pub(crate) fn page_size_pt(&self, page: u32) -> Option<(f32, f32)> {
        self.doc.as_ref()?.page_size(page).ok()
    }

    /// Garantiza en la caché la página actual + 1 vecina por cada lado
    /// (prefetch simple para que prev/next sea INSTANTÁNEO): renderiza solo
    /// los miss, a `cover × rendered_zoom` (la escala de la caché), y
    /// promueve la recencia LRU de las páginas que toca. El render es
    /// síncrono en el hilo del bucle (~18-25 ms/página en la tablet); el
    /// prefetch adelanta la vecina para que el tap de página entre en ella
    /// sin re-render en el momento de volverse visible. En el modo UNA HOJA
    /// SOLO se DIBUJA la página actual (`blit`): las vecinas solo se cachean.
    #[allow(dead_code)] // superado por el render async (worker)
    fn ensure_pages_rendered(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let n = doc.page_count();
        if n == 0 {
            return;
        }
        let lo = self.page.saturating_sub(1);
        let hi = (self.page + 1).min(n - 1);
        // Orden: vecinas PRIMERO y la página actual ÚLTIMA. Con zoom alto cada
        // página (~36 MiB) supera el presupuesto de la caché (48 MiB), de modo
        // que renderizar la actual en medio hacía que la última vecina la
        // EVICTARA y el blit no encontrara bitmap → pantalla en blanco (fondo
        // puro) al soltar el pinch. Renderizarla última garantiza que sobreviva
        // a la evicción (las vecinas son prefetch best-effort y se re-renderizan
        // al navegar).
        let mut order: Vec<u32> = (lo..=hi).filter(|&p| p != self.page).collect();
        order.push(self.page);
        for page in order {
            if self.cache.get(page).is_some() {
                continue; // hit: sin re-render (volver atrás es instantáneo)
            }
            let (pw, ph) = match doc.page_size(page) {
                Ok(s) => s,
                Err(e) => {
                    error!("page_size {page}: {e}");
                    continue;
                }
            };
            let scale = initial_scale(pw, ph, self.win_w, self.win_h) * self.rendered_zoom;
            // Presupuesto (mismo límite que el worker): evita el render de
            // cientos de MB al abrir con zoom alto guardado.
            let scale = self.budget_scale(pw, ph, scale);
            let t0 = Instant::now();
            match doc.render_page(page, scale) {
                Ok(bmp) => {
                    let ms = t0.elapsed().as_secs_f64() * 1000.0;
                    info!(
                        "render page {} @scale {scale:.3} -> {}x{} px: {ms:.2} ms (cache: {} pages / {:.1} MiB)",
                        page + 1,
                        bmp.width,
                        bmp.height,
                        self.cache.len(),
                        self.cache.resident_bytes() as f64 / (1024.0 * 1024.0)
                    );
                    self.cache.insert(
                        page,
                        crate::cache::CachedPage {
                            // Camino SÍNCRONO (legacy, sin crop): el render
                            // ocupa el full de su cuadrícula (origen 0). El
                            // worker async es el que recorta a ventana.
                            full_w: bmp.width,
                            full_h: bmp.height,
                            crop_x: 0,
                            crop_y: 0,
                            bitmap: bmp,
                        },
                    );
                }
                Err(e) => {
                    error!("render page {page}: {e}");
                }
            }
        }
    }

    /// Presenta el frame actual según el modo y el motor disponible.
    ///
    /// - Visor (modo UNA HOJA): present GPU por EGL (`present_viewer`):
    ///   página como textura (subida SOLO al cambiar página/re-render),
    ///   tinta como geometría y overlays como quads por frame; swap por
    ///   `eglSwapBuffers`.
    /// - Picker/Biblioteca: por GPU (productor único, Tarea 2.7) los planos
    ///   cacheados (`lib_header`/`lib_band`; bitmap de la lista) se dibujan
    ///   como texturas dedicadas (re-subidas solo cuando cambian) + swap.
    ///   SIN EGL (`gpu` None o surface sin crear) degradan al camino SW
    ///   histórico con `ANativeWindow_lock` (`blit_library`/`blit_fast`) —
    ///   el lock CPU NUNCA convive con una surface EGL activa.
    ///
    /// Aquí se decide SOLO el estado que depende del `Reader`: fondo rojo sin
    /// documento y materialización de los bitmaps de overlay (toast, badge,
    /// cursor de goma) que faltan. Con el boli activo usa dirty rect +
    /// coalescing por vsync (el bucle principal lo llama una vez por
    /// iteración tras `take_repaint`).
    pub(crate) fn blit(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let t0 = Instant::now();
        let p = self.theme.palette();
        let bg = if self.doc.is_none() {
            theme::ERROR_BG_RGBA
        } else {
            p.rgba_bg()
        };
        // ¿Hay surface EGL usable? Con EGL, TODOS los modos presentan por GL
        // (productor único, Tarea 2.7): la surface ya NO se suelta al entrar
        // en Library/Picker — una ANativeWindow admite UN solo productor de
        // BufferQueue, y alternar el lock CPU de los blits SW con la surface
        // EGL agotaba el slot (eglCreateWindowSurface → EGL_BAD_ALLOC 0x3003
        // en cada vuelta Library→Viewer, TCL 2026-09-04). El camino SW con
        // `ANativeWindow_lock` queda SOLO como fallback sin EGL (gpu None o
        // surface sin crear en esta ventana).
        let egl_ok = self.gpu.as_ref().is_some_and(|g| g.has_surface());
        let mut sw_blit = false;
        match self.mode {
            UiMode::Viewer => {
                // FASE 2 (ADR-006): presentación por GPU (EGL/GLES2). La
                // página es una textura (subida SOLO al cambiar página o
                // re-render nítido), la tinta es geometría (strips con AA),
                // los overlays son quads de los bitmaps Canvas+JNI ya
                // generados y el present es `eglSwapBuffers` (spike 1: p50
                // 0.17 ms). Sin dirty rect CPU: frame completo por vsync.
                //
                // Materialización de overlays: los bitmaps se generan aquí si
                // faltan y `present_viewer` los sube como texturas cacheadas
                // por id de generación (Tarea 2.4: nunca por puntero — ABA
                // cuando el allocator reusa la dirección de un bitmap ya
                // liberado).
                if self.toast.is_some() && self.toast_bitmap.is_none() {
                    self.toast_bitmap = render_toast(self);
                    // Id nuevo inline: `window` (borrow de `self.window`) vive
                    // hasta el blit de abajo — un `&mut self` completo
                    // (next_ovl_id) chocaría; los campos son disjuntos.
                    self.ovl_seq += 1;
                    self.toast_id = self.ovl_seq;
                }
                if !self.chrome_visible && self.mode_badge.is_none() {
                    self.mode_badge = render_mode_badge(self);
                    self.ovl_seq += 1;
                    self.mode_badge_id = self.ovl_seq;
                }
                if self.erase_pt.is_some() && self.eraser_cursor.is_none() && self.erase_r_px > 4.0
                {
                    self.eraser_cursor = render_eraser_cursor(self, self.erase_r_px as i32);
                    self.ovl_seq += 1;
                    self.eraser_cursor_id = self.ovl_seq;
                }
                // Present GPU: se toma el Gpu del Option (take) para poder
                // pasar el Reader (reborrow `&mut`) sin conflicto de
                // préstamos — el present LEE el Reader y consume la
                // instrumentación del cambio de página (`page_turn_t0` →
                // log `page_turn`, A2).
                if let Some(mut g) = self.gpu.take() {
                    g.present_viewer(self);
                    self.gpu = Some(g);
                }
            }
            UiMode::Library => {
                // Zona fija (`lib_header`) + banda de contenido (`lib_band`)
                // CACHEADAS: el present por frame usa los planos ya
                // renderizados (Canvas+JNI UNA vez por cambio estructural o
                // re-band; el scroll solo reposiciona la banda), NO un
                // re-render por frame.
                let content_y0 = lib_content_y0(
                    self.win_h,
                    self.library.lib_search_open,
                    self.status.is_some(),
                );
                // Aviso breve (toast) integrado en el MISMO present que la
                // biblioteca (antes: un segundo present por frame durante
                // ~1,5 s — innecesario). Materialización común a los dos
                // caminos (GPU y SW).
                if self.toast.is_some() && self.toast_bitmap.is_none() {
                    self.toast_bitmap = render_toast(self);
                    // Id nuevo inline: `window` (borrow de `self.window`)
                    // vive hasta el blit de abajo.
                    self.ovl_seq += 1;
                    self.toast_id = self.ovl_seq;
                }
                if egl_ok {
                    if let Some(mut g) = self.gpu.take() {
                        g.present_library(self, content_y0);
                        self.gpu = Some(g);
                    }
                } else {
                    // Fallback SW (sin EGL): composición al buffer de la
                    // ventana con un solo lock+present.
                    sw_blit = true;
                    let header = self.library.lib_header.as_ref();
                    let band = self.library.lib_band.as_ref().map(|(b, o)| (b, *o));
                    let toast_ov: Option<(&Bitmap, i32, i32)> =
                        self.toast_bitmap.as_ref().map(|tb| {
                            let tx = (self.win_w - tb.width as i32) / 2;
                            let ty = self.win_h - tb.height as i32 - 16;
                            (tb, tx, ty)
                        });
                    blit_library(
                        window,
                        p.rgba_lib_bg(),
                        header,
                        band,
                        self.library.lib_scroll as i32,
                        content_y0,
                        toast_ov,
                    );
                }
            }
            UiMode::Picker => {
                if egl_ok {
                    // Ídem: la lista del picker como textura dedicada + swap.
                    if let Some(mut g) = self.gpu.take() {
                        g.present_picker(self);
                        self.gpu = Some(g);
                    }
                } else {
                    // Fallback SW (sin EGL).
                    sw_blit = true;
                    match self.bitmap.as_ref() {
                        Some(bmp) => {
                            blit_fast(window, bmp, 1.0, bg, (self.offset_x, self.offset_y), None)
                        }
                        None => {
                            // Sin lista: solo el fondo (guard hace
                            // unlock_and_post al caer).
                            let Ok(mut guard) = window.lock(None) else {
                                warn!("ANativeWindow_lock failed");
                                return;
                            };
                            let bpp = match guard.format().bytes_per_pixel() {
                                Some(b) => b,
                                None => {
                                    warn!(
                                        "buffer format without bytes_per_pixel: {:?}",
                                        guard.format()
                                    );
                                    return;
                                }
                            };
                            let dst_w = guard.width();
                            let dst_h = guard.height();
                            let dst_stride = guard.stride(); // en píxeles
                            let dst = guard.bits() as *mut u8;
                            crate::draw::fill_buffer(dst, dst_w, dst_h, dst_stride, bpp, bg);
                        }
                    }
                }
            }
        }
        // Log SOLO del camino SW (Library/Picker): los presents GPU loguean
        // su propio tiempo (gl_present / blit ... swap) — comparable con la
        // Fase 1 y con el camino SW. El probe de tinta mantiene el nombre de
        // evento para comparar con la Fase 1.
        if sw_blit {
            info!(
                "blit {}x{}: {:.2} ms (lock+copy+unlock_and_post)",
                self.win_w,
                self.win_h,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some((x0, y0, x1, y1)) = self.take_repaint_probe {
            self.take_repaint_probe = None;
            info!(
                "ink_dirty {}x{} px ({}x{} @ {},{}): {:.2} ms",
                x1 - x0,
                y1 - y0,
                x1 - x0,
                y1 - y0,
                x0,
                y0,
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Lanza el render ASÍNCRONO de `pages` a la escala `target_zoom`
    /// (factor de zoom — el worker calcula `cover × target_zoom` con su
    /// propio documento). `clamp_level` limita el render a un nivel 2^x
    /// cercano (early sharp durante el pinch: evita renders gigantes por
    /// cada Move; el sharp final usa `clamp_level=false` para nitidez
    /// máxima). Reemplaza cualquier lote anterior (seq++).
    ///
    /// **Presupuesto de píxeles**: el worker NUNCA produce un bitmap mayor
    /// que la caché (`CACHE_BYTE_BUDGET`, 48 MiB) — un render de la página a
    /// zoom alto (p. ej. 8800×11640 px = 400 MB) petaba la RAM de la tablet
    /// ("se queda pillada") y expulsaba toda la caché. La escala se reduce
    /// por mitades hasta caber; el `target_zoom` enviado refleja el zoom
    /// EFECTIVO (escala/cover) para que el blit quede 1:1.
    pub(crate) fn launch_render(&mut self, pages: Vec<u32>, target_zoom: f32, clamp_level: bool) {
        // Actor persistente (F3.1): sin worker (documento aún sin abrir del
        // todo) no hay a quién pedir — los llamantes previos al open no
        // lanzaban nada útil tampoco (el hilo efímero fallaba al abrir).
        let Some(worker) = self.render_worker.as_ref() else {
            return;
        };
        self.render_seq += 1;
        let seq = self.render_seq;
        let (tx, rx) = std::sync::mpsc::channel::<WorkerMsg>();
        self.render_rx = Some(rx);
        let (win_w, win_h) = (self.win_w, self.win_h);
        let _ = worker.tx.send(WorkerCmd::Render(WorkerReq {
            seq,
            pages,
            target_zoom,
            clamp_level,
            win_w,
            win_h,
            reply: tx,
        }));
    }

    /// ¿Hay un lote en vuelo para este `zoom`? (ventana ±50%: el sharp final
    /// y el early-sharp del pinch comparten objetivo aproximado).
    pub(crate) fn render_in_flight_for(&self, zoom: f32) -> bool {
        self.render_rx.is_some() && (self.rendered_zoom - zoom).abs() / zoom.max(1e-4) < 0.5
    }

    /// Arranca el worker actor de render con `path` como documento. Llamado
    /// UNA vez por documento (`open_pdf_at` y el intent "abrir con"): el
    /// hilo abre SU PROPIO `MupdfDocument` (MuPDF no es Send) y lo retiene
    /// hasta `Stop`. Un worker anterior se detiene antes.
    pub(crate) fn start_render_worker(&mut self, path: &str) {
        self.stop_render_worker();
        let (tx, rx) = std::sync::mpsc::channel::<WorkerCmd>();
        let path = path.to_string();
        let handle = std::thread::Builder::new()
            .name("render-worker".into())
            .spawn(move || {
                let doc = match MupdfEngine::new().and_then(|e| e.open(std::path::Path::new(&path)))
                {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("render-worker: open {path}: {e}");
                        return;
                    }
                };
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        WorkerCmd::Stop => break,
                        WorkerCmd::Render(req) => {
                            render_worker_req(&doc, &rx, req);
                        }
                    }
                }
            })
            .ok();
        self.render_worker = Some(RenderWorker { tx, handle });
    }

    /// Detiene el worker actor (si existe): envía `Stop` y hace `join()`.
    /// Llamado al cambiar de documento y desde `Drop for Reader`.
    pub(crate) fn stop_render_worker(&mut self) {
        if let Some(w) = self.render_worker.take() {
            let _ = w.tx.send(WorkerCmd::Stop);
            if let Some(handle) = w.handle {
                let _ = handle.join();
            }
        }
        // Los resultados en vuelo de lotes antiguos caducan solos:
        // `poll_render` ya descarta por `seq != render_seq`.
    }

    /// Sondeo del worker de render (desde `tick`): aplica los bitmaps
    /// recibidos a la caché y, cuando llega la página actual, fija
    /// `rendered_zoom` al nivel del lote y repintea (el blit pasa de preview
    /// escalado a 1:1 nítido). Los lotes obsoletos (seq viejo) se descartan.
    /// Al desconectarse el worker (terminado), libera el canal.
    pub(crate) fn poll_render(&mut self) {
        loop {
            let Some(rx) = self.render_rx.as_ref() else {
                return;
            };
            match rx.try_recv() {
                Ok(msg) => {
                    if msg.seq != self.render_seq {
                        continue; // lote obsoleto: descartar
                    }
                    self.cache.insert(
                        msg.page,
                        crate::cache::CachedPage {
                            bitmap: msg.bitmap,
                            full_w: msg.full_w,
                            full_h: msg.full_h,
                            crop_x: msg.crop_x,
                            crop_y: msg.crop_y,
                        },
                    );
                    if msg.page == self.page {
                        self.rendered_zoom = msg.target_zoom;
                        self.fallback_page = None;
                        self.mark_repaint();
                        // La dry puede estar horneada con el FALLBACK bajo la
                        // clave de la página nueva (la `DryKey` no cambia al
                        // llegar el render real: misma página/zoom/anns/dark).
                        // Sin esta invalidación la página real no se mostraría
                        // nunca (quedaría el fallback hasta otra invalidación).
                        self.gpu.as_mut().map(|g| g.invalidate_dry());
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.render_rx = None;
                    break;
                }
            }
        }
    }
}
