// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Prefetch error surfacing (IN-03): a page that fails to render on the
//! worker must reach the client via `Prefetcher::take_errors` instead of
//! being silently dropped. Uses a mock `RenderEngine` whose document fails
//! on a chosen page — no MuPDF, no corpus.

use std::path::Path;
use std::time::Duration;

use pdf_core::engine::{Bitmap, Document, Error, PageText, RenderEngine, Result};
use pdf_core::prefetch::Prefetcher;
use pdf_core::scroll::Viewport;

/// Page index whose render always fails.
const BAD_PAGE: u32 = 1;

struct FailingEngine;

struct FailingDoc;

impl Document for FailingDoc {
    fn page_count(&self) -> u32 {
        4
    }

    fn page_size(&self, _page: u32) -> Result<(f32, f32)> {
        Ok((10.0, 10.0))
    }

    fn render_page(&self, page: u32, _scale: f32) -> Result<Bitmap> {
        if page == BAD_PAGE {
            return Err(Error::Engine(format!("mock render failure on page {page}")));
        }
        Ok(Bitmap {
            width: 2,
            height: 2,
            data: vec![0; 16],
        })
    }

    fn text(&self, _page: u32) -> Result<PageText> {
        Ok(PageText {
            text: String::new(),
            spans: Vec::new(),
        })
    }
}

impl RenderEngine for FailingEngine {
    type Document = FailingDoc;

    fn open(&self, _path: &Path) -> Result<Self::Document> {
        Ok(FailingDoc)
    }
}

#[test]
fn render_errors_reach_the_client_and_worker_keeps_going() {
    let prefetcher =
        Prefetcher::open(FailingEngine, Path::new("unused.pdf"), 1 << 20).expect("open");
    let vp = Viewport {
        first_visible_page: 0,
        visible_count: 3,
    };
    prefetcher.request(&vp, 4, 1, 0);
    assert!(prefetcher.await_idle_timeout(Duration::from_secs(10)));

    let errors = prefetcher.take_errors();
    assert_eq!(
        errors.len(),
        1,
        "exactly the failing page is reported: {errors:?}"
    );
    assert_eq!(errors[0].key.page_idx, BAD_PAGE as usize);
    assert_eq!(errors[0].key.scale_level, 0);
    assert!(
        matches!(&errors[0].error, Error::Engine(msg) if msg.contains("mock render failure")),
        "original engine error is preserved: {:?}",
        errors[0].error
    );

    // The failing page does not stall the queue: every other page rendered.
    let mut resident: Vec<usize> = prefetcher
        .resident_pages()
        .into_iter()
        .map(|k| k.page_idx)
        .collect();
    resident.sort_unstable();
    assert_eq!(resident, vec![0, 2, 3]);

    // Errors are drained: a second call reports nothing new.
    assert!(prefetcher.take_errors().is_empty());
}
