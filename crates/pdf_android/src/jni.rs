// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Toda la interacción Java vía JNI: Intent de lanzamiento ("abrir con"),
//! MediaStore (biblioteca de PDFs del sistema), ContentResolver (copiar
//! bytes de una content:// URI) y Ajustes del permiso "All files access".
//!
//! Módulo resultante de la partición de `lib.rs` (2026-08-13): `reader` y
//! `input` llaman a estas funciones; los tipos de datos que construyen
//! (`LaunchPdf`, `LibraryEntry`, `LibraryScan`) viven en `reader`.

use std::fs;
use std::path::Path;

use android_activity::AndroidApp;
use jni::objects::{JObject, JString, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use log::{error, info, warn};

use crate::reader::{LaunchPdf, LibraryEntry, LibraryScan};

/// Modo inmersivo del visor: oculta la barra de estado y la de navegación del
/// sistema (Android 15 edge-to-edge las deja DIBUJADAS sobre la app, pero el
/// sistema se queda con los touches de su franja — un tap en la status bar
/// abre el shade de notificaciones y jamás llega al fab "✎" ni a la barra de
/// herramientas). Con `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE` las barras
/// reaparecen temporalmente con un swipe desde el borde (estándar en
/// lectores); el resto del tiempo TODA la pantalla es de la app (fix
/// 2026-08-23: el fab de la Fase 3.5 era intocable en la tablet real).
///
/// Deprecado en API 35 pero funcional: `WindowInsetsController.hide` (API
/// 30+). En API < 30 falla y se degrada silenciosamente (el resto de la app
/// sigue funcionando; solo muere la franja táctil superior, que ya era
/// borderline). No loguea errores graves: best-effort en el arranque.
/// Mantiene la pantalla ENCENDIDA mientras la app está en primer plano
/// (`WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON`): sin este flag el
/// timeout del sistema apagaba el display a mitad de escritura con el lápiz
/// (el stylus no cuenta como "user activity" para el suspend en todas las
/// ROMs). Best-effort en el arranque, igual que `enter_immersive`.
pub(crate) fn keep_screen_on(app: &AndroidApp) {
    const FLAG_KEEP_SCREEN_ON: i32 = 0x80; // WindowManager.LayoutParams
    let Ok(vm) = JavaVM::singleton() else {
        log::warn!("keep_screen_on: sin JavaVM");
        return;
    };
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        env.with_local_frame(32, |env| {
            let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let window = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getWindow"),
                    jni_sig!(sig = () -> android.view.Window),
                    &[],
                )?
                .l()?;
            let _ = env.call_method(
                window.as_ref(),
                jni_str!("addFlags"),
                jni_sig!(sig = (int) -> void),
                &[JValue::Int(FLAG_KEEP_SCREEN_ON)],
            )?;
            Ok(())
        })
    });
    match res {
        Ok(()) => info!("keep_screen_on: flag aplicado"),
        Err(e) => warn!("keep_screen_on: {e}"),
    }
}

pub(crate) fn enter_immersive(app: &AndroidApp) {
    let Ok(vm) = JavaVM::singleton() else {
        return;
    };
    let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        env.with_local_frame(32, |env| {
            let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let window = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getWindow"),
                    jni_sig!(sig = () -> android.view.Window),
                    &[],
                )?
                .l()?;
            let controller = env
                .call_method(
                    window.as_ref(),
                    jni_str!("getInsetsController"),
                    jni_sig!(sig = () -> android.view.WindowInsetsController),
                    &[],
                )?
                .l()?;
            let _ = env.call_method(
                controller.as_ref(),
                jni_str!("hide"),
                jni_sig!(sig = (int) -> void),
                &[JValue::Int(1 | 2)],
            )?;
            let _ = env.call_method(
                controller.as_ref(),
                jni_str!("setSystemBarsBehavior"),
                jni_sig!(sig = (int) -> void),
                &[JValue::Int(1)],
            )?;
            Ok(())
        })
    });
}

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
/// - Sin `data` (lanzamiento normal desde el launcher) → `None`: el picker
///   interno sigue siendo el fallback.
///
/// JNI: `AndroidApp::activity_as_ptr()` expone la `Activity` (android-activity
/// 0.6, global ref unowned) y `JavaVM::singleton()` el VM; se llama a
/// `Activity.getIntent().getData()` y, según el scheme, al ContentResolver.
pub(crate) fn launch_intent_pdf(app: &AndroidApp) -> Option<LaunchPdf> {
    let vm = JavaVM::singleton().ok()?;
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let internal_dir = app.internal_data_path();

    let res: jni::errors::Result<Option<LaunchPdf>> = vm.attach_current_thread(|env| {
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
                    Ok(Some(LaunchPdf {
                        name,
                        source: uri_str,
                        path: dest.display().to_string(),
                    }))
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
                    Ok(Some(LaunchPdf {
                        name,
                        source: uri_str,
                        path,
                    }))
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
            error!("launch_intent_pdf: {e}");
            None
        }
    }
}

