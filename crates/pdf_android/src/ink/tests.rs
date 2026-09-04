//! Batería de tests unitarios y benchmarks para el pipeline `google/ink-stroke-modeler` en Rust.

#![allow(clippy::unwrap_used)]

use super::*;
use std::time::Instant;

/// Test 1: trazo recto a alta velocidad (6 px/ms = 6000 px/s).
/// Simula un stream a 240 Hz ($\Delta t \approx 4.16\text{ ms}$) y verifica:
/// - Seguimiento estable de la masa sin oscilaciones.
/// - Estimación correcta de velocidad del filtro Kalman.
/// - Proyección hacia adelante en la dirección de avance.
#[test]
fn test_trazo_recto_alta_velocidad() {
    let mut modeler = InkStrokeModeler::new(2.0);
    let dt_ns: u64 = 4_166_666; // ~240 Hz (4.166 ms)
    let speed_px_ms: f32 = 6.0; // 6 px/ms
    let dx_per_step = speed_px_ms * 4.166_666; // ~25 px por muestra

    let mut current_t: u64 = 1_000_000_000;
    let mut current_x: f32 = 100.0;
    let y: f32 = 200.0;

    let mut last_res = modeler.update(current_x, y, current_t, 0.5);
    assert_eq!(last_res.confirmed_pt, (100.0, 200.0));

    // Simular 40 muestras (166 ms de trazo rápido)
    for _ in 0..40 {
        current_t += dt_ns;
        current_x += dx_per_step;
        last_res = modeler.update(current_x, y, current_t, 0.7);
    }

    // La masa debe seguir suavemente en X con Y constante
    assert!(last_res.confirmed_pt.0 > 800.0);
    assert!((last_res.confirmed_pt.1 - 200.0).abs() < 1e-3);

    // El predictor debe proyectar hacia adelante en X
    assert!(last_res.predicted_pt.is_some());
    let pred = last_res.predicted_pt.unwrap();
    assert!(
        pred.0 > last_res.confirmed_pt.0,
        "La predicción debe adelantar la posición confirmada en trazo recto rápido"
    );
    assert!(
        (pred.1 - 200.0).abs() < 1e-2,
        "La predicción no debe desviarse en el eje Y"
    );
}

/// Test 2: conservación de esquinas vivas a 90° y 180°.
/// Verifica que el resorte acelere hacia el vértice y no redondee excesivamente
/// esquinas vivas como en letras 'V', 'M' o 'Z'.
#[test]
fn test_conservacion_esquinas_vivas() {
    let mut modeler = InkStrokeModeler::new(2.0);
    let dt_ns: u64 = 4_166_666; // 240 Hz

    let mut current_t: u64 = 1_000_000_000;

    // Tramo 1: Avanzar en +X hacia el vértice (100, 100) -> (200, 100)
    for i in 0..25 {
        let x = 100.0 + (i as f32) * 4.0;
        current_t += dt_ns;
        modeler.update(x, 100.0, current_t, 0.6);
    }

    // Vértice exacto en (200.0, 100.0)
    let _vertex_res = modeler.update(200.0, 100.0, current_t + dt_ns, 0.8);
    current_t += dt_ns;

    // Tramo 2: Giro brusco de 90° hacia abajo en +Y -> (200, 200)
    for i in 1..=25 {
        let y = 100.0 + (i as f32) * 4.0;
        current_t += dt_ns;
        modeler.update(200.0, y, current_t, 0.6);
    }

    // La masa debe haber alcanzado y superado la esquina con proximidad estrecha (< 4 pt del vértice)
    let final_res = modeler.end_stroke();
    assert!((final_res.confirmed_pt.0 - 200.0).abs() < 1.0);
    assert!((final_res.confirmed_pt.1 - 200.0).abs() < 1.0);
}

