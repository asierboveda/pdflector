// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! `pdf_android` — Native Android PDF reader cdylib hosted by `android/product`
//! (`GameActivity` + Kotlin `PdfLectorActivity`, ADR-005 / ADR-010).
//!
//! Renders PDFs via `pdf_core` (MuPDF, ADR-001) on an EGL/GLES2 two-layer
//! pipeline (Dry consolidated page + Wet fallback) paired with an AndroidX
//! `GLFrontBufferedRenderer` transparent ink overlay for causal low-latency
//! stylus strokes (`ink::causal`, ADR-009 / ADR-010).
//!
//! For the canonical factual description of connected flows, validation status,
//! and architectural boundaries, see `ESTADO_ACTUAL.md` and `AGENTS.md`.
//!
//! Module layout:
//! - `reader`: `Reader` state machine (`Library`, `Picker`, `Viewer`, `Discover`).
//! - `input`: touch/stylus dispatch, gestures, pinch/pan, and tool routing.
//! - `ink`: causal stylus engine (`causal`, `engine`).
//! - `gpu`: EGL/GLES2 context, shaders, textures, and Dry/Wet pipeline.
//! - `draw`: Canvas+JNI bitmaps, overlays, toolbar, library, and AI text layout.
//! - `jni`: JNI bridge to `PdfLectorActivity`, MediaStore, IME, and ink overlay.
//! - `folder_sync`: background SQLite persistence and folder-sync export worker.
//! - `cache`, `thumbs`, `persist`, `undo`, `view`, `zoom`, `annotations`, `discover`.

// En host solo se compilan los tests: el código que únicamente alcanza
// `android_main` queda sin uso y no debe ensuciar la salida con avisos.
#![cfg_attr(not(target_os = "android"), allow(dead_code, unused_imports))]

// En Android, `crate::android_activity` es la crate real; en host (tests en
// Linux, donde ndk-sys no compila) es un sustituto inerte con los mismos
// nombres, así el resto del código no distingue plataforma.
#[cfg(target_os = "android")]
pub(crate) use ::android_activity;
#[cfg(not(target_os = "android"))]
#[path = "host/android_activity.rs"]
pub(crate) mod android_activity;

#[cfg(target_os = "android")]
use crate::android_activity::{AndroidApp, MainEvent, PollEvent};
use log::info;

mod annotations;
mod cache;
mod draw;
mod folder_sync;
mod gpu;
pub mod ink;
mod input;
mod jni;
mod persist;
mod reader;
mod thumbs;
mod undo;
mod view;
mod zoom;

#[cfg(target_os = "android")]
use crate::input::handle_input;
use crate::reader::Reader;

