// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Herramientas de anotación del visor (Fase 3.5): resaltador con detección
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
//!   pinch/sheet NO se rompen (ver `input.rs`).
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
/// (recorta trazos y subrayados; ver `pdf_core::{split_stroke,trim_highlight}`
/// y `Reader::{begin,update,end}_erase_gesture`). Calibrado en el botón
/// INFERIOR de este boli (0x20).
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
/// pantalla se dibuja a `width × scale` px (ver `Reader::tool_overlay`), así
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

/// Gesto de herramienta EN CURSO (dedo o lápiz bajado con una herramienta
/// activa): puntos en coordenadas de página (puntos PDF). Al levantar,
/// `Reader::end_tool_gesture` lo convierte en una anotación guardada
/// (`Highlight` para el resaltador; para el boli, la POLILÍNEA MUESTREADA
/// de la curva midpoint — cero pop, ver abajo).
///
/// ## Máquina midpoint (Bézier cuadrática por puntos medios)
///
/// Con puntos de control P0..Pn, el trazo en vivo es exactamente la
/// polilínea estampada: tramo recto P0→M1 (Mk = punto medio P(k-1)Pk),
/// curvas cuadráticas M(k-1)→Mk con control P(k-1), y remate M(n-1)→Pn al
/// soltar. La misma polilínea muestreada (`ink_pts`) se PERSISTE en el
/// `Stroke`: el replay lineal de `pdf_core::overlay` la une y reproduce el
/// trazo 1:1 — sin `smooth_polyline` ni re-rasterizado al soltar (cero pop).
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
    /// Vértices crudos en coordenadas de página (el `Down` + cada `Move`).
    /// Para el resaltador: puntos del gesto (bbox). Para el boli: puntos de
    /// control Pk de la máquina midpoint (bbox + degenerate check).
    pub(crate) points: Vec<(f32, f32)>,
    /// Spans de la página PRE-ORDENADOS por Y (B3, solo resaltador):
    /// snapshot del `PageTextCache` en el `Down` (peek sin I/O) para el
    /// preview tentativo por present y el cálculo final al soltar.
    /// Vacío si la página no estaba cacheada (fallback a la vía clásica).
    pub(crate) hl_spans: Vec<TextSpan>,
    /// Último punto medio M(k-1) estampado (inicio de la próxima curva).
    /// `None` hasta el segundo punto (el primer tramo es recto P0→M1).
    pub(crate) prev_mid: Option<(f32, f32)>,
    /// Polilínea MUESTREADA de lo estampado en el frame (coords de página):
    /// se persiste tal cual en el `Stroke` — replay 1:1 (ver doc del tipo).
    pub(crate) ink_pts: Vec<(f32, f32)>,
    /// Presión USI 2.0 normalizada [0,1] por muestra (paralela a `points`):
    /// modula el grosor `w(p) = w_base·(0.6 + 0.8·p)` (plan Área C). Los
    /// drivers sin presión reportan 0.5 (grosor neutro) — ver
    /// `push_with_pressure`. Solo se llena en el boli (Ink).
    pub(crate) pressures: Vec<f32>,
    /// Timestamps en ms monótonos por muestra (paralela a `points`), del
    /// `event_time` NDK del boli: Δt REAL entre muestras para el predictor
    /// (los eventos a 240 Hz batcheados NO llegan uniformes). Base: el
    /// timestamp del Down es t=0 del trazo.
    pub(crate) times_ms: Vec<f32>,
    /// Pipeline de modelado físico de trazo (`google/ink-stroke-modeler`).
    pub(crate) modeler: crate::ink::InkStrokeModeler,
    /// Punto predicho hacia adelante (25–30 ms) por el filtro de Kalman.
    pub(crate) predicted_pt: Option<(f32, f32)>,
}

impl ToolGesture {
    /// Empieza un gesto en `page` con el primer punto (el del `Down`).
    /// `t0_ms`: timestamp NDK del Down (ancla temporal del gesto);
    /// `pressure`: presión inicial normalizada (0.5 si el driver no la da);
    /// `w_base`: grosor base del lápiz configurado.
    pub(crate) fn new(
        page: u32,
        tool: ToolKind,
        pt: (f32, f32),
        t0_ms: f32,
        pressure: f32,
        w_base: f32,
    ) -> Self {
        let mut modeler = crate::ink::InkStrokeModeler::new(w_base);
        let res = modeler.update(pt.0, pt.1, (t0_ms as f64 * 1e6) as u64, pressure);
        Self {
            page,
            tool,
            anchor: pt,
            points: vec![pt],
            prev_mid: None,
            ink_pts: vec![pt],
            pressures: vec![pressure],
            times_ms: vec![t0_ms],
            modeler,
            predicted_pt: res.predicted_pt,
            hl_spans: Vec::new(),
        }
    }

    /// Añade un punto con telemetría USI (presión + timestamp del evento):
    /// en el boli, TODO punto de control lleva presión y Δt real.
    /// Descarta casi-duplicados (distancia < 0.25 pt ≈ < 1 px a zoom 4):
    /// los Move llegan a 240 Hz y un punto por muestra hincharía la
    /// polilínea sin aportar fidelidad. Las series paralelas
    /// `pressures`/`times_ms` se mantienen alineadas con `points` (el
    /// guard corta ANTES de tocar las tres).
    pub(crate) fn push_with_pressure(&mut self, pt: (f32, f32), t_ms: f32, pressure: f32) {
        if let Some(&last) = self.points.last() {
            let d2 = (pt.0 - last.0).powi(2) + (pt.1 - last.1).powi(2);
            if d2 < 0.0625 {
                return;
            }
        }
        self.points.push(pt);
        self.pressures.push(pressure);
        self.times_ms.push(t_ms);
    }

    /// Presión de la última muestra (grosor del próximo tramo): 0.5 si el
    /// gesto no la reportó (vec vacío — nunca en el boli, pero el Highlight
    /// comparte el tipo).
    pub(crate) fn last_pressure(&self) -> f32 {
        self.pressures.last().copied().unwrap_or(0.5)
    }

    /// Actualiza el punto ACTUAL del resaltador (la otra esquina del rect de
    /// selección); para el boli es idéntico a `push`.
    pub(crate) fn set_cur(&mut self, pt: (f32, f32)) {
        self.points = vec![self.anchor, pt];
    }
}
