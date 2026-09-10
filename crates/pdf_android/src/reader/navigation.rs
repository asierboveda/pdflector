// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Navegación y apertura de documentos (extraído de `reader.rs`, 2026-09-06): cambio de página (`goto_page`, `next_page`, `prev_page`, `jump_page`), persistencia de posición (`save_state` con flush diferido A1: `mark_state_dirty`, `flush_state`, `flush_state_if_due`) y apertura de PDFs (`open_pdf`, `open_pdf_at`).

use super::Reader;
use super::UiMode;
use crate::annotations::ToolKind;
use crate::draw::compose_library_snapshot;
use log::error;
use log::info;
use log::warn;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

impl Reader {
    /// Cambia a la página `page` (0-based) — modo UNA HOJA: `page` se fija
    /// directamente (no hay scroll que alinear: la columna de páginas se
    /// eliminó). Base compartida de `next_page`/`prev_page`/`jump_page` y del
    /// tap derecho/izquierdo. No hay salto con re-render: las páginas vecinas
    /// salen de la caché (paso instantáneo). Invalida los overlays cacheados
    /// (indicador, sheet, frame de la animación).
    ///
    /// Fase B (prefetch direccional): registra la dirección del turno
    /// (`last_direction`, signo del delta) y, cuando la página nueva NO está
    /// en caché, lanza la ventana asimétrica 2-delante/1-detrás de
    /// `prefetch_pages` (solo misses) en lugar del ±1 simétrico histórico.
    fn goto_page(&mut self, page: u32) {
        let prev = self.page;
        if prev == page {
            return;
        }
        self.page = page;
        // La visible no se expulsa (ni su propio lote ni el trim la echan).
        self.cache.set_protected(page);
        // Dirección de viaje (fase B): signo del delta. next/prev/jump y los
        // taps delegan todos aquí, así que el signo se calcula UNA vez en el
        // punto común (el delta i64 evita el overflow de u32 sin signo).
        self.last_direction = (page as i64 - prev as i64).signum() as i8;
        self.page_badge = None; // el indicador "N / total" cambia
        self.sheet_bitmap = None; // el indicador del sheet cambia
        info!("page {}", self.page + 1);
        // Instrumentación (A2): arranca el cronómetro del turno — lo cierra
        // `present_viewer` con el log `page_turn <ms>` cuando la página real
        // (no el fallback) queda horneada en la dry.
        self.page_turn_t0 = Some(Instant::now());
        // Cambio de página SIN congelar: si la nueva está en caché (prefetch
        // previo), el blit es inmediato; si no, se muestra la página ANTERIOR
        // (fallback) mientras el worker renderiza la nueva asíncronamente.
        if self.cache.peek(page).is_none() {
            // Guard (robustez del pase de página): el fallback solo sirve si
            // `prev` SIGUE en caché para mostrarse — el sliding window pudo
            // evictarla (miss en N y prev ausente → dry vacía = fondo en vez
            // de contenido). Con `prev` fuera no fijar fallback: el re-bake
            // de la página real al llegar N funciona igual.
            if self.cache.peek(prev).is_some() {
                self.fallback_page = Some(prev);
            } else {
                warn!(
                    "turn to {} without fallback (prev {} evicted)",
                    page + 1,
                    prev + 1
                );
            }
            let pages = self
                .prefetch_pages(page) // ventana direccional ordenada (fase B)
                .into_iter()
                .filter(|&p| self.cache.peek(p).is_none())
                .collect::<Vec<u32>>();
            self.launch_render(pages, self.rendered_zoom, false);
        } else {
            // Destino en caché: el fallback de otro turno ya no sirve (peek de
            // la actual gana siempre). Limpiarlo evita que un rebake futuro
            // muestre la página vieja con el badge nuevo.
            self.fallback_page = None;
        }
        // A1: la persistencia pasa a DIFERIDA (flush a los 2 s desde `tick`
        // o explícito en enter_library/open_pdf_at/Pause) — quita el I/O
        // síncrono de state.json+library.json del tap de pasar página.
        self.mark_state_dirty();
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Páginas candidatas del prefetch alrededor de `page` (fase B), en ORDEN
    /// de lanzamiento:
    /// 1. la página por DETRÁS de la dirección de viaje (radio 1),
    /// 2. la página ACTUAL (`page`),
    /// 3. hacia DELANTE en la dirección de viaje, de la más cercana a la más
    ///    lejana (radio 2).
    ///
    /// El orden no es el del ejemplo del brief (vecina delantera primero):
    /// con la página de atrás como PRIMERA llegada, el LRU de la caché la
    /// sacrifica antes que la actual cuando el lote completo (2+1+actual = 4
    /// páginas ≈ 51 MiB a 12,7 MiB/página) excede los 48 MiB del presupuesto
    /// — la inserción del 4º bitmap expulsa al más antiguo (frente LRU), que
    /// es la de atrás, y la actual sobrevive al lote (nunca pantalla en
    /// blanco tras un salto con lote completo). En el caso común (la de
    /// atrás ya cacheada — el usuario viene de ella) la actual queda PRIMERA
    /// del lote y minimiza la latencia del turno; las delanteras (siguientes
    /// taps probables) entran justo después.
    ///
    /// Sin dirección (`last_direction == 0`: apertura, restore, salto
    /// inicial) → ventana simétrica ±1 (comportamiento previo a la fase B).
    /// Devuelve páginas clampadas al documento, sin duplicados y SIN filtrar
    /// por caché (el llamador lanza solo los misses).
    fn prefetch_pages(&self, page: u32) -> Vec<u32> {
        let Some(doc) = self.doc.as_ref() else {
            return Vec::new();
        };
        let n = doc.page_count();
        if n == 0 {
            return Vec::new();
        }
        let last = n - 1;
        let mut pages = Vec::with_capacity(4);
        // Añade `p` (los guards evitan el overflow de u32 y los duplicados
        // al clampear: docs de 1 página o page == last).
        let mut push = |p: u32| pages.push(p);
        match self.last_direction {
            1 => {
                if page > 0 {
                    push(page - 1); // detrás (radio 1) — víctima LRU natural
                }
                push(page); // actual: sustituye al fallback en un turno miss
                if page < last {
                    push(page + 1); // delante, cercana → lejana (radio 2)
                }
                if page + 1 < last {
                    push(page + 2);
                }
            }
            -1 => {
                if page < last {
                    push(page + 1); // detrás (radio 1) — víctima LRU natural
                }
                push(page); // actual
                if page > 0 {
                    push(page - 1); // delante, cercana → lejana (radio 2)
                }
                if page > 1 {
                    push(page - 2);
                }
            }
            _ => {
                // Sin dirección: ventana simétrica ±1 (comportamiento previo).
                if page > 0 {
                    push(page - 1);
                }
                push(page);
                if page < last {
                    push(page + 1);
                }
            }
        }
        pages
    }

    pub(crate) fn next_page(&mut self) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1);
        if self.page < last {
            self.goto_page(self.page + 1);
        }
    }

    pub(crate) fn prev_page(&mut self) {
        if self.page > 0 {
            self.goto_page(self.page - 1);
        }
    }

    /// Salto rápido de ±N páginas (botones −10/+10 del sheet de ajustes).
    pub(crate) fn jump_page(&mut self, delta: i32) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let last = doc.page_count().saturating_sub(1) as i32;
        let target = (self.page as i32 + delta).clamp(0, last) as u32;
        if target != self.page {
            self.goto_page(target);
        }
    }

    /// Persiste la posición actual (ruta, página, zoom) + modo oscuro en
    /// `internal/state.json` (ver `persist`). Desde la fase A1 la escritura
    /// es DIFERIDA para el cambio de página: `goto_page` solo marca
    /// (`mark_state_dirty`) y `flush_state_if_due`/`flush_state` escriben
    /// aquí (ver `state_dirty`). Siguen *eager* (llamada directa) las
    /// acciones infrecuentes: soltar el pinch, toggles de ajustes y la
    /// apertura de un documento — un cierre inesperado no pierde la posición.
    ///
    /// Además actualiza el REGISTRO DE PROGRESO por libro
    /// (`internal/library.json`, ver `persist::BookProgress`): página actual,
    /// total de páginas y sello de última lectura. El registro se CREA la
    /// primera vez (added_unix) y se actualiza en cada apertura o cambio de
    /// página — de ahí se derivan "Page X of Y", la barra de progreso, el
    /// estado Reading/Finished y los sorts de "My Library" sin abrir el PDF.
    pub(crate) fn save_state(&mut self) {
        let path = self.doc_path.clone().unwrap_or_default();
        let state = crate::persist::ViewerState {
            path: path.clone(),
            page: self.page,
            zoom: self.zoom,
            dark: self.dark,
            theme: Some(self.theme),
            view_mode: self.view_mode,
            cover_fit: self.cover_fit,
            columns: self.columns,
            hide_covers: self.hide_covers,
            cover_size: self.cover_size,
            cover_progress: self.cover_progress,
        };
        crate::persist::save_state(self.internal_dir.as_deref(), &state);
        if self.mode == UiMode::Viewer && !path.is_empty() {
            let pages = self.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
            let now = crate::persist::unix_now();
            self.library.lib_books = crate::persist::touch_progress(
                &self.library.lib_books,
                &path,
                self.page,
                pages,
                now,
            );
            crate::persist::save_progress(self.internal_dir.as_deref(), &self.library.lib_books);
        }
    }

    /// Marca el estado como pendiente de persistir (A1, diferido): registra
    /// `state_dirty_since` para que `flush_state_if_due` (2 s desde `tick`)
    /// escriba sin I/O en el tap. `goto_page` es su único llamador.
    fn mark_state_dirty(&mut self) {
        self.state_dirty = true;
        self.state_dirty_since = Some(Instant::now());
    }

    /// Guarda YA si hay cambios pendientes (flush explícito). Puntos de
    /// salida del visor: `enter_library`, `open_pdf_at` (antes de sustituir
    /// el documento) y el `Pause` de la activity en `android_main`. Sin
    /// estos flushes, un kill dentro de la ventana de 2 s del diferido
    /// perdería la navegación reciente (trade-off documentado en
    /// `state_dirty`).
    pub(crate) fn flush_state(&mut self) {
        if self.state_dirty {
            self.save_state();
            self.state_dirty = false;
            self.state_dirty_since = None;
        }
    }

    /// Flush periódico del estado diferido (A1): si `goto_page` marcó dirty
    /// hace más de 2 s → `save_state` + limpiar. Llamado desde `tick` (~8 ms
    /// con ventana): el coste en reposo es una comparación de `bool` +
    /// `Instant`, sin I/O hasta que toca.
    pub(crate) fn flush_state_if_due(&mut self) {
        if self.state_dirty
            && self
                .state_dirty_since
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(2000))
        {
            self.flush_state();
        }
    }

    /// Abre un PDF por ruta (picker) y pasa al visor con la página 1.
    /// Devuelve false (y deja el estado intacto) si no se pudo abrir.
    pub(crate) fn open_pdf(&mut self, path: &str) -> bool {
        self.open_pdf_at(path, None)
    }

    /// Abre un PDF por ruta y pasa al visor; si `start_page` es Some, salta
    /// a esa página (la posición guardada de la rejilla),
    /// si no a la página 1. Devuelve false (y deja el estado intacto) si no
    /// se pudo abrir.
    pub(crate) fn open_pdf_at(&mut self, path: &str, start_page: Option<u32>) -> bool {
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                return false;
            }
        };
        match engine.open(Path::new(path)) {
            Ok(doc) => {
                // A1: flush explícito del estado diferido antes de sustituir
                // el documento — conserva en library.json la última posición
                // del PDF anterior (el `save_state` del final registra el
                // nuevo; state.json solo guarda un estado actual).
                self.flush_state();
                let pages = doc.page_count();
                info!("opened: {pages} pages");
                // Página de apertura: la guardada (reanudar lectura) o la 1.
                let page = match start_page {
                    Some(p) => p.min(pages.saturating_sub(1)),
                    None => 0,
                };
                self.doc = Some(doc);
                self.page = page;
                self.cache.set_protected(page);
                // Apertura/restore: NO es un turno de navegación — sin
                // dirección de viaje previa → ventana ±1 simétrica (fase B).
                self.last_direction = 0;
                self.zoom = 1.0;
                self.rendered_zoom = 1.0;
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                self.pinch = None;
                self.bitmap = None;
                // Transición al abrir: snapshot de la pantalla de lista
                // (biblioteca: cabecera+banda; picker: bitmap) que el visor
                // funde sobre la página los primeros `LIB_FADE_MS`.
                let snapshot = match self.mode {
                    UiMode::Library => compose_library_snapshot(self),
                    UiMode::Discover | UiMode::Viewer => None,
                    UiMode::Picker => self.bitmap.clone(),
                };
                if let Some(s) = snapshot {
                    self.library.lib_fade = Some((Instant::now(), s));
                    self.library.lib_fade_id = self.next_ovl_id(); // snapshot nuevo
                }
                self.library.lib_header = None; // biblioteca fuera: liberar planos
                self.library.lib_band = None;
                self.library.lib_row_dirty = None;
                self.cache.clear(); // otro documento: nada reutilizable
                self.fallback_page = None;
                if let Some(g) = self.gpu.as_mut() {
                    let bg = self.theme.palette().rgba_bg();
                    g.reset_document(bg);
                }
                self.mode = UiMode::Viewer;
                // EGL (Tarea 2.7, productor único): la surface ya NO se suelta
                // al entrar en Library/Picker, así que al volver al visor
                // sigue viva — este recreate es un no-op defensivo SOLO para
                // el caso degradado de surface sin crear en esta ventana
                // (fallo de eglCreateWindowSurface previo); con surface
                // presente no se toca nada (has_surface → skip).
                if let (Some(g), Some(win)) = (self.gpu.as_mut(), self.window.as_ref())
                    && !g.has_surface()
                {
                    g.recreate_surface(win);
                }
                self.status = None;
                self.doc_path = Some(path.to_string());
                self.start_render_worker(path);
                self.page_badge = None;
                self.sheet_hide_now(); // sheet del visor anterior: fuera (libera también el frame)
                self.clear_selection(); // selección del documento anterior: fuera
                self.close_ai_panel(); // panel de IA del documento anterior: fuera
                self.thumbs.clear(); // portadas de otra biblioteca: no sirven
                self.thumb_failed.clear();
                self.list_dirty = true;
                self.list_drag = None;
                // Herramientas de anotación: reseteo a la navegación limpia
                // (sin herramienta activa, sin gesto en curso
                // y SIN histórico de sesión del documento anterior — el undo
                // es por sesión, decisión documentada en `session_ids`).
                self.tool = ToolKind::Navigate;
                self.tool_gesture = None;
                self.session_ids.clear();
                // Fase B1: texto del documento nuevo (el del anterior no
                // sirve). Prefetch de la página visible +-2: el primer
                // resaltado de esas páginas será un HIT (sin stext en el
                // hilo UI). El resto se extrae perezoso con `get_or_extract`
                // (1-2 ms) y queda cacheado para repeticiones y para la IA
                // (Fase D).
                self.text_cache.clear();
                if let Some(doc) = self.doc.as_ref() {
                    let base = page.saturating_sub(2);
                    let pages: Vec<u32> = (base..(page + 3).min(pages)).collect();
                    let _n = self.text_cache.prefetch(doc, &pages);
                }
                // Anotaciones del documento (sidecar; set vacío si no existe
                // o está corrupto — nunca impide abrir el PDF).
                self.load_annotations(path);
                self.redraw();
                // Nuevo documento: actualizar la posición persistida (el
                // modo oscuro es una preferencia global y se conserva).
                self.save_state();
                // Y la lista de RECIENTES de la biblioteca (dedup por ruta,
                // más reciente primero, máx. 10 — persist::push_recent).
                self.touch_recent(path);
                true
            }
            Err(e) => {
                error!("cannot open {path}: {e}");
                false
            }
        }
    }
}
