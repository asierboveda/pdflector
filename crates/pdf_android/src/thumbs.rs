// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Caché de portadas (thumbnails) de la biblioteca MediaStore en rejilla 3×3.
//!
//! La rejilla muestra la página 1 de cada PDF como portada. Renderizar las
//! 256 portadas de golpe al abrir la biblioteca rompería el presupuesto de
//! RAM y de frame time (AGENTS.md §2/§8), así que las portadas se renderizan
//! **perezosamente** (solo las celdas visibles) y **bajo demanda**, en lotes
//! pequeños dentro de `Reader::tick` (ver `pump_thumbs`), con un placeholder
//! mientras llegan. Este módulo solo aporta la caché LRU acotada; la política
//! de cuándo/cuántas renderizar vive en `reader`.
//!
//! ## Presupuesto de memoria (documentado)
//!
//! - Ancho de portada: `THUMB_W` = 200 px (alto proporcional a la página:
//!   A4 → 200×283 px ≈ 226 KiB RGBA8).
//! - Tope de entradas: `THUMB_MAX_ENTRIES` = 36 (≈ 3 filas de celdas visibles
//!   en la tablet, suficiente para volver arriba/abajo sin re-render).
//! - Tope de bytes: `THUMB_BYTE_BUDGET` = 9 MiB (36 × 226 KiB ≈ 8,1 MiB;
//!   margen para páginas de proporción más alta). Frente al objetivo RSS
//!   < 150 MB de la tablet, las portadas quedan en ~6 % del presupuesto y
//!   NO compiten con la `PageCache` del visor (48 MiB): son estados mutuamente
//!   exclusivos (biblioteca vs visor) y la caché de portadas se limpia al
//!   abrir un PDF (`Reader::open_pdf`).
//! - Clave: el content:// URI de la entrada (único por PDF del sistema).
//!
//! ## Evicción
//!
//! Mismo patrón que `cache::PageCache`: `HashMap` + `VecDeque` (cola de
//! recencia, frente = least-recently-used). `get` promueve la recencia
//! (render y blit la usan en cada frame de la biblioteca); `insert` expulsa
//! del frente hasta cumplir `max_entries` y `byte_budget`. Con ≤ 36 entradas
//! la promoción es O(n) con n ≤ 36 y no merece la pena una crate LRU.

use std::collections::{HashMap, VecDeque};

use pdf_core::Bitmap;

/// Ancho (px) al que se renderiza la portada (alto proporcional a la página).
/// 200 px es un equilibrio entre nitidez de la portada y presupuesto: a la
/// escala del blit de la rejilla (celda ≈ 365 px en la tablet) se escala con
/// vecino-más-cercano (~1,8×, ligeramente escalonado — aceptado como portada
/// funcional; el agente de estilos puede subir `THUMB_W` o añadir bilinear).
pub(crate) const THUMB_W: u32 = 200;

/// Tope de entradas de la caché (LRU).
pub(crate) const THUMB_MAX_ENTRIES: usize = 36;

/// Presupuesto máximo de bytes de las portadas residentes (≈ 36 × 226 KiB).
pub(crate) const THUMB_BYTE_BUDGET: usize = 9 * 1024 * 1024;

/// Bytes que ocupa un `Bitmap` RGBA8 (cifra real, nunca estimada).
fn bitmap_bytes(bmp: &Bitmap) -> usize {
    bmp.width as usize * bmp.height as usize * 4
}

/// Caché LRU de portadas (content:// URI → `Bitmap`), limitada por bytes y
/// por nº de entradas. Guarda SIEMPRE bitmaps normales (la portada no se
/// invierte en modo oscuro: es una vista previa del documento, no la página).
pub(crate) struct ThumbCache {
    map: HashMap<String, Bitmap>,
    lru: VecDeque<String>,
    bytes: usize,
    byte_budget: usize,
    max_entries: usize,
}

impl ThumbCache {
    pub(crate) fn new(byte_budget: usize, max_entries: usize) -> Self {
        Self {
            map: HashMap::new(),
            lru: VecDeque::new(),
            bytes: 0,
            byte_budget,
            max_entries,
        }
    }

    /// Lookup que PROMUEVE la entrada (recencia LRU). Lo usa `pump_thumbs`:
    /// un hit evita re-renderizar la portada y marca la recencia.
    pub(crate) fn get(&mut self, key: &str) -> Option<&Bitmap> {
        if self.map.contains_key(key) {
            self.promote(key);
        }
        self.map.get(key)
    }

    /// Lookup SIN promoción: el render de la rejilla lee las portadas de
    /// cada frame sin reordenar la recencia (el orden lo fija `pump_thumbs`).
    pub(crate) fn peek(&self, key: &str) -> Option<&Bitmap> {
        self.map.get(key)
    }

    /// Inserta (o reemplaza) la portada de `key`, expulsando LRU hasta caber
    /// en `byte_budget` y `max_entries`.
    pub(crate) fn insert(&mut self, key: String, bitmap: Bitmap) {
        let incoming = bitmap_bytes(&bitmap);
        if let Some(old) = self.map.remove(&key) {
            self.bytes -= bitmap_bytes(&old);
            if let Some(pos) = self.lru.iter().position(|k| *k == key) {
                self.lru.remove(pos);
            }
        }
        while self.map.len() >= self.max_entries && !self.lru.is_empty() {
            self.evict_lru();
        }
        while self.bytes + incoming > self.byte_budget && !self.lru.is_empty() {
            self.evict_lru();
        }
        self.bytes += incoming;
        self.map.insert(key.clone(), bitmap);
        self.lru.push_back(key);
    }

    /// Descarta todo (cambio de documento o al volver al visor: las portadas
    /// de otra biblioteca ya no se reutilizarían).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.lru.clear();
        self.bytes = 0;
    }

    /// Nº de portadas residentes (para el log de debug).
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Bytes totales residentes (para el log de debug).
    pub(crate) fn resident_bytes(&self) -> usize {
        self.bytes
    }

    fn promote(&mut self, key: &str) {
        if let Some(pos) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(pos);
            self.lru.push_back(key.to_string());
        }
    }

    fn evict_lru(&mut self) {
        if let Some(victim) = self.lru.pop_front()
            && let Some(bmp) = self.map.remove(&victim)
        {
            self.bytes -= bitmap_bytes(&bmp);
        }
    }
}
