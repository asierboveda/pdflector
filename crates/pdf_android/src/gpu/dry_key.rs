// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Clave de invalidación de la capa Dry (Fase 2 del plan de reestructuración).
//! PURA: sin FFI, sin GL — compila y se testea en host aunque el resto del
//! crate requiera Android (ver plan 2026-09-06, Tarea 2.0).

/// Clave de invalidación de la capa base persistente (Dry FBO).
/// Contiene página, zoom, pan, anotaciones y modo oscuro. Los overlays de UI
/// (chrome, sheet, toast) se componen en fb0 y no invalidan la dry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DryKey {
    pub(crate) page: u32,
    pub(crate) zoom_bits: u32,
    pub(crate) pan_x: i32,
    pub(crate) pan_y: i32,
    pub(crate) ann_count: usize,
    pub(crate) dark: bool,
}

impl DryKey {
    /// La dry cacheada bajo `self` sirve para `other` si las claves coinciden.
    pub(crate) fn invalidates(&self, other: &Self) -> bool {
        self != other
    }
}

/// Firma de las anotaciones de las hojas que hornea la dry (`pages`: la
/// actual en Tap, TODAS las visibles de la columna en Scroll — PR-04): un
/// cambio en cualquiera de ellas invalida la dry, no solo en la actual.
pub(crate) fn ann_key(pages: &[u32], count: impl Fn(u32) -> usize) -> usize {
    // FNV-1a sobre (página, recuento): mover un trazo de una hoja visible
    // a otra también cambia la firma.
    const PRIME: u64 = 0x0100_0000_01b3;
    let mix = |h: u64, v: u64| (h ^ v).wrapping_mul(PRIME);
    pages.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
        mix(mix(h, p as u64), count(p) as u64)
    }) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ann_key_changes_with_any_drawn_page() {
        let counts = |c: [usize; 4]| move |p: u32| c[p as usize];
        let visible = [1, 2, 3];
        let k = ann_key(&visible, counts([0, 2, 5, 1]));
        // Deshacer/borrar en una hoja visible que NO es la primera.
        assert_ne!(k, ann_key(&visible, counts([0, 2, 4, 1])));
        assert_ne!(k, ann_key(&visible, counts([0, 2, 5, 2])));
        // Un trazo que pasa de una hoja visible a otra también cuenta.
        assert_ne!(
            ann_key(&visible, counts([0, 1, 0, 0])),
            ann_key(&visible, counts([0, 0, 1, 0]))
        );
        // Hojas fuera de la vista no invalidan; la firma es determinista.
        assert_eq!(k, ann_key(&visible, counts([9, 2, 5, 1])));
        assert_eq!(k, ann_key(&visible, counts([0, 2, 5, 1])));
        // Modo Tap (una hoja): sigue cambiando con su recuento.
        assert_ne!(
            ann_key(&[2], counts([0, 0, 5, 0])),
            ann_key(&[2], counts([0, 0, 6, 0]))
        );
    }

    fn base() -> DryKey {
        DryKey {
            page: 3,
            zoom_bits: 0x3F800000,
            pan_x: 0,
            pan_y: 0,
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
            DryKey { pan_x: 1, ..k },
            DryKey { pan_y: 1, ..k },
        ] {
            assert!(k.invalidates(&other), "debe invalidar: {other:?}");
        }
    }

    #[test]
    fn identical_keys_do_not_invalidate() {
        assert!(!base().invalidates(&base()));
    }

    #[test]
    fn reset_via_none_key_always_invalidates() {
        let old: Option<DryKey> = None;
        let new_key = base();
        assert!(old.is_none_or(|k| new_key.invalidates(&k)));
    }

    #[test]
    fn same_page_different_documents_would_not_invalidate_without_reset() {
        // Documento A en pág 0, zoom 1.0, 0 anns, light:
        let doc_a_key = DryKey {
            page: 0,
            zoom_bits: 1.0f32.to_bits(),
            pan_x: 0,
            pan_y: 0,
            ann_count: 0,
            dark: false,
        };
        // Documento B nuevo en pág 0, zoom 1.0, 0 anns, light:
        let doc_b_key = DryKey {
            page: 0,
            zoom_bits: 1.0f32.to_bits(),
            pan_x: 0,
            pan_y: 0,
            ann_count: 0,
            dark: false,
        };
        // Sin reset_document (dry_key = None), doc_a_key NO invalida para doc_b_key:
        assert!(!doc_a_key.invalidates(&doc_b_key));
    }
}
