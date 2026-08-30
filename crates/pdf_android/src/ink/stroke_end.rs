//! Rematador de trazo (Stroke End Settlement & Tapering).
//!
//! Basado en el `StrokeEndPredictor` de `google/ink-stroke-modeler` (Apache-2.0):
//! - Al recibir el evento de levantar el lápiz (`Up`), la masa virtual puede encontrarse
//!   aún en tránsito hacia la posición final del contacto.
//! - Simula el frenado natural y progresivo de la masa virtual hacia el punto de despegue
//!   en pasos discretos de $\Delta t \approx 4\text{ ms}$.
//! - Desvanece la presión residual suavemente hacia cero para producir una terminación
//!   afilada y limpia, eliminando cualquier discontinuidad o parpadeo ("cero-pop").
//! - Operación 100% en stack con un array de tamaño fijo ($N \le 4$).

use crate::ink::spring_mass::SpringMassModeler;

/// Muestra de remate generada al levantar el lápiz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeEndSample {
    /// Posición X e Y del punto de remate.
    pub pt: (f32, f32),
    /// Presión desvanecida en $[0.0, 1.0]$.
    pub pressure: f32,
}

/// Capacidad máxima de muestras de remate en stack (sin alocaciones en heap).
pub const MAX_STROKE_END_SAMPLES: usize = 4;

/// Intervalo de tiempo por sub-paso de remate (4 ms).
pub const STROKE_END_STEP_DT_S: f32 = 0.004;

/// Rematador de fin de trazo.
#[derive(Clone, Copy, Debug, Default)]
pub struct StrokeEndPredictor;

impl StrokeEndPredictor {
    /// Genera la secuencia de asentamiento final del trazo en stack.
    ///
    /// Devuelve un array fijo y la cantidad de muestras válidas generadas ($0 \le count \le 4$).
    pub fn settle(
        spring_mass: &mut SpringMassModeler,
        endpoint: (f32, f32),
        initial_pressure: f32,
    ) -> ([StrokeEndSample; MAX_STROKE_END_SAMPLES], usize) {
        let mut samples = [StrokeEndSample {
            pt: endpoint,
            pressure: 0.0,
        }; MAX_STROKE_END_SAMPLES];

        let mut count = 0;

        for i in 0..MAX_STROKE_END_SAMPLES {
            let state = spring_mass.update(endpoint, STROKE_END_STEP_DT_S);

            // Tapering lineal de presión
            let taper_factor = 1.0 - ((i + 1) as f32 / MAX_STROKE_END_SAMPLES as f32);
            let current_pressure = initial_pressure * taper_factor;

            samples[count] = StrokeEndSample {
                pt: state.pos,
                pressure: current_pressure.max(0.0),
            };
            count += 1;

            // Condición de parada temprana: masa prácticamente en reposo
            let dx = state.pos.0 - endpoint.0;
            let dy = state.pos.1 - endpoint.1;
            let dist_sq = dx * dx + dy * dy;
            let vel_sq = state.vel.0 * state.vel.0 + state.vel.1 * state.vel.1;

            if dist_sq < 0.01 && vel_sq < 1.0 {
                break;
            }
        }

        // Garantizar que la última muestra sea exactamente el endpoint físico con presión 0
        if count > 0 {
            samples[count - 1].pt = endpoint;
            samples[count - 1].pressure = 0.0;
        }

        (samples, count)
    }
}
