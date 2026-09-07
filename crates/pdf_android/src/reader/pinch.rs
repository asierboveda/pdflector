// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Zoom/pinch (extraído de `reader.rs`, 2026-09-06): anclaje del gesto (`PinchAnchor`, `begin_pinch`) y zoom rápido/final (`set_zoom_fast`, `set_zoom_sharp`) con la fórmula de anclaje y sus clamps (`anchor_pan`, `clamp_pan`).

use super::Reader;
use crate::PINCH_MAX;
use crate::PINCH_MIN;
use log::info;
use std::time::Instant;

/// Anclaje del pinch en curso: estado que `begin_pinch` captura al caer el
/// segundo dedo y que `set_zoom_fast` usa para recalcular el pan que deja
/// fijo el punto de documento bajo el centro del pinch (ver `anchor_pan`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PinchAnchor {
    /// Centro del pinch en px de ventana (punto de pantalla que permanece
    /// fijo bajo los dedos durante el gesto).
    ax: f32,
    ay: f32,
    /// Zoom al iniciar el gesto (base del factor relativo y del anclaje).
    z0: f32,
    /// Pan de partida (del gesto anterior, 0 si no lo hay): el anclaje es
    /// continuo con el estado previo (a `z == z0` el pan no cambia).
    pan_x0: f32,
    pan_y0: f32,
}

impl Reader {
    /// Fórmula de anclaje del pinch: el pan (px) que, a zoom `z`, deja fijo
    /// en pantalla el punto de documento que estaba bajo el ancla al iniciar
    /// el gesto. `base0`/`base` son la posición de origen del bitmap escalado
    /// al zoom de partida (`z0`) y al zoom actual (`z`) — centrado horizontal
    /// `centered_base` o 0 en Y — y `pan0` el pan de partida.
    ///
    /// Derivación del anclaje. El mapeo pantalla de un punto de documento `q`
    /// (px a zoom 1) es `screen(q, z) = base(z) + pan(z) + q·z`, con
    /// `base(z)` la posición de origen del bitmap escalado:
    ///
    /// - el punto bajo el ancla al iniciar el gesto es
    ///   `q = (ancla − base(z0) − pan0) / z0` y no cambia durante el gesto;
    /// - imponiendo `screen(q, z) == ancla` queda la fórmula:
    ///
    /// ```text
    /// pan(z) = ancla − base(z) − q·z,   con q = (ancla − base(z0) − pan0) / z0
    /// ```
    ///
    /// Propiedades: a `z == z0` devuelve `pan0` (continuidad con el pan del
    /// gesto anterior); como `base(z)` es lineal en `z`, el pan es una función
    /// lineal del zoom (sin saltos entre Moves); al soltar el pinch
    /// (`set_zoom_sharp` re-renderiza a `rendered_zoom = zoom` y `blit_zoom`
    /// vuelve a 1.0) la escala efectiva `doc·z` no cambia, así que el mismo
    /// pan mantiene el anclaje sin que la página salte.
    fn anchor_pan(anchor: f32, base0: f32, base: f32, z0: f32, pan0: f32, z: f32) -> f32 {
        let q = (anchor - base0 - pan0) / z0;
        anchor - base - q * z
    }

