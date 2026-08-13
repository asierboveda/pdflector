# Rediseño de UX — Estructura (2026-08-XX)

Cambios ESTRUCTURALES en `crates/pdf_android/` (el estilo visual lo hará otro
agente después; aquí solo la estructura funcional, "fea pero funcional").
Solo se tocó `pdf_android` (draw.rs, input.rs, reader.rs, lib.rs, jni.rs,
annotations.rs + módulo nuevo `thumbs.rs`); **no** se tocaron
pdf_core/pdf_app/pdf_bench ni zoom.rs/view.rs/persist.rs/cache.rs.

## 1. Visor a pantalla completa

- La barra superior fija (`render_viewer_bar`, botones Open/✏️/●/↶/−10/+10/Dark)
  se eliminó por completo (constantes `VIEWER_BAR_H_DIV` y derivadas, campo
  `Reader::viewer_bar`, `input::viewer_bar_tap`, `draw::render_viewer_bar`).
- El documento ocupa TODA la pantalla. El tap izq/der para página y el pinch
  zoom anclado siguen funcionando sin cambios.

## 2. Sheet de ajustes (panel deslizante desde arriba)

- **Gesto de revelado**: arrastre de UN dedo que empieza en la mitad superior
  de la ventana y baja más de `TAP_SLOP` (`GestureKind::Pull`). No choca con
  el tap de página (el tap es < `TAP_SLOP` de movimiento) ni con el pinch
  (2 dedos → zoom; el sheet se queda como esté).
- **Estado**: `Reader::sheet_open` (objetivo) + `sheet_progress` (0..1,
  dibujo del panel a `y = −alto·(1−progress)`) + `sheet_anim` (animación en
  vuelo). Alto del panel: mitad de la ventana (`sheet_h`).
- **Seguimiento del dedo**: `drag_sheet(dy)` durante el Move; al soltar,
  `end_sheet_drag` anima hacia el objetivo más cercano (abierto si
  `progress ≥ 0.5`). Cierre con swipe up o tap FUERA del panel (`hide_sheet`).
  Un tap dentro pulsa botones (`draw::sheet_buttons`, geometría compartida
  con `input::sheet_tap`).
- **Animación**: ease exponencial (~10 ticks ≈ 150 ms) avanzada por
  `Reader::tick`; el bucle de `android_main` usa `poll_events(Some(16 ms))`
  SOLO mientras `sheet_animating()` o `thumbs_pending()` (en reposo bloquea
  sin batería extra).
- **Controles**: Back (biblioteca MediaStore), Open (picker interno), Dark/
  Light, −10 / +10, "N / total" (tap = página siguiente). Hint: "Swipe up or
  tap outside to close". El sheet se invalida al cambiar página/ventana/modo
  oscuro y se libera al cerrar del todo.

## 3. Lápiz / subrayado / undo / color eliminados

- Se quitaron el botón ✏️ (modo dibujo), ● (color), ↶ (undo) y el gesto de
  dibujo (`handle_draw_motion`, `Reader::{toggle_draw_mode,cycle_stroke_color,
  undo_last_stroke,begin_stroke,extend_stroke,finish_stroke,cancel_stroke,
  screen_to_page,page_at_y}` y el campo `active` de `draw::PageAnnots`).
- **Se MANTIENE** la carga y el render de anotaciones ya guardadas: el
  usuario no pierde sus trazos; solo no se pueden crear desde la UI por ahora.
  `annotations.rs` queda con `#![allow(dead_code)` y la justificación en su
  cabecera; `Reader::save_annotations` también (camino de guardado intacto
  para una fase futura). `LibraryEntry.size` idem (la rejilla no lo muestra).

## 4. Indicador de página abajo a la izquierda

- Overlay pequeño "N / total" (`draw::render_page_badge`, bitmap cacheado en
  `Reader::page_badge`, tamaño ~150×33 px en la tablet, opaco con los colores
  del tema), dibujado en el MISMO lock que las páginas (`blit_stacked` recibe
  ahora `overlays: &[(&Bitmap, x, y)]`).
- **Tap en él = página siguiente** (decisión documentada en
  `input::page_badge_tap`): el indicador además de informar es un acceso
  rápido a la siguiente página (igual que el indicador de la antigua barra).
  Con el sheet abierto, el tap cierra el panel (el cierre no cambia de página).

## 5. Biblioteca en rejilla 3×3 con portada

- **Celda**: portada (página 1) centrada en un área de proporción A4
  (`grid_cover_w/h`, misma para todas las celdas → rejilla uniforme) + título
  debajo en 1-2 líneas truncadas (`draw::split_title`). Geometría compartida
  entre render y tap (`grid_cell_rect`/`grid_rows_y0`/`grid_visible_rows`).
  La cabecera (Rescan/Grant/Back) y la franja de estado se conservan.
- **Portada perezosa**: `Reader::pump_thumbs` renderiza como máximo **3
  portadas por tick** y SOLO de celdas VISIBLES (nunca las 256 de golpe);
  mientras cargan se pinta un placeholder "…" (`render_library_grid`). El
  re-render lo dispara el propio pump (`list_dirty`), con el bucle en modo
  timeout mientras `thumbs_pending()`.
- **Render de la portada sin copiar el PDF**: `jni::open_content_fd` abre la
  content:// URI con `ContentResolver.openFileDescriptor` y devuelve el fd
  nativo; MuPDF abre `/proc/self/fd/{fd}` y solo lee lo necesario de la
  página 1 (nada de leer ficheros de 100 MB enteros; si el proveedor no
  respalda un fichero real, la celda se queda con el placeholder).
- **Caché acotada** (`thumbs.rs`, `ThumbCache` LRU): clave = content:// URI;
  36 entradas, presupuesto 9 MiB, portadas de 200 px de ancho. Presupuesto
  documentado: 36 × (200×283×4 B ≈ 226 KiB) ≈ 8,1 MiB → ~6 % del RSS objetivo
  < 150 MB; no compite con la `PageCache` del visor (48 MiB): estados
  mutuamente excluyentes y `open_pdf` limpia la caché de portadas.
- **Índice de letras A-Z quitado** (decisión documentada): la tira de 27
  celdas estaba diseñada para la lista de filas y no encaja en la rejilla; la
  navegación es por scroll. Se eliminaron `normalize_letter`, `lib_strip_*`,
  `library_filter(_ed)` y `set_library_filter`.

## 6. Verificación

- `cargo build -p pdf_android --target aarch64-linux-android --release`: OK
  sin warnings (NDK r28 + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`).
- `cargo clippy -p pdf_android --target aarch64-linux-android --release`: OK
  (clippy host no compila: `ndk-sys` es solo-Android).
- `cargo fmt --all -- --check`: OK.
- `cargo check -p pdf_core -p pdf_app -p pdf_bench` (host): OK (no tocados).
