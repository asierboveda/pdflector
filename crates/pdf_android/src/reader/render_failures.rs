// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Páginas que MuPDF no pudo renderizar (PR-04): política PURA de
//! reintento con espera exponencial acotada (el tiempo se inyecta; sin
//! reloj de pared en los tests). Sin ella el modo Scroll volvía a pedir la
//! hoja rota en cada tick (`scroll_ensure_rendered`), ocupando el worker
//! en bucle. Mientras la hoja está en error la columna pinta un marcador
//! de error en su ranura; un render posterior correcto lo borra.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Espera tras el primer fallo; se dobla en cada fallo seguido.
pub(crate) const RETRY_BASE: Duration = Duration::from_millis(500);
/// Tope de la espera entre reintentos.
pub(crate) const RETRY_MAX: Duration = Duration::from_secs(30);

/// Espera antes de reintentar una hoja que lleva `failures` fallos
/// seguidos (≥ 1).
pub(crate) fn retry_delay(_failures: u32) -> Duration {
    Duration::ZERO
}

/// Qué pinta la columna en la ranura de una hoja visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlotDraw {
    /// Su bitmap residente (aunque un re-render posterior fallara).
    Bitmap,
    /// Marcador de error: sin bitmap y el último render falló.
    Error,
    /// Aún sin render: el fondo (llega del worker).
    Blank,
}

/// Decide qué pintar en la ranura de una hoja.
pub(crate) fn slot_draw(_cached: bool, _failed: bool) -> SlotDraw {
    SlotDraw::Blank
}

/// Registro de fallos por página: (fallos seguidos, próximo reintento).
#[derive(Clone, Debug, Default)]
pub(crate) struct RenderFailures {
    pages: HashMap<u32, (u32, Instant)>,
}

impl RenderFailures {
    /// El render de `page` falló en `now`.
    pub(crate) fn fail(&mut self, _page: u32, _now: Instant) {}

    /// El render de `page` llegó bien: olvida sus fallos.
    pub(crate) fn succeed(&mut self, _page: u32) {}

    /// ¿Está `page` en error (último render fallido)?
    pub(crate) fn is_failed(&self, _page: u32) -> bool {
        false
    }

    /// ¿Se puede volver a pedir `page` en `now`?
    pub(crate) fn can_request(&self, _page: u32, _now: Instant) -> bool {
        true
    }

    /// Filtra un plan de render: quita las hojas aún en espera.
    pub(crate) fn requestable(&self, plan: Vec<u32>, _now: Instant) -> Vec<u32> {
        plan
    }

    /// Documento nuevo: olvida todos los fallos.
    pub(crate) fn clear(&mut self) {
        self.pages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn retry_delay_doubles_up_to_a_cap() {
        assert_eq!(retry_delay(1), 500 * MS);
        assert_eq!(retry_delay(2), 1000 * MS);
        assert_eq!(retry_delay(3), 2000 * MS);
        assert_eq!(retry_delay(7), RETRY_MAX); // 32 s → tope 30 s
        assert_eq!(retry_delay(60), RETRY_MAX); // sin desbordar
        assert_eq!(retry_delay(u32::MAX), RETRY_MAX);
    }

    #[test]
    fn a_failed_page_is_not_requested_again_until_its_backoff_passes() {
        let t0 = Instant::now();
        let mut f = RenderFailures::default();
        assert!(f.can_request(3, t0));
        f.fail(3, t0);
        assert!(f.is_failed(3));
        // Ningún tick (8-16 ms) la vuelve a pedir durante la espera.
        for k in 0..30 {
            assert!(!f.can_request(3, t0 + 16 * MS * k));
        }
        assert!(f.can_request(3, t0 + 500 * MS));
        // Otra hoja no se ve afectada.
        assert!(f.can_request(4, t0) && !f.is_failed(4));
        // Falla otra vez: espera el doble.
        let t1 = t0 + 500 * MS;
        f.fail(3, t1);
        assert!(!f.can_request(3, t1 + 999 * MS));
        assert!(f.can_request(3, t1 + 1000 * MS));
        assert_eq!(
            f.requestable(vec![2, 3, 4], t1 + 10 * MS),
            vec![2, 4],
            "el plan no incluye la hoja en espera"
        );
        assert_eq!(f.requestable(vec![2, 3, 4], t1 + 1000 * MS), vec![2, 3, 4]);
    }

    #[test]
    fn a_later_successful_render_clears_the_error() {
        let t0 = Instant::now();
        let mut f = RenderFailures::default();
        f.fail(5, t0);
        f.fail(5, t0 + 600 * MS);
        assert!(f.is_failed(5) && !f.can_request(5, t0 + 600 * MS));
        f.succeed(5);
        assert!(!f.is_failed(5));
        assert!(f.can_request(5, t0 + 600 * MS));
        // Un fallo nuevo tras el éxito vuelve a empezar por la espera base.
        let t2 = t0 + 700 * MS;
        f.fail(5, t2);
        assert!(!f.can_request(5, t2 + 499 * MS));
        assert!(f.can_request(5, t2 + 500 * MS));
        f.fail(9, t0);
        f.clear();
        assert!(!f.is_failed(9) && f.can_request(9, t0));
    }

    #[test]
    fn error_placeholder_only_without_a_bitmap() {
        assert_eq!(slot_draw(false, true), SlotDraw::Error);
        assert_eq!(slot_draw(true, true), SlotDraw::Bitmap);
        assert_eq!(slot_draw(true, false), SlotDraw::Bitmap);
        assert_eq!(slot_draw(false, false), SlotDraw::Blank);
    }
}
