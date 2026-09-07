// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Caché LRU de páginas renderizadas (`struct PageCache`, página →
//! `CachedPage`) para el visor UNA HOJA: guarda los crops de página que
//! envía el worker de render y los reutiliza al volver atrás o al hacer
//! prev/next, evitando el re-render de MuPDF (~18-25 ms/página en la tablet)
//! en el camino caliente del blit.
//!
//! ## Valor cacheado: crop centrado a ventana (fix de residency)
//!
//! El bitmap residente NO es el render full a cover (`cover × rendered_zoom`
//! puede pesar 27,4 MiB en landscape — 2200×3112 — y cabía UNA página en el
//! presupuesto), sino el recorte a la ventana de ese render: X CENTRADO +
//! Y ALINEADO ARRIBA (`pdf_core::crop_rect` con `(x=(full_w−w)/2, y=0)`,
//! dims `(min(bw, win_w), min(bh, win_h))`) — ≤ ~12,7 MiB por página → ~3
//! residentes en ambas orientaciones y turnos de página sin re-render.
//! `CachedPage` guarda además los metadatos del render FULL
//! (`full_w/full_h`) y el origen del recorte dentro de él
//! (`crop_x/crop_y`): el quad de la página dibuja el crop tal cual (a
//! `blit_zoom == 1` el crop X-centrado compensa el centrado X del blit y la
//! alineación Y-top coincide con la del blit → la página llena la ventana
//! 1:1 con los mismos píxeles que el render full), pero los consumidores
//! que convierten pantalla ↔ píxeles del render (transición fast→sharp del
//! pinch, `sel_image_png_base64`) y la capa de anotaciones/tinta operan
//! sobre la cuadrícula del render FULL y compensan el origen — ver
//! `gpu::pipeline::render_dry`/`render_wet`, `pinch.rs` y `seleccion.rs`.
//!
//! ## Política (límites y evicción — documentada)
//!
//! - **Clave**: índice de página (`u32`). Todos los crops residentes están
//!   renderizados a la MISMA escala (`cover × rendered_zoom`, ver `reader`):
//!   al cambiar el zoom, la ventana o el documento se invalida con `clear()`
//!   (una escala distinta es un render distinto; conservar niveles antiguos
//!   solo consumiría presupuesto — misma regla que `pdf_core::cache`).
//! - **Presupuesto de bytes**: 48 MiB (`CACHE_BYTE_BUDGET`). En la tablet
//!   (1440×2200 px) el crop a ventana de una página ocupa
//!   1440 × 2200 × 4 B ≈ 12,1 MiB (12,7 MiB en landscape 2200×1440); el
//!   presupuesto admite ~3 residentes (~36-38 MiB), coherente con el objetivo
//!   RSS < 150 MB del proyecto: la caché queda en ~¼ del presupuesto total
//!   (el resto es el .so, MuPDF y los buffers de ventana).
//! - **Tope de entradas**: 5 (`CACHE_MAX_ENTRIES`). Suficiente para "volver
//!   atrás instantáneo" (2-3 páginas hacia atrás) y para la ventana de
//!   prefetch del paso de página (fase B: 2 por delante en la dirección de
//!   viaje + 1 por detrás + la actual, direccional — ver `navigation.rs`),
//!   sin dejar crecer la cola LRU.
//! - **Evicción**: least-recently-used. `get` promueve la entrada (recencia
//!   real, evita re-render en el render de cada frame); `insert` expulsa del
//!   frente de la cola LRU hasta cumplir `byte_budget` y `max_entries`. Con
//!   crops a ventana (~12,1 MiB) el lote completo de la fase B (2+1+actual =
//!   4 páginas ≈ 48,4 MiB) queda justo AL LÍMITE de los 48 MiB: la inserción
//!   del 4º crop puede expulsar al más antiguo del lote — la página de atrás,
//!   que `prefetch_pages` lanza la primera a propósito. Comportamiento
//!   correcto por diseño: el log de evict (`pagecache evict`) es la señal
//!   permanente de residency — en lectura secuencial (≤ 3 páginas en vuelo)
//!   debe CALLAR; si vuelve a aparecer en cada turno, la residency se rompió.
//!   Si una única página supera todo el presupuesto (zoom alto: una página
//!   a 8× puede pesar cientos de MiB) se expulsa TODO y entra sola —
//!   best-effort, idéntico a `pdf_core::cache`, y el footprint coincide con
//!   el del `bitmap` único que ya alojaba la app antes de la caché.
//! - **Modo oscuro**: la caché guarda SIEMPRE bitmaps normales (de colores).
//!   La inversión (255 − v) se aplica al blitear, por página, solo si el modo
//!   oscuro está activo (ver `draw::blit_page`); nunca se almacena una
//!   variante invertida.
//!
//! `HashMap` + `VecDeque` (cola de recencia, frente = LRU): con ≤ 5 entradas
//! la promoción es O(n) con n ≤ 5 y no hace falta una crate LRU dedicada.

use std::collections::{HashMap, VecDeque};

use pdf_core::Bitmap;

/// Presupuesto máximo de bytes residentes de la caché: ≈ 3-4 páginas a ventana
/// (crop cover) en la tablet (ver cabecera del módulo).
pub(crate) const CACHE_BYTE_BUDGET: usize = 48 * 1024 * 1024;

/// Tope de entradas (páginas) residentes.
pub(crate) const CACHE_MAX_ENTRIES: usize = 5;

