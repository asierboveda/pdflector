// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Teclado del sistema (IME) a través del helper Java embebido
//! (`tools/ime/ImeHelper.java`).

use std::fs;
use std::sync::Mutex;

use crate::android_activity::AndroidApp;
use jni::objects::{Global, JClass, JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use log::error;

use super::jni_jstring;

// ---------------------------------------------------------------------------
// Buscador con teclado (IME) — ver tools/ime/ImeHelper.java
// ---------------------------------------------------------------------------
//
// Por qué no hay Teclado por la vía nativa: el backend `native-activity` de
// android-activity 0.6 deja `set_text_input_state` en NOP y `InputEvent::
// TextEvent` solo lo produce `game-activity` (que exige una Activity Java
// compilada; cargo-apk no compila Java). La vía elegida es un helper Java
// MÍNIMO compilado a dex con las tools del SDK (`tools/ime/build.sh`, javac
// + d8 + android.jar), EMBEBIDO aquí con `include_bytes!` y cargado en
// runtime con `DexClassLoader` desde `files/ime/`. El helper crea un
// `EditText` invisible en el hilo UI (runOnUiThread con Runnables compilados)
// y abre el IME real; Rust hace POLLING del texto (`ime_text`) sin bloquear
// el hilo nativo.

/// Dex del helper IME embebido en el binario (ver `tools/ime/build.sh`).
/// Se extrae a `files/ime/classes.dex` la primera vez que se usa.
const IME_DEX: &[u8] = include_bytes!("../../../../tools/ime/classes.dex");

/// Clase `com.pdflector.app.ImeHelper` cargada (GlobalRef cacheado; la
/// clase vive mientras la JVM viva — hasta `onDestroy`).
static IME_LOADED: Mutex<Option<Global<JObject<'static>>>> = Mutex::new(None);

/// Carga (o devuelve la cacheada) la clase del helper IME: extrae el dex
/// embebido a `files/ime/classes.dex` si falta y lo carga con
/// `DexClassLoader` usando el class loader de la activity.
fn ime_class_raw(app: &AndroidApp) -> Option<jni::sys::jobject> {
    {
        let Ok(guard) = IME_LOADED.lock() else {
            return None;
        };
        if let Some(c) = guard.as_ref() {
            return Some(c.as_raw());
        }
    }
    let base = app.internal_data_path()?;
    let dir = base.join("ime");
    fs::create_dir_all(&dir).ok()?;
    let dex_path = dir.join("classes.dex");
    if !dex_path.is_file() {
        fs::write(&dex_path, IME_DEX).ok()?;
    }
    // ART rechaza dexes "writable" ("Writable dex file ... is not allowed"):
    // marcar SOLO-LECTURA (0444) para que DexClassLoader lo acepte.
    if let Ok(meta) = fs::metadata(&dex_path)
        && !meta.permissions().readonly()
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&dex_path, fs::Permissions::from_mode(0o444)).ok()?;
    }
    let vm = JavaVM::singleton().ok()?;
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<Global<JObject<'static>>> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(32, |env| {
            // SAFETY: ref global no owned, válida mientras viva `app`.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let cls_loader = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getClassLoader"),
                    jni_sig!(sig = () -> java.lang.ClassLoader),
                    &[],
                )?
                .l()?;
            let dex_s = env.new_string(dex_path.display().to_string())?;
            let opt_s = env.new_string(dir.join("opt").display().to_string())?;
            let dcl_class = env.find_class(jni_str!("dalvik/system/DexClassLoader"))?;
            let ctor = env.get_method_id(
                &dcl_class,
                jni_str!("<init>"),
                jni_sig!(
                    sig = (java.lang.String, java.lang.String, java.lang.String,
                           java.lang.ClassLoader) -> void
                ),
            )?;
            let ctor_args = [
                JValue::Object(dex_s.as_ref()).as_jni(),
                JValue::Object(opt_s.as_ref()).as_jni(),
                JValue::Object(JObject::null().as_ref()).as_jni(),
                JValue::Object(cls_loader.as_ref()).as_jni(),
            ];
            // SAFETY: ctor verificado con get_method_id y argumentos en el
            // orden/tipo exactos de la firma.
            let dcl = unsafe { env.new_object_unchecked(&dcl_class, ctor, &ctor_args)? };
            let name = env.new_string("com.pdflector.app.ImeHelper")?;
            let class = env
                .call_method(
                    dcl.as_ref(),
                    jni_str!("loadClass"),
                    jni_sig!(sig = (java.lang.String) -> java.lang.Class),
                    &[JValue::Object(name.as_ref())],
                )?
                .l()?;
            env.new_global_ref(class)
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    let class = match res {
        Ok(c) => c,
        Err(e) => {
            error!("ime: load ImeHelper: {e}");
            return None;
        }
    };
    if let Ok(mut g) = IME_LOADED.lock() {
        *g = Some(class);
    }
    IME_LOADED
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.as_raw()))
}

