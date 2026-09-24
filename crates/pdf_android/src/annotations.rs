// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Herramientas de anotación del visor: resaltador con detección
//! de texto y boli (tinta freehand), barra de herramientas discreta.
//!
//! El modelo de datos (`AnnotationSet`, `Stroke`, `Highlight`, `Color`), la
//! lógica (selección de líneas bajo el trazo y suavizado, `selection`/
//! `annotations`) y la persistencia (sidecar SQLite, `store.rs`) viven en
//! pdf_core; este módulo solo aporta el ESTADO de las herramientas del visor:
//!
//! - [`ToolKind`]: herramienta activa en el visor (Navegar / Resaltar /
//!   Boli). Con una herramienta distinta de Navegar el arrastre con un dedo
//!   (o el lápiz de la tablet) dibuja en vez de navegar; los gestos de tap/
//!   pinch/sheet NO se rompen (ver `input/`).
//! - [`ToolGesture`]: gesto de herramienta EN CURSO (dedo/lápiz bajado):
//!   puntos y ancla **en coordenadas de página** (puntos PDF, f32 — el mismo
//!   espacio que `Document::page_size`), aún NO añadido al `AnnotationSet`.
//!   Se añade al levantar (`Reader::end_tool_gesture`).
//! - Paletas: color por defecto del boli (`DEFAULT_INK_COLOR`); el
//!   resaltador usa `pdf_core::HIGHLIGHT_COLOR` (amarillo rotulador,
//!   translúcido). Grosor/color del boli: valores persistidos
//!   (`tool_state.json`); sin controles táctiles (era la barra, eliminada).
//!
//! # ¿Por qué coordenadas de página?
//!
//! La arquitectura (AGENTS.md §4.3) exige anotaciones **vectoriales en
//! coordenadas de página**, dibujadas como capa sobre el bitmap cacheado y
//! nunca rasterizadas en él. Guardar los puntos en puntos PDF los mantiene
//! pegados a la página en cualquier zoom/scroll (la transformación
//! página↔pantalla solo depende de la escala `cover × zoom` y de la posición
//! de la página; ver `Reader::screen_to_page`).

use android_activity::input::ButtonState;
use pdf_core::{Color, TextSpan};

/// Botón "UP" del boli: alterna el modo del boli Ink ↔ Highlight
/// (`Reader::toggle_pen_mode`), también con el boli en el AIRE (los eventos
/// `MotionAction::ButtonPress` llegan sin contacto).
///
/// CALIBRACIÓN (Fase A, ver CHANGELOG 2026-08-25): en este boli el botón
/// SUPERIOR (el del toggle) reporta `AMOTION_EVENT_BUTTON_STYLUS_SECONDARY`
/// (0x40) y el INFERIOR (el del borrado) `STYLUS_PRIMARY` (0x20) — INVERTIDO
/// respecto al estándar Android. Verificado en el logcat `pen_buttons` de la
/// TCL 9469X (ButtonPress en el aire y `button_state` en contacto). Si otro
/// boli reportara distinto, se intercambian ESTAS dos constantes, no el flujo.
pub(crate) const PEN_BTN_MODE: ButtonState = ButtonState(0x40);

/// Botón "DOWN" del boli: MANTENIDO + boli apoyado = BORRAR con GOMA real
/// (recorta trazos parcialmente y elimina subrayados completos; ver
/// `pdf_core::split_stroke` y `Reader::{begin,update,end}_erase_gesture`).
/// Calibrado en el botón INFERIOR de este boli (0x20).
pub(crate) const PEN_BTN_ERASE: ButtonState = ButtonState(0x20);

/// Radio de hit-test del borrado en puntos DE PÁGINA: distancia punto→seg-
/// mento < este radio (+ `width/2` del trazo; ver `pdf_core::stroke_hit`).
/// 8 pt ≈ el ancho de un trazo grueso de boli + margen cómodo de borrado.
pub(crate) const ERASE_HIT_RADIUS_PT: f32 = 8.0;

