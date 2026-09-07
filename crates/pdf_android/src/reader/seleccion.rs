// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Selección de texto (extraído de `reader.rs`, 2026-09-06): máquina de la selección por long-press + arrastre (`begin_sel`…`end_sel`, `clear_selection`, `has_selection`), transformaciones pantalla↔página (`screen_to_page`, `page_screen_rect`, `sel_screen_rect`, `sel_page_rect`), extracción de texto e imagen (`sel_text`, `sel_image_png_base64`) y las acciones Copiar/Subrayar/menú (`copy_sel`, `highlight_sel`, `open_sel_menu`).

use super::Reader;
use super::SelMenu;
use super::SelState;
use crate::SEL_MIN_PX;
use crate::draw::render_sel_menu;
use crate::draw::sel_menu_layout;
use crate::view::initial_scale;
use android_activity::AndroidApp;
use base64::Engine;
use pdf_core::{Annotation, Color, Document, Highlight, Rect, TextSpan};

// ---------------------------------------------------------------------
// Selección de texto: long-press + arrastre, copiar y subrayar (Parte 1)
// ---------------------------------------------------------------------
//
// El gesto vive en `input.rs` (long-press + arrastre); aquí
// el estado (`sel`/`sel_menu`), las transformaciones de coords, la
// extracción de texto y las acciones Copiar/Subrayar. Decisiones
// documentadas en `SelState` (coords de pantalla) y en el doc de la
// cabecera de `lib.rs`.
impl Reader {
    /// ¿Hay una selección activa (en curso o fijada con su menú)? El tap
    /// simple izq/der de página NO se dispara mientras tanto (ver
    /// `input::sel_menu_tap`/`fire_tap_action`).
    ///
    /// `dead_code` intencional (2026-08-XX): los gestos consultan el estado
    /// directamente (`sel`/`sel_menu`) y esta es la API pública que pide la
    /// Parte 1 para que otros agentes (p. ej. la Parte 2 —IA—) sepan si hay
    /// selección sin tocar el estado interno.
    #[allow(dead_code)]
    pub(crate) fn has_selection(&self) -> bool {
        self.sel.is_some()
    }