/// Radio (px) de movimiento máximo entre Down y Up para considerar el gesto un
/// "tap" (no un swipe). ~20 px a 320 dpi (ViewConfiguration touch slop ≈ 8 dp).
pub(crate) const TAP_SLOP: f32 = 24.0;
/// Discriminación de gestos de dos dedos (pan vs pinch zoom):
/// Desplazamiento mínimo acumulado del centro (px) para confirmar traslación pura (pan).
pub(crate) const TWO_FINGER_PAN_MIN_DISP_PX: f32 = 4.0;
/// Variación relativa de distancia entre dedos (|d − start_dist| / start_dist)
/// necesaria para confirmar zoom desde el estado Undecided (3.5%).
pub(crate) const TWO_FINGER_ZOOM_REL_THRESHOLD: f32 = 0.035;
/// Cota mínima absoluta en px para confirmar zoom si los dedos están muy juntos.
pub(crate) const TWO_FINGER_ZOOM_MIN_PX: f32 = 6.0;
/// Variación relativa mínima para transición de Pan a Zoom (6.0%).
pub(crate) const TWO_FINGER_PAN_TO_ZOOM_REL: f32 = 0.06;
/// Cota mínima en px para transición de Pan a Zoom tras frenar la traslación.
pub(crate) const TWO_FINGER_PAN_TO_ZOOM_MIN_PX: f32 = 12.0;
/// Umbral de movimiento (px) tras el long-press para EXTENDER el rect de
/// selección: el ancla es el punto del dedo al superar `LONG_PRESS_MS` y el
/// rect (un punto) solo empieza a seguir al dedo si se mueve más de esto
/// (los micro-drags no extienden la selección); un long-press sin arrastre
/// no fija selección.
pub(crate) const SELECT_SLOP: f32 = 8.0;
/// Tamaño mínimo (px) del rect de selección para pedir la explicación:
/// un rect degenerado (long-press sin arrastre) se descarta.
pub(crate) const SEL_MIN_PX: f32 = 2.0;
/// Umbral (px de pantalla) que separa un "toque" (sin gesto) de un gesto de
/// herramienta real (boli/resaltador): si el gesto no recorre al menos esto,
/// `Reader::end_tool_gesture` lo descarta (mismo criterio que el tamaño
/// mínimo de la selección de texto, `SEL_MIN_PX`). Un dedo o el lápiz que
/// toca y suelta sin intención de dibujar no crea una anotación por accidente.
pub(crate) const TOOL_MIN_PX: f32 = 6.0;
/// Tiempo tras el último trazo del stylus durante el cual se ignora el táctil
/// (palm rejection por tiempo): evita pans/zooms accidentales de la palma
/// al apoyar la mano al escribir. 500ms es un compromiso entre reactividad
/// y seguridad (Saber usa ~300-500ms, Samsung Notes ~400ms).
pub(crate) const STYLUS_IGNORE_MS: u64 = 500;
/// Duración del aviso breve ("copied", "highlighted", ...) sobre el indicador
/// de página (`Reader::toast`, expirado en `Reader::tick`).
pub(crate) const TOAST_MS: std::time::Duration = std::time::Duration::from_millis(1500);
/// Duración (s) de la transición al abrir un libro: el snapshot de la
/// biblioteca/picker se funde sobre la página. ~12 frames a 60 fps.
pub(crate) const LIB_FADE_MS: f32 = 0.18;

// ---------------------------------------------------------------------------
// Configuración de "Preguntar a la IA": Gemini (texto e imagen)
// ---------------------------------------------------------------------------
//
// La API key va EMBEBIDA en el APK: es una app de USO PERSONAL (sin
// telemetría ni servidores propios) y la consulta va directa del dispositivo
// a Google por HTTPS. Se obtiene en https://aistudio.google.com/apikey.
// PRIVACIDAD: la key viaja en el binario — no publiques el APK ni el repo
// con la key real; el fichero está ignorado por Git y un placeholder
// compila pero el panel avisa de que falta la clave.
pub(crate) const GOOGLE_API_KEY: &str = include_str!("../google_key.txt");
/// Modelo de GEMINI para "Preguntar a la IA": multimodal (texto + imagen),
/// así que la misma llamada explica texto, ecuaciones y gráficos a partir
/// del crop de la selección y del contexto de la hoja. Es la variante Lite:
/// medida en el escritorio con la misma clave, responde en ~1 s, mientras
/// `gemini-flash-latest` devolvía 503 por alta demanda (y en la tablet la
/// consulta tardaba decenas de segundos antes de fallar).
pub(crate) const GEMINI_MODEL: &str = "gemini-flash-lite-latest";
/// Respaldo si el modelo principal responde ocupado (503): otro Lite,
/// también medido a ~1 s con texto e imagen.
pub(crate) const GEMINI_FALLBACK_MODEL: &str = "gemini-3.5-flash-lite";
/// Límites del factor de zoom continuo (1.0 = página completa a pantalla).
/// `PINCH_MIN = 1.0`: SIN zoom hacia fuera — la página no se puede ver más
/// pequeña que el encuadre inicial contain; el pan queda limitado a los
/// bordes de la hoja (ver `Reader::clamp_pan`).
pub(crate) const PINCH_MIN: f32 = 1.0;
pub(crate) const PINCH_MAX: f32 = 8.0;

/// Sistema de diseño Astryx adaptado a Canvas JNI nativo (colores ARGB 0xAARRGGBB).
///
/// Jerarquía de superficies (`body → surface → card → popover`), tokens de forma
/// (`RADIUS_*`) y escala tipográfica geométrica
/// con contraste WCAG AA verificado en los 4 temas persistidos.
pub(crate) mod theme {
    use serde::{Deserialize, Serialize};

