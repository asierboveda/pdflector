// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Biblioteca del sistema vía JNI: nivel de API, consulta de PDFs en
//! MediaStore, lectura de bytes de una content:// URI y Ajustes del
//! permiso "All files access".

use crate::android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use log::{error, info};

use super::jni_jstring;
use crate::reader::{LibraryEntry, LibraryScan};

/// Nivel de API de Android (Build.VERSION.SDK_INT). Decide qué columnas de
/// MediaStore existen (RELATIVE_PATH/_SIZE solo desde API 29) y qué permiso
/// exige la lectura de PDFs ajenos (MANAGE_EXTERNAL_STORAGE desde API 30).
pub(crate) fn android_sdk_int() -> i32 {
    let vm = match JavaVM::singleton() {
        Ok(v) => v,
        Err(e) => {
            error!("android_sdk_int: JVM no disponible: {e}");
            return 0;
        }
    };
    let res: jni::errors::Result<i32> = vm.attach_current_thread(|env| {
        env.with_local_frame(8, |env| {
            let class = env.find_class(jni_str!("android/os/Build$VERSION"))?;
            env.get_static_field(&class, jni_str!("SDK_INT"), jni_sig!(sig = int))
                .map(|v| v.i().unwrap_or(0))
        })
    });
    match res {
        Ok(sdk) => sdk,
        Err(e) => {
            // Excepción Java pendiente: limpiarla para no envenenar JNI posterior.
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("android_sdk_int: {e}");
            0
        }
    }
}

