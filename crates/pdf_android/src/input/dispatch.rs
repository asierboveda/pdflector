// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Bucle de eventos de input (extraído de `input.rs`, 2026-09-06):
//! `handle_input` drena `input_events_iter()` y despacha cada `MotionEvent` a
//! `handle_motion` (visor/listas), drenando antes el history del stylus y
//! calculando el ancla de tiempo/presión USI (`pending_t0_ns` /
//! `pending_pressure`) que el trazo consume.

use super::motion::{PenButtons, handle_motion};
use super::stylus::{feed_stylus_history, is_stylus_tool, normalize_pressure};
use crate::reader::Reader;
use android_activity::input::{InputEvent, MotionAction};
use android_activity::{AndroidApp, InputStatus};
use log::warn;

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
