// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Core Design Tokens (Fase 2 of UI/UX Plan)
//!
//! Exposes exact, strict color definitions for the Design System:
//! - Modo Papel (Notion / Lovable inspired)
//! - Modo Noche (Vercel Dark inspired)
//!
//! This module is framework-agnostic. The UI shells (egui, slint) consume
//! these tokens to style their components.

use pdf_core::annotations::Color;

/// The active theme mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Paper,
    Night,
}

/// A parsed RGBA color ready for UI usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ThemeColor {
    /// Creates a new `ThemeColor` from RGBA bytes.
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Creates a `ThemeColor` from a 24-bit RGB hex code (e.g. `0xF6F5F4`), setting alpha to 255.
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xFF) as u8,
            g: ((hex >> 8) & 0xFF) as u8,
            b: (hex & 0xFF) as u8,
            a: 255,
        }
    }
}

impl From<ThemeColor> for Color {
    fn from(c: ThemeColor) -> Self {
        Color {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }
}

/// The core design tokens (colors and styles) of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignTokens {
    /// App background.
    pub canvas: ThemeColor,
    /// Panels and floating surfaces.
    pub surface: ThemeColor,
    /// Hairline borders for surfaces.
    pub border: ThemeColor,
    /// UI text and icons.
    pub ink: ThemeColor,
    /// Active buttons, selections.
    pub accent: ThemeColor,
    /// Highlighter: Yellow.
    pub highlight_yellow: ThemeColor,
    /// Highlighter: Green.
    pub highlight_green: ThemeColor,
    /// Highlighter: Blue.
    pub highlight_blue: ThemeColor,
}

impl DesignTokens {
    /// Modo Papel: Inspiración Notion / Lovable.
    /// Emula tinta sobre papel, blanco cálido, sin fatiga.
    pub const PAPER: Self = Self {
        canvas: ThemeColor::from_hex(0xF6F5F4),
        surface: ThemeColor::from_hex(0xFFFFFF),
        border: ThemeColor::from_hex(0xE5E5E6),
        ink: ThemeColor::from_hex(0x1C1C1C),
        accent: ThemeColor::from_hex(0x0075DE),
        highlight_yellow: ThemeColor::from_hex(0xFDECC8),
        highlight_green: ThemeColor::from_hex(0xDBEDDB),
        highlight_blue: ThemeColor::from_hex(0xD3E5EF),
    };

    /// Modo Noche: Inspiración Vercel Dark.
    /// Lectura a oscuras y paneles OLED.
    pub const NIGHT: Self = Self {
        canvas: ThemeColor::from_hex(0x000000),
        surface: ThemeColor::from_hex(0x0A0A0A),
        border: ThemeColor::from_hex(0x1A1A1A),
        ink: ThemeColor::from_hex(0xEDEDED),
        accent: ThemeColor::from_hex(0x0070F3),
        // Las notas del plan actual no especifican colores de subrayado oscuros,
        // usamos los de papel por ahora, o bien en el renderizado oscuro se
        // invierten o filtran (ej. usando un blend mode distinto).
        highlight_yellow: ThemeColor::from_hex(0xFDECC8),
        highlight_green: ThemeColor::from_hex(0xDBEDDB),
        highlight_blue: ThemeColor::from_hex(0xD3E5EF),
    };

    /// Devuelve los tokens correspondientes al modo especificado.
    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Paper => Self::PAPER,
            ThemeMode::Night => Self::NIGHT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests de caracterización: fijan los valores públicos actuales de los
    // tokens para que mover el módulo no cambie nada observable.

    #[test]
    fn from_hex_descompone_rgb_y_alfa_opaco() {
        assert_eq!(
            ThemeColor::from_hex(0x0075DE),
            ThemeColor::new(0x00, 0x75, 0xDE, 255)
        );
        assert_eq!(
            ThemeColor::from_hex(0xFF_123456),
            ThemeColor::new(0x12, 0x34, 0x56, 255)
        );
    }

    #[test]
    fn theme_color_se_convierte_a_color_de_anotacion() {
        let c: Color = ThemeColor::new(1, 2, 3, 4).into();
        assert_eq!((c.r, c.g, c.b, c.a), (1, 2, 3, 4));
    }

    #[test]
    fn tokens_papel_tienen_los_valores_esperados() {
        let t = DesignTokens::PAPER;
        assert_eq!(t.canvas, ThemeColor::from_hex(0xF6F5F4));
        assert_eq!(t.surface, ThemeColor::from_hex(0xFFFFFF));
        assert_eq!(t.border, ThemeColor::from_hex(0xE5E5E6));
        assert_eq!(t.ink, ThemeColor::from_hex(0x1C1C1C));
        assert_eq!(t.accent, ThemeColor::from_hex(0x0075DE));
        assert_eq!(t.highlight_yellow, ThemeColor::from_hex(0xFDECC8));
        assert_eq!(t.highlight_green, ThemeColor::from_hex(0xDBEDDB));
        assert_eq!(t.highlight_blue, ThemeColor::from_hex(0xD3E5EF));
    }

    #[test]
    fn tokens_noche_tienen_los_valores_esperados() {
        let t = DesignTokens::NIGHT;
        assert_eq!(t.canvas, ThemeColor::from_hex(0x000000));
        assert_eq!(t.surface, ThemeColor::from_hex(0x0A0A0A));
        assert_eq!(t.border, ThemeColor::from_hex(0x1A1A1A));
        assert_eq!(t.ink, ThemeColor::from_hex(0xEDEDED));
        assert_eq!(t.accent, ThemeColor::from_hex(0x0070F3));
        assert_eq!(t.highlight_yellow, ThemeColor::from_hex(0xFDECC8));
        assert_eq!(t.highlight_green, ThemeColor::from_hex(0xDBEDDB));
        assert_eq!(t.highlight_blue, ThemeColor::from_hex(0xD3E5EF));
    }

    #[test]
    fn for_mode_elige_los_tokens_del_modo() {
        assert_eq!(
            DesignTokens::for_mode(ThemeMode::Paper),
            DesignTokens::PAPER
        );
        assert_eq!(
            DesignTokens::for_mode(ThemeMode::Night),
            DesignTokens::NIGHT
        );
    }
}