    /// Comienza el modo selección: ancla = punto del LONG-PRESS y punto
    /// actual = el mismo (el rect, un PUNTO aún sin arrastrar, crece con
    /// `update_sel`). Solo se llama al superar `LONG_PRESS_MS` con el dedo
    /// quieto (`input::tick_gestures`, `GestureKind::Selecting`). Blit
    /// directo (sin re-render):
    /// como en el pinch, la página está cacheada y solo cambia la capa.
    pub(crate) fn begin_sel(&mut self, ax: f32, ay: f32) {
        self.sel = Some(SelState {
            anchor: (ax, ay),
            cur: (ax, ay),
        });
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Actualiza el punto actual del arrastre (posición del dedo) y
    /// redibuja el rect (blit directo, página cacheada).
    pub(crate) fn update_sel(&mut self, cx: f32, cy: f32) {
        if let Some(s) = self.sel.as_mut() {
            s.cur = (cx, cy);
        }
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Fija la selección al levantar el dedo: si el rect es significativo
    /// (≥ `SEL_MIN_PX` por lado) abre el menú Copiar/Subrayar/IA; un
    /// long-press sin arrastre (rect degenerado, el punto) se descarta.
    pub(crate) fn end_sel(&mut self) {
        let Some((l, t, r, b)) = self.sel_screen_rect() else {
            self.clear_selection(); // no hubo arrastre
            return;
        };
        if (r - l).abs() < SEL_MIN_PX || (b - t).abs() < SEL_MIN_PX {
            self.clear_selection(); // long-press sin arrastre: nada que fijar
            return;
        }
        self.open_sel_menu();
        if self.window.is_some() {
            self.blit();
        }
    }

    /// Descarta la selección y su menú (si los hay) y redibuja solo si había
    /// algo visible que quitar. Es la acción de "tocar fuera del menú" y la
    /// limpieza de cualquier transición (cambio de página/documento, gesto
    /// cancelado, segundo dedo).
    pub(crate) fn clear_selection(&mut self) {
        let had = self.sel.is_some() || self.sel_menu.is_some();
        self.sel = None;
        self.sel_menu = None;
        if had && self.window.is_some() {
            self.blit();
        }
    }

    /// Transformación pantalla → página (px de ventana → puntos PDF): la
    /// INVERSA exacta del mapeo del blit (`screen = (dx, dy) + pt × scale`,
    /// con `scale = cover × zoom` y `dx/dy` la esquina del bitmap escalado —
    /// centrado cover + pan de anclaje; ver `blit` y `PageAnnots`). Es la
    /// misma familia de transformación que usan el pinch (`anchor_pan`) y la
    /// capa de anotaciones, así que el rect de selección queda alineado con
    /// lo que se ve. None si la página actual no está disponible.
    pub(crate) fn screen_to_page(&self, sx: f32, sy: f32) -> Option<(f32, f32)> {
        let doc = self.doc.as_ref()?;
        let (pw, ph) = doc.page_size(self.page).ok()?;
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        let scale = cover * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let dx = (Self::centered_base(self.win_w, pw * cover, self.zoom) + self.pan_x).round();
        let dy = self.pan_y.round();
        Some(((sx - dx) / scale, (sy - dy) / scale))
    }

    /// Rectángulo de la página actual en px de ventana (left, top, right,
    /// bottom): la posición del bitmap escalado + su tamaño a la escala
    /// efectiva `cover × zoom` — la MISMA geometría del blit. None si la
    /// página no está disponible. Se usa para RECORTAR el rect de selección
    /// a los bordes de la hoja (nunca a la ventana entera).
    fn page_screen_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.doc.as_ref()?;
        let (pw, ph) = doc.page_size(self.page).ok()?;
        let cover = initial_scale(pw, ph, self.win_w, self.win_h);
        let scale = cover * self.zoom;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let dx = (Self::centered_base(self.win_w, pw * cover, self.zoom) + self.pan_x).round();
        let dy = self.pan_y.round();
        Some((dx, dy, dx + pw * scale, dy + ph * scale))
    }

    /// Rect normalizado de la selección en px de ventana (left, top, right,
    /// bottom), RECORTADO a los bordes de la página actual: si el dedo
    /// arrastra fuera de la hoja (letterbox/pan), el rect se detiene en el
    /// borde. None sin selección o sin página.
    pub(crate) fn sel_screen_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let s = self.sel?;
        let l = s.anchor.0.min(s.cur.0);
        let r = s.anchor.0.max(s.cur.0);
        let t = s.anchor.1.min(s.cur.1);
        let b = s.anchor.1.max(s.cur.1);
        let (pl, pt, pr, pb) = self.page_screen_rect()?;
        Some((l.max(pl), t.max(pt), r.min(pr), b.min(pb)))
    }

    /// Rect de la selección en coordenadas de PÁGINA (puntos PDF): convierte
    /// las dos esquinas del rect de pantalla con `screen_to_page`. Lo usan
    /// la extracción de texto (`sel_text`) y el subrayado
    /// (`highlight_sel`) — la ÚNICA conversión a página que se hace.
    pub(crate) fn sel_page_rect(&self) -> Option<Rect> {
        let (l, t, r, b) = self.sel_screen_rect()?;
        let a = self.screen_to_page(l, t)?;
        let c = self.screen_to_page(r, b)?;
        // `Rect::new` normaliza extents negativos (defensa; ya ordenado).
        Some(Rect::new(a.0, a.1, c.0 - a.0, c.1 - a.1))
    }

