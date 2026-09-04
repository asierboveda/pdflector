// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Input multitáctil: máquina de gestos del visor (tap/pinch/sheet) y
//! taps/arrastre de las listas (picker interno y biblioteca MediaStore).
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
//! que se desliza más de `TAP_SLOP` cancela el tap). El tap es INMEDIATO (se
//! dispara en el propio Up, sin ventana de doble-tap): un doble-tap rápido
//! son DOS cambios de página. El pinch con dos dedos
//! sigue haciendo zoom (factor RELATIVO + anclado, `Reader::begin_pinch`).
//!
//! El **sheet de ajustes** (panel desde el borde superior, la mitad de la
//! ventana; ver `Reader::sheet_*` y `draw::render_sheet`) se abre con TAP en
//! la barra superior del chrome (el pull-down se eliminó). Con el sheet
//! visible, un arrastre vertical lo mueve (subir = cerrar) y un TAP fuera
//! del panel lo cierra; un tap dentro pulsa
//! sus botones (Back/Open/Dark/−10/N/+10, misma geometría que
//! `draw::sheet_buttons`). El gesto del sheet NO choca con el tap de página
//! (el tap es < `TAP_SLOP` de movimiento) ni con el pinch (2 dedos → zoom,
//! el sheet se queda como esté). El indicador "N / total" abajo a la
//! izquierda también es táctil: tap = página siguiente (`page_badge_tap`).
//!
//! El modo dibujo (trazo con un dedo) se ELIMINÓ con la barra superior
//! (2026-08-XX): no queda ningún gesto de dibujo en el visor.
//!
//! ## Selección de texto: long-press + arrastre (2026-08-XX, Parte 1)
//!
//! Mantener un dedo QUIETO (sin levantarlo y sin moverse más de `TAP_SLOP`)
//! sobre el documento durante `LONG_PRESS_MS` (400 ms) entra en MODO
//! SELECCIÓN — `tick_gestures` (desde `Reader::tick`, poll con timeout de
//! `Reader::needs_tick` mientras el dedo esté abajo) fija el ancla en el
//! punto del dedo y materializa el rect como PUNTO en `Reader::sel`
//! (`begin_sel`); al arrastrar (manteniendo pulsado, > `SELECT_SLOP` desde
//! el ancla) el rect sigue al dedo (`update_sel`); al levantar, `end_sel`
//! fija la selección y abre el menú Copiar/Subrayar/IA — y un long-press
//! SIN arrastre se descarta (el punto no tiene texto que extraer). El
//! long-press NO dispara el tap de página: el tap simple es INMEDIATO (sin
//! ventana de doble-tap, `fire_tap_action` en el propio Up), así que un
//! doble-tap rápido son DOS cambios de página. El tap izq/der de página NO
//! se dispara nunca mientras hay selección/menú abierto (`sel_menu_tap`
//! consume esos taps); tocar fuera del menú lo cierra y descarta la
//! selección. El long-press solo aplica con el sheet cerrado.

use std::time::Instant;

use android_activity::input::{Button, ButtonState, InputEvent, MotionAction, MotionEvent};
use android_activity::{AndroidApp, InputStatus};
use log::warn;

use crate::annotations::{PEN_BTN_ERASE, PEN_BTN_MODE, PenMode, ToolKind};
use crate::draw::{
    SettingsMenuItem, ViewMenuItem, settings_menu_geometry, sheet_buttons, view_menu_geometry,
    viewer_top_chrome_buttons,
};
use crate::jni::launch_all_files_settings;
use crate::reader::{
    BookStatus, LibSort, LibraryCoverFit, LibraryGroupBy, LibraryViewMode, ListDrag, PickRow,
    PickerKind, Reader, UiMode, grid_cell_h, grid_cell_w, grid_gap, grid_pad, lib_add_btn_w,
    lib_chip_h, lib_chips, lib_cont_block_h, lib_cont_card_w, lib_cont_gap, lib_content_y0,
    lib_empty_state_geom, lib_grid_y0, lib_header_h, lib_org_block_h, lib_org_chip_h,
    lib_org_chips, lib_search_chips_y0, lib_search_h, lib_search_panel_h, lib_section_title_h,
    list_row_gap, list_row_h, page_badge_rect, picker_btn_w, picker_header_h, picker_row_h,
    settings_menu_button_rect, sheet_h, view_menu_button_rect, viewer_bottom_chrome_h,
    viewer_top_chrome_h,
};
use crate::{PINCH_MAX, PINCH_MIN, SELECT_SLOP, TAP_SLOP};

