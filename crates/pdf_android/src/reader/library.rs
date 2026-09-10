// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Biblioteca, picker y selector de añadir (extraído de `reader.rs`, 2026-09-06): entrada/salida (`enter_library`, `open_picker`, `exit_picker`, `reload_curated_library`, `clear_library`, `rescan`), listas y filtros (`picker_*`, `grid_*`, `entry_*`, `refresh_lib_filtered`, `apply_filter`, `lib_set_*`, `lib_folders`, scrolls `lib_*_max_*`, `touch_recent`) y el flujo "＋ Añadir" con IME (`add_book`, `rescan_select`, `cancel_add`, `add_selected`, `lib_open_keyboard`/`lib_clear_search`/`lib_close_ime`, `poll_ime_query`).

use super::BookStatus;
use super::LibSort;
use super::LibraryEntry;
use super::LibraryGroupBy;
use super::LibraryViewMode;
use super::PickRow;
use super::PickerKind;
use super::Reader;
use super::UiMode;
use super::book_status;
use super::entry_author;
use super::geometry::grid_cell_h;
use super::geometry::grid_gap;
use super::geometry::lib_chips_row_w;
use super::geometry::lib_content_y0;
use super::geometry::lib_grid_y0;
use super::geometry::lib_org_row_w;
use super::geometry::list_row_gap;
use super::geometry::list_row_h;
use super::geometry::picker_visible_rows;
use super::scan_pdfs;
use crate::annotations::ToolKind;
use crate::jni::query_media_store;
use crate::jni::read_content_uri_bytes;
use crate::jni::sanitize_pdf_name;
use crate::persist::BookProgress;
use crate::persist::{self};
use android_activity::AndroidApp;
use log::error;
use log::info;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

impl Reader {
    /// Entra en la biblioteca (botón "← Library" del sheet del visor):
    /// reconstruye la biblioteca CURADA desde `internal/library.json` y deja
    /// de mostrar la página. El campo de búsqueda arranca CERRADO. Vacía →
    /// EMPTY STATE ("Tu biblioteca está vacía" + botón "Añadir PDF").
    pub(crate) fn enter_library(&mut self, app: &AndroidApp) {
        // A1: flush explícito del estado diferido ANTES de cambiar de modo —
        // `save_state` registra el progreso del libro solo en modo Viewer y
        // la biblioteca recarga `library.json` justo debajo: sin este flush
        // el progreso de una lectura reciente (<2 s) no se reflejaría.
        self.flush_state();
        self.mode = UiMode::Library;
        // EGL (Tarea 2.7, productor único): la surface del visor NO se
        // suelta al entrar en la biblioteca — la biblioteca presenta por el
        // MISMO EGL (planos cacheados como texturas + swap). Soltar aquí la
        // surface era la causa raíz del EGL_BAD_ALLOC 0x3003 en cada vuelta
        // Library→Viewer (la ventana no admite alternar productor CPU/GPU).
        self.list_scroll = 0;
        self.library.lib_search_open = false;
        self.list_dirty = true;
        self.bitmap = None; // lista del picker (no se usa en la biblioteca)
        self.library.lib_header = None; // zona fija: se re-renderiza en el rebuild
        self.library.lib_band = None; // banda de contenido: idem
        self.library.lib_row_dirty = None;
        // La caché de páginas del visor (48 MiB) no sirve en la biblioteca:
        // liberarla aquí evita RSS doble (páginas + zona fija + banda +
        // portadas) y se re-renderiza al volver a un PDF.
        self.cache.clear();
        // Re-cargar los registros persistidos (recents + progreso): la
        // biblioteca debe reflejar cualquier lectura hecha en otra sesión o
        // proceso (barras de progreso / sort-filtros).
        self.recents = persist::load_recents(self.internal_dir.as_deref());
        self.library.lib_books = persist::load_progress(self.internal_dir.as_deref());
        self.sheet_hide_now(); // fuera del visor: el sheet no pinta en biblioteca
        self.clear_selection(); // selección del visor: fuera (no pinta en biblioteca)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        // Herramientas del visor: fuera (no pinta en biblioteca).
        self.tool = ToolKind::Navigate;
        self.tool_gesture = None;
        self.session_ids.clear();
        self.lib_close_ime(app);
        self.reload_curated_library(app);
    }