/// Convierte un `JObject` que es un `java.lang.String` a `String` Rust (copia).
/// `new_cast_local_ref` verifica el tipo (IsInstanceOf) y crea una ref local.
fn jni_jstring(env: &mut jni::Env, obj: &JObject) -> jni::errors::Result<String> {
    let jstr = env.new_cast_local_ref::<JString>(obj)?;
    jstr.try_to_string(env)
}

/// Sanea un nombre de fichero recibido del sistema (DISPLAY_NAME o segmento de
/// URI) para usarlo como nombre local: solo caracteres seguros, extensión .pdf
/// garantizada y longitud acotada.
pub(crate) fn sanitize_pdf_name(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        return "external.pdf".to_string();
    }
    let mut s = s;
    if !s.to_ascii_lowercase().ends_with(".pdf") {
        s.push_str(".pdf");
    }
    if s.chars().count() > 80 {
        s = format!("{}.pdf", s.chars().take(76).collect::<String>());
    }
    s
}

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
fn drain_stream(env: &mut jni::Env, stream: &JObject) -> jni::errors::Result<Vec<u8>> {
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

/// Un file descriptor nativo abierto sobre una content:// URI.
///
/// `open_content_fd` usa `ContentResolver.openFileDescriptor(uri, "r")` y
/// devuelve el `ParcelFileDescriptor` como `Global` (JNI) junto con su fd
/// nativo. El `Global` mantiene vivo el PFD (y por tanto el fd) mientras se
/// renderiza la portada; `close` cierra el PFD vía JNI y suelta la referencia.
///
/// # Por qué un fd y no los bytes (portadas de la biblioteca)
///
/// `read_content_uri_bytes` copia el PDF COMPLETO a memoria, inaceptable para
/// renderizar una portada (un PDF de 100 MB se leería entero por un thumbnail
/// de 200 px). Con el fd, MuPDF abre `/proc/self/fd/{fd}` (ruta que resuelve
/// al fichero real de MediaProvider) y solo lee los objetos que necesita para
/// la página 1 — sin copia, sin pico de memoria. Para fds que no respalden un
/// fichero real (proveedores que materializan en un pipe/tmp), el open de
/// MuPDF falla y la celda se queda con su placeholder (comportamiento
/// degradado aceptado y documentado).
pub(crate) struct ContentFd {
    /// fd nativo (valido mientras viva `pfd`).
    pub(crate) fd: i32,
    /// ParcelFileDescriptor como ref global JNI (mantiene el fd abierto).
    pfd: jni::objects::Global<jni::objects::JObject<'static>>,
}

impl ContentFd {
    /// Ruta `/proc/self/fd/{fd}` con la que MuPDF abre el fichero sin copiarlo.
    pub(crate) fn proc_path(&self) -> String {
        format!("/proc/self/fd/{}", self.fd)
    }

    /// Cierra el PFD (libera el fd nativo) y suelta la ref global.
    pub(crate) fn close(self) {
        let vm = match JavaVM::singleton() {
            Ok(v) => v,
            Err(e) => {
                error!("ContentFd::close: JVM no disponible: {e}");
                return;
            }
        };
        let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
            let obj = self.pfd.as_obj();
            let _ = env.call_method(obj, jni_str!("close"), jni_sig!(sig = () -> void), &[]);
            Ok(())
        });
        // `self.pfd` (Global) se dropea aquí: borra la ref global JNI.
    }
}