/// Umbral de LONG-PRESS para entrar en MODO SELECCIÓN (selección de texto):
/// mantener un dedo QUIETO (sin levantarlo y sin moverse más de `TAP_SLOP`)
/// sobre el documento durante `LONG_PRESS_MS` fija el ancla en ese punto y
/// muestra el rect de selección (un punto, aún sin arrastrar). Valor estándar
/// de long-press en Android (~400 ms); `tick_gestures` lo mide desde el Down
/// con `press_at` (el poll con timeout de `needs_tick` mantiene el bucle vivo
/// mientras el dedo esté abajo).
pub(crate) const LONG_PRESS_MS: std::time::Duration = std::time::Duration::from_millis(400);

/// Gesto multitáctil en curso (máquina de gestos).
#[derive(Clone, Copy, Debug)]
enum GestureKind {
    None,
    /// Un dedo: posible tap (página anterior/siguiente, indicador de página,
    /// sheet abierto: botón o cerrar). El gesto se CANCELA si el dedo se
    /// mueve más de `TAP_SLOP` (un pequeño deslizamiento no cambia de
    /// página — sin scroll por arrastre en el modo página a página); al
    /// INMEDIATO (en el propio Up, sin diferir). Mientras el dedo está
    /// quieto, `press_at` mide el long-press: al superar `LONG_PRESS_MS`
    /// `tick_gestures` entra en MODO SELECCIÓN y el tap NUNCA se dispara.
    Tap {
        start_x: f32,
        start_y: f32,
    },
    /// Un dedo: arrastre VERTICAL que mueve el sheet de ajustes YA visible
    /// (subir/bajar). `start_y` = Y del Down; el progreso del sheet sigue
    /// a `dy = y − start_y` (`Reader::drag_sheet`).
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
    /// Long-press + arrastre (selección de texto): el ancla es el punto del
    /// dedo al superar `LONG_PRESS_MS` (`tick_gestures` materializa el rect
    /// como punto con `Reader::begin_sel`); al moverse > `SELECT_SLOP` el
    /// rect sigue al dedo (`Reader::update_sel`); al soltar se fija
    /// (`Reader::end_sel`) y se abre el menú Copiar/Subrayar/IA (un
    /// long-press sin arrastre se descarta). Un segundo dedo cancela la
    /// selección en curso y pasa al pinch.
    Selecting {
        anchor: (f32, f32),
    },
    /// Un dedo: gesto de herramienta de anotación (resaltador o boli, Fase
    /// 3.5). El Down con una herramienta activa (y fuera del "chrome" de la
    /// UI) entra aquí: cada Move añade puntos (boli) o extiende el rect
    /// (resaltador) a través de `Reader::{begin,update,end}_tool_gesture`; al
    /// soltar, `end_tool_gesture` crea la anotación guardada. Un segundo dedo
    /// cancela el gesto en curso y pasa al pinch (la herramienta sigue
    /// activa: el siguiente Down vuelve a dibujar). SOLO entra con STYLUS:
    /// los dedos NUNCA dibujan (separación dedo/stylus — ver `Pan`).
    ToolDrawing,
    /// Un dedo (STYLUS con el botón DOWN del boli pulsado): BORRADO. Cada
    /// Move hace hit-test contra las anotaciones de la página y las elimina
    /// en vivo (ver `Reader::{begin,update,end}_erase_gesture`); al levantar
    /// se persiste UNA vez. No crea anotaciones ni entra en el undo.
    Erase,
    /// Un dedo (DEDO, con herramienta activa): mover la página (pan) — "los
    /// gestos con la mano son para mover/zoom". `start` es la posición del
    /// Down y `pan0` el pan de partida; cada Move fija
    /// `pan = pan0 + (cur − start)` del documento. Dos dedos lo convierten
    /// en `Pinch`.
    Pan {
        start: (f32, f32),
        pan0: (f32, f32),
    },
}

