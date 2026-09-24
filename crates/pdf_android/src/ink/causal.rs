//! Implementación mínima causal del contrato de tinta.
//!
//! Cada salida contiene solo muestras reales aceptadas por orden temporal.
//! Este motor no filtra, suaviza, remata ni extrapola el trazo.

use super::engine::{
    DirtyRect, InkDelta, InkEngine, InkError, InkFinal, InkRange, InkSample, InkStyle,
    validate_sample, validate_style,
};

#[derive(Clone, Debug, PartialEq)]
struct ActiveStroke {
    samples: Vec<InkSample>,
    dirty_rect: DirtyRect,
}

/// Motor determinista de captura causal.
#[derive(Clone, Debug, PartialEq)]
pub struct CausalInkEngine {
    style: InkStyle,
    active: Option<ActiveStroke>,
}

impl CausalInkEngine {
    pub fn new(style: InkStyle) -> Result<Self, InkError> {
        let style = validate_style(style)?;
        Ok(Self {
            style,
            active: None,
        })
    }

    pub fn style(&self) -> InkStyle {
        self.style
    }

    pub fn begin(&mut self, sample: InkSample) -> Result<InkDelta, InkError> {
        <Self as InkEngine>::begin(self, sample)
    }

    pub fn push(&mut self, sample: InkSample) -> Result<InkDelta, InkError> {
        <Self as InkEngine>::push(self, sample)
    }

    pub fn push_batch(&mut self, samples: &[InkSample]) -> Result<InkDelta, InkError> {
        <Self as InkEngine>::push_batch(self, samples)
    }

    pub fn finish(&mut self) -> Result<InkFinal, InkError> {
        <Self as InkEngine>::finish(self)
    }

    pub fn cancel(&mut self) -> Result<(), InkError> {
        <Self as InkEngine>::cancel(self)
    }

    pub fn samples(&self, range: InkRange) -> Option<&[InkSample]> {
        <Self as InkEngine>::samples(self, range)
    }

    /// Accepted samples for the active stroke, in causal order. Renderers may
    /// consume this slice directly instead of maintaining a second point list.
    pub fn active_samples(&self) -> &[InkSample] {
        self.active
            .as_ref()
            .map_or(&[], |active| active.samples.as_slice())
    }

    /// Latest accepted sample while a stroke is active.
    pub fn last_sample(&self) -> Option<InkSample> {
        self.active_samples().last().copied()
    }
}

impl InkEngine for CausalInkEngine {
    fn begin(&mut self, sample: InkSample) -> Result<InkDelta, InkError> {
        if self.active.is_some() {
            return Err(InkError::StrokeAlreadyActive);
        }
        let sample = validate_sample(sample)?;
        let dirty_rect = DirtyRect::for_segment(sample, sample, self.style.width);
        self.active = Some(ActiveStroke {
            samples: vec![sample],
            dirty_rect,
        });
        Ok(InkDelta {
            range: InkRange::new(0, 1),
            dirty_rect,
        })
    }

    fn push(&mut self, sample: InkSample) -> Result<InkDelta, InkError> {
        self.push_batch(std::slice::from_ref(&sample))
    }

    fn push_batch(&mut self, samples: &[InkSample]) -> Result<InkDelta, InkError> {
        let previous_time = self
            .active
            .as_ref()
            .ok_or(InkError::NoActiveStroke)?
            .samples
            .last()
            .map(|sample| sample.time_ns)
            .ok_or(InkError::NoActiveStroke)?;

        // Pre-validar todo el lote evita mutaciones parciales sin crear un
        // Vec temporal. La segunda pasada normaliza y añade las muestras.
        let mut last_time = previous_time;
        for &sample in samples {
            let sample = validate_sample(sample)?;
            if sample.time_ns <= last_time {
                return Err(InkError::NonMonotonicTimestamp {
                    previous: last_time,
                    received: sample.time_ns,
                });
            }
            last_time = sample.time_ns;
        }

        if samples.is_empty() {
            return Ok(InkDelta::empty());
        }

        let brush_width = self.style.width;
        let active = self.active.as_mut().ok_or(InkError::NoActiveStroke)?;
        let start = active.samples.len();
        let mut dirty_rect = DirtyRect::empty();
        let mut previous = active
            .samples
            .last()
            .copied()
            .ok_or(InkError::NoActiveStroke)?;
        for &raw_sample in samples {
            let sample = validate_sample(raw_sample)?;
            dirty_rect = dirty_rect.union(DirtyRect::for_segment(previous, sample, brush_width));
            active.samples.push(sample);
            previous = sample;
        }
        active.dirty_rect = active.dirty_rect.union(dirty_rect);
        Ok(InkDelta {
            range: InkRange::new(start, active.samples.len()),
            dirty_rect,
        })
    }

    fn finish(&mut self) -> Result<InkFinal, InkError> {
        let active = self.active.as_ref().ok_or(InkError::NoActiveStroke)?;
        if active.samples.len() < 2 {
            return Err(InkError::DegenerateStroke);
        }
        let active = self.active.take().ok_or(InkError::NoActiveStroke)?;
        let points = active.samples.iter().map(|sample| sample.point()).collect();
        Ok(InkFinal {
            points,
            style: self.style,
            dirty_rect: active.dirty_rect,
        })
    }

    fn cancel(&mut self) -> Result<(), InkError> {
        if self.active.take().is_none() {
            return Err(InkError::NoActiveStroke);
        }
        Ok(())
    }

    fn samples(&self, range: InkRange) -> Option<&[InkSample]> {
        let active = self.active.as_ref()?;
        if range.start() > range.end() || range.end() > active.samples.len() {
            return None;
        }
        Some(&active.samples[range.start()..range.end()])
    }
}

/// Nombre corto para consumidores que no necesitan distinguir la variante.
pub type CausalEngine = CausalInkEngine;
