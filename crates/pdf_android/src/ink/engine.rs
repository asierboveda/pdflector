//! Contrato neutral de captura de tinta en coordenadas de página.
//!
//! Este módulo solo define datos y el ciclo de vida observable por un
//! consumidor. La implementación causal está en [`super::causal`]. No hay
//! predicción opcional: una muestra que no llegó del lápiz nunca aparece en
//! un `InkDelta`.

use std::fmt;

use pdf_core::Color;

/// Muestra real del lápiz, expresada en coordenadas de página.
///
/// `time_ns` es un reloj monotónico suministrado por la capa de entrada. El
/// motor exige que cada muestra posterior tenga un valor estrictamente mayor.
/// La presión finita se normaliza al intervalo `[0, 1]`; los valores no
/// finitos son rechazados por el motor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkSample {
    pub x: f32,
    pub y: f32,
    pub time_ns: u64,
    pub pressure: f32,
}

impl InkSample {
    /// Construye una muestra y normaliza una presión finita fuera de rango.
    ///
    /// Las coordenadas y la presión no finitas se conservan para que el
    /// motor pueda rechazarlas explícitamente, incluso cuando el consumidor
    /// construye la muestra mediante un literal o este constructor.
    pub fn new(x: f32, y: f32, time_ns: u64, pressure: f32) -> Self {
        Self {
            x,
            y,
            time_ns,
            pressure: if pressure.is_finite() {
                pressure.clamp(0.0, 1.0)
            } else {
                pressure
            },
        }
    }

    /// Variante fallible útil para validar una muestra antes de encolarla.
    pub fn try_new(x: f32, y: f32, time_ns: u64, pressure: f32) -> Result<Self, InkError> {
        let sample = Self::new(x, y, time_ns, pressure);
        validate_sample(sample)?;
        Ok(sample)
    }

    /// Devuelve la posición en el formato de puntos de `pdf_core::Stroke`.
    pub fn point(self) -> (f32, f32) {
        (self.x, self.y)
    }
}

/// Apariencia de un trazo persistible por el formato actual de `Stroke`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkStyle {
    pub width: f32,
    pub color: Color,
}

impl InkStyle {
    pub const fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }

    pub fn try_new(width: f32, color: Color) -> Result<Self, InkError> {
        let style = Self::new(width, color);
        validate_style(style)?;
        Ok(style)
    }
}

impl Default for InkStyle {
    fn default() -> Self {
        Self::new(
            2.0,
            Color {
                r: 28,
                g: 32,
                b: 43,
                a: 255,
            },
        )
    }
}

/// Rectángulo de página que debe volver a pintarse.
///
/// El rectángulo incluye la huella completa del pincel y un píxel de margen
/// para el antialiasing. Por eso puede ser mayor que el segmento exacto, pero
/// nunca queda por dentro de él.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirtyRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl DirtyRect {
    const ANTIALIAS_MARGIN: f32 = 1.0;

    pub const fn empty() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }
    }

    /// Bounds conservadores para el segmento entre dos muestras.
    pub fn for_segment(a: InkSample, b: InkSample, brush_width: f32) -> Self {
        let radius = brush_width.max(0.0) * 0.5 + Self::ANTIALIAS_MARGIN;
        let min_x = a.x.min(b.x) - radius;
        let min_y = a.y.min(b.y) - radius;
        let max_x = a.x.max(b.x) + radius;
        let max_y = a.y.max(b.y) + radius;
        Self {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        }
    }

    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let min_x = self.x.min(other.x);
        let min_y = self.y.min(other.y);
        let max_x = (self.x + self.w).max(other.x + other.w);
        let max_y = (self.y + self.h).max(other.y + other.h);
        Self {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        }
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        !self.is_empty()
            && x >= self.x
            && y >= self.y
            && x <= self.x + self.w
            && y <= self.y + self.h
    }

    pub const fn is_empty(self) -> bool {
        self.w == 0.0 && self.h == 0.0
    }
}

/// Rango append-only dentro del almacenamiento interno del trazo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InkRange {
    start: usize,
    end: usize,
}

impl InkRange {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn start(self) -> usize {
        self.start
    }

    pub const fn end(self) -> usize {
        self.end
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Delta append-only: solo contiene un rango `Copy` sobre las muestras
/// aceptadas y el rectángulo incremental que debe repintarse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkDelta {
    pub range: InkRange,
    pub dirty_rect: DirtyRect,
}

impl InkDelta {
    pub const fn empty() -> Self {
        Self {
            range: InkRange::new(0, 0),
            dirty_rect: DirtyRect::empty(),
        }
    }
}

/// Resultado neutral al terminar un trazo.
///
/// Sus puntos y estilo son directamente compatibles con construir un
/// `pdf_core::Stroke` en una capa superior, pero este motor no depende de ese
/// formato ni de persistencia.
#[derive(Clone, Debug, PartialEq)]
pub struct InkFinal {
    pub points: Vec<(f32, f32)>,
    pub style: InkStyle,
    pub dirty_rect: DirtyRect,
}

/// Fallos explícitos del ciclo de vida del motor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkError {
    StrokeAlreadyActive,
    NoActiveStroke,
    NonMonotonicTimestamp { previous: u64, received: u64 },
    InvalidCoordinate,
    InvalidPressure,
    InvalidStyleWidth,
    DegenerateStroke,
}

impl fmt::Display for InkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StrokeAlreadyActive => f.write_str("an ink stroke is already active"),
            Self::NoActiveStroke => f.write_str("there is no active ink stroke"),
            Self::NonMonotonicTimestamp { previous, received } => write!(
                f,
                "ink timestamp must increase strictly (previous={previous}, received={received})"
            ),
            Self::InvalidCoordinate => f.write_str("ink coordinates must be finite"),
            Self::InvalidPressure => f.write_str("ink pressure must be finite"),
            Self::InvalidStyleWidth => {
                f.write_str("ink style width must be finite and non-negative")
            }
            Self::DegenerateStroke => f.write_str("an ink stroke needs at least two samples"),
        }
    }
}

impl std::error::Error for InkError {}

/// Contrato de un motor que conserva exclusivamente muestras causales.
pub trait InkEngine {
    fn begin(&mut self, sample: InkSample) -> Result<InkDelta, InkError>;
    fn push(&mut self, sample: InkSample) -> Result<InkDelta, InkError>;
    fn push_batch(&mut self, samples: &[InkSample]) -> Result<InkDelta, InkError>;
    fn finish(&mut self) -> Result<InkFinal, InkError>;
    fn cancel(&mut self) -> Result<(), InkError>;
    fn samples(&self, range: InkRange) -> Option<&[InkSample]>;
}

pub(crate) fn validate_style(style: InkStyle) -> Result<InkStyle, InkError> {
    if !style.width.is_finite() || style.width < 0.0 {
        return Err(InkError::InvalidStyleWidth);
    }
    Ok(style)
}

pub(crate) fn validate_sample(sample: InkSample) -> Result<InkSample, InkError> {
    if !sample.x.is_finite() || !sample.y.is_finite() {
        return Err(InkError::InvalidCoordinate);
    }
    if !sample.pressure.is_finite() {
        return Err(InkError::InvalidPressure);
    }
    Ok(InkSample::new(
        sample.x,
        sample.y,
        sample.time_ns,
        sample.pressure,
    ))
}
