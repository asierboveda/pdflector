// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Texturas por página del modo Scroll (PR-01): política PURA (sin FFI ni
//! GL, testeable en host) de las ranuras de textura que retienen las hojas
//! visibles de la columna. A diferencia de `page_tex` (UNA hoja, modo Tap),
//! la columna dibuja 2-3 hojas por frame: re-subir sus bitmaps en cada frame
//! de scroll costaría decenas de ms, así que cada hoja conserva su textura
//! mientras su bitmap residente no cambie. El llamador (`Gpu`) sube y borra
//! los nombres GL que esta política le indica.

/// Máximo de texturas de página residentes (las visibles de la columna;
/// a zoom 1 en la tablet caben 2-3 hojas en pantalla).
pub(crate) const PAGE_SLOTS_MAX: usize = 4;

/// Identidad del bitmap residente de una página: (ancho, alto, dirección de
/// los datos). Junto al índice de página basta: el mismo índice con las
/// mismas dimensiones es el mismo contenido (re-render a la misma escala),
/// así que un reuso de dirección (ABA) no puede mostrar otra hoja.
pub(crate) type BitmapKey = (u32, u32, usize);

/// Ranuras LRU (frente = víctima): (página, clave del bitmap, textura GL).
#[derive(Default)]
pub(crate) struct PageSlots {
    slots: Vec<(u32, BitmapKey, u32)>,
}

impl PageSlots {
    /// Textura ya subida para (`page`, `key`); un acierto pasa a MRU.
    pub(crate) fn lookup(&mut self, page: u32, key: BitmapKey) -> Option<u32> {
        let pos = self
            .slots
            .iter()
            .position(|&(p, k, _)| p == page && k == key)?;
        let slot = self.slots.remove(pos);
        self.slots.push(slot);
        Some(slot.2)
    }

    /// Registra la textura `tex` recién subida para (`page`, `key`).
    /// Devuelve las texturas a borrar: la anterior de la misma página (otro
    /// bitmap) y las víctimas LRU por encima de `PAGE_SLOTS_MAX`.
    pub(crate) fn insert(&mut self, page: u32, key: BitmapKey, tex: u32) -> Vec<u32> {
        let mut freed = Vec::new();
        if let Some(pos) = self.slots.iter().position(|&(p, _, _)| p == page) {
            freed.push(self.slots.remove(pos).2);
        }
        self.slots.push((page, key, tex));
        while self.slots.len() > PAGE_SLOTS_MAX {
            freed.push(self.slots.remove(0).2);
        }
        freed
    }

    /// Vacía las ranuras (cambio de documento, salida del modo Scroll):
    /// devuelve todas las texturas a borrar.
    pub(crate) fn drain(&mut self) -> Vec<u32> {
        self.slots.drain(..).map(|(_, _, tex)| tex).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_returns_the_uploaded_texture_only_for_the_same_bitmap() {
        let mut s = PageSlots::default();
        assert_eq!(s.lookup(3, (10, 20, 0xA)), None);
        assert!(s.insert(3, (10, 20, 0xA), 7).is_empty());
        assert_eq!(s.lookup(3, (10, 20, 0xA)), Some(7));
        assert_eq!(s.lookup(3, (11, 20, 0xA)), None); // otro render
        assert_eq!(s.lookup(4, (10, 20, 0xA)), None); // otra hoja
    }

    #[test]
    fn a_new_bitmap_of_the_same_page_replaces_its_texture() {
        let mut s = PageSlots::default();
        s.insert(3, (10, 20, 0xA), 7);
        assert_eq!(s.insert(3, (20, 40, 0xB), 8), vec![7]);
        assert_eq!(s.lookup(3, (20, 40, 0xB)), Some(8));
        assert_eq!(s.lookup(3, (10, 20, 0xA)), None);
    }

    #[test]
    fn least_recently_used_page_is_evicted_beyond_the_cap() {
        let mut s = PageSlots::default();
        for p in 0..PAGE_SLOTS_MAX as u32 {
            assert!(s.insert(p, (1, 1, p as usize), 100 + p).is_empty());
        }
        // La 0 se usa: la víctima pasa a ser la 1.
        assert_eq!(s.lookup(0, (1, 1, 0)), Some(100));
        assert_eq!(s.insert(9, (1, 1, 9), 109), vec![101]);
        assert_eq!(s.lookup(1, (1, 1, 1)), None);
        assert_eq!(s.lookup(0, (1, 1, 0)), Some(100));
    }

    #[test]
    fn drain_returns_every_texture_once() {
        let mut s = PageSlots::default();
        s.insert(0, (1, 1, 1), 5);
        s.insert(1, (1, 1, 2), 6);
        let mut all = s.drain();
        all.sort();
        assert_eq!(all, vec![5, 6]);
        assert!(s.drain().is_empty());
        assert_eq!(s.lookup(0, (1, 1, 1)), None);
    }
}
