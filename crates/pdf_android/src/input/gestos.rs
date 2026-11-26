// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Máquina de gestos del visor: el estado
//! de la máquina (`GestureState` + `GestureKind`), el temporizador de
//! long-press (`LONG_PRESS_MS`) y los TAPS del visor — página, chrome del
//! visor, sheet de ajustes y panel de IA — resueltos por
//! `fire_tap_action`. El procesamiento de eventos (`handle_motion`,
//! `tick_gestures`) vive en `motion`; `dispatch` (handle_input) y `stylus`
//! (USI 2.0) son los otros submódulos de `input`.

use crate::android_activity::AndroidApp;
use crate::annotations::ToolKind;
use crate::draw::{sheet_buttons, viewer_top_chrome_buttons};
use crate::reader::{
    Reader, ReadingMode, page_badge_rect, sheet_h, viewer_bottom_chrome_h, viewer_top_chrome_h,
};
use std::time::Instant;

/// Umbral de LONG-PRESS para entrar en MODO SELECCIÓN (selección de texto):
/// mantener un dedo QUIETO (sin levantarlo y sin moverse más de `TAP_SLOP`)
/// sobre el documento durante `LONG_PRESS_MS` fija el ancla en ese punto y
/// muestra el rect de selección (un punto, aún sin arrastrar). Valor estándar
/// de long-press en Android (~400 ms); `tick_gestures` lo mide desde el Down
/// con `press_at` (el poll con timeout del bucle mantiene `tick` activo
/// mientras haya ventana).
pub(crate) const LONG_PRESS_MS: std::time::Duration = std::time::Duration::from_millis(400);

/// Modo del gesto de dos dedos (discriminación robusta pan vs pinch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TwoFingerMode {
    /// Fase indecisa (< 4-6 px de movimiento): filtra ruido inicial antes de
    /// clasificar como traslación pura o zoom.
    Undecided,
    /// Traslación pura con dos dedos (zoom congelado, pan 1:1 vía `pan_by`).
    Pan,
    /// Pinch zoom activo (zoom continuo 1:1 vía `set_zoom_fast`, sin escalones).
    Zoom,
}

/// Gesto multitáctil en curso (máquina de gestos).
#[derive(Clone, Copy, Debug)]
pub(crate) enum GestureKind {
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
    /// Dos dedos: traslación (pan) o zoom (pinch). `start_dist` es la
    /// distancia de referencia y `start_zoom` el zoom de partida.
    /// Incorpora discriminación robusta por coherencia direccional,
    /// umbral relativo (%) e histéresis de modo (`TwoFingerMode`) para
    /// evitar zoom parásito durante traslaciones con dos dedos.
    Pinch {
        start_dist: f32,
        start_zoom: f32,
        /// Centro del pinch al inicio del gesto (para desplazamiento acumulado).
        start_mid: (f32, f32),
        /// Centro del pinch del Move anterior (traslación: `pan += mid − prev_mid`).
        prev_mid: (f32, f32),
        /// Posición del puntero 0 en el Move anterior.
        prev_a: (f32, f32),
        /// Posición del puntero 1 en el Move anterior.
        prev_b: (f32, f32),
        /// Modo actual del gesto de dos dedos (Undecided, Pan o Zoom).
        mode: TwoFingerMode,
    },
    /// Long-press + arrastre (selección de texto): el ancla es el punto del
    /// dedo al superar `LONG_PRESS_MS` (`tick_gestures` materializa el rect
    /// como punto con `Reader::begin_sel`); al moverse > `SELECT_SLOP` el
    /// rect sigue al dedo (`Reader::update_sel`); al soltar se fija
    /// (`Reader::end_sel`) y se pide la explicación a la IA (un
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
    /// Un dedo (DEDO): mover la página (pan) — "los gestos con la mano son
    /// para mover/zoom". Entra al bajar el dedo con herramienta activa, o
    /// desde un `Tap` que supera `TAP_SLOP` con la hoja ampliada (zoom > 1)
    /// en modo Navegar. `start` es la posición del Down y `pan0` el pan de
    /// partida; cada Move fija `pan = pan0 + (cur − start)`. Dos dedos lo
    /// convierten en `Pinch`.
    Pan {
        start: (f32, f32),
        pan0: (f32, f32),
    },
    /// Arrastre sobre el panel de IA: desplaza su scroll línea a línea.
    /// `last_y` es la Y del Move anterior y `accum` el resto sin consumir.
    AiScroll {
        last_y: f32,
        accum: f32,
    },
    /// Dedo o lápiz sobre la barra de herramientas (o con su popover
    /// abierto): al levantar sin moverse es un tap de la barra; mantenido
    /// `LONG_PRESS_MS` sobre la tarjeta pasa a `ToolbarDrag`.
    ToolbarPress {
        start_x: f32,
        start_y: f32,
    },
    /// Arrastre de la barra tras la pulsación larga: al soltar se acopla al
    /// borde más cercano.
    ToolbarDrag,
    /// STYLUS dibujando el lazo de Recorte (herramienta Recorte o botón
    /// superior mantenido).
    Lasso,
    /// STYLUS moviendo o escalando la selección de Recorte.
    RecorteDrag,
}

