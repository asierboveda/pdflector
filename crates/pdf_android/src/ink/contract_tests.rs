use super::causal::CausalInkEngine;
use super::engine::{InkError, InkSample, InkStyle};
use pdf_core::{Color, Stroke};

fn sample(x: f32, y: f32, time_ns: u64, pressure: f32) -> InkSample {
    InkSample::new(x, y, time_ns, pressure)
}

fn style() -> InkStyle {
    InkStyle::new(
        4.0,
        Color {
            r: 10,
            g: 20,
            b: 30,
            a: 255,
        },
    )
}

fn engine() -> CausalInkEngine {
    CausalInkEngine::new(style()).expect("valid style")
}

#[test]
fn line_emits_only_the_accepted_real_samples() {
    let mut engine = engine();

    let first = sample(10.0, 20.0, 100, 0.25);
    let delta = engine
        .begin(first)
        .expect("begin should accept first sample");

    assert_eq!(engine.samples(delta.range).unwrap(), &[first]);
    assert!(!delta.dirty_rect.is_empty());

    let second = sample(11.0, 20.0, 101, 0.5);
    let delta = engine
        .push(second)
        .expect("push should accept later sample");
    assert_eq!(engine.samples(delta.range).unwrap(), &[second]);
}

#[test]
fn batch_is_append_only_and_preserves_order() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();

    let batch = vec![
        sample(2.0, 1.0, 2, 0.5),
        sample(4.0, 2.0, 3, 0.6),
        sample(6.0, 3.0, 4, 0.7),
    ];
    let delta = engine.push_batch(&batch).expect("batch should be accepted");

    assert_eq!(engine.samples(delta.range).unwrap(), batch.as_slice());
    assert_eq!(
        engine.active_samples(),
        &[sample(0.0, 0.0, 1, 0.5), batch[0], batch[1], batch[2]]
    );
    assert_eq!(engine.last_sample(), batch.last().copied());
    assert!(delta.dirty_rect.contains(0.0, 0.0));
    assert!(delta.dirty_rect.contains(6.0, 3.0));
}

#[test]
fn active_samples_and_last_sample_expose_the_single_causal_sequence() {
    let mut engine = engine();
    let points = [sample(0.0, 0.0, 1, 0.5), sample(2.0, 1.0, 2, 0.6)];
    engine.begin(points[0]).unwrap();
    engine.push(points[1]).unwrap();

    assert_eq!(engine.active_samples(), &points);
    assert_eq!(engine.last_sample(), Some(points[1]));

    engine.finish().unwrap();
    assert!(engine.active_samples().is_empty());
    assert_eq!(engine.last_sample(), None);
}

#[test]
fn corner_is_not_smoothed_or_replaced() {
    let mut engine = engine();
    let points = [
        sample(0.0, 0.0, 1, 0.5),
        sample(10.0, 0.0, 2, 0.5),
        sample(10.0, 10.0, 3, 0.5),
    ];
    engine.begin(points[0]).unwrap();
    let mut emitted = vec![points[0]];
    let delta = engine.push_batch(&points[1..]).unwrap();
    emitted.extend(engine.samples(delta.range).unwrap());

    assert_eq!(emitted, points);
}

#[test]
fn jitter_is_emitted_as_is_without_smoothing() {
    let mut engine = engine();
    let points = [
        sample(100.0, 100.0, 1, 0.5),
        sample(100.05, 99.98, 2, 0.5),
        sample(99.97, 100.03, 3, 0.5),
    ];
    engine.begin(points[0]).unwrap();
    let delta = engine.push_batch(&points[1..]).unwrap();

    assert_eq!(engine.samples(delta.range).unwrap(), &points[1..]);
}

#[test]
fn repeated_and_decreasing_timestamps_are_rejected_deterministically() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 10, 0.5)).unwrap();

    let repeated = engine.push(sample(1.0, 0.0, 10, 0.5));
    assert_eq!(
        repeated,
        Err(InkError::NonMonotonicTimestamp {
            previous: 10,
            received: 10
        })
    );
    let decreasing = engine.push(sample(1.0, 0.0, 9, 0.5));
    assert_eq!(
        decreasing,
        Err(InkError::NonMonotonicTimestamp {
            previous: 10,
            received: 9
        })
    );

    let accepted = engine.push(sample(1.0, 0.0, 11, 0.5));
    assert!(
        accepted.is_ok(),
        "a rejected sample must not advance the clock"
    );
}

