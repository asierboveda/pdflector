// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Sustituto de host (Linux) de la crate `android-activity`.
//!
//! `android-activity`/`ndk` solo compilan para Android. Este módulo replica
//! los nombres que usa la app para que `cargo test -p pdf_android --lib`
//! compile en Linux. Nada de esto se ejecuta: los tests no crean
//! `AndroidApp` ni ventanas, y la app real usa la crate de verdad.

use std::path::PathBuf;

/// Rectángulo de contenido de la Activity (solo se lee `top`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Configuración del dispositivo (solo se lee la densidad).
pub struct ConfigurationRef;

impl ConfigurationRef {
    pub fn density(&self) -> Option<u32> {
        None
    }
}

/// Handle de la app. En host no se crea nunca; los métodos devuelven "sin datos".
pub struct AndroidApp(());

impl AndroidApp {
    pub fn internal_data_path(&self) -> Option<PathBuf> {
        None
    }

    pub fn external_data_path(&self) -> Option<PathBuf> {
        None
    }

    pub fn config(&self) -> ConfigurationRef {
        ConfigurationRef
    }

    pub fn content_rect(&self) -> Rect {
        Rect::default()
    }

    pub fn native_window(&self) -> Option<ndk::native_window::NativeWindow> {
        None
    }
}

/// Tipos de input que la lógica de gestos maneja como valores.
pub mod input {
    /// Bits de botones pulsados (`AMotionEvent_getButtonState`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct ButtonState(pub u32);

    /// Botón que cambia en un `ButtonPress`/`ButtonRelease`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Button {
        Primary,
        Secondary,
        Tertiary,
        Back,
        Forward,
        StylusPrimary,
        StylusSecondary,
        Unknown(u32),
    }

    impl From<Button> for u32 {
        fn from(b: Button) -> u32 {
            match b {
                Button::Primary => 1 << 0,
                Button::Secondary => 1 << 1,
                Button::Tertiary => 1 << 2,
                Button::Back => 1 << 3,
                Button::Forward => 1 << 4,
                Button::StylusPrimary => 1 << 5,
                Button::StylusSecondary => 1 << 6,
                Button::Unknown(v) => v,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MotionAction {
        Down,
        Up,
        Move,
        Cancel,
        Outside,
        PointerDown,
        PointerUp,
        HoverMove,
        Scroll,
        HoverEnter,
        HoverExit,
        ButtonPress,
        ButtonRelease,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ToolType {
        Unknown,
        Finger,
        Stylus,
        Mouse,
        Eraser,
        Palm,
    }
}

/// Ventana nativa y formato de buffer (`ndk`).
pub mod ndk {
    pub mod hardware_buffer_format {
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum HardwareBufferFormat {
            R8G8B8A8_UNORM,
        }

        impl HardwareBufferFormat {
            pub fn bytes_per_pixel(self) -> Option<usize> {
                Some(4)
            }
        }
    }

    pub mod native_window {
        use super::hardware_buffer_format::HardwareBufferFormat;
        use std::ffi::c_void;
        use std::ptr::NonNull;

        /// Error de las llamadas a la ventana (nunca ocurre: no hay ventanas en host).
        #[derive(Debug)]
        pub struct WindowError;

        impl std::fmt::Display for WindowError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("no native window on host")
            }
        }

        /// `ANativeWindow`. Sin constructor: en host no existe ninguna.
        #[derive(Debug)]
        pub struct NativeWindow(());

        impl NativeWindow {
            pub fn width(&self) -> i32 {
                0
            }

            pub fn height(&self) -> i32 {
                0
            }

            pub fn ptr(&self) -> NonNull<c_void> {
                NonNull::dangling()
            }

            pub fn set_buffers_geometry(
                &self,
                _width: i32,
                _height: i32,
                _format: Option<HardwareBufferFormat>,
            ) -> Result<(), WindowError> {
                Err(WindowError)
            }

            pub fn lock(&self, _dirty: Option<()>) -> Result<LockGuard, WindowError> {
                Err(WindowError)
            }
        }

        /// Buffer bloqueado de la ventana.
        pub struct LockGuard(());

        impl LockGuard {
            pub fn format(&self) -> HardwareBufferFormat {
                HardwareBufferFormat::R8G8B8A8_UNORM
            }

            pub fn width(&self) -> usize {
                0
            }

            pub fn height(&self) -> usize {
                0
            }

            pub fn stride(&self) -> usize {
                0
            }

            pub fn bits(&mut self) -> *mut c_void {
                std::ptr::null_mut()
            }
        }
    }
}