/// Estado de los gestos: pointers activos (pointer_id, x, y) + gesto en curso
/// + temporizador del long-press (selección).
pub(crate) struct GestureState {
    pointers: Vec<(i32, f32, f32)>,
    kind: GestureKind,
    /// Long-press: `Instant` del Down del dedo que está en `Tap` sin moverse
    /// más de `TAP_SLOP`. Some mientras el dedo esté abajo y el gesto siga
    /// siendo un tap potencial; `Reader::needs_tick` mantiene el poll con
    /// timeout para que `tick_gestures` dispare la selección al superar
    /// `LONG_PRESS_MS` aunque no llegue más input. Se desarma al moverse >
    /// `TAP_SLOP`, al entrar en el pinch o al levantar/cancelar el dedo.
    press_at: Option<Instant>,
}

impl GestureState {
    pub(crate) fn new() -> Self {
        Self {
            pointers: Vec::new(),
            kind: GestureKind::None,
            press_at: None,
        }
    }

    /// ¿Temporizador de long-press activo (dedo quieto en `Tap`)? El bucle de
    /// eventos mantiene el poll con timeout mientras tanto para que el modo
    /// selección entre aunque el dedo no se mueva.
    #[allow(dead_code)] // long-press aún activo vía tick_gestures
    pub(crate) fn press_pending(&self) -> bool {
        self.press_at.is_some()
    }
}

/// Tap simple: tercio izquierdo → página anterior; tercio derecho → página siguiente;
/// tercio central → alternar visibilidad del chrome del visor.
fn tap_page(reader: &mut Reader, x: f32) {
    let third = reader.win_w as f32 / 3.0;
    if x < third {
        reader.prev_page();
    } else if x > 2.0 * third {
        reader.next_page();
    } else {
        reader.toggle_chrome();
    }
}

/// Tap en el chrome del visor (barra superior e inferior).
/// Devuelve true si el tap fue consumido por el chrome.
fn viewer_chrome_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) -> bool {
    if !reader.chrome_visible {
        return false;
    }
    let top_h = viewer_top_chrome_h(reader.win_h);
    let bot_h = viewer_bottom_chrome_h(reader.win_h);
    let win_w = reader.win_w as f32;
    let win_h = reader.win_h as f32;

    if y < top_h {
        let btns = viewer_top_chrome_buttons(win_w, win_h);
        for (tag, (l, t, r, b)) in btns {
            if x >= l && x < r && y >= t && y < b {
                match tag {
                    "Back" => reader.enter_library(app),
                    "Theme" => reader.cycle_theme(),
                    _ => {}
                }
                reader.touch_chrome();
                return true;
            }
        }
        // Sin gesto pull-down: el sheet se abre/cierra con tap en la barra
        // superior (el tap central gobierna el chrome + ajustes).
        reader.toggle_sheet();
        reader.touch_chrome();
        return true;
    }

    if y > win_h - bot_h {
        reader.touch_chrome();
        return true;
    }

    false
}