    /// Clamp del pan de anclaje a los bordes de la hoja (modo UNA HOJA): con
    /// zoom-in la página se puede mover HASTA sus bordes pero un borde de la
    /// hoja NUNCA entra dentro de la ventana (nada de fondo visible alrededor
    /// de la página). `page` es el tamaño de la página en pantalla (`dw·zoom`
    /// o `dh·zoom`) y `win` el tamaño de la ventana en ese eje (f32).
    ///
    /// Geometría real del blit (`blit`): la página ocupa en pantalla
    /// `[base + pan, base + pan + page]`, con `base` la posición de origen
    /// del bitmap escalado SIN pan — centrada en X (`centered_base`,
    /// `align_top = false`) y alineada al borde superior en Y (`base = 0`,
    /// `align_top = true`; el anclaje Y real del pinch es "arriba",
    /// confirmado en `blit`: `dy = pan_y`).
    ///
    /// - Si `page >= win` (la página es más grande que la ventana): exige
    ///   cubrirla entera, `base + pan <= 0` y `base + pan + page >= win`, o
    ///   sea `pan ∈ [win − page − base, −base]`. En Y (`base = 0`) queda
    ///   `pan.clamp(win − page, 0)`; en X el rango se desplaza por el
    ///   centrado de `centered_base`: `[(win − page)/2, (page − win)/2]`.
    /// - Si `page < win` (página más pequeña; solo posible con zoom < 1):
    ///   centrada en X (el centrado ya lo hace `centered_base` → pan 0) y
    ///   arriba en Y (pan 0).
    fn clamp_pan(pan: f32, page: f32, win: f32, align_top: bool) -> f32 {
        if page >= win {
            let base = if align_top { 0.0 } else { (win - page) / 2.0 };
            pan.clamp(win - page - base, -base)
        } else {
            0.0
        }
    }

    /// Fija el anclaje del pinch en curso: el centro del pinch en px de
    /// ventana + el zoom y el pan de partida. `input` lo llama al caer el
    /// segundo dedo (PointerDown); `set_zoom_fast` recalcula después el pan
    /// para que el punto de documento bajo este ancla permanezca fijo.
    pub(crate) fn begin_pinch(&mut self, ax: f32, ay: f32) {
        self.pinch = Some(PinchAnchor {
            ax,
            ay,
            z0: self.zoom,
            pan_x0: self.pan_x,
            pan_y0: self.pan_y,
        });
    }

