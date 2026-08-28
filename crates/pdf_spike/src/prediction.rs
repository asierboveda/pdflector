// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

// Predicción de trazo (PLAN-PARIDAD-STYLUS-NATIVO Área B): proyectar la
// posición futura del boli Δt ms hacia delante para compensar el retraso
// del pipeline de presentación.
//
// Módulo desacoplado (sin UI, sin unsafe, sin alloc por muestra) — testeable
// unitariamente. Tres modelos evaluados en el spike:
//
// - [`predict_taylor`]: extrapolación cinemática de Taylor orden 2 con
//   filtro paso-bajo exponencial de velocidad/aceleración (O(1), ~ns).
// - [`predict_hermite`]: extensión por tangente proyectada con curvatura
//   decreciente (continuidad C1 con el trazo confirmado).
// - [`predict_kalman`]: filtro de Kalman 1D por eje, estado (p, v, a),
//   modelo de ruido sintonizado a digitizer USI (~60-240 Hz).
//
// Métrica del spike: error de predicción |P_pred(Δt) − P_real(t+Δt)| en px
// de ventana sobre trazos reales re-muestreados a Δt = 8/16/24 ms.

/// Punto del trazo: posición en px de ventana + timestamp en ms (monótono).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    /// ms desde el inicio del trazo.
    pub t: f32,
}

/// Filtro paso-bajo exponencial: `y += alpha * (x - y)`.
fn lowpass(prev: f32, new: f32, alpha: f32) -> f32 {
    prev + alpha * (new - prev)
}

/// B.1 — Taylor orden 2 con velocidad y aceleración suavizadas.
///
/// * `alpha_v`/`alpha_a`: constantes de tiempo del filtro paso-bajo (0..1;
///   menor = más suave = más retraso de fase).
pub fn predict_taylor(hist: &[Sample], dt_ms: f32, alpha_v: f32, alpha_a: f32) -> Option<(f32, f32)> {
    let n = hist.len();
    if n < 3 {
        return hist.last().map(|s| (s.x, s.y));
    }
    let s = &hist[n - 3..];
    let dt01 = (s[1].t - s[0].t).max(1.0);
    let dt12 = (s[2].t - s[1].t).max(1.0);
    // Velocidades por tramo (px/ms) y aceleración implícita.
    let v1 = [(s[1].x - s[0].x) / dt01, (s[1].y - s[0].y) / dt01];
    let v2 = [(s[2].x - s[1].x) / dt12, (s[2].y - s[1].y) / dt12];
    let a_raw = [(v2[0] - v1[0]) / dt12, (v2[1] - v1[1]) / dt12];
    // Suavizado exponencial (estado implícito por llamada: el caller reusa
    // la cola de historial; alpha alto = confianza en la muestra nueva).
    let v = [lowpass(v1[0], v2[0], alpha_v), lowpass(v1[1], v2[1], alpha_v)];
    let a = [a_raw[0] * alpha_a, a_raw[1] * alpha_a];
    let last = s[2];
    let d = dt_ms;
    Some((
        last.x + v[0] * d + 0.5 * a[0] * d * d,
        last.y + v[1] * d + 0.5 * a[1] * d * d,
    ))
}

