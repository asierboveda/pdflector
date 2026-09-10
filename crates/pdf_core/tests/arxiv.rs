// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

#![allow(clippy::unwrap_used)]

use pdf_core::arxiv::{ArxivError, ArxivId, ArxivQuery, parse_arxiv_id};

#[test]
fn parse_accepts_all_id_shapes() {
    assert_eq!(parse_arxiv_id("2401.12345").unwrap().0, "2401.12345");
    assert_eq!(
        parse_arxiv_id("https://arxiv.org/abs/hep-th/9901001v3")
            .unwrap()
            .0,
        "hep-th/9901001"
    );
    assert_eq!(parse_arxiv_id("arXiv:0706.0001v2").unwrap().0, "0706.0001");
    assert!(parse_arxiv_id("hep-th_9901001").is_err());
}

#[test]
fn parse_handles_urls_and_versions() {
    let (id, ver) = parse_arxiv_id("https://arxiv.org/pdf/2401.12345v2.pdf").unwrap();
    assert_eq!(id, "2401.12345");
    assert_eq!(ver, Some("2".to_string()));

    let (id, ver) = parse_arxiv_id("http://arxiv.org/abs/0706.0001").unwrap();
    assert_eq!(id, "0706.0001");
    assert_eq!(ver, None);
}

#[test]
fn query_builder_to_params() {
    let q = ArxivQuery::new()
        .with_search_query("cat:cs.AI")
        .with_id_list(vec!["2401.12345".into(), "0706.0001".into()])
        .with_start(10)
        .with_max_results(25)
        .with_sort_by("submittedDate")
        .with_sort_order("descending");

    let params = q.to_params();
    assert_eq!(
        params,
        vec![
            ("search_query".to_string(), "cat:cs.AI".to_string()),
            ("id_list".to_string(), "2401.12345,0706.0001".to_string()),
            ("start".to_string(), "10".to_string()),
            ("max_results".to_string(), "25".to_string()),
            ("sortBy".to_string(), "submittedDate".to_string()),
            ("sortOrder".to_string(), "descending".to_string()),
        ]
    );

    let empty_q = ArxivQuery::new();
    assert!(empty_q.to_params().is_empty());
}

#[test]
fn arxiv_id_variants_and_error_display() {
    let _m = ArxivId::Modern("2401.12345".into());
    let _m4 = ArxivId::Modern4("0706.0001".into());
    let _c = ArxivId::Classic("hep-th/9901001".into());

    let err = ArxivError::InvalidId("bad_id".into());
    assert!(err.to_string().contains("bad_id"));
}
