// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Predicción de trazo (Fase 1 de PLAN-PARIDAD-STYLUS-NATIVO, ADR-006):
//! proyecta la posición futura del boli Δt ms hacia delante para compensar el
//! retraso del pipeline de presentación (1 vsync = 16 ms a 60 Hz).
//!
//! Predictor: **Hermite amortiguado** (ganador del Spike 2, datos en
//! ADR-006) — velocidad con paso-bajo + aceleración clampada, continuidad
//! C1 garantizada con el último segmento confirmado (el tramo efímero
//! arranca EXACTAMENTE en el último punto confirmado con su tangente).
//!
//! Contrato de rendimiento: cero alocaciones, cálculo O(1) sobre las últimas
//! 3 muestras (arrays fijos `[f32; 2]`) — coste medido < 1 µs por evento
//! (informe de la fase). Sin `unwrap`/`expect`; predicción degenerada
//! (historial corto) = último punto conocido.

/// Punto del trazo en coords de PÁGINA + timestamp en ms (monótono, del
/// `event_time` NDK del boli — no se asume intervalo uniforme).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub x: f32,
    pub y: f32,
    /// ms monótonos (base `System.nanoTime()` del evento NDK, re-escalada).
    pub t: f32,
}

/// Límite físico de aceleración (px/ms²): de 0 a 1440 px de pantalla en
/// ~270 ms a aceleración constante. Por encima, ruido del digitizer.
pub const MAX_ACCEL_PX_MS2: f32 = 0.02;

/// Constante del paso-bajo de velocidad (0..1): confianza en la muestra
/// nueva. 0.4 = suave sin retraso de fase apreciable (Spike 2).
pub const ALPHA_V: f32 = 0.4;

/// Horizonte de predicción por defecto: 1 vsync a 60 Hz (ADR-006).
pub const PREDICTION_DT_MS: f32 = 16.0;

#[inline]
fn lowpass(prev: f32, new: f32, alpha: f32) -> f32 {
    prev + alpha * (new - prev)
}

/// Predictor Hermite amortiguado: posición futura `Δt` ms tras la última
/// muestra. Desplazamiento = `v·Δt + ½·a·Δt²·e^(−Δt/τ)` con τ = 40 ms
/// (la curvatura decae exponencialmente: el arco se endereza — sin
/// "latigazos" en giros). Devuelve `None` solo sin historial.
///
/// C1: la tangente en el punto de salida es la velocidad suavizada del
/// último segmento → el segmento efímero empalma sin quiebro.
pub fn predict_hermite(hist: &[Sample], dt_ms: f32) -> Option<(f32, f32)> {
    let n = hist.len();
    let last = hist.last()?;
    if n < 2 {
        return Some((last.x, last.y));
    }
    let (a, b) = (&hist[n - 2], last);
    let dt = (b.t - a.t).max(1.0);
    // Velocidad del último segmento, suavizada con la del anterior (si hay).
    let v_seg = [(b.x - a.x) / dt, (b.y - a.y) / dt];
    let v = if n >= 3 {
        let c = &hist[n - 3];
        let dt2 = (a.t - c.t).max(1.0);
        [
            lowpass((a.x - c.x) / dt2, v_seg[0], ALPHA_V),
            lowpass((a.y - c.y) / dt2, v_seg[1], ALPHA_V),
        ]
    } else {
        v_seg
    };
    // Aceleración implícita del último cambio de velocidad, clampada al
    // límite físico (por diferencias amplifica el ruido: 2σ/dt² — el Spike 2
    // midió 48 px de error sin el clamp).
    let a_lim = if n >= 3 {
        let c = &hist[n - 3];
        let dt2 = (a.t - c.t).max(1.0);
        let v_prev = [(a.x - c.x) / dt2, (a.y - c.y) / dt2];
        [
            (v_seg[0] - v_prev[0]).clamp(-MAX_ACCEL_PX_MS2, MAX_ACCEL_PX_MS2),
            (v_seg[1] - v_prev[1]).clamp(-MAX_ACCEL_PX_MS2, MAX_ACCEL_PX_MS2),
        ]
    } else {
        [0.0; 2]
    };
    let damp = (-dt_ms / 40.0f32).exp();
    Some((
        b.x + v[0] * dt_ms + 0.5 * a_lim[0] * dt_ms * dt_ms * damp,
        b.y + v[1] * dt_ms + 0.5 * a_lim[1] * dt_ms * dt_ms * damp,
    ))
}