/// Copia `text` al portapapeles del sistema con el contexto de la Activity:
/// `ClipboardManager.setPrimaryClip(ClipData.newPlainText("text", text))`.
/// Lo llama el botón "Copiar" del menú de selección (`Reader::copy_sel`); un
/// fallo solo se loguea (el texto no se pierde: el usuario puede volver a
/// seleccionar).
///
/// JNI: `getSystemService(Context.CLIPBOARD_SERVICE)` devuelve un
/// `java.lang.Object` cuyo tipo real es `android.content.ClipboardManager`;
/// JNI resuelve el método por nombre+firma sobre el objeto real, así que no
/// hace falta un cast explícito. `ClipData.newPlainText` es estático.
pub(crate) fn copy_to_clipboard(app: &AndroidApp, text: &str) {
    let vm = match JavaVM::singleton() {
        Ok(v) => v,
        Err(e) => {
            error!("copy_to_clipboard: JVM no disponible: {e}");
            return;
        }
    };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        env.with_local_frame(16, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app` (mismo
            // patrón que `launch_intent_pdf`).
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let svc = env.new_string("clipboard")?;
            let cm = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getSystemService"),
                    jni_sig!(sig = (java.lang.String) -> java.lang.Object),
                    &[JValue::Object(svc.as_ref())],
                )?
                .l()?;
            if cm.is_null() {
                error!("copy_to_clipboard: getSystemService(clipboard) null");
                return Ok(());
            }
            let label = env.new_string("text")?;
            let jtext = env.new_string(text)?;
            let cd_class = env.find_class(jni_str!("android/content/ClipData"))?;
            let clip = env
                .call_static_method(
                    &cd_class,
                    jni_str!("newPlainText"),
                    jni_sig!(
                        sig = (java.lang.String, java.lang.String) -> android.content.ClipData
                    ),
                    &[
                        JValue::Object(label.as_ref()),
                        JValue::Object(jtext.as_ref()),
                    ],
                )?
                .l()?;
            env.call_method(
                cm.as_ref(),
                jni_str!("setPrimaryClip"),
                jni_sig!(sig = (android.content.ClipData) -> void),
                &[JValue::Object(clip.as_ref())],
            )?;
            Ok(())
        })
    });
    match res {
        Ok(()) => info!("clipboard: {} chars", text.chars().count()),
        Err(e) => {
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("copy_to_clipboard: {e}");
        }
    }
}

/// Abre una content:// URI con `ContentResolver.openFileDescriptor(uri, "r")`
/// y devuelve el fd nativo + el PFD como `Global`. `None` si falla (URI
/// inválida, permiso, proveedor sin fd). El caller debe llamar a `close`.
pub(crate) fn open_content_fd(app: &AndroidApp, uri_str: &str) -> Option<ContentFd> {
    let vm = JavaVM::singleton().ok()?;
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<Option<ContentFd>> = vm.attach_current_thread(|env| {
        env.with_local_frame(16, |env| {
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
            let mode = env.new_string("r")?;
            let pfd = env
                .call_method(
                    resolver.as_ref(),
                    jni_str!("openFileDescriptor"),
                    jni_sig!(
                        sig = (android.net.Uri, java.lang.String) -> android.os.ParcelFileDescriptor
                    ),
                    &[JValue::Object(uri.as_ref()), JValue::Object(mode.as_ref())],
                )?
                .l()?;
            if pfd.is_null() {
                return Ok(None);
            }
            let fd = env
                .call_method(
                    pfd.as_ref(),
                    jni_str!("getFd"),
                    jni_sig!(sig = () -> int),
                    &[],
                )?
                .i()?;
            let global = env.new_global_ref(pfd)?;
            if fd < 0 {
                return Ok(None);
            }
            Ok(Some(ContentFd { fd, pfd: global }))
        })
    });
    match res {
        Ok(Some(cfd)) => Some(cfd),
        Ok(None) => {
            error!("open_content_fd: sin fd para {uri_str}");
            None
        }
        Err(e) => {
            let _: jni::errors::Result<()> = vm.attach_current_thread(|env| {
                env.exception_clear();
                Ok(())
            });
            error!("open_content_fd {uri_str}: {e}");
            None
        }
    }
}
