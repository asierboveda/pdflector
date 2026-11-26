// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Modo de lectura CONTINUO (PR-01): junto al visor página a página (Tap),
//! el modo Scroll apila las páginas en vertical con un hueco entre ellas;
//! el arrastre con un dedo desplaza el documento y el indicador de página
//! sigue a la hoja que más ocupa la pantalla.
//!
//! El modo es una preferencia POR DOCUMENTO (`persist::BookProgress`).

use serde::{Deserialize, Serialize};

/// Modo de lectura del visor (por documento, persistido en `library.json`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReadingMode {
    /// Página a página: tap izquierda/derecha pasa de hoja (comportamiento
    /// histórico; default de los registros viejos sin el campo).
    #[default]
    Tap,
    /// Continuo: páginas apiladas en vertical, arrastre = desplazamiento.
    Scroll,
}
