// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Página de texto cacheadas: `PageTextCache` — Fase B (subrayado sin
//! latencia).
//!
//! El flujo auditado del resaltador (`pdf_android/input.rs`:
//! `end_tool_gesture` Highlight) llama a `Document::text(page)` en el hilo
//! UI justo al soltar el gesto; `text()` ejecuta `load_page` +
//! `to_text_page` (stext) + `structured()` — parsing no trivial en la TCL
//! (A55). La caché mueve esa extracción FUERA del gesto: la primera vez por
//! página se paga (y se puede prefetchear al abrir / en hilo fondo), el
//! resto es un hit de LRU de coste ~0.
//!
//! También es la base de la Fase D (IA con contexto): `ai.rs` necesita el
//! texto de muchas páginas (BM25 / RAG); con esta caché la extracción se
//! amortiza entre el subrayado, la selección y la IA.
//!
//! Diseño:
//! - `LruCache<u32, Arc<PageText>>` (crate `lru`, ya en el workspace):
//!   límite por Nº de páginas (no bytes — el texto es pequeño: ~2 KB/página,
//!   así 512 páginas ≈ 1 MB, irrelevante frente al presupuesto de 150 MB).
//! - `get_or_extract`: hit → clon de `Arc` (barato); miss → `doc.text(page)`
//!   una vez y `put`.
//! - `prefetch`: extrae en lote una lista de páginas (al abrir el PDF: la
//!   visible ±2); devuelve cuántas extrajo de verdad (misses).
//! - Seguro para hilo fondo SI el worker tiene su propio `&dyn Document`
//!   (MuPDF no es Send; el patrón actor de `prefetch.rs` ya abre el
//!   documento dentro del worker).

use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;

use crate::engine::{Document, PageText, Result};

/// Páginas que caben por defecto: 512 ≈ 1 MB de texto, cubre
/// `large_document.pdf` (500 pág.) entero y deja margen.
const DEFAULT_CAPACITY: usize = 512;

/// Caché LRU de `PageText` por índice de página (Fase B1).
pub struct PageTextCache {
    lru: LruCache<u32, Arc<PageText>>,
}

impl Default for PageTextCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl PageTextCache {
    /// Caché con `capacity` páginas (0 desactiva la caché: `get_or_extract`
    /// siempre extrae y devuelve; útil para pruebas o memoria mínima).
    pub fn new(capacity: usize) -> Self {
        Self {
            // lru 0.18 exige NonZeroUsize; 0 → mínimo (1) = prácticamente
            // desactivada.
            lru: LruCache::new(NonZeroUsize::new(capacity.max(1)).expect(">= 1")),
        }
    }

    /// Nº de páginas residentes.
    pub fn len(&self) -> usize {
        self.lru.len()
    }

    /// ¿Vacía?
    pub fn is_empty(&self) -> bool {
        self.lru.is_empty()
    }

    /// Devuelve el texto cacheado de `page` (sin tocar el motor). `None` si
    /// no está residente o la caché está vacía. No actualiza el MRU (peek).
    pub fn get(&self, page: u32) -> Option<Arc<PageText>> {
        self.lru.peek(&page).cloned()
    }

    /// Hit → `Arc<PageText>` cacheado; miss → extrae `doc.text(page)` UNA
    /// vez, lo inserta y lo devuelve. El error del motor se propaga (el
    /// caller decide si avisar, como hoy con `doc.text`).
    ///
    /// Nota `lru` 0.18: `get_or_insert` no existe con cierre que devuelve
    /// `Result`, así que el patrón es `get` → miss → `text` → `put`
    /// (atómicamente dentro de `&mut self`, sin carrera en hilo único; si se
    /// comparte entre hilos, `Mutex<PageTextCache>` o worker-own).
    pub fn get_or_extract(&mut self, doc: &dyn Document, page: u32) -> Result<Arc<PageText>> {
        if let Some(arc) = self.lru.get(&page) {
            return Ok(arc.clone());
        }
        let pt = doc.text(page)?;
        let arc = Arc::new(pt);
        self.lru.put(page, arc.clone());
        Ok(arc)
    }

    /// Extrae y cachea varias páginas (prefetch). Devuelve cuántas se
    /// extrajeron de verdad (misses) — los hits no cuentan. El error de una
    /// página (p. ej. out of range) se ignora y no aborta el lote.
    ///
    /// Uso típico: al abrir un PDF, `prefetch(&doc, visible ± 2)` en hilo
    /// fondo (worker con documento propio, patrón de `prefetch.rs`) para
    /// que el primer highlight de cada página sea un hit.
    pub fn prefetch(&mut self, doc: &dyn Document, pages: &[u32]) -> usize {
        let mut n = 0;
        for &p in pages {
            if self.lru.contains(&p) {
                continue;
            }
            if let Ok(pt) = doc.text(p) {
                self.lru.put(p, Arc::new(pt));
                n += 1;
            }
        }
        n
    }

