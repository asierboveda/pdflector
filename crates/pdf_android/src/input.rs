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
//! que se desliza más de `TAP_SLOP` cancela el tap). El tap es INMEDIATO (se
//! dispara en el propio Up, sin ventana de doble-tap): un doble-tap rápido
//! son DOS cambios de página. El pinch con dos dedos
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

use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus};
use log::warn;

use crate::annotations::ToolKind;
use crate::draw::sheet_buttons;
use crate::draw::{tool_fab_rect, toolbar_buttons, toolbar_rect};
use crate::jni::launch_all_files_settings;
use crate::reader::{
    BookStatus, GRID_COLS, LibSort, ListDrag, Reader, UiMode, grid_cell_h, grid_cell_w, grid_gap,
    grid_pad, lib_chip_h, lib_chips, lib_cont_block_h, lib_cont_card_w, lib_cont_gap,
    lib_content_y0, lib_empty_state_geom, lib_grid_y0, lib_header_h, lib_org_block_h,
    lib_org_chip_h, lib_org_chips, lib_search_chips_y0, lib_search_h, lib_search_panel_h,
    lib_section_title_h, page_badge_rect, picker_btn_w, picker_header_h, picker_row_h,
    picker_visible_rows, sheet_h,
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
    /// mueve más de `TAP_SLOP` sin convertirse en un pull del sheet (un
    /// pequeño deslizamiento no cambia de página — sin scroll por arrastre en
    /// el modo página a página); al soltar sin moverse se dispara el tap
    /// INMEDIATO (en el propio Up, sin diferir). Mientras el dedo está
    /// quieto, `press_at` mide el long-press: al superar `LONG_PRESS_MS`
    /// `tick_gestures` entra en MODO SELECCIÓN y el tap NUNCA se dispara.
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
    /// `TAP_SLOP` (pull del sheet o cancelación), al entrar en el pinch o al
    /// levantar/cancelar el dedo.
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

/// Tap en el botón flotante de la barra de herramientas ("✎" esquina
/// superior derecha): muestra/oculta la barra. Devuelve true si el punto cae
/// en el botón (consumido — nunca cambia de página).
fn tool_fab_tap(reader: &mut Reader, x: f32, y: f32) -> bool {
    let (l, t, r, b) = tool_fab_rect(reader.win_w, reader.win_h);
    if x >= l && x < r && y >= t && y < b {
        reader.toggle_toolbar();
        true
    } else {
        false
    }
}

/// Tap DENTRO de la barra de herramientas (misma geometría que
/// `draw::toolbar_buttons`): "Resaltar"/"Boli" activan la herramienta,
/// "↶" deshace el último trazo de la sesión, "●" cicla el color del boli y
/// "→" vuelve a modo navegación y cierra la barra. Un tap en el hueco de la
/// barra (fuera de los botones) se consume igualmente (no navega) para no
/// cambiar de página mientras la barra está abierta.
pub(crate) fn toolbar_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) -> bool {
    let (l, t, r, b) = toolbar_rect(reader.win_w, reader.win_h);
    if !(x >= l && x < r && y >= t && y < b) {
        return false;
    }
    for (label, (bl, bt, br, bb)) in toolbar_buttons(reader, reader.win_w, reader.win_h) {
        if x >= bl && x < br && y >= bt && y < bb {
            match label {
                "Resaltar" => reader.set_tool(ToolKind::Highlight),
                "Boli" => reader.set_tool(ToolKind::Ink),
                "↶" => reader.undo_last_annotation(),
                "●" => reader.cycle_ink_color(),
                "━" => reader.cycle_ink_width(),
                _ => reader.close_toolbar(), // "→": navegación + cerrar barra
            }
            return true;
        }
    }
    // Hueco de la barra (fuera de los botones): consumido para no navegar.
    let _ = app;
    true
}