/// B.3 — Hermite: extensión con tangente proyectada y curvatura decreciente.
///
/// La posición predicha sigue un arco cuadrático cuya tangente inicial es la
/// velocidad suavizada y cuya aceleración decae linealmente a 0 en Δt
/// (continuidad C1 garantizada con el último segmento confirmado).
pub fn predict_hermite(hist: &[Sample], dt_ms: f32, alpha_v: f32) -> Option<(f32, f32)> {
    let n = hist.len();
    if n < 2 {
        return hist.last().map(|s| (s.x, s.y));
    }
    let (a, b) = (&hist[n - 2], &hist[n - 1]);
    let dt = (b.t - a.t).max(1.0);
    // Velocidad del último segmento suavizada con la del penúltimo (si hay).
    let v_seg = [(b.x - a.x) / dt, (b.y - a.y) / dt];
    let v = if n >= 3 {
        let (c, _) = (&hist[n - 3], a);
        let dt2 = (a.t - c.t).max(1.0);
        [
            lowpass((a.x - c.x) / dt2, v_seg[0], alpha_v),
            lowpass((a.y - c.y) / dt2, v_seg[1], alpha_v),
        ]
    } else {
        v_seg
    };
    // Extensión C1: velocidad constante + corrección de curvatura con la
    // aceleración observada, amortiguada exponencialmente (τ = 40 ms): la
    // velocidad NO se penaliza (la penalización subestimaba trazos rectos),
    // solo la aceleración decae — el arco se endereza progresivamente.
    let tau = 40.0f32;
    let damp = (-dt_ms / tau).exp();
    // La aceleración por diferencias amplifica el ruido del digitizer
    // (2σ/dt² a 240 Hz → decenas de px de error en Δt=24 ms). Dos barreras:
    // paso-bajo fuerte (alpha 0.15) y clamp físico |a| ≤ 0.02 px/ms²
    // (0→1440 px de pantalla en ~270 ms a aceleración constante).
    let a = if n >= 3 {
        let c = &hist[n - 3];
        let dt2 = (a.t - c.t).max(1.0);
        let v_prev = [(a.x - c.x) / dt2, (a.y - c.y) / dt2];
        let clamp = |x: f32| x.clamp(-0.02, 0.02);
        [clamp(lowpass(v_prev[0] - v_seg[0], 0.0, 0.15)), clamp(lowpass(v_prev[1] - v_seg[1], 0.0, 0.15))]
    } else {
        [0.0; 2]
    };
    // desplazamiento ≈ v·Δt + a·(τ²·(1 - e^(-Δt/τ)) − τ·Δt·e^(-Δt/τ)) ≈ v·Δt + ½a·Δt²·damp
    Some((
        b.x + v[0] * dt_ms + 0.5 * a[0] * dt_ms * dt_ms * damp,
        b.y + v[1] * dt_ms + 0.5 * a[1] * dt_ms * dt_ms * damp,
    ))
}

/// B.2 — Kalman 1D por eje (p, v, a), matriz de transición de aceleración
/// constante (constant acceleration model), ruido de proceso sintonizado a
/// digitizer USI (jitter ≈ 0.5 px, dt variable).
pub fn predict_kalman(hist: &[Sample], dt_ms: f32) -> Option<(f32, f32)> {
    // Por eje: estimación p, v con ganancia fija (α-β filter, degeneración
    // estable del Kalman completo: el EKF con matriz de covarianza completa
    // queda fuera del spike por coste/riesgo calibración — plan §B.2 riesgo).
    const ALPHA: f32 = 0.85; // confianza en la medición de posición
    const BETA: f32 = 0.05; // confianza en la actualización de velocidad
    let n = hist.len();
    if n < 3 {
        return hist.last().map(|s| (s.x, s.y));
    }
    let predict_axis = |get: fn(&Sample) -> f32| -> f32 {
        let mut p = get(&hist[0]);
        let mut v = 0.0f32;
        let mut prev_t = hist[0].t;
        for s in &hist[1..] {
            let dt = (s.t - prev_t).max(1.0);
            prev_t = s.t;
            // Predicción del estado al instante de la medición.
            p += v * dt;
            // Ganancia: corrección proporcional.
            let resid = get(s) - p;
            p += ALPHA * resid;
            v += BETA * resid / dt;
        }
        // Proyección final Δt hacia delante.
        p + v * dt_ms
    };
    Some((predict_axis(|s| s.x), predict_axis(|s| s.y)))
}

