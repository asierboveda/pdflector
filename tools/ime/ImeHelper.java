// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda
//
// ImeHelper — teclado del buscador de PDFLector (NativeActivity).
//
// Por qué existe: el backend native-activity de android-activity 0.6 no
// entrega texto del teclado por la vía nativa (set_text_input_state es NOP y
// TextEvent solo llega desde game-activity, que exige una Activity Java
// compilada; cargo-apk no compila Java). Este helper es un edit invisible
// (1x1 px) dentro del decor de la Activity: pide el foco y muestra el IME
// real del sistema con InputMethodManager. Desde Rust se hace POLLING del
// texto (ver jni::ime_*), sin bloquear el hilo nativo.
//
// Build (host; tools del SDK Android ya presentes en este repo):
//   tools/ime/build.sh
// Produce tools/ime/classes.dex, que jni.rs embebe con include_bytes! y
// carga en runtime con DexClassLoader desde files/ime/.
package com.pdflector.app;

import android.app.Activity;
import android.content.Context;
import android.graphics.Color;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.ViewGroup;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.FrameLayout;

public final class ImeHelper {
    private static EditText sEdit;
    private static boolean sAttached;
    // Copia del texto escrita por el TextWatcher en el hilo UI; volatile
    // para que el hilo nativo (polling) la lea sin condiciones de carrera
    // visibles. El hilo de lectura nunca muta el EditText.
    private static volatile String sText = "";

    private ImeHelper() {}

    /** Crea (una vez) el EditText invisible, lo enfoca y abre el teclado. */
    public static void attach(final Activity activity, final String initial) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (sEdit == null) {
                    sEdit = new EditText(activity);
                    sEdit.setSingleLine(true);
                    sEdit.setInputType(InputType.TYPE_CLASS_TEXT
                            | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
                    sEdit.setBackgroundColor(Color.TRANSPARENT);
                    sEdit.setTextColor(Color.TRANSPARENT);
                    sEdit.setHintTextColor(Color.TRANSPARENT);
                    sEdit.setCursorVisible(false);
                    FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(1, 1);
                    lp.gravity = Gravity.LEFT | Gravity.TOP;
                    ((ViewGroup) activity.getWindow().getDecorView()).addView(sEdit, lp);
                    sEdit.addTextChangedListener(new TextWatcher() {
                        @Override
                        public void beforeTextChanged(CharSequence s, int a, int b, int c) {}

                        @Override
                        public void onTextChanged(CharSequence s, int a, int b, int c) {
                            sText = s.toString();
                        }

                        @Override
                        public void afterTextChanged(Editable s) {}
                    });
                }
                sEdit.setText(initial == null ? "" : initial);
                sEdit.setSelection(sEdit.length());
                sEdit.requestFocus();
                sAttached = true;
                InputMethodManager imm = (InputMethodManager) activity
                        .getSystemService(Context.INPUT_METHOD_SERVICE);
                imm.showSoftInput(sEdit, InputMethodManager.SHOW_IMPLICIT);
            }
        });
    }

    /** Re-muestra el teclado si el edit ya existe (p. ej. re-foco). */
    public static void show(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (sEdit == null || !sAttached) {
                    return;
                }
                sEdit.requestFocus();
                InputMethodManager imm = (InputMethodManager) activity
                        .getSystemService(Context.INPUT_METHOD_SERVICE);
                imm.showSoftInput(sEdit, InputMethodManager.SHOW_IMPLICIT);
            }
        });
    }

    /** Oculta el teclado y suelta el foco del edit invisible. */
    public static void hide(final Activity activity) {
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                if (sEdit == null || !sAttached) {
                    return;
                }
                InputMethodManager imm = (InputMethodManager) activity
                        .getSystemService(Context.INPUT_METHOD_SERVICE);
                imm.hideSoftInputFromWindow(sEdit.getWindowToken(), 0);
                sEdit.clearFocus();
            }
        });
    }

    public static boolean isAttached() {
        return sAttached;
    }

    /** Texto ACTUAL del campo (copia volatile escrita por el TextWatcher). */
    public static String getText() {
        return sText;
    }

    /** Sustituye el texto (p. ej. al limpiar con la "✕"). */
    public static void setText(final String t) {
        sText = t == null ? "" : t;
        if (sEdit != null && sAttached) {
            sEdit.post(new Runnable() {
                @Override
                public void run() {
                    sEdit.setText(sText);
                }
            });
        }
    }
}