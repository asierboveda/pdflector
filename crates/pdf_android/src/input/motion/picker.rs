// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Input de las LISTAS: `handle_picker_motion` (Down/Move/Up/Cancel con
//! scroll vertical/horizontal y tap, sin pinch) para picker, biblioteca y
//! Descubrir, `classify_list_motion` y `picker_tap` (picker interno y
//! selector de añadir).

use super::library::library_down_zone;
use super::list_tap;
use crate::TAP_SLOP;
use crate::android_activity::AndroidApp;
use crate::android_activity::input::MotionAction;
use crate::reader::{
    ListDrag, PickRow, PickerKind, Reader, UiMode, picker_btn_w, picker_header_h, picker_row_h,
};

/// Tap del picker: botones de la cabecera (Back/Rescan) o fila de la lista.
/// El fallback interno (`PickerKind::Files`) abre el PDF; el selector de
/// añadir (`PickerKind::Select`) lo CURA en la biblioteca (`add_selected`).
/// La geometría DEBE reflejar exactamente la de `render_picker_list` (mismas
/// fórmulas de layout).
pub(super) fn picker_tap(reader: &mut Reader, app: &AndroidApp, x: f32, y: f32) {
    let win_w = reader.win_w as f32;
    let row_h = picker_row_h(reader.win_h) as f32;
    let header_h = picker_header_h(reader.win_h) as f32;
    let status_h = if reader.status.is_some() { row_h } else { 0.0 };
    let btn_w = picker_btn_w(reader.win_w) as f32;
    let selecting = reader.picker_kind == PickerKind::Select;

    // Cabecera: botones a la derecha (Back a la izquierda de Rescan).
    if y < header_h {
        let rescan_x = win_w - btn_w - 8.0;
        if x >= rescan_x {
            if selecting {
                reader.rescan_select(app);
            } else {
                reader.rescan(app);
            }
            return;
        }
        let back_x = win_w - btn_w * 2.0 - 16.0;
        if (reader.doc.is_some() || selecting) && x >= back_x && x < rescan_x {
            if selecting {
                reader.cancel_add(app);
            } else {
                reader.exit_picker();
            }
            return;
        }
        return;
    }

    // Franja de estado (no seleccionable) + barra de breadcrumb del gestor.
    let crumbs = if reader.picker_has_crumb() {
        row_h
    } else {
        0.0
    };
    let rows_y0 = header_h + status_h + crumbs;
    if y < rows_y0 {
        // Tap en la barra de breadcrumb (dentro de una carpeta): subir un
        // nivel del gestor de archivos.
        if selecting && y >= header_h + status_h && reader.picker_has_crumb() {
            reader.picker_sel_up();
        }
        return;
    }

    let row = ((y - rows_y0) / row_h) as usize + reader.list_scroll;
    if selecting {
        // Gestor de archivos del selector: carpeta → entrar; PDF → curar.
        if let Some(pr) = reader.picker_rows().get(row) {
            match pr {
                PickRow::Folder(name) => {
                    let name = name.clone();
                    reader.picker_sel_enter(&name);
                }
                PickRow::File(idx) => reader.add_selected(app, *idx),
            }
        }
    } else if row < reader.picker_len() {
        let name = reader.pdf_list[row].name.clone();
        let path = reader.pdf_list[row].path.clone();
        if !reader.open_pdf(&path) {
            reader.status = Some(format!("Cannot open {name}"));
            reader.list_dirty = true;
            reader.redraw();
        }
    }
}

/// Qué es el desplazamiento (`dx`, `dy`) de un dedo en una lista desde su Down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListMotion {
    /// Dentro de `TAP_SLOP`: el Move no hace nada y el Up es un tap.
    Tap,
    /// Arrastre HORIZONTAL (carousel/filas de chips de la biblioteca).
    Horizontal,
    /// Arrastre VERTICAL (scroll de la lista).
    Vertical,
}

/// Clasifica el desplazamiento de un dedo en una lista (picker, biblioteca,
/// Descubrir): por debajo de `TAP_SLOP` es tap; si no, manda el eje
/// dominante (empate = vertical).
pub(crate) fn classify_list_motion(dx: f32, dy: f32) -> ListMotion {
    let moved = (dx * dx + dy * dy).sqrt();
    if moved > TAP_SLOP && dx.abs() > dy.abs() {
        ListMotion::Horizontal
    } else if moved > TAP_SLOP {
        ListMotion::Vertical
    } else {
        ListMotion::Tap
    }
}

