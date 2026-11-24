// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Toast y panel de IA: aviso breve (`show_toast`) y "Preguntar a la IA" — `ask_ai` (hilo de fondo Gemini con imagen y contexto de la hoja), `ai_answer`, `rebuild_ai_panel`, `close_ai_panel`, `ai_scroll`.

use super::AiPanel;
use super::AiPhase;
use super::Reader;
use crate::draw::ai_panel_layout;
use crate::draw::render_ai_panel;
use log::info;
use std::time::Instant;

impl Reader {
    /// Aviso breve sobre el indicador de página ("copied", ...): texto +
    /// timestamp; `tick` lo expira a los `TOAST_MS` y el bitmap cacheado se
    /// invalida al cambiar el texto.
    pub(crate) fn show_toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
        self.toast_bitmap = None;
        self.redraw();
    }

    // ---------------------------------------------------------------------
    // "Preguntar a la IA" (Parte 2): hilo de fondo + IA híbrida + panel
    // ---------------------------------------------------------------------
    //
    // Al soltar la selección (`Reader::end_sel`) se llama a
    // `ask_ai`: se cierra el menú, se abre el panel en fase Asking
    // ("preguntando…") y se lanza un hilo de fondo (std::thread + mpsc, el
    // mismo patrón de `pdf_core::prefetch`) que llama a la IA y envía el
    // resultado por el canal. El hilo de UI sondea el canal en `tick`
    // (`try_recv`, sin bloquear) y al llegar el mensaje pasa el panel a
    // Answer (texto envuelto con scroll) o Error (mensaje claro en el mismo
    // panel). Decisiones:
    //
    // - GEMINI SIEMPRE: con IMAGEN de la selección (crop del bitmap
    //   cacheado, `sel_image_png_base64`) se llama a
    //   `GeminiClient::explain_image` con `GEMINI_MODEL`; la imagen es la
    //   fuente principal para ecuaciones/gráficos (el texto extraído de una
    //   ecuación es parcial o nulo). El prompt incluye además el texto de
    //   la selección y el de la hoja completa como contexto. Sin imagen, la
    //   misma explicación va solo con texto (`explain_text`).
    // - Respaldo: solo si Gemini responde ocupado (503/Busy), se prueba al
    //   momento con `GEMINI_FALLBACK_MODEL` DENTRO del hilo de fondo (nunca
    //   bloquea el hilo de UI); cualquier otro error se muestra en el panel.
    // - La key va EMBEBIDA en el APK (uso personal, sin telemetría; ver
    //   `lib.rs`). Una consulta no se cancela al cerrar el panel: el hilo
    //   termina solo y el resultado se descarta al soltar el receptor.
    // - El hilo de fondo evita bloquear el hilo de UI durante la red
    /// "IA": lanza la consulta a Gemini (imagen y contexto, o solo texto)
    /// en un hilo de fondo y abre el panel en fase "preguntando…". Si no hay
    /// ni texto ni imagen aprovechable avisa "no text" y no abre el panel
    /// (mismo comportamiento que Copiar; con imagen — PDF escaneado — sí
    /// abre: la imagen es la fuente principal). El texto y la imagen se
    /// capturan ANTES de cerrar el menú (`sel_text` y
    /// `sel_image_png_base64`).
    pub(crate) fn ask_ai(&mut self) {
        let text = self.sel_text();
        // Imagen de la selección (ecuaciones/gráficos): PNG base64 del crop
        // del bitmap cacheado. None si el crop no es posible (sin bitmap,
        // rect vacío) — en ese caso la explicación va solo con texto.
        let image = self.sel_image_png_base64();
        // Contexto: el texto de la hoja completa sitúa la selección (un
        // párrafo o una ecuación se entienden dentro de su hoja).
        let page_context = self.page_context_text();
        self.clear_selection(); // el panel sustituye a la selección
        if text.is_empty() && image.is_none() {
            self.show_toast("no text");
            return;
        }
        if crate::GOOGLE_API_KEY.trim().len() < 20 {
            self.ai_text =
                "Falta tu clave de Gemini: pégala en crates/pdf_android/google_key.txt y reinstala la app."
                    .to_string();
            self.ai_phase = AiPhase::Error;
            self.rebuild_ai_panel();
            self.redraw();
            return;
        }
        info!(
            "ask_ai: {} chars, image {} B PNG, context {} chars -> Gemini ({})",
            text.chars().count(),
            image.as_ref().map_or(0, String::len),
            page_context.chars().count(),
            crate::GEMINI_MODEL
        );
        let mut prompt = "Eres un asistente de estudio. Explica de forma clara y concisa, en español, el contenido seleccionado por el estudiante dentro del contexto de su hoja. Si es una ecuación o un gráfico, explica qué significa y para qué sirve. No te limites a repetir el texto: explícalo. Escribe las fórmulas en notación sencilla y legible sin LaTeX complejo (por ejemplo x^2, a/b, raíz(x)) y usa párrafos cortos.".to_string();
        if !text.is_empty() {
            prompt.push_str("\n\nTexto de la selección:\n");
            prompt.push_str(&text);
        }
        if !page_context.is_empty() && page_context != text {
            prompt.push_str("\n\nTexto de la hoja (contexto):\n");
            prompt.push_str(&page_context);
        }
        if image.is_some() {
            prompt.push_str("\n\nLa imagen adjunta es la selección exacta del estudiante: es la fuente principal de lo que hay que explicar.");
        }
        // Panel en fase Asking ("preguntando…") y hilo de fondo con la
        // llamada HTTP: el UI nunca espera por la red.
        self.ai_text = "preguntando…".to_string();
        self.ai_phase = AiPhase::Asking;
        self.rebuild_ai_panel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let client =
                pdf_core::ai::GeminiClient::with_model(crate::GOOGLE_API_KEY, crate::GEMINI_MODEL);
            let attempt = |client: &pdf_core::ai::GeminiClient| match &image {
                Some(b64) => client.explain_image(&prompt, b64),
                None => client.explain_text(&prompt),
            };
            let result = match attempt(&client) {
                Ok(answer) => Ok(answer),
                // Si el modelo principal está ocupado (503), se prueba al
                // momento con el modelo de respaldo; esperar y repetir con
                // el mismo modelo saturado solo añade latencia.
                Err(pdf_core::AiError::Busy(_)) => {
                    let fallback = pdf_core::ai::GeminiClient::with_model(
                        crate::GOOGLE_API_KEY,
                        crate::GEMINI_FALLBACK_MODEL,
                    );
                    attempt(&fallback)
                }
                Err(e) => Err(e),
            };
            // El error también viaja por el canal (`AiError` implementa
            // Display): el hilo de UI decide si es respuesta o error.
            let _ = tx.send(result);
        });
        self.ai_rx = Some(rx);
        self.redraw();
    }

    /// Aplica el resultado del hilo de IA al panel (fase Answer/Error) y
    /// libera el receptor (deja de sondear el canal y de pedir ticks).
    pub(crate) fn ai_answer(&mut self, text: String, phase: AiPhase) {
        self.ai_text = text;
        self.ai_phase = phase;
        self.ai_rx = None;
        self.rebuild_ai_panel();
        self.redraw();
    }

    /// (Re)construye el panel de IA con el texto y la fase actuales
    /// (`ai_text`/`ai_phase`): layout (`draw::ai_panel_layout`, incluye el
    /// envoltorio de líneas y los botones) + render del bitmap
    /// (`draw::render_ai_panel`). None si la ventana no está lista (se deja
    /// el panel anterior).
    fn rebuild_ai_panel(&mut self) {
        let Some(layout) = ai_panel_layout(self) else {
            return;
        };
        let Some(bitmap) = render_ai_panel(self) else {
            return;
        };
        let (mx, my, mrx, mry) = layout.rect;
        self.ai_panel = Some(AiPanel {
            x: mx as i32,
            y: my as i32,
            w: (mrx - mx) as i32,
            h: (mry - my) as i32,
            buttons: layout.buttons,
            bitmap,
            lines: layout.lines.len(),
            scroll: layout.scroll,
            visible: layout.visible,
            scrollable: layout.scrollable,
        });
        self.ai_panel_id = self.next_ovl_id(); // contenido nuevo del panel
    }

    /// Cierra el panel de IA (✕ o tap fuera): descarta el resultado
    /// pendiente si la consulta aún está en vuelo (el hilo de fondo termina
    /// solo y su mensaje se descarta al soltar el receptor).
    pub(crate) fn close_ai_panel(&mut self) {
        let had = self.ai_panel.is_some();
        self.ai_panel = None;
        self.ai_rx = None;
        self.ai_text = String::new();
        self.ai_phase = AiPhase::Asking;
        if had {
            self.redraw();
        }
    }

    /// Scroll del cuerpo del panel de IA (▲/▼, un paso = una línea): solo
    /// si el texto desborda (`scrollable`); re-renderiza el bitmap con la
    /// nueva ventana de líneas visibles.
    pub(crate) fn ai_scroll(&mut self, delta: i32) {
        let (scrollable, lines, visible, scroll) = match &self.ai_panel {
            Some(p) => (p.scrollable, p.lines, p.visible, p.scroll),
            None => return,
        };
        if !scrollable {
            return;
        }
        let max = lines.saturating_sub(visible);
        let target = (scroll as i32 + delta).clamp(0, max as i32) as usize;
        if target == scroll {
            return;
        }
        if let Some(p) = self.ai_panel.as_mut() {
            p.scroll = target;
        }
        // Re-render con la nueva ventana de líneas visibles (ambos lados se
        // evalúan antes de bindear: el bitmap es owned, el préstamo mutable
        // de `ai_panel` vive solo en el cuerpo y termina antes del bump).
        if let Some(bmp) = render_ai_panel(self) {
            if let Some(p) = self.ai_panel.as_mut() {
                p.bitmap = bmp;
            }
            // Contenido nuevo del panel → id de generación nuevo (caché GPU).
            self.ai_panel_id = self.next_ovl_id();
        }
        self.redraw();
    }
}
