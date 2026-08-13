//! Input multitáctil: máquina de gestos del visor (swipe/pinch/tap) y taps/
//! arrastre de las listas (picker interno y biblioteca MediaStore).
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): `lib` solo
//! llama a `handle_input`; los gestos tocan `Reader` a través de sus campos y
//! métodos `pub(crate)`. El zoom es SOLO con pinch (fast durante el gesto,
//! sharp al soltar); el doble-tap se eliminó.

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus};
use log::warn;

use crate::draw::library_buttons;
use crate::jni::launch_all_files_settings;
use crate::reader::{
    Reader, UiMode, lib_strip_cell, lib_strip_letter, lib_strip_w, picker_btn_h, picker_btn_w,
    picker_header_h, picker_row_h, picker_visible_rows,
};
use crate::{
    COLOR_BTN_W_DIV, DARK_BTN_W_DIV, JUMP_BTN_W_DIV, OPEN_BTN_W_DIV, PENCIL_BTN_W_DIV, PINCH_MAX,
    PINCH_MIN, SWIPE_FRACTION, TAP_SLOP, UNDO_BTN_W_DIV, VIEWER_BAR_H_DIV,
};

/// Gesto multitáctil en curso (máquina de gestos).
#[derive(Clone, Copy, Debug)]
enum GestureKind {
    None,
    /// Un dedo: arrastre vertical = scroll continuo (el contenido sigue al
    /// dedo, sin cambiar de página); barrido horizontal dominante = salto de
    /// página; al soltar sin moverse = tap. `consumed` evita disparar más de
    /// una página dentro del mismo gesto.
    Swipe {
        start_x: f32,
        start_y: f32,
        /// `scroll_y` de partida: el scroll continuo sigue al dedo
        /// (`scroll_y = start_scroll − dy`).
        start_scroll: f32,
        consumed: bool,
    },
    /// Dos dedos: pinch zoom. `start_dist` es la distancia entre dedos al
    /// iniciar el gesto y `start_zoom` el zoom de partida; el zoom resultante
    /// es `start_zoom * dist / start_dist`.
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

/// Tap en la barra superior del visor (overlay opaco en (0,0), alto
/// `win_h/VIEWER_BAR_H_DIV`): Open | ✏️ (modo dibujo) | ● (color) | ↶ (undo)
/// | −10 | "N / total" (tap = página siguiente) | +10 | Dark. Devuelve true
/// si el punto cae en la barra (consumido). Las regiones DEBEN reflejar
/// `render_viewer_bar` (mismas divisiones de la ventana).
fn viewer_bar_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) -> bool {
    let (w, h) = (reader.win_w as f32, reader.win_h as f32);
    if y > h / VIEWER_BAR_H_DIV as f32 {
        return false;
    }
    let open_w = w / OPEN_BTN_W_DIV as f32;
    let pencil_w = w / PENCIL_BTN_W_DIV as f32;
    let color_w = w / COLOR_BTN_W_DIV as f32;
    let undo_w = w / UNDO_BTN_W_DIV as f32;
    let dark_w = w / DARK_BTN_W_DIV as f32;
    let jump_w = w / JUMP_BTN_W_DIV as f32;
    let left_end = open_w + pencil_w + color_w + undo_w + jump_w;
    let right_start = w - dark_w - jump_w;
    if x < open_w {
        reader.enter_library(app);
    } else if x < open_w + pencil_w {
        reader.toggle_draw_mode();
    } else if x < open_w + pencil_w + color_w {
        reader.cycle_stroke_color();
    } else if x < open_w + pencil_w + color_w + undo_w {
        reader.undo_last_stroke();
    } else if x < left_end {
        reader.jump_page(-10);
    } else if x < right_start {
        // Zona del indicador: página siguiente (tap repetido avanza).
        reader.next_page();
    } else if x < w - dark_w {
        reader.jump_page(10);
    } else {
        reader.toggle_dark();
    }
    true
}

