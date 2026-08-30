//! Predictor cinemático de Kalman para proyección de trazo hacia el futuro (20–35 ms).
//!
//! Basado en el `KalmanPredictor` de `google/ink-stroke-modeler` (Apache-2.0):
//! - Mantiene filtros de Kalman de 3 estados (posición, velocidad, aceleración) para X e Y.
//! - Utiliza el modelo de ruido de aceleración blanca continua (Continuous White-Noise Acceleration).
//! - Modulación dinámica del horizonte de predicción: reduce $\Delta t$ cuando la aceleración
//!   centrípeta o curvatura es alta para evitar sobre-elongaciones en esquinas vivas.
//! - Ejecución analítica exacta en stack $O(1)$ sin alocaciones dinámicas ni librerías externas.

/// Horizonte máximo de predicción hacia adelante en segundos (30 ms).
pub const DEFAULT_PREDICTION_HORIZON_S: f32 = 0.030;

/// Horizonte mínimo en segundos durante giros bruscos (5 ms).
pub const MIN_PREDICTION_HORIZON_S: f32 = 0.005;

/// Desviación estándar de ruido de medición en puntos PDF ($\approx 0.5\text{ pt}$).
pub const MEASUREMENT_NOISE_SIGMA: f32 = 0.5;

/// Desviación estándar de ruido de aceleración del proceso ($\text{pt/s}^2$).
pub const PROCESS_ACCEL_NOISE_SIGMA: f32 = 400.0;

/// Filtro de Kalman 1D de 3 estados: $\mathbf{x} = [p, v, a]^T$.
#[derive(Clone, Copy, Debug)]
pub struct KalmanFilter1D {
    /// Vector de estado: [0] = posición, [1] = velocidad, [2] = aceleración.
    pub state: [f32; 3],
    /// Matriz de covarianza de error 3x3 simétrica (almacenada como 6 elementos o 3x3 plano).
    pub p: [[f32; 3]; 3],
}

impl KalmanFilter1D {
    /// Inicializa el filtro con una posición inicial conocida.
    pub fn new(pos: f32) -> Self {
        let r = MEASUREMENT_NOISE_SIGMA;
        Self {
            state: [pos, 0.0, 0.0],
            p: [[r * r, 0.0, 0.0], [0.0, 100.0, 0.0], [0.0, 0.0, 1000.0]],
        }
    }

    /// Paso de predicción y actualización con una nueva medición `z` transcurrido `dt_s`.
    pub fn update(&mut self, z: f32, dt: f32) {
        if dt <= 0.0 {
            return;
        }

        // 1. Matriz de transición F
        // F = [1, dt, 0.5*dt^2]
        //     [0,  1,       dt]
        //     [0,  0,        1]
        let dt2 = dt * dt;
        let half_dt2 = 0.5 * dt2;

        let s0 = self.state[0] + self.state[1] * dt + self.state[2] * half_dt2;
        let s1 = self.state[1] + self.state[2] * dt;
        let s2 = self.state[2];

        // 2. Covarianza del ruido de proceso Q (modelo de aceleración continua)
        let q_var = PROCESS_ACCEL_NOISE_SIGMA * PROCESS_ACCEL_NOISE_SIGMA;
        let dt3 = dt2 * dt;
        let dt4 = dt2 * dt2;
        let dt5 = dt4 * dt;

        let q00 = q_var * (dt5 / 20.0);
        let q01 = q_var * (dt4 / 8.0);
        let q02 = q_var * (dt3 / 6.0);
        let q11 = q_var * (dt3 / 3.0);
        let q12 = q_var * (dt2 / 2.0);
        let q22 = q_var * dt;

        // P_prior = F * P * F^T + Q
        let p = self.p;
        let fp00 = p[0][0] + p[1][0] * dt + p[2][0] * half_dt2;
        let fp01 = p[0][1] + p[1][1] * dt + p[2][1] * half_dt2;
        let fp02 = p[0][2] + p[1][2] * dt + p[2][2] * half_dt2;

        let fp10 = p[1][0] + p[2][0] * dt;
        let fp11 = p[1][1] + p[2][1] * dt;
        let fp12 = p[1][2] + p[2][1] * dt;

        let fp20 = p[2][0];
        let fp21 = p[2][1];
        let fp22 = p[2][2];

        let mut p_prior = [
            [
                fp00 + fp01 * dt + fp02 * half_dt2 + q00,
                fp01 + fp02 * dt + q01,
                fp02 + q02,
            ],
            [
                fp10 + fp11 * dt + fp12 * half_dt2 + q01,
                fp11 + fp12 * dt + q11,
                fp12 + q12,
            ],
            [
                fp20 + fp21 * dt + fp22 * half_dt2 + q02,
                fp21 + fp22 * dt + q12,
                fp22 + q22,
            ],
        ];

        // 3. Medición H = [1, 0, 0]
        let r = (MEASUREMENT_NOISE_SIGMA * MEASUREMENT_NOISE_SIGMA).max(1e-4);
        let residual = z - s0;
        let s_cov = p_prior[0][0] + r;

        if !s_cov.is_finite() || s_cov < 1e-6 {
            return;
        }

        let inv_s = 1.0 / s_cov;
        let k0 = (p_prior[0][0] * inv_s).clamp(-2.0, 2.0);
        let k1 = (p_prior[1][0] * inv_s).clamp(-100.0, 100.0);
        let k2 = (p_prior[2][0] * inv_s).clamp(-1000.0, 1000.0);

        // 4. Actualización del vector de estado con clamps defensivos
        let next_pos = s0 + k0 * residual;
        let next_vel = (s1 + k1 * residual).clamp(-10_000.0, 10_000.0);
        let next_acc = (s2 + k2 * residual).clamp(-100_000.0, 100_000.0);

        if next_pos.is_finite() && next_vel.is_finite() && next_acc.is_finite() {
            self.state[0] = next_pos;
            self.state[1] = next_vel;
            self.state[2] = next_acc;
        } else {
            self.state[0] = z;
            self.state[1] = 0.0;
            self.state[2] = 0.0;
        }

        // 5. Actualización simétrica y positiva de covarianza P = (I - K*H) * P_prior
        p_prior[0][0] = (p_prior[0][0] - k0 * p_prior[0][0]).max(1e-4);
        p_prior[0][1] -= k0 * p_prior[0][1];
        p_prior[0][2] -= k0 * p_prior[0][2];

        p_prior[1][0] = p_prior[0][1];
        p_prior[1][1] = (p_prior[1][1] - k1 * p_prior[0][1]).max(1e-4);
        p_prior[1][2] -= k1 * p_prior[0][2];

        p_prior[2][0] = p_prior[0][2];
        p_prior[2][1] = p_prior[1][2];
        p_prior[2][2] = (p_prior[2][2] - k2 * p_prior[0][2]).max(1e-4);

        self.p = p_prior;
    }