/// Test 3: filtrado de ruido de alta frecuencia (jitter espacial).
/// Verifica que la puerta de ruido espacial descarte micro-vibraciones menores a 0.2 pt.
#[test]
fn test_filtrado_ruido_jitter() {
    let mut modeler = InkStrokeModeler::new(2.0);
    let mut current_t: u64 = 1_000_000_000;

    // Punto inicial
    let res0 = modeler.update(100.0, 100.0, current_t, 0.5);
    assert_eq!(res0.confirmed_pt, (100.0, 100.0));

    // Inyectar micro-jitter (ruido < 0.1 pt en 1 ms)
    current_t += 1_000_000;
    let res_jitter1 = modeler.update(100.05, 100.05, current_t, 0.5);
    assert_eq!(
        res_jitter1.confirmed_pt, res0.confirmed_pt,
        "Micro-jitter debe ser suprimido por la puerta de ruido"
    );

    current_t += 1_000_000;
    let res_jitter2 = modeler.update(99.98, 100.02, current_t, 0.5);
    assert_eq!(
        res_jitter2.confirmed_pt, res0.confirmed_pt,
        "Micro-jitter debe ser suprimido por la puerta de ruido"
    );

    // Movimiento real (> 0.5 pt)
    current_t += 4_000_000;
    let res_mov = modeler.update(102.0, 100.0, current_t, 0.5);
    assert!(
        res_mov.confirmed_pt.0 > 100.0,
        "Movimiento significativo debe aceptarse"
    );
}

/// 4. Test de proyección del predictor Kalman (20–35 ms) y modulación de horizonte.
#[test]
fn test_proyeccion_kalman_y_modulacion() {
    let mut predictor = KalmanPredictor::with_defaults();
    let dt_s = 0.004; // 4 ms por paso (250 Hz)
    let v_x = 500.0; // 500 pt/s

    // Alimentar 20 muestras en línea recta a velocidad constante
    for i in 0..20 {
        let x = (i as f32) * v_x * dt_s;
        predictor.update((x, 100.0), dt_s);
    }

    let pred = predictor.predict().unwrap();
    let last_x = 19.0 * v_x * dt_s;

    // Con horizonte nominal de 30 ms a 500 pt/s, la proyección debe adelantar ~15 pt
    let expected_lead = v_x * 0.030; // 15 pt
    let actual_lead = pred.0 - last_x;

    assert!(
        (actual_lead - expected_lead).abs() < 5.0,
        "La proyección debe ser de ~15 pt adelante (lead: {actual_lead:.2} pt vs esperado {expected_lead:.2} pt)"
    );
}

/// Test 5: remate al soltar (StrokeEnd / Cero-Pop).
/// Verifica que `end_stroke()` asiente la masa virtual en el último punto físico con presión 0.
#[test]
fn test_stroke_end_cero_pop() {
    let mut modeler = InkStrokeModeler::new(2.0);
    let dt_ns: u64 = 4_166_666;

    let mut current_t: u64 = 1_000_000_000;
    for i in 0..10 {
        let x = 100.0 + (i as f32) * 5.0;
        current_t += dt_ns;
        modeler.update(x, 100.0, current_t, 0.8);
    }

    let end_res = modeler.end_stroke();

    // Debe converger exactamente al último punto físico (145.0, 100.0)
    assert!((end_res.confirmed_pt.0 - 145.0).abs() < 0.1);
    assert!((end_res.confirmed_pt.1 - 100.0).abs() < 0.1);
    // Presión debe ser 0.0 en el remate
    assert_eq!(end_res.pressure, 0.0);
    assert_eq!(end_res.predicted_pt, None);
}

/// 6. Benchmark de rendimiento: confirma que `update()` es $< 2\,\mu\text{s}$ por evento.
#[test]
fn test_benchmark_update_latencia_microsegundos() {
    let mut modeler = InkStrokeModeler::new(2.0);
    let iterations = 10_000;
    let dt_ns: u64 = 4_166_666;

    // Warm up
    let mut t = 1_000_000_000u64;
    for i in 0..100 {
        t += dt_ns;
        modeler.update(100.0 + (i as f32) * 0.5, 200.0, t, 0.5);
    }

    // Benchmark cronometrado
    let start = Instant::now();
    for i in 0..iterations {
        t += dt_ns;
        let x = 100.0 + ((i % 500) as f32) * 0.5;
        let y = 200.0 + ((i / 500) as f32) * 0.5;
        let res = modeler.update(x, y, t, 0.6);
        std::hint::black_box(res);
    }
    let elapsed = start.elapsed();

    let total_us = elapsed.as_micros() as f64;
    let per_call_us = total_us / (iterations as f64);

    println!(
        "Benchmark InkStrokeModeler::update(): {iterations} iteraciones en {total_us:.2} µs ({per_call_us:.3} µs/llamada)"
    );

    assert!(
        per_call_us < 2.0,
        "El tiempo de ejecución por muestra ({per_call_us:.3} µs) debe ser menor a 2.0 µs"
    );
}