    /// Abre el picker interno (PDFs de los directorios de la app; el fallback
    /// histórico). Con la biblioteca curada no hay ruta de UI hacia él (las
    /// altas van por `add_book`); se conserva el método por si una fase
    /// futura reintroduce la entrada.
    #[allow(dead_code)]
    pub(crate) fn open_picker(&mut self, app: &AndroidApp) {
        self.mode = UiMode::Picker;
        // EGL (Tarea 2.7): igual que en `enter_library` — el picker también
        // presenta por EGL (sin soltar la surface).
        self.pdf_list = scan_pdfs(app);
        self.list_scroll = 0;
        self.status = None;
        self.list_dirty = true;
        self.bitmap = None;
        self.sheet_hide_now();
        self.clear_selection(); // selección del visor: fuera (no pinta en el picker)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        self.redraw();
    }

    /// Vuelve del picker al visor sin cambiar el documento (botón Back).
    pub(crate) fn exit_picker(&mut self) {
        self.mode = UiMode::Viewer;
        self.list_dirty = true;
        self.bitmap = None; // lista del picker (las páginas siguen en la caché)
        self.library.lib_header = None; // biblioteca fuera: liberar planos cedeados
        self.library.lib_band = None;
        self.library.lib_row_dirty = None;
        self.list_drag = None;
        self.redraw();
    }

