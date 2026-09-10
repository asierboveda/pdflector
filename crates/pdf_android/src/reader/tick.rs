// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Tick del bucle de eventos (extraído de `reader.rs`, 2026-09-06): `tick` y su predicado `needs_tick`, visibilidad de filas/carousel (`lib_cont_visible`, `lib_visible_grid_rows`/`lib_visible_list_rows`), pump de portadas (`thumbs_pending`, `ensure_thumb_worker`, `pump_thumbs`), la evicción diferida del pagecache en ticks idle (`trim_to_budget`, fix p95 — ver `crate::cache`) y accesores de frame del bucle (`take_repaint`, `needs_repaint`, `has_window`).

use super::AiPhase;
use super::Reader;
use super::UiMode;
use super::geometry::grid_cell_h;
use super::geometry::lib_cont_block_h;
use super::geometry::lib_content_y0;
use super::geometry::lib_grid_y0;
use super::geometry::list_row_gap;
use super::geometry::list_row_h;
use crate::LIB_FADE_MS;
use crate::TOAST_MS;
use crate::draw::paste_lib_thumbs;
use android_activity::AndroidApp;
use log::info;
use log::warn;
use std::time::Duration;
use std::time::Instant;

impl Reader {
    /// ¿Trabajo diferido pendiente en el bucle de eventos? (poll con timeout
    /// de 16 ms → `tick`): animación del sheet, portadas de la biblioteca,
    /// long-press del dedo en el documento (modo selección) o aviso breve
    /// visible. En reposo el poll bloquea sin gastar batería.
    #[allow(dead_code)] // el bucle usa `has_window()` (timeout fijo)
    pub(crate) fn needs_tick(&mut self) -> bool {
        self.repaint
            // Buscador con teclado: mientras el IME esté abierto, `tick`
            // hace polling del texto tecleado (re-filtra la rejilla en vivo).
            || self.ime_active
            || self.tool_gesture.is_some()
            || self.sheet_anim
            || (self.chrome_visible && self.chrome_hide_at.is_some())
            || self.thumbs_pending()
            || self.toast.is_some()
            || self.gesture.press_pending()
            // Transición al abrir un libro: el tick expira el fade.
            || self.library.lib_fade.is_some()
            // Consulta de IA en vuelo: `tick` sondea el canal del hilo de
            // fondo (sin esto el poll bloquearía y la respuesta tardaría en
            // aparecer hasta el siguiente evento de input).
            || self.ai_rx.is_some()
            // Render asíncrono en vuelo (zoom sharp / cambio de página):
            // sondear hasta que el worker termine.
            || self.render_rx.is_some()
            // Debounce del pinch (F3.2): los dedos llevan quietos < 350 ms
            // o el render nítido aún no llegó — `tick` decide el disparo.
            || self.last_pinch_move.is_some()
            // Operación de Discover en curso (feed, búsqueda o descarga de paper):
            // sondear canal del DiscoverWorker sin bloquear.
            || self.discover_busy()
    }