/// Abre el teclado del buscador con `initial` como texto inicial (el helper
/// crea el EditText invisible e idempotente). Pensado para llamarse al tocar
/// el campo "Buscar..." de la biblioteca.
pub(crate) fn ime_attach(app: &AndroidApp, initial: &str) {
    let Some(raw) = ime_class_raw(app) else {
        return;
    };
    let Some(vm) = JavaVM::singleton().ok() else {
        return;
    };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let initial = initial.to_string();
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(16, |env| {
            // SAFETY: ref global no owned (GlobalRef cacheado en IME_LOADED), válida mientras la JVM viva.
            let class = unsafe { env.as_cast_raw::<JClass>(&raw)? };
            // SAFETY: ref global no owned, válida mientras viva `app`.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            let jinit = env.new_string(&initial)?;
            env.call_static_method(
                class.as_ref(),
                jni_str!("attach"),
                jni_sig!(sig = (android.app.Activity, java.lang.String) -> void),
                &[
                    JValue::Object(activity.as_ref()),
                    JValue::Object(jinit.as_ref()),
                ],
            )?;
            Ok(())
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    if let Err(e) = res {
        error!("ime_attach: {e}");
    }
}

/// Oculta el teclado y suelta el foco del campo invisible.
pub(crate) fn ime_hide(app: &AndroidApp) {
    let Some(raw) = ime_class_raw(app) else {
        return;
    };
    let Some(vm) = JavaVM::singleton().ok() else {
        return;
    };
    let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(8, |env| {
            // SAFETY: ref global no owned (GlobalRef cacheado en IME_LOADED), válida mientras la JVM viva.
            let class = unsafe { env.as_cast_raw::<JClass>(&raw)? };
            // SAFETY: ref global no owned, válida mientras viva `app`.
            let activity = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
            env.call_static_method(
                class.as_ref(),
                jni_str!("hide"),
                jni_sig!(sig = (android.app.Activity) -> void),
                &[JValue::Object(activity.as_ref())],
            )?;
            Ok(())
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    if let Err(e) = res {
        error!("ime_hide: {e}");
    }
}

/// Texto ACTUAL del campo del buscador (lo que ha tecleado el usuario; el
/// helper lo publica como String volatile, seguro de leer desde este hilo).
pub(crate) fn ime_text(app: &AndroidApp) -> Option<String> {
    let raw = ime_class_raw(app)?;
    let vm = JavaVM::singleton().ok()?;
    let res: jni::errors::Result<String> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(8, |env| {
            // SAFETY: ref global no owned (GlobalRef cacheado en IME_LOADED), válida mientras la JVM viva.
            let class = unsafe { env.as_cast_raw::<JClass>(&raw)? };
            let obj = env
                .call_static_method(
                    class.as_ref(),
                    jni_str!("getText"),
                    jni_sig!(sig = () -> java.lang.String),
                    &[],
                )?
                .l()?;
            jni_jstring(env, &obj)
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    match res {
        Ok(t) => Some(t),
        Err(e) => {
            error!("ime_text: {e}");
            None
        }
    }
}

/// Sustituye el texto del campo (p. ej. al limpiar con la "✕").
pub(crate) fn ime_set_text(app: &AndroidApp, text: &str) {
    let Some(raw) = ime_class_raw(app) else {
        return;
    };
    let Some(vm) = JavaVM::singleton().ok() else {
        return;
    };
    let text = text.to_string();
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(8, |env| {
            // SAFETY: ref global no owned (GlobalRef cacheado en IME_LOADED), válida mientras la JVM viva.
            let class = unsafe { env.as_cast_raw::<JClass>(&raw)? };
            let jt = env.new_string(&text)?;
            env.call_static_method(
                class.as_ref(),
                jni_str!("setText"),
                jni_sig!(sig = (java.lang.String) -> void),
                &[JValue::Object(jt.as_ref())],
            )?;
            Ok(())
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    if let Err(e) = res {
        error!("ime_set_text: {e}");
    }
}
