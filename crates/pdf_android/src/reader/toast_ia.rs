// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Toast y panel de IA (extraído de `reader.rs`, 2026-09-06): aviso breve (`show_toast`) y "Preguntar a la IA" — `ask_ai` (hilo de fondo híbrido Gemini/Groq con imagen), `ai_answer`, `rebuild_ai_panel`, `close_ai_panel`, `ai_scroll`.

use super::AiPanel;
use super::AiPhase;
use super::Reader;
use crate::draw::ai_panel_layout;
use crate::draw::render_ai_panel;
use crate::persist::{self};
use log::info;
use std::time::Duration;
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
    // El tap en "IA" del menú de selección (`input::sel_menu_tap`) llama a
    // `ask_ai`: se cierra el menú, se abre el panel en fase Asking
    // ("preguntando…") y se lanza un hilo de fondo (std::thread + mpsc, el
    // mismo patrón de `pdf_core::prefetch`) que llama a la IA y envía el
    // resultado por el canal. El hilo de UI sondea el canal en `tick`
    // (`try_recv`, sin bloquear) y al llegar el mensaje pasa el panel a
    // Answer (texto envuelto con scroll) o Error (mensaje claro en el mismo
    // panel). Decisiones:
    //
    // - HÍBRIDO (2026-08-XX): con IMAGEN de la selección (crop del bitmap
    //   cacheado, `sel_image_png_base64`) se llama a
    //   `GeminiClient::explain_image` (pdf_core::ai) con `GEMINI_MODEL`: el
    //   modelo de visión de Groq fue RETIRADO (403), así que la imagen va a
    //   Gemini; la imagen es la fuente principal para ecuaciones/gráficos y
    //   el texto extraído va como contexto adicional en el prompt (puede ser
    //   "" en un PDF escaneado). Sin imagen, se cae a `GroqClient::chat`
    //   solo-texto con `GROQ_MODEL` (el flujo de siempre).
    // - Reintento: si Gemini falla (p. ej. 503/busy), se intenta UNA vez más
    //   tras ~1 s DENTRO del hilo de fondo (nunca bloquea el hilo de UI); si
    //   sigue fallando, el panel muestra un error claro ("modelo ocupado,
    //   reintenta") con el detalle del error original.
    // - La key va EMBEBIDA en el APK (uso personal, sin telemetría; ver
    //   `lib.rs`). Una consulta no se cancela al cerrar el panel: el hilo
    //   termina solo y el resultado se descarta al soltar el receptor.
    // - El hilo de fondo evita bloquear el hilo de UI durante la red
    /// "IA": lanza la consulta a la IA (Gemini con imagen / Groq solo-texto)
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
        // rect vacío) — en ese caso se cae al envío solo-texto de siempre.
        let image = self.sel_image_png_base64();
        self.clear_selection(); // el panel sustituye al menú de selección
        if text.is_empty() && image.is_none() {
            self.show_toast("no text");
            return;
        }
        info!(
            "ask_ai: {} chars, image {} B PNG -> text Groq ({}), image Gemini ({})",
            text.chars().count(),
            image.as_ref().map_or(0, String::len),
            crate::GROQ_MODEL,
            crate::GEMINI_MODEL
        );
        // Panel en fase Asking ("preguntando…") y hilo de fondo con la
        // llamada HTTP: el UI nunca espera por la red.
        self.ai_text = "preguntando…".to_string();
        self.ai_phase = AiPhase::Asking;
        self.rebuild_ai_panel();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = match image {
                // Con imagen (ecuación/gráfico): Gemini con `explain_image`;
                // el prompt es la instrucción + el texto extraído como
                // contexto adicional (puede ser "" en un PDF escaneado — la
                // imagen es la fuente principal). El modelo de visión de
                // Groq está retirado (403), por eso la imagen va a Gemini.
                Some(b64) => {
                    let client = pdf_core::ai::GeminiClient::with_model(
                        crate::GOOGLE_API_KEY,
                        crate::GEMINI_MODEL,
                    );
                    let mut prompt =
                        "Explica de forma clara y concisa lo que se ve en la imagen (ecuación, gráfico o texto)."
                            .to_string();
                    if !text.is_empty() {
                        prompt.push_str("\n\nTexto extraído de la página (contexto adicional):\n");
                        prompt.push_str(&text);
                    }
                    match client.explain_image(&prompt, &b64) {
                        Ok(answer) => Ok(answer),
                        Err(first_err) => {
                            // Reintento ÚNICO tras ~1 s: Gemini devuelve
                            // 503/busy en picos de carga y el segundo intento
                            // suele pasar. El sleep va en el hilo de fondo
                            // (el hilo de UI nunca se bloquea).
                            std::thread::sleep(std::time::Duration::from_secs(1));
                            match client.explain_image(&prompt, &b64) {
                                Ok(answer) => Ok(answer),
                                Err(second_err) => Err(pdf_core::AiError::Http {
                                    status: 503,
                                    body: format!(
                                        "modelo ocupado, reintenta — error original: {first_err}; tras reintento: {second_err}"
                                    ),
                                }),
                            }
                        }
                    }
                }
                // Sin imagen: el chat solo-texto de Groq de siempre.
                None => {
                    let client = pdf_core::ai::GroqClient::with_model(
                        crate::GROQ_API_KEY,
                        crate::GROQ_MODEL,
                    );
                    let system = "Eres un asistente de estudio. Explica de forma clara y concisa el texto que te dan.";
                    client.chat(system, &text)
                }
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