    /// Color transparente para fondos de bitmap sin opacidad.
    pub(crate) const TRANSPARENT: u32 = 0x0000_0000;

    /// Fondo de error cuando no se pudo abrir el PDF (Astryx error inverted dark).
    pub(crate) const ERROR_BG_RGBA: [u8; 4] = [0x41, 0x17, 0x1F, 0xFF];

    /// Relleno y borde del rect de selección (Astryx `--color-accent` `#0064E0`).
    pub(crate) const SEL_FILL_RGBA: [u8; 4] = [0x01, 0x71, 0xE3, 0x3D];
    pub(crate) const SEL_BORDER_RGBA: [u8; 4] = [0x00, 0x64, 0xE0, 0xFF];

    // ---- Escala de forma (Astryx Shape: inner → element → container → page → full) ----
    pub(crate) const RADIUS_INNER: f32 = 4.0;
    pub(crate) const RADIUS_ELEMENT: f32 = 8.0;
    pub(crate) const RADIUS_CONTAINER: f32 = 12.0;
    pub(crate) const RADIUS_CARD: f32 = 16.0;
    pub(crate) const RADIUS_PAGE: f32 = 24.0;
    pub(crate) const RADIUS_FULL: f32 = 999.0;

    // ---- Escala tipográfica geométrica (Astryx Type Scale) ----
    pub(crate) const FONT_DISPLAY: f32 = 26.0;
    pub(crate) const FONT_H1: f32 = 20.0;
    pub(crate) const FONT_TITLE: f32 = 17.0;
    pub(crate) const FONT_BODY: f32 = 14.0;
    pub(crate) const FONT_CAPTION: f32 = 12.0;
    pub(crate) const FONT_LABEL_CAPS: f32 = 11.0;
    pub(crate) const FONT_MICRO: f32 = 10.0;

    /// Tracking tipográfico (`Paint.setLetterSpacing` en em).
    pub(crate) const TRACKING_CAPS: f32 = 0.08;
    pub(crate) const TRACKING_TIGHT: f32 = -0.015;

    /// Temas persistidos en `state.json` (se preservan las variantes exactas).
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
    pub(crate) enum AppTheme {
        #[default]
        DefaultLight,
        SepiaLight,
        DefaultDark,
        SepiaDark,
    }

    impl AppTheme {
        /// Cicla al siguiente tema: Claro → Papel → Oscuro → Nocturno → Claro.
        pub(crate) fn next(self) -> Self {
            match self {
                Self::DefaultLight => Self::SepiaLight,
                Self::SepiaLight => Self::DefaultDark,
                Self::DefaultDark => Self::SepiaDark,
                Self::SepiaDark => Self::DefaultLight,
            }
        }

        /// ¿Modo oscuro activo?
        pub(crate) fn is_dark(self) -> bool {
            matches!(self, Self::DefaultDark | Self::SepiaDark)
        }