#[test]
fn invalid_batch_is_rejected_atomically_without_partial_append() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();

    let batch = [sample(1.0, 0.0, 2, 0.5), sample(2.0, 0.0, 2, 0.5)];
    assert!(engine.push_batch(&batch).is_err());

    let delta = engine.push(sample(1.0, 0.0, 2, 0.5)).unwrap();
    assert_eq!(delta.range.len(), 1);
    assert_eq!(engine.samples(delta.range).unwrap()[0].x, 1.0);
}

#[test]
fn finite_pressure_is_clamped_and_non_finite_pressure_is_rejected() {
    let mut engine = engine();
    let first = engine.begin(sample(0.0, 0.0, 1, 2.0)).unwrap();
    assert_eq!(engine.samples(first.range).unwrap()[0].pressure, 1.0);

    let invalid = engine.push(sample(1.0, 0.0, 2, f32::NAN));
    assert_eq!(invalid, Err(InkError::InvalidPressure));
    let accepted = engine.push(sample(1.0, 0.0, 2, -1.0)).unwrap();
    assert_eq!(engine.samples(accepted.range).unwrap()[0].pressure, 0.0);
}

#[test]
fn finish_returns_the_current_stroke_shape_without_smoothing() {
    let mut engine = engine();
    let points = [
        sample(1.0, 2.0, 1, 0.2),
        sample(3.0, 4.0, 2, 0.8),
        sample(5.0, 6.0, 3, 0.4),
    ];
    engine.begin(points[0]).unwrap();
    engine.push_batch(&points[1..]).unwrap();

    let final_stroke = engine.finish().expect("a two-point stroke should finish");
    let stroke = Stroke::new(
        final_stroke.points.clone(),
        final_stroke.style.width,
        final_stroke.style.color,
    )
    .expect("final data must be compatible with Stroke");
    assert_eq!(
        stroke,
        Stroke::new(
            points.iter().map(|point| (point.x, point.y)).collect(),
            4.0,
            style().color,
        )
        .unwrap()
    );
}

#[test]
fn double_finish_is_an_explicit_error() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();
    engine.push(sample(1.0, 0.0, 2, 0.5)).unwrap();
    engine.finish().unwrap();

    assert_eq!(engine.finish(), Err(InkError::NoActiveStroke));
}

#[test]
fn finish_rejects_a_degenerate_single_sample_stroke() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();

    assert_eq!(engine.finish(), Err(InkError::DegenerateStroke));
    assert!(engine.push(sample(1.0, 0.0, 2, 0.5)).is_ok());
}

#[test]
fn cancel_drops_the_stroke_and_delivers_no_final() {
    let mut engine = engine();
    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();
    engine.push(sample(1.0, 0.0, 2, 0.5)).unwrap();
    engine
        .cancel()
        .expect("active stroke should be cancellable");

    assert_eq!(engine.finish(), Err(InkError::NoActiveStroke));
    assert!(engine.begin(sample(5.0, 5.0, 3, 0.5)).is_ok());
}

#[test]
fn lifecycle_errors_are_explicit() {
    let mut engine = engine();
    assert_eq!(
        engine.push(sample(1.0, 1.0, 1, 0.5)),
        Err(InkError::NoActiveStroke)
    );
    assert_eq!(engine.cancel(), Err(InkError::NoActiveStroke));

    engine.begin(sample(0.0, 0.0, 1, 0.5)).unwrap();
    assert_eq!(
        engine.begin(sample(1.0, 1.0, 2, 0.5)),
        Err(InkError::StrokeAlreadyActive)
    );
}

#[test]
fn dirty_rect_is_conservative_for_brush_coverage() {
    let mut engine = engine();
    engine.begin(sample(10.0, 20.0, 1, 0.5)).unwrap();
    let delta = engine.push(sample(30.0, 40.0, 2, 0.5)).unwrap();

    assert!(delta.dirty_rect.contains(10.0 - 2.0, 20.0 - 2.0));
    assert!(delta.dirty_rect.contains(30.0 + 2.0, 40.0 + 2.0));
}

#[test]
fn invalid_style_width_is_rejected_before_capture() {
    let negative = InkStyle::new(-1.0, style().color);
    assert_eq!(
        CausalInkEngine::new(negative),
        Err(InkError::InvalidStyleWidth)
    );

    let non_finite = InkStyle::new(f32::NAN, style().color);
    assert_eq!(
        CausalInkEngine::new(non_finite),
        Err(InkError::InvalidStyleWidth)
    );
    assert_eq!(
        InkStyle::try_new(-1.0, style().color),
        Err(InkError::InvalidStyleWidth)
    );
}
