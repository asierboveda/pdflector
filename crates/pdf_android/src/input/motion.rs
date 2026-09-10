// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Procesamiento de `MotionEvent`s (extraído de `input.rs`, 2026-09-06):
//! `tick_gestures` (long-press quieto → modo selección), `handle_motion`
//! (máquina de gestos del visor: tap/pull/pinch/pan, trazo de herramienta y
//! borrado con stylus) y el input de las LISTAS — `list_tap`, `library_tap`,
//! `picker_tap`, `library_down_zone` y `handle_picker_motion` (scroll y taps
//! del picker interno y de la biblioteca MediaStore). El estado que consume
//! (`GestureState`/`GestureKind`) vive en `gestos`; el re-escalado de
//! timestamps del stylus (`gesture_ms`), en `stylus`.

use super::gestos::{GestureKind, LONG_PRESS_MS, TwoFingerMode, fire_tap_action};
use super::stylus::gesture_ms;
use crate::annotations::{PEN_BTN_ERASE, PEN_BTN_MODE, PenMode, ToolKind};
use crate::draw::{SettingsMenuItem, ViewMenuItem, settings_menu_geometry, view_menu_geometry};
use crate::jni::launch_all_files_settings;
use crate::reader::discover_categories::ARXIV_CATEGORIES;
use crate::reader::discover_state::{DiscoverScreen, DiscoverTab};
use crate::reader::{
    BookStatus, LibSort, LibraryCoverFit, LibraryGroupBy, LibraryViewMode, ListDrag, PickRow,
    PickerKind, Reader, UiMode, disc_card_action_rect, disc_card_gap, disc_card_h, disc_card_rect,
    disc_cat_row_rect, disc_content_y0, disc_detail_back_rect, disc_detail_layout,
    disc_more_btn_rect, disc_search_rect, disc_subtabs_rect, grid_cell_h, grid_cell_w, grid_gap,
    grid_pad, lib_add_btn_w, lib_chip_h, lib_chips, lib_cont_block_h, lib_cont_card_w,
    lib_cont_gap, lib_content_y0, lib_empty_state_geom, lib_grid_y0, lib_header_h, lib_org_block_h,
    lib_org_chip_h, lib_org_chips, lib_search_chips_y0, lib_search_h, lib_search_panel_h,
    lib_section_title_h, lib_tabs_rect, list_row_gap, list_row_h, picker_btn_w, picker_header_h,
    picker_row_h, settings_menu_button_rect, view_menu_button_rect,
};
use crate::{PINCH_MAX, PINCH_MIN, SELECT_SLOP, TAP_SLOP};
use android_activity::AndroidApp;
use android_activity::input::{Button, ButtonState, MotionAction};
use std::time::Instant;

/// Avanza la máquina de gestos desde el bucle de eventos (timeout ~16 ms,
/// `Reader::tick`): detecta el LONG-PRESS — si el dedo lleva quieto en `Tap`
/// (sin moverse más de `TAP_SLOP`, con el sheet cerrado) más de
/// `LONG_PRESS_MS`, entra en MODO SELECCIÓN: fija el ancla en el punto del
/// dedo y materializa el rect como PUNTO (`begin_sel`); el tap de página
/// NUNCA se disparará para este dedo (el tap solo se dispara en un down+up
/// rápido, sin long-press). El temporizador se desarma al moverse, al entrar
/// en el pinch o al levantar.
pub(crate) fn tick_gestures(reader: &mut Reader, _app: &AndroidApp) {
    // Con una herramienta de anotación activa el long-press NO entra en modo
    // selección: el dedo es tinta/resaltador. (El gesto de herramienta no
    // necesita tick: el trazo avanza con los Moves.)
    if reader.tool != ToolKind::Navigate {
        return;
    }
    if !matches!(reader.gesture.kind, GestureKind::Tap { .. })
        || reader.sheet_progress > 0.0
        || reader.gesture.pointers.len() != 1
    {
        return;
    }
    let Some(at) = reader.gesture.press_at else {
        return;
    };
    if at.elapsed() < LONG_PRESS_MS {
        return;
    }
    reader.gesture.press_at = None;
    let Some(&(_, ax, ay)) = reader.gesture.pointers.first() else {
        return;
    };
    // Nueva selección: descartar la anterior (y su menú) y cerrar el panel de
    // IA si estaba abierto (una selección nueva implica una consulta nueva y
    // evita que el panel viejo tape el nuevo rect/menú).
    reader.clear_selection();
    reader.close_ai_panel();
    reader.gesture.kind = GestureKind::Selecting { anchor: (ax, ay) };
    // Materializa el rect como PUNTO (ancla = actual): feedback visual de que
    // el long-press entró en modo selección; el rect crece al arrastrar.
    reader.begin_sel(ax, ay);
}

/// Empieza el gesto de pinch con los punteros actuales (≥ 2): fija el ancla
/// (centro de los dedos), el zoom y el pan de partida en `Reader` y marca el
/// gesto con la distancia inicial (base del factor RELATIVO del zoom).
///
/// BUG arreglado (antes el código exigía `distancia > 8 px` para empezar el
/// pinch): con los dedos a ≤ 8 px el gesto se quedaba en `Tap` con 2
/// punteros, ningún Move actuaba (el arm de `Tap` exige 1 puntero) y al
/// levantar los dedos se disparaba un CAMBIO DE PÁGINA (el pinch "se
/// bugeaba"). Ahora el pinch empieza SIEMPRE con el segundo dedo; `start_dist`
/// nunca es 0 (mínimo 1 px), así que un toque con los dedos casi juntos no
/// divide por cero en el Move y, si no hay separación, `set_zoom_sharp`
/// resulta un no-op (zoom sin cambios).
fn begin_pinch_gesture(reader: &mut Reader, pts: &[(i32, f32, f32)]) {
    let (_, ax, ay) = pts[0];
    let (_, bx, by) = pts[1];
    let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
    reader.begin_pinch((ax + bx) / 2.0, (ay + by) / 2.0);
    // El long-press muere al pasar al pinch (un dedo solo no entra en
    // selección si un segundo dedo cae durante la espera).
    reader.gesture.press_at = None;
    let mid = ((ax + bx) / 2.0, (ay + by) / 2.0);
    reader.gesture.kind = GestureKind::Pinch {
        start_dist: d.max(1.0),
        start_zoom: reader.zoom,
        start_mid: mid,
        prev_mid: mid,
        prev_a: (ax, ay),
        prev_b: (bx, by),
        mode: TwoFingerMode::Undecided,
    };
}