    /// Vacía la caché (llamar al cambiar de documento).
    pub fn clear(&mut self) {
        self.lru.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{PageText, TextSpan};

    /// Fake Document sintético (sin MuPDF): devuelve spans deterministas y
    /// cuenta las extracciones para verificar la amortización.
    #[derive(Default)]
    struct FakeDoc {
        calls: std::cell::Cell<u32>,
    }

    impl Document for FakeDoc {
        fn page_count(&self) -> u32 {
            100
        }
        fn page_size(&self, _page: u32) -> crate::Result<(f32, f32)> {
            Ok((595.0, 842.0))
        }
        fn render_page(&self, _page: u32, _scale: f32) -> crate::Result<crate::Bitmap> {
            unimplemented!("no render in textcache tests")
        }
        fn text(&self, page: u32) -> crate::Result<PageText> {
            self.calls.set(self.calls.get() + 1);
            Ok(PageText {
                text: format!("contenido de página {page}"),
                spans: vec![TextSpan {
                    text: format!("línea de página {page}"),
                    x: 30.0,
                    y: 20.0,
                    w: 400.0,
                    h: 12.0,
                }],
            })
        }
    }

    #[test]
    fn first_call_extracts_once_then_hits() {
        let mut cache = PageTextCache::new(4);
        let doc = FakeDoc::default();
        let a = cache.get_or_extract(&doc, 0).expect("extract");
        assert_eq!(a.spans[0].text, "línea de página 0");
        assert_eq!(doc.calls.get(), 1);

        // Segundo acceso: hit, sin llamar al motor.
        let b = cache.get_or_extract(&doc, 0).expect("hit");
        assert_eq!(doc.calls.get(), 1, "hit must not call the engine");
        assert!(Arc::ptr_eq(&a, &b), "hit returns the same Arc (no copy)");
    }

    #[test]
    fn prefetch_extracts_missing_pages_only() {
        let mut cache = PageTextCache::new(8);
        let doc = FakeDoc::default();
        // Pre-cargar la página 0.
        cache.get_or_extract(&doc, 0).unwrap();
        let n = cache.prefetch(&doc, &[0, 1, 2, 3]);
        assert_eq!(n, 3, "page 0 already resident, 3 new");
        assert_eq!(doc.calls.get(), 4);
        assert_eq!(cache.len(), 4);

        // Nueva pasada: todo hits.
        let n2 = cache.prefetch(&doc, &[0, 1, 2, 3]);
        assert_eq!(n2, 0, "all resident");
        assert_eq!(doc.calls.get(), 4);
    }

    #[test]
    fn prefetch_ignores_engine_errors_and_continues() {
        // doc.text falla para pares (página "corrupta"): prefetch no aborta.
        struct Partial;
        impl Document for Partial {
            fn page_count(&self) -> u32 {
                10
            }
            fn page_size(&self, _p: u32) -> crate::Result<(f32, f32)> {
                Ok((100.0, 100.0))
            }
            fn render_page(&self, _p: u32, _s: f32) -> crate::Result<crate::Bitmap> {
                unimplemented!()
            }
            fn text(&self, page: u32) -> crate::Result<PageText> {
                if page % 2 == 0 {
                    Err(crate::Error::Engine("fake corrupt".into()))
                } else {
                    Ok(PageText {
                        text: "x".into(),
                        spans: vec![],
                    })
                }
            }
        }
        let mut cache = PageTextCache::new(8);
        let n = cache.prefetch(&Partial, &[0, 1, 2, 3]);
        assert_eq!(n, 2, "pages 1 and 3 extracted, 0/2 errored");
        assert!(cache.get(1).is_some());
        assert!(cache.get(0).is_none());
    }

    #[test]
    fn lru_evicts_oldest_page() {
        let mut cache = PageTextCache::new(2);
        let doc = FakeDoc::default();
        cache.get_or_extract(&doc, 0).unwrap();
        cache.get_or_extract(&doc, 1).unwrap();
        cache.get_or_extract(&doc, 2).unwrap(); // evicta 0
        assert!(cache.get(0).is_none(), "0 evicted (LRU)");
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    #[test]
    fn clear_drops_all() {
        let mut cache = PageTextCache::new(4);
        let doc = FakeDoc::default();
        cache.get_or_extract(&doc, 0).unwrap();
        cache.get_or_extract(&doc, 1).unwrap();
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(cache.get(0).is_none());
    }
}