/// Tap en el indicador de página "N / total" (overlay abajo a la izquierda):
/// página siguiente. Devuelve true si el punto cae en el indicador.
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
/// `draw::sheet_buttons`): temas, navegación y acciones.
fn sheet_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    for (label, (l, t, r, b)) in sheet_buttons(reader, reader.win_w as f32, reader.win_h as f32) {
        if x >= l && x < r && y >= t && y < b {
            match label {
                "Theme:Light" => reader.set_theme(crate::theme::AppTheme::DefaultLight),
                "Theme:Sepia" => reader.set_theme(crate::theme::AppTheme::SepiaLight),
                "Theme:Dark" => reader.set_theme(crate::theme::AppTheme::DefaultDark),
                "Theme:Nord" => reader.set_theme(crate::theme::AppTheme::SepiaDark),
                "← Library" => reader.enter_library(app),
                "Search" => reader.enter_library_search(app),
                "Close" => reader.hide_sheet(),
                "-10" => reader.jump_page(-10),
                "+10" => reader.jump_page(10),
                _ => reader.next_page(), // "N / total"
            }
            return;
        }
    }
}

/// Tap con el menú de selección abierto.
fn sel_menu_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let Some(menu) = &reader.sel_menu else {
        return;
    };
    let inside = x >= menu.x as f32
        && x < (menu.x + menu.w) as f32
        && y >= menu.y as f32
        && y < (menu.y + menu.h) as f32;
    if !inside {
        reader.clear_selection();
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
        Some("IA") => reader.ask_ai(),
        Some(_) | None => reader.clear_selection(),
    }
}

/// Tap con el panel de "Preguntar a la IA" abierto.
fn ai_panel_tap(reader: &mut Reader, x: f32, y: f32) {
    let Some(panel) = &reader.ai_panel else {
        return;
    };
    let inside = x >= panel.x as f32
        && x < (panel.x + panel.w) as f32
        && y >= panel.y as f32
        && y < (panel.y + panel.h) as f32;
    if !inside {
        reader.close_ai_panel();
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
        _ => {}
    }
}

