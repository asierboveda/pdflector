// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

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
//!
//! ## Selección de texto: doble-tap + arrastre (2026-08-XX, Parte 1)
//!
//! Un tap simple en el área de PÁGINA (fuera del sheet, del indicador y del
//! menú) se DIFIERE (`GestureState::pending_tap`, ventana `DOUBLE_TAP_MS`):
//! si llega un segundo down en el mismo sitio y dentro de la ventana, es un
//! DOBLE-TAP y se inicia la selección por arrastre — `GestureKind::Selecting`
//! (ancla = punto del doble-tap; al moverse > `SELECT_SLOP` se materializa
//! el rect en `Reader::sel`, que sigue al dedo; al levantar, `end_sel` fija
//! la selección y abre el menú Copiar/Subrayar/IA). Si la ventana expira sin
//! segundo tap, `tick_gestures` (desde `Reader::tick`, poll con timeout de
//! `Reader::needs_tick`) ejecuta el tap de página — el tap simple espera
//! ~300 ms, el precio estándar del doble-tap. El tap izq/der de página NO se
//! dispara nunca mientras hay selección/menú abierto (`sel_menu_tap`
//! consume esos taps); tocar fuera del menú lo cierra y descarta la
//! selección. Los taps del sheet, del indicador y del menú no se difieren.

use std::time::Instant;

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
use crate::{DOUBLE_TAP_MS, PINCH_MAX, PINCH_MIN, SELECT_SLOP, TAP_SLOP};

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
    /// Segundo down de un doble-tap (selección de texto por arrastre): el
    /// ancla es el punto del doble-tap; al moverse > `SELECT_SLOP` se
    /// materializa la selección (`Reader::begin_sel`) y el rect sigue al
    /// dedo (`Reader::update_sel`); al soltar se fija (`Reader::end_sel`) y
    /// se abre el menú Copiar/Subrayar/IA. Un segundo dedo cancela la
    /// selección en curso y pasa al pinch.
    Selecting {
        anchor: (f32, f32),
    },
}

/// Tap simple del área de página en espera de confirmación de doble-tap: si
/// llega un segundo down en el mismo sitio y dentro de `DOUBLE_TAP_MS`, es el
/// comienzo de la selección y el tap NUNCA se dispara; si la ventana expira
/// sin segundo tap, `tick_gestures` ejecuta el tap de página. Almacena la
/// posición del tap y el momento del Up.
#[derive(Clone, Copy, Debug)]
struct TapPending {
    x: f32,
    y: f32,
    at: Instant,
}

/// Estado de los gestos: pointers activos (pointer_id, x, y) + gesto en curso
/// + tap diferido por la ventana de doble-tap.
pub(crate) struct GestureState {
    pointers: Vec<(i32, f32, f32)>,
    kind: GestureKind,
    /// Tap simple del área de página a la espera de un posible segundo tap
    /// (doble-tap → selección). Some mientras la ventana de 300 ms esté
    /// abierta; `Reader::needs_tick` mantiene el poll con timeout para que
    /// `tick_gestures` lo resuelva aunque no llegue más input.
    pending_tap: Option<TapPending>,
}

impl GestureState {
    pub(crate) fn new() -> Self {
        Self {
            pointers: Vec::new(),
            kind: GestureKind::None,
            pending_tap: None,
        }
    }

