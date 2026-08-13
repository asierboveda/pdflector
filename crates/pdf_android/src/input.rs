//! Input multitáctil: máquina de gestos del visor (tap/pinch/pull del sheet)
//! y taps/arrastre de las listas (picker interno y biblioteca MediaStore).
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): `lib` solo
//! llama a `handle_input`; los gestos tocan `Reader` a través de sus campos y
//! métodos `pub(crate)`.
//!
//! ## Visor página a página + sheet de ajustes (2026-08-XX)
//!
//! El arrastre para scrollear se ELIMINÓ por decisión del autor: el visor
//! vuelve a ser página a página. TAP en la mitad izquierda = página anterior,
//! TAP en la mitad derecha = página siguiente (tap simple, sin drag; un dedo
//! que se desliza más de `TAP_SLOP` cancela el tap). El pinch con dos dedos
//! sigue haciendo zoom (factor RELATIVO + anclado, `Reader::begin_pinch`).
//!
//! El **sheet de ajustes** (panel deslizante desde el borde superior, la
//! mitad de la ventana; ver `Reader::sheet_*` y `draw::render_sheet`) se
//! revela con un arrastre de UN dedo que empieza en la MITAD SUPERIOR y baja
//! (más de `TAP_SLOP`): el panel sigue al dedo y al soltar se queda abierto si
//! pasó de la mitad. Con el sheet visible, un arrastre vertical lo mueve
//! (subir = cerrar) y un TAP fuera del panel lo cierra; un tap dentro pulsa
//! sus botones (Back/Open/Dark/−10/N/+10, misma geometría que
//! `draw::sheet_buttons`). El gesto del sheet NO choca con el tap de página
//! (el tap es < `TAP_SLOP` de movimiento) ni con el pinch (2 dedos → zoom,
//! el sheet se queda como esté). El indicador "N / total" abajo a la
//! izquierda también es táctil: tap = página siguiente (`page_badge_tap`).
//!
//! El modo dibujo (trazo con un dedo) se ELIMINÓ con la barra superior
//! (2026-08-XX): no queda ningún gesto de dibujo en el visor.

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus};
use log::warn;

use crate::draw::{library_buttons, sheet_buttons};
use crate::jni::launch_all_files_settings;
use crate::reader::{
    GRID_COLS, Reader, UiMode, grid_cell_h, grid_cell_w, grid_gap, grid_pad, grid_rows_y0,
    grid_visible_rows, page_badge_rect, picker_btn_h, picker_btn_w, picker_header_h, picker_row_h,
    picker_visible_rows, sheet_h,
};
use crate::{PINCH_MAX, PINCH_MIN, TAP_SLOP};

/// Gesto multitáctil en curso (máquina de gestos).
#[derive(Clone, Copy, Debug)]
enum GestureKind {
    None,
    /// Un dedo: posible tap (página anterior/siguiente, indicador de página,
    /// sheet abierto: botón o cerrar). El gesto se CANCELA si el dedo se
    /// mueve más de `TAP_SLOP` sin convertirse en un pull del sheet (un
    /// pequeño deslizamiento no cambia de página — sin scroll por arrastre en
    /// el modo página a página); al soltar sin moverse se dispara el tap.
    Tap {
        start_x: f32,
        start_y: f32,
    },
    /// Un dedo: arrastre VERTICAL que controla el sheet de ajustes (revelado
    /// con un tirón hacia abajo desde la mitad superior; con el sheet visible,
    /// subir/bajar lo mueve). `start_y` = Y del Down; el progreso del sheet
    /// sigue a `dy = y − start_y` (`Reader::drag_sheet`).
    Pull {
        start_y: f32,
    },
    /// Dos dedos: pinch zoom. `start_dist` es la distancia entre dedos al
    /// iniciar el gesto y `start_zoom` el zoom de partida; el zoom resultante
    /// es `start_zoom * dist / start_dist` (factor RELATIVO, no incremental
    /// por evento). El anclaje (punto de pantalla fijo bajo los dedos) se
    /// registra en `Reader::begin_pinch` con el centro del pinch.
    Pinch {
        start_dist: f32,
        start_zoom: f32,
    },
}