        /// Paleta semántica Astryx completa para este tema.
        pub(crate) fn palette(self) -> ThemePalette {
            match self {
                // Astryx Core Light (`#F1F4F7` body → `#FFFFFF` surface/card/popover, `#0064E0` accent)
                Self::DefaultLight => ThemePalette {
                    is_dark: false,
                    bg_body: 0xFFF1_F4F7,
                    bg_surface: 0xFFFF_FFFF,
                    bg_card: 0xFFFF_FFFF,
                    bg_popover: 0xFFFF_FFFF,
                    bg_muted: 0xFFE6_ECF2,
                    bg_inverted: 0xFF0A_1317,
                    text_primary: 0xFF0A_1317,
                    text_secondary: 0xFF44_5564,
                    text_accent: 0xFF00_64E0,
                    on_accent: 0xFFFF_FFFF,
                    on_inverted: 0xFFFF_FFFF,
                    border: 0xFFD8_E0E8,
                    border_emphasized: 0xFFBC_C6D0,
                    focus_ring: 0x8001_71E3,
                    track: 0xFFCC_D3DB,
                    accent: 0xFF00_64E0,
                    accent_muted: 0xFFD8_EBFF,
                    on_accent_muted: 0xFF04_2F97,
                    warning_bg: 0xFFFB_F0CE,
                    warning_text: 0xFF6B_3A06,
                    error_bg: 0xFFFC_DCE1,
                    error_text: 0xFF8B_0A1E,
                    error_border: 0xFFE3_193B,
                    teal_bg: 0xFFD4_F1EF,
                    teal_text: 0xFF08_3943,
                    purple_bg: 0xFFE7_E0FF,
                    purple_text: 0xFF3E_0697,
                },
                // Astryx Paper Light (papel cálido editorial + acento Astryx Teal)
                Self::SepiaLight => ThemePalette {
                    is_dark: false,
                    bg_body: 0xFFEC_E3D2,
                    bg_surface: 0xFFF8_F3E8,
                    bg_card: 0xFFF8_F3E8,
                    bg_popover: 0xFFFB_F7EE,
                    bg_muted: 0xFFE4_D9C3,
                    bg_inverted: 0xFF26_1D15,
                    text_primary: 0xFF26_1D15,
                    text_secondary: 0xFF5D_4B3B,
                    text_accent: 0xFF00_6663,
                    on_accent: 0xFFFF_FFFF,
                    on_inverted: 0xFFF8_F3E8,
                    border: 0xFFD8_CBB4,
                    border_emphasized: 0xFFBE_AE93,
                    focus_ring: 0x8000_6E6A,
                    track: 0xFFCF_C1A8,
                    accent: 0xFF00_6E6A,
                    accent_muted: 0xFFCF_E8E5,
                    on_accent_muted: 0xFF06_3D3B,
                    warning_bg: 0xFFF5_E7C1,
                    warning_text: 0xFF62_3B07,
                    error_bg: 0xFFF6_D9D5,
                    error_text: 0xFF84_1515,
                    error_border: 0xFFC8_2333,
                    teal_bg: 0xFFCF_E8E5,
                    teal_text: 0xFF06_3D3B,
                    purple_bg: 0xFFE5_DCF2,
                    purple_text: 0xFF3F_1B78,
                },
                // Astryx Core Dark (`#111112` body → `#1F1F22` card → `#28292C` popover)
                Self::DefaultDark => ThemePalette {
                    is_dark: true,
                    bg_body: 0xFF11_1112,
                    bg_surface: 0xFF1F_1F22,
                    bg_card: 0xFF1F_1F22,
                    bg_popover: 0xFF28_292C,
                    bg_muted: 0xFF2B_2D31,
                    bg_inverted: 0xFFFF_FFFF,
                    text_primary: 0xFFDF_E2E5,
                    text_secondary: 0xFFB0_B5BB,
                    text_accent: 0xFF5B_AEFF,
                    on_accent: 0xFFFF_FFFF,
                    on_inverted: 0xFF0A_1317,
                    border: 0xFF2E_3035,
                    border_emphasized: 0xFF49_4D53,
                    focus_ring: 0x8026_94FE,
                    track: 0xFF49_4D53,
                    accent: 0xFF01_71E3,
                    accent_muted: 0xFF16_324E,
                    on_accent_muted: 0xFFAF_D7FF,
                    warning_bg: 0xFF3E_2F10,
                    warning_text: 0xFFFB_D65A,
                    error_bg: 0xFF41_171F,
                    error_text: 0xFFFF_B2B8,
                    error_border: 0xFFF5_394F,
                    teal_bg: 0xFF11_3638,
                    teal_text: 0xFF66_E4D8,
                    purple_bg: 0xFF2A_1F4B,
                    purple_text: 0xFFC7_C2FF,
                },
                // Astryx Nocturne Paper (sombra cálida de estudio nocturno + Teal luminoso)
                Self::SepiaDark => ThemePalette {
                    is_dark: true,
                    bg_body: 0xFF17_130F,
                    bg_surface: 0xFF23_1E18,
                    bg_card: 0xFF23_1E18,
                    bg_popover: 0xFF2E_2820,
                    bg_muted: 0xFF32_2B23,
                    bg_inverted: 0xFFF3_E6D0,
                    text_primary: 0xFFF3_E6D0,
                    text_secondary: 0xFFC4_B094,
                    text_accent: 0xFF55_D8D0,
                    on_accent: 0xFFFF_FFFF,
                    on_inverted: 0xFF17_130F,
                    border: 0xFF39_3127,
                    border_emphasized: 0xFF56_4A3C,
                    focus_ring: 0x8026_A69A,
                    track: 0xFF56_4A3C,
                    accent: 0xFF13_847E,
                    accent_muted: 0xFF17_3634,
                    on_accent_muted: 0xFF8C_ECE5,
                    warning_bg: 0xFF3E_3014,
                    warning_text: 0xFFF8_D568,
                    error_bg: 0xFF41_1C1B,
                    error_text: 0xFFFF_B5B0,
                    error_border: 0xFFF2_5C5C,
                    teal_bg: 0xFF17_3634,
                    teal_text: 0xFF8C_ECE5,
                    purple_bg: 0xFF2D_2242,
                    purple_text: 0xFFD0_C5FF,
                },
            }
        }
    }

