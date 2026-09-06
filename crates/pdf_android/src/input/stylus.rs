// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Bloque stylus USI 2.0 (extraído de `input.rs`, 2026-09-06): drenado del
//! history de 240 Hz del lápiz/borrador (`STYLUS_HISTORY_CAP`,
//! `feed_stylus_history`/`feed_stylus_sample`), detección de herramienta
//! stylus/borrador (`is_stylus_tool`), re-escalado de timestamps NDK a ms del
//! gesto (`gesture_ms`) y normalización de presión (`normalize_pressure`).

use super::gestos::GestureKind;
use crate::reader::Reader;
use android_activity::input::MotionEvent;

/// Tope de muestras históricas consumidas por evento del boli: Android
/// batchea a 240 Hz; si el looper se retrasa, el history acumularía un
/// retraso enorme. Cap duro conservando las más RECIENTES (`skip(len - cap)`:
/// las viejas ya son latencia perdida, no se redibujan).
const STYLUS_HISTORY_CAP: usize = 16;

/// ¿La herramienta del puntero es lápiz/borrador físico?
pub(crate) fn is_stylus_tool(t: android_activity::input::ToolType) -> bool {
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
pub(crate) fn feed_stylus_history(reader: &mut Reader, motion: &MotionEvent) {
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
pub(crate) fn gesture_ms(event_ns: i64, t0_ns: u64) -> f32 {
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
pub(crate) fn normalize_pressure(raw: f32) -> f32 {
    if raw <= 0.0 || raw > 1.0 { 0.5 } else { raw }
}
