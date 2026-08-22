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
//! - Paletas: color por defecto del boli (`DEFAULT_INK_COLOR`) y colores de
//!   la paleta del boli (`INK_PALETTE`, que cicla el botón "●" de la barra);
//!   el resaltador usa `pdf_core::HIGHLIGHT_COLOR` (amarillo rotulador,
//!   translúcido).
//!
//! # ¿Por qué coordenadas de página?
//!
//! La arquitectura (AGENTS.md §4.3) exige anotaciones **vectoriales en
//! coordenadas de página**, dibujadas como capa sobre el bitmap cacheado y
//! nunca rasterizadas en él. Guardar los puntos en puntos PDF los mantiene
//! pegados a la página en cualquier zoom/scroll (la transformación
//! página↔pantalla solo depende de la escala `cover × zoom` y de la posición
//! de la página; ver `Reader::screen_to_page`).

use pdf_core::Color;

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

/// Paleta del botón "●" de la barra de herramientas (ciclar color del boli):
/// tonos cálidos-medio (warm-neutral) que se leen bien sobre papel blanco y
/// sobre la página invertida en modo oscuro. Cicla Boli: negro azulado →
/// marrón sepia → azul apagado → vino.
pub(crate) const INK_PALETTE: [Color; 4] = [
    DEFAULT_INK_COLOR,
    Color {
        r: 122,
        g: 74,
        b: 39,
        a: 255,
    },
    Color {
        r: 33,
        g: 77,
        b: 132,
        a: 255,
    },
    Color {
        r: 122,
        g: 36,
        b: 61,
        a: 255,
    },
];

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
/// (`Highlight` para el resaltador, `Stroke` suavizado para el boli).
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
    /// Vértices en coordenadas de página (el `Down` + cada `Move`).
    pub(crate) points: Vec<(f32, f32)>,
}

impl ToolGesture {
    /// Empieza un gesto en `page` con el primer punto (el del `Down`).
    pub(crate) fn new(page: u32, tool: ToolKind, pt: (f32, f32)) -> Self {
        Self {
            page,
            tool,
            anchor: pt,
            points: vec![pt],
        }
    }

    /// Añade un punto, descartando los casi coincidentes con el anterior
    /// (distancia < 0.25 pt ≈ < 1 px a zoom 4): los Move del táctil/lápiz
    /// llegan a 60-120 Hz y un punto por evento hincharía la polilínea sin
    /// aportar fidelidad (el trazo sigue siendo suave y ligero de serializar).
    pub(crate) fn push(&mut self, pt: (f32, f32)) {
        if let Some(&last) = self.points.last() {
            let d2 = (pt.0 - last.0).powi(2) + (pt.1 - last.1).powi(2);
            if d2 < 0.0625 {
                return;
            }
        }
        self.points.push(pt);
    }

    /// Actualiza el punto ACTUAL del resaltador (la otra esquina del rect de
    /// selección); para el boli es idéntico a `push`.
    pub(crate) fn set_cur(&mut self, pt: (f32, f32)) {
        self.points = vec![self.anchor, pt];
    }
}