/// Input del modo dibujo (un dedo): el arrastre crea un trazo (polilínea en
/// coordenadas de página) en vez de hacer scroll; los taps en la barra
/// superior siguen funcionando (Open/✏️/●/↶/saltos/Dark). Sin pinch ni
/// scroll: un segundo dedo se ignora (los PointerDown/PointerUp extra no
/// cambian el estado; el trazo termina en el Up del último dedo o en Cancel).
fn handle_draw_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
) {
    match action {
        MotionAction::Down => {
            reader.gesture.pointers = pts;
            // Empezar el trazo solo si el dedo cae sobre una página (fuera de
            // la barra y de los huecos de la columna).
            if let Some(&(_, x, y)) = reader.gesture.pointers.first() {
                let bar_h = reader.win_h as f32 / VIEWER_BAR_H_DIV as f32;
                if y > bar_h
                    && let Some(page) = reader.page_at_y(y)
                {
                    reader.begin_stroke(page, reader.screen_to_page(page, x, y));
                }
            }
        }
        MotionAction::Move => {
            let page = reader.active_stroke.as_ref().map(|a| a.page);
            if let (Some(page), Some(&(_, x, y))) = (page, pts.first()) {
                reader.extend_stroke(page, reader.screen_to_page(page, x, y));
            }
            reader.gesture.pointers = pts;
        }
        MotionAction::Up => {
            // El dedo se levanta: cerrar el trazo (los de < 2 puntos — taps —
            // se descartan en `finish_stroke`). La barra superior solo
            // responde a TAPS: si el gesto terminó guardando un trazo (aunque
            // el Up caiga sobre la barra), NO se dispara ningún botón — un
            // trazo que termina en el borde superior no debe alternar el modo
            // dibujo por accidente.
            let up = pts.first().copied();
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
            if !reader.finish_stroke()
                && let Some((_, x, y)) = up
            {
                viewer_bar_tap(reader, app, x, y);
            }
        }
        MotionAction::Cancel => {
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
            reader.cancel_stroke();
        }
        MotionAction::PointerDown | MotionAction::PointerUp => {
            reader.gesture.pointers = pts;
        }
        _ => {}
    }
}

