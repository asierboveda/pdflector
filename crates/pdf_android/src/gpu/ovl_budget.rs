// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Política de presupuesto LRU por bytes de las texturas de overlay (Fase 2
//! del plan de reestructuración, Tarea 2.4). PURA: sin FFI, sin GL — compila
//! y se testea en host aunque el resto del crate requiera Android (ver plan
//! 2026-09-06, Tarea 2.0). El llamador (`Gpu::overlay_tex`) hace
//! `glDeleteTextures` de los ids evictados que `insert` devuelve.

/// Presupuesto máximo de bytes de las texturas de overlay residentes (8 MiB;
/// los bitmaps de UI —chrome, toast, badges, menús, cursor— son pequeños y
/// caben varios; un overlay que no cabe ni vaciando la caché no se cachea).
pub(crate) const OVL_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Presupuesto LRU por bytes de las entradas de overlay: `entries` guarda
/// (id, bytes) en orden LRU (frente = víctima) y `insert` evicta del frente
/// hasta que la entrada nueva cabe. La clave es el `id` de generación que el
/// Reader asigna a cada bitmap de overlay (NUNCA el puntero de `data`: ABA
/// cuando el allocator reusa la dirección de un bitmap viejo).
pub(crate) struct OvlBudget {
    entries: Vec<(u64, usize)>, // (id, bytes) en orden LRU (frente = víctima)
    bytes: usize,
    budget: usize,
}

impl OvlBudget {
    pub(crate) fn new(budget: usize) -> Self {
        Self { entries: Vec::new(), bytes: 0, budget }
    }

    /// Registra una inserción de `bytes` para `id`; evicta del frente hasta caber.
    /// Devuelve los ids evictados (para glDeleteTextures en el llamador).
    pub(crate) fn insert(&mut self, id: u64, bytes: usize) -> Vec<u64> {
        self.touch(id);
        let mut evicted = Vec::new();
        while self.bytes + bytes > self.budget && !self.entries.is_empty() {
            let (vid, vb) = self.entries.remove(0);
            self.bytes -= vb;
            evicted.push(vid);
        }
        if self.bytes + bytes <= self.budget {
            self.entries.push((id, bytes));
            self.bytes += bytes;
        }
        evicted
    }

    pub(crate) fn touch(&mut self, id: u64) {
        if let Some(pos) = self.entries.iter().position(|e| e.0 == id) {
            let e = self.entries.remove(pos);
            self.entries.push(e);
        }
    }

    /// ¿Está `id` residente? (el llamador lo consulta tras `insert` para no
    /// subir texturas de entradas que no cupieron ni vaciando la caché).
    pub(crate) fn contains(&self, id: u64) -> bool {
        self.entries.iter().any(|e| e.0 == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_lru_front_until_budget() {
        let mut b = OvlBudget::new(100);
        assert!(b.insert(1, 40).is_empty());
        assert!(b.insert(2, 40).is_empty());
        assert_eq!(b.insert(3, 40), vec![1]); // 40+40+40 > 100 → evict 1
    }

    #[test]
    fn touch_promotes_recency() {
        let mut b = OvlBudget::new(100);
        b.insert(1, 40);
        b.insert(2, 40);
        b.touch(1); // 1 pasa a MRU
        assert_eq!(b.insert(3, 40), vec![2]); // víctima ahora 2
    }

    #[test]
    fn oversized_entry_is_dropped() {
        let mut b = OvlBudget::new(50);
        assert!(b.insert(9, 80).is_empty()); // no cabe ni vacía: se descarta
        assert!(b.insert(1, 10).is_empty()); // sigue operativa
    }

    #[test]
    fn contains_reflects_residency() {
        let mut b = OvlBudget::new(100);
        assert!(!b.contains(1));
        b.insert(1, 40);
        assert!(b.contains(1));
        assert!(b.insert(2, 80).contains(&1)); // 40+80 > 100 → evict 1
        assert!(!b.contains(1));
        assert!(b.contains(2));
        b.touch(1); // id no residente: no-op
        assert!(!b.contains(1));
    }
}