    /// Tick del bucle de eventos (timeout ~16 ms): avanza la animación del
    /// sheet, detecta el long-press del dedo en el documento (entra en modo
    /// selección), expira el aviso breve (toast) y renderiza un lote de
    /// portadas pendientes de la biblioteca. `lib::android_main` lo invoca en
    /// los eventos Wake/Timeout, que solo ocurren mientras `needs_tick()` (sin
    /// despertar el loop en reposo).
    pub(crate) fn tick(&mut self, app: &AndroidApp) {
        // Persistencia diferida (A1): si el estado lleva >2 s sucio (cambio
        // de página reciente) se escribe aquí — el tap de página no hace I/O.
        self.flush_state_if_due();
        // Buscador con teclado: recoger lo tecleado y re-filtrar la rejilla
        // (el IME escribe en un EditText invisible; ver `jni::ime_text`).
        self.poll_ime_query(app);
        // Auto-ocultar chrome del visor tras ~2.5 s
        if self.chrome_visible
            && let Some(hide_at) = self.chrome_hide_at
            && Instant::now() >= hide_at
        {
            self.hide_chrome();
        }
        // Long-press: si el dedo lleva quieto > `LONG_PRESS_MS` en el área de
        // página (sin sheet), `input::tick_gestures` entra en modo selección.
        crate::input::tick_gestures(self, app);
        // Render ASÍNCRONO (zoom sharp / cambio de página): aplica los
        // bitmaps que ya llegaron — el UI nunca se congela esperándolos.
        let cache_inserts_before = self.cache.insert_count();
        self.poll_render();
        // Evicción DIFERIDA fuera del frame del turno (fix p95): `insert` no
        // evicta hasta `byte_budget + EVICT_SLACK` (ver `crate::cache`); el
        // recorte estricto a presupuesto ocurre AQUÍ, solo en ticks
        // REALMENTE idle — el poll no insertó nada en este tick (snapshot de
        // `insert_count`; un tick que recibe renders presenta o cachea el
        // lote del turno y no debe pagar frees) y no hay repaint pendiente
        // (un tick con repaint presenta frame; el free no debe pisar ese
        // frame). El coste del free cae fuera del camino crítico del pase de
        // página y el overshoot de `EVICT_SLACK` se recoge en el siguiente
        // tick idle (~8 ms).
        if cache_inserts_before == self.cache.insert_count() && !self.repaint {
            self.cache.trim_to_budget();
        }
        // Debounce del pinch (F3.2): 350 ms de quietud con los dedos en
        // pantalla y el bitmap a otro zoom (> 5%) → render nítido SIN
        // esperar a soltar. Con el actor persistente el lote en vuelo no
        // bloquea (preemption por seq); `render_in_flight_for` evita
        // duplicar el render al mismo nivel.
        if let Some(t) = self.last_pinch_move
            && t.elapsed() >= Duration::from_millis(350)
            && (self.rendered_zoom - self.zoom).abs() / self.zoom.max(1e-4) > 0.05
            && !self.render_in_flight_for(self.zoom)
        {
            self.launch_render(vec![self.page], self.zoom, false);
        }
        // Resultado del hilo de IA (si hay una consulta en vuelo): `try_recv`
        // sondea el canal SIN bloquear; al llegar el mensaje se actualiza el
        // panel (fase Answer/Error) y se libera el receptor. Mientras tanto el
        // poll con timeout se mantiene vivo vía `needs_tick` (ai_rx.is_some).
        if let Some(rx) = self.ai_rx.as_ref() {
            let outcome = {
                match rx.try_recv() {
                    Ok(Ok(answer)) => Some((answer, AiPhase::Answer)),
                    Ok(Err(e)) => Some((format!("Error: {e}"), AiPhase::Error)),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    // El hilo murió sin enviar (defensa): mostrar error en vez
                    // de quedarse en "preguntando…" para siempre.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                        "Error: sin respuesta del servidor".to_string(),
                        AiPhase::Error,
                    )),
                }
            };
            if let Some((text, phase)) = outcome {
                self.ai_answer(text, phase);
            }
        }
        // Aviso breve: expira a los TOAST_MS (libera también su bitmap).
        if let Some((_, at)) = &self.toast
            && at.elapsed() >= TOAST_MS
        {
            self.toast = None;
            self.toast_bitmap = None;
            self.redraw();
        }
        // Transición al abrir un libro: expirada → se libera (el visor ya
        // muestra solo la página). Durante la transición, cada tick redibuja
        // con un alfa decreciente (el fade se anima en `blit`).
        if let Some((started, _)) = self.library.lib_fade {
            if started.elapsed().as_secs_f32() >= LIB_FADE_MS {
                self.library.lib_fade = None;
                self.redraw();
            } else {
                self.redraw(); // un frame más de la transición
            }
        }
        if self.sheet_anim {
            let target = if self.sheet_open { 1.0 } else { 0.0 };
            // Ease exponencial: ~10 ticks (≈ 150 ms) para recorrer el 95 %.
            self.sheet_progress += (target - self.sheet_progress) * 0.3;
            if (target - self.sheet_progress).abs() < 0.01 {
                self.sheet_progress = target;
                self.sheet_anim = false;
            }
            if self.sheet_progress <= 0.0 {
                self.sheet_bitmap = None; // liberar el bitmap al cerrar del todo
            }
            self.redraw();
        }
        if self.mode == UiMode::Library && self.pump_thumbs(app) {
            if let Some((mut band, origin)) = self.library.lib_band.take() {
                // Pegar las portadas nuevas sobre la banda EXISTENTE (memcpy
                // por celda): sin re-render del canvas (antes un rebuild
                // completo de la pantalla por cada lote de portadas).
                paste_lib_thumbs(self, &mut band, origin);
                // Banda mutada in-place: nueva generación de su plano (la
                // textura GPU dedicada se re-subirá — Tarea 2.7).
                self.library.lib_band_ver += 1;
                self.library.lib_band = Some((band, origin));
                self.splice_band_rows();
                self.redraw();
            } else {
                // Sin banda aún: el primer rebuild la crea con las portadas.
                self.list_dirty = true;
                self.redraw();
            }
        }
        // Worker de Discover (arXiv): sondea respuestas del feed, búsquedas
        // y progreso/finalización de descargas de papers sin bloquear.
        self.pump_discover(app);
    }

    // ---------------------------------------------------------------------
    // Portadas de la biblioteca (perezosas, bajo demanda — ver `thumbs`)
    // ---------------------------------------------------------------------
    /// ¿Hay portadas pendientes entre las celdas VISIBLES de la biblioteca
    /// (carousel de "Continue Reading" + rejilla)? El bucle de eventos la
    /// usa para mantener el poll con timeout mientras `pump_thumbs` tiene
    /// trabajo.
    pub(crate) fn thumbs_pending(&mut self) -> bool {
        if self.mode != UiMode::Library || self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        // Carousel de Continue Reading (clave = ruta local), solo si está
        // visible.
        if self.lib_cont_visible() {
            // Clonar las rutas: `thumbs.get` (mutable) no convive con el
            // préstamo de `lib_continue_reading()` (inmutable).
            let paths: Vec<String> = self
                .lib_continue_reading()
                .iter()
                .map(|b| b.path.clone())
                .collect();
            for path in paths {
                if self.thumbs.get(&path).is_none() && !self.thumb_failed.contains(&path) {
                    return true;
                }
            }
        }
        // Portadas de la rejilla / lista (clave = content:// URI), solo filas visibles.
        if !self.hide_covers {
            if self.is_grid() {
                let cols = self.effective_grid_cols();
                let (row0, rows) = self.lib_visible_grid_rows();
                for row in row0..row0 + rows {
                    for col in 0..cols {
                        let Some(uri) = self.grid_entry_at(row, col).map(|e| e.uri.clone()) else {
                            continue;
                        };
                        if self.thumbs.get(&uri).is_none() && !self.thumb_failed.contains(&uri) {
                            return true;
                        }
                    }
                }
            } else {
                let (idx0, count) = self.lib_visible_list_rows();
                for idx in idx0..idx0 + count {
                    let Some(uri) = self.list_entry_at(idx).map(|e| e.uri.clone()) else {
                        continue;
                    };
                    if self.thumbs.get(&uri).is_none() && !self.thumb_failed.contains(&uri) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// ¿La fila del carousel de "Continue Reading" está dentro de la ventana
    /// (con el scroll vertical actual)? Solo entonces se renderizan sus
    /// portadas.
    fn lib_cont_visible(&self) -> bool {
        if !self.lib_has_cont() {
            return false;
        }
        let content_y0 = lib_content_y0(
            self.win_h,
            self.library.lib_search_open,
            self.status.is_some(),
        ) as f32;
        let block_h = lib_cont_block_h(self.win_w, self.win_h, true);
        let top = content_y0 - self.library.lib_scroll;
        let bottom = top + block_h;
        bottom > content_y0 && top < self.win_h as f32
    }

    /// Rango de filas de la rejilla VISIBLES (o a punto de serlo) con el
    /// scroll vertical actual: (primera fila, nº de filas con 1 de margen de
    /// prefetch por abajo). Coords compartidas con el render y el tap.
    pub(crate) fn lib_visible_grid_rows(&self) -> (usize, usize) {
        let content_y0 = lib_content_y0(
            self.win_h,
            self.library.lib_search_open,
            self.status.is_some(),
        ) as f32;
        let grid_y0_screen = content_y0 + lib_grid_y0(self.win_w, self.win_h, self.lib_has_cont())
            - self.library.lib_scroll;
        if grid_y0_screen >= self.win_h as f32 {
            return (0, 0); // la rejilla está por debajo de la ventana
        }
        let cols = self.effective_grid_cols();
        let ch = grid_cell_h(self.win_w, cols, self.cover_size);
        let row0 = ((content_y0 - grid_y0_screen) / ch).max(0.0) as usize;
        let below = ((self.win_h as f32 - grid_y0_screen) / ch).ceil().max(0.0) as usize;
        (row0, below + 1)
    }

    /// Rango de filas de la lista VISIBLES con el scroll vertical actual.
    pub(crate) fn lib_visible_list_rows(&self) -> (usize, usize) {
        let content_y0 = lib_content_y0(
            self.win_h,
            self.library.lib_search_open,
            self.status.is_some(),
        ) as f32;
        let grid_y0_screen = content_y0 + lib_grid_y0(self.win_w, self.win_h, self.lib_has_cont())
            - self.library.lib_scroll;
        if grid_y0_screen >= self.win_h as f32 {
            return (0, 0);
        }
        let rh = list_row_h(self.win_h, self.cover_size) + list_row_gap();
        let row0 = ((content_y0 - grid_y0_screen) / rh).max(0.0) as usize;
        let below = ((self.win_h as f32 - grid_y0_screen) / rh).ceil().max(0.0) as usize;
        (row0, below + 1)
    }

    fn ensure_thumb_worker(&mut self) {
        if self.thumb_worker.is_none() {
            let (worker, rx) = crate::thumbs::ThumbWorker::spawn();
            self.thumb_worker = Some(worker);
            self.thumb_rx = Some(rx);
        }
    }

    /// Recibe las portadas terminadas por el worker de fondo (`try_recv`) y
    /// encola peticiones para las celdas visibles que aún no están en caché.
    /// Cero I/O síncrono en el hilo UI (Fase E1).
    fn pump_thumbs(&mut self, _app: &AndroidApp) -> bool {
        if self.win_w <= 0 || self.win_h <= 0 {
            return false;
        }
        self.ensure_thumb_worker();

        let mut changed = false;

        // 1. Drenar portadas listas desde el canal MPSC en segundo plano
        if let Some(rx) = self.thumb_rx.as_ref() {
            while let Ok(msg) = rx.try_recv() {
                match msg.bitmap {
                    Some(bmp) => {
                        info!("thumb cached in background: {}", msg.key);
                        self.thumbs.insert(msg.key, bmp);
                        changed = true;
                    }
                    None => {
                        warn!("thumb failed in background: {}", msg.key);
                        self.thumb_failed.insert(msg.key);
                    }
                }
            }
        }

        // 2. Recolectar celdas visibles que aún no están en caché ni fallaron
        let mut needed = Vec::new();

        if self.lib_cont_visible() {
            let cont_paths: Vec<String> = self
                .lib_continue_reading()
                .iter()
                .map(|b| b.path.clone())
                .collect();
            for path in cont_paths {
                if self.thumbs.peek(&path).is_none()
                    && !self.thumb_failed.contains(&path)
                    && !needed.contains(&path)
                {
                    needed.push(path);
                }
            }
        }

        if !self.hide_covers {
            if self.is_grid() {
                let cols = self.effective_grid_cols();
                let (row0, rows) = self.lib_visible_grid_rows();
                for row in row0..row0 + rows {
                    for col in 0..cols {
                        if let Some(uri) = self.grid_entry_at(row, col).map(|e| e.uri.clone())
                            && self.thumbs.peek(&uri).is_none()
                            && !self.thumb_failed.contains(&uri)
                            && !needed.contains(&uri)
                        {
                            needed.push(uri);
                        }
                    }
                }
            } else {
                let (idx0, count) = self.lib_visible_list_rows();
                for idx in idx0..idx0 + count {
                    if let Some(uri) = self.list_entry_at(idx).map(|e| e.uri.clone())
                        && self.thumbs.peek(&uri).is_none()
                        && !self.thumb_failed.contains(&uri)
                        && !needed.contains(&uri)
                    {
                        needed.push(uri);
                    }
                }
            }
        }

        if !needed.is_empty()
            && let Some(w) = self.thumb_worker.as_ref()
        {
            w.request(needed);
        }

        changed
    }

    /// ¿Hay un blit pendiente por coalescer? (el bucle principal lo llama
    /// una vez por iteración tras procesar eventos).
    pub(crate) fn take_repaint(&mut self) -> bool {
        let r = self.repaint;
        self.repaint = false;
        r
    }

    /// ¿El bucle debe correr con timeout (~16 ms) para redibujar? (gesto en
    /// curso u otro trabajo diferido).
    #[allow(dead_code)] // apoyo: el flujo real usa `needs_tick(&mut self)`
    pub(crate) fn needs_repaint(&self) -> bool {
        self.repaint
            || self.tool_gesture.is_some()
            || self.sheet_anim
            || self.ai_rx.is_some()
            || self.library.lib_fade.is_some()
    }

    /// ¿Tenemos ventana (ANativeWindow activo)? El bucle principal usa un
    /// poll con timeout (16 ms) mientras haya ventana — el poll bloqueante
    /// puede perder los toques del visor (ver `lib.rs`).
    pub(crate) fn has_window(&self) -> bool {
        self.window.is_some()
    }
}
