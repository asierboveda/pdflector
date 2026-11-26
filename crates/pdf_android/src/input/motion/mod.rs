// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Procesamiento de `MotionEvent`s, partido por `UiMode` (IN-02):
//! - `viewer`: `tick_gestures` y `handle_motion` (máquina de gestos del
//!   visor: tap/pull/pinch/pan, trazo de herramienta y borrado con stylus);
//! - `picker`: input de las LISTAS (`handle_picker_motion`: scroll y taps
//!   del picker interno, la biblioteca y Descubrir) y `picker_tap`;
//! - `library`: `library_tap` y `library_down_zone`;
//! - `discover`: `discover_tap`.
//!
//! Aquí queda el enrutado por modo (`motion_route`, `list_tap`). El estado
//! que consume (`GestureState`/`GestureKind`) vive en `gestos`; las muestras
//! de stylus conservan los timestamps monotónicos originales.

mod discover;
mod library;
mod picker;
mod viewer;

use crate::android_activity::AndroidApp;
use crate::reader::{Reader, UiMode};
use discover::discover_tap;
use library::library_tap;
use picker::picker_tap;

pub(crate) use viewer::{PenButtons, handle_motion, tick_gestures};

/// Manejador al que va un `MotionEvent` según el modo de UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MotionRoute {
    /// Máquina de gestos del visor (`handle_motion`: tap/pinch/pan/trazo).
    Viewer,
    /// Input de las listas (`handle_picker_motion`: scroll + tap, sin pinch).
    List,
}

/// Decide el manejador de un `MotionEvent`: picker, biblioteca y Descubrir
/// comparten el input de lista; el visor usa su máquina de gestos. Función
/// pura para poder caracterizar el enrutado en host (sin `AndroidApp`).
pub(crate) fn motion_route(mode: UiMode) -> MotionRoute {
    if mode == UiMode::Picker || mode == UiMode::Library || mode == UiMode::Discover {
        MotionRoute::List
    } else {
        MotionRoute::Viewer
    }
}

/// Pantalla de lista que recibe un tap (ninguna en el visor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListTapTarget {
    Picker,
    Library,
    Discover,
}

/// Destino del tap de lista según el modo (`None` en el visor).
pub(crate) fn list_tap_target(mode: UiMode) -> Option<ListTapTarget> {
    match mode {
        UiMode::Picker => Some(ListTapTarget::Picker),
        UiMode::Library => Some(ListTapTarget::Library),
        UiMode::Discover => Some(ListTapTarget::Discover),
        UiMode::Viewer => None,
    }
}

/// Tap sobre la lista activa (picker interno o biblioteca MediaStore).
fn list_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    match list_tap_target(reader.mode) {
        Some(ListTapTarget::Picker) => picker_tap(reader, app, x, y),
        Some(ListTapTarget::Library) => library_tap(reader, app, x, y),
        Some(ListTapTarget::Discover) => discover_tap(reader, app, x, y),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::android_activity::input::MotionAction;
    use crate::{PINCH_MIN, TAP_SLOP};
    use picker::{ListMotion, classify_list_motion};
    use viewer::{TapDrag, classify_tap_drag};

    // --- Caracterización del enrutado por `UiMode` (IN-02) ---

    const ALL_MODES: [UiMode; 4] = [
        UiMode::Viewer,
        UiMode::Picker,
        UiMode::Library,
        UiMode::Discover,
    ];

    const ALL_ACTIONS: [MotionAction; 13] = [
        MotionAction::Down,
        MotionAction::Up,
        MotionAction::Move,
        MotionAction::Cancel,
        MotionAction::Outside,
        MotionAction::PointerDown,
        MotionAction::PointerUp,
        MotionAction::HoverMove,
        MotionAction::Scroll,
        MotionAction::HoverEnter,
        MotionAction::HoverExit,
        MotionAction::ButtonPress,
        MotionAction::ButtonRelease,
    ];

    #[test]
    fn route_viewer_goes_to_gesture_machine() {
        assert_eq!(motion_route(UiMode::Viewer), MotionRoute::Viewer);
    }

    #[test]
    fn route_list_modes_go_to_list_input() {
        assert_eq!(motion_route(UiMode::Picker), MotionRoute::List);
        assert_eq!(motion_route(UiMode::Library), MotionRoute::List);
        assert_eq!(motion_route(UiMode::Discover), MotionRoute::List);
    }

    #[test]
    fn route_depends_only_on_mode_for_every_action() {
        // Down/Move/Up/Cancel (y el resto de acciones) van al mismo manejador
        // dentro de un modo: el enrutado no mira la acción.
        for mode in ALL_MODES {
            let expected = if mode == UiMode::Viewer {
                MotionRoute::Viewer
            } else {
                MotionRoute::List
            };
            for action in ALL_ACTIONS {
                assert_eq!(motion_route(mode), expected, "{mode:?} {action:?}");
            }
        }
    }

    #[test]
    fn list_tap_targets_the_active_list_screen() {
        assert_eq!(list_tap_target(UiMode::Picker), Some(ListTapTarget::Picker));
        assert_eq!(
            list_tap_target(UiMode::Library),
            Some(ListTapTarget::Library)
        );
        assert_eq!(
            list_tap_target(UiMode::Discover),
            Some(ListTapTarget::Discover)
        );
    }

    #[test]
    fn list_tap_is_ignored_in_viewer() {
        assert_eq!(list_tap_target(UiMode::Viewer), None);
    }

    #[test]
    fn every_list_routed_mode_has_a_tap_target() {
        for mode in ALL_MODES {
            assert_eq!(
                motion_route(mode) == MotionRoute::List,
                list_tap_target(mode).is_some(),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn same_drag_scrolls_list_but_pans_or_cancels_viewer() {
        // Un arrastre más allá del slop: en una lista es scroll; en el visor
        // desplaza la hoja ampliada o solo cancela el tap a zoom 1.
        let d = TAP_SLOP + 5.0;
        assert_eq!(classify_list_motion(0.0, d), ListMotion::Vertical);
        assert_eq!(classify_tap_drag(d, 2.0, false), TapDrag::Pan);
        assert_eq!(classify_tap_drag(d, PINCH_MIN, false), TapDrag::Cancel);
        // Dentro del slop ambos siguen siendo tap.
        assert_eq!(classify_list_motion(0.0, TAP_SLOP), ListMotion::Tap);
        assert_eq!(classify_tap_drag(TAP_SLOP, 2.0, false), TapDrag::Stay);
    }
}
