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
//!
//! Partición de `input.rs` (2026-09-06, Tarea 4.3 de la reestructuración): el
//! fichero único se dividió en cuatro submódulos por responsabilidad —
//! `gestos` (estado de la máquina + taps del visor), `motion` (procesamiento
//! de `MotionEvent`s del visor y de las listas), `dispatch` (bucle de
//! eventos, `handle_input`) y `stylus` (bloque USI 2.0 del lápiz/borrador).
//! `mod input;` en `lib.rs` sigue resolviendo igual: la ruta `crate::input::*`
//! se conserva con re-exports (ver abajo).

// Máquina de gestos del visor: `GestureState`/`GestureKind`, temporizador de
// long-press y los taps del visor (página, chrome, sheet, menús).
mod gestos;
// Procesamiento de `MotionEvent`s: `tick_gestures`, `handle_motion` y el input
// de las listas (picker/biblioteca): taps, zona del Down y scroll.
mod motion;
// Bucle de eventos (`handle_input`): drena `input_events_iter()` y despacha a
// `motion`, drenando antes el history del stylus (USI).
mod dispatch;
// Bloque stylus USI 2.0: history 240 Hz del lápiz, timestamps y presión.
mod stylus;

pub(crate) use dispatch::handle_input;
pub(crate) use gestos::GestureState;
pub(crate) use motion::tick_gestures;