/// Expansión del hit-test del borrado contra HIGHLIGHTS: el punto debe caer
/// dentro del rect del resaltador expandido 4 pt (ver `pdf_core::
/// highlight_hit`) — un subrayado fino se borra fácil sin tocar exacto.
pub(crate) const ERASE_HL_PAD_PT: f32 = 4.0;

/// Modo del boli (control total SIN menús: el boli dibuja/subraya según este
/// modo; el botón UP lo alterna). Se persiste en `tool_state.json` (campo
/// "mode") — retrocompatible: un fichero viejo sin el campo carga como
/// `Ink` (`#[serde(default)]` no hace falta porque `load_pen_mode` parsea
/// con fallback a `Ink`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PenMode {
    /// Boli: tinta freehand.
    Ink,
    /// Resaltador: subraya el texto bajo el trazo.
    Highlight,
}

impl PenMode {
    /// Etiqueta del modo para el toast del toggle ("✏️ Pen" / "🖍️ Highlighter")
    pub(crate) fn label(self) -> &'static str {
        match self {
            PenMode::Ink => "✏️ Pen",
            PenMode::Highlight => "🖍️ Highlighter",
        }
    }
}

/// Grosor del trazo nuevo del boli en puntos PDF (PDF points, 1/72"). En
/// pantalla se dibuja a `width × scale` px, así
/// que a zoom 1 (~2 px/punto en la tablet) un trazo de 2 pt ≈ 4 px — un
/// rotulador fino. El grosor vive en unidades de página (no de pantalla)
/// porque la anotación es vectorial: un trazo de 2 pt ocupa el mismo área
/// del papel en cualquier zoom, como la tinta real.
pub(crate) const STROKE_WIDTH_PT: f32 = 2.0;

/// Color por defecto del boli: negro azulado cálido (tinta de bolígrafo
/// sobre papel), opaco (se dibuja tal cual sobre la página; en modo oscuro
/// la página se invierte pero la tinta conserva su color — la capa de
/// anotaciones es independiente del modo de visualización).
pub(crate) const DEFAULT_INK_COLOR: Color = Color {
    r: 28,
    g: 32,
    b: 43,
    a: 255,
};

/// Herramienta de anotación activa en el visor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolKind {
    /// Sin herramienta: el visor se comporta como siempre (tap = página,
    /// pinch = zoom, long-press = selección de texto, pull = sheet).
    Navigate,
    /// Resaltador: arrastrar sobre texto selecciona las líneas bajo el trazo
    /// y crea una `Highlight` alineada al texto (`pdf_core::selection`).
    Highlight,
    /// Boli: el arrastre dibuja tinta freehand (`Stroke` suavizado con
    /// `pdf_core::annotations::smooth_polyline` al soltar).
    Ink,
}

/// Gesto de herramienta EN CURSO. Ink conserva las muestras causales en el
/// motor y Highlight conserva sus puntos de gesto; ambos se expresan en
/// coordenadas de página. Al finalizar se crea la anotación correspondiente.
#[derive(Clone, Debug)]
pub(crate) struct ToolGesture {
    /// Página (0-based) sobre la que se dibuja. Fija en el `Down`; el trazo
    /// nunca cambia de página aunque el dedo se desplace por encima de otras
    /// (los puntos fuera de la página se recortan en el render).
    pub(crate) page: u32,
    /// Herramienta que originó el gesto (decide la anotación resultante).
    pub(crate) tool: ToolKind,
    /// Ancla del gesto en página (el punto del Down): para el resaltador
    /// define una esquina del rect de selección.
    pub(crate) anchor: (f32, f32),
    /// Puntos de Highlight en coordenadas de página. Ink usa exclusivamente
    /// `ink_engine.active_samples()` como fuente de geometría.
    pub(crate) points: Vec<(f32, f32)>,
    /// Spans de la página PRE-ORDENADOS por Y (B3, solo resaltador):
    /// snapshot del `PageTextCache` en el `Down` (peek sin I/O) para el
    /// preview tentativo por present y el cálculo final al soltar.
    /// Vacío si la página no estaba cacheada (fallback a la vía clásica).
    pub(crate) hl_spans: Vec<TextSpan>,
    /// Única fuente de muestras Ink; Wet y Stroke final leen estos mismos datos.
    pub(crate) ink_engine: Option<crate::ink::CausalInkEngine>,
    /// Este gesto ya escribió segmentos al overlay nativo y debe confirmarlos
    /// en Up o cancelarlos si el sistema cancela el gesto.
    pub(crate) ink_overlay_used: bool,
    /// Ruta decidida en Down. Si el overlay no estaba listo entonces, todo el
    /// gesto permanece en Wet; no se migra a mitad del trazo.
    pub(crate) ink_overlay_route: bool,
}

