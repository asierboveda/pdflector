// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Inercia (fling) del modo Scroll (PR-04): física PURA con el tiempo
//! inyectado (sin reloj de pared; testeable en host).
//!
//! - `VelocityTracker`: velocidad vertical del dedo a partir de las
//!   muestras de Move recientes (`event_time` en ns del `MotionEvent`).
//! - `Fling`: deceleración por fricción exponencial, independiente del
//!   ritmo de frames.
//! - `ScrollFling`: máquina de estados que usa el visor (Down para la
//!   inercia, Move registra, Up la lanza, cada tick la avanza y se detiene
//!   en los extremos del documento sin rebasarlos).

use super::scroll::ScrollLayout;

/// Ventana de muestras para la velocidad de suelta (ns): solo cuenta el
/// último tramo del gesto (un dedo que se para antes de levantar no lanza).
const VELOCITY_WINDOW_NS: i64 = 100_000_000;
/// Muestras retenidas como máximo (Moves a 120-240 Hz en la ventana).
const VELOCITY_MAX_SAMPLES: usize = 32;
/// Velocidad mínima de suelta para deslizar (px/s).
const FLING_MIN_V: f32 = 500.0;
/// Tope de velocidad (px/s): un salto espurio de muestras no dispara la
/// columna al otro extremo del documento.
const FLING_MAX_V: f32 = 12_000.0;
/// Velocidad por debajo de la cual la inercia termina (px/s).
const FLING_STOP_V: f32 = 30.0;
/// Constante de tiempo de la fricción (s): `v(t) = v0·e^(−t/τ)`; el
/// recorrido total es `v0·τ` (8000 px/s → ~3600 px, casi dos pantallas).
const FLING_TAU: f32 = 0.45;

/// Muestras de velocidad del dedo (t en ns, y en px de pantalla).
#[derive(Clone, Debug, Default)]
pub(crate) struct VelocityTracker {
    samples: Vec<(i64, f32)>,
}

impl VelocityTracker {
    /// Olvida las muestras (nuevo gesto).
    pub(crate) fn reset(&mut self) {
        self.samples.clear();
    }

    /// Registra la posición `y` del dedo en el instante `t_ns`.
    pub(crate) fn push(&mut self, t_ns: i64, y: f32) {
        if !y.is_finite() {
            return;
        }
        // Un reloj que retrocede (evento fuera de orden) reinicia la serie.
        if self.samples.last().is_some_and(|&(t, _)| t > t_ns) {
            self.samples.clear();
        }
        self.samples.push((t_ns, y));
        self.samples
            .retain(|&(t, _)| t_ns - t <= VELOCITY_WINDOW_NS);
        if self.samples.len() > VELOCITY_MAX_SAMPLES {
            let extra = self.samples.len() - VELOCITY_MAX_SAMPLES;
            self.samples.drain(..extra);
        }
    }

    /// Velocidad (px/s, + = dedo hacia abajo) en el instante `now_ns`: la
    /// pendiente entre la muestra más vieja y la más reciente de la
    /// ventana que acaba en `now_ns` (0 con menos de dos muestras).
    pub(crate) fn velocity(&self, now_ns: i64) -> f32 {
        let mut recent = self
            .samples
            .iter()
            .filter(|&&(t, _)| t <= now_ns && now_ns - t <= VELOCITY_WINDOW_NS);
        let Some(&(t0, y0)) = recent.next() else {
            return 0.0;
        };
        let Some(&(t1, y1)) = recent.next_back() else {
            return 0.0;
        };
        let dt = (t1 - t0) as f32 / 1e9;
        if dt <= 0.0 { 0.0 } else { (y1 - y0) / dt }
    }
}

/// Inercia en curso: velocidad del contenido (px/s en el eje del dedo).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fling {
    v: f32,
}

impl Fling {
    /// Inercia para una velocidad de suelta `v` (None si el arrastre fue
    /// lento: soltar despacio no desliza).
    pub(crate) fn start(v: f32) -> Option<Fling> {
        if !v.is_finite() || v.abs() < FLING_MIN_V {
            return None;
        }
        Some(Fling {
            v: v.clamp(-FLING_MAX_V, FLING_MAX_V),
        })
    }

    /// Velocidad actual (px/s). Solo la leen los tests: la app solo avanza
    /// la inercia (`step`), así que en el build real no se compila.
    #[cfg(test)]
    pub(crate) fn velocity(&self) -> f32 {
        self.v
    }

