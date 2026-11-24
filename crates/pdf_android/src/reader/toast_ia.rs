// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Toast y panel de IA: aviso breve (`show_toast`) y "Preguntar a la IA" — `ask_ai` (hilo de fondo Gemini con imagen y contexto de la hoja), `ai_answer`, `rebuild_ai_panel`, `close_ai_panel`, `ai_scroll`.

use super::AiPanel;
use super::AiPhase;
use super::PendingAiRequest;
use super::Reader;
use crate::draw::ai_panel_layout;
use crate::draw::render_ai_panel;
use log::info;
use std::time::Instant;

fn build_ai_prompt(page_num: u32, selected_text: &str, has_image: bool) -> String {
    let mut prompt = format!(
        "Eres un asistente de estudio. Explica de forma clara y concisa, en español, el contenido seleccionado por el estudiante en la página {page_num}. Si es una ecuación o un gráfico, explica qué significa y para qué sirve. No te limites a repetir el texto: explícalo. Escribe las fórmulas en notación sencilla y legible sin LaTeX complejo (por ejemplo x^2, a/b, raíz(x)) y usa párrafos cortos."
    );
    if !selected_text.is_empty() {
        prompt.push_str("\n\nTexto seleccionado:\n");
        prompt.push_str(selected_text);
    }
    if has_image {
        prompt.push_str("\n\nLa imagen adjunta es el recorte exacto de la selección del estudiante: es la fuente principal de lo que hay que explicar.");
    }
    prompt
}

impl Reader {
    /// Aviso breve sobre el indicador de página ("copied", ...): texto +
    /// timestamp; `tick` lo expira a los `TOAST_MS` y el bitmap cacheado se
    /// invalida al cambiar el texto.
    pub(crate) fn show_toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
        self.toast_bitmap = None;
        self.redraw();
    }

    /// Captura únicamente el recorte y texto de la selección (nunca el resto de
    /// la página) y abre el panel en fase de confirmación antes de enviar nada
    /// a Google Gemini (ADR-015).
    pub(crate) fn prepare_ai_explain(&mut self) {
        let text = self.sel_text();
        let image = self.sel_image_png_base64();
        let page = self.page + 1;
        self.clear_selection();
        if text.is_empty() && image.is_none() {
            self.show_toast("no text");
            return;
        }
        let what = match (!text.is_empty(), image.is_some()) {
            (true, true) => "el recorte y el texto seleccionado",
            (false, true) => "el recorte seleccionado",
            (true, false) => "el texto seleccionado",
            (false, false) => return,
        };
        self.ai_rx = None;
        self.ai_pending = Some(PendingAiRequest { page, text, image });
        self.ai_text = format!(
            "Se enviará a Google Gemini únicamente {what} de la página {page} (sin el resto de la hoja).\n\nPulsa Explicar para continuar o × para cancelar."
        );
        self.ai_phase = AiPhase::Confirm;
        self.rebuild_ai_panel();
        self.redraw();
    }

    /// Confirma el envío a Gemini tras pulsar explícitamente «Explicar» en el
    /// panel de confirmación. Envía solo el recorte y texto capturados.
    pub(crate) fn confirm_ask_ai(&mut self) {
        let Some(req) = self.ai_pending.take() else {
            return;
        };
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
            "ask_ai: page {}, {} chars, image {} B PNG -> Gemini ({})",
            req.page,
            req.text.chars().count(),
            req.image.as_ref().map_or(0, String::len),
            crate::GEMINI_MODEL
        );
        let prompt = build_ai_prompt(req.page, &req.text, req.image.is_some());
        let image = req.image;
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
                Err(pdf_core::AiError::Busy(_)) => {
                    let fallback = pdf_core::ai::GeminiClient::with_model(
                        crate::GOOGLE_API_KEY,
                        crate::GEMINI_FALLBACK_MODEL,
                    );
                    attempt(&fallback)
                }
                Err(e) => Err(e),
            };
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

    /// Cierra el panel de IA (✕ o tap fuera): descarta la selección pendiente o
    /// el resultado en vuelo (el hilo de fondo termina solo y su mensaje se
    /// descarta al soltar el receptor).
    pub(crate) fn close_ai_panel(&mut self) {
        let had = self.ai_panel.is_some();
        self.ai_panel = None;
        self.ai_pending = None;
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

#[cfg(test)]
mod tests {
    use super::build_ai_prompt;

    #[test]
    fn prompt_contains_only_page_number_selected_text_and_image_note() {
        let prompt = build_ai_prompt(7, "E = mc^2", true);
        assert!(prompt.contains("página 7"));
        assert!(prompt.contains("E = mc^2"));
        assert!(prompt.contains("imagen adjunta"));
        assert!(!prompt.contains("Texto de la hoja"));

        let image_only = build_ai_prompt(3, "", true);
        assert!(image_only.contains("página 3"));
        assert!(!image_only.contains("Texto seleccionado:"));
        assert!(image_only.contains("imagen adjunta"));
    }
}