/// Procesa un `MotionEvent`: actualiza la máquina de gestos y actúa sobre el
/// reader. En modo picker se delega en `handle_picker_motion` (arrastre + tap
/// de lista), sin pinch.
///
/// Convención de direcciones (lector en portrait, semántica de libro/scroll):
/// - arrastre vertical = scroll continuo (el documento sigue al dedo);
/// - barrido horizontal → derecha = página anterior; izquierda = siguiente;
/// - tap en la mitad derecha = siguiente; izquierda = anterior (fallback).
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
    // Modo dibujo (botón "✏️"): el arrastre crea trazos en vez de hacer
    // scroll; la máquina de gestos (swipe/pinch/tap) queda suspendida.
    if reader.draw_mode {
        handle_draw_motion(reader, app, action, pts);
        return;
    }
    match action {
        MotionAction::Down => {
            // Primer dedo: arranca un posible swipe (o tap al soltar sin moverse).
            reader.gesture.pointers = pts;
            if let Some(&(_, x, y)) = reader.gesture.pointers.first() {
                reader.gesture.kind = GestureKind::Swipe {
                    start_x: x,
                    start_y: y,
                    start_scroll: reader.scroll_y,
                    consumed: false,
                };
            }
        }
        MotionAction::PointerDown => {
            reader.gesture.pointers = pts;
            // Segundo dedo: pinch. Distancia inicial = base del factor de zoom.
            if reader.gesture.pointers.len() >= 2 {
                let (_, ax, ay) = reader.gesture.pointers[0];
                let (_, bx, by) = reader.gesture.pointers[1];
                let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                if d > 8.0 {
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
                GestureKind::Swipe {
                    start_x,
                    start_y,
                    start_scroll,
                    consumed: false,
                } if reader.gesture.pointers.len() == 1 => {
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    let dx = cx - start_x;
                    let dy = cy - start_y;
                    let moved = (dx * dx + dy * dy).sqrt();
                    if moved > TAP_SLOP && dy.abs() >= dx.abs() {
                        // Arrastre vertical → scroll continuo: el contenido
                        // sigue al dedo (`start_scroll − dy`), sin cambiar de
                        // página. El TAP_SLOP evita micro-scrolls en los taps.
                        reader.scroll_to(start_scroll - dy);
                    } else if dx.abs() > dy.abs() && dx.abs() > reader.win_w as f32 * SWIPE_FRACTION
                    {
                        // Barrido horizontal dominante → salto de página
                        // (comportamiento previo): derecha = anterior,
                        // izquierda = siguiente.
                        if dx > 0.0 {
                            reader.prev_page();
                        } else {
                            reader.next_page();
                        }
                        // Consumido: no volver a disparar dentro de este gesto.
                        reader.gesture.kind = GestureKind::Swipe {
                            start_x,
                            start_y,
                            start_scroll,
                            consumed: true,
                        };
                    }
                }
                GestureKind::Pinch {
                    start_dist,
                    start_zoom,
                } if reader.gesture.pointers.len() >= 2 => {
                    let (_, ax, ay) = reader.gesture.pointers[0];
                    let (_, bx, by) = reader.gesture.pointers[1];
                    let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                    if d > 1.0 {
                        let zoom = (start_zoom * d / start_dist).clamp(PINCH_MIN, PINCH_MAX);
                        // Fast: solo actualiza `zoom` y blitea el bitmap cacheado
                        // con `blit_fast`; el re-render nítido llega al soltar.
                        reader.set_zoom_fast(zoom);
                    }
                }
                _ => {}
            }
        }
        MotionAction::Up => {
            // El dedo que se levanta todavía aparece en `pts` con sus últimas
            // coordenadas: usarlas para decidir tap vs swipe antes de limpiar.
            let up = pts.first().copied();
            let kind = reader.gesture.kind;
            reader.gesture.pointers.clear();
            if let GestureKind::Swipe {
                start_x,
                start_y,
                start_scroll: _,
                consumed: false,
            } = kind
            {
                // Sin movimiento relevante → tap (fallback derecha/izquierda).
                if let Some((_, x, y)) = up {
                    let moved = ((x - start_x).powi(2) + (y - start_y).powi(2)).sqrt();
                    if moved <= TAP_SLOP && !viewer_bar_tap(reader, app, x, y) {
                        tap_page(reader, x);
                    }
                }
            }
            reader.gesture.kind = GestureKind::None;
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
            // restante no inicia un swipe (se ignora hasta que se levanta).
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

/// Tap de la biblioteca MediaStore: botones de la cabecera (Back/Grant/
/// Rescan), celda de la tira de letras (índice: filtra por la letra inicial
/// normalizada; repetir la activa la desactiva) o fila de la lista (abrir
/// PDF). La geometría DEBE reflejar exactamente la de
/// `render_library_list` (mismas fórmulas de layout).
fn library_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let win_w = reader.win_w as f32;
    let row_h = picker_row_h(reader.win_h) as f32;
    let header_h = picker_header_h(reader.win_h) as f32;
    let status_h = if reader.status.is_some() { row_h } else { 0.0 };
    let btn_w = picker_btn_w(reader.win_w) as f32;
    let btn_h = picker_btn_h(reader.win_h) as f32;
    let btn_y = (header_h - btn_h) / 2.0;
    let rows_y0 = (header_h + status_h) as i32;

    // Cabecera: botones a la derecha (Rescan, Grant, Back — ver library_buttons).
    if y < header_h {
        for (label, (l, t, r, b)) in library_buttons(reader, win_w, btn_w, btn_h, btn_y) {
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
    if y < rows_y0 as f32 {
        return;
    }

    // Tira de letras (índice, borde derecho): tocar una celda filtra la
    // lista por esa letra (normalizada, minúsculas + sin acentos); repetir
    // la letra activa quita el filtro (todas).
    let strip_w = lib_strip_w(reader.win_w) as f32;
    let list_w = win_w - strip_w;
    if x >= list_w {
        if let Some(cell) = lib_strip_cell(reader.win_h, rows_y0, y) {
            let letter = lib_strip_letter(cell).to_ascii_lowercase();
            let next = if reader.library_filter == Some(letter) {
                None
            } else {
                Some(letter)
            };
            reader.set_library_filter(next);
        }
        return;
    }

    // Fila de la lista FILTRADA (la misma vista que pinta
    // `render_library_list`): abrir el PDF de la entrada.
    let row = ((y - rows_y0 as f32) / row_h) as usize + reader.list_scroll;
    if let Some(entry) = reader.library_entry_at(row) {
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

/// Input del picker (un solo dedo): arrastre vertical = scroll de la lista,
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
                    let row_h = picker_row_h(reader.win_h) as f32;
                    let visible = picker_visible_rows(reader.win_h, reader.status.is_some());
                    // La lista activa (picker interno o biblioteca MediaStore,
                    // esta última con el filtro por letra aplicado).
                    let list_len = if reader.mode == UiMode::Picker {
                        reader.pdf_list.len()
                    } else {
                        reader.filtered_library_len()
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

/// Input multitáctil: swipe (1 dedo), pinch (2 dedos) y tap como fallback.
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
