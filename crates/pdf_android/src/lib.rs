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

#![cfg_attr(not(target_os = "android"), allow(dead_code))]

#[cfg(target_os = "android")]
use android_activity::{AndroidApp, MainEvent, PollEvent};
#[cfg(target_os = "android")]
use log::info;

mod annotations;
mod cache;
#[cfg(target_os = "android")]
mod draw;
mod folder_sync;
mod gpu;
pub mod ink;
#[cfg(target_os = "android")]
mod input;
#[cfg(target_os = "android")]
mod jni;
mod persist;
#[cfg(target_os = "android")]
mod reader;
mod thumbs;
mod undo;
mod view;
#[cfg(target_os = "android")]
mod zoom;

#[cfg(target_os = "android")]
use crate::input::handle_input;
#[cfg(target_os = "android")]
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
#[cfg(target_os = "android")]
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

/// Constantes de color, temas y tipografía para la interfaz (0xAARRGGBB para Canvas JNI).
pub(crate) mod theme {
    use serde::{Deserialize, Serialize};

    /// Color transparente para fondos de bitmap sin opacidad.
    pub(crate) const TRANSPARENT: u32 = 0x00000000;

    /// Fondo de error cuando no se pudo abrir el PDF (rojo oscuro opaco).
    pub(crate) const ERROR_BG_RGBA: [u8; 4] = [0x5A, 0x12, 0x12, 0xFF];

    /// Color del relleno del rect de selección (azul accent, alfa ~30 %: 77/255).
    pub(crate) const SEL_FILL_RGBA: [u8; 4] = [0x4D, 0xA3, 0xFF, 0x4D];
    /// Color del borde del rect de selección (1-2 px, alfa completo).
    pub(crate) const SEL_BORDER_RGBA: [u8; 4] = [0x4D, 0xA3, 0xFF, 0xFF];

    /// Jerarquía tipográfica única (Readest Design System).
    pub(crate) const FONT_DISPLAY: f32 = 24.0;
    pub(crate) const FONT_TITLE: f32 = 17.0;
    pub(crate) const FONT_BODY: f32 = 14.0;
    pub(crate) const FONT_CAPTION: f32 = 12.0;
    pub(crate) const FONT_LABEL_CAPS: f32 = 11.0;

    /// Temas disponibles en PDFLector (reglas y paletas derivadas exactas de Readest).
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
    pub(crate) enum AppTheme {
        #[default]
        DefaultLight,
        SepiaLight,
        DefaultDark,
        SepiaDark,
    }

    impl AppTheme {
        /// Cicla al siguiente tema: Default-Light → Sepia-Light → Default-Dark → Sepia-Dark → Default-Light...
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

        /// Obtiene la paleta completa calculada para este tema según las reglas Readest.
        pub(crate) fn palette(self) -> ThemePalette {
            match self {
                Self::DefaultLight => ThemePalette {
                    name: "Default-Light",
                    is_dark: false,
                    base_100: 0xFFFFFFFF,
                    base_200: 0xFFF2F2F2,
                    base_300: 0xFFE0E0E0,
                    base_content: 0xFF171717,
                    neutral: 0xFFD9D9D9,
                    neutral_content: 0xFF737373,
                    primary: 0xFF0066CC,
                    primary_content: 0xFFFFFFFF,
                },
                Self::SepiaLight => ThemePalette {
                    name: "Sepia-Light",
                    is_dark: false,
                    base_100: 0xFFF1E8D0,
                    base_200: 0xFFE6DCBF,
                    base_300: 0xFFD4C8A5,
                    base_content: 0xFF5B4636,
                    neutral: 0xFFC9BC96,
                    neutral_content: 0xFF8A705B,
                    primary: 0xFF008B8B,
                    primary_content: 0xFFFFFFFF,
                },
                Self::DefaultDark => ThemePalette {
                    name: "Default-Dark",
                    is_dark: true,
                    base_100: 0xFF242424,
                    // Fondo de biblioteca (delta de luminosidad +10% más profundo en dark para contraste de tarjetas)
                    base_200: 0xFF141414,
                    base_300: 0xFF3D3D3D,
                    base_content: 0xFFE0E0E0,
                    neutral: 0xFF474747,
                    neutral_content: 0xFF9E9E9E,
                    primary: 0xFF77BBEE,
                    primary_content: 0xFF111111,
                },
                Self::SepiaDark => ThemePalette {
                    name: "Sepia-Dark",
                    is_dark: true,
                    base_100: 0xFF342E25,
                    // Fondo de biblioteca (delta de luminosidad +10% más profundo en dark para contraste de tarjetas)
                    base_200: 0xFF201B15,
                    base_300: 0xFF4D4437,
                    base_content: 0xFFFFD595,
                    neutral: 0xFF615747,
                    neutral_content: 0xFFC4A572,
                    primary: 0xFF48D1CC,
                    primary_content: 0xFF1A1610,
                },
            }
        }
    }

