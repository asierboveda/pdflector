// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Sheet de ajustes, chrome del visor y temas (extraído de `reader.rs`, 2026-09-06): animación/arrastre del sheet (`sheet_animating`, `begin/drag/end_sheet_drag`, `hide/toggle_sheet`, `sheet_hide_now`), barras de chrome con auto-hide (`show/hide/toggle/touch_chrome`) y el cambio de tema (`set_theme`, `cycle_theme`, `toggle_dark`).

use super::Reader;
use super::geometry::sheet_h;
use crate::draw::render_sheet;
use crate::theme;
use log::info;
use std::time::Duration;
use std::time::Instant;

// ---------------------------------------------------------------------
// Sheet de ajustes (panel deslizante desde arriba, 2026-08-XX)
// ---------------------------------------------------------------------
impl Reader {
    /// ¿Animación del sheet en vuelo? La consulta global de trabajo diferido
    /// es `needs_tick` (incluye esta señal + portadas + long-press + aviso
    /// breve); `sheet_animating` ya no se usa desde `lib` (2026-08-XX).
    #[allow(dead_code)]
    pub(crate) fn sheet_animating(&self) -> bool {
        self.sheet_anim
    }

    /// Comienza el arrastre del sheet (dedo deslizándose): deja de animar y
    /// deja que el dedo controle `sheet_progress` directamente.
    pub(crate) fn begin_sheet_drag(&mut self) {
        self.sheet_anim = false;
    }

    /// Arrastre del sheet: `dy` = desplazamiento vertical del dedo desde el
    /// Down (px, positivo = hacia abajo). El progreso sigue al dedo
    /// (`dy / alto del sheet`, recortado a [0, 1]) y se redibuja el frame.
    /// El redraw es BARATO con el sheet visible: `blit` usa el frame de
    /// copiar el overlay del sheet sobre el frame, NO
    /// re-blitea la página completa (ver `blit`) — la animación del sheet
    /// no degrada el frame time.
    pub(crate) fn drag_sheet(&mut self, dy: f32) {
        let h = sheet_h(self.win_h).max(1) as f32;
        self.sheet_progress = (dy / h).clamp(0.0, 1.0);
        if self.sheet_progress > 0.0 && self.sheet_bitmap.is_none() {
            // El sheet se empieza a ver: materializar su bitmap cacheado.
            self.sheet_bitmap = render_sheet(self);
            self.sheet_id = self.next_ovl_id();
        }
        self.redraw();
    }

    /// Fin del arrastre: anima el sheet hasta el objetivo más cercano
    /// (abierto si `progress >= 0.5`, cerrado si no).
    pub(crate) fn end_sheet_drag(&mut self) {
        self.sheet_open = self.sheet_progress >= 0.5;
        self.sheet_anim = true;
    }

    /// Cierra el sheet CON animación (tap fuera del panel): no toca el
    /// progreso actual; `tick` lo anima hasta 0 (el tap en el documento no
    /// cambia de página a propósito: cerrar el panel no debe avanzar).
    pub(crate) fn hide_sheet(&mut self) {
        if self.sheet_progress > 0.0 {
            self.sheet_open = false;
            self.sheet_anim = true;
        }
    }

    /// Abre/cierra el sheet CON animación (tap en la barra superior del
    /// chrome): `tick` anima el progreso y `blit` materializa el bitmap al
    /// hacerse visible. Sustituye al antiguo gesto pull-down (eliminado).
    pub(crate) fn toggle_sheet(&mut self) {
        if self.sheet_progress > 0.0 || self.sheet_open {
            self.hide_sheet();
        } else {
            self.sheet_open = true;
            self.sheet_anim = true;
        }
    }

    /// Oculta el sheet INMEDIATAMENTE (sin animación): al entrar en la
    /// biblioteca o al abrir otro documento el estado del visor se reinicia.
    pub(crate) fn sheet_hide_now(&mut self) {
        self.sheet_open = false;
        self.sheet_progress = 0.0;
        self.sheet_anim = false;
        self.sheet_bitmap = None;
    }

    /// Muestra el chrome del visor (barra superior e inferior) y programa el auto-hide a 4.5s.
    pub(crate) fn show_chrome(&mut self) {
        self.chrome_visible = true;
        self.chrome_hide_at = Some(Instant::now() + Duration::from_millis(4500));
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.redraw();
    }

    /// Oculta el chrome del visor.
    pub(crate) fn hide_chrome(&mut self) {
        self.chrome_visible = false;
        self.chrome_hide_at = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.redraw();
    }

    /// Alterna la visibilidad del chrome del visor.
    pub(crate) fn toggle_chrome(&mut self) {
        if self.chrome_visible {
            self.hide_chrome();
        } else {
            self.show_chrome();
        }
    }

    /// Resetea el temporizador de auto-ocultado del chrome (al tocar un control).
    pub(crate) fn touch_chrome(&mut self) {
        if self.chrome_visible {
            self.chrome_hide_at = Some(Instant::now() + Duration::from_millis(4500));
        }
    }

    /// Establece un tema específico. Invalida cachés y persiste estado.
    pub(crate) fn set_theme(&mut self, theme: crate::theme::AppTheme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.dark = self.theme.is_dark();
        self.page_badge = None;
        self.sheet_bitmap = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.library.lib_header = None;
        self.library.lib_band = None;
        self.toast_bitmap = None;
        self.sel_menu = None;
        info!("theme set to {:?} (dark: {})", self.theme, self.dark);
        self.save_state();
        self.redraw();
    }

    /// Cicla el tema activo (DefaultLight → SepiaLight → DefaultDark → SepiaDark → DefaultLight).
    /// Invalida las caches de bitmaps y persiste el nuevo tema.
    pub(crate) fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
        self.dark = self.theme.is_dark();
        self.page_badge = None;
        self.sheet_bitmap = None;
        self.chrome_top_bitmap = None;
        self.chrome_bottom_bitmap = None;
        self.library.lib_header = None;
        self.library.lib_band = None;
        self.toast_bitmap = None;
        self.sel_menu = None;
        info!("theme cycled to {:?} (dark: {})", self.theme, self.dark);
        self.save_state();
        self.redraw();
    }

    /// Alterna el modo oscuro (cicla al tema siguiente).
    #[allow(dead_code)]
    pub(crate) fn toggle_dark(&mut self) {
        self.cycle_theme();
    }
}