/// Input del picker/biblioteca (un solo dedo): arrastre VERTICAL = scroll de
/// la lista (picker: por filas; biblioteca: por PÍXELES del contenido
/// completo, recientes + rejilla); arrastre HORIZONTAL = scroll del carousel
/// de recientes o de las filas de chips (biblioteca); tap (sin arrastre) =
/// selección. Reemplaza a la máquina de gestos del visor (sin pinch).
pub(super) fn handle_picker_motion(
    reader: &mut Reader,
    app: &AndroidApp,
    action: MotionAction,
    pts: Vec<(i32, f32, f32)>,
    _up_idx: Option<usize>,
) {
    match action {
        MotionAction::Down => {
            if let Some(&(_, x, y)) = pts.first() {
                let (zone, h0) = if reader.mode == UiMode::Picker {
                    (0, 0.0)
                } else if reader.mode == UiMode::Library {
                    let z = library_down_zone(reader, y);
                    let h = match z {
                        2 => reader.library.lib_letters_x,
                        3 => reader.library.lib_folders_x,
                        4 => reader.library.lib_sort_x,
                        5 => reader.library.lib_filter_x,
                        _ => 0.0,
                    };
                    (z, h)
                } else {
                    (0, 0.0)
                };
                let v0 = if reader.mode == UiMode::Picker {
                    reader.list_scroll as f32
                } else if reader.mode == UiMode::Library {
                    reader.library.lib_scroll
                } else {
                    reader.discover_scroll()
                };
                reader.list_drag = Some(ListDrag {
                    sx: x,
                    sy: y,
                    v0,
                    h0,
                    zone,
                });
            }
        }
        MotionAction::Move => {
            if let Some(drag) = reader.list_drag.as_ref()
                && let Some(&(_, x, y)) = pts.first()
            {
                let dx = x - drag.sx;
                let dy = y - drag.sy;
                let motion = classify_list_motion(dx, dy);
                if motion == ListMotion::Horizontal {
                    // Arrastre HORIZONTAL (solo biblioteca): el scroll de
                    // partida se guardó en `h0` según la zona del Down.
                    if reader.mode == UiMode::Library {
                        let max = match drag.zone {
                            2 => reader.lib_chips_max_x(0),
                            3 => reader.lib_chips_max_x(1),
                            4 => reader.lib_org_max_x(0),
                            5 => reader.lib_org_max_x(1),
                            _ => 0.0,
                        };
                        let s = (drag.h0 - dx).clamp(0.0, max);
                        let changed = match drag.zone {
                            2 => reader.library.lib_letters_x != s,
                            3 => reader.library.lib_folders_x != s,
                            4 => reader.library.lib_sort_x != s,
                            5 => reader.library.lib_filter_x != s,
                            _ => false,
                        };
                        if changed {
                            match drag.zone {
                                2 => reader.library.lib_letters_x = s,
                                3 => reader.library.lib_folders_x = s,
                                4 => reader.library.lib_sort_x = s,
                                5 => reader.library.lib_filter_x = s,
                                _ => {}
                            }
                            // Scroll horizontal de una fila: se re-renderiza
                            // SOLO esa fila (bitmap pequeño) y se remienda
                            // sobre su contenedor; la pantalla no se
                            // re-renderiza (antes `list_dirty` reconstruía
                            // TODO por frame de arrastre).
                            reader.library.lib_row_dirty = Some(drag.zone);
                            reader.redraw();
                        }
                    }
                } else if motion == ListMotion::Vertical {
                    // Arrastre VERTICAL: picker por filas, biblioteca por px.
                    if reader.mode == UiMode::Picker {
                        let row_h = picker_row_h(reader.win_h) as f32;
                        let max_scroll =
                            reader.picker_len().saturating_sub(reader.picker_visible());
                        let s =
                            (drag.v0 - dy / row_h).round().clamp(0.0, max_scroll as f32) as usize;
                        if s != reader.list_scroll {
                            reader.list_scroll = s;
                            reader.list_dirty = true;
                            reader.redraw();
                        }
                    } else if reader.mode == UiMode::Library {
                        let max_v = reader.lib_max_scroll();
                        let s = (drag.v0 - dy).clamp(0.0, max_v);
                        if s != reader.library.lib_scroll {
                            reader.library.lib_scroll = s;
                            reader.redraw();
                        }
                    } else if reader.mode == UiMode::Discover {
                        let max_v = reader.discover_max_scroll();
                        let s = (drag.v0 - dy).clamp(0.0, max_v);
                        if s != reader.discover_scroll() {
                            reader.set_discover_scroll(s);
                            reader.redraw();
                        }
                    }
                }
            }
        }
        MotionAction::Up => {
            let drag = reader.list_drag.take();
            if let (Some(d), Some(&(_, x, y))) = (drag, pts.first()) {
                let motion = classify_list_motion(x - d.sx, y - d.sy);
                if motion == ListMotion::Tap {
                    list_tap(reader, app, x, y);
                }
            }
            reader.gesture.pointers.clear();
        }
        MotionAction::Cancel => {
            reader.list_drag = None;
            reader.gesture.pointers.clear();
        }
        // PointerUp: se ignora un segundo dedo (el picker no tiene pinch).
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_motion_within_slop_is_a_tap() {
        assert_eq!(classify_list_motion(0.0, 0.0), ListMotion::Tap);
        assert_eq!(classify_list_motion(TAP_SLOP, 0.0), ListMotion::Tap);
        assert_eq!(classify_list_motion(0.0, -TAP_SLOP), ListMotion::Tap);
    }

    #[test]
    fn list_motion_beyond_slop_follows_dominant_axis() {
        let d = TAP_SLOP + 1.0;
        assert_eq!(classify_list_motion(d, 0.0), ListMotion::Horizontal);
        assert_eq!(classify_list_motion(-d, 1.0), ListMotion::Horizontal);
        assert_eq!(classify_list_motion(0.0, d), ListMotion::Vertical);
        assert_eq!(classify_list_motion(1.0, -d), ListMotion::Vertical);
    }

    #[test]
    fn list_motion_diagonal_tie_is_vertical() {
        let d = TAP_SLOP;
        assert_eq!(classify_list_motion(d, d), ListMotion::Vertical);
        assert_eq!(classify_list_motion(-d, d), ListMotion::Vertical);
    }
}