/// Curva de respuesta de presión (plan §Área C): grosor en px de página.
///
/// `w(p) = w_base · (0.6 + 0.8·p)` con p ∈ [0,1] clamped: p=0 → 0.6·w_base
/// (trazo fino sin desaparecer), p=1 → 1.4·w_base (trazo fuerte), p=0.5 →
/// w_base (punto neutro). Lineal: control predecible; la sigmoide queda
/// como calibración futura (plan §Área C.1).
#[inline]
pub fn pressure_width(w_base: f32, pressure: f32) -> f32 {
    w_base * (0.6 + 0.8 * pressure.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Trazo recto a 6 px/ms (trazo rápido real): predicción a 16 ms acierta
    /// con error < 2 px (Spike 2: mediana 1.5 px).
    #[test]
    fn recto_rapido_acierta() {
        let h: Vec<Sample> = (0..20)
            .map(|i| Sample {
                x: 100.0 + i as f32 * 24.0,
                y: 500.0,
                t: i as f32 * 4.0,
            })
            .collect();
        let (px, py) = predict_hermite(&h, 16.0).unwrap();
        let truth_x = 100.0 + 19.0 * 24.0 + 6.0 * 16.0;
        assert!((px - truth_x).abs() < 2.0, "px={} truth={}", px, truth_x);
        assert!(py.abs() - 500.0 < 0.01);
    }

    /// Giro de 90°: la predicción se desvía (predecir lo impredecible) pero
    /// decae con el amortiguamiento — sin latigazo (> 60 px a 16 ms).
    #[test]
    fn giro_90_sin_latigazo() {
        let mut h: Vec<Sample> = (0..12)
            .map(|i| Sample {
                x: 100.0 + i as f32 * 24.0,
                y: 500.0,
                t: i as f32 * 4.0,
            })
            .collect();
        for i in 0..8 {
            let t = 48.0 + i as f32 * 4.0;
            h.push(Sample {
                x: 364.0,
                y: 500.0 + (i + 1) as f32 * 24.0,
                t,
            });
        }
        let (px, py) = predict_hermite(&h, 16.0).unwrap();
        // Última muestra: (364, 692) bajando a 6 px/ms → pred razonable.
        assert!((px - 364.0).abs() < 60.0, "px={}", px);
        assert!(py > 692.0 && py < 792.0, "py={}", py);
    }

    /// Giro de 180° (retroceso): sin latigazo — el clamp limita a.
    #[test]
    fn giro_180_sin_latigazo() {
        let mut h: Vec<Sample> = (0..12)
            .map(|i| Sample {
                x: 100.0 + i as f32 * 24.0,
                y: 500.0,
                t: i as f32 * 4.0,
            })
            .collect();
        for i in 0..8 {
            let t = 48.0 + i as f32 * 4.0;
            h.push(Sample {
                x: 364.0 - (i + 1) as f32 * 24.0,
                y: 500.0,
                t,
            });
        }
        let (px, py) = predict_hermite(&h, 16.0).unwrap();
        // Retrocediendo a -6 px/ms desde x=172: la predicción continúa hacia
        // atrás x ≈ 172 − 6·16 = 76 (amortiguada por el clamp de a; auditoría
        // fix B: la cota anterior px > 180 era un error del test — el valor
        // correcto es ~76, no un latigazo). Rango correcto: (50, 170).
        assert!(px > 50.0 && px < 170.0, "px={}", px);
        assert!((py - 500.0).abs() < 60.0);
    }

    /// Ruido del digitizer (σ = 0.5 px): la predicción a 16 ms sigue en el
    /// orden de px, no explota (el clamp de aceleración es la barrera).
    #[test]
    fn ruido_no_explota() {
        let noise = [0.4, -0.5, 0.3, -0.2, 0.6, -0.4, 0.1, -0.6, 0.5, -0.3];
        let h: Vec<Sample> = (0..10)
            .map(|i| Sample {
                x: 100.0 + i as f32 * 24.0,
                y: 500.0 + noise[i],
                t: i as f32 * 4.0,
            })
            .collect();
        let (px, py) = predict_hermite(&h, 16.0).unwrap();
        assert!(
            (px - (100.0 + 9.0 * 24.0 + 6.0 * 16.0)).abs() < 8.0,
            "px={}",
            px
        );
        assert!((py - 500.0).abs() < 8.0, "py={}", py);
    }

    /// Historial corto / vacío: degradación a último punto, sin panic.
    #[test]
    fn historial_corto_degrada() {
        assert_eq!(predict_hermite(&[], 16.0), None);
        let h = [Sample {
            x: 7.0,
            y: 8.0,
            t: 0.0,
        }];
        assert_eq!(predict_hermite(&h, 16.0), Some((7.0, 8.0)));
    }

    /// Curva de presión: neutro en p=0.5, límites 0.6/1.4, clamp fuera de rango.
    #[test]
    fn curva_presion() {
        let w = 4.0f32;
        assert!((pressure_width(w, 0.5) - w).abs() < 1e-4);
        assert!((pressure_width(w, 0.0) - 0.6 * w).abs() < 1e-4);
        assert!((pressure_width(w, 1.0) - 1.4 * w).abs() < 1e-4);
        assert!((pressure_width(w, -0.7) - 0.6 * w).abs() < 1e-4);
        assert!((pressure_width(w, 2.0) - 1.4 * w).abs() < 1e-4);
    }

    /// C1: la tangente del segmento efímero en el punto de salida coincide
    /// con la velocidad suavizada del último segmento confirmado.
    #[test]
    fn continuidad_c1() {
        let h: Vec<Sample> = (0..6)
            .map(|i| Sample {
                x: i as f32 * 24.0,
                y: 0.0,
                t: i as f32 * 4.0,
            })
            .collect();
        let (px, py) = predict_hermite(&h, PREDICTION_DT_MS).unwrap();
        // Velocidad confirmada 6 px/ms → a 1 ms del punto de salida la
        // tangente avanza ~6 px en x y ~0 en y (curvatura despreciable en
        // tramo recto). Verificación de pendiente del primer 1 ms:
        let slope_pred = (py - h[5].y) / (px - h[5].x);
        assert!(slope_pred.abs() < 0.02, "slope={}", slope_pred);
    }
}