    /// ¿Sigue deslizando?
    pub(crate) fn active(&self) -> bool {
        self.v.abs() >= FLING_STOP_V
    }

    /// Avanza `dt` segundos: devuelve el desplazamiento (px, eje del dedo).
    /// Integral EXACTA de la fricción exponencial en el intervalo, así un
    /// frame de 100 ms recorre lo mismo que diez de 10 ms.
    pub(crate) fn step(&mut self, dt: f32) -> f32 {
        if !dt.is_finite() || dt <= 0.0 {
            return 0.0;
        }
        let decay = (-dt / FLING_TAU).exp();
        let d = self.v * FLING_TAU * (1.0 - decay);
        self.v *= decay;
        d
    }
}

/// Estado de inercia del visor en modo Scroll.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScrollFling {
    tracker: VelocityTracker,
    fling: Option<Fling>,
}

impl ScrollFling {
    /// Un dedo toca la pantalla: la inercia se detiene en seco. Devuelve
    /// si había inercia (ese toque solo frena: no es un tap).
    pub(crate) fn touch_down(&mut self) -> bool {
        self.tracker.reset();
        self.fling.take().is_some()
    }

    /// Muestra de un arrastre de un dedo en curso.
    pub(crate) fn track(&mut self, t_ns: i64, y: f32) {
        self.tracker.push(t_ns, y);
    }

    /// Fin del arrastre: lanza la inercia si la suelta fue rápida.
    pub(crate) fn release(&mut self, t_ns: i64, y: f32) -> bool {
        self.tracker.push(t_ns, y);
        let v = self.tracker.velocity(t_ns);
        self.tracker.reset();
        self.fling = Fling::start(v);
        self.fling.is_some()
    }

    /// Detiene la inercia (salto de página, zoom, cambio de modo...).
    pub(crate) fn stop(&mut self) {
        self.tracker.reset();
        self.fling = None;
    }

    /// ¿Hay inercia en curso?
    pub(crate) fn is_active(&self) -> bool {
        self.fling.is_some()
    }

