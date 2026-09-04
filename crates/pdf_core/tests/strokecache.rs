// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Fase C2: `StrokeCache` para la composición de la capa de anotaciones.
//!
//! Evita re-rasterizar N trazos por frame cuando la página no ha cambiado de
//! trazos. Comprueba que el hit de caché devuelve la capa existente (sin
//! re-rasterizar), que la invalidación por página funciona, y que la
//! composición con caché produce un resultado idéntico pixel a pixel a
//! `composite_annotations`.

use std::sync::Arc;

use pdf_core::strokecache::{StrokeCache, StrokeKey};
use pdf_core::{
    Annotated, Annotation, AnnotationSet, Color, Highlight, Rect, Stroke, ViewTransform,
    composite_annotations,
};

fn sample_stroke(y: f32) -> Stroke {
    Stroke::new(
        vec![(10.0, y), (50.0, y + 5.0), (100.0, y)],
        2.5,
        Color {
            r: 200,
            g: 20,
            b: 20,
            a: 255,
        },
    )
    .unwrap()
}

fn sample_highlight(y: f32) -> Highlight {
    Highlight {
        rects: vec![Rect::new(10.0, y, 90.0, 14.0)],
        color: Color {
            r: 255,
            g: 240,
            b: 0,
            a: 128,
        },
    }
}

#[test]
fn cache_miss_renders_strokes_and_subsequent_call_hits() {
    let mut cache = StrokeCache::new(4);
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Stroke(sample_stroke(20.0)));
    set.add(0, Annotation::Stroke(sample_stroke(50.0)));
    let anns: Vec<Annotated> = set.for_page(0).into_iter().cloned().collect();
    let refs: Vec<&Annotated> = anns.iter().collect();

    let key = StrokeKey::new(0, 1.0, 200, 200);
    let xform = ViewTransform::IDENTITY;

    // Primer acceso (miss): genera el bitmap de la capa de tinta
    let first = cache.get_or_render(key, &refs, &xform);
    assert!(
        first.is_some(),
        "debe generar la capa para trazos existentes"
    );
    let first_arc = first.unwrap();
    assert_eq!(first_arc.width, 200);
    assert_eq!(first_arc.height, 200);

    // Segundo acceso (hit): debe devolver exactamente el mismo Arc
    let second = cache.get_or_render(key, &refs, &xform);
    assert!(second.is_some());
    let second_arc = second.unwrap();
    assert!(
        Arc::ptr_eq(&first_arc, &second_arc),
        "el hit debe devolver el mismo Arc sin re-rasterizar"
    );
}

#[test]
fn cache_returns_none_when_no_strokes() {
    let mut cache = StrokeCache::new(4);
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Highlight(sample_highlight(20.0)));
    let anns: Vec<Annotated> = set.for_page(0).into_iter().cloned().collect();
    let refs: Vec<&Annotated> = anns.iter().collect();

    let key = StrokeKey::new(0, 1.0, 200, 200);
    let layer = cache.get_or_render(key, &refs, &ViewTransform::IDENTITY);
    assert!(
        layer.is_none(),
        "sin trazos no debe alocar bitmap para la capa de tinta"
    );
}

#[test]
fn invalidate_page_evicts_cached_layer() {
    let mut cache = StrokeCache::new(4);
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Stroke(sample_stroke(20.0)));
    let anns: Vec<Annotated> = set.for_page(0).into_iter().cloned().collect();
    let refs: Vec<&Annotated> = anns.iter().collect();

    let key = StrokeKey::new(0, 1.0, 200, 200);
    let first = cache
        .get_or_render(key, &refs, &ViewTransform::IDENTITY)
        .unwrap();

    // Invalidar página 0
    cache.invalidate_page(0);

    // Próximo acceso debe ser miss y generar un nuevo Arc
    let second = cache
        .get_or_render(key, &refs, &ViewTransform::IDENTITY)
        .unwrap();
    assert!(
        !Arc::ptr_eq(&first, &second),
        "tras invalidar la página debe re-generar la capa"
    );
}

#[test]
fn different_zoom_levels_are_independent_entries() {
    let mut cache = StrokeCache::new(4);
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Stroke(sample_stroke(20.0)));
    let anns: Vec<Annotated> = set.for_page(0).into_iter().cloned().collect();
    let refs: Vec<&Annotated> = anns.iter().collect();

    let key1 = StrokeKey::new(0, 1.0, 200, 200);
    let xform1 = ViewTransform {
        zoom: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };
    let arc1 = cache.get_or_render(key1, &refs, &xform1).unwrap();

    let key2 = StrokeKey::new(0, 2.0, 400, 400);
    let xform2 = ViewTransform {
        zoom: 2.0,
        offset_x: 0.0,
        offset_y: 0.0,
    };
    let arc2 = cache.get_or_render(key2, &refs, &xform2).unwrap();

    assert_eq!(arc1.width, 200);
    assert_eq!(arc2.width, 400);
    assert!(!Arc::ptr_eq(&arc1, &arc2));
}

#[test]
fn composite_with_cache_matches_composite_annotations_output() {
    let mut cache = StrokeCache::new(4);
    let mut set = AnnotationSet::new();
    set.add(0, Annotation::Highlight(sample_highlight(15.0)));
    set.add(0, Annotation::Stroke(sample_stroke(30.0)));
    set.add(0, Annotation::Stroke(sample_stroke(60.0)));
    set.add(0, Annotation::Highlight(sample_highlight(80.0)));

    let anns: Vec<Annotated> = set.for_page(0).into_iter().cloned().collect();
    let refs: Vec<&Annotated> = anns.iter().collect();

    let (w, h) = (200u32, 200u32);
    let xform = ViewTransform::IDENTITY;

    // Buffer 1: render directo con `composite_annotations`
    let mut buf_direct = vec![255u8; (w * h * 4) as usize];
    composite_annotations(&mut buf_direct, w, h, &refs, &xform);

    // Buffer 2: render usando `StrokeCache::composite`
    let mut buf_cached = vec![255u8; (w * h * 4) as usize];
    cache.composite(&mut buf_cached, w, h, 0, &refs, &xform);
    let mut max_diff = 0u8;
    for (&d, &c) in buf_direct.iter().zip(buf_cached.iter()) {
        max_diff = max_diff.max(d.abs_diff(c));
    }
    assert!(
        max_diff <= 2,
        "diferencia máxima entre directo y cacheado debe ser <= 2 por cuantización de 8 bits, obtenido: {max_diff}"
    );
    let mut buf_cached2 = vec![255u8; (w * h * 4) as usize];
    cache.composite(&mut buf_cached2, w, h, 0, &refs, &xform);
    assert_eq!(buf_cached, buf_cached2);
}