/// Ejecuta la acción de un tap simple en `(x, y)`.
fn fire_tap_action(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    // La UI (menús, sheet, chrome) responde SIEMPRE, también con herramienta
    // de anotación activa: el dedo debe poder ir a Biblioteca o abrir ajustes
    // sin cambiar antes a navegación. Solo el tap sobre la PÁGINA queda
    // supeditado a la herramienta.
    if reader.sel_menu.is_some() {
        sel_menu_tap(reader, app, x, y);
        return;
    }
    if reader.ai_panel.is_some() {
        ai_panel_tap(reader, x, y);
        return;
    }
    if reader.sheet_progress > 0.0 {
        if y < sheet_h(reader.win_h) as f32 {
            sheet_tap(reader, app, x, y);
        } else {
            reader.hide_sheet();
        }
        return;
    }
    if viewer_chrome_tap(reader, app, x, y) {
        // Tap en botones o barras de chrome del visor consumido
        return;
    }
    if reader.tool != ToolKind::Navigate {
        // Herramienta activa: el dedo sobre la página no cambia de página
        // (el trazo con dedo/stylus lo gestiona el gesto de herramienta).
        return;
    }
    if !reader.chrome_visible && page_badge_tap(reader, x, y) {
        // Indicador de página: siguiente (consumido).
    } else {
        tap_page(reader, x);
    }
}

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
    reader.gesture.kind = GestureKind::Pinch {
        start_dist: d.max(1.0),
        start_zoom: reader.zoom,
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
struct PenButtons {
    state: ButtonState,
    action: Button,
}

#[allow(clippy::too_many_arguments)]
fn handle_motion(
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
    if reader.mode == UiMode::Picker || reader.mode == UiMode::Library {
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
                    start_dist,
                    start_zoom,
                } if reader.gesture.pointers.len() >= 2 => {
                    let (_, ax, ay) = reader.gesture.pointers[0];
                    let (_, bx, by) = reader.gesture.pointers[1];
                    let d = ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt();
                    if d > 1.0 {
                        // Re-anclar el pinch al centro ACTUAL de los dedos
                        // ANTES de aplicar el zoom. BUG arreglado: el ancla
                        // solo se fijaba al caer el segundo dedo, así que si
                        // el gesto incluía traslación (el pinch real: un dedo
                        // quieto y el otro que se mueve, o ambos desplazándose)
                        // el contenido quedaba anclado al centro INICIAL y se
                        // DERIVABA bajo los dedos (el punto bajo los dedos no
                        // se quedaba fijo). `begin_pinch` re-captura z0/pan0
                        // del estado actual, por lo que el pan es continuo (a
                        // zoom == z0 el pan no cambia) y el factor de zoom
                        // sigue siendo RELATIVO a la distancia inicial del
                        // gesto (`start_dist`/`start_zoom` no se tocan).
                        reader.begin_pinch((ax + bx) / 2.0, (ay + by) / 2.0);
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
                        reader.lib_sort = LibSort::Title;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAuthor => {
                        reader.lib_sort = LibSort::Author;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortAdded => {
                        reader.lib_sort = LibSort::RecentlyAdded;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortRead => {
                        reader.lib_sort = LibSort::RecentlyRead;
                        reader.apply_filter();
                        reader.save_state();
                        reader.view_menu_open = false;
                    }
                    ViewMenuItem::SortProgress => {
                        reader.lib_sort = LibSort::Progress;
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
        let has_filter = !reader.lib_query.is_empty()
            || reader.lib_letter.is_some()
            || reader.lib_folder.is_some();
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
    let panel_h = lib_search_panel_h(reader.win_h, reader.lib_search_open);
    if reader.lib_search_open && y >= panel_top && y < panel_top + panel_h {
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
        reader.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    if y < content_y0 {
        return;
    }

    // Contenido scrolleable: pasar a coordenadas de CONTENIDO (y del Down +
    // scroll vertical).
    let yc = y - content_y0 + reader.lib_scroll;
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
            let i = ((x - grid_pad(win_w) + reader.lib_carousel_x) / (cw + lib_cont_gap())).floor();
            if i >= 0.0
                && let Some(book) = reader.lib_continue_reading().get(i as usize)
            {
                // Clonar ruta+nombre: `open_pdf_at` necesita &mut self.
                let path = book.path.clone();
                let name = book.name.clone();
                let start = crate::persist::progress_for(&reader.lib_books, &path).map(|p| p.page);
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
    if reader.lib_search_open {
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
        reader.lib_search_open,
        reader.status.is_some(),
    ) as f32;
    let yc = y - content_y0 + reader.lib_scroll;
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
                } else {
                    let z = library_down_zone(reader, y);
                    let h = match z {
                        1 => reader.lib_carousel_x,
                        2 => reader.lib_letters_x,
                        3 => reader.lib_folders_x,
                        4 => reader.lib_sort_x,
                        5 => reader.lib_filter_x,
                        _ => 0.0,
                    };
                    (z, h)
                };
                let v0 = if reader.mode == UiMode::Picker {
                    reader.list_scroll as f32
                } else {
                    reader.lib_scroll
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
                            1 => reader.lib_carousel_x != s,
                            2 => reader.lib_letters_x != s,
                            3 => reader.lib_folders_x != s,
                            4 => reader.lib_sort_x != s,
                            5 => reader.lib_filter_x != s,
                            _ => false,
                        };
                        if changed {
                            match drag.zone {
                                1 => reader.lib_carousel_x = s,
                                2 => reader.lib_letters_x = s,
                                3 => reader.lib_folders_x = s,
                                4 => reader.lib_sort_x = s,
                                5 => reader.lib_filter_x = s,
                                _ => {}
                            }
                            // Scroll horizontal de una fila: se re-renderiza
                            // SOLO esa fila (bitmap pequeño) y se remienda
                            // sobre su contenedor; la pantalla no se
                            // re-renderiza (antes `list_dirty` reconstruía
                            // TODO por frame de arrastre).
                            reader.lib_row_dirty = Some(drag.zone);
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
                    } else {
                        let max_v = reader.lib_max_scroll();
                        let s = (drag.v0 - dy).clamp(0.0, max_v);
                        if s != reader.lib_scroll {
                            reader.lib_scroll = s;
                            // Scroll vertical = solo cambiar de donde se copia
                            // la banda de contenido al buffer (memcpy); el
                            // render (Canvas+JNI) solo se relanza si el scroll
                            // sale de la banda actual (lo decide `redraw`).
                            // ANTES: `list_dirty = true` re-renderizaba la
                            // pantalla entera por frame (~20-60 ms → el lag y
                            // el parpadeo del scroll de la biblioteca).
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
                // HISTORY 240 Hz del boli (Ink y Erase): drenar las muestras
                // batcheadas ANTES del evento real (orden temporal). También
                // en el Up: su history cierra el trazo sin cuerda recta final.
                if matches!(action, MotionAction::Move | MotionAction::Up) {
                    feed_stylus_history(reader, motion);
                }
                let pts: Vec<(i32, f32, f32)> = motion
                    .pointers()
                    .map(|p| (p.pointer_id(), p.x(), p.y()))
                    .collect();
                // Fase 1 USI: timestamp (ns, System.nanoTime) y presión del
                // PRIMER pointer stylus — el ancla del Down (gesture_t0_ns)
                // y la presión del evento real salen de aquí. En multitouch
                // solo el stylus importa (guard pointers.len()==1 aguas
                // abajo); si no hay stylus, (0, 0.5) neutros.
                // Nota: `Pointer` (wrapper) no expone event_time (solo
                // HistoricalPointer y el MotionEvent); el timestamp del
                // evento real viene de `motion.event_time()` y es común a
                // todos los pointers del batch. La presión sí es por pointer.
                let stylus_t_ns = motion.event_time() as u64;
                let stylus_pressure = motion
                    .pointers()
                    .find(|p| is_stylus_tool(p.tool_type()))
                    .map(|p| normalize_pressure(p.pressure()))
                    .unwrap_or(0.5);
                reader.pending_t0_ns = Some(stylus_t_ns);
                reader.pending_pressure = Some(stylus_pressure);
                // Separación dedo/stylus (S-Pen, Saber): solo el STYLUS (o
                // borrador/estilo invertido) dibuja con la herramienta
                // activa; los dedos (y la palma) navegan (pan/pinch).
                let stylus = motion.pointers().any(|p| {
                    matches!(
                        p.tool_type(),
                        android_activity::input::ToolType::Stylus
                            | android_activity::input::ToolType::Eraser
                    )
                });
                let up_idx = if action == MotionAction::PointerUp {
                    Some(motion.pointer_index())
                } else {
                    None
                };
                handle_motion(
                    reader,
                    app,
                    action,
                    pts,
                    up_idx,
                    stylus,
                    PenButtons {
                        state: motion.button_state(),
                        action: motion.action_button(),
                    },
                    motion.event_time(),
                    stylus_pressure,
                );
                InputStatus::Handled
            }
            InputEvent::KeyEvent(_) | InputEvent::TextEvent(_) | InputEvent::TextAction(_) | _ => {
                InputStatus::Unhandled
            }
        });
        if !read {
            if reader.take_repaint() {
                reader.blit();
            }
            break;
        }
    }
}

/// Tope de muestras históricas consumidas por evento del boli: Android
/// batchea a 240 Hz; si el looper se retrasa, el history acumularía un
/// retraso enorme. Cap duro conservando las más RECIENTES (`skip(len - cap)`:
/// las viejas ya son latencia perdida, no se redibujan).
const STYLUS_HISTORY_CAP: usize = 16;

/// ¿La herramienta del puntero es lápiz/borrador físico?
fn is_stylus_tool(t: android_activity::input::ToolType) -> bool {
    matches!(
        t,
        android_activity::input::ToolType::Stylus | android_activity::input::ToolType::Eraser
    )
}

/// Alimenta UNA muestra del boli al gesto en curso (la máquina de estados la
/// lleva el evento real en `handle_motion`; aquí solo el trazo/goma). Replica
/// los brazos Move de ToolDrawing/Erase (mismos guards: un puntero, kind
/// activo): los puntos históricos encadenan `update_tool_gesture` (curva
/// midpoint) o `update_erase_gesture` (`erase_last` barre sin huecos).
///
/// Fase 1 (USI 2.0): cada muestra lleva `t_ms` (timestamp NDK re-escalado al
/// ancla del gesto) y `pressure` normalizada [0,1] — el predictor y el
/// grosor dependiente de presión los consumen.
fn feed_stylus_sample(reader: &mut Reader, x: f32, y: f32, t_ms: f32, pressure: f32) {
    match reader.gesture.kind {
        GestureKind::ToolDrawing if reader.gesture.pointers.len() == 1 => {
            reader.update_tool_gesture(x, y, t_ms, pressure);
        }
        GestureKind::Erase if reader.gesture.pointers.len() == 1 => {
            reader.update_erase_gesture(x, y);
        }
        _ => {}
    }
}

/// Drena el history de los punteros stylus del evento (Move/Up) con cap
/// `STYLUS_HISTORY_CAP`. Sin Vec intermedio: iteración directa sobre
/// `p.history()` (ExactSizeIterator; `skip` conserva las recientes).
///
/// NOTA de alcance: el drain solo alimenta el gesto EN CURSO
/// (ToolDrawing/Erase con un puntero). El FILTRO stylus vs palma del
/// Down/PointerDown lo hace `handle_motion` (flag `stylus` +
/// `pointers.len() == 1`): si el panel multiplexa palma+stylus en un solo
/// MotionEvents, ese evento nunca arranca un trazo — el drain no cambia ese
/// comportamiento.
fn feed_stylus_history(reader: &mut Reader, motion: &MotionEvent) {
    let t0 = reader.gesture_t0_ns;
    for p in motion.pointers().filter(|p| is_stylus_tool(p.tool_type())) {
        let hist = p.history();
        let skip = hist.len().saturating_sub(STYLUS_HISTORY_CAP);
        for hp in hist.skip(skip) {
            // Timestamp NDK (System.nanoTime) → ms monótonos del gesto,
            // re-escalados con el ancla tomada en el Down (gesture_t0_ns).
            // La presión va por eje AXIS_PRESSURE (USI 2.0 la reporta;
            // drivers sin presión dan 0.0 → neutral 0.5 en el gestor).
            let t_ms = gesture_ms(hp.event_time(), t0);
            let pressure = normalize_pressure(hp.pressure());
            feed_stylus_sample(reader, hp.x(), hp.y(), t_ms, pressure);
        }
    }
}

/// Re-escala un timestamp NDK (ns, base System.nanoTime) a ms del gesto:
/// `gesture_t0_ns` es el event_time del Down (ancla t=0). Sin ancla (0.0,
/// p. ej. muestra de dedo tras un gesto borrado) devuelve 0.
#[inline]
fn gesture_ms(event_ns: i64, t0_ns: u64) -> f32 {
    if t0_ns == 0 {
        return 0.0;
    }
    let d = event_ns as i128 - t0_ns as i128;
    // ns → ms con saturación i128→f32 (un gesto no dura horas; wrap no ocurre
    // en relojes monótonos de Android de 64 bits, pero el cast no debe colar
    // basura en el predictor si el driver reporta tiempos fuera de orden).
    (d as f64 / 1_000_000.0).clamp(-1_000.0, 1_000.0) as f32
}

/// Normaliza la presión del driver a [0.5, 1] usable: USI 2.0 reporta
/// [0,1] con 0.0 en hover/sin contacto; un 0.0 EXACTO en una muestra de
/// Move suele ser "axis no reportado" (algunos firmwares) → 0.5 neutro
/// (w_base) en vez de aplastar el trazo a 0.6·w.
#[inline]
fn normalize_pressure(raw: f32) -> f32 {
    if raw <= 0.0 || raw > 1.0 { 0.5 } else { raw }
}