/// Estado de los gestos: pointers activos (pointer_id, x, y) + gesto en curso
/// + temporizador del long-press (selección).
pub(crate) struct GestureState {
    pub(crate) pointers: Vec<(i32, f32, f32)>,
    pub(crate) kind: GestureKind,
    /// Long-press: `Instant` del Down del dedo que está en `Tap` sin moverse
    /// más de `TAP_SLOP`. Some mientras el dedo esté abajo y el gesto siga
    /// siendo un tap potencial; el poll con timeout del bucle mantiene `tick`
    /// activo para que `tick_gestures` dispare la selección al superar
    /// `LONG_PRESS_MS` aunque no llegue más input. Se desarma al moverse >
    /// `TAP_SLOP`, al entrar en el pinch o al levantar/cancelar el dedo.
    pub(crate) press_at: Option<Instant>,
}

impl GestureState {
    pub(crate) fn new() -> Self {
        Self {
            pointers: Vec::new(),
            kind: GestureKind::None,
            press_at: None,
        }
    }
}

/// Acción de un tap simple sobre la página.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageTap {
    Prev,
    Next,
    Chrome,
}

/// Acción del tap sobre la página según el modo de lectura: en Tap, tercio
/// izquierdo → anterior, derecho → siguiente, central → chrome; en Scroll
/// no hay pase de página por tap (se navega arrastrando): todo tap
/// alterna el chrome.
pub(crate) fn page_tap_action(mode: ReadingMode, x: f32, win_w: f32) -> PageTap {
    if mode == ReadingMode::Scroll {
        return PageTap::Chrome;
    }
    let third = win_w / 3.0;
    if x < third {
        PageTap::Prev
    } else if x > 2.0 * third {
        PageTap::Next
    } else {
        PageTap::Chrome
    }
}

/// Tap simple: tercio izquierdo → página anterior; tercio derecho → página siguiente;
/// tercio central → alternar visibilidad del chrome del visor. En modo
/// Scroll todo tap alterna el chrome (`page_tap_action`).
fn tap_page(reader: &mut Reader, x: f32) {
    match page_tap_action(reader.reading_mode, x, reader.win_w as f32) {
        PageTap::Prev => reader.prev_page(),
        PageTap::Next => reader.next_page(),
        PageTap::Chrome => reader.toggle_chrome(),
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
                "Mode:Tap" => reader.set_reading_mode(ReadingMode::Tap),
                "Mode:Scroll" => reader.set_reading_mode(ReadingMode::Scroll),
                "-10" => reader.jump_page(-10),
                "+10" => reader.jump_page(10),
                _ => reader.next_page(), // "N / total"
            }
            return;
        }
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
        Some("Explicar") => reader.confirm_ask_ai(),
        Some("×") => reader.close_ai_panel(),
        Some("▲") => reader.ai_scroll(-1),
        Some("▼") => reader.ai_scroll(1),
        _ => {}
    }
}

/// Ejecuta la acción de un tap simple en `(x, y)`.
pub(crate) fn fire_tap_action(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    // La UI (menús, sheet, chrome) responde SIEMPRE, también con herramienta
    // de anotación activa: el dedo debe poder ir a Biblioteca o abrir ajustes
    // sin cambiar antes a navegación. Solo el tap sobre la PÁGINA queda
    // supeditado a la herramienta.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_mode_keeps_the_three_thirds() {
        let w = 1200.0;
        assert_eq!(page_tap_action(ReadingMode::Tap, 100.0, w), PageTap::Prev);
        assert_eq!(page_tap_action(ReadingMode::Tap, 600.0, w), PageTap::Chrome);
        assert_eq!(page_tap_action(ReadingMode::Tap, 1100.0, w), PageTap::Next);
    }

    #[test]
    fn scroll_mode_taps_never_turn_pages() {
        let w = 1200.0;
        for x in [0.0, 100.0, 600.0, 1100.0, 1199.0] {
            assert_eq!(page_tap_action(ReadingMode::Scroll, x, w), PageTap::Chrome);
        }
    }
}
