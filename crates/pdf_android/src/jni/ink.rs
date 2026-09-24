// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Puente de bajo coste a la capa wet `GLFrontBufferedRenderer` de la Activity.
//! La Activity queda retenida mediante una referencia global JNI y los
//! identificadores de método se resuelven una sola vez al crear el puente.

use std::sync::atomic::{AtomicBool, Ordering};

use android_activity::AndroidApp;
use jni::ids::JMethodID;
use jni::objects::{Global, JObject, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::{JavaVM, jni_sig, jni_str};

/// Acceso a la superficie wet propiedad de `PdfLectorActivity`.
///
/// Todos los métodos se invocan desde el bucle nativo principal. El `Global`
/// mantiene viva la Activity mientras exista este puente; se libera al
/// destruir el Reader, en un hilo ya asociado a la VM.
pub(crate) struct InkOverlay {
    vm: JavaVM,
    activity: Global<JObject<'static>>,
    render_segment: JMethodID,
    commit: JMethodID,
    cancel: JMethodID,
    clear: JMethodID,
    is_ready: JMethodID,
    failed: AtomicBool,
}

impl InkOverlay {
    /// Construye el puente y verifica de una vez que el host expone el contrato.
    pub(crate) fn new(app: &AndroidApp) -> Option<Self> {
        let vm = JavaVM::singleton().ok()?;
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        let activity = vm.attach_current_thread(|env| {
            env.with_local_frame(16, |env| {
                // SAFETY: android-activity expone aquí una referencia global
                // no poseída, válida mientras viva AndroidApp.
                let borrowed = unsafe { env.as_cast_raw::<JObject>(&raw_activity)? };
                let activity = env.new_global_ref(borrowed.as_ref())?;
                let activity_class = env.get_object_class(borrowed.as_ref())?;
                let render_segment = env.get_method_id(
                    &activity_class,
                    jni_str!("renderInkSegment"),
                    jni_sig!(sig = (float, float, float, float, float, int) -> boolean),
                )?;
                let commit = env.get_method_id(
                    &activity_class,
                    jni_str!("commitInk"),
                    jni_sig!(sig = () -> void),
                )?;
                let cancel = env.get_method_id(
                    &activity_class,
                    jni_str!("cancelInk"),
                    jni_sig!(sig = () -> void),
                )?;
                let clear = env.get_method_id(
                    &activity_class,
                    jni_str!("clearInk"),
                    jni_sig!(sig = () -> void),
                )?;
                let is_ready = env.get_method_id(
                    &activity_class,
                    jni_str!("isInkOverlayReady"),
                    jni_sig!(sig = () -> boolean),
                )?;
                Ok::<_, jni::errors::Error>((
                    activity,
                    render_segment,
                    commit,
                    cancel,
                    clear,
                    is_ready,
                ))
            })
        });
        let activity = match activity {
            Ok(activity) => activity,
            Err(_) => {
                // Method lookup can leave NoSuchMethodError pending. Clear it
                // before the native event loop continues without the overlay.
                let _ = vm.attach_current_thread(|env| {
                    env.exception_clear();
                    Ok::<_, jni::errors::Error>(())
                });
                log::warn!("ink overlay JNI methods unavailable; using Rust wet fallback");
                return None;
            }
        };

        Some(Self {
            vm,
            activity: activity.0,
            render_segment: activity.1,
            commit: activity.2,
            cancel: activity.3,
            clear: activity.4,
            is_ready: activity.5,
            failed: AtomicBool::new(false),
        })
    }

    /// Envía un segmento causal en píxeles de superficie.
    pub(crate) fn render_segment(
        &self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width_px: f32,
        color_argb: i32,
    ) -> bool {
        if self.failed.load(Ordering::Relaxed) {
            return false;
        }
        let result = self.vm.attach_current_thread(|env| {
            // SAFETY: the method ID and argument signature are checked in `new`.
            let result = unsafe {
                env.call_method_unchecked(
                    self.activity.as_ref(),
                    self.render_segment,
                    ReturnType::Primitive(Primitive::Boolean),
                    &[
                        JValue::Float(x0).as_jni(),
                        JValue::Float(y0).as_jni(),
                        JValue::Float(x1).as_jni(),
                        JValue::Float(y1).as_jni(),
                        JValue::Float(width_px).as_jni(),
                        JValue::Int(color_argb).as_jni(),
                    ],
                )
            }
            .and_then(|value| value.z());
            if result.is_err() {
                env.exception_clear();
            }
            result
        });
        match result {
            Ok(accepted) => accepted,
            Err(_) => {
                self.mark_failed();
                false
            }
        }
    }

    pub(crate) fn commit(&self) {
        self.call_void(self.commit, &[]);
    }

    pub(crate) fn cancel(&self) {
        self.call_void(self.cancel, &[]);
    }

    pub(crate) fn clear(&self) {
        self.call_void(self.clear, &[]);
    }

    pub(crate) fn is_ready(&self) -> bool {
        if self.failed.load(Ordering::Relaxed) {
            return false;
        }
        let result = self.vm.attach_current_thread(|env| {
            // SAFETY: method ID se obtuvo del objeto retenido en `new`, y su
            // firma se comprobó allí como `()Z`.
            let result = unsafe {
                env.call_method_unchecked(
                    self.activity.as_ref(),
                    self.is_ready,
                    ReturnType::Primitive(Primitive::Boolean),
                    &[],
                )
            }
            .and_then(|value| value.z());
            if result.is_err() {
                env.exception_clear();
            }
            result
        });
        match result {
            Ok(ready) => ready,
            Err(_) => {
                self.mark_failed();
                false
            }
        }
    }

    fn call_void(&self, method: JMethodID, args: &[jni::sys::jvalue]) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let result = self.vm.attach_current_thread(|env| {
            // SAFETY: cada ID se obtuvo en `new`; los argumentos y la firma
            // coinciden con el método que selecciona el campo `method`.
            let result = unsafe {
                env.call_method_unchecked(
                    self.activity.as_ref(),
                    method,
                    ReturnType::Primitive(Primitive::Void),
                    args,
                )
            }
            .map(|_| ());
            if result.is_err() {
                env.exception_clear();
            }
            result
        });
        if result.is_err() {
            self.mark_failed();
        }
    }

    fn mark_failed(&self) {
        if !self.failed.swap(true, Ordering::Relaxed) {
            log::warn!("ink overlay JNI bridge unavailable; using Rust wet fallback");
        }
    }
}
