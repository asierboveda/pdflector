// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Herramienta activa y borrado: pan de la herramienta mano (`begin_pan`, `set_pan`, `should_ignore_touch`), gestos de la herramienta boli/resaltador (`begin/update/end/cancel_tool_gesture`) y de la goma (`begin_erase_gesture`, `eraser_radius_px`, `update/end_erase_gesture`).

use super::Reader;
use crate::annotations::ERASE_HIT_RADIUS_PT;
use crate::annotations::ERASE_HL_PAD_PT;
use crate::annotations::ToolGesture;
use crate::annotations::ToolKind;
use log::{error, info};
use pdf_core::{Annotation, Gesture, Stroke};

trait InkOverlayOps {
    fn is_ready(&self) -> bool;
    fn render_segment(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width_px: f32,
        color_argb: i32,
    ) -> bool;
    fn commit(&mut self);
    fn cancel(&mut self);
    fn clear(&mut self);
}

impl InkOverlayOps for crate::jni::InkOverlay {
    fn is_ready(&self) -> bool {
        crate::jni::InkOverlay::is_ready(self)
    }

    fn render_segment(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width_px: f32,
        color_argb: i32,
    ) -> bool {
        crate::jni::InkOverlay::render_segment(self, x0, y0, x1, y1, width_px, color_argb)
    }

    fn commit(&mut self) {
        crate::jni::InkOverlay::commit(self);
    }

    fn cancel(&mut self) {
        crate::jni::InkOverlay::cancel(self);
    }

    fn clear(&mut self) {
        crate::jni::InkOverlay::clear(self);
    }
}

fn commit_used_ink_overlay<O: InkOverlayOps>(
    overlay: &mut O,
    overlay_used: bool,
    page: u32,
    annotation_id: u64,
) -> Option<(u32, u64)> {
    if !overlay_used {
        return None;
    }
    if overlay.is_ready() {
        overlay.commit();
        Some((page, annotation_id))
    } else {
        overlay.cancel();
        None
    }
}

fn cancel_used_ink_overlay<O: InkOverlayOps>(overlay: &mut O, overlay_used: bool) {
    if overlay_used {
        overlay.cancel();
    }
}

fn cancel_gesture_for_transition<O: InkOverlayOps>(
    gesture: &mut Option<ToolGesture>,
    overlay: Option<&mut O>,
) -> bool {
    let Some(gesture) = gesture.take() else {
        return false;
    };
    if let Some(overlay) = overlay {
        cancel_used_ink_overlay(overlay, gesture.ink_overlay_used);
    }
    true
}

fn clear_ink_overlay_for_reader_exit<O: InkOverlayOps>(
    gesture: &mut Option<ToolGesture>,
    mut overlay: Option<&mut O>,
    pending_ink_clear: &mut Option<(u32, u64)>,
) -> bool {
    let discarded_gesture = cancel_gesture_for_transition(gesture, overlay.as_deref_mut());
    if let Some(overlay) = overlay {
        overlay.clear();
    }
    *pending_ink_clear = None;
    discarded_gesture
}

fn submit_ink_overlay_segment_for_gesture<O: InkOverlayOps>(
    gesture: &mut ToolGesture,
    overlay: Option<&mut O>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    scale: f32,
) {
    if !gesture.ink_overlay_route {
        return;
    }
    let Some(engine) = gesture.ink_engine.as_ref() else {
        return;
    };
    let style = engine.style();
    let was_used = gesture.ink_overlay_used;
    let Some(overlay) = overlay else {
        return;
    };
    if !overlay.is_ready() {
        cancel_used_ink_overlay(overlay, was_used);
        gesture.ink_overlay_used = false;
        gesture.ink_overlay_route = false;
        return;
    }
    let color_argb = ((style.color.a as i32) << 24)
        | ((style.color.r as i32) << 16)
        | ((style.color.g as i32) << 8)
        | style.color.b as i32;
    if overlay.render_segment(x0, y0, x1, y1, style.width * scale, color_argb) {
        gesture.ink_overlay_used = true;
    } else {
        cancel_used_ink_overlay(overlay, true);
        gesture.ink_overlay_route = false;
        gesture.ink_overlay_used = false;
    }
}

