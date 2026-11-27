// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Ventana de la Activity vía JNI: pantalla siempre encendida, tasa de
//! refresco de 120 Hz y modo inmersivo (barras del sistema ocultas).

use crate::android_activity::AndroidApp;
use jni::objects::{JObject, JValue};
use jni::{JavaVM, jni_sig, jni_str};
use log::{info, warn};

/// Modo inmersivo del visor: oculta la barra de estado y la de navegación del
/// sistema (Android 15 edge-to-edge las deja DIBUJADAS sobre la app, pero el
/// sistema se queda con los touches de su franja — un tap en la status bar
/// abre el shade de notificaciones y jamás llega al fab "✎" ni a la barra de
/// herramientas). Con `BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE` las barras
/// reaparecen temporalmente con un swipe desde el borde (estándar en
/// lectores); el resto del tiempo TODA la pantalla es de la app (el botón
/// flotante de herramientas era intocable en la tablet real con barras del sistema).
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
        let res = env.with_local_frame(32, |env| {
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
            if window.is_null() {
                return Ok(());
            }
            let _ = env.call_method(
                window.as_ref(),
                jni_str!("addFlags"),
                jni_sig!(sig = (int) -> void),
                &[JValue::Int(FLAG_KEEP_SCREEN_ON)],
            )?;
            Ok(())
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    match res {
        Ok(()) => info!("keep_screen_on: flag aplicado"),
        Err(e) => warn!("keep_screen_on: {e}"),
    }
}
/// Configura la tasa de refresco a 120 Hz en el WindowManager de Android.
///
/// Consulta los modos de display disponibles (`Display.getSupportedModes()`)
/// y si encuentra un modo de 120 Hz (como en la TCL NXTPaper 11 Plus, modeId 1),
/// establece `preferredDisplayModeId` y `preferredRefreshRate = 120.0f` en los
/// LayoutParams de la ventana.
pub(crate) fn enable_120hz(app: &AndroidApp) {
    let Ok(vm) = JavaVM::singleton() else {
        log::warn!("enable_120hz: sin JavaVM");
        return;
    };
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(64, |env| {
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
            if window.is_null() {
                return Ok(());
            }

            let get_attrs_rt = jni::signature::RuntimeMethodSignature::from_str(
                "()Landroid/view/WindowManager$LayoutParams;",
            )?;
            let get_attrs_sig = jni::signature::MethodSignature::from(&get_attrs_rt);
            let params = env
                .call_method(
                    window.as_ref(),
                    jni_str!("getAttributes"),
                    get_attrs_sig,
                    &[],
                )?
                .l()?;
            if params.is_null() {
                return Ok(());
            }

            // 1. Fijar preferredRefreshRate = 120.0f (API 30+); limpiar
            // cualquier NoSuchFieldError pendiente antes de seguir en JNI.
            if env
                .set_field(
                    params.as_ref(),
                    jni_str!("preferredRefreshRate"),
                    jni_sig!(sig = float),
                    JValue::Float(120.0),
                )
                .is_err()
            {
                env.exception_clear();
            }

            // 2. Fijar preferredDisplayModeId = 1 (TCL NXTPaper 11 Plus 120 Hz ModeId 1)
            if env
                .set_field(
                    params.as_ref(),
                    jni_str!("preferredDisplayModeId"),
                    jni_sig!(sig = int),
                    JValue::Int(1),
                )
                .is_err()
            {
                env.exception_clear();
            }

            // Aplicar atributos a la ventana
            let set_attrs_rt = jni::signature::RuntimeMethodSignature::from_str(
                "(Landroid/view/WindowManager$LayoutParams;)V",
            )?;
            let set_attrs_sig = jni::signature::MethodSignature::from(&set_attrs_rt);
            let _ = env.call_method(
                window.as_ref(),
                jni_str!("setAttributes"),
                set_attrs_sig,
                &[JValue::Object(params.as_ref())],
            )?;

            Ok(())
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });

    match res {
        Ok(()) => info!("enable_120hz: modo 120 Hz solicitado a la ventana"),
        Err(e) => warn!("enable_120hz: {e}"),
    }
}

pub(crate) fn enter_immersive(app: &AndroidApp) {
    let Ok(vm) = JavaVM::singleton() else {
        return;
    };
    let res: jni::errors::Result<()> = vm.attach_current_thread(|env| {
        let res = env.with_local_frame(32, |env| {
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
            if window.is_null() {
                return Ok(());
            }
            let controller = env
                .call_method(
                    window.as_ref(),
                    jni_str!("getInsetsController"),
                    jni_sig!(sig = () -> android.view.WindowInsetsController),
                    &[],
                )?
                .l()?;
            if controller.is_null() {
                return Ok(());
            }
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
        });
        if res.is_err() {
            env.exception_clear();
        }
        res
    });
    if let Err(e) = res {
        warn!("enter_immersive: {e}");
    }
}