/// Tap DENTRO del sheet de ajustes: botones (misma geometría que
/// `draw::sheet_buttons`): "← Library" (biblioteca MediaStore), Dark/Light,
/// Search (biblioteca con la búsqueda lista para empezar — sin teclado, la
/// búsqueda ES la barra de filtros de la biblioteca), −10/+10 y "N / total"
/// (página siguiente). Un tap en el hueco del sheet (fuera de los botones) no
/// hace nada: el panel se cierra con un tap FUERA del sheet o con un arrastre
/// hacia arriba.
fn sheet_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    for (label, (l, t, r, b)) in sheet_buttons(reader, reader.win_w as f32, reader.win_h as f32) {
        if x >= l && x < r && y >= t && y < b {
            match label {
                "← Library" => reader.enter_library(app),
                "Dark" | "Light" => reader.toggle_dark(),
                "Search" => reader.enter_library_search(app),
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

/// Ejecuta la acción de un tap simple en `(x, y)` — la lógica del Up del
/// gesto `Tap`, disparada INMEDIATAMENTE al soltar (sin ventana de doble-tap):
/// menú de selección abierto → botón o cerrar; panel de IA abierto → botón o
/// cerrar; sheet visible → botón o cerrar; si no, indicador de página o tap
/// de página.
fn fire_tap_action(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    // Barra de herramientas y botón flotante: SIEMPRE con prioridad (también
    // con una herramienta activa — son la vía para volver a navegación).
    if tool_fab_tap(reader, x, y) {
        return;
    }
    if reader.toolbar_open && toolbar_tap(reader, app, x, y) {
        return;
    }
    // Con una herramienta de anotación activa el tap simple NO navega (el
    // dedo ya lo consumió el gesto de herramienta; un toque sin arrastre se
    // descarta en `end_tool_gesture`). Para pasar página hay que volver a
    // modo navegación (→ o ✎).
    if reader.tool != ToolKind::Navigate {
        return;
    }
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
/// - tirón hacia abajo desde la mitad superior = revelar el sheet de ajustes;
///   arrastre vertical con el sheet visible = moverlo (arriba cierra);
/// - pinch con dos dedos = zoom (factor relativo + anclado al centro del
///   pinch);
/// - mantener un dedo quieto durante `LONG_PRESS_MS` = entrar en MODO
///   SELECCIÓN (ancla en el punto del dedo; arrastrar extiende el rect,
///   soltar fija y abre el menú Copiar/Subrayar/IA; sin arrastre se
///   descarta);
/// - un dedo que se desliza más de `TAP_SLOP` y no es un pull cancela el tap
///   (sin scroll: el arrastre se eliminó por decisión del autor).
fn handle_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
    up_idx: Option<usize>,
    stylus: bool,
) {
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
            // NO se mueve más de `TAP_SLOP` (pull del sheet o cancelación).
            reader.gesture.pointers = pts;
            // Herramienta de anotación activa (Fase 3.5): el Down en la
            // página (fuera del "chrome" de la UI — botón flotante y barra)
            // empieza un GESTO DE HERRAMIENTA (boli/resaltador) en vez de un
            // tap: el arrastre dibuja y al soltar se crea la anotación. Los
            // gestos existentes no se rompen: con la herramienta Navegar
            // (la barra cerrada) esto no aplica y todo sigue igual.
            if reader.tool != ToolKind::Navigate
                && reader.gesture.pointers.len() == 1
                && let Some(&(_, x, y)) = reader.gesture.pointers.first()
                && !reader.chrome_hit(x, y)
            {
                // SEPARACIÓN DEDO/STYLUS: solo el lápiz dibuja con la
                // herramienta activa; el dedo (con herramienta activa) hace
                // PAN (mover el documento) y con dos dedos PINCH (zoom).
                if stylus {
                    log::info!("tool gesture iniciado con STYLUS");
                    reader.begin_tool_gesture(x, y);
                    if reader.tool_gesture.is_some() {
                        reader.gesture.kind = GestureKind::ToolDrawing;
                        return; // gesto de herramienta: sin tap ni long-press
                    }
                } else {
                    // Dedo: modo mano — pan 1 dedo (el pinch 2 dedos lo
                    // convierte el PointerDown). Sin tap ni long-press.
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
            // PALM REJECTION mientras se dibuja con el STYLUS: si la mano u
            // otro dedo toca durante un trazo del lápiz, ese segundo puntero
            // NO es un pinch — se IGNORA por completo (el trazo sigue; nada
            // de zoom/reescala). Arregla el "parpadeo" al escribir sobre
            // trazos existentes: al apoyar la palma al soltar, el código
            // convertía el gesto en pinch y la página reescalaba de golpe.
            if matches!(reader.gesture.kind, GestureKind::ToolDrawing) && stylus {
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
                        // dedo quieto durante la espera) y el gesto pasa a
                        // pull del sheet o se cancela.
                        reader.gesture.press_at = None;
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
                    reader.update_tool_gesture(cx, cy);
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
                    // anotación guardada (boli suavizado / resaltador alineado
                    // al texto; un toque sin arrastre se descarta).
                    reader.end_tool_gesture();
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
            reader.gesture.pointers.clear();
            reader.gesture.kind = GestureKind::None;
            reader.gesture.press_at = None;
            reader.clear_selection();
            reader.cancel_tool_gesture(); // el trazo en curso se descarta
            if pinch_active {
                reader.set_zoom_sharp(reader.zoom);
            }
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

    // CABECERA: botón "＋ Add book" (a la derecha; rescan + toast).
    if y < header_h {
        let pad = grid_pad(reader.win_w);
        let btn_w = (reader.win_w as f32 * 0.24).clamp(120.0, 220.0);
        let btn_h = (header_h * 0.5).clamp(36.0, 52.0);
        let btn_y = (header_h - btn_h) / 2.0;
        let btn_x = reader.win_w as f32 - pad - btn_w;
        if x >= btn_x && x < btn_x + btn_w && y >= btn_y && y < btn_y + btn_h {
            reader.add_book(app);
        }
        return;
    }

    // CAMPO de búsqueda: "✕" limpia los filtros (si los hay); tocar el campo
    // abre/cierra el panel de chips de letra/carpeta.
    if y < search_y + search_hh {
        let field_right = reader.win_w as f32 - grid_pad(reader.win_w);
        let has_filter = reader.lib_letter.is_some() || reader.lib_folder.is_some();
        if has_filter {
            let xw = search_hh - 8.0;
            let xx = field_right - 14.0 - xw;
            if x >= xx && x < xx + xw {
                reader.lib_clear_search();
                return;
            }
        }
        reader.lib_toggle_search();
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
    let has_cont = !reader.lib_continue_reading().is_empty();

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
            let cw = lib_cont_card_w(win_w);
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
                            "Recently Read" => LibSort::RecentlyRead,
                            "Title" => LibSort::Title,
                            "Author" => LibSort::Author,
                            _ => LibSort::RecentlyAdded,
                        });
                    } else {
                        reader.lib_set_status(match label.as_str() {
                            "Reading" => Some(BookStatus::Reading),
                            "Finished" => Some(BookStatus::Finished),
                            "Unread" => Some(BookStatus::Unread),
                            _ => None,
                        });
                    }
                    return;
                }
            }
        }
        return;
    }

    // Celda de la rejilla (lista FILTRADA): fila por y, columna por x (misma
    // geometría que `lib_grid_cell_rect`). Abre el libro (reanuda en su
    // página guardada si está empezado — lo hace `open_library_entry`).
    let row = ((yc - grid_y0) / grid_cell_h(win_w)) as usize;
    let cell_w = grid_cell_w(win_w);
    let pad = grid_pad(win_w);
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
                        let visible = picker_visible_rows(reader.win_h, reader.status.is_some());
                        let max_scroll = reader.pdf_list.len().saturating_sub(visible);
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
                let pts: Vec<(i32, f32, f32)> = motion
                    .pointers()
                    .map(|p| (p.pointer_id(), p.x(), p.y()))
                    .collect();
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
                handle_motion(reader, app, action, pts, up_idx, stylus);
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
