//! Filtro y sanitizador de entrada para eventos del digitalizador táctil / stylus (240 Hz).
//!
//! Basado en el pipeline de entrada de `google/ink-stroke-modeler` (Apache-2.0):
//! - Descarta muestras con coordenadas inválidas (NaN, Inf).
//! - Filtra saltos temporales erráticos ($\Delta t \le 0$ o desbordamientos).
//! - Puerta de ruido espacial para suprimir micro-jitter del sensor ($d < 0.2\text{ pt}$).
//! - Normaliza la presión en $[0.0, 1.0]$ sin alocaciones dinámicas.

/// Muestra sanitizada lista para alimentar el simulador físico.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SanitizedInput {
    /// Posición X en coordenadas de página / pantalla.
    pub x: f32,
    /// Posición Y en coordenadas de página / pantalla.
    pub y: f32,
    /// Timestamp en segundos desde el inicio del trazo.
    pub t_s: f32,
    /// Intervalo de tiempo ($\Delta t$) respecto a la muestra anterior en segundos.
    pub dt_s: f32,
    /// Presión normalizada en el rango $[0.0, 1.0]$.
    pub pressure: f32,
}

/// Umbral cuadrado de la puerta de ruido espacial ($0.2\text{ pt} \implies d^2 = 0.04$).
pub const NOISE_GATE_MIN_DIST_SQ: f32 = 0.04;

/// Intervalo máximo admisible entre muestras antes de considerar un stall temporal (0.5 s).
pub const MAX_VALID_DT_S: f32 = 0.5;

/// Intervalo mínimo estándar para evitar división por cero en derivadas (1 µs).
pub const MIN_VALID_DT_S: f32 = 1e-6;

/// Estado del filtro de entrada (memoria en stack $O(1)$).
#[derive(Clone, Copy, Debug)]
pub struct InputFilter {
    /// Timestamp del primer evento del trazo (ancla t=0 en nanosegundos).
    t0_ns: Option<u64>,
    /// Timestamp del evento anterior en nanosegundos.
    last_t_ns: Option<u64>,
    /// Última posición aceptada por la puerta de ruido.
    last_pos: Option<(f32, f32)>,
    /// Último valor de presión sanitizado.
    last_pressure: f32,
}

impl Default for InputFilter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl InputFilter {
    /// Crea un nuevo filtro de entrada listo para recibir un trazo.
    #[inline]
    pub const fn new() -> Self {
        Self {
            t0_ns: None,
            last_t_ns: None,
            last_pos: None,
            last_pressure: 0.5,
        }
    }

    /// Reinicia el estado del filtro para un nuevo trazo.
    #[inline]
    pub fn reset(&mut self) {
        self.t0_ns = None;
        self.last_t_ns = None;
        self.last_pos = None;
        self.last_pressure = 0.5;
    }

    /// Procesa una muestra bruta del sensor.
    ///
    /// Devuelve `Some(SanitizedInput)` si la muestra es válida y supera la puerta
    /// de ruido espacial y temporal, o `None` si debe ser descartada.
    pub fn process(
        &mut self,
        x: f32,
        y: f32,
        t_ns: u64,
        raw_pressure: f32,
    ) -> Option<SanitizedInput> {
        // 1. Validación de números finitos
        if !x.is_finite() || !y.is_finite() {
            return None;
        }

        // 2. Normalización de presión
        let pressure = if raw_pressure.is_finite() && raw_pressure >= 0.0 {
            raw_pressure.clamp(0.0, 1.0)
        } else {
            self.last_pressure
        };

        // 3. Validación y anclaje temporal
        let t0 = match self.t0_ns {
            Some(t0) => t0,
            None => {
                self.t0_ns = Some(t_ns);
                self.last_t_ns = Some(t_ns);
                self.last_pos = Some((x, y));
                self.last_pressure = pressure;
                return Some(SanitizedInput {
                    x,
                    y,
                    t_s: 0.0,
                    dt_s: 0.0,
                    pressure,
                });
            }
        };

        let last_t = self.last_t_ns.unwrap_or(t_ns);
        if t_ns < last_t {
            // Reloj no monotónico o evento desordenado: descartar
            return None;
        }

        let raw_dt_ns = t_ns.saturating_sub(last_t);
        let mut dt_s = (raw_dt_ns as f64 * 1e-9) as f32;

        if dt_s < MIN_VALID_DT_S {
            // Muestras duplicadas en el mismo timestamp
            return None;
        }

        if dt_s > MAX_VALID_DT_S {
            // Salto temporal grande (pausa del usuario o stall del hilo UI): clamp defensivo
            dt_s = MAX_VALID_DT_S;
        }

        let t_s = (t_ns.saturating_sub(t0) as f64 * 1e-9) as f32;

        // 4. Puerta de ruido espacial (Spatial Noise Gate)
        if let Some((lx, ly)) = self.last_pos {
            let dx = x - lx;
            let dy = y - ly;
            let dist_sq = dx * dx + dy * dy;

            // Si el desplazamiento es insignificante (< 0.2 pt) y el tiempo es corto, descartar jitter
            if dist_sq < NOISE_GATE_MIN_DIST_SQ && dt_s < 0.05 {
                return None;
            }
        }

        // Muestra aceptada: actualizar estado
        self.last_t_ns = Some(t_ns);
        self.last_pos = Some((x, y));
        self.last_pressure = pressure;

        Some(SanitizedInput {
            x,
            y,
            t_s,
            dt_s,
            pressure,
        })
    }
}
