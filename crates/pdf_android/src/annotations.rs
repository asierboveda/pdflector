// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Anotaciones a mano (Stroke) en pdf_android: estado del trazo en curso y
//! paleta de colores.
//!
//! El modelo de datos (`AnnotationSet`, `Stroke`, `Color`) y la persistencia
//! (sidecar SQLite, `store.rs`) viven en pdf_core; este módulo solo aportaba
//! el estado de dibujo del visor:
//!
//! - [`ActiveStroke`]: el trazo que se está dibujando (dedo bajado), con sus
//!   puntos **en coordenadas de página** (puntos PDF, f32 — el mismo espacio
//!   que `Document::page_size`), aún NO añadido al `AnnotationSet`. Se añadía
//!   al levantar el dedo (`Reader::finish_stroke`, eliminado).
//! - `DEFAULT_STROKE_COLOR` / `STROKE_PALETTE`: color por defecto y colores
//!   entre los que alternaba el botón "●" de la barra superior.
//! - `STROKE_WIDTH_PT`: grosor del trazo nuevo en puntos PDF (la pantalla
//!   dibuja `width × scale`, ver `draw.rs`).
//!
//! # ¿Por qué coordenadas de página?
//!
//! La arquitectura (AGENTS.md §4.3) exige anotaciones **vectoriales en
//! coordenadas de página**, dibujadas como capa sobre el bitmap cacheado y
//! nunca rasterizadas en él. Guardar los puntos en puntos PDF los mantiene
//! pegados a la página en cualquier zoom/scroll (la transformación
//! página↔pantalla solo depende de la escala `cover × zoom` y de la posición
//! de la página en la columna).
//!
//! # dead_code intencional (2026-08-XX)
//!
//! El modo dibujo (✏️ / ● / ↶ de la barra superior) se ELIMINÓ de la UI por
//! decisión del autor (visor minimalista: pantalla completa, sin gesto de
//! dibujo). El módulo se conserva ÍNTEGRO con `#![allow(dead_code)]` porque:
//!
//! 1. La carga y el render de anotaciones YA GUARDADAS siguen vivos en
//!    `reader`/`draw` (el usuario no pierde sus trazos, solo la creación
//!    desde la UI por ahora);
//! 2. Reintroducir el dibujo en una fase futura solo requiere volver a
//!    conectar este estado con el input — el modelo (pdf_core) y la
//!    transformación página↔pantalla no se tocan.
#![allow(dead_code)]

use pdf_core::Color;

/// Grosor del trazo nuevo en puntos PDF (PDF points, 1/72"). En pantalla se
/// dibuja a `width × scale` px (ver `draw::draw_stroke`), así que a zoom 1
/// (~2 px/punto en la tablet) un trazo de 2 pt ≈ 4 px — un rotulador fino.
/// El grosor vive en unidades de página (no de pantalla) porque la anotación
/// es vectorial: un trazo de 2 pt ocupa el mismo área del papel en cualquier
/// zoom, como la tinta real.
pub(crate) const STROKE_WIDTH_PT: f32 = 2.0;

/// Color por defecto de los trazos nuevos: rojo (tinta sobre márgenes).
pub(crate) const DEFAULT_STROKE_COLOR: Color = Color {
    r: 220,
    g: 40,
    b: 40,
    a: 255,
};

/// Paleta del botón "●" (alternar color): rojo → azul → verde → rojo...
/// Colores opacos a propósito (se dibujan tal cual sobre la página, en
/// modo oscuro la página se invierte pero la tinta conserva su color).
pub(crate) const STROKE_PALETTE: [Color; 3] = [
    DEFAULT_STROKE_COLOR,
    Color {
        r: 40,
        g: 80,
        b: 220,
        a: 255,
    },
    Color {
        r: 30,
        g: 160,
        b: 70,
        a: 255,
    },
];

/// Trazo en curso (dedo bajado en modo dibujo): polilínea de puntos en
/// coordenadas de página + grosor y color. Al levantar el dedo,
/// `Reader::finish_stroke` lo convierte en [`pdf_core::Stroke`] y lo añade al
/// `AnnotationSet` (que lo persiste en el sidecar).
#[derive(Clone, Debug)]
pub(crate) struct ActiveStroke {
    /// Página (0-based) sobre la que se dibuja. Fija en el `Down`; el trazo
    /// nunca cambia de página aunque el dedo se desplace por encima de otras
    /// (los puntos fuera de la página se recortan en el render).
    pub(crate) page: u32,
    /// Vértices en coordenadas de página (puntos PDF).
    pub(crate) points: Vec<(f32, f32)>,
    /// Grosor en puntos PDF (constante por ahora).
    pub(crate) width: f32,
    pub(crate) color: Color,
}

impl ActiveStroke {
    /// Empieza un trazo en `page` con el primer punto (el del `Down`).
    pub(crate) fn new(page: u32, pt: (f32, f32), color: Color) -> Self {
        Self {
            page,
            points: vec![pt],
            width: STROKE_WIDTH_PT,
            color,
        }
    }

    /// Añade un punto, descartando los casi coincidentes con el anterior
    /// (distancia < 0.25 pt ≈ < 1 px a zoom 4): los Move del táctil llegan a
    /// 60-120 Hz y un punto por evento hincharía la polilínea sin aportar
    /// fidelidad (el trazo sigue siendo suave y ligero de serializar).
    pub(crate) fn push(&mut self, pt: (f32, f32)) {
        if let Some(&last) = self.points.last() {
            let d2 = (pt.0 - last.0).powi(2) + (pt.1 - last.1).powi(2);
            if d2 < 0.0625 {
                return;
            }
        }
        self.points.push(pt);
    }
}
