// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Fase 3 annotations benchmark: does the vector annotation model (Fase 3,
//! docs/PLAN.md §3.4) scale to a real document with hundreds of annotations?
//!
//! The Fase 3 acceptance criterion is that 200+ visible strokes must not
//! degrade frame time, and persistence (SQLite sidecar) must stay cheap for
//! hundreds of annotations. This bench measures the four hot paths of the
//! model:
//!
//!   annotations/add             — cost per `AnnotationSet::add`, the ingest
//!                                 path every pen stroke pays, for a set
//!                                 growing to 100/1000 annotations.
//!   annotations/for_page        — cost of `for_page` on a page holding 200
//!                                 annotations out of 1000 in the set: the
//!                                 lookup the draw path pays every frame
//!                                 (the criterion's "200+ strokes" case).
//!   annotations/serialize       — serde_json to_string/from_str of the whole
//!                                 set: the JSON persist cost that bounds a
//!                                 save (user action, not per frame).
//!   annotations/store_roundtrip — `AnnotationStore::save` + `load` over a
//!                                 real temp SQLite sidecar: the full
//!                                 persist+reload cycle.
//!
//! Run with:
//!     cargo bench -p pdf_bench --bench annotations
//! Results land in target/criterion/<group>/report/.

use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use pdf_core::AnnotationStore;
use pdf_core::annotations::{Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, TextNote};

/// Page that carries the "hot" 200 annotations in the spread sets.
const HOT_PAGE: usize = 0;
/// Set sizes measured (small vs. a real heavily-annotated document).
const N_SMALL: usize = 100;
const N_LARGE: usize = 1000;
/// Pages the spread sets are distributed over.
const PAGES: usize = 50;
/// The Fase 3 criterion threshold: 200 strokes on one page.
const HOT: usize = 200;

fn color() -> Color {
    Color {
        r: 255,
        g: 0,
        b: 0,
        a: 200,
    }
}

fn stroke_kind() -> Annotation {
    Annotation::Stroke(
        Stroke::new(
            vec![(10.0, 20.0), (30.5, 40.25), (50.0, 60.0)],
            2.5,
            color(),
        )
        .expect("valid stroke"),
    )
}

fn highlight_kind() -> Annotation {
    Annotation::Highlight(Highlight {
        rects: vec![
            Rect::new(10.0, 20.0, 100.0, 12.0),
            Rect::new(10.0, 34.0, 80.0, 12.0),
        ],
        color: color(),
    })
}

fn text_note_kind() -> Annotation {
    Annotation::TextNote(TextNote {
        anchor: (5.0, 5.0),
        text: "revisar §3".to_string(),
    })
}

/// The three supported kinds, cycled, so sets mix strokes, highlights and
/// notes like a real annotated document.
fn kind_for(i: usize) -> Annotation {
    match i % 3 {
        0 => stroke_kind(),
        1 => highlight_kind(),
        _ => text_note_kind(),
    }
}

/// Builds a set of `total` annotations over `pages` pages, with `hot` of
/// them on `HOT_PAGE` (the remainder spread over the other pages) — the
/// shape a real heavily-annotated document takes. Requires `pages >= 2`.
fn build_set(total: usize, pages: usize, hot: usize) -> AnnotationSet {
    let mut set = AnnotationSet::new();
    for i in 0..total {
        let page = if i < hot {
            HOT_PAGE
        } else {
            HOT_PAGE + 1 + (i % (pages - 1))
        };
        set.add(page, kind_for(i)).expect("add valid annotation");
    }
    set
}