/// Página renderizada residente: el bitmap es el recorte a la ventana del
/// render full — X CENTRADO + Y ALINEADO ARRIBA (`crop_rect` con
/// `y = 0`, fix de residency — el render full a cover pesa hasta 27,4 MiB
/// en landscape y dejaba 1 solo residente; el recorte deja ≤ ~12,7 MiB →
/// ~3 residentes). Los metadatos describen el render FULL (la escala
/// cover × rendered_zoom con la que se dibujó) y la posición del recorte
/// dentro de él: los consumidores que convierten pantalla ↔ píxeles del
/// render (transición fast→sharp del pinch, selección → imagen) y la capa
/// de anotaciones/tinta operan sobre la cuadrícula FULL y compensan el
/// origen.
pub(crate) struct CachedPage {
    /// Recorte a ventana del render full (`full_w × full_h`), cuyo píxel
    /// (0,0) corresponde al píxel (`crop_x`, `crop_y`) del render full.
    pub(crate) bitmap: Bitmap,
    /// Ancho del render FULL (pre-recorte), en píxeles del render.
    pub(crate) full_w: u32,
    /// Alto del render FULL (pre-recorte), en píxeles del render.
    ///
    /// `allow(dead_code)`: hoy ningún consumidor lee `full_h` (el mapeo
    /// pantalla→render usa `full_w`/`crop_x`/`crop_y` — en Y la caja full
    /// arranca en el borde superior, `dy = pan_y`, independiente del alto).
    /// Se conserva como parte del contrato del render full (simetría con
    /// `full_w`, que consumen pinch, sel_image y la capa de tinta) por si un
    /// consumidor futuro necesite el alto pre-recorte (p. ej. sel_image si
    /// algún día mide sobre el full en vez del crop).
    #[allow(dead_code)]
    pub(crate) full_h: u32,
    /// Columna inicial del recorte dentro del render full.
    pub(crate) crop_x: u32,
    /// Fila inicial del recorte dentro del render full.
    pub(crate) crop_y: u32,
}

/// Bytes que ocupa el bitmap de una `CachedPage` (cifra real del buffer,
/// nunca estimada).
fn bitmap_bytes(page: &CachedPage) -> usize {
    page.bitmap.width as usize * page.bitmap.height as usize * 4
}

/// Caché LRU de páginas renderizadas (página → `CachedPage`), limitada por
/// bytes y por nº de entradas.
pub(crate) struct PageCache {
    map: HashMap<u32, CachedPage>,
    /// Cola de recencia: el frente es el least-recently-used (primera víctima).
    lru: VecDeque<u32>,
    /// Bytes totales de los bitmaps residentes.
    bytes: usize,
    byte_budget: usize,
    max_entries: usize,
}

impl PageCache {
    pub(crate) fn new(byte_budget: usize, max_entries: usize) -> Self {
        Self {
            map: HashMap::new(),
            lru: VecDeque::new(),
            bytes: 0,
            byte_budget,
            max_entries,
        }
    }

    /// Lookup que PROMUEVE la entrada (recencia LRU). Lo usa el render
    /// (`ensure_pages_rendered`): un hit evita el re-render y marca la página
    /// como recientemente usada.
    #[allow(dead_code)] // el flujo async usa `peek`; get queda como API
    pub(crate) fn get(&mut self, page: u32) -> Option<&CachedPage> {
        if self.map.contains_key(&page) {
            self.promote(page);
        }
        self.map.get(&page)
    }

    /// Lookup SIN promoción: el blit de cada frame lee las páginas visibles
    /// sin reordenar la recencia (el orden lo fija el render/prefetch).
    pub(crate) fn peek(&self, page: u32) -> Option<&CachedPage> {
        self.map.get(&page)
    }

    /// Inserta (o reemplaza) la `CachedPage` de `page`, expulsando LRU hasta
    /// caber en `byte_budget` y `max_entries`. Una página que supera todo el
    /// presupuesto expulsa el resto y entra sola (best-effort, ver cabecera).
    pub(crate) fn insert(&mut self, page: u32, cached: CachedPage) {
        let incoming = bitmap_bytes(&cached);
        // Reemplazo de una página ya residente: liberar sus bytes y su hueco
        // en la cola antes de reinsertarla como la más reciente.
        if let Some(old) = self.map.remove(&page) {
            self.bytes -= bitmap_bytes(&old);
            if let Some(pos) = self.lru.iter().position(|&p| p == page) {
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
        self.map.insert(page, cached);
        self.lru.push_back(page);
    }

    /// Descarta todo (cambio de zoom, de ventana o de documento): los bitmaps
    /// viejos son de otra escala y nunca se reutilizarían.
    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.lru.clear();
        self.bytes = 0;
    }

    /// Nº de páginas residentes (para el log de debug).
    #[allow(dead_code)] // métrica de debug
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }

    /// Bytes totales residentes (para el log de debug).
    #[allow(dead_code)] // métrica de debug
    pub(crate) fn resident_bytes(&self) -> usize {
        self.bytes
    }

    #[allow(dead_code)]
    fn promote(&mut self, page: u32) {
        if let Some(pos) = self.lru.iter().position(|&p| p == page) {
            self.lru.remove(pos);
            self.lru.push_back(page);
        }
    }

    fn evict_lru(&mut self) {
        if let Some(victim) = self.lru.pop_front()
            && let Some(cached) = self.map.remove(&victim)
        {
            self.bytes -= bitmap_bytes(&cached);
            // Señal PERMANENTE de residency (fix del diagnóstico de speed): en
            // lectura secuencial con crops a ventana (~12,1 MiB, ~3 caben en
            // el presupuesto) el lote de prefetch NO debe evictar — si este
            // log aparece en cada turno, la residency volvió a romperse.
            log::info!("pagecache evict page={victim} total={}", self.bytes);
        }
    }
}
