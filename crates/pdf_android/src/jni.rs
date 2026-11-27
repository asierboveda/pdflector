// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Toda la interacción Java vía JNI: Intent de lanzamiento ("abrir con"),
//! MediaStore (biblioteca de PDFs del sistema), ContentResolver (copiar
//! bytes de una content:// URI) y Ajustes del permiso "All files access".
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): `reader` y
//! `input` llaman a estas funciones; los tipos de datos que construyen
//! (`LaunchPdf`, `LibraryEntry`, `LibraryScan`) viven en `reader`.
//!
//! Dividido por responsabilidad en submódulos (IN-03): `window` (pantalla
//! encendida, 120 Hz, inmersivo), `launch` (Intent de lanzamiento), `parse`
//! (ayudantes puros: percent-decoding, arXiv, nombres de PDF), `media`
//! (MediaStore, content:// URIs, Ajustes de permisos), `ime` (teclado) e
//! `ink` (capa de tinta). Los caminos `crate::jni::xxx` no cambian: aquí se
//! reexporta todo, y en host `host` aporta sustitutos inertes.

#[cfg(target_os = "android")]
use jni::objects::{JObject, JString};

#[cfg(target_os = "android")]
mod ime;
#[cfg(target_os = "android")]
mod ink;
#[cfg(target_os = "android")]
mod launch;
#[cfg(target_os = "android")]
mod media;
mod parse;
#[cfg(target_os = "android")]
mod window;

#[cfg(target_os = "android")]
pub(crate) use ime::{ime_attach, ime_hide, ime_set_text, ime_text};
#[cfg(target_os = "android")]
pub(crate) use ink::InkOverlay;
#[cfg(target_os = "android")]
pub(crate) use launch::launch_intent_request;
#[cfg(target_os = "android")]
pub(crate) use media::{
    android_sdk_int, launch_all_files_settings, query_media_store, read_content_uri_bytes,
};
// Glob: en host `percent_decode` solo lo usan los tests de abajo, y una
// reexportación explícita sin uso daría `unused_imports`.
pub(crate) use parse::*;
#[cfg(target_os = "android")]
pub(crate) use window::{enable_120hz, enter_immersive, keep_screen_on};

// En host (tests en Linux) no hay JVM: sustitutos inertes con la misma firma.
#[cfg(not(target_os = "android"))]
mod host;
#[cfg(not(target_os = "android"))]
pub(crate) use host::*;