/// Cost per `AnnotationSet::add`: N inserts into one set. Throughput is set
/// to Elements(N), so the report is the per-add cost. The UI pays this once
/// per stroke drawn: with the Fase 3 budget of 200+ strokes this path must
/// stay cheap (it runs on the UI thread during inking, though off the render
/// path itself).
fn bench_add(c: &mut Criterion) {
    // Pre-built kinds: the iteration clones one, so the measurement is the
    // ingest cost (id assignment, normalization, map insert), not the
    // geometry construction.
    let pool = [stroke_kind(), highlight_kind(), text_note_kind()];
    let mut g = c.benchmark_group("annotations/add");
    for n in [N_SMALL, N_LARGE] {
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(format!("n{n}")), &n, |b, &n| {
            // Fresh set per iteration: SmallInput re-runs the (cheap) setup.
            b.iter_batched(
                AnnotationSet::new,
                |mut set| {
                    for i in 0..n {
                        black_box(set.add(i % 10, pool[i % 3].clone()));
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }
    g.finish();
}

/// Cost of `for_page` on the page holding 200 annotations (1000 in the set
/// over 50 pages): the lookup the draw path pays every frame, and the direct
/// check of the Fase 3 criterion (200+ visible strokes without degrading
/// frame time). The result is a `Vec<&Annotated>` allocated per call — that
/// allocation is part of what is measured.
fn bench_for_page(c: &mut Criterion) {
    let set = build_set(N_LARGE, PAGES, HOT);
    let mut g = c.benchmark_group("annotations/for_page");
    g.throughput(Throughput::Elements(HOT as u64));
    g.bench_function("200_of_1000", |b| {
        b.iter(|| {
            let anns = set.for_page(HOT_PAGE);
            black_box(anns.len());
        });
    });
    g.finish();
}

/// Cost of persisting the whole set as JSON: `to_string` (encode) and
/// `from_str` (decode). The sidecar store (store.rs) writes one JSON payload
/// per annotation row, so this group bounds the per-annotation JSON cost of a
/// save/load cycle. Saves happen on user action, never per frame; the budget
/// is that hundreds of annotations serialize in low ms.
fn bench_serialize(c: &mut Criterion) {
    let mut g = c.benchmark_group("annotations/serialize");
    for n in [N_SMALL, N_LARGE] {
        let set = build_set(n, PAGES, HOT.min(n));
        // Encoded once up front: `to_string` measures encode, `from_str`
        // measures decode of the exact bytes a save would write.
        let json = serde_json::to_string(&set).expect("serialize set");
        g.throughput(Throughput::Bytes(json.len() as u64));
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("to_string_n{n}")),
            &set,
            |b, set| b.iter(|| black_box(serde_json::to_string(set).expect("serialize").len())),
        );
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("from_str_n{n}")),
            &json,
            |b, json| {
                b.iter(|| {
                    let back: AnnotationSet = serde_json::from_str(json).expect("deserialize");
                    black_box(back.len());
                });
            },
        );
    }
    g.finish();
}

/// Cost of one full persist+reload cycle on a real SQLite sidecar
/// (`AnnotationStore::save` + `load` on a temp .db): the persistence path of
/// Fase 3 §3.5. Saves happen on user action (pen up / close), not per frame;
/// the budget is that hundreds of annotations round-trip without a
/// perceptible hitch.
fn bench_store_roundtrip(c: &mut Criterion) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut g = c.benchmark_group("annotations/store_roundtrip");
    for n in [N_SMALL, N_LARGE] {
        let set = build_set(n, PAGES, HOT.min(n));
        // Unique temp sidecar per parameter, so parallel bench processes (or
        // repeated runs) never clobber each other's database.
        let db = std::env::temp_dir().join(format!(
            "pdflector_annotations_bench_{}_{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let store = AnnotationStore::open(&db).expect("open sidecar");
        // Sanity check once, outside the timed loop: the round-trip must
        // preserve the set size.
        store.save(&set).expect("save sanity");
        assert_eq!(store.load().expect("load sanity").len(), n);

        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("save_load_n{n}")),
            &(store, set),
            |b, (store, set)| {
                b.iter(|| {
                    store.save(set).expect("save");
                    let loaded = store.load().expect("load");
                    black_box(loaded.len());
                });
            },
        );
        // Best-effort cleanup of the temp sidecar (the store keeps the
        // connection open, but nothing writes to it after the bench ends).
        let _ = std::fs::remove_file(&db);
    }
    g.finish();
}

fn benches(c: &mut Criterion) {
    bench_add(c);
    bench_for_page(c);
    bench_serialize(c);
    bench_store_roundtrip(c);
}

criterion_group!(all, benches);
criterion_main!(all);