    /// Reconstruye la BIBLIOTECA CURADA desde `internal/library.json` — SIN
    /// consultar MediaStore: una entrada `LibraryEntry` por registro cuyo PDF
    /// sigue existiendo en disco (`uri` = RUTA LOCAL, `folder` = "PDF"; las
    /// portadas y aperturas van por ruta). Antes de listar ejecuta la
    /// MIGRACIÓN one-shot de instalaciones antiguas (`migrate_internal_pdfs`).
    /// Vacía → empty state con "Añadir PDF".
    /// Refresca los datos en memoria de la biblioteca curada sin cambiar de modo
    /// ni resetear scrolls ni invalidar la superficie activa si estamos en otro modo.
    pub(crate) fn refresh_curated_library_data(&mut self) {
        self.library.lib_books = persist::load_progress(self.internal_dir.as_deref());
        self.migrate_internal_pdfs();
        let mut entries = Vec::new();
        for b in &self.library.lib_books {
            let p = Path::new(&b.path);
            if !p.is_file() {
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            entries.push(LibraryEntry {
                name,
                folder: "PDF".to_string(),
                uri: b.path.clone(),
                size: 0,
            });
        }
        info!(
            "curated library: {} of {} records with file on disk",
            entries.len(),
            self.library.lib_books.len()
        );
        self.library_list = entries;
        self.permission_granted = true;
        self.refresh_lib_filtered();
        self.list_dirty = true;
        self.library.lib_header = None;
        self.library.lib_band = None;
        self.library.lib_row_dirty = None;
    }

    pub(crate) fn reload_curated_library(&mut self, _app: &AndroidApp) {
        self.mode = UiMode::Library;
        self.picker_kind = PickerKind::Files; // el selector temporal queda fuera
        self.refresh_curated_library_data();
        // Datos nuevos: scroll al origen (vertical y horizontales) y lista
        // filtrada recalculada; el sort activo ordena por added/read.
        self.list_scroll = 0;
        self.library.lib_scroll = 0.0;
        self.library.lib_folders_x = 0.0;
        self.library.lib_letters_x = 0.0;
        self.library.lib_sort_x = 0.0;
        self.library.lib_filter_x = 0.0;
        self.bitmap = None;
        self.redraw();
    }

    /// Vacía la biblioteca curada (elimina library.json y los PDFs internos).
    pub(crate) fn clear_library(&mut self, app: &AndroidApp) {
        if let Some(dir) = self.internal_dir.as_deref() {
            let lib_file = crate::persist::library_path(dir);
            if lib_file.exists()
                && let Err(e) = fs::remove_file(&lib_file)
            {
                log::warn!("failed to remove library.json: {e}");
            }
            let pdf_dir = dir.join("pdfs");
            if let Ok(entries) = fs::read_dir(&pdf_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        }
        self.library.lib_books.clear();
        self.library_list.clear();
        self.library.lib_filtered.clear();
        self.reload_curated_library(app);
        self.settings_menu_open = false;
        self.show_toast("Library cleared");
        self.list_dirty = true;
        self.redraw();
    }

    /// MIGRACIÓN one-shot de instalaciones antiguas: versiones previas
    /// copiaban los PDFs abiertos a `internal/pdfs/` sin registrarlos en
    /// `library.json`. Si el registro está VACÍO y la carpeta NO, se importa
    /// cada PDF como libro (added = ahora). Idempotente: tras guardar, el
    /// registro deja de estar vacío y no vuelve a ejecutarse.
    fn migrate_internal_pdfs(&mut self) {
        if !self.library.lib_books.is_empty() {
            return;
        }
        let Some(dir) = self.internal_dir.as_deref() else {
            return;
        };
        let pdfs_dir = dir.join("pdfs");
        let Ok(rd) = fs::read_dir(&pdfs_dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")))
            .collect();
        if paths.is_empty() {
            return;
        }
        paths.sort();
        let now = persist::unix_now();
        let books: Vec<BookProgress> = paths
            .iter()
            .map(|p| BookProgress {
                path: p.display().to_string(),
                page: 0,
                // Contar páginas exigiría abrir CADA PDF durante el arranque
                // (launch time); el total real queda sellado en la primera
                // apertura (`save_state` → `touch_progress`).
                page_count: 0,
                last_read_unix: now,
                added_unix: now,
            })
            .collect();
        info!(
            "migration: imported {} PDFs from {} into library.json",
            books.len(),
            pdfs_dir.display()
        );
        self.library.lib_books = books;
        persist::save_progress(self.internal_dir.as_deref(), &self.library.lib_books);
    }

    /// Nº de filas de la lista del picker según su variante (el fallback lee
    /// `pdf_list`; el selector de añadir, la TEMPORAL `select_list`). Lo
    /// consumen el clamp de scroll y el tap.
    pub(crate) fn picker_len(&self) -> usize {
        match self.picker_kind {
            PickerKind::Files => self.pdf_list.len(),
            // El selector de añadir navega por CARPETAS: las filas visibles
            // son la vista actual del gestor (carpetas + PDFs), no la lista
            // plana de MediaStore.
            PickerKind::Select => self.picker_rows().len(),
        }
    }

    /// Construye la vista ACTUAL del gestor de archivos del selector de
    /// añadir: carpetas primero (únicas, del nivel actual) y luego los PDFs
    /// directamente contenidos en `sel_dir`. Orden alfabético
    /// (case-insensitive).
    pub(crate) fn picker_rows(&self) -> Vec<PickRow> {
        if self.picker_kind != PickerKind::Select {
            return Vec::new();
        }
        let cur = self.sel_dir.join("/");
        let mut folders: Vec<String> = Vec::new();
        let mut files: Vec<usize> = Vec::new();
        for (idx, e) in self.select_list.iter().enumerate() {
            let f = e.folder.trim_end_matches('/');
            if self.sel_dir.is_empty() {
                // Raíz: carpeta = primer segmento; PDF = sin carpeta.
                if f.is_empty() {
                    files.push(idx);
                } else if let Some(seg) = f.split('/').next()
                    && !seg.is_empty()
                    && !folders.iter().any(|x| x == seg)
                {
                    folders.push(seg.to_string());
                }
                continue;
            }
            // Nivel: PDF directo (== cur) o subcarpeta (siguiente segmento).
            if f == cur {
                files.push(idx);
            } else if let Some(rest) = f.strip_prefix(&format!("{cur}/"))
                && let Some(seg) = rest.split('/').next()
                && !seg.is_empty()
                && !folders.iter().any(|x| x == seg)
            {
                folders.push(seg.to_string());
            }
        }
        folders.sort_by_key(|f| f.to_lowercase());
        files.sort_by_key(|&i| self.select_list[i].name.to_lowercase());
        let mut rows = Vec::with_capacity(folders.len() + files.len());
        rows.extend(folders.into_iter().map(PickRow::Folder));
        rows.extend(files.into_iter().map(PickRow::File));
        rows
    }

    /// Entra en la carpeta `name` del gestor (push al breadcrumb).
    pub(crate) fn picker_sel_enter(&mut self, name: &str) {
        self.sel_dir.push(name.to_string());
        self.list_scroll = 0;
        self.list_dirty = true;
        self.redraw();
    }

    /// Sube un nivel del gestor de archivos; en la raíz no hace nada.
    pub(crate) fn picker_sel_up(&mut self) {
        if self.sel_dir.pop().is_some() {
            self.list_scroll = 0;
            self.list_dirty = true;
            self.redraw();
        }
    }

    /// ¿El selector de añadir muestra la barra de breadcrumb (dentro de una
    /// carpeta)? Añade una fila fija entre la cabecera y la lista.
    pub(crate) fn picker_has_crumb(&self) -> bool {
        self.picker_kind == PickerKind::Select && !self.sel_dir.is_empty()
    }

    /// Nº real de filas visibles del picker (resta la barra de breadcrumb
    /// del selector de añadir cuando está visible).
    pub(crate) fn picker_visible(&self) -> usize {
        let crumbs = if self.picker_has_crumb() { 1 } else { 0 };
        picker_visible_rows(self.win_h, self.status.is_some()).saturating_sub(crumbs)
    }

    /// Nº de columnas efectivas de la rejilla (Auto -> 3, Manual -> columns clamp 1..4).
    pub(crate) fn effective_grid_cols(&self) -> usize {
        if self.auto_columns {
            3
        } else {
            self.columns.clamp(1, 4) as usize
        }
    }

    /// Nº de filas de celdas de la rejilla de la biblioteca con
    /// el filtro actual aplicado (`lib_filtered`).
    #[allow(dead_code)]
    pub(crate) fn grid_total_rows(&self) -> usize {
        let cols = self.effective_grid_cols();
        self.library.lib_filtered.len().div_ceil(cols)
    }

    /// Entrada de la rejilla en la fila `row` (0-based) y columna `col`
    /// (0..cols) — resolución sobre la lista FILTRADA (`lib_filtered`).
    /// None si la celda está fuera de rango.
    pub(crate) fn grid_entry_at(&self, row: usize, col: usize) -> Option<&LibraryEntry> {
        let cols = self.effective_grid_cols();
        let idx = row.checked_mul(cols)?.checked_add(col)?;
        self.library
            .lib_filtered
            .get(idx)
            .and_then(|&i| self.library_list.get(i))
    }

    /// Entrada de la lista en el índice `idx` de la lista FILTRADA.
    pub(crate) fn list_entry_at(&self, idx: usize) -> Option<&LibraryEntry> {
        self.library
            .lib_filtered
            .get(idx)
            .and_then(|&i| self.library_list.get(i))
    }

    /// Ruta local del PDF de la biblioteca (la copia en `internal/pdfs/`): la
    /// clave del registro de progreso (`library.json`), de `recents.json` y
    /// de `state.json`. Debe coincidir con la que usa `open_library_entry`.
    pub(crate) fn entry_path(&self, e: &LibraryEntry) -> String {
        let dir = self.internal_dir.as_deref().unwrap_or(Path::new(""));
        dir.join("pdfs")
            .join(sanitize_pdf_name(&e.name))
            .display()
            .to_string()
    }

    /// Clave de orden "recientemente añadido" de la entrada `i` de
    /// `library_list` (added_unix; sin registro → i64::MIN, al final).
    fn sort_added_key(&self, i: usize) -> i64 {
        let e = &self.library_list[i];
        persist::progress_for(&self.library.lib_books, &self.entry_path(e))
            .map(|p| p.added_unix)
            .unwrap_or(i64::MIN)
    }

    /// Clave de orden "recientemente leído" (last_read_unix; sin registro →
    /// i64::MIN, al final).
    fn sort_read_key(&self, i: usize) -> i64 {
        let e = &self.library_list[i];
        persist::progress_for(&self.library.lib_books, &self.entry_path(e))
            .map(|p| p.last_read_unix)
            .unwrap_or(i64::MIN)
    }

    // ---------------------------------------------------------------------
    // Biblioteca rediseñada: filtros SIN teclado + recientes (2026-08-XX)
    // ---------------------------------------------------------------------
    //
    // Búsqueda: el enunciado pedía un campo de texto con el teclado del
    // sistema vía JNI (InputMethodManager + InputConnection). VERIFICADO en el
    // código de android-activity 0.6.1 (el backend `native-activity` de este
    // proyecto): `NativeActivity::set_text_input_state` es un NOP
    // ("Unsupported") y `InputEvent::TextEvent` SOLO lo produce el backend
    // game-activity (GameTextInput, que exige una Activity Java compilada —
    // y cargo-apk/ndk-build no compilan fuentes Java, ver cabecera de lib.rs).
    // Sin un `onCreateInputConnection` que entregue `commitText`, el teclado
    // blando NO puede mandar texto a una NativeActivity. Por eso el filtro es
    // SIN teclado: letra inicial (A-Z / #) + carpeta, vía los chips de
    // `lib_chips` (ver el worker_done de la sesión).
    /// Reconstruye la caché `lib_filtered` (índices de `library_list`):
    /// filtra por BÚSQUEDA (carpeta + letra) y por ESTADO
    /// (Reading/Finished/Unread), y ORDENA por el sort activo (`lib_sort`).
    /// Se llama al cambiar filtro, sort o al re-consultar MediaStore; sin
    /// filtros equivale a todas las entradas en orden de MediaStore (por
    /// carpeta, luego nombre).
    fn refresh_lib_filtered(&mut self) {
        let mut idxs: Vec<usize> = self
            .library_list
            .iter()
            .enumerate()
            .filter(|(_, e)| self.library.entry_passes(e))
            .map(|(i, _)| i)
            .collect();
        // Filtro de ESTADO (derivado del registro de progreso).
        if let Some(s) = self.library.lib_status {
            idxs.retain(|&i| {
                let e = &self.library_list[i];
                book_status(persist::progress_for(
                    &self.library.lib_books,
                    &self.entry_path(e),
                )) == s
            });
        }
        match self.library.lib_sort {
            LibSort::Title => idxs.sort_by(|&a, &b| {
                self.library_list[a]
                    .name
                    .to_lowercase()
                    .cmp(&self.library_list[b].name.to_lowercase())
            }),
            LibSort::Author => idxs.sort_by(|&a, &b| {
                entry_author(&self.library_list[a])
                    .to_lowercase()
                    .cmp(&entry_author(&self.library_list[b]).to_lowercase())
                    .then_with(|| {
                        self.library_list[a]
                            .name
                            .to_lowercase()
                            .cmp(&self.library_list[b].name.to_lowercase())
                    })
            }),
            LibSort::Progress => {
                let mut keyed: Vec<(usize, f32)> = idxs
                    .iter()
                    .map(|&i| {
                        let e = &self.library_list[i];
                        let pct =
                            persist::progress_for(&self.library.lib_books, &self.entry_path(e))
                                .map(|p| p.pct())
                                .unwrap_or(0.0);
                        (i, pct)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| {
                            self.library_list[a.0]
                                .name
                                .to_lowercase()
                                .cmp(&self.library_list[b.0].name.to_lowercase())
                        })
                });
                idxs = keyed.into_iter().map(|(i, _)| i).collect();
            }
            LibSort::RecentlyAdded | LibSort::RecentlyRead => {
                // Precomputar la clave (added/read) una vez por entrada: evita
                // reconstruir la ruta local en cada comparación del sort.
                let mut keyed: Vec<(usize, i64)> = idxs
                    .iter()
                    .map(|&i| {
                        let k = if self.library.lib_sort == LibSort::RecentlyAdded {
                            self.sort_added_key(i)
                        } else {
                            self.sort_read_key(i)
                        };
                        (i, k)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    b.1.cmp(&a.1).then_with(|| {
                        self.library_list[a.0]
                            .name
                            .to_lowercase()
                            .cmp(&self.library_list[b.0].name.to_lowercase())
                    })
                });
                idxs = keyed.into_iter().map(|(i, _)| i).collect();
            }
        }
        if self.group_by == LibraryGroupBy::Author && self.library.lib_sort != LibSort::Author {
            idxs.sort_by(|&a, &b| {
                entry_author(&self.library_list[a])
                    .to_lowercase()
                    .cmp(&entry_author(&self.library_list[b]).to_lowercase())
            });
        }
        self.library.lib_filtered = idxs;
    }

    /// Aplica un cambio de filtro/sort: recalcula la lista, clampa el scroll
    /// y re-renderiza.
    pub(crate) fn apply_filter(&mut self) {
        self.refresh_lib_filtered();
        let max_v = self.lib_max_scroll();
        if self.library.lib_scroll > max_v {
            self.library.lib_scroll = max_v;
        }
        self.list_dirty = true;
        self.redraw();
    }

    /// Fija el filtro de letra inicial del panel de búsqueda (None = todas;
    /// chip "All"). Al elegir, el panel se cierra: el campo de búsqueda
    /// muestra el resumen del filtro activo (ver `draw::search_summary`).
    pub(crate) fn lib_set_letter(&mut self, letter: Option<char>) {
        if self.library.lib_letter != letter {
            self.library.lib_letter = letter;
            self.library.lib_search_open = false;
            self.apply_filter();
        }
    }

    /// Fija el filtro de carpeta del panel de búsqueda (None = todas; chip
    /// "All"). Al elegir, el panel se cierra (el campo muestra el resumen).
    pub(crate) fn lib_set_folder(&mut self, folder: Option<String>) {
        if self.library.lib_folder != folder {
            self.library.lib_folder = folder;
            self.library.lib_search_open = false;
            self.apply_filter();
        }
    }

    /// Fija el ORDEN de "My Library" (chips de sort: Recently Added /
    /// Recently Read / Title / Author).
    pub(crate) fn lib_set_sort(&mut self, sort: LibSort) {
        if self.library.lib_sort != sort {
            self.library.lib_sort = sort;
            self.apply_filter();
        }
    }

    /// Fija el filtro de ESTADO de "My Library" (None = All; chips de
    /// filter: Reading / Finished / Unread).
    pub(crate) fn lib_set_status(&mut self, status: Option<BookStatus>) {
        if self.library.lib_status != status {
            self.library.lib_status = status;
            self.apply_filter();
        }
    }

    /// Carpetas distintas de la biblioteca (orden de MediaStore, dedup
    /// case-insensitive) para la fila de chips de carpetas del panel de
    /// búsqueda.
    pub(crate) fn lib_folders(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.library_list {
            if e.folder.is_empty() {
                continue;
            }
            if !out.iter().any(|f| f.eq_ignore_ascii_case(&e.folder)) {
                out.push(e.folder.clone());
            }
        }
        out
    }

    /// ¿La biblioteca se muestra en REJILLA (vs lista)?
    pub(crate) fn is_grid(&self) -> bool {
        self.view_mode == LibraryViewMode::Grid
    }

    /// Alto total (px) del contenido scrolleable de la biblioteca.
    pub(crate) fn lib_content_h(&self) -> f32 {
        let win_w = self.win_w;
        let win_h = self.win_h;
        let grid_y0 = lib_grid_y0(win_w, win_h);
        let count = self.library.lib_filtered.len();
        if self.is_grid() {
            let cols = self.effective_grid_cols();
            let rows = count.div_ceil(cols);
            let gap = grid_gap(win_w);
            grid_y0 + rows as f32 * (grid_cell_h(win_w, cols, self.cover_size) + gap) + 40.0
        } else {
            let gap = list_row_gap();
            grid_y0 + count as f32 * (list_row_h(win_h, self.cover_size) + gap) + 40.0
        }
    }

    /// Scroll vertical máximo (px) del contenido de la biblioteca.
    pub(crate) fn lib_max_scroll(&self) -> f32 {
        let viewport = (self.win_h
            - lib_content_y0(
                self.win_h,
                self.library.lib_search_open,
                self.status.is_some(),
            )) as f32;
        (self.lib_content_h() - viewport).max(0.0)
    }

    /// Scroll horizontal máximo (px) de la fila de chips `row` del panel de
    /// búsqueda (0 = letras, 1 = carpetas).
    pub(crate) fn lib_chips_max_x(&self, row: usize) -> f32 {
        (lib_chips_row_w(self, row) - self.win_w as f32).max(0.0)
    }

    /// Scroll horizontal máximo (px) de la fila de organización `row`
    /// (0 = sort, 1 = filter).
    pub(crate) fn lib_org_max_x(&self, row: usize) -> f32 {
        (lib_org_row_w(self, row) - self.win_w as f32).max(0.0)
    }

    /// Registra un PDF abierto en la lista de recientes (persistida en
    /// `internal/recents.json`; ver `persist`): dedup por ruta, más reciente
    /// primero, máx. `RECENTS_MAX`. Se llama desde `open_pdf` y desde el
    /// "abrir con" del arranque (que no pasa por `open_pdf`).
    pub(crate) fn touch_recent(&mut self, path: &str) {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.recents = persist::push_recent(&self.recents, path.to_string(), name);
        persist::save_recents(self.internal_dir.as_deref(), &self.recents);
    }

    /// Entra en la biblioteca con el campo de búsqueda ABIERTO y los
    /// filtros limpios: es el botón "Search" del sheet — sin teclado, la
    /// búsqueda ES el panel de chips (letra/carpeta) del campo de búsqueda,
    /// así que "buscar" equivale a saltar a la biblioteca con el panel
    /// desplegado.
    pub(crate) fn enter_library_search(&mut self, app: &AndroidApp) {
        self.library.lib_letter = None;
        self.library.lib_folder = None;
        self.enter_library(app);
        self.library.lib_search_open = true;
        self.list_dirty = true;
        self.redraw();
    }

    /// "＋ Añadir" (cabecera) / "Añadir PDF" (empty state): consulta
    /// MediaStore en una LISTA TEMPORAL (`select_list`; NUNCA
    /// `library_list`) y abre el selector (`UiMode::Picker` +
    /// `PickerKind::Select`, título "Selecciona PDF") con TODOS los PDFs del
    /// sistema para elegir cuál curar. La biblioteca no cambia hasta que el
    /// usuario toca un PDF (`add_selected`).
    pub(crate) fn add_book(&mut self, app: &AndroidApp) {
        self.lib_close_ime(app);
        let scan = query_media_store(app, self.sdk_int);
        self.permission_granted = scan.permission_granted;
        self.select_list = scan.entries;
        self.sel_dir = Vec::new();
        self.picker_kind = PickerKind::Select;
        self.mode = UiMode::Picker;
        self.list_scroll = 0;
        self.status = if !self.permission_granted {
            Some("All files access not granted — grant it in system Settings".to_string())
        } else if let Some(e) = scan.error {
            Some(format!("MediaStore error: {e}"))
        } else if self.select_list.is_empty() {
            Some("No PDFs found on the device".to_string())
        } else {
            None
        };
        info!("add picker: {} PDFs in MediaStore", self.select_list.len());
        self.sheet_hide_now();
        self.clear_selection(); // selección del visor: fuera (no pinta aquí)
        self.close_ai_panel(); // panel de IA del visor: fuera
        self.list_drag = None;
        self.bitmap = None;
        self.library.lib_header = None; // biblioteca fuera: liberar planos cedeados
        self.library.lib_band = None;
        self.library.lib_row_dirty = None;
        self.list_dirty = true;
        self.redraw();
    }

    /// "Reescanear" del selector de añadir: re-consulta MediaStore y refresca
    /// SOLO la lista temporal (`select_list`); la biblioteca curada queda
    /// intacta.
    pub(crate) fn rescan_select(&mut self, app: &AndroidApp) {
        let scan = query_media_store(app, self.sdk_int);
        self.permission_granted = scan.permission_granted;
        self.select_list = scan.entries;
        self.sel_dir = Vec::new();
        self.list_scroll = 0;
        self.status = if !self.permission_granted {
            Some("All files access not granted — grant it in system Settings".to_string())
        } else if let Some(e) = scan.error {
            Some(format!("MediaStore error: {e}"))
        } else if self.select_list.is_empty() {
            Some("No PDFs found on the device".to_string())
        } else {
            None
        };
        info!("rescan select: {} PDFs", self.select_list.len());
        self.list_dirty = true;
        self.redraw();
    }

    /// "Atrás" del selector de añadir: descarta la lista temporal y vuelve a
    /// la biblioteca curada SIN ningún cambio.
    pub(crate) fn cancel_add(&mut self, app: &AndroidApp) {
        self.select_list = Vec::new();
        self.reload_curated_library(app);
    }

    /// Confirmación del selector: copia el PDF elegido a `internal/pdfs/`
    /// (nombre saneado), cuenta páginas abriéndolo con MuPDF, crea su
    /// registro de progreso (`touch_progress`), aplica el TOPE `LIBRARY_MAX`
    /// con evicción LRU (`enforce_library_limit`: borra fichero + portada
    /// cacheada de cada expulsado), guarda `library.json` y reconstruye la
    /// biblioteca curada. Toast si hubo expulsión.
    pub(crate) fn add_selected(&mut self, app: &AndroidApp, index: usize) {
        let Some(entry) = self.select_list.get(index).cloned() else {
            return;
        };
        let Some(dir) = app.internal_data_path() else {
            error!("add selected: internal_data_path unavailable");
            return;
        };
        let pdfs_dir = dir.join("pdfs");
        if let Err(e) = fs::create_dir_all(&pdfs_dir) {
            error!("add selected: create_dir_all {}: {e}", pdfs_dir.display());
            return;
        }
        let dest = pdfs_dir.join(sanitize_pdf_name(&entry.name));
        match read_content_uri_bytes(app, &entry.uri) {
            Some(bytes) => {
                if let Err(e) = fs::write(&dest, &bytes) {
                    error!("add selected: write {}: {e}", dest.display());
                    self.status = Some(format!("Cannot copy {}", entry.name));
                    self.list_dirty = true;
                    self.redraw();
                    return;
                }
            }
            None => {
                error!("add selected: cannot read {}", entry.uri);
                self.status = Some(format!("Cannot read {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        }
        let engine = match MupdfEngine::new() {
            Ok(e) => e,
            Err(e) => {
                error!("MupdfEngine::new: {e}");
                self.status = Some(format!("Cannot open {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        };
        let page_count = match engine.open(&dest) {
            Ok(doc) => doc.page_count(),
            Err(e) => {
                error!("add selected: cannot open {}: {e}", dest.display());
                self.status = Some(format!("Invalid PDF {}", entry.name));
                self.list_dirty = true;
                self.redraw();
                return;
            }
        };
        let path = dest.display().to_string();
        let now = persist::unix_now();
        let books = persist::touch_progress(&self.library.lib_books, &path, 0, page_count, now);
        // E4: Política estricta anti-borrado automático. La biblioteca NUNCA
        // elimina un PDF automáticamente. Solo la acción explícita del
        // usuario desde el menú puede borrar un libro.
        self.library.lib_books = books;
        persist::save_progress(self.internal_dir.as_deref(), &self.library.lib_books);
        info!(
            "added {path} ({page_count} pages); library {} books (0 auto-evicted)",
            self.library.lib_books.len()
        );
        self.select_list = Vec::new();
        self.reload_curated_library(app);
    }

    /// "Buscar...": abre el TECLADO del sistema sobre el EditText invisible
    /// (`jni::ime_attach`; ver `tools/ime/ImeHelper.java`) y activa el polling
    /// de `tick`. El texto tecleado filtra la rejilla por subcadena.
    pub(crate) fn lib_open_keyboard(&mut self, app: &AndroidApp) {
        crate::jni::ime_attach(app, &self.library.lib_query);
        self.ime_active = true;
    }

    /// "✕" del campo de búsqueda: limpia el texto tecleado, cierra el
    /// teclado y re-aplica (recalcula `lib_filtered`; vuelve a verse toda la
    /// biblioteca).
    pub(crate) fn lib_clear_search(&mut self, app: &AndroidApp) {
        self.library.lib_query.clear();
        crate::jni::ime_set_text(app, "");
        self.ime_active = false;
        crate::jni::ime_hide(app);
        self.apply_filter();
    }

    /// Cierra el teclado del buscador si está abierto (al entrar al visor,
    /// al abrir el selector de añadir, etc.). No toca el texto del filtro.
    pub(crate) fn lib_close_ime(&mut self, app: &AndroidApp) {
        if self.ime_active {
            self.ime_active = false;
            crate::jni::ime_hide(app);
        }
    }

    /// Polling del texto tecleado (llamado desde `tick`): si cambió, se
    /// re-filtra la rejilla (busca mientras se escribe, sin botón).
    pub(crate) fn poll_ime_query(&mut self, app: &AndroidApp) {
        if !self.ime_active {
            return;
        }
        let Some(t) = crate::jni::ime_text(app) else {
            return;
        };
        if self.mode == UiMode::Discover {
            if t != self.discover.query {
                self.discover.query = t;
                self.discover.dirty = true;
                self.redraw();
            }
        } else if t != self.library.lib_query {
            self.library.lib_query = t;
            self.refresh_lib_filtered();
            let max_v = self.lib_max_scroll();
            if self.library.lib_scroll > max_v {
                self.library.lib_scroll = max_v;
            }
            self.list_dirty = true;
            self.redraw();
        }
    }

    /// Abre un documento de la biblioteca. Entrada CURADA: `uri` es la RUTA
    /// LOCAL del fichero ya copiado en `internal/pdfs/` → abrir directo sin
    /// copiar (`open_pdf_at`, reanuda en la página guardada). Entrada
    /// clásica de MediaStore (content://) → copia los bytes a `internal/
    /// pdfs/` y abre con MuPDF. Devuelve false (estado intacto) si algo falla.
    pub(crate) fn open_library_entry(&mut self, app: &AndroidApp, entry: &LibraryEntry) -> bool {
        // Biblioteca CURADA: el fichero ya está en `internal/pdfs/`.
        if Path::new(&entry.uri).is_file() {
            self.lib_close_ime(app);
            let start =
                crate::persist::progress_for(&self.library.lib_books, &entry.uri).map(|p| p.page);
            return self.open_pdf_at(&entry.uri, start);
        }
        let Some(dir) = app.internal_data_path() else {
            error!("open library: internal_data_path unavailable");
            return false;
        };
        let pdfs_dir = dir.join("pdfs");
        if let Err(e) = fs::create_dir_all(&pdfs_dir) {
            error!("open library: create_dir_all {}: {e}", pdfs_dir.display());
            return false;
        }
        let dest = pdfs_dir.join(sanitize_pdf_name(&entry.name));
        match read_content_uri_bytes(app, &entry.uri) {
            Some(bytes) => {
                let n = bytes.len();
                if let Err(e) = fs::write(&dest, &bytes) {
                    error!("open library: write {}: {e}", dest.display());
                    return false;
                }
                info!(
                    "library open: {} ({}) -> {} ({} bytes)",
                    entry.name,
                    entry.folder,
                    dest.display(),
                    n
                );
            }
            None => {
                error!("open library: cannot read {}", entry.uri);
                return false;
            }
        }
        let path = dest.display().to_string();
        // Reanudar en la página guardada si el libro ya se empezó
        // (registro de progreso de `library.json`); si no, página 1.
        self.lib_close_ime(app);
        let start = crate::persist::progress_for(&self.library.lib_books, &path).map(|p| p.page);
        self.open_pdf_at(&path, start)
    }

    /// Relee los directorios de la app (botón Rescan del picker).
    pub(crate) fn rescan(&mut self, app: &AndroidApp) {
        self.pdf_list = scan_pdfs(app);
        self.list_scroll = 0;
        self.status = None;
        self.list_dirty = true;
        info!("rescan: {} PDFs", self.pdf_list.len());
        self.redraw();
    }
}