/// Consulta MediaStore (`MediaStore.Files.getContentUri("external")`) pidiendo
/// los PDFs del sistema: proyección `[_ID, DISPLAY_NAME, RELATIVE_PATH,
/// _SIZE]`, selección `mime_type='application/pdf'` y orden por carpeta +
/// nombre. Cada fila se convierte a content URI con `ContentUris.withAppendedId`.
///
/// En API 30+ (Android 11+) sin el appop "All files access" el proveedor
/// lanza SecurityException al leer documentos ajenos: se detecta antes con
/// `Environment.isExternalStorageManager()` y no se consulta (se devuelve
/// `permission_granted=false`); si la query fallara igualmente, se captura la
/// excepción y se reporta el error.
pub(crate) fn query_media_store(app: &AndroidApp, sdk_int: i32) -> LibraryScan {
    let vm = match JavaVM::singleton() {
        Ok(v) => v,
        Err(e) => {
            error!("query_media_store: JVM no disponible: {e}");
            return LibraryScan {
                entries: Vec::new(),
                permission_granted: false,
                error: Some("JVM unavailable".to_string()),
            };
        }
    };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<LibraryScan> = vm.attach_current_thread(|env| {
        env.with_local_frame(128, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app` (mismo
            // patrón que `launch_intent_pdf`).
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };

            // API 30+: comprobar el appop antes de consultar (sin él, el
            // proveedor lanza SecurityException). En ≤ 12 el acceso a
            // documentos se asume cubierto por READ_EXTERNAL_STORAGE (manifest).
            let mut permission_granted = sdk_int < 30;
            if sdk_int >= 30 {
                let env_class = env.find_class(jni_str!("android/os/Environment"))?;
                permission_granted = env
                    .call_static_method(
                        &env_class,
                        jni_str!("isExternalStorageManager"),
                        jni_sig!(sig = () -> boolean),
                        &[],
                    )?
                    .z()?;
            }
            if !permission_granted {
                return Ok(LibraryScan {
                    entries: Vec::new(),
                    permission_granted: false,
                    error: None,
                });
            }

            let resolver = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getContentResolver"),
                    jni_sig!(sig = () -> android.content.ContentResolver),
                    &[],
                )?
                .l()?;

            // content://media/external/file
            let files_class = env.find_class(jni_str!("android/provider/MediaStore$Files"))?;
            let vol = env.new_string("external")?;
            let files_uri = env
                .call_static_method(
                    &files_class,
                    jni_str!("getContentUri"),
                    jni_sig!(sig = (java.lang.String) -> android.net.Uri),
                    &[JValue::Object(vol.as_ref())],
                )?
                .l()?;

            // Proyección: RELATIVE_PATH y _SIZE solo existen desde API 29.
            let mut cols: Vec<(&str, &str)> = vec![("id", "_id"), ("name", "_display_name")];
            if sdk_int >= 29 {
                cols.push(("folder", "relative_path"));
                cols.push(("size", "_size"));
            }
            let projection =
                env.new_object_array(cols.len() as i32, jni_str!("java/lang/String"), JObject::null())?;
            for (i, &(_, col)) in cols.iter().enumerate() {
                let c = env.new_string(col)?;
                projection.set_element(env, i, &c)?;
            }

            // selection mime_type=? con args ["application/pdf"].
            let selection = env.new_string("mime_type=?")?;
            let args = env.new_object_array(1, jni_str!("java/lang/String"), JObject::null())?;
            let mime = env.new_string("application/pdf")?;
            args.set_element(env, 0, &mime)?;

            // sortOrder: carpeta + nombre (case-insensitive) — la biblioteca
            // agrupa por carpeta como Evince.
            let sort = env.new_string("relative_path COLLATE NOCASE, _display_name COLLATE NOCASE")?;

            let cursor = env
                .call_method(
                    resolver.as_ref(),
                    jni_str!("query"),
                    jni_sig!(
                        sig = (android.net.Uri, [java.lang.String], java.lang.String, [java.lang.String], java.lang.String) -> android.database.Cursor
                    ),
                    &[
                        JValue::Object(files_uri.as_ref()),
                        JValue::Object(projection.as_ref()),
                        JValue::Object(selection.as_ref()),
                        JValue::Object(args.as_ref()),
                        JValue::Object(sort.as_ref()),
                    ],
                )?
                .l()?;
            if cursor.is_null() {
                return Ok(LibraryScan {
                    entries: Vec::new(),
                    permission_granted: true,
                    error: Some("MediaStore query returned a null cursor".to_string()),
                });
            }

            // Índices de columna (getColumnIndex puede devolver -1 si la
            // columna no existe en esta versión: se trata como dato ausente).
            let mut idx = Vec::with_capacity(cols.len());
            for (_, col) in &cols {
                let c = env.new_string(col)?;
                let i = env
                    .call_method(
                        cursor.as_ref(),
                        jni_str!("getColumnIndex"),
                        jni_sig!(sig = (java.lang.String) -> int),
                        &[JValue::Object(c.as_ref())],
                    )?
                    .i()?;
                env.delete_local_ref(c);
                idx.push(i);
            }

            let mut entries = Vec::new();
            loop {
                let more = env
                    .call_method(cursor.as_ref(), jni_str!("moveToNext"), jni_sig!(sig = () -> boolean), &[])?
                    .z()?;
                if !more {
                    break;
                }
                // Frame propio por fila: las refs locales (nombres, URI, clases)
                // se liberan al salir y no agotan la tabla de refs del hilo con
                // bibliotecas grandes (el límite por defecto son 512).
                // La anotación de tipo es necesaria: E0283 por la ambigüedad de
                // `E: From<Error>` en `with_local_frame` con el `?` interno.
                let entry: jni::errors::Result<Option<LibraryEntry>> =
                    env.with_local_frame(16, |env| {
                    let id = cursor_long(env, &cursor, idx[0])?;
                    let name = cursor_string(env, &cursor, idx[1])?;
                    let folder = if idx.len() > 2 {
                        cursor_string(env, &cursor, idx[2])?
                    } else {
                        String::new()
                    };
                    let size = if idx.len() > 3 { cursor_long(env, &cursor, idx[3])? } else { 0 };

                    // content URI: ContentUris.withAppendedId(files_uri, _ID).
                    let cu_class = env.find_class(jni_str!("android/content/ContentUris"))?;
                    let content_uri = env
                        .call_static_method(
                            &cu_class,
                            jni_str!("withAppendedId"),
                            jni_sig!(sig = (android.net.Uri, long) -> android.net.Uri),
                            &[JValue::Object(files_uri.as_ref()), JValue::Long(id)],
                        )?
                        .l()?;
                    // El content URI es un objeto Uri: su representación String
                    // se obtiene con toString() antes de pasarla a jni_jstring.
                    let uri_str_obj = env
                        .call_method(
                            content_uri.as_ref(),
                            jni_str!("toString"),
                            jni_sig!(sig = () -> java.lang.String),
                            &[],
                        )?
                        .l()?;
                    let uri_str = jni_jstring(env, &uri_str_obj)?;
                    let name = if name.is_empty() {
                        format!("pdf_{id}.pdf")
                    } else {
                        name
                    };
                    Ok(Some(LibraryEntry { name, folder, uri: uri_str, size }))
                });
                if let Some(entry) = entry? {
                    entries.push(entry);
                }
            }
            env.call_method(cursor.as_ref(), jni_str!("close"), jni_sig!(sig = () -> void), &[])?;
            Ok(LibraryScan {
                entries,
                permission_granted: true,
                error: None,
            })
        })
    });
    match res {
        Ok(scan) => scan,
        Err(e) => {
            // Excepción Java pendiente (p. ej. SecurityException del proveedor):
            // limpiarla para no envenenar llamadas JNI posteriores.
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("query_media_store: {e}");
            LibraryScan {
                entries: Vec::new(),
                permission_granted: false,
                error: Some(format!("MediaStore query failed: {e}")),
            }
        }
    }
}

/// `Cursor.getString(col)` → String (vacío si null o columna ausente).
fn cursor_string(env: &mut jni::Env, cursor: &JObject, idx: i32) -> jni::errors::Result<String> {
    if idx < 0 {
        return Ok(String::new());
    }
    let j = env
        .call_method(
            cursor.as_ref(),
            jni_str!("getString"),
            jni_sig!(sig = (int) -> java.lang.String),
            &[JValue::Int(idx)],
        )?
        .l()?;
    if j.is_null() {
        Ok(String::new())
    } else {
        jni_jstring(env, &j)
    }
}

/// `Cursor.getLong(col)` → i64 (0 si columna ausente).
fn cursor_long(env: &mut jni::Env, cursor: &JObject, idx: i32) -> jni::errors::Result<i64> {
    if idx < 0 {
        return Ok(0);
    }
    env.call_method(
        cursor.as_ref(),
        jni_str!("getLong"),
        jni_sig!(sig = (int) -> long),
        &[JValue::Int(idx)],
    )
    .map(|v| v.j().unwrap_or(0))
}

/// Lee un `java.io.InputStream` hasta EOF (búfer 64 KB) y lo cierra.
/// También lo usa `launch` para el "abrir con" de una content:// URI.
pub(super) fn drain_stream(env: &mut jni::Env, stream: &JObject) -> jni::errors::Result<Vec<u8>> {
    let mut bytes: Vec<u8> = Vec::new();
    const BUF_LEN: i32 = 65536;
    let buf = env.new_byte_array(BUF_LEN as usize)?;
    loop {
        let n = env
            .call_method(
                stream.as_ref(),
                jni_str!("read"),
                jni_sig!(sig = ([byte], int, int) -> int),
                &[
                    JValue::Object(buf.as_ref()),
                    JValue::Int(0),
                    JValue::Int(BUF_LEN),
                ],
            )?
            .i()?;
        if n < 0 {
            break; // EOF
        }
        let mut chunk = vec![0i8; n as usize];
        buf.get_region(env, 0, &mut chunk)?;
        bytes.extend(chunk.into_iter().map(|b| b as u8));
    }
    env.call_method(
        stream.as_ref(),
        jni_str!("close"),
        jni_sig!(sig = () -> void),
        &[],
    )?;
    Ok(bytes)
}

/// Abre una content:// URI con `ContentResolver.openInputStream` y devuelve
/// todos sus bytes (para copiar un PDF de la biblioteca a almacenamiento
/// interno). Mismo patrón JNI que el "abrir con" de `launch_intent_pdf`.
pub(crate) fn read_content_uri_bytes(app: &AndroidApp, uri_str: &str) -> Option<Vec<u8>> {
    let vm = JavaVM::singleton().ok()?;
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<Vec<u8>> = vm.attach_current_thread(|env| {
        env.with_local_frame(32, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app`.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let resolver = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getContentResolver"),
                    jni_sig!(sig = () -> android.content.ContentResolver),
                    &[],
                )?
                .l()?;
            let uri_class = env.find_class(jni_str!("android/net/Uri"))?;
            let jstr = env.new_string(uri_str)?;
            let uri = env
                .call_static_method(
                    &uri_class,
                    jni_str!("parse"),
                    jni_sig!(sig = (java.lang.String) -> android.net.Uri),
                    &[JValue::Object(jstr.as_ref())],
                )?
                .l()?;
            let stream = env
                .call_method(
                    resolver.as_ref(),
                    jni_str!("openInputStream"),
                    jni_sig!(sig = (android.net.Uri) -> java.io.InputStream),
                    &[JValue::Object(uri.as_ref())],
                )?
                .l()?;
            if stream.is_null() {
                error!("read_content_uri_bytes: openInputStream null para {uri_str}");
                return Ok(Vec::new());
            }
            drain_stream(env, &stream)
        })
    });
    match res {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        Ok(_) => None,
        Err(e) => {
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("read_content_uri_bytes {uri_str}: {e}");
            None
        }
    }
}