    /// Extrae el texto bajo la selección actual: llama a `doc.text(page)`
    /// UNA sola vez y concatena el texto de los spans cuyo bbox INTERSECTA
    /// el rect de selección en página, en orden de lectura (ordenados por y
    /// y luego x).
    ///
    /// Devuelve cadena vacía si no hay selección, si la página no tiene
    /// texto extraíble (p. ej. PDF ESCANEADO: `spans` vacío) o si el rect no
    /// cubre ningún span — en ese caso "Copiar" avisa "no text" en vez de
    /// copiar basura.
    pub(crate) fn sel_text(&mut self) -> String {
        let Some(page_rect) = self.sel_page_rect() else {
            return String::new();
        };
        let Some(doc) = self.doc.as_ref() else {
            return String::new();
        };
        let Ok(pt) = self.text_cache.get_or_extract(doc, self.page) else {
            return String::new();
        };
        let pt = pt.as_ref();
        if pt.spans.is_empty() {
            return String::new(); // PDF escaneado / sin texto extraíble
        }
        // Intersección de bbox en coords de página (el rect ya está en
        // página): `span` corta al rect si sus bordes se solapan.
        let (px, py) = (page_rect.x, page_rect.y);
        let (qx, qy) = (page_rect.x + page_rect.w, page_rect.y + page_rect.h);
        let mut hits: Vec<&TextSpan> = pt
            .spans
            .iter()
            .filter(|s| s.x < qx && s.x + s.w > px && s.y < qy && s.y + s.h > py)
            .collect();
        // Orden de lectura: por y (fila), luego x (izquierda → derecha).
        // MuPDF ya devuelve los spans en orden aproximado de lectura, pero
        // el sort garantiza el orden aunque la selección cruce columnas.
        hits.sort_by(|a, b| {
            a.y.partial_cmp(&b.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        // Los spans son líneas de texto (stext line): se unen con salto de
        // línea para conservar la lectura por filas al copiar.
        hits.iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// PNG base64 de la REGIÓN seleccionada (para "Preguntar a la IA" con
    /// visión: ecuaciones/gráficos, Fase 5): recorta del bitmap cacheado de
    /// la página actual el rect de selección (px de ventana,
    /// `sel_screen_rect`) y lo codifica a PNG en memoria → base64 SIN
    /// prefijo (el contrato de `GeminiClient::explain_image`). None si no
    /// hay bitmap, escala inválida o el crop queda vacío (zoom/pan raro) —
    /// el llamador cae al envío solo-texto.
    ///
    /// Mapeo ventana → píxeles del bitmap: la MISMA geometría del blit
    /// (`blit`): el render FULL de la página se dibujaría en `(dx, dy)`
    /// escalado por `blit_zoom = zoom / rendered_zoom`, así que un px de
    /// pantalla `s` cae en el px del render full `(s − origen) / blit_zoom`.
    /// El bitmap residente es el recorte a ventana (X-centrado, Y-top) de
    /// ese render (fix de residency), así que se resta el origen del crop
    /// (`cached.crop_x/crop_y`) para caer en los píxeles realmente
    /// almacenados; si la selección cae fuera del crop (pan/zoom extremos
    /// que dejan ver parte de la página recortada) el clamp recorta a lo
    /// disponible. Se usa floor/ceil para que el crop cubra al menos la
    /// región seleccionada — el rect de selección ya viene recortado a la
    /// hoja por `sel_screen_rect`, pero el pan puede dejar parte del rect
    /// fuera del bitmap.
    ///
    /// Modo oscuro: la caché guarda SIEMPRE bitmaps normales (la inversión
    /// se aplica al blitear, `draw::blit_page`), así que el crop sale con
    /// los colores del DOCUMENTO — decisión documentada: para explicar una
    /// ecuación la información es la misma y el modelo de visión no se
    /// confunde con un fondo negro.
    pub(crate) fn sel_image_png_base64(&self) -> Option<String> {
        let cached = self.cache.peek(self.page)?;
        let bmp = &cached.bitmap;
        let (l, t, r, b) = self.sel_screen_rect()?;
        // Escala de dibujo del render cacheado (relativa a su render): 1:1
        // nítido en reposo (`rendered_zoom == zoom`), vecino-más-cercano del
        // bitmap viejo durante el pinch. Si no es finita (defensa), no hay
        // imagen que mandar.
        // Blit EFECTIVO del bitmap residente (fix salto-pinch): deriva del
        // propio bitmap, no de `rendered_zoom` (puede discrepar del residente).
        let blit_zoom = match self.entry_blit_zoom() {
            Some(b) => b,
            None => {
                if self.rendered_zoom.is_finite() && self.rendered_zoom > 0.0 {
                    self.zoom / self.rendered_zoom
                } else {
                    return None;
                }
            }
        };
        if !blit_zoom.is_finite() || blit_zoom <= 0.0 {
            return None;
        }
        // Esquina del render FULL escalado en pantalla (misma aritmética que
        // el blit sobre el bitmap sin recortar: centrado horizontal cover +
        // pan de anclaje; Y alineado arriba). El bitmap residente es el crop
        // centrado a ventana de ese render: restamos el origen del crop para
        // caer en los píxeles almacenados (ver doc del mapeo arriba).
        let dx =
            (((self.win_w as f32 - cached.full_w as f32 * blit_zoom) / 2.0) + self.pan_x).round();
        let dy = self.pan_y.round();
        let x0 = ((l - dx) / blit_zoom).floor() - cached.crop_x as f32;
        let y0 = ((t - dy) / blit_zoom).floor() - cached.crop_y as f32;
        let x1 = ((r - dx) / blit_zoom).ceil() - cached.crop_x as f32;
        let y1 = ((b - dy) / blit_zoom).ceil() - cached.crop_y as f32;
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return None; // NaN/inf (defensa): sin imagen
        }
        // Rect en píxeles del bitmap residente (crop), recortado a sus bordes.
        let (x0, y0) = (x0.max(0.0) as u32, y0.max(0.0) as u32);
        let (x1, y1) = (
            (x1 as i64).min(bmp.width as i64) as u32,
            (y1 as i64).min(bmp.height as i64) as u32,
        );
        if x0 >= x1 || y0 >= y1 {
            return None; // crop vacío (rect fuera del bitmap): sin imagen
        }
        let (w, h) = (x1 - x0, y1 - y0);
        // Crop fila a fila del RGBA8 row-major (un rango contiguo por fila:
        // `start = (row × width + x0) × 4`).
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y0..y1 {
            let start = ((row * bmp.width + x0) * 4) as usize;
            rgba.extend_from_slice(&bmp.data[start..start + (w as usize) * 4]);
        }
        // Codificación PNG en memoria (encoder `png`, RGBA8 → PNG) y base64
        // sin prefijo (contrato de `GeminiClient::explain_image`).
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&rgba).ok()?;
        }
        Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes))
    }

    /// "Copiar": copia el texto de la selección al portapapeles de Android
    /// (`jni::copy_to_clipboard`) y muestra "copied"; si no hay texto (PDF
    /// escaneado) avisa "no text". En ambos casos cierra el menú y descarta
    /// la selección.
    pub(crate) fn copy_sel(&mut self, app: &AndroidApp) {
        let text = self.sel_text();
        if text.is_empty() {
            self.show_toast("no text");
        } else {
            crate::jni::copy_to_clipboard(app, &text);
            self.show_toast("copied");
        }
        self.clear_selection();
    }

    /// "Subrayar": añade al `AnnotationSet` un `Annotation::Highlight` con
    /// el rect de selección en página (amarillo por defecto) y lo PERSISTE
    /// en el sidecar SQLite (`AnnotationStore::save`). El render de
    /// highlights ya existente (`draw::draw_annotations`, relleno
    /// translúcido bajo los trazos) lo muestra al redibujar; el frame
    /// compuesto del sheet se invalida para que lo recoja. Cierra el menú y
    /// descarta la selección.
    pub(crate) fn highlight_sel(&mut self) {
        let Some(rect) = self.sel_page_rect() else {
            self.clear_selection();
            return;
        };
        let ann = Annotation::Highlight(Highlight {
            // El rect de selección completo como un único rect (el modelo
            // permite varios rects por línea; aquí basta con la caja).
            rects: vec![rect],
            // Amarillo por defecto, alfa ~43 % (translúcido sobre el texto).
            color: Color {
                r: 255,
                g: 235,
                b: 59,
                a: 110,
            },
        });
        if let Some(id) = self.annotations.add(self.page as usize, ann) {
            // El id devuelto va a la pila de la sesión: el "↶" de la barra
            // de herramientas deshace también los subrayados hechos con la
            // selección de texto (misma sesión).
            self.session_ids.push(id);
            self.save_annotations();
            self.show_toast("highlighted");
        } else {
            self.show_toast("highlight failed");
        }
        self.clear_selection();
    }

    /// Abre el menú flotante de la selección fijada: calcula la geometría
    /// (`draw::sel_menu_layout`, cerca del rect, dentro de la ventana),
    /// renderiza el bitmap (Canvas+JNI) y guarda ambos en `sel_menu`.
    fn open_sel_menu(&mut self) {
        let Some(layout) = sel_menu_layout(self) else {
            return;
        };
        let Some(bitmap) = render_sel_menu(self) else {
            return;
        };
        let (mx, my, mrx, mry) = layout.rect;
        self.sel_menu = Some(SelMenu {
            x: mx as i32,
            y: my as i32,
            w: (mrx - mx) as i32,
            h: (mry - my) as i32,
            bitmap,
            buttons: layout.buttons,
        });
        self.sel_menu_id = self.next_ovl_id(); // menú nuevo → textura nueva
    }
}
