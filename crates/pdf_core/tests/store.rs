// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Integration tests for the SQLite sidecar store (Fase 3, PLAN §3.5):
//! exact save/load round-trips, id and `next_id` preservation across reload,
//! empty-file behaviour and the sidecar path convention. No PDF engine
//! involved — the store only touches [`AnnotationSet`].

use std::path::{Path, PathBuf};

use pdf_core::store::{AnnotationStore, sidecar_path};
use pdf_core::{Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, TextNote};

/// Unique per-test temp dir, removed on drop (also on panic).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pdflector-store-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn color() -> Color {
    Color {
        r: 255,
        g: 0,
        b: 0,
        a: 200,
    }
}

fn stroke() -> Stroke {
    Stroke::new(
        vec![(10.0, 20.0), (30.5, 40.25), (50.0, 60.0)],
        2.5,
        color(),
    )
    .expect("valid stroke")
}

fn highlight() -> Highlight {
    Highlight {
        rects: vec![Rect::new(10.0, 20.0, 100.0, 12.0)],
        color: color(),
    }
}

fn text_note() -> TextNote {
    TextNote {
        anchor: (5.0, 5.0),
        text: "revisar §3".to_string(),
    }
}

/// 4 annotations over pages 0..3: stroke on 0, stroke+highlight on 1, text
/// note on 2. Ids 0..3, `next_id` 4.
fn sample_set() -> AnnotationSet {
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Stroke(stroke()))
        .expect("add stroke p0");
    set.add(1, Annotation::Stroke(stroke()))
        .expect("add stroke p1");
    set.add(1, Annotation::Highlight(highlight()))
        .expect("add highlight p1");
    set.add(2, Annotation::TextNote(text_note()))
        .expect("add note p2");
    set
}

#[test]
fn sidecar_path_follows_plan_convention() {
    let cases = [
        (
            Path::new("/lib/doc.pdf"),
            Path::new("/lib/annotations/doc.db"),
        ),
        (
            Path::new("/lib/a.b.pdf"),
            Path::new("/lib/annotations/a.b.db"),
        ),
        (Path::new("/lib/doc"), Path::new("/lib/annotations/doc.db")),
        (
            Path::new("libs/sub/doc.pdf"),
            Path::new("libs/sub/annotations/doc.db"),
        ),
        (Path::new("doc.pdf"), Path::new("annotations/doc.db")),
    ];
    for (pdf, want) in cases {
        assert_eq!(sidecar_path(pdf), want, "for {pdf:?}");
    }
}

#[test]
fn save_then_load_round_trips_exactly() {
    let tmp = TempDir::new("roundtrip");
    let db = tmp.path().join("ann.db");
    let store = AnnotationStore::open(&db).expect("open");
    let set = sample_set();

    store.save(&set).expect("save");
    let loaded = store.load().expect("load");

    assert_eq!(loaded, set);
    assert_eq!(loaded.ids(), set.ids());

    // z-order per page survives (ORDER BY id == insertion order).
    let page1 = loaded.for_page(1);
    assert_eq!(page1.len(), 2);
    assert!(matches!(page1[0].kind, Annotation::Stroke(_)));
    assert!(matches!(page1[1].kind, Annotation::Highlight(_)));
}

#[test]
fn fresh_file_loads_as_empty_set() {
    let tmp = TempDir::new("empty");
    let db = tmp.path().join("ann.db");
    let store = AnnotationStore::open(&db).expect("open");

    let loaded = store.load().expect("load");
    assert!(loaded.is_empty());
    assert_eq!(loaded.len(), 0);
    assert!(loaded.ids().is_empty());

    // The empty set round-trips with `next_id` 0: the first add after a
    // reload gets id 0, not a skipped id.
    store.save(&loaded).expect("save empty");
    let mut again = store.load().expect("load");
    assert_eq!(again, loaded);
    let id = again
        .add(0, Annotation::TextNote(text_note()))
        .expect("add");
    assert_eq!(id, 0);
}

#[test]
fn ids_and_next_id_survive_reload_and_are_not_reused() {
    let mut set = sample_set(); // ids 0..3, next_id 4
    assert!(set.remove(1)); // leave a gap: ids 0, 2, 3

    let tmp = TempDir::new("ids");
    let db = tmp.path().join("ann.db");
    let store = AnnotationStore::open(&db).expect("open");

    store.save(&set).expect("save");
    let mut loaded = store.load().expect("load");
    assert_eq!(loaded, set);
    assert_eq!(loaded.ids(), vec![0, 2, 3]);

    // `next_id` was restored: the new id continues from 4, never reusing the
    // freed id 1.
    let new_id = loaded.add(0, Annotation::Stroke(stroke())).expect("add");
    assert_eq!(new_id, 4);
    assert_eq!(loaded.ids(), vec![0, 2, 3, 4]);
}

#[test]
fn persists_across_reopen_and_save_is_idempotent() {
    let tmp = TempDir::new("reopen");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(&library).expect("create library dir");
    let pdf = library.join("libro.pdf");
    let db = sidecar_path(&pdf);
    assert_eq!(db, library.join("annotations/libro.db"));

    let set = sample_set();
    {
        // `annotations/` does not exist yet: open must create it.
        let store = AnnotationStore::open(&db).expect("open creates dirs");
        assert_eq!(store.path(), db.as_path());
        store.save(&set).expect("save");
        store.save(&set).expect("save again (idempotent)");
    }

    let store = AnnotationStore::open(&db).expect("reopen");
    assert_eq!(store.load().expect("load"), set);
}