    /// Paleta semántica de color Astryx para renderizado nativo Canvas+JNI.
    #[derive(Copy, Clone, Debug)]
    pub(crate) struct ThemePalette {
        pub(crate) is_dark: bool,
        // Superficies Astryx (`body → surface → card → popover`)
        pub(crate) bg_body: u32,
        pub(crate) bg_surface: u32,
        pub(crate) bg_card: u32,
        pub(crate) bg_popover: u32,
        pub(crate) bg_muted: u32,
        pub(crate) bg_inverted: u32,
        // Tipografía e iconos
        pub(crate) text_primary: u32,
        pub(crate) text_secondary: u32,
        pub(crate) text_accent: u32,
        pub(crate) on_accent: u32,
        pub(crate) on_inverted: u32,
        // Bordes, pistas y anillo de foco
        pub(crate) border: u32,
        pub(crate) border_emphasized: u32,
        pub(crate) focus_ring: u32,
        pub(crate) track: u32,
        // Acento principal y superficie tonal
        pub(crate) accent: u32,
        pub(crate) accent_muted: u32,
        pub(crate) on_accent_muted: u32,
        // Roles semánticos de estado y taxonomía
        pub(crate) warning_bg: u32,
        pub(crate) warning_text: u32,
        pub(crate) error_bg: u32,
        pub(crate) error_text: u32,
        pub(crate) error_border: u32,
        pub(crate) teal_bg: u32,
        pub(crate) teal_text: u32,
        pub(crate) purple_bg: u32,
        pub(crate) purple_text: u32,
    }

    impl ThemePalette {
        pub(crate) fn badge_bg(&self) -> u32 {
            (self.bg_popover & 0x00FF_FFFF) | 0xEB00_0000
        }
        pub(crate) fn badge_border(&self) -> u32 {
            self.border_emphasized
        }
        pub(crate) fn badge_text(&self) -> u32 {
            self.text_primary
        }
        pub(crate) fn status_bg(&self) -> u32 {
            self.error_bg
        }
        pub(crate) fn status_border(&self) -> u32 {
            self.error_border
        }
        pub(crate) fn status_text(&self) -> u32 {
            self.error_text
        }
        pub(crate) fn rgba_bg(&self) -> [u8; 4] {
            argb_to_rgba(self.bg_body)
        }
        pub(crate) fn rgba_lib_bg(&self) -> [u8; 4] {
            argb_to_rgba(self.bg_body)
        }
    }

