//! Port de `google/ink-stroke-modeler` a Rust puro para Android/Stylus.
//!
//! Pipeline completo de modelado físico de trazo:
//! 1. `InputFilter`: Sanitizado de eventos 240 Hz, control de jitter y timestamps.
//! 2. `SpringMassModeler`: Simulación física críticamente amortiguada ($\zeta = 1.0$)
//!    con integración analítica exacta y aceleración en giros vivos.
//! 3. `KalmanPredictor`: Estimación de velocidad/aceleración y proyección dinámica a 20–35 ms.
//! 4. `StrokeEndPredictor`: Asentamiento de masa al despegar sin discontinuidades ("cero-pop").
//!
//! Invariantes de rendimiento (AGENTS.md):
//! - $O(1)$ en stack: Cero alocaciones dinámicas (`Vec`, `Box`) durante el trazo.
//! - Cero `unwrap()` o `expect()` en producción.
//! - Tiempo de ejecución $< 2\,\mu\text{s}$ por muestra.

pub mod input_filter;
pub mod kalman_predictor;
pub mod spring_mass;
pub mod stroke_end;

#[cfg(test)]
pub mod tests;

use input_filter::InputFilter;
use kalman_predictor::KalmanPredictor;
use spring_mass::SpringMassModeler;
use stroke_end::StrokeEndPredictor;

/// Resultado de modelado producido por cada muestra del lápiz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelerResult {
    /// Posición suavizada confirmada (X, Y) de la masa virtual.
    pub confirmed_pt: (f32, f32),
    /// Posición predicha proyectada hacia adelante ($\approx 25\text{--}30\text{ ms}$).
    pub predicted_pt: Option<(f32, f32)>,
    /// Presión normalizada resultante en $[0.0, 1.0]$.
    pub pressure: f32,
}

/// Orquestador del pipeline de modelado físico de trazo.
#[derive(Clone, Copy, Debug)]
pub struct InkStrokeModeler {
    /// Filtro de sanitización y control de ruido.
    pub filter: InputFilter,
    /// Simulador físico masa-resorte.
    pub spring_mass: SpringMassModeler,
    /// Predictor cinemático de Kalman.
    pub predictor: KalmanPredictor,
    /// Grosor base del trazo configurado.
    pub w_base: f32,
    /// Último punto bruto registrado.
    last_raw_pt: (f32, f32),
    /// Última posición confirmada por la masa.
    last_confirmed_pt: (f32, f32),
    /// Última presión sanitizada.
    last_pressure: f32,
}

impl InkStrokeModeler {
    /// Crea un nuevo modelador de trazo con el grosor base especificado.
    pub fn new(w_base: f32) -> Self {
        Self {
            filter: InputFilter::new(),
            spring_mass: SpringMassModeler::with_defaults(),
            predictor: KalmanPredictor::with_defaults(),
            w_base,
            last_raw_pt: (0.0, 0.0),
            last_confirmed_pt: (0.0, 0.0),
            last_pressure: 0.5,
        }
    }

    /// Reinicia todos los estados del pipeline para comenzar un nuevo trazo.
    pub fn reset(&mut self) {
        self.filter.reset();
        self.spring_mass.reset();
        self.predictor.reset();
        self.last_raw_pt = (0.0, 0.0);
        self.last_confirmed_pt = (0.0, 0.0);
        self.last_pressure = 0.5;
    }

    /// Procesa una muestra entrante del lápiz en tiempo real ($O(1)$ en stack).
    ///
    /// Ejecuta la cadena: Sanitizado $\to$ Masa-Resorte $\to$ Filtro Kalman $\to$ Proyección.
    pub fn update(&mut self, x: f32, y: f32, t_ns: u64, pressure: f32) -> ModelerResult {
        let sanitized = match self.filter.process(x, y, t_ns, pressure) {
            Some(s) => s,
            None => {
                // Muestra filtrada por jitter: devolver último estado conocido
                let predicted_pt = self.predictor.predict();
                return ModelerResult {
                    confirmed_pt: self.last_confirmed_pt,
                    predicted_pt,
                    pressure: self.last_pressure,
                };
            }
        };

        self.last_raw_pt = (sanitized.x, sanitized.y);
        self.last_pressure = sanitized.pressure;

        // 1. Simulación física masa-resorte
        let mass_state = self
            .spring_mass
            .update((sanitized.x, sanitized.y), sanitized.dt_s);
        self.last_confirmed_pt = mass_state.pos;

        // 2. Estimación y proyección con filtro de Kalman
        self.predictor.update(mass_state.pos, sanitized.dt_s);
        let predicted_pt = self.predictor.predict();

        ModelerResult {
            confirmed_pt: mass_state.pos,
            predicted_pt,
            pressure: sanitized.pressure,
        }
    }

    /// Finaliza el trazo al levantar el lápiz (`Touch Up`).
    ///
    /// Asienta la masa virtual suavemente en el punto final con tapering de presión.
    pub fn end_stroke(&mut self) -> ModelerResult {
        let (samples, count) =
            StrokeEndPredictor::settle(&mut self.spring_mass, self.last_raw_pt, self.last_pressure);

        let final_pt = if count > 0 {
            samples[count - 1].pt
        } else {
            self.last_raw_pt
        };

        self.last_confirmed_pt = final_pt;

        ModelerResult {
            confirmed_pt: final_pt,
            predicted_pt: None,
            pressure: 0.0,
        }
    }
}