impl ToolGesture {
    /// Empieza un gesto en `page` con el primer punto (el del `Down`).
    /// `t0_ns`: timestamp NDK del Down (ancla temporal absoluta del gesto);
    /// `pressure`: presión inicial normalizada (0.5 si el driver no la da);
    /// `w_base`: grosor base del lápiz configurado.
    pub(crate) fn try_new(
        page: u32,
        tool: ToolKind,
        pt: (f32, f32),
        t0_ns: u64,
        pressure: f32,
        w_base: f32,
        color: Color,
    ) -> Result<Self, crate::ink::InkError> {
        let sample = crate::ink::InkSample::new(pt.0, pt.1, t0_ns, pressure);
        let ink_engine = if tool == ToolKind::Ink {
            let mut engine =
                crate::ink::CausalInkEngine::new(crate::ink::InkStyle::new(w_base, color))?;
            engine.begin(sample)?;
            Some(engine)
        } else {
            None
        };
        Ok(Self {
            page,
            tool,
            anchor: pt,
            points: if tool == ToolKind::Highlight {
                vec![pt]
            } else {
                Vec::new()
            },
            hl_spans: Vec::new(),
            ink_engine,
            ink_overlay_used: false,
            ink_overlay_route: false,
        })
    }

    /// Añade una muestra Ink real. Timestamps repetidos o decrecientes se
    /// descartan explícitamente; no se fabrican tiempos ni coordenadas.
    pub(crate) fn push_ink_sample(
        &mut self,
        sample: crate::ink::InkSample,
    ) -> Result<Option<crate::ink::InkDelta>, crate::ink::InkError> {
        let Some(engine) = self.ink_engine.as_mut() else {
            return Ok(None);
        };
        if engine
            .last_sample()
            .is_some_and(|previous| sample.time_ns <= previous.time_ns)
        {
            return Ok(None);
        }
        engine.push(sample).map(Some)
    }

    /// Actualiza el punto actual del resaltador (la otra esquina del rect de
    /// selección).
    pub(crate) fn set_cur(&mut self, pt: (f32, f32)) {
        self.points = vec![self.anchor, pt];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_gesture_keeps_only_newer_real_samples_and_does_not_filter_by_distance() {
        let mut gesture = ToolGesture::try_new(
            0,
            ToolKind::Ink,
            (10.0, 20.0),
            100,
            0.5,
            2.0,
            DEFAULT_INK_COLOR,
        )
        .unwrap();

        assert!(
            gesture
                .push_ink_sample(crate::ink::InkSample::new(99.0, 99.0, 100, 0.8))
                .unwrap()
                .is_none()
        );
        assert!(
            gesture
                .push_ink_sample(crate::ink::InkSample::new(10.0, 20.0, 101, 0.6))
                .unwrap()
                .is_some()
        );

        let samples = gesture.ink_engine.as_ref().unwrap().active_samples();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].point(), (10.0, 20.0));
        assert_eq!(samples[1].point(), (10.0, 20.0));
        assert_eq!(samples[1].time_ns, 101);
        assert_eq!(samples[1].pressure, 0.6);
    }
}