/// Convierte un `JObject` que es un `java.lang.String` a `String` Rust (copia).
/// `new_cast_local_ref` verifica el tipo (IsInstanceOf) y crea una ref local.
#[cfg(target_os = "android")]
fn jni_jstring(env: &mut jni::Env, obj: &JObject) -> jni::errors::Result<String> {
    let jstr = env.new_cast_local_ref::<JString>(obj)?;
    jstr.try_to_string(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("hep-th%2F9901001"), "hep-th/9901001");
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("2401.12345"), "2401.12345");
        assert_eq!(
            percent_decode("pdflector%3A%2F%2Farxiv%2F2401.12345"),
            "pdflector://arxiv/2401.12345"
        );
    }

    #[test]
    fn test_sanitize_pdf_name_preserves_brackets() {
        assert_eq!(
            sanitize_pdf_name("BERT [1810.04805].pdf"),
            "BERT [1810.04805].pdf"
        );
        assert_eq!(
            sanitize_pdf_name("Paper: A Survey [2401.12345] (2).pdf"),
            "Paper_ A Survey [2401.12345] _2_.pdf"
        );
    }

    #[test]
    fn test_parse_arxiv_target_variants() {
        // Modern IDs
        assert_eq!(
            parse_arxiv_target("2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("2401.12345v2").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("0706.0001").as_deref(),
            Some("0706.0001")
        );

        // Classic IDs with subject/archive
        assert_eq!(
            parse_arxiv_target("hep-th/9901001").as_deref(),
            Some("hep-th/9901001")
        );
        assert_eq!(
            parse_arxiv_target("hep-th/9901001v3").as_deref(),
            Some("hep-th/9901001")
        );

        // Direct arXiv URLs (abs and pdf)
        assert_eq!(
            parse_arxiv_target("https://arxiv.org/abs/2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("https://arxiv.org/pdf/2401.12345.pdf").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("http://arxiv.org/abs/hep-th/9901001").as_deref(),
            Some("hep-th/9901001")
        );
        assert_eq!(
            parse_arxiv_target("arxiv.org/abs/2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("arxiv.org/pdf/2401.12345").as_deref(),
            Some("2401.12345")
        );

        // Export arXiv URLs
        assert_eq!(
            parse_arxiv_target("https://export.arxiv.org/abs/2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("https://export.arxiv.org/pdf/2401.12345.pdf").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("export.arxiv.org/abs/2401.12345").as_deref(),
            Some("2401.12345")
        );
        // Trailing and leading punctuation variants
        assert_eq!(
            parse_arxiv_target("https://arxiv.org/abs/2401.12345.").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("https://arxiv.org/pdf/2401.12345.pdf,").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("(https://arxiv.org/abs/2401.12345)").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("Check this out: https://arxiv.org/abs/2401.12345!").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("2401.12345.").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("hep-th/9901001,").as_deref(),
            Some("hep-th/9901001")
        );

        // Custom scheme pdflector://
        assert_eq!(
            parse_arxiv_target("pdflector://arxiv/2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("pdflector://arxiv/hep-th%2F9901001").as_deref(),
            Some("hep-th/9901001")
        );
        assert_eq!(
            parse_arxiv_target("pdflector://arxiv/hep-th/9901001").as_deref(),
            Some("hep-th/9901001")
        );
        assert_eq!(
            parse_arxiv_target("pdflector://2401.12345").as_deref(),
            Some("2401.12345")
        );

        // PaperTok base64url keys
        // "arxiv:2401.12345" base64url = "YXJ4aXY6MjQwMS4xMjM0NQ"
        assert_eq!(
            parse_arxiv_target("https://papertok.app/#/public/paper/YXJ4aXY6MjQwMS4xMjM0NQ")
                .as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target("https://papertok.app/public/paper/YXJ4aXY6MjQwMS4xMjM0NQ")
                .as_deref(),
            Some("2401.12345")
        );

        // Non-arXiv PaperTok keys (e.g. "doi:10.1000/182" base64url = "ZG9pOjEwLjEwMDAvMTgy")
        assert_eq!(
            parse_arxiv_target("https://papertok.app/#/public/paper/ZG9pOjEwLjEwMDAvMTgy"),
            None
        );

        // Plain non-arXiv URLs and arbitrary text
        assert_eq!(parse_arxiv_target("https://doi.org/10.1000/182"), None);
        assert_eq!(parse_arxiv_target("https://example.com/paper.pdf"), None);
        assert_eq!(parse_arxiv_target("hello world"), None);
        assert_eq!(parse_arxiv_target(""), None);

        // Mixed shared text (e.g. from PaperTok or browser share dialog)
        assert_eq!(
            parse_arxiv_target("Check out this paper: https://arxiv.org/abs/2401.12345").as_deref(),
            Some("2401.12345")
        );
        assert_eq!(
            parse_arxiv_target(
                "Title of Paper\nhttps://papertok.app/#/public/paper/YXJ4aXY6MjQwMS4xMjM0NQ"
            )
            .as_deref(),
            Some("2401.12345")
        );
    }

    #[test]
    fn ime_jni_borrows_global_class_ref_and_clears_exceptions() {
        let src = include_str!("jni/ime.rs");
        assert!(
            !src.contains("JClass::from_raw("),
            "jni/ime.rs must borrow the GlobalRef via env.as_cast_raw::<JClass>, never wrap it in an owning JClass::from_raw"
        );
        assert!(
            src.contains("as_cast_raw::<JClass>"),
            "jni/ime.rs must cast the cached GlobalRef non-owningly with as_cast_raw::<JClass>"
        );
        // Every function calling attach_current_thread must clear pending Java exceptions on Err.
        let attach_calls = src.matches("attach_current_thread(").count();
        let clear_calls = src.matches("exception_clear()").count();
        assert!(
            clear_calls >= attach_calls,
            "expected at least {attach_calls} exception_clear() guards in jni/ime.rs, found {clear_calls}"
        );
    }
}