    /// ¿Hay un tap diferido esperando la ventana de doble-tap? (el bucle de
    /// eventos mantiene el poll con timeout mientras tanto).
    pub(crate) fn tap_pending(&self) -> bool {
        self.pending_tap.is_some()
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

/// Tap con el menú de selección abierto: dentro de un botón → acción
/// (Copiar → portapapeles; Subrayar → highlight persistido; "IA" → abre el
/// panel de IA con la consulta a Groq); fuera → cerrar el menú y descartar
/// la selección. NUNCA cambia de página: el tap izq/der no se dispara
/// mientras hay selección activa.
fn sel_menu_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let Some(menu) = &reader.sel_menu else {
        return;
    };
    let inside = x >= menu.x as f32
        && x < (menu.x + menu.w) as f32
        && y >= menu.y as f32
        && y < (menu.y + menu.h) as f32;
    if !inside {
        reader.clear_selection(); // tocar fuera: cierra y descarta
        return;
    }
    let hit: Option<&'static str> = menu
        .buttons
        .iter()
        .find(|(_, (l, t, r, b))| x >= *l && x < *r && y >= *t && y < *b)
        .map(|(label, _)| *label);
    match hit {
        Some("Copiar") => reader.copy_sel(app),
        Some("Subrayar") => reader.highlight_sel(),
        // Parte 2: "IA" abre el panel de "Preguntar a la IA" (hilo de fondo
        // + Groq; ver `Reader::ask_ai`). El menú se cierra dentro de
        // `ask_ai` (`clear_selection`), que abre el panel en su lugar.
        Some("IA") => reader.ask_ai(),
        // Defensa: botón desconocido (imposible hoy) → cerrar y descartar.
        Some(_) | None => reader.clear_selection(),
    }
}

/// Tap con el panel de "Preguntar a la IA" abierto: dentro de un botón →
/// acción ("×" → cerrar; "▲"/"▼" → scroll del cuerpo); fuera → cerrar el
/// panel. Un tap DENTRO del panel pero fuera de sus botones no hace nada
/// (evita cerrar el panel por accidente mientras se lee la respuesta; se
/// cierra con ✕ o con tap fuera). NUNCA cambia de página: el tap izq/der no
/// se dispara mientras el panel está abierto (misma regla que el menú de
/// selección, ver `fire_tap_action`).
fn ai_panel_tap(reader: &mut Reader, x: f32, y: f32) {
    let Some(panel) = &reader.ai_panel else {
        return;
    };
    let inside = x >= panel.x as f32
        && x < (panel.x + panel.w) as f32
        && y >= panel.y as f32
        && y < (panel.y + panel.h) as f32;
    if !inside {
        reader.close_ai_panel(); // tap fuera: cerrar el panel
        return;
    }
    let hit: Option<&'static str> = panel
        .buttons
        .iter()
        .find(|(_, (l, t, r, b))| x >= *l && x < *r && y >= *t && y < *b)
        .map(|(label, _)| *label);
    match hit {
        Some("×") => reader.close_ai_panel(),
        Some("▲") => reader.ai_scroll(-1),
        Some("▼") => reader.ai_scroll(1),
        _ => {} // dentro del panel, fuera de los botones: no hacer nada
    }
}

/// Ejecuta la acción de un tap simple en `(x, y)` — la MISMA lógica que el
/// Up del gesto Tap, compartida con el tap diferido del doble-tap
/// (`tick_gestures`/Down): menú de selección abierto → botón o cerrar;
/// sheet visible → botón o cerrar; si no, indicador de página o tap de
/// página.
fn fire_tap_action(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    if reader.sel_menu.is_some() {
        sel_menu_tap(reader, app, x, y);
    } else if reader.ai_panel.is_some() {
        // Panel de IA abierto: sus botones (✕/▲/▼) o cerrar con tap fuera.
        // Va ANTES del sheet y del tap de página: mientras el panel esté
        // abierto ningún otro gesto de tap actúa (misma regla que el menú
        // de selección; el pinch sí sigue funcionando).
        ai_panel_tap(reader, x, y);
    } else if reader.sheet_progress > 0.0 {
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

/// Avanza la máquina de gestos desde el bucle de eventos (timeout ~16 ms,
/// `Reader::tick`): dispara el tap de página diferido cuando expira la
/// ventana de doble-tap (`DOUBLE_TAP_MS`) sin un segundo down. Si el tap se
/// confirmó como segundo tap (selección en curso), `pending_tap` ya fue
/// tomado por el Down y aquí no hay nada que hacer.
pub(crate) fn tick_gestures(reader: &mut Reader, app: &AndroidApp) {
    if let Some(p) = reader.gesture.pending_tap.take() {
        if p.at.elapsed() >= DOUBLE_TAP_MS {
            fire_tap_action(reader, app, p.x, p.y);
        } else {
            reader.gesture.pending_tap = Some(p);
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
            // Primer dedo: arranca un posible tap (página, indicador o
            // sheet), salvo que sea el SEGUNDO down de un doble-tap (tap
            // previo en el mismo sitio dentro de DOUBLE_TAP_MS): entonces
            // empieza la selección por arrastre (ancla = punto del doble-tap;
            // el rect se materializa al moverse > SELECT_SLOP).
            reader.gesture.pointers = pts;
            if let Some(&(_, x, y)) = reader.gesture.pointers.first() {
                let is_double_tap = match reader.gesture.pending_tap.take() {
                    Some(p) if p.at.elapsed() < DOUBLE_TAP_MS => {
                        let moved = ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt();
                        moved <= TAP_SLOP
                    }
                    Some(p) => {
                        // Fuera de la ventana o en otro sitio: el tap previo
                        // era un tap simple y se ejecuta ahora (no llegó a
                        // expirar en `tick_gestures`).
                        fire_tap_action(reader, app, p.x, p.y);
                        false
                    }
                    None => false,
                };
                if is_double_tap {
                    // Descartar selección/menú anterior y empezar el gesto de
                    // selección (un doble-tap no dispara el tap de página).
                    // También se cierra el panel de IA si estaba abierto: una
                    // selección nueva implica una consulta nueva (y evita que
                    // el panel viejo tape el nuevo rect/menú).
                    reader.clear_selection();
                    reader.close_ai_panel();
                    reader.gesture.kind = GestureKind::Selecting { anchor: (x, y) };
                } else {
                    reader.gesture.kind = GestureKind::Tap {
                        start_x: x,
                        start_y: y,
                    };
                }
            }
        }
        MotionAction::PointerDown => {
            reader.gesture.pointers = pts;
            // Segundo dedo: pinch. Distancia inicial = base del factor de
            // zoom; el centro del pinch (punto medio de los dedos) se fija
            // como ancla del zoom (`begin_pinch`): el punto de documento bajo
            // los dedos permanece fijo en pantalla durante el gesto.
            if reader.gesture.pointers.len() >= 2 {
                // Segundo dedo durante la selección: se cancela la selección
                // en curso (no fijada) y se pasa al pinch.
                if matches!(reader.gesture.kind, GestureKind::Selecting { .. }) {
                    reader.clear_selection();
                }
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
                GestureKind::Selecting { anchor } if reader.gesture.pointers.len() == 1 => {
                    // Arrastre de selección: al superar SELECT_SLOP desde el
                    // ancla (punto del doble-tap) se materializa el rect
                    // (`begin_sel`); después el rect sigue al dedo
                    // (`update_sel`, blit directo de la página cacheada).
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    let moved = ((cx - anchor.0).powi(2) + (cy - anchor.1).powi(2)).sqrt();
                    if reader.sel.is_none() && moved > SELECT_SLOP {
                        reader.begin_sel(anchor.0, anchor.1);
                    }
                    if reader.sel.is_some() {
                        reader.update_sel(cx, cy);
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
                            if reader.sel_menu.is_some() {
                                // Menú de selección abierto: el tap va al menú
                                // (botón o cerrar/descartar la selección);
                                // NUNCA cambia de página (el tap izq/der no
                                // se dispara mientras hay selección).
                                sel_menu_tap(reader, app, x, y);
                            } else if reader.sheet_progress > 0.0 {
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
                                // Tap de página en el área del documento: se
                                // DIFIERE para detectar el doble-tap de
                                // selección. Si llega un segundo down en el
                                // mismo sitio y dentro de DOUBLE_TAP_MS,
                                // empieza la selección y el tap NUNCA se
                                // dispara; si la ventana expira,
                                // `tick_gestures` ejecuta el cambio de
                                // página (coste: el tap simple espera
                                // ~300 ms, el precio estándar del doble-tap).
                                reader.gesture.pending_tap = Some(TapPending {
                                    x,
                                    y,
                                    at: Instant::now(),
                                });
                            }
                        }
                    }
                }
                GestureKind::Pull { .. } => {
                    // Fin del arrastre del sheet: animar hasta el objetivo
                    // más cercano (abierto si pasó de la mitad).
                    reader.end_sheet_drag();
                }
                GestureKind::Selecting { .. } => {
                    // Fin del arrastre de selección: fija la selección y abre
                    // el menú Copiar/Subrayar/IA (un doble-tap sin arrastre
                    // no fija nada: `end_sel` descarta los rects degenerados).
                    reader.end_sel();
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
            reader.gesture.pending_tap = None;
            reader.clear_selection();
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
