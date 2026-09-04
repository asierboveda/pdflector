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
//! - Ancho de portada: `THUMB_W` = 240 px (alto proporcional a la página:
//!   A4 → 240×339 px ≈ 326 KiB RGBA8). A 200 px el vecino-más-cercano del
//!   pegado (celda ≈ 365 px) se veía blocky; 240 px + bilinear suavizan la
//!   portada sin duplicar el presupuesto.
//! - Tope de entradas: `THUMB_MAX_ENTRIES` = 36 (≈ 3 filas de celdas visibles
//!   en la tablet, suficiente para volver arriba/abajo sin re-render).
//! - Tope de bytes: `THUMB_BYTE_BUDGET` = 12 MiB (36 × 326 KiB ≈ 11,7 MiB;
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
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{JoinHandle, spawn};

use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Bitmap, Document, RenderEngine};

/// Ancho (px) al que se renderiza la portada (alto proporcional a la página).
/// 200 px es un equilibrio entre nitidez de la portada y presupuesto: a la
/// escala del blit de la rejilla (celda ≈ 365 px en la tablet) se escala con
/// vecino-más-cercano (~1,8×, ligeramente escalonado — aceptado como portada
/// funcional; el agente de estilos puede subir `THUMB_W` o añadir bilinear).
pub(crate) const THUMB_W: u32 = 240;

/// Tope de entradas de la caché (LRU).
pub(crate) const THUMB_MAX_ENTRIES: usize = 36;

/// Presupuesto máximo de bytes de las portadas residentes (≈ 36 × 226 KiB).
pub(crate) const THUMB_BYTE_BUDGET: usize = 12 * 1024 * 1024;

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

    /// Elimina la entrada `key` si existe (true si estaba). La usa la
    /// evicción LRU de la biblioteca curada (`Reader::add_selected`): la
    /// portada del libro borrado no debe quedar residente. NO altera la
    /// política de evicción existente (solo retira una entrada puntual).
    #[allow(dead_code)]
    pub(crate) fn remove(&mut self, key: &str) -> bool {
        if let Some(bmp) = self.map.remove(key) {
            self.bytes -= bitmap_bytes(&bmp);
            if let Some(pos) = self.lru.iter().position(|k| k == key) {
                self.lru.remove(pos);
            }
            true
        } else {
            false
        }
    }

    /// Descarta todo (cambio de documento o al volver al visor: las portadas
    /// de otra biblioteca ya no se reutilizarían).
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.lru.clear();
        self.bytes = 0;
    }

    /// Nº de portadas residentes (para el log de debug).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Bytes totales residentes (para el log de debug).
    #[allow(dead_code)]
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

/// Mensaje devuelto por el worker de portadas al hilo UI.
pub(crate) struct ThumbMsg {
    pub(crate) key: String,
    pub(crate) bitmap: Option<Bitmap>,
}

/// Comandos enviados al worker actor de portadas.
pub(crate) enum ThumbCmd {
    Request(Vec<String>),
    Stop,
}

/// Worker actor para la generación de portadas en segundo plano (Fase E1).
/// Retiene su propia instancia de `MupdfEngine` en un hilo dedicado.
pub(crate) struct ThumbWorker {
    tx: Sender<ThumbCmd>,
    handle: Option<JoinHandle<()>>,
}

impl ThumbWorker {
    pub(crate) fn spawn() -> (Self, Receiver<ThumbMsg>) {
        let (cmd_tx, cmd_rx) = channel::<ThumbCmd>();
        let (reply_tx, reply_rx) = channel::<ThumbMsg>();

        let handle = spawn(move || {
            let engine = match MupdfEngine::new() {
                Ok(e) => e,
                Err(e) => {
                    log::error!("ThumbWorker: MupdfEngine::new failed: {e}");
                    return;
                }
            };

            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    ThumbCmd::Stop => break,
                    ThumbCmd::Request(mut queue) => {
                        queue.reverse();
                        while let Some(path) = queue.pop() {
                            // Preemption: si ha llegado una petición más reciente, priorizarla
                            if let Ok(newer) = cmd_rx.try_recv() {
                                match newer {
                                    ThumbCmd::Stop => return,
                                    ThumbCmd::Request(mut newer_queue) => {
                                        newer_queue.reverse();
                                        queue = newer_queue;
                                        continue;
                                    }
                                }
                            }

                            let bmp = render_thumb_path(&engine, &path);
                            if reply_tx
                                .send(ThumbMsg {
                                    key: path,
                                    bitmap: bmp,
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            }
        });

        (
            Self {
                tx: cmd_tx,
                handle: Some(handle),
            },
            reply_rx,
        )
    }

    pub(crate) fn request(&self, paths: Vec<String>) {
        let _ = self.tx.send(ThumbCmd::Request(paths));
    }
}

impl Drop for ThumbWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(ThumbCmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn render_thumb_path(engine: &MupdfEngine, path: &str) -> Option<Bitmap> {
    let doc = engine.open(Path::new(path)).ok()?;
    let (pw, _ph) = doc.page_size(0).ok()?;
    if !pw.is_finite() || pw <= 0.0 {
        return None;
    }
    doc.render_page(0, THUMB_W as f32 / pw).ok()
}