    /// Frame de inercia de `dt` s con la columna `layout` en `off`:
    /// devuelve el offset nuevo (clampado), o None si no hay inercia. Al
    /// llegar a un extremo del documento la inercia termina allí.
    pub(crate) fn frame(&mut self, layout: &ScrollLayout, off: f32, dt: f32) -> Option<f32> {
        let fling = self.fling.as_mut()?;
        // El contenido sigue al dedo: dedo hacia abajo = offset menor.
        let target = off - fling.step(dt);
        let next = layout.clamp_offset(target);
        if (next - target).abs() > 1e-3 || !fling.active() {
            self.fling = None; // extremo del documento o fricción agotada
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const MS: i64 = 1_000_000;

    fn layout() -> ScrollLayout {
        ScrollLayout {
            win_w: 1200.0,
            win_h: 2000.0,
            margin: 20.0,
            gap: 20.0,
            ref_w: 1160.0,
            ref_h: 1640.0,
            zoom: 1.0,
            pages: 10,
            sizes: Arc::from(Vec::new()),
        }
    }

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn velocity_comes_from_recent_move_samples() {
        let mut t = VelocityTracker::default();
        t.push(0, 100.0);
        t.push(10 * MS, 120.0);
        t.push(20 * MS, 140.0);
        assert!(
            close(t.velocity(20 * MS), 2000.0, 1.0),
            "{}",
            t.velocity(20 * MS)
        );
        // Muestras viejas (antes de una pausa) no cuentan.
        let mut t = VelocityTracker::default();
        t.push(0, 0.0);
        t.push(500 * MS, 500.0);
        t.push(510 * MS, 510.0);
        t.push(520 * MS, 520.0);
        assert!(
            close(t.velocity(520 * MS), 1000.0, 1.0),
            "{}",
            t.velocity(520 * MS)
        );
        // Dedo quieto antes de levantar: sin velocidad.
        assert_eq!(t.velocity(800 * MS), 0.0);
        // Hacia arriba: velocidad negativa.
        let mut t = VelocityTracker::default();
        t.push(0, 900.0);
        t.push(16 * MS, 800.0);
        assert!(t.velocity(16 * MS) < -5000.0);
    }

    #[test]
    fn slow_releases_do_not_fling() {
        assert!(Fling::start(200.0).is_none());
        assert!(Fling::start(-200.0).is_none());
        assert!(Fling::start(f32::NAN).is_none());
        let f = Fling::start(3000.0).expect("rápido: desliza");
        assert!(f.active());
        // Velocidades absurdas (muestras de un salto) se acotan.
        let f = Fling::start(1e7).unwrap();
        assert!(f.velocity() < 1e5);
    }

    #[test]
    fn fling_decelerates_and_stops_independently_of_frame_rate() {
        let mut a = Fling::start(3000.0).unwrap();
        let mut b = a;
        let d_a = a.step(0.1);
        let d_b: f32 = (0..10).map(|_| b.step(0.01)).sum();
        assert!(d_a > 0.0 && close(d_a, d_b, 0.5), "{d_a} vs {d_b}");
        assert!(close(a.velocity(), b.velocity(), 0.5));
        assert!(a.velocity() < 3000.0);
        // La velocidad nunca crece y la inercia termina en un tiempo finito.
        let mut f = Fling::start(-4000.0).unwrap();
        let mut last = f.velocity().abs();
        let mut total = 0.0;
        let mut frames = 0;
        while f.active() {
            total += f.step(0.016);
            assert!(f.velocity().abs() <= last);
            last = f.velocity().abs();
            frames += 1;
            assert!(frames < 400, "la inercia no termina");
        }
        assert!(total < -500.0, "recorrido {total}");
    }

    #[test]
    fn a_fast_swipe_keeps_gliding_after_release() {
        let mut s = ScrollFling::default();
        s.touch_down();
        // Dedo hacia ARRIBA rápido: el documento avanza (offset crece).
        for i in 0..6 {
            s.track(i * 10 * MS, 1500.0 - i as f32 * 40.0);
        }
        assert!(s.release(60 * MS, 1260.0));
        assert!(s.is_active());
        let l = layout();
        let off1 = s.frame(&l, 3000.0, 0.016).expect("desliza");
        assert!(off1 > 3000.0, "{off1}");
        let off2 = s.frame(&l, off1, 0.016).unwrap();
        // Decelera: el segundo frame avanza menos que el primero.
        assert!(off2 - off1 < off1 - 3000.0 && off2 > off1);
    }

    #[test]
    fn a_new_touch_stops_the_fling() {
        let mut s = ScrollFling::default();
        s.track(0, 0.0);
        s.track(10 * MS, 50.0);
        assert!(s.release(20 * MS, 100.0));
        s.touch_down();
        assert!(!s.is_active());
        assert_eq!(s.frame(&layout(), 3000.0, 0.016), None);
        // Un arrastre lento posterior no relanza nada.
        s.track(100 * MS, 100.0);
        assert!(!s.release(300 * MS, 101.0));
    }

    #[test]
    fn fling_stops_at_the_document_ends_without_overshoot() {
        let l = layout();
        let max = l.content_h() - l.win_h;
        // Hacia el principio (dedo bajando rápido) desde cerca del borde.
        let mut s = ScrollFling::default();
        s.track(0, 0.0);
        s.track(10 * MS, 80.0);
        assert!(s.release(20 * MS, 160.0));
        let mut off = 50.0;
        let mut frames = 0;
        while let Some(o) = s.frame(&l, off, 0.016) {
            assert!(o >= 0.0 && o <= off, "{o}");
            off = o;
            frames += 1;
            assert!(frames < 400);
        }
        assert_eq!(off, 0.0);
        assert!(!s.is_active());
        // Hacia el final.
        let mut s = ScrollFling::default();
        s.track(0, 1000.0);
        s.track(10 * MS, 920.0);
        assert!(s.release(20 * MS, 840.0));
        let mut off = max - 30.0;
        while let Some(o) = s.frame(&l, off, 0.016) {
            assert!(o <= max && o >= off);
            off = o;
        }
        assert!(close(off, max, 1e-3));
    }

    #[test]
    fn the_touch_that_stops_a_glide_is_not_a_tap() {
        let mut s = ScrollFling::default();
        assert!(!s.touch_down(), "sin inercia: un toque normal");
        s.track(0, 0.0);
        s.track(10 * MS, 60.0);
        assert!(s.release(20 * MS, 120.0));
        assert!(s.touch_down(), "frenó una inercia: se traga el tap");
        assert!(!s.touch_down());
        // Inercia ya agotada por la fricción: el toque vuelve a ser un tap.
        s.track(100 * MS, 0.0);
        s.track(110 * MS, 60.0);
        assert!(s.release(120 * MS, 120.0));
        while s.frame(&layout(), 5000.0, 0.016).is_some() {}
        assert!(!s.touch_down());
    }
}
