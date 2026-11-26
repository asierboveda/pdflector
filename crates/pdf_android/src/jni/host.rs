// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Sustitutos de host (Linux) de las funciones JNI de `jni.rs`.
//!
//! Solo existen para que `cargo test -p pdf_android --lib` compile y ejecute
//! los tests puros fuera de Android. No hay JVM: todo es inerte y devuelve el
//! valor "sin datos" que la app ya maneja cuando una llamada JNI falla.

use crate::android_activity::AndroidApp;
use crate::reader::{LaunchRequest, LibraryScan};

pub(crate) fn keep_screen_on(_app: &AndroidApp) {}

pub(crate) fn enable_120hz(_app: &AndroidApp) {}

pub(crate) fn enter_immersive(_app: &AndroidApp) {}

pub(crate) fn launch_intent_request(_app: &AndroidApp) -> Option<LaunchRequest> {
    None
}

pub(crate) fn android_sdk_int() -> i32 {
    0
}

pub(crate) fn query_media_store(_app: &AndroidApp, _sdk_int: i32) -> LibraryScan {
    LibraryScan {
        entries: Vec::new(),
        permission_granted: false,
        error: Some("MediaStore no disponible en host".to_string()),
    }
}

pub(crate) fn read_content_uri_bytes(_app: &AndroidApp, _uri_str: &str) -> Option<Vec<u8>> {
    None
}

pub(crate) fn launch_all_files_settings(_app: &AndroidApp) {}

pub(crate) fn ime_attach(_app: &AndroidApp, _initial: &str) {}

pub(crate) fn ime_hide(_app: &AndroidApp) {}

pub(crate) fn ime_text(_app: &AndroidApp) -> Option<String> {
    None
}

pub(crate) fn ime_set_text(_app: &AndroidApp, _text: &str) {}

/// Sin capa wet en host: `new` siempre devuelve `None`, así que el Reader
/// dibuja la tinta solo por la ruta seca, como cuando el puente falla en Android.
pub(crate) struct InkOverlay;

impl InkOverlay {
    pub(crate) fn new(_app: &AndroidApp) -> Option<Self> {
        None
    }

    pub(crate) fn render_segment(
        &self,
        _x0: f32,
        _y0: f32,
        _x1: f32,
        _y1: f32,
        _width_px: f32,
        _color_argb: i32,
    ) -> bool {
        false
    }

    pub(crate) fn commit(&self) {}

    pub(crate) fn cancel(&self) {}

    pub(crate) fn clear(&self) {}

    pub(crate) fn is_ready(&self) -> bool {
        false
    }
}
