// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Ciclo de vida del `Reader` (extraído de `reader.rs`, 2026-09-06): construcción (`Reader::new`), apertura de la ventana (`set_window`, `init_window`) y su liberación (`terminate_window`).

use super::AiPhase;
use super::LibraryCoverFit;
use super::LibraryGroupBy;
use super::LibraryViewMode;
use super::PickerKind;
use super::Reader;
use super::UiMode;
use super::library_state::LibraryState;
use super::load_pen_mode;
use super::scan_pdfs;
use crate::PINCH_MAX;
use crate::PINCH_MIN;
use crate::annotations::ToolKind;
use crate::cache::CACHE_BYTE_BUDGET;
use crate::cache::CACHE_MAX_ENTRIES;
use crate::cache::PageCache;
use crate::gpu::Gpu;
use crate::input::GestureState;
use crate::jni::android_sdk_int;
use crate::jni::launch_intent_pdf;
use crate::persist::{self};
use crate::theme;
use crate::thumbs::THUMB_BYTE_BUDGET;
use crate::thumbs::THUMB_MAX_ENTRIES;
use crate::thumbs::ThumbCache;
use android_activity::AndroidApp;
use android_activity::ndk::hardware_buffer_format::HardwareBufferFormat;
use android_activity::ndk::native_window::NativeWindow;
use log::error;
use log::info;
use log::warn;
use pdf_core::engine::mupdf::{MupdfDocument, MupdfEngine};
use pdf_core::{
    Annotation, AnnotationSet, Bitmap, Color, Document, Gesture, Highlight, PageTextCache, Rect,
    RenderEngine, Stroke, TextSpan,
};
use std::collections::HashSet;
use std::path::Path;