/// Estado de los gestos: pointers activos (pointer_id, x, y) + gesto en curso.
pub(crate) struct GestureState {
    pointers: Vec<(i32, f32, f32)>,
    kind: GestureKind,
}

impl GestureState {
    pub(crate) fn new() -> Self {
        Self {
            pointers: Vec::new(),
            kind: GestureKind::None,
        }
    }
}

/// Tap simple: mitad derecha → página siguiente; mitad izquierda → anterior.
fn tap_page(reader: &mut Reader, x: f32) {
    if x >= reader.win_w as f32 / 2.0 {
        reader.next_page();
    } else {
        reader.prev_page();
    }
}

/// Tap en el indicador de página "N / total" (overlay abajo a la izquierda):
/// página siguiente. Devuelve true si el punto cae en el indicador
/// (consumido). Decisión documentada (2026-08-XX): el indicador además de
/// informar es un acceso rápido a la página siguiente, igual que el
/// indicador de la antigua barra superior.
fn page_badge_tap(reader: &mut Reader, x: f32, y: f32) -> bool {
    let (l, t, r, b) = page_badge_rect(reader.win_w, reader.win_h);
    if x >= l as f32 && x < r as f32 && y >= t as f32 && y < b as f32 {
        reader.next_page();
        true
    } else {
        false
    }
}

/// Tap DENTRO del sheet de ajustes: botones (misma geometría que
/// `draw::sheet_buttons`): Back (biblioteca MediaStore), Open (picker
/// interno), Dark/Light, −10/+10 y "N / total" (página siguiente). Un tap en
/// el hueco del sheet (fuera de los botones) no hace nada: el panel se cierra
/// con un tap FUERA del sheet o con un arrastre hacia arriba.
fn sheet_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    for (label, (l, t, r, b)) in sheet_buttons(reader, reader.win_w as f32, reader.win_h as f32) {
        if x >= l && x < r && y >= t && y < b {
            match label {
                "Back" => reader.enter_library(app),
                "Open" => reader.open_picker(app),
                "Dark" | "Light" => reader.toggle_dark(),
                "-10" => reader.jump_page(-10),
                "+10" => reader.jump_page(10),
                _ => reader.next_page(), // "N / total"
            }
            return;
        }
    }
}

