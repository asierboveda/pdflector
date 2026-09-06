// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Clave de invalidación de la capa Dry (Fase 2 del plan de reestructuración).
//! PURA: sin FFI, sin GL — compila y se testea en host aunque el resto del
//! crate requiera Android (ver plan 2026-09-06, Tarea 2.0).

/// Clave de invalidación de la capa base persistente (Dry FBO).
/// Reducida desde 9 campos (incluía pan/chrome/sheet/toast) a 4: solo lo que
/// cambia el CONTENIDO de página+anotaciones. Pan, chrome, sheet y toast se
/// componen como overlays en fb0 y ya no invalidan la dry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DryKey {
    pub(crate) page: u32,
    pub(crate) zoom_bits: u32,
    pub(crate) ann_count: usize,
    pub(crate) dark: bool,
}

impl DryKey {
    /// La dry cacheada bajo `self` sirve para `other` si las claves coinciden.
    pub(crate) fn invalidates(&self, other: &Self) -> bool {
        self != other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DryKey {
        DryKey {
            page: 3,
            zoom_bits: 0x3F800000,
            ann_count: 12,
            dark: false,
        }
    }

    #[test]
    fn content_fields_invalidate() {
        let k = base();
        for other in [
            DryKey { page: 4, ..k },
            DryKey {
                zoom_bits: 0x40000000,
                ..k
            },
            DryKey { ann_count: 13, ..k },
            DryKey { dark: true, ..k },
        ] {
            assert!(k.invalidates(&other), "debe invalidar: {other:?}");
        }
    }

    #[test]
    fn identical_keys_do_not_invalidate() {
        assert!(!base().invalidates(&base()));
    }
}