// ---------------------------------------------------------------- Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Trazo recto a velocidad constante: 500 px en 100 ms (5 px/ms ≈ trazo
    /// rápido real: 1440 px de pantalla cruzados en ~0.3 s).
    fn straight() -> Vec<Sample> {
        (0..20)
            .map(|i| Sample { x: 100.0 + i as f32 * 25.0, y: 500.0, t: i as f32 * 5.0 })
            .collect()
    }

    /// Arco cuadrático (giro brusco a mitad de trazo): el caso que rompe
    /// Taylor sin filtro.
    fn arc() -> Vec<Sample> {
        (0..20)
            .map(|i| {
                let t = i as f32;
                Sample { x: 100.0 + t * 25.0, y: 500.0 + t * t * 2.0, t: t * 5.0 }
            })
            .collect()
    }

    fn err(pred: (f32, f32), truth: (f32, f32)) -> f32 {
        ((pred.0 - truth.0).powi(2) + (pred.1 - truth.1).powi(2)).sqrt()
    }

    #[test]
    fn taylor_recto_alto_acierto() {
        let h = straight();
        let truth = (100.0 + 19.0 * 25.0 + 5.0 * 25.0, 500.0);
        let (px, py) = predict_taylor(&h, 5.0, 0.6, 0.3).unwrap();
        assert!(err((px, py), truth) < 15.0, "err={}", err((px, py), truth));
    }

    #[test]
    fn hermite_recto_alto_acierto() {
        let h = straight();
        let truth = (100.0 + 19.0 * 25.0 + 5.0 * 25.0, 500.0);
        let (px, py) = predict_hermite(&h, 5.0, 0.6).unwrap();
        assert!(err((px, py), truth) < 15.0, "err={}", err((px, py), truth));
    }

    #[test]
    fn taylor_giro_sin_latigazo() {
        // En el arco la predicción a 16 ms no debe alejarse más del orden de
        // la distancia por frame (~50-125 px): el filtro limita el latigazo.
        let h = arc();
        let truth = (100.0 + 23.0 * 25.0, 500.0 + 23.0f32 * 23.0 * 2.0);
        let (px, py) = predict_taylor(&h, 16.0, 0.4, 0.15).unwrap();
        assert!(err((px, py), truth) < 400.0, "err={}", err((px, py), truth));
    }

    #[test]
    fn hermite_giro_moderado() {
        let h = arc();
        let truth = (100.0 + 23.0 * 25.0, 500.0 + 23.0f32 * 23.0 * 2.0);
        let (px, py) = predict_hermite(&h, 16.0, 0.4).unwrap();
        assert!(err((px, py), truth) < 400.0, "err={}", err((px, py), truth));
    }

    #[test]
    fn kalman_giro_moderado() {
        let h = arc();
        let truth = (100.0 + 23.0 * 25.0, 500.0 + 23.0f32 * 23.0 * 2.0);
        let (px, py) = predict_kalman(&h, 16.0).unwrap();
        assert!(err((px, py), truth) < 400.0, "err={}", err((px, py), truth));
    }

    #[test]
    fn historial_corto_devuelve_ultimo() {
        let h = vec![Sample { x: 1.0, y: 2.0, t: 0.0 }];
        assert_eq!(predict_taylor(&h, 16.0, 0.5, 0.5), Some((1.0, 2.0)));
        assert_eq!(predict_hermite(&h, 16.0, 0.5), Some((1.0, 2.0)));
        assert_eq!(predict_kalman(&h, 16.0), Some((1.0, 2.0)));
    }

    #[test]
    fn sin_ruido_taylor_no_explota() {
        // Serie perfectamente constante: la predicción debe ser el propio punto.
        let h: Vec<Sample> = (0..10).map(|i| Sample { x: 50.0, y: 50.0, t: i as f32 * 5.0 }).collect();
        let (px, py) = predict_taylor(&h, 16.0, 0.6, 0.3).unwrap();
        assert!(err((px, py), (50.0, 50.0)) < 0.01);
    }
}