/// Abre la pantalla de Ajustes "Acceso a todos los archivos"
/// (Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, API 30+) para que
/// el usuario conceda MANAGE_EXTERNAL_STORAGE. Botón Grant de la biblioteca.
pub(crate) fn launch_all_files_settings(app: &AndroidApp) {
    let vm = match JavaVM::singleton() {
        Ok(v) => v,
        Err(e) => {
            error!("grant settings: JVM no disponible: {e}");
            return;
        }
    };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        env.with_local_frame(16, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app`.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
            let intent = env.new_object(&intent_class, jni_sig!(sig = () -> void), &[])?;
            let action =
                env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
            env.call_method(
                &intent,
                jni_str!("setAction"),
                jni_sig!(sig = (java.lang.String) -> android.content.Intent),
                &[JValue::Object(action.as_ref())],
            )?;
            let pkg = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getPackageName"),
                    jni_sig!(sig = () -> java.lang.String),
                    &[],
                )?
                .l()?;
            let pkg_str = jni_jstring(env, &pkg)?;
            let data = env.new_string(format!("package:{pkg_str}"))?;
            let uri_class = env.find_class(jni_str!("android/net/Uri"))?;
            let uri = env
                .call_static_method(
                    &uri_class,
                    jni_str!("parse"),
                    jni_sig!(sig = (java.lang.String) -> android.net.Uri),
                    &[JValue::Object(data.as_ref())],
                )?
                .l()?;
            env.call_method(
                &intent,
                jni_str!("setData"),
                jni_sig!(sig = (android.net.Uri) -> android.content.Intent),
                &[JValue::Object(uri.as_ref())],
            )?;
            env.call_method(
                activity.as_ref(),
                jni_str!("startActivity"),
                jni_sig!(sig = (android.content.Intent) -> void),
                &[JValue::Object(intent.as_ref())],
            )?;
            Ok(())
        })
    });
    match res {
        Ok(()) => info!("All files access settings lanzada"),
        Err(e) => {
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("launch_all_files_settings: {e}");
        }
    }
}