/// Procesa un `MotionEvent` del VISOR: actualiza la máquina de gestos y actúa
/// sobre el reader. En modo picker/biblioteca se delega en
/// `handle_picker_motion` (arrastre + tap de lista, sin pinch).
///
/// Gestos del visor (página a página):
/// - tap en la mitad derecha = página siguiente; izquierda = anterior
///   (con el sheet visible, el tap cierra el panel o pulsa un botón);
/// - tirón hacia abajo desde la mitad superior = revelar el sheet de ajustes;
///   arrastre vertical con el sheet visible = moverlo (arriba cierra);
/// - pinch con dos dedos = zoom (factor relativo + anclado al centro del
///   pinch);
/// - un dedo que se desliza más de `TAP_SLOP` y no es un pull cancela el tap
///   (sin scroll: el arrastre se eliminó por decisión del autor).
fn handle_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
    up_idx: Option<usize>,
) {
    if reader.mode == UiMode::Picker || reader.mode == UiMode::Library {
        handle_picker_motion(reader, app, action, pts, up_idx);
        return;
    }
    match action {
        MotionAction::Down => {
            // Primer dedo: arranca un posible tap (página, indicador o sheet).
            reader.gesture.pointers = pts;
            if let Some(&(_, x, y)) = reader.gesture.pointers.first() {
                reader.gesture.kind = GestureKind::Tap {
                    start_x: x,
                    start_y: y,
                };
            }
        }
        MotionAction::PointerDown => {
            reader.gesture.pointers = pts;
            // Segundo dedo: pinch. Distancia inicial = base del factor de
            // zoom; el centro del pinch (punto medio de los dedos) se fija
            // como ancla del zoom (`begin_pinch`): el punto de documento bajo
            // los dedos permanece fijo en pantalla durante el gesto.
            if reader.gesture.pointers.len() >= 2 {
                let (_, ax, ay) = reader.gesture.pointers[0];
                let (_, bx, by) = reader.gesture.pointers[1];
                let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                if d > 8.0 {
                    reader.begin_pinch((ax + bx) / 2.0, (ay + by) / 2.0);
                    reader.gesture.kind = GestureKind::Pinch {
                        start_dist: d,
                        start_zoom: reader.zoom,
                    };
                }
            }
        }
        MotionAction::Move => {
            reader.gesture.pointers = pts;
            let kind = reader.gesture.kind;
            match kind {
                GestureKind::Tap { start_x, start_y } if reader.gesture.pointers.len() == 1 => {
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    let moved = ((cx - start_x).powi(2) + (cy - start_y).powi(2)).sqrt();
                    if moved > TAP_SLOP {
                        let (dx, dy) = (cx - start_x, cy - start_y);
                        let sheet_visible = reader.sheet_progress > 0.0;
                        // ¿Pull del sheet? (1 dedo, deslizamiento vertical
                        // dominante):
                        // - sheet cerrado: tirar hacia abajo desde la mitad
                        //   superior (el gesto de revelado del enunciado);
                        // - sheet visible: cualquier arrastre vertical lo
                        //   mueve (bajar = mantener/abrir, subir = cerrar).
                        let pull = if sheet_visible {
                            dy.abs() > dx.abs()
                        } else {
                            dy > 0.0 && start_y < reader.win_h as f32 / 2.0
                        };
                        if pull {
                            reader.begin_sheet_drag();
                            reader.gesture.kind = GestureKind::Pull { start_y };
                        } else {
                            // Deslizamiento que no es del sheet: cancela el
                            // tap (sin scroll; arrastre eliminado).
                            reader.gesture.kind = GestureKind::None;
                        }
                    }
                }
                GestureKind::Pull { start_y, .. } if reader.gesture.pointers.len() == 1 => {
                    let (_, _, cy) = reader.gesture.pointers[0];
                    reader.drag_sheet(cy - start_y);
                }
                GestureKind::Pinch {
                    start_dist,
                    start_zoom,
                } if reader.gesture.pointers.len() >= 2 => {
                    let (_, ax, ay) = reader.gesture.pointers[0];
                    let (_, bx, by) = reader.gesture.pointers[1];
                    let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                    if d > 1.0 {
                        // Factor RELATIVO a la distancia inicial del gesto
                        // (no incremental por evento): pinch-out/in sin mover
                        // los dedos devuelve exactamente el zoom de partida.
                        let zoom = (start_zoom * d / start_dist).clamp(PINCH_MIN, PINCH_MAX);
                        // Fast: solo actualiza `zoom` y el pan de anclaje y
                        // blitea el bitmap cacheado con `blit_fast`; el
                        // re-render nítido llega al soltar.
                        reader.set_zoom_fast(zoom);
                    }
                }
                _ => {}
            }
        }
        MotionAction::Up => {
            // El dedo que se levanta todavía aparece en `pts` con sus últimas
            // coordenadas: usarlas para decidir tap vs gesto cancelado antes
            // de limpiar.
            let up = pts.first().copied();
            let kind = reader.gesture.kind;
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
            match kind {
                GestureKind::Tap { start_x, start_y } => {
                    // Sin movimiento relevante → tap.
                    if let Some((_, x, y)) = up {
                        let moved = ((x - start_x).powi(2) + (y - start_y).powi(2)).sqrt();
                        if moved <= TAP_SLOP {
                            if reader.sheet_progress > 0.0 {
                                // Sheet visible: tap DENTRO → botones; tap
                                // FUERA → cerrar el panel (sin cambiar de
                                // página: cerrar el sheet no debe avanzar).
                                if y < sheet_h(reader.win_h) as f32 {
                                    sheet_tap(reader, app, x, y);
                                } else {
                                    reader.hide_sheet();
                                }
                            } else if page_badge_tap(reader, x, y) {
                                // Indicador de página: siguiente (consumido).
                            } else {
                                tap_page(reader, x);
                            }
                        }
                    }
                }
                GestureKind::Pull { .. } => {
                    // Fin del arrastre del sheet: animar hasta el objetivo
                    // más cercano (abierto si pasó de la mitad).
                    reader.end_sheet_drag();
                }
                GestureKind::Pinch { .. } | GestureKind::None => {}
            }
        }
        MotionAction::PointerUp => {
            // `up_idx` es el índice del pointer levantado dentro del evento
            // (mismo orden que `pts`): quitarlo del estado.
            if let Some(idx) = up_idx
                && idx < reader.gesture.pointers.len()
            {
                reader.gesture.pointers.remove(idx);
            }
            // Al quedar menos de dos dedos el pinch termina: re-render nítido
            // UNA única vez a la resolución final (`set_zoom_sharp`). El dedo
            // restante no inicia un tap (se ignora hasta que se levanta).
            if matches!(reader.gesture.kind, GestureKind::Pinch { .. })
                && reader.gesture.pointers.len() < 2
            {
                reader.gesture.kind = GestureKind::None;
                reader.set_zoom_sharp(reader.zoom);
            }
        }
        MotionAction::Cancel => {
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
        }
        _ => {} // HoverMove, Scroll, Outside, ...: sin gesto definido.
    }
}