    /// Proyecta la posición en un horizonte temporal futuro $\Delta t$.
    #[inline]
    pub fn predict_future(&self, dt: f32) -> f32 {
        let res = self.state[0] + self.state[1] * dt + 0.5 * self.state[2] * dt * dt;
        if res.is_finite() { res } else { self.state[0] }
    }
}

/// Predictor cinemático bidimensional.
#[derive(Clone, Copy, Debug)]
pub struct KalmanPredictor {
    filter_x: Option<KalmanFilter1D>,
    filter_y: Option<KalmanFilter1D>,
    horizon_s: f32,
}

impl KalmanPredictor {
    /// Crea un nuevo predictor con el horizonte por defecto (30 ms).
    #[inline]
    pub const fn new(horizon_s: f32) -> Self {
        Self {
            filter_x: None,
            filter_y: None,
            horizon_s,
        }
    }

    /// Crea un predictor con los parámetros estándar (30 ms).
    #[inline]
    pub const fn with_defaults() -> Self {
        Self::new(DEFAULT_PREDICTION_HORIZON_S)
    }

    /// Reinicia los filtros.
    #[inline]
    pub fn reset(&mut self) {
        self.filter_x = None;
        self.filter_y = None;
    }

    /// Actualiza el estado con la posición confirmada de la masa virtual.
    pub fn update(&mut self, pos: (f32, f32), dt_s: f32) {
        let fx = match self.filter_x.as_mut() {
            Some(f) => {
                f.update(pos.0, dt_s);
                f
            }
            None => {
                self.filter_x = Some(KalmanFilter1D::new(pos.0));
                return;
            }
        };

        let _ = fx; // borrow checker

        if let Some(fy) = self.filter_y.as_mut() {
            fy.update(pos.1, dt_s);
        } else {
            self.filter_y = Some(KalmanFilter1D::new(pos.1));
        }
    }

    /// Calcula la posición predicha proyectada hacia adelante.
    ///
    /// Modula dinámicamente el horizonte de predicción según la aceleración
    /// centrípeta instantánea para evitar sobre-proyecciones en esquinas.
    pub fn predict(&self) -> Option<(f32, f32)> {
        let fx = self.filter_x.as_ref()?;
        let fy = self.filter_y.as_ref()?;

        let pos_x = fx.state[0];
        let pos_y = fy.state[0];

        if !pos_x.is_finite() || !pos_y.is_finite() {
            return None;
        }

        let vx = fx.state[1];
        let vy = fy.state[1];
        let ax = fx.state[2];
        let ay = fy.state[2];

        let speed = (vx * vx + vy * vy).sqrt();
        if !speed.is_finite() || speed < 1.0 {
            // Reposo o movimiento imperceptible: sin proyección
            return Some((pos_x, pos_y));
        }

        // Aceleración perpendicular (centrípeta): a_perp = |v_x * a_y - v_y * a_x| / speed
        let a_perp = (vx * ay - vy * ax).abs() / speed.max(1e-3);

        // Modulación dinámica del horizonte: a mayor aceleración angular, menor horizonte
        let horizon_decay = (1.0 + a_perp * 0.005).max(1.0);
        let effective_horizon =
            (self.horizon_s / horizon_decay).clamp(MIN_PREDICTION_HORIZON_S, self.horizon_s);

        let pred_x = fx.predict_future(effective_horizon);
        let pred_y = fy.predict_future(effective_horizon);

        if pred_x.is_finite() && pred_y.is_finite() {
            let max_lead = speed * self.horizon_s * 1.5;
            let dx = (pred_x - pos_x).clamp(-max_lead, max_lead);
            let dy = (pred_y - pos_y).clamp(-max_lead, max_lead);
            Some((pos_x + dx, pos_y + dy))
        } else {
            Some((pos_x, pos_y))
        }
    }
}
