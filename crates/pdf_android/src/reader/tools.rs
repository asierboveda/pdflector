// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Herramienta activa y borrado (extraído de `reader.rs`, 2026-09-06): pan de la herramienta mano (`begin_pan`, `set_pan`, `should_ignore_touch`), gestos de la herramienta boli/resaltador (`begin/update/end/cancel_tool_gesture`) y de la goma (`begin_erase_gesture`, `eraser_radius_px`, `update/end_erase_gesture`).

use super::Reader;
use crate::annotations::ERASE_HIT_RADIUS_PT;
use crate::annotations::ERASE_HL_PAD_PT;
use crate::annotations::ToolGesture;
use crate::annotations::ToolKind;
use crate::view::initial_scale;
use log::info;
use pdf_core::{Annotation, Document, Gesture, Stroke};

impl Reader {
    /// Pan con DEDO (herramienta activa, modo mano): devuelve el pan de
    /// partida al bajar el dedo (mismo formato que `set_pan`).
    pub(crate) fn begin_pan(&self) -> (f32, f32) {
        (self.pan_x, self.pan_y)
    }

    /// Fija el pan (px) del documento y marca repintado — el blit por vsync
    /// aplicará el desplazamiento del frame al `ANativeWindow`.
    pub(crate) fn set_pan(&mut self, x: f32, y: f32) {
        self.pan_x = x;
        self.pan_y = y;
        self.mark_repaint();
    }

    /// ¿Debe ignorarse el táctil del dedo/palma por haber escrito hace poco con el stylus?
    pub(crate) fn should_ignore_touch(&self) -> bool {
        if let Some(t) = self.last_stylus_time {
            t.elapsed().as_millis() < crate::STYLUS_IGNORE_MS as u128
        } else {
            false
        }
    }

