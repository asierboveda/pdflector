// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Intent de lanzamiento de la Activity: "abrir con" (content:// y
//! file://), `pdflector://` y SEND text/plain (peticiones de arXiv).

use std::fs;
use std::path::Path;

use crate::android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use log::{error, info, warn};

use super::jni_jstring;
use super::media::drain_stream;
use super::parse::{percent_decode, sanitize_pdf_name};
use crate::reader::{LaunchPdf, LaunchRequest};

/// Lee el Intent de lanzamiento de la Activity (JNI) y, si es un "abrir con"
/// de un PDF (`ACTION_VIEW` + `data`), prepara el documento para abrirlo:
///
/// - `content://` URI → copia los bytes a `internal_data_path()/pdfs/<nombre>`
///   con `ContentResolver.openInputStream` (el sistema ya concedió
///   `FLAG_GRANT_READ_URI_PERMISSION` al lanzar "abrir con"; la copia a
///   almacenamiento interno esquiva Scoped Storage y deja MuPDF abriendo por
///   ruta normal, sin necesidad de conservar el permiso). El nombre viene de
///   `ContentResolver.query` (`OpenableColumns.DISPLAY_NAME`), con fallbacks.
/// - `file://` ruta → la devuelve directamente (abre el PDF in situ).
/// - `pdflector://` URI → petición remota para descargar de arXiv.
/// - `android.intent.action.SEND` text/plain → petición remota de arXiv desde EXTRA_TEXT.
/// - Sin intent relevante (lanzamiento normal desde el launcher) → `None`.
///
/// JNI: `AndroidApp::activity_as_ptr()` expone la `Activity` (android-activity
/// 0.6, global ref unowned) y `JavaVM::singleton()` el VM.
pub(crate) fn launch_intent_request(app: &AndroidApp) -> Option<LaunchRequest> {
    let vm = JavaVM::singleton().ok()?;
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let internal_dir = app.internal_data_path();

    let res: jni::errors::Result<Option<LaunchRequest>> = vm.attach_current_thread(|env| {
        env.with_local_frame(64, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app` (no se
            // dropea al salir del scope). Mismo patrón que la doc de jni 0.22.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let intent = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getIntent"),
                    jni_sig!(sig = () -> android.content.Intent),
                    &[],
                )?
                .l()?;
            if intent.is_null() {
                return Ok(None);
            }

            // 1. getAction() y EXTRA_TEXT antes del early-return de data.is_null()
            let action = match env
                .call_method(
                    intent.as_ref(),
                    jni_str!("getAction"),
                    jni_sig!(sig = () -> java.lang.String),
                    &[],
                )?
                .l()?
            {
                s if s.is_null() => String::new(),
                s => jni_jstring(env, &s)?,
            };

            let extra_text = {
                let key = env.new_string("android.intent.extra.TEXT")?;
                match env
                    .call_method(
                        intent.as_ref(),
                        jni_str!("getStringExtra"),
                        jni_sig!(sig = (java.lang.String) -> java.lang.String),
                        &[JValue::Object(key.as_ref())],
                    )?
                    .l()?
                {
                    s if s.is_null() => None,
                    s => Some(jni_jstring(env, &s)?),
                }
            };

            // ACTION_SEND: compartir texto / URL a PDFLector (ej. desde PaperTok o navegador)
            if action == "android.intent.action.SEND" {
                let text = extra_text.unwrap_or_default();
                info!("launch_intent_request: ACTION_SEND text='{text}'");
                return Ok(Some(LaunchRequest::Remote(text)));
            }

            if let Some(text) = extra_text
                && action != "android.intent.action.MAIN"
            {
                info!("launch_intent_request: action={action} extra_text='{text}'");
                return Ok(Some(LaunchRequest::Remote(text)));
            }

            // Intent.getData() → Uri; null en un lanzamiento normal.
            let data = env
                .call_method(
                    intent.as_ref(),
                    jni_str!("getData"),
                    jni_sig!(sig = () -> android.net.Uri),
                    &[],
                )?
                .l()?;
            if data.is_null() {
                return Ok(None);
            }
            let uri_str = match env
                .call_method(
                    data.as_ref(),
                    jni_str!("toString"),
                    jni_sig!(sig = () -> java.lang.String),
                    &[],
                )?
                .l()?
            {
                s if s.is_null() => "?".to_string(),
                s => jni_jstring(env, &s)?,
            };
            let scheme = match env
                .call_method(
                    data.as_ref(),
                    jni_str!("getScheme"),
                    jni_sig!(sig = () -> java.lang.String),
                    &[],
                )?
                .l()?
            {
                s if s.is_null() => String::new(),
                s => jni_jstring(env, &s)?,
            };
            match scheme.as_str() {
                "pdflector" => {
                    let decoded = percent_decode(&uri_str);
                    info!("launch_intent_request: pdflector URI '{uri_str}' -> '{decoded}'");
                    Ok(Some(LaunchRequest::Remote(decoded)))
                }
                "content" => {
                    let resolver = env
                        .call_method(
                            activity.as_ref(),
                            jni_str!("getContentResolver"),
                            jni_sig!(sig = () -> android.content.ContentResolver),
                            &[],
                        )?
                        .l()?;
                    // Nombre del fichero: OpenableColumns.DISPLAY_NAME vía
                    // ContentResolver.query (proyección null → todas las
                    // columnas; el proveedor de DocumentosProvider la acepta).
                    let mut name = String::new();
                    let cursor = env
                        .call_method(
                            resolver.as_ref(),
                            jni_str!("query"),
                            jni_sig!(
                                sig = (android.net.Uri, [java.lang.String], java.lang.String, [java.lang.String], java.lang.String) -> android.database.Cursor
                            ),
                            &[
                                JValue::Object(data.as_ref()),
                                JValue::Object(JObject::null().as_ref()),
                                JValue::Object(JObject::null().as_ref()),
                                JValue::Object(JObject::null().as_ref()),
                                JValue::Object(JObject::null().as_ref()),
                            ],
                        )?
                        .l()?;
                    if !cursor.is_null() {
                        let moved = env
                            .call_method(
                                cursor.as_ref(),
                                jni_str!("moveToFirst"),
                                jni_sig!(sig = () -> boolean),
                                &[],
                            )?
                            .z()?;
                        if moved {
                            let display = env.new_string("_display_name")?;
                            let idx = env
                                .call_method(
                                    cursor.as_ref(),
                                    jni_str!("getColumnIndex"),
                                    jni_sig!(sig = (java.lang.String) -> int),
                                    &[JValue::Object(display.as_ref())],
                                )?
                                .i()?;
                            env.delete_local_ref(display);
                            if idx >= 0 {
                                let jname = env
                                    .call_method(
                                        cursor.as_ref(),
                                        jni_str!("getString"),
                                        jni_sig!(sig = (int) -> java.lang.String),
                                        &[JValue::Int(idx)],
                                    )?
                                    .l()?;
                                if !jname.is_null() {
                                    name = jni_jstring(env, &jname)?;
                                }
                            }
                        }
                        env.call_method(
                            cursor.as_ref(),
                            jni_str!("close"),
                            jni_sig!(sig = () -> void),
                            &[],
                        )?;
                    }
                    if name.is_empty() {
                        // Fallback: último segmento de la ruta (p. ej. un ID).
                        let seg = env
                            .call_method(
                                data.as_ref(),
                                jni_str!("getLastPathSegment"),
                                jni_sig!(sig = () -> java.lang.String),
                                &[],
                            )?
                            .l()?;
                        if !seg.is_null() {
                            name = jni_jstring(env, &seg)?;
                        }
                    }
                    let name = sanitize_pdf_name(&name);
                    // Copia bytes → internal/pdfs/<name> (permiso de lectura
                    // del content:// no hace falta conservarlo tras copiar).
                    let stream = env
                        .call_method(
                            resolver.as_ref(),
                            jni_str!("openInputStream"),
                            jni_sig!(sig = (android.net.Uri) -> java.io.InputStream),
                            &[JValue::Object(data.as_ref())],
                        )?
                        .l()?;
                    if stream.is_null() {
                        error!("open-with {uri_str}: openInputStream null");
                        return Ok(None);
                    }
                    let bytes = drain_stream(env, &stream)?;
                    info!("open-with {uri_str}: {} bytes para {}", bytes.len(), name);
                    let Some(dir) = internal_dir else {
                        error!("open-with: internal_data_path unavailable");
                        return Ok(None);
                    };
                    let pdfs_dir = dir.join("pdfs");
                    if let Err(e) = fs::create_dir_all(&pdfs_dir) {
                        error!("open-with: create_dir_all {}: {e}", pdfs_dir.display());
                        return Ok(None);
                    }
                    let dest = pdfs_dir.join(&name);
                    if let Err(e) = fs::write(&dest, bytes) {
                        error!("open-with: write {}: {e}", dest.display());
                        return Ok(None);
                    }
                    Ok(Some(LaunchRequest::File(LaunchPdf {
                        name,
                        source: uri_str,
                        path: dest.display().to_string(),
                    })))
                }
                "file" => {
                    // file:// → ruta local directa (abre el PDF in situ).
                    let path = match env
                        .call_method(
                            data.as_ref(),
                            jni_str!("getPath"),
                            jni_sig!(sig = () -> java.lang.String),
                            &[],
                        )?
                        .l()?
                    {
                        s if s.is_null() => return Ok(None),
                        s => jni_jstring(env, &s)?,
                    };
                    let name = Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "external.pdf".to_string());
                    info!("open-with file://: {path}");
                    Ok(Some(LaunchRequest::File(LaunchPdf {
                        name,
                        source: uri_str,
                        path,
                    })))
                }
                "http" | "https" => {
                    let decoded = percent_decode(&uri_str);
                    info!("launch_intent_request: http(s) URI '{uri_str}' -> '{decoded}'");
                    Ok(Some(LaunchRequest::Remote(decoded)))
                }
                other => {
                    warn!("open-with {uri_str}: scheme '{other}' no soportado");
                    Ok(None)
                }
            }
        })
    });
    match res {
        Ok(Some(lp)) => Some(lp),
        Ok(None) => None,
        Err(e) => {
            // Si el error fue una excepción Java, queda pendiente en el JVM:
            // limpiarla para no envenenar llamadas JNI posteriores.
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("launch_intent_request: {e}");
            None
        }
    }
}