/// Tap sobre la lista activa (picker interno o biblioteca MediaStore).
fn list_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    match reader.mode {
        UiMode::Picker => picker_tap(reader, app, x, y),
        UiMode::Library => library_tap(reader, app, x, y),
        UiMode::Viewer => {}
    }
}

/// Tap de la biblioteca MediaStore (rejilla 3×3): botones de la cabecera
/// (Back/Grant/Rescan) o celda de la rejilla (abrir PDF). La geometría DEBE
/// reflejar exactamente la de `render_library_grid` (mismas fórmulas de
/// layout: `grid_cell_rect`, `grid_rows_y0`, `grid_visible_rows`).
fn library_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let header_h = picker_header_h(reader.win_h) as f32;
    let btn_w = picker_btn_w(reader.win_w) as f32;
    let btn_h = picker_btn_h(reader.win_h) as f32;
    let btn_y = (header_h - btn_h) / 2.0;

    // Cabecera: botones a la derecha (Rescan, Grant, Back — ver library_buttons).
    if y < header_h {
        for (label, (l, t, r, b)) in
            library_buttons(reader, reader.win_w as f32, btn_w, btn_h, btn_y)
        {
            if x >= l && x < r && y >= t && y < b {
                match label {
                    "Rescan" => reader.rescan_library(app),
                    "Grant" => {
                        reader.grant_pending = true;
                        launch_all_files_settings(app);
                    }
                    "Back" => reader.exit_picker(),
                    _ => {}
                }
                return;
            }
        }
        return;
    }

    // Franja de estado: no es seleccionable.
    let rows_y0 = grid_rows_y0(reader.win_h, reader.status.is_some()) as f32;
    if y < rows_y0 {
        return;
    }

    // Celda de la rejilla: fila = (y − rows_y0) / cell_h + scroll; columna
    // por x (misma geometría que `grid_cell_rect`).
    let row = ((y - rows_y0) / grid_cell_h(reader.win_w)) as usize + reader.list_scroll;
    let cell_w = grid_cell_w(reader.win_w);
    let pad = grid_pad(reader.win_w);
    let col = ((x - pad) / (cell_w + grid_gap())).floor() as usize;
    if col < GRID_COLS
        && let Some(entry) = reader.grid_entry_at(row, col)
    {
        let entry = entry.clone();
        if !reader.open_library_entry(app, &entry) {
            reader.status = Some(format!("Cannot open {}", entry.name));
            reader.list_dirty = true;
            reader.redraw();
        }
    }
}

/// Tap del picker: botones de la cabecera (Back/Rescan) o fila de la lista
/// (abrir PDF). La geometría DEBE reflejar exactamente la de
/// `render_picker_list` (mismas fórmulas de layout).
fn picker_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let win_w = reader.win_w as f32;
    let row_h = picker_row_h(reader.win_h) as f32;
    let header_h = picker_header_h(reader.win_h) as f32;
    let status_h = if reader.status.is_some() { row_h } else { 0.0 };
    let btn_w = picker_btn_w(reader.win_w) as f32;

    // Cabecera: botones a la derecha (Back a la izquierda de Rescan).
    if y < header_h {
        let rescan_x = win_w - btn_w - 8.0;
        if x >= rescan_x {
            reader.rescan(app);
            return;
        }
        let back_x = win_w - btn_w * 2.0 - 16.0;
        if reader.doc.is_some() && x >= back_x && x < rescan_x {
            reader.exit_picker();
            return;
        }
        return;
    }

    // Franja de estado: no es seleccionable.
    let rows_y0 = header_h + status_h;
    if y < rows_y0 {
        return;
    }

    let row = ((y - rows_y0) / row_h) as usize + reader.list_scroll;
    if row < reader.pdf_list.len() {
        let name = reader.pdf_list[row].name.clone();
        let path = reader.pdf_list[row].path.clone();
        if !reader.open_pdf(&path) {
            reader.status = Some(format!("Cannot open {name}"));
            reader.list_dirty = true;
            reader.redraw();
        }
    }
}