impl Reader {
    pub(crate) fn new(app: &AndroidApp) -> Self {
        let mut reader = Self {
            doc: None,
            page: 0,
            window: None,
            bitmap: None,
            cache: PageCache::new(CACHE_BYTE_BUDGET, CACHE_MAX_ENTRIES),
            rendered_zoom: 1.0,
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            pinch: None,
            offset_x: 0,
            offset_y: 0,
            win_w: 0,
            win_h: 0,
            gesture: GestureState::new(),
            mode: UiMode::Library,
            pdf_list: Vec::new(),
            picker_kind: PickerKind::Files,
            select_list: Vec::new(),
            sel_dir: Vec::new(),
            library_list: Vec::new(),
            permission_granted: false,
            sdk_int: android_sdk_int(),
            grant_pending: false,
            list_scroll: 0,
            library: LibraryState::new(app.internal_data_path().as_deref()),
            ime_active: false,
            view_mode: LibraryViewMode::Grid,
            cover_fit: LibraryCoverFit::Crop,
            auto_columns: true,
            columns: 3,
            view_menu_open: false,
            settings_menu_open: false,
            hide_covers: false,
            recent_shelf_enabled: true,
            cover_size: 1,
            cover_progress: false,
            clear_confirm_until: None,
            group_by: LibraryGroupBy::None,
            recents: persist::load_recents(app.internal_data_path().as_deref()),
            list_dirty: true,
            status: None,
            doc_path: None,
            internal_dir: app.internal_data_path(),
            theme: theme::AppTheme::DefaultLight,
            dark: false,
            chrome_visible: false,
            chrome_hide_at: None,
            ovl_seq: 0,
            chrome_top_bitmap: None,
            chrome_top_id: 0,
            chrome_bottom_bitmap: None,
            chrome_bottom_id: 0,
            sheet_open: false,
            sheet_progress: 0.0,
            sheet_anim: false,
            sheet_bitmap: None,
            sheet_id: 0,
            page_badge: None,
            page_badge_id: 0,
            mode_badge: None,
            mode_badge_id: 0,
            erase_pt: None,
            erase_r_px: 0.0,
            eraser_cursor: None,
            eraser_cursor_id: 0,
            thumbs: ThumbCache::new(THUMB_BYTE_BUDGET, THUMB_MAX_ENTRIES),
            thumb_failed: HashSet::new(),
            list_drag: None,
            annotations: AnnotationSet::new(),
            annot_sidecar: None,
            text_cache: PageTextCache::default(),
            status_bar_top: 0, // se fija en runtime (content_rect top)
            sel: None,
            sel_menu: None,
            sel_menu_id: 0,
            ai_panel: None,
            ai_panel_id: 0,
            ai_text: String::new(),
            ai_phase: AiPhase::Asking,
            ai_rx: None,
            toast: None,
            toast_bitmap: None,
            toast_id: 0,
            tool: ToolKind::Navigate,
            erase_dirty: false,
            erase_last: None,
            ink_color: {
                let ts = persist::load_tool_state(app.internal_data_path().as_deref());
                ts.ink_color
            },
            ink_width: {
                let ts = persist::load_tool_state(app.internal_data_path().as_deref());
                ts.ink_width
            },
            pen_mode: load_pen_mode(app.internal_data_path().as_deref()),
            tool_gesture: None,
            session_ids: Vec::new(),
            repaint: false,
            take_repaint_probe: None,
            gpu: None,
            pending_t0_ns: None,
            pending_pressure: None,
            gesture_t0_ns: 0,
            last_stylus_time: None,
            render_rx: None,
            render_seq: 0,
            render_worker: None,
            last_pinch_move: None,
            fallback_page: None,
            thumb_worker: None,
            thumb_rx: None,
        };
        match launch_intent_pdf(app) {
            // "Abrir con" (ACTION_VIEW): el PDF se abre directamente, sin pasar
            // por la biblioteca. Si falla, se cae al picker interno con el
            // motivo como estado (comportamiento previo al spike de biblioteca).
            Some(lp) => {
                info!("open-with intent: {} ({})", lp.name, lp.source);
                let engine = match MupdfEngine::new() {
                    Ok(e) => e,
                    Err(e) => {
                        // Prácticamente infalible (solo falla ante fallo catastrófico
                        // del allocator); si ocurriera seguimos sin motor.
                        error!("MupdfEngine::new: {e}");
                        MupdfEngine
                    }
                };
                match engine.open(Path::new(&lp.path)) {
                    Ok(doc) => {
                        info!("opened: {} pages", doc.page_count());
                        reader.doc = Some(doc);
                        reader.doc_path = Some(lp.path.clone());
                        reader.mode = UiMode::Viewer;
                        // Anotaciones del documento (sidecar SQLite; set vacío
                        // si no existe o está corrupto).
                        reader.load_annotations(&lp.path);
                        // Registrar también el "abrir con": el próximo arranque
                        // sin intent restaurará este PDF en su última posición.
                        reader.save_state();
                        // Y añadirlo a los RECIENTES de la biblioteca (el
                        // "abrir con" no pasa por `open_pdf`).
                        reader.touch_recent(&lp.path);
                        reader.start_render_worker(&lp.path);
                    }
                    Err(e) => {
                        error!("cannot open {}: {e}", lp.path);
                        reader.mode = UiMode::Picker;
                        reader.status = Some(format!("Cannot open {}", lp.name));
                        reader.pdf_list = scan_pdfs(app);
                    }
                }
            }
            // Lanzamiento normal sin intent. Estado persistido (`persist`): si
            // el PDF guardado sigue accesible, se abre directamente en su
            // página/zoom/tema; si ya no existe (o no se puede abrir),
            // se BORRA el estado y se muestra la BIBLIOTECA CURADA
            // (`internal/library.json`) — SIN escanear MediaStore.
            None => {
                let restored =
                    if let Some(state) = persist::load_state(reader.internal_dir.as_deref()) {
                        if let Some(th) = state.theme {
                            reader.theme = th;
                            reader.dark = th.is_dark();
                        } else {
                            reader.dark = state.dark;
                            reader.theme = if state.dark {
                                theme::AppTheme::DefaultDark
                            } else {
                                theme::AppTheme::DefaultLight
                            };
                        }
                        // Preferencias de la BIBLIOTECA (menús ⋯/☰, Tarea 1:
                        // esqueleto): se restauran aunque el PDF guardado ya
                        // no exista (caen a la biblioteca con su layout).
                        reader.view_mode = state.view_mode;
                        reader.cover_fit = state.cover_fit;
                        reader.columns = state.columns;
                        reader.hide_covers = state.hide_covers;
                        reader.recent_shelf_enabled = state.recent_shelf_enabled;
                        reader.cover_size = state.cover_size;
                        reader.cover_progress = state.cover_progress;
                        // Solo restaurar si el PDF sigue accesible: `open_pdf`
                        // falla si no se puede abrir (corrupto) y deja el
                        // estado intacto.
                        if Path::new(&state.path).exists() && reader.open_pdf(&state.path) {
                            let pages = reader.doc.as_ref().map(|d| d.page_count()).unwrap_or(0);
                            reader.page = state.page.min(pages.saturating_sub(1));
                            reader.zoom = state.zoom.clamp(PINCH_MIN, PINCH_MAX);
                            reader.rendered_zoom = reader.zoom;
                            // Modo UNA HOJA: la página restaurada se fija
                            // directamente (no hay scroll que alinear).
                            reader.cache.clear();
                            reader.page_badge = None; // indicador de la página restaurada
                            info!(
                                "restored {} @page {} zoom {:.3} theme {:?}",
                                state.path,
                                reader.page + 1,
                                reader.zoom,
                                reader.theme
                            );
                            reader.save_state();
                            reader.redraw();
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                if !restored {
                    // Sin estado (primer arranque) o PDF ya no accesible:
                    // limpiar el estado huérfano y mostrar la BIBLIOTECA
                    // CURADA. Sin intent NO hay escaneo de MediaStore: la
                    // rejilla sale de `library.json` (con migración one-shot
                    // de los PDFs que instalaciones antiguas dejaran en
                    // `internal/pdfs/`); vacía → empty state con "Añadir PDF".
                    persist::clear_state(reader.internal_dir.as_deref());
                    reader.reload_curated_library(app);
                }
            }
        }
        reader
    }

    /// Sustituye el handle de ventana por el actual y re-fuerza el formato del
    /// buffer. `app.native_window()` devuelve siempre el window vigente de la
    /// glue de NativeActivity; tras una recreación de la surface es un
    /// `ANativeWindow` NUEVO que necesita `set_buffers_geometry` otra vez.
    pub(crate) fn set_window(&mut self, window: NativeWindow) {
        if let Err(e) =
            window.set_buffers_geometry(0, 0, Some(HardwareBufferFormat::R8G8B8A8_UNORM))
        {
            warn!("set_buffers_geometry(R8G8B8A8_UNORM): {e}");
        }
        // El pipeline del visor es GPU (EGL): el contexto se crea UNA vez con
        // la primera ventana de Viewer y sobrevive a surfaces nuevas
        // (recreate_surface). En modos SW (Library/Picker) no se toca.
        if self.mode == UiMode::Viewer {
            match self.gpu.as_mut() {
                Some(g) => {
                    g.recreate_surface(&window);
                }
                None => {
                    // SAFETY: EGL/GLES sobre una NativeWindow válida de
                    // android_activity; fallo → Viewer cae al camino SW.
                    let gpu = unsafe { Gpu::new(&window) };
                    if gpu.is_none() {
                        warn!("gpu: EGL init failed — Viewer en SW");
                    }
                    self.gpu = gpu;
                }
            }
        }
        self.window = Some(window);
    }

    /// `InitWindow`: nueva ventana lista. Fuerza buffers RGBA8888 (0,0 =
    /// conservar tamaño base; solo cambia el formato) e invalida la caché.
    pub(crate) fn init_window(&mut self, window: NativeWindow) {
        self.set_window(window);
        self.bitmap = None;
        self.library.lib_header = None;
        self.library.lib_band = None;
        self.page_badge = None;
        self.mode_badge = None;
        self.sheet_bitmap = None;
        self.list_dirty = true;
        // Nueva ventana → posible nueva escala cover: las páginas de la caché
        // se reutilizan si el tamaño no cambió; el redraw detecta el cambio de
        // `win_w/h` y limpia la caché si hace falta.
        self.redraw();
    }

    /// `TerminateWindow`: soltar la ventana (drop → `ANativeWindow_release`).
    pub(crate) fn terminate_window(&mut self) {
        if let Some(g) = self.gpu.as_mut() {
            g.drop_surface();
        }
        self.window = None;
        self.bitmap = None;
        self.page_badge = None;
        self.mode_badge = None;
        self.sheet_bitmap = None;
        self.list_dirty = true;
    }
}