    /// Zoom DURANTE el pinch (fast): solo actualiza el factor (1.0 = página
    /// completa) y hace un redraw de solo blit — `blit` escala el bitmap
    /// cacheado de la página ACTUAL (la única hoja) con el zoom RELATIVO
    /// `zoom / rendered_zoom` (vecino-más-cercano), SIN re-renderizar MuPDF.
    /// El re-render nítido a la resolución final se hace UNA vez al soltar el
    /// pinch (`set_zoom_sharp`). El redraw normal (render + blit) no se usa
    /// aquí porque `ensure_pages_rendered` re-renderizaría en cada Move.
    ///
    /// El zoom es un factor RELATIVO a la distancia inicial del gesto
    /// (`zoom = z0 × dist / start_dist`, calculado por `input`); aquí solo se
    /// aplica. Además recalcula el pan de ANCLAJE: el punto de documento que
    /// estaba bajo el centro del pinch al iniciar (`begin_pinch`) se mantiene
    /// fijo en pantalla (ver `anchor_pan`) y se CLAMPEA a los bordes de la
    /// hoja (`clamp_pan`): un borde de la página nunca entra dentro de la
    /// ventana. Al hacer zoom-in solo se ve una porción de ESA página,
    /// recortada a sus bordes (nunca otra hoja: el blit solo dibuja la página
    /// actual).
    pub(crate) fn set_zoom_fast(&mut self, zoom: f32) {
        // F3.2: todo Move del pinch actualiza el instante del debounce.
        self.last_pinch_move = Some(Instant::now());
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        if (self.zoom - zoom).abs() < 1e-4 {
            return;
        }
        // De vuelta a zoom 1.0 (PINCH_MIN): la página vuelve a su posición
        // natural (centrada en X, alineada arriba en Y). BUG: sin este
        // reset, un pan residual de un zoom previo dejaba la vista
        // "colgada" en un offset (p. ej. el tercio inferior de la página,
        // pan_y ≈ −400 px con la página cover más alta que la ventana) sin
        // forma de corregirlo — la app NO tiene gesto de pan (el arrastre
        // se eliminó), así que el único "home" posible es el centrado. El
        // clamp de X ya fuerza ~0 (la página a cover casi iguala la
        // ventana), pero en Y el rango de `clamp_pan` admite offsets
        // grandes; por eso el reset es explícito en ambos ejes.
        if zoom <= PINCH_MIN + 1e-4 {
            self.pan_x = 0.0;
            self.pan_y = 0.0;
            self.zoom = zoom;
            if self.window.is_some() {
                self.blit();
            }
            return;
        }
        if let Some(p) = self.pinch {
            let (dw, dh) = self.page_doc_size_px(self.page);
            if dw > 0.0 && dh > 0.0 {
                // X: el bitmap escalado se centra en la ventana → base
                // dependiente del zoom. Y: el borde superior de la página
                // actual queda en el borde superior del viewport (modo UNA
                // HOJA, sin scroll) → base 0. Ambos se clampean después a los
                // bordes de la hoja (ver `clamp_pan`).
                self.pan_x = Self::clamp_pan(
                    Self::anchor_pan(
                        p.ax,
                        Self::centered_base(self.win_w, dw, p.z0),
                        Self::centered_base(self.win_w, dw, zoom),
                        p.z0,
                        p.pan_x0,
                        zoom,
                    ),
                    dw * zoom,
                    self.win_w as f32,
                    false,
                );
                self.pan_y = Self::clamp_pan(
                    Self::anchor_pan(p.ay, 0.0, 0.0, p.z0, p.pan_y0, zoom),
                    dh * zoom,
                    self.win_h as f32,
                    true,
                );
            }
        }
        self.zoom = zoom;
        // Redraw de solo blit: reutiliza los bitmaps de la caché (escala de la
        // última renderización, `rendered_zoom`); `blit` escala la página
        // actual con el zoom nuevo. El render y el reescalado de ventana los
        // cubre el bucle de eventos (RedrawNeeded/WindowResized) si hicieran
        // falta.
        // EARLY SHARP: si el vecino-más-cercano ya se ve borroso
        // (blit_zoom > 1.6), lanzar en el worker un render de la página a un
        // nivel 2^ceil(log2 zoom) ≤ 2× (clamp pequeño y rápido): al llegar,
        // `poll_render` fija `rendered_zoom` y el pinch sigue pero con el
        // bitmap NUEVO (nitidez progresiva sin esperar al soltar y SIN
        // renders gigantes por cada Move). Solo si no hay otro lote en vuelo.
        let blit_zoom = self.zoom / self.rendered_zoom.max(1e-4);
        if blit_zoom > 1.6 && !self.render_in_flight_for(self.zoom) {
            self.launch_render(vec![self.page], self.zoom, true);
        }
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Zoom FINAL del pinch (sharp): setea el factor (1.0 = página completa),
    /// conserva el pan de anclaje calculado por el último `set_zoom_fast` (el
    /// punto bajo los dedos no salta al re-renderizar) y re-renderiza la
    /// página actual UNA única vez a la escala continua resultante (render
    /// directo a resolución de pantalla — el camino medido más rápido en la
    /// tablet, ver nota de rendimiento en la cabecera de `lib.rs`): la caché
    /// se limpia y el redraw renderiza vía `ensure_pages_rendered`. Persiste
    /// el zoom (solo aquí, al soltar el gesto: `set_zoom_fast` es transitorio
    /// y escribir en cada Move de 60-120 Hz llenaría el disco).
    pub(crate) fn set_zoom_sharp(&mut self, zoom: f32) {
        // Fin del gesto (F3.2): el debounce deja de aplicar — aquí mismo se
        // lanza el render final nítido.
        self.last_pinch_move = None;
        let zoom = zoom.clamp(PINCH_MIN, PINCH_MAX);
        // Sin cambio REAL de zoom (p. ej. dos dedos tocando sin Moves, o un
        // pinch-in que se quedó en el mínimo): no re-renderizar — evita
        // limpiar la caché y pagar el render (~20-40 ms) por un no-op.
        if (self.zoom - zoom).abs() < 1e-4 && (self.rendered_zoom - zoom).abs() < 1e-4 {
            return;
        }
        let (dw, dh) = self.page_doc_size_px(self.page);
        if dw > 0.0 && dh > 0.0 {
            if zoom <= PINCH_MIN + 1e-4 {
                // De vuelta a zoom 1.0: posición natural (ver `set_zoom_fast`).
                self.pan_x = 0.0;
                self.pan_y = 0.0;
            } else {
                // Transición fast→sharp SIN salto: durante el pinch el bitmap
                // VIEJO se dibuja con tamaño `round(ancho_FULL del render viejo
                // × zoom/rendered_zoom)` px (vecino-más-cercano) y tras el
                // re-render el NUEVO a `round(dw × zoom)` px 1:1 — la
                // diferencia (≤ 1 px, por el redondeo de píxeles del render)
                // desplazaría el borde izquierdo de la página al soltar.
                // Corregimos el pan para que el borde DIBUJADO quede en el
                // mismo píxel: en `blit`, `dx = round((win − w)/2 + pan)`,
                // así que para que el nuevo dx iguale al dibujado en fast
                // basta `pan_nuevo = dx_fast − (win − w_nuevo)/2` (la
                // corrección es solo en X: en Y el borde superior es
                // `dy = round(pan_y)`, independiente del tamaño del bitmap).
                //
                // El ancho se toma de `full_w` del CachedPage, NO de
                // `bmp.width`: el bitmap cacheado es el CROP centrado a la
                // ventana del render full (fix de residency) y `bmp.width`
                // es la ventana, no la página. El anclaje/clamp del pinch
                // trabajan sobre la caja FULL (`dw·zoom`), y el crop centrado
                // se dibuja compensando exactamente ese centrado (ver
                // render_dry): alinear la caja full entre fast y sharp alinea
                // el CONTENIDO. Con `bmp.width` (crop) el dx_fast quedaría
                // desplazado `crop_x·blit_zoom` px → salto visible al soltar.
                let old_blit = self.zoom / self.rendered_zoom.max(1e-4);
                let old_w = match self.cache.peek(self.page) {
                    Some(b) => b.full_w as f32 * old_blit,
                    None => dw * zoom, // sin bitmap (defensa): sin corrección
                };
                let new_w = (dw as f64 * zoom as f64).round() as f32;
                let dx_fast = ((self.win_w as f32 - old_w) / 2.0 + self.pan_x).round();
                self.pan_x = dx_fast - (self.win_w as f32 - new_w) / 2.0;
            }
            // El pan de anclaje YA es el del zoom final (último
            // `set_zoom_fast`); el re-render a la nueva escala
            // (`rendered_zoom = zoom`) mantiene el mismo mapeo
            // documento→pantalla (la escala efectiva `doc·zoom` no cambia),
            // así que el punto bajo los dedos permanece fijo al soltar.
            // Reclamp del pan al zoom FINAL (por si `set_zoom_sharp` llega
            // sin un `set_zoom_fast` previo, p. ej. pinch sin Moves):
            // `clamp_pan` solo depende de `page = doc·zoom`, así que un pan
            // ya clampeado no cambia y el rango cubre la ventana entera
            // también tras el re-render (`rendered_zoom = zoom` → la escala
            // efectiva `doc·zoom` no cambia).
            self.pan_x = Self::clamp_pan(self.pan_x, dw * zoom, self.win_w as f32, false);
            self.pan_y = Self::clamp_pan(self.pan_y, dh * zoom, self.win_h as f32, true);
        }
        self.zoom = zoom;
        // SHARP ASÍNCRONO: NO se limpia la caché ni se re-renderiza en el
        // hilo UI (antes: `cache.clear() + redraw()` congelaba 20-400 ms). El
        // bitmap VIEJO sigue en caché y el blit lo escala a `zoom/rendered_zoom`
        // (preview vecino-más-cercano); el worker renderiza SOLO la página
        // actual (un render por lote: las vecinas se renderizan al navegar,
        // evita 3 renders gigantes de golpe) y, al llegar, `poll_render` fija
        // `rendered_zoom` y repintea (1:1 nítido). El presupuesto de píxeles
        // (launch_render) acota el bitmap para no petar la RAM.
        self.launch_render(vec![self.page], zoom, false);
        info!("zoom {:.3}", self.zoom);
        self.save_state();
        self.mark_repaint();
    }
}