/// Procesa un `MotionEvent` del VISOR: actualiza la máquina de gestos y actúa
/// sobre el reader. En modo picker/biblioteca se delega en
/// `handle_picker_motion` (arrastre + tap de lista, sin pinch).
///
/// Gestos del visor (página a página):
/// - tap en la mitad derecha = página siguiente; izquierda = anterior
///   (con el sheet visible, el tap cierra el panel o pulsa un botón);
/// - tap en la barra superior del chrome = abrir/cerrar el sheet de
///   ajustes (el pull-down se eliminó, 2026-09-03);
/// - arrastre vertical con el sheet visible = moverlo (arriba cierra);
/// - pinch con dos dedos = zoom (factor relativo + anclado al centro del
///   pinch);
/// - mantener un dedo quieto durante `LONG_PRESS_MS` = entrar en MODO
///   SELECCIÓN (ancla en el punto del dedo; arrastrar extiende el rect,
///   soltar fija y abre el menú Copiar/Subrayar/IA; sin arrastre se
///   descarta);
/// - un dedo que se desliza más de `TAP_SLOP` cancela el tap (sin scroll:
///   el arrastre se eliminó por decisión del autor).
///
/// Botones del boli en un MotionEvent (state=botones pulsados, action=boton del evento).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PenButtons {
    pub(crate) state: ButtonState,
    pub(crate) action: Button,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
    up_idx: Option<usize>,
    stylus: bool,
    buttons: PenButtons,
    event_time: i64,
    stylus_pressure: f32,
) {
    // FASE A — CALIBRACIÓN DE BOTONES DEL BOLI (ver CHANGELOG 2026-08-25):
    // este boli reporta los bits estándar (0x20 primary / 0x40 secondary),
    // verificados en aire y contacto. Log a debug! para diagnóstico futuro.
    if stylus
        && matches!(
            action,
            MotionAction::Down | MotionAction::Move | MotionAction::Up
        )
    {
        log::debug!(
            "pen_buttons: {action:?} stylus buttons=0x{:x}",
            buttons.state.0
        );
    }
    if matches!(
        action,
        MotionAction::ButtonPress | MotionAction::ButtonRelease
    ) {
        log::debug!(
            "pen_buttons: {action:?} action_button={:?} (0x{:x}) ptr={}",
            buttons.action,
            u32::from(buttons.action),
            pts.len()
        );
    }
    if reader.mode == UiMode::Picker
        || reader.mode == UiMode::Library
        || reader.mode == UiMode::Discover
    {
        handle_picker_motion(reader, app, action, pts, up_idx);
        return;
    }
    match action {
        MotionAction::Down => {
            // Primer dedo: arranca un posible TAP (página, indicador o
            // sheet) y arma el temporizador de LONG-PRESS (`press_at`, que
            // `tick_gestures` convierte en modo selección si el dedo se queda
            // quieto `LONG_PRESS_MS`). El tap es INMEDIATO (sin ventana de
            // doble-tap): el long-press y el tap no compiten — el long-press
            // solo entra si el dedo NO se levanta antes de `LONG_PRESS_MS` y
            // NO se mueve más de `TAP_SLOP`.
            reader.gesture.pointers = pts;
            // CONTROL TOTAL CON EL BOLI (sin menús): el Down del STYLUS sobre
            // la página (fuera del chrome de la UI) o dibuja (Ink/Highlight
            // según el modo persistido del boli, SIEMPRE activo) o BORRA si
            // trae el botón DOWN pulsado. El dedo sigue navegando igual
            // (tap/pinch/pan); los gestos existentes no se rompen.
            if reader.gesture.pointers.len() == 1
                && let Some(&(_, x, y)) = reader.gesture.pointers.first()
            {
                // SEPARACIÓN DEDO/STYLUS: solo el lápiz dibuja/borra; los
                // dedos (y la palma) navegan (pan/pinch).
                if stylus {
                    // El modo ERASE nunca coexiste con un gesto de tinta en
                    // curso: si hay trazo, este Down no hace nada (el trazo
                    // actual termina como estaba).
                    if reader.tool_gesture.is_some() {
                        return;
                    }
                    if buttons.state.0 & PEN_BTN_ERASE.0 != 0 {
                        // [C] BORRAR: botón DOWN mantenido + tocar el PDF.
                        if reader.begin_erase_gesture(x, y) {
                            reader.gesture.kind = GestureKind::Erase;
                            return; // borrado: sin tap ni long-press
                        }
                    } else {
                        // [A] Dibujar SIEMPRE, según el modo persistido del
                        // boli (sin depender de la barra de herramientas).
                        let mode_tool = match reader.pen_mode {
                            PenMode::Ink => ToolKind::Ink,
                            PenMode::Highlight => ToolKind::Highlight,
                        };
                        reader.begin_tool_gesture(x, y, mode_tool);
                        if reader.tool_gesture.is_some() {
                            reader.gesture.kind = GestureKind::ToolDrawing;
                            return; // gesto de herramienta: sin tap ni long-press
                        }
                    }
                    // Si no arrancó gesto (p. ej. fuera de la página), el
                    // Down sigue como tap normal.
                } else if reader.tool != ToolKind::Navigate {
                    // Dedo con herramienta ACTIVA (barra): modo mano — pan 1
                    // dedo (el pinch 2 dedos lo convierte el PointerDown).
                    // Con la barra cerrada (tool == Navigate) el dedo cae al
                    // TAP normal (página) — el boli controla la anotación.
                    // Palm rejection por tiempo: tras escribir con stylus, se
                    // ignora el táctil un margen (evita pans/zooms de la palma).
                    if reader.should_ignore_touch() {
                        return;
                    }
                    reader.gesture.kind = GestureKind::Pan {
                        start: (x, y),
                        pan0: reader.begin_pan(),
                    };
                    return;
                }
            }
            // Defensa: si los DOS dedos llegan en un único ACTION_DOWN
            // (algunos dispositivos/API los entregan juntos, sin PointerDown
            // posterior), empezar el pinch directamente — un "Tap" con 2
            // punteros no coincide con ningún gesto de Move y al levantar
            // dispararía un cambio de página (el bug del pinch).
            if reader.gesture.pointers.len() >= 2 {
                begin_pinch_gesture(reader, &reader.gesture.pointers.clone());
            } else if let Some(&(_, x, y)) = reader.gesture.pointers.first() {
                reader.gesture.kind = GestureKind::Tap {
                    start_x: x,
                    start_y: y,
                };
                reader.gesture.press_at = Some(Instant::now());
            }
        }
        MotionAction::PointerDown => {
            reader.gesture.pointers = pts;
            // Palm rejection por tiempo: tras escribir con stylus, se ignora
            // el táctil un margen (evita pinch/pan de la palma).
            if reader.should_ignore_touch() {
                return;
            }
            // PALM REJECTION mientras se dibuja/borra con el STYLUS: si la
            // mano u otro dedo toca durante un trazo del lápiz, ese segundo
            // puntero NO es un pinch — se IGNORA por completo (el trazo
            // sigue; nada de zoom/reescala). Arregla el "parpadeo" al escribir
            // sobre trazos existentes: al apoyar la palma al soltar, el código
            // convertía el gesto en pinch y la página reescalaba de golpe.
            if matches!(
                reader.gesture.kind,
                GestureKind::ToolDrawing | GestureKind::Erase
            ) && stylus
            {
                return;
            }
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
                // Segundo dedo durante un gesto de herramienta: se descarta
                // el trazo en curso (no se crea anotación) y se pasa al
                // pinch — la herramienta sigue activa para el siguiente Down.
                if matches!(reader.gesture.kind, GestureKind::ToolDrawing) {
                    reader.cancel_tool_gesture();
                }
                // Durante el BORRADO el segundo puntero no es un pinch: se
                // ignora (el borrado continúa; ver palm rejection arriba).
                if matches!(reader.gesture.kind, GestureKind::Erase) {
                    return;
                }
                begin_pinch_gesture(reader, &reader.gesture.pointers.clone());
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
                        // El dedo se movió: el long-press muere (se exige un
                        // dedo quieto) y el gesto pasa a arrastre del sheet
                        let (dx, dy) = (cx - start_x, cy - start_y);
                        let sheet_visible = reader.sheet_progress > 0.0;
                        // ¿Arrastre del sheet? Solo con el sheet YA visible:
                        // moverlo (bajar = mantener/abrir, subir = cerrar).
                        // El pull-down con el sheet cerrado se eliminó: los
                        // ajustes se abren con tap en la barra superior.
                        let pull = sheet_visible && dy.abs() > dx.abs();
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
                    mut start_dist,
                    mut start_zoom,
                    start_mid,
                    prev_mid,
                    prev_a,
                    prev_b,
                    mut mode,
                } if reader.gesture.pointers.len() >= 2 => {
                    let (_, ax, ay) = reader.gesture.pointers[0];
                    let (_, bx, by) = reader.gesture.pointers[1];
                    let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                    if d > 1.0 {
                        let mid = ((ax + bx) / 2.0, (ay + by) / 2.0);
                        let d_mid_x = mid.0 - prev_mid.0;
                        let d_mid_y = mid.1 - prev_mid.1;
                        let mid_disp = (d_mid_x.powi(2) + d_mid_y.powi(2)).sqrt();

                        let cum_mid_x = mid.0 - start_mid.0;
                        let cum_mid_y = mid.1 - start_mid.1;
                        let cum_mid_disp = (cum_mid_x.powi(2) + cum_mid_y.powi(2)).sqrt();

                        let va_x = ax - prev_a.0;
                        let va_y = ay - prev_a.1;
                        let va_len = (va_x.powi(2) + va_y.powi(2)).sqrt();

                        let vb_x = bx - prev_b.0;
                        let vb_y = by - prev_b.1;
                        let vb_len = (vb_x.powi(2) + vb_y.powi(2)).sqrt();

                        let dot = va_x * vb_x + va_y * vb_y;

                        let prev_d = ((prev_a.0 - prev_b.0).powi(2)
                            + (prev_a.1 - prev_b.1).powi(2))
                        .sqrt()
                        .max(1.0);
                        let frame_delta_d = (d - prev_d).abs();

                        let cum_delta_d = (d - start_dist).abs();
                        let rel_cum_delta = cum_delta_d / start_dist.max(1.0);

                        let kin = TwoFingerKinematics {
                            dot,
                            mid_disp,
                            cum_mid_disp,
                            cum_delta_d,
                            rel_cum_delta,
                            frame_delta_d,
                            va_len,
                            vb_len,
                        };
                        let new_mode = discriminate_two_finger_mode(mode, &kin);

                        if mode == TwoFingerMode::Pan && new_mode == TwoFingerMode::Zoom {
                            // Al pasar de Pan a Zoom (usuario frenó la traslación y abrió/cerró dedos),
                            // re-anclamos la distancia de partida al estado actual para que el zoom
                            // arranque suavemente desde 1.0x sin salto brusco.
                            start_dist = d;
                            start_zoom = reader.zoom;
                        }
                        mode = new_mode;
                        match mode {
                            TwoFingerMode::Undecided | TwoFingerMode::Pan => {
                                // Traslación pura con dos dedos (zoom congelado).
                                reader.pan_by(d_mid_x, d_mid_y);
                            }
                            TwoFingerMode::Zoom => {
                                // Zoom activo y fluido (1:1 relativo a la distancia de partida).
                                reader.begin_pinch(mid.0, mid.1);
                                let zoom =
                                    (start_zoom * d / start_dist).clamp(PINCH_MIN, PINCH_MAX);
                                reader.set_zoom_fast(zoom);
                            }
                        }

                        reader.gesture.kind = GestureKind::Pinch {
                            start_dist,
                            start_zoom,
                            start_mid,
                            prev_mid: mid,
                            prev_a: (ax, ay),
                            prev_b: (bx, by),
                            mode,
                        };
                    }
                }
                GestureKind::Selecting { anchor } if reader.gesture.pointers.len() == 1 => {
                    // Arrastre de selección: al superar SELECT_SLOP desde el
                    // ancla (punto del long-press) el rect sigue al dedo
                    // (`update_sel`, blit directo de la página cacheada). El
                    // rect ya se materializó como punto al entrar en el modo
                    // (`begin_sel` en `tick_gestures`); los micro-drags (<
                    // SELECT_SLOP) no extienden la selección (como antes).
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    let moved = ((cx - anchor.0).powi(2) + (cy - anchor.1).powi(2)).sqrt();
                    if moved > SELECT_SLOP {
                        reader.update_sel(cx, cy);
                    }
                }
                GestureKind::ToolDrawing if reader.gesture.pointers.len() == 1 => {
                    // Arrastre de herramienta (boli/resaltador): cada Move
                    // añade el punto o extiende el rect y re-blitea con el
                    // frame compuesto + la capa temporal del trazo (la página
                    // NO se re-blitea por evento — requisito 5).
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    let t0 = reader.gesture_t0_ns;
                    let t_ms = if stylus {
                        gesture_ms(event_time, t0)
                    } else {
                        0.0
                    };
                    reader.update_tool_gesture(cx, cy, t_ms, stylus_pressure);
                }
                GestureKind::Erase if reader.gesture.pointers.len() == 1 => {
                    // Arrastre de BORRADO: hit-test del punto y eliminación
                    // en vivo (la anotación desaparece bajo el boli).
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    reader.update_erase_gesture(cx, cy);
                }
                GestureKind::Pan { start, pan0 } if reader.gesture.pointers.len() == 1 => {
                    // Dedo con herramienta activa: mover el documento (pan).
                    let (_, cx, cy) = reader.gesture.pointers[0];
                    reader.set_pan(pan0.0 + (cx - start.0), pan0.1 + (cy - start.1));
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
            // El dedo se levantó: el temporizador de long-press se desarma
            // (si el long-press ya disparó, `press_at` ya es None).
            reader.gesture.press_at = None;
            match kind {
                GestureKind::Tap { start_x, start_y } => {
                    // Sin movimiento relevante y SIN long-press (el dedo se
                    // levantó antes de LONG_PRESS_MS) → TAP INMEDIATO: la
                    // acción se dispara aquí mismo, sin diferir. Un long-press
                    // habría cambiado el gesto a Selecting (`tick_gestures`),
                    // así que un tap nunca dispara la selección.
                    if let Some((_, x, y)) = up {
                        let moved = ((x - start_x).powi(2) + (y - start_y).powi(2)).sqrt();
                        if moved <= TAP_SLOP {
                            fire_tap_action(reader, app, x, y);
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
                    // el menú Copiar/Subrayar/IA (un long-press sin arrastre
                    // no fija nada: `end_sel` descarta los rects degenerados).
                    reader.end_sel();
                }
                GestureKind::ToolDrawing => {
                    // Fin del gesto de herramienta: convierte el trazo en una
                    // anotación guardada (curva midpoint muestreada /
                    // resaltador alineado al texto; un toque sin arrastre se
                    // descarta). La posición del Up cierra el remate
                    // M_last→P_up (el drain de history anterior ya estampó
                    // las muestras intermedias).
                    if let Some((_, ux, uy)) = up {
                        reader.end_tool_gesture(ux, uy);
                    }
                }
                GestureKind::Erase => {
                    // Fin del borrado: persiste UNA vez si algo se eliminó.
                    reader.end_erase_gesture();
                }
                GestureKind::Pan { .. } => {
                    // Fin del pan con dedo: no hay nada que asentar (el pan
                    // ya quedó aplicado en cada Move).
                }
                GestureKind::Pinch { .. } => {
                    // Defensa: si los DOS dedos se levantan en un único
                    // ACTION_UP (sin PointerUp previo), el pinch termina aquí
                    // y el zoom fast se quedaría sin re-render nítido (vista
                    // borrosa con el bitmap viejo escalado): asentar el render
                    // igual que hace `PointerUp`.
                    reader.set_zoom_sharp(reader.zoom);
                }
                GestureKind::None => {}
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
            // Un Cancel (p. ej. el sistema roba el gesto) también termina el
            // pinch: sin esto el zoom fast quedaba sin re-render nítido y la
            // vista se quedaba con el bitmap viejo escalado (borroso) hasta
            // el siguiente pinch o cambio de página.
            let pinch_active = matches!(reader.gesture.kind, GestureKind::Pinch { .. });
            let erasing = matches!(reader.gesture.kind, GestureKind::Erase);
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
            reader.gesture.press_at = None;
            reader.clear_selection();
            if erasing {
                // El borrado a medio terminar se persiste igual (la memoria
                // manda: las anotaciones ya eliminadas no vuelven).
                reader.end_erase_gesture();
            } else {
                reader.cancel_tool_gesture(); // el trazo en curso se descarta
            }
            if pinch_active {
                reader.set_zoom_sharp(reader.zoom);
            }
        }
        MotionAction::ButtonPress => {
            // [B] Botón UP del boli: alterna el modo (funciona TAMBIÉN con
            // el boli en el AIRE: ButtonPress llega sin contacto). Fuente de
            // verdad de la calibración: `action_button()` en
            // Press/Release.
            if u32::from(buttons.action) == PEN_BTN_MODE.0 {
                reader.toggle_pen_mode();
            }
        }
        MotionAction::ButtonRelease => {
            // Sin acción: el toggle se decide en el Press (un Press+Release
            // no debe alternar dos veces).
        }
        _ => {} // HoverMove, Scroll, Outside, ...: sin gesto definido.
    }
}

/// Tap sobre la lista activa (picker interno o biblioteca MediaStore).
fn list_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    match reader.mode {
        UiMode::Picker => picker_tap(reader, app, x, y),
        UiMode::Library => library_tap(reader, app, x, y),
        UiMode::Discover => discover_tap(reader, app, x, y),
        UiMode::Viewer => {}
    }
}

/// Tap de la biblioteca (biblioteca personal premium): botón "＋ Add book"
/// de la cabecera, campo de búsqueda (toggle del panel de chips + "✕"),
/// chips del panel de búsqueda (fila 0 = letras A-Z/#, fila 1 = carpetas),
/// tarjeta del carousel de Continue Reading (abre el libro en su página
/// guardada), chips de organización (sort/filter) o celda de la rejilla
/// (abre el libro). La geometría DEBE reflejar exactamente la de
/// `render_library_zone` (mismas fórmulas: `lib_chips`, `lib_content_y0`,
/// `lib_cont_block_h`, `lib_grid_cell_rect`, `lib_org_chips`).
fn library_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    // Pestaña Descubrir en la cabecera
    let (_tab_lib, tab_disc) = lib_tabs_rect(reader.win_w, reader.win_h);
    if x >= tab_disc.0 && x <= tab_disc.2 && y >= tab_disc.1 && y <= tab_disc.3 {
        reader.enter_discover(app);
        return;
    }

    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;

    let (vl, vt, vr, vb) = view_menu_button_rect(reader.win_w, reader.win_h);
    let (sl, st, sr, sb) = settings_menu_button_rect(reader.win_w, reader.win_h);
    let hit_view = x >= vl && x < vr && y >= vt && y < vb;
    let hit_settings = x >= sl && x < sr && y >= st && y < sb;

    // ViewMenu dropdown abierto: procesar tap en ítems o cerrar
    if reader.view_menu_open {
        let (_card_rect, items) = view_menu_geometry(reader.win_w, reader.win_h);
        let mut handled = false;
        for (item, rect) in items {
            if x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3 {
                match item {
                    ViewMenuItem::Grid => {
                        reader.view_mode = LibraryViewMode::Grid;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::List => {
                        reader.view_mode = LibraryViewMode::List;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::ColumnsAuto => {
                        reader.auto_columns = !reader.auto_columns;
                        reader.save_state();
                    }
                    ViewMenuItem::ColumnsDec => {
                        reader.auto_columns = false;
                        reader.columns = reader.columns.saturating_sub(1).clamp(1, 4);
                        reader.save_state();
                    }
                    ViewMenuItem::ColumnsInc => {
                        reader.auto_columns = false;
                        reader.columns = (reader.columns + 1).clamp(1, 4);
                        reader.save_state();
                    }
                    ViewMenuItem::CoverCrop => {
                        reader.cover_fit = LibraryCoverFit::Crop;
                        reader.hide_covers = false;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::CoverFit => {
                        reader.cover_fit = LibraryCoverFit::Fit;
                        reader.hide_covers = false;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::CoverHide => {
                        reader.hide_covers = !reader.hide_covers;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::RecentShelf => {
                        reader.recent_shelf_enabled = !reader.recent_shelf_enabled;
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::GroupNone => {
                        reader.group_by = LibraryGroupBy::None;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::GroupAuthor => {
                        reader.group_by = LibraryGroupBy::Author;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortTitle => {
                        reader.library.lib_sort = LibSort::Title;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAuthor => {
                        reader.library.lib_sort = LibSort::Author;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAdded => {
                        reader.library.lib_sort = LibSort::RecentlyAdded;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortRead => {
                        reader.library.lib_sort = LibSort::RecentlyRead;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortProgress => {
                        reader.library.lib_sort = LibSort::Progress;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                }
                handled = true;
                break;
            }
        }
        if handled {
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        if hit_view {
            reader.view_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        reader.view_menu_open = false;
        reader.list_dirty = true;
        reader.redraw();
        return;
    }

    // SettingsMenu dropdown abierto: procesar tap en ítems o cerrar
    if reader.settings_menu_open {
        let (_card_rect, items) = settings_menu_geometry(reader.win_w, reader.win_h);
        let mut handled = false;
        for (item, rect) in items {
            if x >= rect.0 && x < rect.2 && y >= rect.1 && y < rect.3 {
                match item {
                    SettingsMenuItem::RecentShelf => {
                        reader.recent_shelf_enabled = !reader.recent_shelf_enabled;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverSizeSmall => {
                        reader.cover_size = 0;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverSizeMedium => {
                        reader.cover_size = 1;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverSizeLarge => {
                        reader.cover_size = 2;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::CoverProgress => {
                        reader.cover_progress = !reader.cover_progress;
                        reader.save_state();
                        reader.settings_menu_open = false;
                    }
                    SettingsMenuItem::ClearLibrary => {
                        let now = Instant::now();
                        if let Some(until) = reader.clear_confirm_until
                            && now <= until
                        {
                            reader.clear_confirm_until = None;
                            reader.clear_library(app);
                            return;
                        }
                        reader.clear_confirm_until = Some(now + std::time::Duration::from_secs(3));
                        reader.list_dirty = true;
                        reader.redraw();
                        return;
                    }
                }
                handled = true;
                break;
            }
        }
        if handled {
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        if hit_settings {
            reader.settings_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        reader.settings_menu_open = false;
        reader.list_dirty = true;
        reader.redraw();
        return;
    }
    if y < header_h {
        // "⋯" View: alterna su dropdown y cierra el de Settings.
        if hit_view {
            reader.view_menu_open = !reader.view_menu_open;
            reader.settings_menu_open = false;
            reader.list_dirty = true; // re-render de la cabecera (highlight)
            reader.redraw();
            return;
        }
        // "☰" Settings: alterna su dropdown y cierra el de View.
        if hit_settings {
            reader.settings_menu_open = !reader.settings_menu_open;
            reader.view_menu_open = false;
            reader.list_dirty = true;
            reader.redraw();
            return;
        }
        let pad = grid_pad(reader.win_w);
        let top_pad = 36.0f32;
        let btn_w = lib_add_btn_w(reader.win_w);
        let btn_h = ((header_h - top_pad) * 0.52).clamp(34.0, 46.0);
        let btn_y = top_pad + (header_h - top_pad - btn_h) / 2.0;
        let btn_x = reader.win_w as f32 - pad - btn_w;
        if x >= btn_x && x < btn_x + btn_w && y >= btn_y && y < btn_y + btn_h {
            reader.add_book(app);
        }
        return;
    }

    // CAMPO de búsqueda: la "✕" limpia el texto tecleado y cierra el
    // teclado; tocar el campo abre el TECLADO del sistema (`jni::ime_*`).
    if y < search_y + search_hh {
        let field_right = reader.win_w as f32 - grid_pad(reader.win_w);
        let has_filter = !reader.library.lib_query.is_empty()
            || reader.library.lib_letter.is_some()
            || reader.library.lib_folder.is_some();
        if has_filter {
            let xw = search_hh - 8.0;
            let xx = field_right - 14.0 - xw;
            if x >= xx && x < xx + xw {
                reader.lib_clear_search(app);
                return;
            }
        }
        reader.lib_open_keyboard(app);
        return;
    }

    // PANEL de búsqueda desplegado: fila 0 = letras A-Z/#, fila 1 = carpetas.
    // La zona usa la MISMA geometría que el render (`lib_search_chips_y0/1`,
    // donde `lib_chips` coloca los chips) y que `lib_down_zone`; antes usaba
    // `panel_top = header_h + search_h` (6 px por encima de la fila real), de
    // modo que un tap en el borde superior del panel caía fuera de los chips.
    let panel_top = lib_search_chips_y0(reader);
    let panel_h = lib_search_panel_h(reader.win_h, reader.library.lib_search_open);
    if reader.library.lib_search_open && y >= panel_top && y < panel_top + panel_h {
        let row = if y < lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) {
            0
        } else {
            1
        };
        for (label, (l, t, r, b), _active) in lib_chips(reader, row) {
            if x >= l && x < r && y >= t && y < b {
                if row == 0 {
                    if label == "All" {
                        reader.lib_set_letter(None);
                    } else {
                        reader.lib_set_letter(label.chars().next());
                    }
                } else if label == "All" {
                    reader.lib_set_folder(None);
                } else {
                    reader.lib_set_folder(Some(label.clone()));
                }
                return;
            }
        }
        return;
    }

    // Franja de estado: no es seleccionable.
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    if y < content_y0 {
        return;
    }

    // Contenido scrolleable: pasar a coordenadas de CONTENIDO (y del Down +
    // scroll vertical).
    let yc = y - content_y0 + reader.library.lib_scroll;
    let win_w = reader.win_w;
    // Biblioteca minimalista: la sección Continue Reading está oculta (siempre
    // `false`); el bloque de organización tampoco existe (rejilla directa).
    let has_cont = reader.lib_has_cont();

    // EMPTY STATE: botón "Add PDF"/"Grant access" (misma geometría que el
    // render).
    if reader.library_list.is_empty() {
        if let Some(g) = lib_empty_state_geom(reader) {
            let (l, t, r, b) = g.button;
            if x >= l && x < r && y >= t && y < b {
                if reader.permission_granted {
                    reader.add_book(app);
                } else {
                    reader.grant_pending = true;
                    launch_all_files_settings(app);
                }
            }
        }
        return;
    }

    // CONTINUE READING: tap en cualquier punto de la tarjeta (portada o
    // texto, incluido el botón "Read") abre el libro en su página guardada.
    let cont_block_h = lib_cont_block_h(win_w, reader.win_h, has_cont);
    if yc < cont_block_h {
        if has_cont && yc >= lib_section_title_h(reader.win_h) {
            let cw = lib_cont_card_w(win_w, reader.win_h);
            let i = ((x - grid_pad(win_w) + reader.library.lib_carousel_x) / (cw + lib_cont_gap()))
                .floor();
            if i >= 0.0
                && let Some(book) = reader.lib_continue_reading().get(i as usize)
            {
                // Clonar ruta+nombre: `open_pdf_at` necesita &mut self.
                let path = book.path.clone();
                let name = book.name.clone();
                let start =
                    crate::persist::progress_for(&reader.library.lib_books, &path).map(|p| p.page);
                if !reader.open_pdf_at(&path, start) {
                    reader.status = Some(format!("Cannot open {name}"));
                    reader.list_dirty = true;
                    reader.redraw();
                }
            }
        }
        return;
    }

    // Título de "My Library": no seleccionable. Tras él, el bloque de
    // ORGANIZACIÓN (chips de sort/filter) antes de la rejilla.
    let grid_y0 = lib_grid_y0(win_w, reader.win_h, has_cont);
    if yc < grid_y0 {
        let org_top = grid_y0 - lib_org_block_h(reader.win_h);
        if yc >= org_top {
            let row = if yc < org_top + lib_org_chip_h(reader.win_h) {
                0
            } else {
                1
            };
            for (label, (l, t, r, b), _active) in lib_org_chips(reader, row) {
                if x >= l && x < r && y >= t && y < b {
                    if row == 0 {
                        reader.lib_set_sort(match label.as_str() {
                            "Recently Read" | "Leídos" => LibSort::RecentlyRead,
                            "Title" | "Título" => LibSort::Title,
                            "Author" | "Autor" => LibSort::Author,
                            _ => LibSort::RecentlyAdded,
                        });
                    } else {
                        reader.lib_set_status(match label.as_str() {
                            "Reading" | "En lectura" => Some(BookStatus::Reading),
                            "Finished" | "Terminados" => Some(BookStatus::Finished),
                            "Unread" | "Por leer" => Some(BookStatus::Unread),
                            _ => None,
                        });
                    }
                    return;
                }
            }
        }
        return;
    }

    if reader.is_grid() {
        let cols = reader.effective_grid_cols();
        let row = ((yc - grid_y0) / grid_cell_h(win_w, cols, reader.cover_size)) as usize;
        let cell_w = grid_cell_w(win_w, cols);
        let pad = grid_pad(win_w);
        let col = ((x - pad) / (cell_w + grid_gap(win_w))).floor() as usize;
        if col < cols
            && let Some(entry) = reader.grid_entry_at(row, col)
        {
            let entry = entry.clone();
            if !reader.open_library_entry(app, &entry) {
                reader.status = Some(format!("Cannot open {}", entry.name));
                reader.list_dirty = true;
                reader.redraw();
            }
        }
    } else {
        let pad = grid_pad(win_w);
        if x >= pad && x < win_w as f32 - pad {
            let row_h = list_row_h(reader.win_h, reader.cover_size);
            let row_gap = list_row_gap();
            let total_row_h = row_h + row_gap;
            let rel_y = yc - grid_y0;
            if rel_y >= 0.0 {
                let idx = (rel_y / total_row_h).floor() as usize;
                let in_row_y = rel_y - idx as f32 * total_row_h;
                if in_row_y <= row_h
                    && let Some(entry) = reader.list_entry_at(idx)
                {
                    let entry = entry.clone();
                    if !reader.open_library_entry(app, &entry) {
                        reader.status = Some(format!("Cannot open {}", entry.name));
                        reader.list_dirty = true;
                        reader.redraw();
                    }
                }
            }
        }
    }
}

/// Tap del picker: botones de la cabecera (Back/Rescan) o fila de la lista.
/// El fallback interno (`PickerKind::Files`) abre el PDF; el selector de
/// añadir (`PickerKind::Select`) lo CURA en la biblioteca (`add_selected`).
/// La geometría DEBE reflejar exactamente la de `render_picker_list` (mismas
/// fórmulas de layout).
fn picker_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let win_w = reader.win_w as f32;
    let row_h = picker_row_h(reader.win_h) as f32;
    let header_h = picker_header_h(reader.win_h) as f32;
    let status_h = if reader.status.is_some() { row_h } else { 0.0 };
    let btn_w = picker_btn_w(reader.win_w) as f32;
    let selecting = reader.picker_kind == PickerKind::Select;

    // Cabecera: botones a la derecha (Back a la izquierda de Rescan).
    if y < header_h {
        let rescan_x = win_w - btn_w - 8.0;
        if x >= rescan_x {
            if selecting {
                reader.rescan_select(app);
            } else {
                reader.rescan(app);
            }
            return;
        }
        let back_x = win_w - btn_w * 2.0 - 16.0;
        if (reader.doc.is_some() || selecting) && x >= back_x && x < rescan_x {
            if selecting {
                reader.cancel_add(app);
            } else {
                reader.exit_picker();
            }
            return;
        }
        return;
    }

    // Franja de estado (no seleccionable) + barra de breadcrumb del gestor.
    let crumbs = if reader.picker_has_crumb() {
        row_h
    } else {
        0.0
    };
    let rows_y0 = header_h + status_h + crumbs;
    if y < rows_y0 {
        // Tap en la barra de breadcrumb (dentro de una carpeta): subir un
        // nivel del gestor de archivos.
        if selecting && y >= header_h + status_h && reader.picker_has_crumb() {
            reader.picker_sel_up();
        }
        return;
    }

    let row = ((y - rows_y0) / row_h) as usize + reader.list_scroll;
    if selecting {
        // Gestor de archivos del selector: carpeta → entrar; PDF → curar.
        if let Some(pr) = reader.picker_rows().get(row) {
            match pr {
                PickRow::Folder(name) => {
                    let name = name.clone();
                    reader.picker_sel_enter(&name);
                }
                PickRow::File(idx) => reader.add_selected(app, *idx),
            }
        }
    } else if row < reader.picker_len() {
        let name = reader.pdf_list[row].name.clone();
        let path = reader.pdf_list[row].path.clone();
        if !reader.open_pdf(&path) {
            reader.status = Some(format!("Cannot open {name}"));
            reader.list_dirty = true;
            reader.redraw();
        }
    }
}

/// Zona de la biblioteca donde cayó el Down (qué arrastra en HORIZONTAL):
/// 0 = contenido (scroll vertical), 1 = carousel de Continue Reading, 2 =
/// fila de chips de LETRAS (búsqueda), 3 = fila de chips de CARPETAS
/// (búsqueda), 4 = fila de chips de SORT, 5 = fila de chips de FILTER.
/// Misma geometría que `library_tap` y `render_library_zone`.
fn library_down_zone(reader: &Reader, y: f32) -> u8 {
    let header_h = lib_header_h(reader.win_h);
    let search_h = lib_search_h();
    let search_y = header_h + 6.0;
    let search_hh = search_h - 12.0;
    if y < search_y + search_hh {
        return 0; // cabecera + campo de búsqueda: sin arrastre horizontal
    }
    // Panel de búsqueda desplegado: fila 0 = letras (2), fila 1 = carpetas (3).
    if reader.library.lib_search_open {
        let panel_top = search_y + search_hh + 6.0;
        let panel_h = lib_search_panel_h(reader.win_h, true);
        if y >= panel_top && y < panel_top + panel_h {
            return if y < lib_search_chips_y0(reader) + lib_chip_h(reader.win_h) {
                2
            } else {
                3
            };
        }
    }
    // Contenido: ¿la fila del carousel de Continue Reading (bajo su título)?
    let content_y0 = lib_content_y0(
        reader.win_h,
        reader.library.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let yc = y - content_y0 + reader.library.lib_scroll;
    let has_cont = reader.lib_has_cont();
    let cont_h = lib_cont_block_h(reader.win_w, reader.win_h, has_cont);
    if yc >= lib_section_title_h(reader.win_h) && yc < cont_h {
        return 1;
    }
    // Organización: fila SORT (4) / FILTER (5).
    let grid_y0 = lib_grid_y0(reader.win_w, reader.win_h, has_cont);
    let org_top = grid_y0 - lib_org_block_h(reader.win_h);
    if yc >= org_top && yc < grid_y0 {
        return if yc < org_top + lib_org_chip_h(reader.win_h) {
            4
        } else {
            5
        };
    }
    0
}

/// Input del picker/biblioteca (un solo dedo): arrastre VERTICAL = scroll de
/// la lista (picker: por filas; biblioteca: por PÍXELES del contenido
/// completo, recientes + rejilla); arrastre HORIZONTAL = scroll del carousel
/// de recientes o de las filas de chips (biblioteca); tap (sin arrastre) =
/// selección. Reemplaza a la máquina de gestos del visor (sin pinch).
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
                let (zone, h0) = if reader.mode == UiMode::Picker {
                    (0, 0.0)
                } else if reader.mode == UiMode::Library {
                    let z = library_down_zone(reader, y);
                    let h = match z {
                        1 => reader.library.lib_carousel_x,
                        2 => reader.library.lib_letters_x,
                        3 => reader.library.lib_folders_x,
                        4 => reader.library.lib_sort_x,
                        5 => reader.library.lib_filter_x,
                        _ => 0.0,
                    };
                    (z, h)
                } else {
                    (0, 0.0)
                };
                let v0 = if reader.mode == UiMode::Picker {
                    reader.list_scroll as f32
                } else if reader.mode == UiMode::Library {
                    reader.library.lib_scroll
                } else {
                    reader.discover_scroll()
                };
                reader.list_drag = Some(ListDrag {
                    sx: x,
                    sy: y,
                    v0,
                    h0,
                    zone,
                });
            }
        }
        MotionAction::Move => {
            if let Some(drag) = reader.list_drag.as_ref()
                && let Some(&(_, x, y)) = pts.first()
            {
                let dx = x - drag.sx;
                let dy = y - drag.sy;
                let moved = (dx * dx + dy * dy).sqrt();
                if moved > TAP_SLOP && dx.abs() > dy.abs() {
                    // Arrastre HORIZONTAL (solo biblioteca): el scroll de
                    // partida se guardó en `h0` según la zona del Down.
                    if reader.mode == UiMode::Library {
                        let max = match drag.zone {
                            1 => reader.lib_cont_max_x(),
                            2 => reader.lib_chips_max_x(0),
                            3 => reader.lib_chips_max_x(1),
                            4 => reader.lib_org_max_x(0),
                            5 => reader.lib_org_max_x(1),
                            _ => 0.0,
                        };
                        let s = (drag.h0 - dx).clamp(0.0, max);
                        let changed = match drag.zone {
                            1 => reader.library.lib_carousel_x != s,
                            2 => reader.library.lib_letters_x != s,
                            3 => reader.library.lib_folders_x != s,
                            4 => reader.library.lib_sort_x != s,
                            5 => reader.library.lib_filter_x != s,
                            _ => false,
                        };
                        if changed {
                            match drag.zone {
                                1 => reader.library.lib_carousel_x = s,
                                2 => reader.library.lib_letters_x = s,
                                3 => reader.library.lib_folders_x = s,
                                4 => reader.library.lib_sort_x = s,
                                5 => reader.library.lib_filter_x = s,
                                _ => {}
                            }
                            // Scroll horizontal de una fila: se re-renderiza
                            // SOLO esa fila (bitmap pequeño) y se remienda
                            // sobre su contenedor; la pantalla no se
                            // re-renderiza (antes `list_dirty` reconstruía
                            // TODO por frame de arrastre).
                            reader.library.lib_row_dirty = Some(drag.zone);
                            reader.redraw();
                        }
                    }
                } else if moved > TAP_SLOP {
                    // Arrastre VERTICAL: picker por filas, biblioteca por px.
                    if reader.mode == UiMode::Picker {
                        let row_h = picker_row_h(reader.win_h) as f32;
                        let max_scroll =
                            reader.picker_len().saturating_sub(reader.picker_visible());
                        let s =
                            (drag.v0 - dy / row_h).round().clamp(0.0, max_scroll as f32) as usize;
                        if s != reader.list_scroll {
                            reader.list_scroll = s;
                            reader.list_dirty = true;
                            reader.redraw();
                        }
                    } else if reader.mode == UiMode::Library {
                        let max_v = reader.lib_max_scroll();
                        let s = (drag.v0 - dy).clamp(0.0, max_v);
                        if s != reader.library.lib_scroll {
                            reader.library.lib_scroll = s;
                            reader.redraw();
                        }
                    } else if reader.mode == UiMode::Discover {
                        let max_v = reader.discover_max_scroll();
                        let s = (drag.v0 - dy).clamp(0.0, max_v);
                        if s != reader.discover_scroll() {
                            reader.set_discover_scroll(s);
                            reader.redraw();
                        }
                    }
                }
            }
        }
        MotionAction::Up => {
            let drag = reader.list_drag.take();
            if let (Some(d), Some(&(_, x, y))) = (drag, pts.first()) {
                let moved = ((x - d.sx).powi(2) + (y - d.sy).powi(2)).sqrt();
                if moved <= TAP_SLOP {
                    list_tap(reader, app, x, y);
                }
            }
            reader.gesture.pointers.clear();
        }
        MotionAction::Cancel => {
            reader.list_drag = None;
            reader.gesture.pointers.clear();
        }
        // PointerUp: se ignora un segundo dedo (el picker no tiene pinch).
        _ => {}
    }
}

/// Tap en la pantalla de Descubrir (arXiv Discover): cabecera (tabs Biblioteca/Descubrir,
/// sub-tabs Feed/Buscar/Áreas, buscador) y contenido (tarjetas de papers, áreas o ficha).
fn discover_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let content_y0 = disc_content_y0(
        reader.win_h,
        reader.discover.screen,
        reader.status.is_some() || reader.discover.status_msg.is_some(),
    );

    // 1. Cabecera fija
    if y < content_y0 as f32 {
        // En Ficha: botón [← Volver]
        if reader.discover.screen == DiscoverScreen::Detail {
            let back_rect = disc_detail_back_rect(reader.win_w, reader.win_h);
            if x >= back_rect.0 && x <= back_rect.2 && y >= back_rect.1 && y <= back_rect.3 {
                reader.discover.screen = match reader.discover.active_tab {
                    DiscoverTab::Search => DiscoverScreen::Search,
                    _ => DiscoverScreen::Feed,
                };
                reader.discover.dirty = true;
                reader.redraw();
                return;
            }
        }

        // Pestañas principales: [Biblioteca] y [Descubrir]
        let (tab_lib, _tab_disc) = lib_tabs_rect(reader.win_w, reader.win_h);
        if x >= tab_lib.0 && x <= tab_lib.2 && y >= tab_lib.1 && y <= tab_lib.3 {
            reader.exit_discover(app);
            return;
        }

        // Sub-pestañas: [Feed] [Buscar] [Áreas]
        let subtabs = disc_subtabs_rect(reader.win_w, reader.win_h);
        for (tab, rect) in subtabs {
            if x >= rect.0 && x <= rect.2 && y >= rect.1 && y <= rect.3 {
                if tab != reader.discover.active_tab
                    || reader.discover.screen == DiscoverScreen::Detail
                {
                    reader.discover.active_tab = tab;
                    reader.discover.screen = match tab {
                        DiscoverTab::Feed => DiscoverScreen::Feed,
                        DiscoverTab::Search => DiscoverScreen::Search,
                        DiscoverTab::Areas => DiscoverScreen::Areas,
                    };
                    reader.discover.dirty = true;
                    if tab == DiscoverTab::Feed && reader.discover.feed_entries.is_empty() {
                        reader.discover_refresh_feed();
                    }
                    reader.redraw();
                }
                return;
            }
        }

        // Barra de búsqueda en pantalla Search
        if reader.discover.screen == DiscoverScreen::Search {
            let (input_r, clear_r, search_btn_r) = disc_search_rect(reader.win_w, reader.win_h);
            if !reader.discover.query.is_empty()
                && x >= clear_r.0
                && x <= clear_r.2
                && y >= clear_r.1
                && y <= clear_r.3
            {
                reader.discover.query.clear();
                crate::jni::ime_set_text(app, "");
                reader.discover.dirty = true;
                reader.redraw();
                return;
            }
            if x >= search_btn_r.0
                && x <= search_btn_r.2
                && y >= search_btn_r.1
                && y <= search_btn_r.3
            {
                let q = reader.discover.query.clone();
                reader.lib_close_ime(app);
                reader.discover_search(&q);
                return;
            }
            if x >= input_r.0 && x <= input_r.2 && y >= input_r.1 && y <= input_r.3 {
                reader.discover_open_keyboard(app);
                return;
            }
        }
        return;
    }

    // 2. Contenido scrolleable
    let scroll = reader.discover_scroll();
    let cy = y - content_y0 as f32 + scroll;

    match reader.discover.screen {
        DiscoverScreen::Feed | DiscoverScreen::Search => {
            let entries = if reader.discover.screen == DiscoverScreen::Feed {
                reader.discover.feed_entries.clone()
            } else {
                reader.discover.search_entries.clone()
            };

            for (idx, entry) in entries.iter().enumerate() {
                let card_rect = disc_card_rect(reader.win_w, idx);
                if x >= card_rect.0 && x <= card_rect.2 && cy >= card_rect.1 && cy <= card_rect.3 {
                    let action_rect = disc_card_action_rect(card_rect);
                    let hit_action = x >= action_rect.0
                        && x <= action_rect.2
                        && cy >= action_rect.1
                        && cy <= action_rect.3;

                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    if hit_action && is_downloading {
                        reader.discover.cancel();
                        reader.show_toast("Descarga cancelada");
                        reader.redraw();
                        return;
                    }

                    let local_entry = reader.find_arxiv_in_library(&entry.id);
                    if hit_action && local_entry.is_some() {
                        reader.open_library_entry(app, &local_entry.unwrap());
                        return;
                    }
                    // Tocar tarjeta o botón "Ficha": abrir ficha detallada
                    reader.discover.selected_entry = Some(entry.clone());
                    reader.discover.screen = DiscoverScreen::Detail;
                    reader.discover.detail_scroll = 0.0;
                    reader.discover.dirty = true;
                    reader.redraw();
                    return;
                }
            }

            // Botón "Cargar más"
            let has_more = if reader.discover.screen == DiscoverScreen::Feed {
                reader.discover.feed_has_more
            } else {
                reader.discover.search_has_more
            };
            if has_more {
                let last_bottom = 16.0 + entries.len() as f32 * (disc_card_h() + disc_card_gap());
                let more_rect = disc_more_btn_rect(reader.win_w, last_bottom);
                if x >= more_rect.0 && x <= more_rect.2 && cy >= more_rect.1 && cy <= more_rect.3 {
                    reader.discover_more();
                    return;
                }
            }
        }
        DiscoverScreen::Areas => {
            for (idx, cat) in ARXIV_CATEGORIES.iter().enumerate() {
                let row_rect = disc_cat_row_rect(reader.win_w, idx);
                if x >= row_rect.0 && x <= row_rect.2 && cy >= row_rect.1 && cy <= row_rect.3 {
                    reader
                        .discover
                        .toggle_category(cat.code, reader.internal_dir.as_deref());
                    reader.discover.dirty = true;
                    reader.redraw();
                    return;
                }
            }
        }
        DiscoverScreen::Detail => {
            if let Some(entry) = reader.discover.selected_entry.clone() {
                let (action_rect, _) = disc_detail_layout(reader.win_w, &entry);
                if x >= action_rect.0
                    && x <= action_rect.2
                    && cy >= action_rect.1
                    && cy <= action_rect.3
                {
                    let is_downloading =
                        reader.discover.downloading_id.as_deref() == Some(entry.id.as_str());
                    if is_downloading {
                        reader.discover.cancel();
                        reader.show_toast("Descarga cancelada");
                        reader.redraw();
                    } else if let Some(local_entry) = reader.find_arxiv_in_library(&entry.id) {
                        reader.open_library_entry(app, &local_entry);
                    } else {
                        let id = entry.id.clone();
                        reader.discover_download(&id, Some(entry));
                    }
                    return;
                }
            }
        }
    }
}

/// Métricas cinemáticas de dos dedos usadas para clasificar el gesto.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TwoFingerKinematics {
    pub(crate) dot: f32,
    pub(crate) mid_disp: f32,
    pub(crate) cum_mid_disp: f32,
    pub(crate) cum_delta_d: f32,
    pub(crate) rel_cum_delta: f32,
    pub(crate) frame_delta_d: f32,
    pub(crate) va_len: f32,
    pub(crate) vb_len: f32,
}

/// Clasifica y actualiza el modo del gesto de dos dedos (discriminación Pan vs Zoom).
pub(crate) fn discriminate_two_finger_mode(
    current_mode: TwoFingerMode,
    k: &TwoFingerKinematics,
) -> TwoFingerMode {
    match current_mode {
        TwoFingerMode::Undecided => {
            // Confirmar Pan: coherencia direccional (ambos dedos se desplazan en el
            // mismo semiespacio dot >= 0 o traslación domina claramente la distancia),
            // desplazamiento acumulado del centro supera el umbral y cambio relativo bajo.
            if (k.dot >= 0.0 || k.cum_mid_disp > 2.0 * k.cum_delta_d)
                && k.cum_mid_disp > crate::TWO_FINGER_PAN_MIN_DISP_PX
                && k.cum_mid_disp > 1.5 * k.cum_delta_d
                && k.rel_cum_delta < crate::TWO_FINGER_ZOOM_REL_THRESHOLD
            {
                TwoFingerMode::Pan
            } else if (k.dot < 0.0 && (k.va_len > 2.0 || k.vb_len > 2.0))
                || (k.cum_delta_d >= crate::TWO_FINGER_ZOOM_MIN_PX
                    && k.rel_cum_delta >= crate::TWO_FINGER_ZOOM_REL_THRESHOLD)
                || (k.cum_delta_d >= 1.8 * k.cum_mid_disp && k.cum_delta_d >= 4.0)
                || ((k.va_len < 1.0 && k.vb_len >= 3.0 || k.vb_len < 1.0 && k.va_len >= 3.0)
                    && k.frame_delta_d >= 2.5)
            {
                TwoFingerMode::Zoom
            } else {
                TwoFingerMode::Undecided
            }
        }
        TwoFingerMode::Pan => {
            // En modo Pan el zoom está 100% congelado (cero zoom parásito por rodadura).
            // Transición Pan -> Zoom: solo si el usuario detiene la traslación
            // (mid_disp bajo) y realiza un cambio de distancia deliberado.
            if k.mid_disp < 4.0
                && k.frame_delta_d > 2.5
                && (k.cum_delta_d > crate::TWO_FINGER_PAN_TO_ZOOM_MIN_PX
                    || k.rel_cum_delta > crate::TWO_FINGER_PAN_TO_ZOOM_REL)
            {
                TwoFingerMode::Zoom
            } else {
                TwoFingerMode::Pan
            }
        }
        TwoFingerMode::Zoom => {
            // Ya confirmado en Zoom: se mantiene aquí hasta levantar dedos.
            TwoFingerMode::Zoom
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pan_discrimination_parallel_drift() {
        let k = TwoFingerKinematics {
            dot: 12.0 * 12.2 + 0.5 * 0.4,
            mid_disp: 12.1,
            cum_mid_disp: 12.1,
            cum_delta_d: 0.5,
            rel_cum_delta: 0.5 / 300.0,
            frame_delta_d: 0.5,
            va_len: 12.1,
            vb_len: 12.2,
        };
        let next = discriminate_two_finger_mode(TwoFingerMode::Undecided, &k);
        assert_eq!(next, TwoFingerMode::Pan);
    }

    #[test]
    fn test_pan_mode_freezes_zoom_despite_drift() {
        let k = TwoFingerKinematics {
            dot: 15.0 * 15.0,
            mid_disp: 15.0,
            cum_mid_disp: 400.0,
            cum_delta_d: 25.0,
            rel_cum_delta: 25.0 / 300.0,
            frame_delta_d: 0.8,
            va_len: 15.0,
            vb_len: 15.0,
        };
        let next = discriminate_two_finger_mode(TwoFingerMode::Pan, &k);
        assert_eq!(next, TwoFingerMode::Pan);
    }

    #[test]
    fn test_pinch_out_symmetric_triggers_zoom() {
        let k = TwoFingerKinematics {
            dot: (-3.0) * 3.0,
            mid_disp: 0.0,
            cum_mid_disp: 0.0,
            cum_delta_d: 6.0,
            rel_cum_delta: 6.0 / 200.0,
            frame_delta_d: 6.0,
            va_len: 3.0,
            vb_len: 3.0,
        };
        let next = discriminate_two_finger_mode(TwoFingerMode::Undecided, &k);
        assert_eq!(next, TwoFingerMode::Zoom);
    }

    #[test]
    fn test_pinch_asymmetric_one_finger_still() {
        let k = TwoFingerKinematics {
            dot: 0.0,
            mid_disp: 2.0,
            cum_mid_disp: 2.0,
            cum_delta_d: 4.0,
            rel_cum_delta: 4.0 / 200.0,
            frame_delta_d: 4.0,
            va_len: 0.0,
            vb_len: 4.0,
        };
        let next = discriminate_two_finger_mode(TwoFingerMode::Undecided, &k);
        assert_eq!(next, TwoFingerMode::Zoom);
    }

    #[test]
    fn test_pan_to_zoom_transition_after_stop() {
        let k = TwoFingerKinematics {
            dot: (-2.0) * 2.0,
            mid_disp: 0.5,
            cum_mid_disp: 300.0,
            cum_delta_d: 15.0,
            rel_cum_delta: 15.0 / 200.0,
            frame_delta_d: 3.5,
            va_len: 2.0,
            vb_len: 2.0,
        };
        let next = discriminate_two_finger_mode(TwoFingerMode::Pan, &k);
        assert_eq!(next, TwoFingerMode::Zoom);
    }
}