    /// Paleta de color derivada de Readest para renderizado UI.
    #[allow(dead_code)] // tokens Readest completos: no todos los roles están cableados a widgets aún
    #[derive(Copy, Clone, Debug)]
    pub(crate) struct ThemePalette {
        pub(crate) name: &'static str,
        pub(crate) is_dark: bool,
        pub(crate) base_100: u32,
        pub(crate) base_200: u32,
        pub(crate) base_300: u32,
        pub(crate) base_content: u32,
        pub(crate) neutral: u32,
        pub(crate) neutral_content: u32,
        pub(crate) primary: u32,
        pub(crate) primary_content: u32,
    }

    #[allow(dead_code)] // Métodos de acceso a tokens Readest; se conservan por completitud del sistema de diseño.
    impl ThemePalette {
        pub(crate) fn bg(&self) -> u32 {
            self.base_100
        }
        pub(crate) fn lib_bg(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn card_bg(&self) -> u32 {
            self.base_100
        }
        pub(crate) fn card_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn btn_bg(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn btn_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn btn_text(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn text_primary(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn text_secondary(&self) -> u32 {
            self.neutral_content
        }
        pub(crate) fn text_muted(&self) -> u32 {
            self.neutral_content
        }
        pub(crate) fn accent(&self) -> u32 {
            self.primary
        }
        pub(crate) fn accent_text(&self) -> u32 {
            self.primary_content
        }
        pub(crate) fn progress_track(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn progress_fill(&self) -> u32 {
            self.primary
        }
        pub(crate) fn cover_shadow(&self) -> u32 {
            if self.is_dark { 0x70000000 } else { 0x34000000 }
        }
        pub(crate) fn cover_placeholder(&self) -> u32 {
            self.base_200
        }
        pub(crate) fn sel_overlay(&self) -> u32 {
            if self.is_dark { 0x5577BBEE } else { 0x440066CC }
        }
        pub(crate) fn popup_bg(&self) -> u32 {
            if self.is_dark { 0xF2222222 } else { 0xF2FFFFFF }
        }
        pub(crate) fn popup_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn badge_bg(&self) -> u32 {
            if self.is_dark { 0xDD222222 } else { 0xDDFFFFFF }
        }
        pub(crate) fn badge_border(&self) -> u32 {
            self.base_300
        }
        pub(crate) fn badge_text(&self) -> u32 {
            self.base_content
        }
        pub(crate) fn status_bg(&self) -> u32 {
            if self.is_dark { 0xFF3A1A1A } else { 0xFFFFEAEA }
        }
        pub(crate) fn status_border(&self) -> u32 {
            if self.is_dark { 0xFF5A2A2A } else { 0xFFFFCCCC }
        }
        pub(crate) fn status_text(&self) -> u32 {
            if self.is_dark { 0xFFFF9999 } else { 0xFFCC0000 }
        }
        pub(crate) fn rgba_bg(&self) -> [u8; 4] {
            let a = ((self.base_100 >> 24) & 0xFF) as u8;
            let r = ((self.base_100 >> 16) & 0xFF) as u8;
            let g = ((self.base_100 >> 8) & 0xFF) as u8;
            let b = (self.base_100 & 0xFF) as u8;
            [r, g, b, a]
        }
        pub(crate) fn rgba_lib_bg(&self) -> [u8; 4] {
            let a = ((self.base_200 >> 24) & 0xFF) as u8;
            let r = ((self.base_200 >> 16) & 0xFF) as u8;
            let g = ((self.base_200 >> 8) & 0xFF) as u8;
            let b = (self.base_200 & 0xFF) as u8;
            [r, g, b, a]
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