/// Input del picker/biblioteca (un solo dedo): arrastre vertical = scroll de
/// la lista o rejilla (filas de `picker_row_h` o `grid_cell_h` según el modo),
/// tap (sin arrastre) = selección. Reemplaza a la máquina de gestos del visor
/// (sin pinch).
fn handle_picker_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
    _up_idx: Option<usize>,
) {
    match action {
        MotionAction::Down => {
            if let Some(&(_, x, y)) = pts.first() {
                reader.picker_drag = Some((x, y, reader.list_scroll));
            }
        }
        MotionAction::Move => {
            if let Some((sx, sy, sscroll)) = reader.picker_drag
                && let Some(&(_, x, y)) = pts.first()
            {
                let moved = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
                if moved > TAP_SLOP {
                    // Alto de fila y nº de filas visibles según el modo
                    // (picker: filas de lista; biblioteca: filas de celdas).
                    let row_h = if reader.mode == UiMode::Picker {
                        picker_row_h(reader.win_h) as f32
                    } else {
                        grid_cell_h(reader.win_w)
                    };
                    let visible = if reader.mode == UiMode::Picker {
                        picker_visible_rows(reader.win_h, reader.status.is_some())
                    } else {
                        grid_visible_rows(reader.win_w, reader.win_h, reader.status.is_some())
                    };
                    let list_len = if reader.mode == UiMode::Picker {
                        reader.pdf_list.len()
                    } else {
                        reader.grid_total_rows()
                    };
                    let max_scroll = list_len.saturating_sub(visible);
                    let s = (sscroll as f32 - (y - sy) / row_h)
                        .round()
                        .clamp(0.0, max_scroll as f32) as usize;
                    if s != reader.list_scroll {
                        reader.list_scroll = s;
                        reader.list_dirty = true;
                        reader.redraw();
                    }
                }
            }
        }
        MotionAction::Up => {
            let drag = reader.picker_drag.take();
            if let (Some((sx, sy, _)), Some(&(_, x, y))) = (drag, pts.first()) {
                let moved = ((x - sx).powi(2) + (y - sy).powi(2)).sqrt();
                if moved <= TAP_SLOP {
                    list_tap(reader, app, x, y);
                }
            }
            reader.gesture.pointers.clear();
        }
        MotionAction::Cancel => {
            reader.picker_drag = None;
            reader.gesture.pointers.clear();
        }
        // PointerUp: se ignora un segundo dedo (el picker no tiene pinch).
        _ => {}
    }
}

/// Input multitáctil: tap (1 dedo, página anterior/siguiente o sheet), pull
/// (1 dedo, sheet de ajustes) y pinch (2 dedos, zoom).
pub(crate) fn handle_input(app: &AndroidApp, reader: &mut Reader) {
    let Ok(mut iter) = app.input_events_iter() else {
        warn!("input_events_iter failed");
        return;
    };
    loop {
        let read = iter.next(|event| match event {
            InputEvent::MotionEvent(motion) => {
                let action = motion.action();
                let pts: Vec<(i32, f32, f32)> = motion
                    .pointers()
                    .map(|p| (p.pointer_id(), p.x(), p.y()))
                    .collect();
                let up_idx = if action == MotionAction::PointerUp {
                    Some(motion.pointer_index())
                } else {
                    None
                };
                handle_motion(reader, app, action, pts, up_idx);
                InputStatus::Handled
            }
            InputEvent::KeyEvent(_) | InputEvent::TextEvent(_) | InputEvent::TextAction(_) | _ => {
                InputStatus::Unhandled
            }
        });
        if !read {
            break;
        }
    }
}