    /// Gesto de herramienta: el Down convierte el punto de pantalla a
    /// coordenadas de página y crea el `ToolGesture` en la página actual.
    /// El blit pasa a usar el frame compuesto + capa temporal (sin
    /// re-renderizar ni re-blitear la página) mientras el gesto dure.
    pub(crate) fn begin_tool_gesture(&mut self, sx: f32, sy: f32, tool: ToolKind) -> bool {
        if tool == ToolKind::Navigate {
            return false;
        }
        // B3: el resaltador pre-ordena los spans en el Down (una vez por
        // gesto) para el preview por present y el cálculo al soltar.
        let want_hl = tool == ToolKind::Highlight;
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return false;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        // Fase 1 USI: ancla temporal (event_time del Down, la fija `input`)
        // y presión inicial. Sin ancla (driver sin timestamps) → t0=0 y las
        // muestras degradan a la ventana sin Δt real (el predictor usa el
        // clamp de dt mínimo); sin presión → 0.5 (w_base neutro).
        let t0 = self.pending_t0_ns.take().unwrap_or(0);
        let pressure = self.pending_pressure.take().unwrap_or(0.5);
        self.tool_gesture = Some(ToolGesture::new(
            self.page,
            tool,
            pt,
            0.0,
            pressure,
            self.ink_width,
        ));
        // El Down ES t=0: el campo se fija tras crear el gesto.
        if let Some(g) = self.tool_gesture.as_mut() {
            g.times_ms[0] = 0.0;
        }
        if want_hl {
            let cached = self.text_cache.get(self.page);
            if let Some(pt) = cached {
                let mut v = pt.spans.clone();
                pdf_core::sort_spans_by_y(&mut v);
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.hl_spans = v;
                }
            }
        }
        self.gesture_t0_ns = t0;
        // FASE 2: la tinta es geometría GPU por frame — sin frame base que
        // clonar ni stamping. El siguiente blit pinta página + gesto.
        if self.window.is_some() {
            self.mark_repaint();
        }
        true
    }

    /// Gesto de herramienta: cada Move añade el punto (boli) o actualiza el
    /// rect (resaltador) y MARCA repintar — el blit real ocurre UNA vez por
    /// vsync en el bucle principal (coalescing de eventos, como Saber/Flutter):
    /// si bliteáramos por Move a 120 Hz, el BufferQueue de SurfaceFlinger
    /// (pantalla 60 Hz) bloquearía cada `unlock_and_post` ~16 ms (backpressure)
    /// → jitter/lag. Con un blit por vsync y dirty rect, el coste por frame es
    /// <1 ms y la latencia es ≤1 frame (16 ms).
    pub(crate) fn update_tool_gesture(&mut self, sx: f32, sy: f32, t_ms: f32, pressure: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        // La herramienta del gesto EN CURSO (la puso `input` según el modo
        // del boli); `self.tool` (barra) es irrelevante aquí.
        let Some(tool) = self.tool_gesture.as_ref().map(|g| g.tool) else {
            return;
        };
        // Actualizar tiempo de stylus para palm rejection por tiempo
        self.last_stylus_time = Some(std::time::Instant::now());
        match tool {
            ToolKind::Ink => {
                // Pipeline de modelado físico (google/ink-stroke-modeler):
                // 1. Sanitiza el evento (240 Hz, noise gate 0.2 pt).
                // 2. Simulación masa-resorte críticamente amortiguada (ζ = 1.0).
                // 3. Estimación y proyección cinemática Kalman a 25–30 ms.
                let Some(g) = self.tool_gesture.as_mut() else {
                    return;
                };
                let t_ns = self
                    .gesture_t0_ns
                    .saturating_add((t_ms as f64 * 1e6) as u64);
                let model_res = g.modeler.update(pt.0, pt.1, t_ns, pressure);
                g.predicted_pt = model_res.predicted_pt;
                let confirmed_pt = model_res.confirmed_pt;

                let Some(&last) = g.points.last() else {
                    return;
                };
                let n0 = g.points.len();
                g.push_with_pressure(confirmed_pt, t_ms, model_res.pressure);
                if g.points.len() == n0 {
                    return;
                }
                let mid = (
                    (last.0 + confirmed_pt.0) / 2.0,
                    (last.1 + confirmed_pt.1) / 2.0,
                );
                let prev_mid = g.prev_mid;
                // Muestrear en página la MISMA curva midpoint que dibuja el
                // present (una sola fuente de verdad para la polilínea).
                let a = prev_mid.unwrap_or(last);
                let steps = 6usize;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let om = 1.0 - t;
                    let q = if prev_mid.is_some() {
                        (
                            om * om * a.0 + 2.0 * om * t * last.0 + t * t * mid.0,
                            om * om * a.1 + 2.0 * om * t * last.1 + t * t * mid.1,
                        )
                    } else {
                        (a.0 + t * (mid.0 - a.0), a.1 + t * (mid.1 - a.1))
                    };
                    g.ink_pts.push(q);
                }
                g.prev_mid = Some(mid);
            }
            ToolKind::Highlight => {
                if let Some(g) = self.tool_gesture.as_mut() {
                    g.set_cur(pt);
                }
            }
            ToolKind::Navigate => {}
        }
        self.mark_repaint();
    }

    /// Gesto de herramienta: al levantar el dedo convierte el gesto en una
    /// anotación GUARDADA (persistida en el sidecar):
    ///
    /// - **Boli**: la polilínea MUESTREADA de la curva midpoint (`ink_pts`,
    ///   lo estampado en vivo) simplificada con Douglas-Peucker fino →
    ///   `Stroke` con el grosor/color actuales. Cero pop: el frame no se
    ///   re-pinta. Un gesto sin arrastre (un toque) se descarta.
    /// - **Resaltador**: `pdf_core::highlight_under_gesture` selecciona las
    ///   líneas de texto bajo el trazo (extracción perezosa, solo ahora) y
    ///   crea el `Highlight` alineado al texto; "no text" si no hay líneas.
    ///
    /// El id nuevo se apunta en `session_ids` (historial de sesión).
    /// `(sx, sy)` = posición del Up (remate M_last→P_up en el boli; las
    /// muestras de history ya se estamparon por el drain previo, así que el
    /// hueco que cierra es solo el último tramo hasta el punto de soltar).
    pub(crate) fn end_tool_gesture(&mut self, _sx: f32, _sy: f32) {
        let Some(g) = self.tool_gesture.take() else {
            return;
        };
        // Actualizar tiempo para palm rejection: tras soltar, se ignora el táctil un margen
        if g.tool != ToolKind::Navigate {
            self.last_stylus_time = Some(std::time::Instant::now());
        }
        // Gesto degenerado (un toque sin arrastre): descartar silenciosamente.
        // El umbral está en px de PANTALLA (TOOL_MIN_PX, el recorrido mínimo
        // del dedo/lápiz); el bbox del gesto en página se convierte con la
        // escala efectiva del blit (cover × zoom).
        let scale = self
            .doc
            .as_ref()
            .and_then(|d| d.page_size(g.page).ok())
            .map(|(pw, ph)| initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom)
            .unwrap_or(1.0);
        let min_d_pt = crate::TOOL_MIN_PX / scale;
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        for &(x, y) in &g.points {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        if max_x - min_x < min_d_pt && max_y - min_y < min_d_pt {
            // Un toque sin arrastre: sin gesto, sin anotación. La tinta que
            // el present dibujó era solo geometría del frame — muere sola.
            if self.window.is_some() {
                self.mark_repaint();
            }
            return;
        }
        let mut g = g;
        match g.tool {
            ToolKind::Ink => {
                // REMATE M_last→P_up: asienta la masa virtual con StrokeEndPredictor
                // y tapering suave de presión sin "cero-pop".
                let end_res = g.modeler.end_stroke();
                let end_pt = end_res.confirmed_pt;
                if let Some(m_last) = g.prev_mid.or_else(|| g.ink_pts.last().copied())
                    && m_last != end_pt
                {
                    g.ink_pts.push(end_pt);
                }
                // CERO POP: lo estampado en vivo ES el trazo final. Se
                // persiste la polilínea MUESTREADA de la curva midpoint
                // (`ink_pts`) simplificada con Douglas-Peucker fino
                // (ε 0.35 pt ≈ 0.7 px a 2 px/pt: replay < 1 px del vivo —
                // invisible). Sin Catmull-Rom ni re-rasterizado: la tinta del
                // frame no se toca.
                let sampled = if g.ink_pts.len() >= 40 {
                    pdf_core::simplify_polyline(&g.ink_pts, 0.35)
                } else {
                    g.ink_pts.clone()
                };
                if let Some(s) = Stroke::new(sampled, self.ink_width, self.ink_color)
                    && let Some(id) = self.annotations.add(g.page as usize, Annotation::Stroke(s))
                {
                    self.session_ids.push(id);
                    self.save_annotations();
                    self.show_toast("ink");
                }
            }
            ToolKind::Highlight => {
                // El resaltador usa TODO el trazo (los puntos del gesto), no
                // solo ancla→cursor: un trazo curvo selecciona las líneas
                // bajo su bbox completo.
                // B3: camino indexado sobre los spans del Down (sin I/O en
                // el gesto); fallback a la vía clásica si no estaban cacheados.
                let gesture = Gesture::Points(g.points.clone());
                let hl = if g.hl_spans.is_empty() {
                    let spans = self
                        .doc
                        .as_ref()
                        .and_then(|d| self.text_cache.get_or_extract(d, g.page).ok())
                        .map(|t| t.spans.clone())
                        .unwrap_or_default();
                    pdf_core::highlight_under_gesture(&spans, &gesture, pdf_core::HIGHLIGHT_COLOR)
                } else {
                    pdf_core::highlight_under_gesture_sorted(
                        &g.hl_spans,
                        &gesture,
                        pdf_core::HIGHLIGHT_COLOR,
                    )
                };
                if let Some(hl) = hl {
                    if let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Highlight(hl))
                    {
                        self.session_ids.push(id);
                        self.save_annotations();
                        self.show_toast("highlighted");
                    }
                } else {
                    // Sin líneas de texto bajo el trazo (zona en blanco o PDF
                    // escaneado): el resaltador dibuja un RECT LIBRE con el
                    // bbox del trazo (altura de línea ~13 pt) — el boli
                    // SIEMPRE pinta algo en modo Highlighter.
                    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
                    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
                    for &(x, y) in &g.points {
                        min_x = min_x.min(x);
                        min_y = min_y.min(y);
                        max_x = max_x.max(x);
                        max_y = max_y.max(y);
                    }
                    let line_h = 13.0f32;
                    let cy = (min_y + max_y) / 2.0 - line_h / 2.0;
                    let rect = pdf_core::Rect::new(min_x, cy, (max_x - min_x).max(1.0), line_h);
                    let hl = pdf_core::Highlight {
                        rects: vec![rect],
                        color: pdf_core::HIGHLIGHT_COLOR,
                    };
                    if let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Highlight(hl))
                    {
                        self.session_ids.push(id);
                        self.save_annotations();
                        self.show_toast("highlighted");
                    }
                }
            }
            ToolKind::Navigate => {}
        }
        // FASE 2: la tinta es geometría GPU — el próximo present pinta la
        // página con el Stroke ya persistido (sin frame base que restaurar).
        if self.window.is_some() {
            self.mark_repaint();
        }
    }

    /// Gesto de herramienta cancelado (segundo dedo, Cancel del sistema,
    /// ocultar la barra): descarta el trazo en curso sin crear anotación.
    pub(crate) fn cancel_tool_gesture(&mut self) {
        if self.tool_gesture.take().is_some() && self.window.is_some() {
            // El present GPU ya no dibuja el gesto: un frame normal.
            self.mark_repaint();
        }
    }

    // ---------------------------------------------------------------------
    // Borrado con el boli (botón DOWN mantenido + tocar el PDF): control
    // total SIN menús ([C] de la tarea). El erase nunca coexiste con un
    // gesto de tinta (`input` solo lo inicia si `tool_gesture` está libre) y
    // NO entra en el undo de sesión (`session_ids` intacto; decisión:
    // permanente).
    // ---------------------------------------------------------------------
    /// Comienza un gesto de borrado (Down del boli con el botón DOWN
    /// pulsado). Devuelve false si no se pudo iniciar (ya hay tinta en curso
    /// o el punto no cae en la página) — en ese caso `input` no entra en el
    /// modo Erase.
    pub(crate) fn begin_erase_gesture(&mut self, sx: f32, sy: f32) -> bool {
        // El modo ERASE nunca coexiste con `tool_gesture` de dibujo: si hay
        // tinta en curso, este Down no inicia borrado (el trazo sigue como
        // estaba).
        if self.tool_gesture.is_some() {
            return false;
        }
        if self.screen_to_page(sx, sy).is_none() {
            return false;
        }
        self.last_stylus_time = Some(std::time::Instant::now());
        self.erase_dirty = false;
        self.erase_last = None;
        self.erase_pt = Some((sx, sy));
        self.erase_r_px = self.eraser_radius_px();
        self.eraser_cursor = None;
        true
    }

    /// Radio del cursor de la goma en px (radio en puntos × escala efectiva).
    fn eraser_radius_px(&self) -> f32 {
        let scale = self
            .doc
            .as_ref()
            .and_then(|d| d.page_size(self.page).ok())
            .map(|(pw, ph)| initial_scale(pw, ph, self.win_w, self.win_h) * self.zoom)
            .unwrap_or(1.0);
        ERASE_HIT_RADIUS_PT * scale
    }

    /// Arrastre de borrado: hit-test del punto (en coords de página) contra
    /// TODAS las anotaciones de la página actual — incluidas las de
    /// SESIONES ANTERIORES (el sidecar se carga en `load_annotations`). Cada
    /// anotación cruzada se elimina EN VIVO (`AnnotationSet::remove`): el
    /// frame de la página se invalida y el siguiente blit la recompone sin
    /// ella — desaparece bajo el boli.
    pub(crate) fn update_erase_gesture(&mut self, sx: f32, sy: f32) {
        let Some(pt) = self.screen_to_page(sx, sy) else {
            return;
        };
        self.last_stylus_time = Some(std::time::Instant::now());
        // Cursor de la goma sigue al boli (el radio se calculó al empezar).
        self.erase_pt = Some((sx, sy));
        // Snapshot SOLO de IDS (sin clonar el `Annotation` completo): cada
        // id se resuelve contra el estado VIGENTE dentro del bucle (el set
        // puede mutar en iteraciones anteriores). Con history a 240 Hz (hasta
        // 16 muestras/evento) esto elimina el coste dominante del borrado:
        // antes se clonaba el set entero POR MUESTRA; ahora solo un Vec de
        // u64 por llamada.
        let snapshot: Vec<u64> = self
            .annotations
            .for_page(self.page as usize)
            .iter()
            .map(|a| a.id)
            .collect();
        let mut changed = false;
        for id in snapshot {
            // El kind se lee POR REFERENCIA en cada iteración (el set puede
            // haber mutado en las anteriores): CERO clones por muestra —
            // split_stroke/trim_highlight trabajan sobre &Stroke/&Highlight y
            // solo remove/add tocan el set (después del hit-test). Con 276
            // trazos y el boli tocando 0-2 por muestra, este bucle es memcpy
            // de ids + hit-tests, nada más.
            let Some(ann) = self
                .annotations
                .for_page(self.page as usize)
                .into_iter()
                .find(|a| a.id == id)
            else {
                continue; // anotación ya eliminada/repicada por el barrido
            };
            match &ann.kind {
                pdf_core::Annotation::Stroke(s) => {
                    // GOMA REAL sobre trazo: se recorta (parte la línea en
                    // trozos), no se elimina entera.
                    if let Some(parts) = pdf_core::annotations::split_stroke(
                        s,
                        pt,
                        ERASE_HIT_RADIUS_PT,
                        self.erase_last,
                    ) {
                        self.annotations.remove(id);
                        let mut kept = 0;
                        for part in parts {
                            if self
                                .annotations
                                .add(self.page as usize, pdf_core::Annotation::Stroke(part))
                                .is_some()
                            {
                                kept += 1;
                            }
                        }
                        info!("erase: stroke {id} -> {kept} piece(s)");
                        changed = true;
                    }
                }
                pdf_core::Annotation::Highlight(h) => {
                    // GOMA REAL sobre subrayado: se parte en rectos (con
                    // barrido: una goma rápida no salta la línea).
                    if let Some(rects) = pdf_core::annotations::trim_highlight(
                        h,
                        pt,
                        ERASE_HL_PAD_PT,
                        self.erase_last,
                    ) {
                        let color = h.color; // Copy: último uso del borrow del set
                        self.annotations.remove(id);
                        if !rects.is_empty() {
                            self.annotations.add(
                                self.page as usize,
                                pdf_core::Annotation::Highlight(pdf_core::Highlight {
                                    rects,
                                    color,
                                }),
                            );
                        }
                        info!("erase: highlight {id} trimmed");
                        changed = true;
                    }
                }
                pdf_core::Annotation::TextNote(_) => {}
            }
        }
        self.erase_last = Some(pt);
        if changed {
            self.erase_dirty = true;
            self.mark_repaint();
        }
    }

    /// Fin del borrado (Up o Cancel del sistema): persiste UNA sola vez si
    /// algo se eliminó (`store.save`, hilo de fondo).
    pub(crate) fn end_erase_gesture(&mut self) {
        self.last_stylus_time = Some(std::time::Instant::now());
        self.erase_pt = None;
        self.eraser_cursor = None;
        if self.erase_dirty {
            self.erase_dirty = false;
            self.save_annotations();
        }
    }
}