/// Starts a tool gesture from the real Down sample.
#[inline]
fn new_tool_gesture(
    page: u32,
    tool: ToolKind,
    pt: (f32, f32),
    time_ns: u64,
    pressure: f32,
    ink_width: f32,
    ink_color: pdf_core::Color,
) -> Result<ToolGesture, crate::ink::InkError> {
    ToolGesture::try_new(page, tool, pt, time_ns, pressure, ink_width, ink_color)
}

impl Reader {
    /// Descarta cualquier tinta wet que siga en la superficie compartida al
    /// salir del documento. Las anotaciones secas y su sidecar permanecen.
    pub(crate) fn clear_ink_overlay_for_reader_exit(&mut self) {
        let discarded_gesture = clear_ink_overlay_for_reader_exit(
            &mut self.tool_gesture,
            self.ink_overlay.as_mut(),
            &mut self.pending_ink_clear,
        );
        if discarded_gesture && self.window.is_some() {
            self.mark_repaint();
        }
    }

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

    /// Pan RELATIVO por gesto de dos dedos (traslación pura dentro del
    /// pinch): suma el delta al pan actual y lo clampa a los bordes de la
    /// hoja (igual que el anclaje; sin clamp el contenido dejaría huecos).
    pub(crate) fn pan_by(&mut self, dx: f32, dy: f32) {
        let (dw, dh) = self.page_doc_size_px(self.page);
        if dw > 0.0 && dh > 0.0 {
            self.pan_x = Self::clamp_pan(self.pan_x + dx, dw * self.zoom, self.win_w as f32, false);
            self.pan_y = Self::clamp_pan(self.pan_y + dy, dh * self.zoom, self.win_h as f32, false);
            self.mark_repaint();
        }
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
    pub(crate) fn begin_tool_gesture(
        &mut self,
        sx: f32,
        sy: f32,
        tool: ToolKind,
        event_time_ns: i64,
        pressure: f32,
    ) -> bool {
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
        let Ok(time_ns) = u64::try_from(event_time_ns) else {
            return false;
        };
        let mut gesture = match new_tool_gesture(
            self.page,
            tool,
            pt,
            time_ns,
            pressure,
            self.ink_width,
            self.ink_color,
        ) {
            Ok(gesture) => gesture,
            Err(e) => {
                error!("begin ink gesture: {e}");
                return false;
            }
        };
        if tool == ToolKind::Ink {
            gesture.ink_overlay_route = self.ink_overlay.as_ref().is_some_and(|o| o.is_ready());
        }
        self.tool_gesture = Some(gesture);
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
        if tool == ToolKind::Ink {
            self.submit_ink_overlay_segment(sx, sy, sx, sy);
        }
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
    pub(crate) fn update_tool_gesture(
        &mut self,
        sx: f32,
        sy: f32,
        event_time_ns: i64,
        pressure: f32,
    ) {
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
                let Ok(time_ns) = u64::try_from(event_time_ns) else {
                    return;
                };
                let (accepted, previous) = {
                    let Some(g) = self.tool_gesture.as_mut() else {
                        return;
                    };
                    let previous = g
                        .ink_engine
                        .as_ref()
                        .and_then(crate::ink::CausalInkEngine::last_sample);
                    let accepted = g
                        .push_ink_sample(crate::ink::InkSample::new(pt.0, pt.1, time_ns, pressure))
                        .is_ok_and(|delta| delta.is_some());
                    (
                        accepted,
                        previous.map(|sample| (g.page, sample.x, sample.y)),
                    )
                };
                if !accepted {
                    return;
                }
                if let Some((page, px, py)) = previous {
                    let (x0, y0) = self.page_to_screen(page, px, py).unwrap_or((sx, sy));
                    self.submit_ink_overlay_segment(x0, y0, sx, sy);
                }
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

    fn page_to_screen(&self, page: u32, x: f32, y: f32) -> Option<(f32, f32)> {
        Some(self.page_screen_transform(page)?.page_to_screen(x, y))
    }

    fn submit_ink_overlay_segment(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let Some(page) = self.tool_gesture.as_ref().map(|gesture| gesture.page) else {
            return;
        };
        let scale = self.page_scale(page).unwrap_or(1.0);
        let Some(gesture) = self.tool_gesture.as_mut() else {
            return;
        };
        submit_ink_overlay_segment_for_gesture(
            gesture,
            self.ink_overlay.as_mut(),
            x0,
            y0,
            x1,
            y1,
            scale,
        );
    }

    /// Gesto de herramienta: al levantar crea la anotación persistida.
    ///
    /// - **Boli**: se persisten exactamente las muestras causales aceptadas
    ///   por el motor que alimentó Wet; no se filtran, interpolan ni predicen.
    /// - **Resaltador**: `pdf_core::highlight_under_gesture` selecciona las
    ///   líneas de texto bajo el trazo (extracción perezosa, solo ahora) y
    ///   crea el `Highlight` alineado al texto; "no text" si no hay líneas.
    ///
    /// El id nuevo se apunta en `session_ids` (historial de sesión).
    /// La muestra Up se añade antes de llamar aquí, después de drenar History.
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
        // escala efectiva del blit (contain × zoom).
        let scale = self.page_scale(g.page).unwrap_or(1.0);
        let min_d_pt = crate::TOOL_MIN_PX / scale;
        let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
        let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        match g.tool {
            ToolKind::Ink => {
                if let Some(engine) = g.ink_engine.as_ref() {
                    for sample in engine.active_samples() {
                        min_x = min_x.min(sample.x);
                        min_y = min_y.min(sample.y);
                        max_x = max_x.max(sample.x);
                        max_y = max_y.max(sample.y);
                    }
                }
            }
            ToolKind::Highlight => {
                for &(x, y) in &g.points {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
            ToolKind::Navigate => return,
        }
        if max_x - min_x < min_d_pt && max_y - min_y < min_d_pt {
            if g.ink_overlay_used
                && let Some(overlay) = self.ink_overlay.as_mut()
            {
                overlay.cancel();
            }
            if self.window.is_some() {
                self.mark_repaint();
            }
            return;
        }
        let mut g = g;
        match g.tool {
            ToolKind::Ink => {
                let final_stroke = g
                    .ink_engine
                    .as_mut()
                    .and_then(|engine| engine.finish().ok());
                if let Some(final_stroke) = final_stroke
                    && let Some(stroke) = Stroke::new(
                        final_stroke.points,
                        final_stroke.style.width,
                        final_stroke.style.color,
                    )
                    && let Some(id) = self
                        .annotations
                        .add(g.page as usize, Annotation::Stroke(stroke))
                {
                    self.session_ids.push(id);
                    self.save_annotations();
                    self.show_toast("ink");
                    if g.ink_overlay_used
                        && let Some(overlay) = self.ink_overlay.as_mut()
                    {
                        self.pending_ink_clear = commit_used_ink_overlay(overlay, true, g.page, id);
                    }
                } else if let Some(overlay) = self.ink_overlay.as_mut() {
                    cancel_used_ink_overlay(overlay, g.ink_overlay_used);
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
        if !cancel_gesture_for_transition(&mut self.tool_gesture, self.ink_overlay.as_mut()) {
            return;
        }
        if self.window.is_some() {
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
        self.mark_repaint();
        true
    }

    /// Radio del cursor de la goma en px (radio en puntos × escala efectiva).
    fn eraser_radius_px(&self) -> f32 {
        let scale = self.page_scale(self.page).unwrap_or(1.0);
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

        let pr = self.erase_last.unwrap_or(pt);
        let min_sweep_x = pt.0.min(pr.0);
        let max_sweep_x = pt.0.max(pr.0);
        let min_sweep_y = pt.1.min(pr.1);
        let max_sweep_y = pt.1.max(pr.1);

        let snapshot: Vec<u64> = self
            .annotations
            .for_page(self.page as usize)
            .iter()
            .map(|a| a.id)
            .collect();
        let mut changed = false;
        for id in snapshot {
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
                    let r = ERASE_HIT_RADIUS_PT + s.width / 2.0;
                    let min_x = min_sweep_x - r;
                    let max_x = max_sweep_x + r;
                    let min_y = min_sweep_y - r;
                    let max_y = max_sweep_y + r;

                    // Poda AABB: si la caja englobante del trazo no solapa el barrido, saltar
                    let mut s_min_x = f32::INFINITY;
                    let mut s_max_x = f32::NEG_INFINITY;
                    let mut s_min_y = f32::INFINITY;
                    let mut s_max_y = f32::NEG_INFINITY;
                    for &(x, y) in &s.points {
                        s_min_x = s_min_x.min(x);
                        s_max_x = s_max_x.max(x);
                        s_min_y = s_min_y.min(y);
                        s_max_y = s_max_y.max(y);
                    }
                    if s_min_x > max_x || s_max_x < min_x || s_min_y > max_y || s_max_y < min_y {
                        continue;
                    }

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
                    let min_x = min_sweep_x - ERASE_HL_PAD_PT;
                    let max_x = max_sweep_x + ERASE_HL_PAD_PT;
                    let min_y = min_sweep_y - ERASE_HL_PAD_PT;
                    let max_y = max_sweep_y + ERASE_HL_PAD_PT;

                    let overlaps = h.rects.iter().any(|r| {
                        let r_min_x = r.x.min(r.x + r.w);
                        let r_max_x = r.x.max(r.x + r.w);
                        let r_min_y = r.y.min(r.y + r.h);
                        let r_max_y = r.y.max(r.y + r.h);
                        !(r_min_x > max_x || r_max_x < min_x || r_min_y > max_y || r_max_y < min_y)
                    });
                    if !overlaps {
                        continue;
                    }

                    // Borrado completo de subrayado: tocar cualquier parte
                    // (o cruzarla con el barrido) elimina la anotación entera.
                    let hit = h.rects.iter().any(|r| {
                        let in_rect = |p: (f32, f32)| -> bool {
                            p.0 >= r.x - ERASE_HL_PAD_PT
                                && p.0 <= r.x + r.w + ERASE_HL_PAD_PT
                                && p.1 >= r.y - ERASE_HL_PAD_PT
                                && p.1 <= r.y + r.h + ERASE_HL_PAD_PT
                        };
                        if in_rect(pt) {
                            return true;
                        }
                        if let Some(pr) = self.erase_last {
                            const SWEEP_SAMPLES: usize = 8;
                            for i in 1..=SWEEP_SAMPLES {
                                let t = i as f32 / SWEEP_SAMPLES as f32;
                                let s = (pr.0 + (pt.0 - pr.0) * t, pr.1 + (pt.1 - pr.1) * t);
                                if in_rect(s) {
                                    return true;
                                }
                            }
                        }
                        false
                    });
                    if hit {
                        self.annotations.remove(id);
                        info!("erase: highlight {id} removed");
                        changed = true;
                    }
                }
                pdf_core::Annotation::TextNote(_) => {}
            }
        }
        self.erase_last = Some(pt);
        if changed {
            self.erase_dirty = true;
            if let Some(gpu) = self.gpu.as_mut() {
                gpu.invalidate_dry();
            }
        }
        self.mark_repaint();
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
        self.mark_repaint();
    }
}

#[cfg(test)]
mod ink_overlay_lifecycle_tests {
    use super::*;

    #[derive(Default)]
    struct FakeOverlay {
        ready: bool,
        render_ok: bool,
        commits: usize,
        cancels: usize,
        clears: usize,
        segments: usize,
        events: Vec<&'static str>,
    }

    impl InkOverlayOps for FakeOverlay {
        fn is_ready(&self) -> bool {
            self.ready
        }

        fn render_segment(
            &mut self,
            _x0: f32,
            _y0: f32,
            _x1: f32,
            _y1: f32,
            _width_px: f32,
            _color_argb: i32,
        ) -> bool {
            self.segments += 1;
            self.render_ok
        }

        fn commit(&mut self) {
            self.commits += 1;
        }

        fn cancel(&mut self) {
            self.cancels += 1;
            self.events.push("cancel");
        }

        fn clear(&mut self) {
            self.clears += 1;
            self.events.push("clear");
        }
    }

    fn ink_gesture() -> ToolGesture {
        ToolGesture::try_new(
            7,
            ToolKind::Ink,
            (10.0, 10.0),
            1,
            0.5,
            2.0,
            pdf_core::Color {
                r: 28,
                g: 32,
                b: 43,
                a: 255,
            },
        )
        .unwrap()
    }

    #[test]
    fn end_commits_used_overlay_and_tracks_page_and_annotation() {
        let mut overlay = FakeOverlay {
            ready: true,
            ..FakeOverlay::default()
        };

        let pending = commit_used_ink_overlay(&mut overlay, true, 7, 42);

        assert_eq!(pending, Some((7, 42)));
        assert_eq!(overlay.commits, 1);
        assert_eq!(overlay.cancels, 0);
    }

    #[test]
    fn cancel_discards_used_overlay_without_committing() {
        let mut overlay = FakeOverlay::default();

        cancel_used_ink_overlay(&mut overlay, true);

        assert_eq!(overlay.cancels, 1);
        assert_eq!(overlay.commits, 0);
    }

    #[test]
    fn overlay_losing_readiness_switches_gesture_to_native_wet_fallback() {
        let mut gesture = ink_gesture();
        gesture.ink_overlay_route = true;
        gesture.ink_overlay_used = true;
        let mut overlay = FakeOverlay {
            ready: false,
            ..FakeOverlay::default()
        };

        submit_ink_overlay_segment_for_gesture(
            &mut gesture,
            Some(&mut overlay),
            10.0,
            10.0,
            20.0,
            20.0,
            1.0,
        );

        assert!(!gesture.ink_overlay_route);
        assert!(!gesture.ink_overlay_used);
        assert_eq!(overlay.cancels, 1);
        assert_eq!(overlay.segments, 0);
    }

    #[test]
    fn transition_cancels_overlay_before_discarding_used_ink_gesture() {
        let mut gesture = ink_gesture();
        gesture.ink_overlay_used = true;
        let mut gesture = Some(gesture);
        let mut overlay = FakeOverlay::default();

        let discarded = cancel_gesture_for_transition(&mut gesture, Some(&mut overlay));

        assert!(discarded);
        assert!(gesture.is_none());
        assert_eq!(overlay.cancels, 1);
        assert_eq!(overlay.commits, 0);
    }

    #[test]
    fn reader_exit_clears_overlay_after_gesture_has_ended() {
        let mut gesture = None;
        let mut overlay = FakeOverlay::default();
        let mut pending = Some((7, 42));

        clear_ink_overlay_for_reader_exit(&mut gesture, Some(&mut overlay), &mut pending);

        assert!(gesture.is_none());
        assert_eq!(pending, None);
        assert_eq!(overlay.clears, 1);
        assert_eq!(overlay.cancels, 0);
        assert_eq!(overlay.commits, 0);
        assert_eq!(overlay.events, ["clear"]);
    }

    #[test]
    fn reader_exit_cancels_active_overlay_before_clearing_it() {
        let mut gesture = ink_gesture();
        gesture.ink_overlay_used = true;
        let mut gesture = Some(gesture);
        let mut overlay = FakeOverlay::default();
        let mut pending = None;

        clear_ink_overlay_for_reader_exit(&mut gesture, Some(&mut overlay), &mut pending);

        assert!(gesture.is_none());
        assert_eq!(pending, None);
        assert_eq!(overlay.cancels, 1);
        assert_eq!(overlay.clears, 1);
        assert_eq!(overlay.commits, 0);
        assert_eq!(overlay.events, ["cancel", "clear"]);
    }
}