    pub(crate) fn argb_to_rgba(c: u32) -> [u8; 4] {
        [
            ((c >> 16) & 0xFF) as u8,
            ((c >> 8) & 0xFF) as u8,
            (c & 0xFF) as u8,
            ((c >> 24) & 0xFF) as u8,
        ]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rel_luminance(argb: u32) -> f64 {
            let chan = |shift: u32| -> f64 {
                let s = ((argb >> shift) & 0xFF) as f64 / 255.0;
                if s <= 0.03928 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * chan(16) + 0.7152 * chan(8) + 0.0722 * chan(0)
        }

        fn contrast_ratio(fg: u32, bg: u32) -> f64 {
            let l1 = rel_luminance(fg);
            let l2 = rel_luminance(bg);
            (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
        }

        /// Fija los tokens de forma y tipografía que usan los widgets: escalas
        /// estrictamente monótonas y radios en el ritmo de 4 px de Astryx.
        #[test]
        fn astryx_shape_and_type_scales_are_ordered() {
            let radii = [
                RADIUS_INNER,
                RADIUS_ELEMENT,
                RADIUS_CONTAINER,
                RADIUS_CARD,
                RADIUS_PAGE,
                RADIUS_FULL,
            ];
            assert!(radii.windows(2).all(|w| w[0] < w[1]), "radios: {radii:?}");
            assert!(radii.iter().all(|r| r % 4.0 == 0.0 || *r == RADIUS_FULL));

            let fonts = [
                FONT_DISPLAY,
                FONT_H1,
                FONT_TITLE,
                FONT_BODY,
                FONT_CAPTION,
                FONT_LABEL_CAPS,
                FONT_MICRO,
            ];
            assert!(fonts.windows(2).all(|w| w[0] > w[1]), "fuentes: {fonts:?}");
            const { assert!(TRACKING_CAPS > 0.0 && TRACKING_TIGHT < 0.0) };
        }

        /// Fija los accesores derivados de la paleta (insignias, estado y fondo RGBA).
        #[test]
        fn palette_derived_roles_map_to_semantic_tokens() {
            for t in [
                AppTheme::DefaultLight,
                AppTheme::SepiaLight,
                AppTheme::DefaultDark,
                AppTheme::SepiaDark,
            ] {
                let p = t.palette();
                assert_eq!(p.is_dark, t.is_dark());
                assert_eq!(p.badge_bg() & 0x00FF_FFFF, p.bg_popover & 0x00FF_FFFF);
                assert_eq!(p.badge_bg() >> 24, 0xEB);
                assert_eq!(p.badge_border(), p.border_emphasized);
                assert_eq!(p.badge_text(), p.text_primary);
                assert_eq!(p.status_bg(), p.error_bg);
                assert_eq!(p.status_border(), p.error_border);
                assert_eq!(p.status_text(), p.error_text);
                assert_eq!(p.rgba_bg(), argb_to_rgba(p.bg_body));
                assert_eq!(p.rgba_lib_bg(), argb_to_rgba(p.bg_body));
            }
            assert_eq!(argb_to_rgba(0x8011_2233), [0x11, 0x22, 0x33, 0x80]);
        }

        #[test]
        fn all_astryx_themes_meet_wcag_aa_contrast() {
            let themes = [
                AppTheme::DefaultLight,
                AppTheme::SepiaLight,
                AppTheme::DefaultDark,
                AppTheme::SepiaDark,
            ];
            for t in themes {
                let p = t.palette();
                let pairs = [
                    ("text_primary/bg_body", p.text_primary, p.bg_body),
                    ("text_primary/bg_card", p.text_primary, p.bg_card),
                    ("text_primary/bg_popover", p.text_primary, p.bg_popover),
                    ("text_secondary/bg_body", p.text_secondary, p.bg_body),
                    ("text_secondary/bg_card", p.text_secondary, p.bg_card),
                    ("text_secondary/bg_popover", p.text_secondary, p.bg_popover),
                    ("text_accent/bg_card", p.text_accent, p.bg_card),
                    ("on_accent/accent", p.on_accent, p.accent),
                    (
                        "on_accent_muted/accent_muted",
                        p.on_accent_muted,
                        p.accent_muted,
                    ),
                    ("error_text/error_bg", p.error_text, p.error_bg),
                    ("error_text/bg_popover", p.error_text, p.bg_popover),
                    ("on_inverted/bg_inverted", p.on_inverted, p.bg_inverted),
                    ("warning_text/warning_bg", p.warning_text, p.warning_bg),
                    ("teal_text/teal_bg", p.teal_text, p.teal_bg),
                    ("purple_text/purple_bg", p.purple_text, p.purple_bg),
                ];
                for (name, fg, bg) in pairs {
                    let ratio = contrast_ratio(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{:?} pair {name} contrast {ratio:.2} < 4.5:1",
                        t
                    );
                }
            }
        }
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("pdf_android"),
    );

    let mut reader = Reader::new(&app);
    let mut running = true;
    crate::jni::enter_immersive(&app);
    crate::jni::keep_screen_on(&app);
    crate::jni::enable_120hz(&app);

    while running {
        // Timeout del poll con ventana (8 ms → tick por vsync a 120 Hz):
        // con `poll_events(None)` (reposo) el looper de android-activity
        // puede quedarse dormido y PERDER los toques del visor (bug medido
        // en la TCL: el keyevent despertaba pero el touch no — la app
        // "se quedaba pillada"). A 120 Hz el tick a 8 ms garantiza que no se
        // acumule lag de eventos. Sin ventana (o sin activity), el poll sigue bloqueante.
        let timeout = if reader.has_window() {
            Some(std::time::Duration::from_millis(8))
        } else {
            None
        };
        app.poll_events(timeout, |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                info!("InitWindow");
                reader.status_bar_top = app.content_rect().top;
                info!("status_bar_top: {}", reader.status_bar_top);
                if let Some(win) = app.native_window() {
                    reader.init_window(win);
                }
                // NO re-aplicar keep_screen_on aquí: getWindow().addFlags
                // desde el hilo android_main lanza "Only the original thread
                // that created a view hierarchy can touch its views" (la
                // ventana la crea el UI thread Java); la llamada del arranque
                // (before del bucle, en onCreate aún sin jerarquía) aplica el
                // flag al objeto Window y persiste (verificado: dumpsys
                // window → fl=KEEP_SCREEN_ON).
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                info!("TerminateWindow");
                reader.terminate_window();
            }
            PollEvent::Main(MainEvent::WindowResized { .. }) => {
                info!("WindowResized");
                reader.status_bar_top = app.content_rect().top;
                // Re-obtener el handle: tras resize/recreate puede estar stale
                // (ver nota de refresco en la cabecera del módulo).
                if let Some(win) = app.native_window() {
                    reader.set_window(win);
                }
                reader.redraw();
            }
            PollEvent::Main(MainEvent::ConfigChanged { .. }) => {
                reader.set_density_dpi(app.config().density().unwrap_or(160));
                reader.redraw();
            }
            PollEvent::Main(MainEvent::RedrawNeeded { .. }) => {
                if let Some(win) = app.native_window() {
                    reader.set_window(win);
                }
                reader.redraw();
            }
            PollEvent::Main(MainEvent::InputAvailable) => {
                handle_input(&app, &mut reader);
            }
            PollEvent::Main(MainEvent::Resume { .. }) => {
                info!("Resume");
                // Biblioteca CURADA: el Resume NO re-consulta MediaStore (la
                // rejilla sale de `internal/library.json`); solo se limpia la
                // marca "pendiente de conceder permiso" — el selector de
                // añadir re-comprueba el permiso por sí mismo al invocarse.
                reader.grant_pending = false;
                reader.folder_sync.request_scan();
            }
            PollEvent::Main(MainEvent::Pause) => {
                info!("Pause");
                // A1: flush explícito del estado diferido al pausar (Home /
                // cambio de app): el proceso puede morir en segundo plano
                // antes de que `tick` cumpla los 2 s del flush periódico.
                reader.flush_state();
                reader.flush_annotation_publication();
            }
            PollEvent::Main(MainEvent::Destroy) => {
                info!("Destroy: saliendo del bucle");
                running = false;
            }
            PollEvent::Wake | PollEvent::Timeout | PollEvent::Main(_) => {
                // Trabajo diferido mientras `has_window()`: animación del sheet,
                // lote de portadas, long-press, render asíncrono, IA y Discover.
                reader.tick(&app);
            }
            _ => {} // PollEvent es #[non_exhaustive]: cubrir futuras variantes
        });
        // Coalescing por vsync (Fase C, comparativa saber-notes): UN blit por
        // iteración si `update_tool_gesture` marcó repintar. Los Moves del
        // boli llegan a 120 Hz pero la pantalla solo presenta a 60 Hz —
        // blitear por evento bloquearía el BufferQueue (~16 ms por
        // unlock_and_post) y encadenaría el lag. Con coalescing: latencia ≤1
        // frame y coste por frame = dirty rect (< 1 ms).
        if reader.take_repaint() {
            reader.blit();
        }
    }

    info!("android_main: fin");
}
