# CHANGELOG — PDFLector

> Últimas 5 entradas. Historial completo en `docs/log/memory-2026-08.md`.
> Formato: `AAAA-MM-DD — Título`.

## 2026-08-25 — Tarea 1/3: esqueleto del header de biblioteca (⋯ View + ☰ Settings)

Estructura de los menús de cabecera estilo Readest (sin contenido todavía,
Tareas 2-3):

- **Modelo**: enums `LibraryViewMode {Grid,List}`, `LibraryCoverFit {Crop,Fit}`
  y `LibraryGroupBy {None,Author}` + 8 campos en `Reader` con defaults
  (`view_mode`, `cover_fit`, `auto_columns`, `columns=3`, `view_menu_open`,
  `settings_menu_open`, `hide_covers`, `recent_shelf_enabled`) + helper
  `is_grid()`.
- **Geometría**: `view_menu_button_rect`/`settings_menu_button_rect` —
  círculos ~32 dp (≈65 px) con 8 dp de separación, alineados a la izquierda
  del "＋ Añadir" (anclaje a derecha con `grid_pad`), centrados en el Y de la
  fila de botones (`lib_header_buttons_cy`); los dropdowns futuros cuelgan
  del borde derecho (`dropdown-end`).
- **Render** (`render_library_header`): dos círculos con glifos ⋯ y ☰ (14 sp,
  `base_content`); el botón con su dropdown abierto se marca en `primary`
  (highlight).
- **Input** (`library_tap`): tap en ⋯ alterna `view_menu_open` (cierra
  settings); tap en ☰ alterna `settings_menu_open` (cierra view); tap FUERA
  de ambos cierra los dos (sin disparar otra acción); `list_dirty`+`redraw()`
  para re-renderizar la cabecera cacheada.
- **Persistencia** (`ViewerState`): 5 campos nuevos con `#[serde(default)]`
  (view_mode/cover_fit/columns/hide_covers/recent_shelf_enabled; defaults de
  arranque con funciones para columns=3 y shelf=true) — `state.json` viejo
  carga sin romper (verificado: fichero sin campos → defaults) y `save_state`
  los persiste (verificado con `view_mode=List` sobreviviendo a reinicios).

## 2026-08-25 — Control total de anotaciones con el BOLI (sin menús)

Toda la anotación se controla solo con el boli (S-Pen/Saber, TCL 9469X):

- **A) Tocar con el boli SIEMPRE dibuja**: el boli pasa a dibujar (Ink) o
  subrayar (Highlight) según el MODO actual sin depender de la barra de
  herramientas; el dedo sigue navegando (tap página, pinch zoom — nunca
  dibuja).
- **B) Botón UP del boli (el SUPERIOR) = alternar modo** con toast
  ("✏️ Pen" / "🖍️ Highlighter"). Calibrado en fase A: este boli reporta el botón
  superior como `STYLUS_SECONDARY` (0x40) — INVERTIDO al estándar; las
  constantes `PEN_BTN_MODE`/`PEN_BTN_ERASE` concentran la asignación. El
  toggle funciona en CONTACTO; en el AIRE el "stylus-handwriting-event-
  receiver" de Android 15 (window del sistema uid 1000, `INTERCEPTS_STYLUS`)
  consume el hover del stylus — limitación del sistema, documentada en
  `input.rs`/`annotations.rs`.
- **C) Botón DOWN (el INFERIOR) + apoyar = GOMA REAL**: recorte parcial en
  vivo (un trazo se PARTE en trozos por el barrido de la goma; un subrayado
  se divide en rectos) con `pdf_core::{split_stroke,trim_highlight}` (puras,
  con tests) + barrido continuo entre pasadas (`erase_last`). Se guarda una
  sola vez al levantar; no entra en el undo (permanente).
- **D) El modo se persiste** en `tool_state.json` (campo `mode`, cargado con
  fallback `Ink` por retrocompatibilidad — `persist.rs` NO se tocó: el JSON
  se lee/escribe como `Value` conservando ink_color/ink_width).
- **E) Barra intacta y funcional** (Resaltar/Boli/↶/●/━/→); elegir Boli/
  Resaltar en la barra sincroniza el modo persistido del boli.
- **Fixes de la verificación en tablet**: (1) el dedo volvió a pasar página
  (el Down reestructurado hacía pan a todo contacto — ahora solo con
  herramienta activa); (2) el resaltador dibuja un RECT LIBRE (bbox del
  trazo, altura de línea 13 pt) cuando no hay texto bajo el trazo — el boli
  SIEMPRE pinta en modo Highlighter.
- **Pulido (2ª iteración)**: (1) INDICADOR DE MODO en el visor: pill
  minimalista abajo a la derecha con ✏️/🖍️ (`draw::render_mode_badge`, cacheada
  en `Reader::mode_badge`, se refresca al alternar modo) — el usuario SIEMPRE
  ve con qué va a dibujar; (2) el borrado del SUBRAYADO ahora usa barrido
  (muestreo del segmento goma anterior→actual en `trim_highlight`): una goma
  rápida ya no "salta" por encima de una línea de 13 pt sin recortarla.
- **Pulido (3ª iteración, feedback del usuario)**: (1) ICONOS MINIMALISTAS
  lineales (estilo del pack "minimalista" de IconScout, sin emojis): el boli
  es una PLUMA diagonal dibujada con el rasterizador de tinta (cuerpo ancho +
  punta fina) y el resaltador es el símbolo de TEXT0 SUBRAYADO (barra +
  patillas finas); (2) BORRADO CON GOMA REAL MEJORADA: `split_stroke` recorta
  los SEGMENTOS por intersección exacta círculo→segmento (el hueco es
  exactamente el círculo de la goma, no un "mordisco" de vértices) con
  barrido multi-círculo muestreado; (3) CURSOR DE GOMA visible durante el
  borrado (círculo del tamaño real de la goma que sigue al boli) — el usuario
  ve exactamente qué se va a borrar.

## 2026-08-25 — Buscador con TECLADO real (sin chips A-Z) + biblioteca minimalista

El buscador de la biblioteca pasa de chips de letra/carpeta a TEXTO LIBRE con
el teclado del sistema, y la biblioteca queda en solo libros + buscador
(estilo Readest):

- **Teclado del sistema en NativeActivity** (`jni.rs` + `tools/ime/`): el
  backend native-activity de android-activity 0.6 no entrega texto por la vía
  nativa (set_text_input_state es NOP; TextEvent solo en game-activity, que
  exige Activity Java). Vía implementada: helper Java MÍNIMO
  (`tools/ime/ImeHelper.java`) compilado a dex con las tools del SDK
  (`tools/ime/build.sh`, javac + d8 + android.jar), EMBEBIDO en el binario
  (`include_bytes!`) y cargado en runtime con `DexClassLoader` desde
  `files/ime/` (el dex se marca 0444: ART rechaza dexes writable). El helper
  crea un EditText invisible en el hilo UI (`runOnUiThread`) y abre el IME;
  Rust hace polling del texto en `tick` (`needs_tick` + ime_active).
- **UI del buscador**: tocar "Buscar por título o carpeta..." abre el teclado;
  el texto filtra la rejilla por subcadena case-insensitive MIENTRAS se
  escribe; "✕" limpia y cierra el teclado; sin coincidencias → "Sin
  resultados — toca ✕ para limpiar". Eliminados de la UI los chips A-Z y de
  carpetas (código conservado, inactivo).
- **Fix**: abrir un libro curado desde la rejilla (la uri es la RUTA LOCAL;
  antes intentaba copiarla como content:// → "No content provider").

## 2026-08-25 — Biblioteca minimalista estilo Readest (rejilla + buscador)

La biblioteca se simplifica a SOLO libros + buscador, siguiendo el lenguaje
visual de Readest:

- **Se eliminaron de la UI la sección "Continue Reading" (recientes/leyendo
  ahora) y los chips de ORGANIZACIÓN (sort "Recientes/Los leídos/Título/Autor"
  y filtro "Todos/En lectura/Terminados/Por leer")** (`draw.rs`, `reader.rs`, `input.rs`):
  la rejilla de portadas empieza justo bajo el campo de búsqueda
  (`lib_grid_y0` = 8 px), sin títulos de sección intermedios.
- El **buscador** se mantiene como único filtro: campo "Buscar por título o
  carpeta..." + panel de chips de letra A-Z / carpetas (sin teclado, limitación
  de native-activity documentada).
- El orden de la rejilla queda FIJO por "añadido más reciente primero"
  (`LibSort::RecentlyAdded` por defecto); los libros curados aparecen arriba.
- Código del carousel/organización conservado con `#[allow(dead_code)]`
  documentado (no se borra lógica, solo se desactiva de la UI).

## 2026-08-25 — Biblioteca curada: altas por selector + tope fijo LRU (50)

La biblioteca deja de ser un escaneo de MediaStore y pasa a ser CURADA:

- **Arranque sin intent sin escaneo** (`reader.rs`, `lib.rs`): `Reader::new`
  ya no llama a `rescan_library`; muestra SOLO lo registrado en
  `internal/library.json` (`reload_curated_library`). Vacía → empty state
  "Tu biblioteca está vacía + Añadir PDF". `Resume` tampoco re-consulta
  MediaStore. Migración one-shot: instalaciones antiguas con PDFs en
  `internal/pdfs/` y registro vacío se auto-importan (added = ahora).
- **Selector "＋ Añadir" GESTOR DE ARCHIVOS**: el botón de cabecera consulta
  MediaStore en una lista TEMPORAL (`select_list`, nunca `library_list`) y
  abre un navegador por CARPETAS (árbol de `RELATIVE_PATH`): las carpetas
  del nivel actual primero (📁, en `primary`), los PDFs después (📄), con
  barra de breadcrumb "⬆ …" para subir de nivel y "Atrás" para cancelar.
  Se evita así la lista plana inabarcable de MediaStore. Tap en un PDF =
  copiar a `internal/pdfs/` + contar páginas (MuPDF) + registrar progreso
  (tope LRU aplicado).
- **Tope fijo con evicción LRU** (`persist.rs`): `LIBRARY_MAX = 50`;
  `enforce_library_limit(books, max)` (pura, con tests) devuelve las
  expulsadas del menos recientemente leído al menos (nunca leído →
  ordena por `added_unix`). Al añadir el 51º se borra fichero + portada
  cacheada del expulsado y se muestra toast "Biblioteca llena — se eliminó X".
- **Portadas por ruta local**: las entradas curadas usan su ruta local como
  clave de portada; el pump renderiza por ruta (`render_thumb_path`) en vez
  de fd content://. `ThumbCache::remove(key)` nuevo para purgar la portada
  de un libro evictado (sin tocar la política LRU existente).
- **Eliminado `rescan_library`**: la rejilla jamás lista PDFs del sistema por
  sí sola ([7] verificado con 200 PDFs en Download — solo aparecen curados).

## 2026-08-25 — Iteración 3 — cierre de defectos visuales

Cierre de defectos visuales, bugs residuales y pulido perceptual en `crates/pdf_android`:

- **Bugs corregidos**:
  - **[A1] Eliminación del botón fantasma ✏️**: Eliminado el renderizado residual de `tool_fab` y su captura de toques `tool_fab_tap` en el visor, dejando la esquina superior derecha completamente limpia.
  - **[A2] Swatch de tema en Top Bar**: Incorporado padding interno $\ge 18$ px en `viewer_top_chrome_buttons`, garantizando que el swatch circular quede 100% dentro de la tarjeta flotante sin cortar el radio de curvatura.
  - **[A3] Paleta Default-Dark & Sepia-Dark**: Alineados los valores derivados de Readest en `Default-Dark` (`#141414` base-200, `#242424` base-100, `#77BBEE` primary) y `Sepia-Dark` (`#201B15` base-200, `#342E25` base-100, `#48D1CC` primary).
- **Pulido perceptual**:
  - **[B1] Barra de progreso**: Implementado track de ancho completo en `base-300` con fill `primary` redondeado y separación vertical $\ge 8$ px del texto de autor (eliminado el aspecto de enlace subrayado en rejilla y Continue Reading).
  - **[B2] Sheet sin espacio muerto**: Altura de panel ajustada al 42% de `win_h` y distribución vertical uniforme (`space-evenly`) de las 3 secciones (`TEMA`, `LECTURA`, `DOCUMENTO`).
  - **[B3] Contraste de tarjetas en dark mode**: Delta de luminosidad perceptible de +10% en superficies `base-100` frente al fondo `base-200`, reforzado con borde 1px `base-300` y sombra multicapa `0x70000000`.
  - **[B4] Swatches informativos en selector de temas**: Círculos de previsualización que muestran el color real de fondo del tema (`#FFFFFF`, `#F1E8D0`, `#242424`, `#342E25`) con anillo `primary` de 2px en el tema activo.
- **Rendimiento**: Scroll de biblioteca mantenido en 3.9 ms – 6.5 ms por frame blit (muy inferior al límite de 33 ms).
- **Evidencias visuales**: Generadas 6 capturas en `/tmp/`: `pl3-biblio-light.png`, `pl3-biblio-default-dark.png`, `pl3-biblio-sepia-dark.png`, `pl3-viewer-chrome-light.png`, `pl3-sheet-light.png`, `pl3-sheet-dark.png`.

## 2026-08-25 — Iteración 2 — fidelidad visual

Pasada exhaustiva de fidelidad visual y calidad perceptual sobre `crates/pdf_android` adaptando al detalle el lenguaje visual de Readest, con superficies separadas, elevación con sombras multinivel, marcos con contención 2:3 y barras flotantes:

- **Biblioteca (`draw.rs`, `reader.rs`, `lib.rs`)**:
  - **B1 Superficies separadas**: Fondo de biblioteca en `base-200` (`#F2F2F2` en light, `#141414` en dark); tarjetas, barras y filas en `base-100` (`#FFFFFF` en light, `#1C1C1C` en dark) con sombras de elevación y bordes de 1px `base-300`.
  - **B2 Portadas en marco 2:3**: Renderizado `CONTAIN` centrado dentro del marco 2:3 con padding interno de 8px, bordes de papel y sombreado de lomo, evitando el corte/sangre total de celda.
  - **B3 Rejilla con respiro**: Gutter horizontal $\ge 3.5\%$ de `win_w` (50px en 1440w), títulos en 14sp peso 600 `base-content` bajo el marco, autor en 12sp `neutral-content`, y barra de progreso de 4px en `base-300` con fill `primary`.
  - **B4 Continue Reading rediseñado**: Tarjetas horizontales (alto 15% `win_h`, ancho 52% `win_w`) con marco 2:3 a la izquierda, título en 17sp bold `base-content`, autor en 12sp `neutral-content`, progreso separado de 4px, metadatos y botón de píldora `"Continuar"` relleno en `primary` con texto de alto contraste.
  - **B5 Chips de Búsqueda y Organización**: Píldoras con altura $\ge 40$ px (44px), inactivas con fondo `base-100` y borde `base-300`, activas con fondo `primary` y texto en contraste.
- **Visor (`draw.rs`, `reader.rs`, `input.rs`)**:
  - **V1 Barras Flotantes**: Top bar y Bottom bar como tarjetas flotantes con radio de 24px, sombra de elevación multinivel y márgenes $\ge 2\%$ `win_w` respecto a los bordes superior, inferior y laterales.
  - **V2 Botones Táctiles**: Áreas táctiles de todos los controles $\ge 48\times 48$ px con glifos claros e interactivos.
  - **V3 Barra Inferior con Slider**: Tarjeta flotante con texto `"Pág. N de M · P%"` en 13sp `base-content`, pista de 6px radio completo en `base-300`, fill `primary`, y thumb circular de 22px `primary` con borde de 2px `base-100`.
  - **V4 Top Bar**: Píldora `"← Biblioteca"` ($\ge 48$ px alto), título del libro en 16sp peso 600 `base-content`, y swatch circular de 26px del color `primary` activo con anillo interactivo de selección de tema.
- **Sheet de Ajustes (`draw.rs`, `reader.rs`, `input.rs`)**:
  - **S1 Panel Real**: Panel con altura 55-60% de `win_h` (~1276px en 2200h), radio de 24px en el borde inferior y sombra proyectada.
  - **S2 Secciones Estructuradas**: Encabezados de sección en 11sp mayúsculas `neutral-content` (`TEMA`, `LECTURA`, `DOCUMENTO`), fila de 4 swatches circulares de 26px (`Claro`, `Sepia`, `Oscuro`, `Sepia D.`) con anillo activo de 2px `primary`, fila de navegación (`◀ −10`, `Pág. N / Total`, `+10 ▶`), y fila de acciones (`← Biblioteca`, `🔍 Buscar`, `✕ Cerrar`).
  - **S3 Dimensiones de Botones**: Todos los botones del sheet con altura $\ge 48$ px.
- **Global (G1-G3)**:
  - **G1**: Sombras multinivel visibles en temas claros y oscuros (alpha $\ge 0x60$ + borde `base-300`).
  - **G2**: Tipografía $\ge 12$ sp en todo el texto informativo (únicamente labels en mayúsculas a 11sp).
  - **G3**: Contraste garantizado con texto informativo en `base-content`.
- **Rendimiento**: Frame time en scroll de biblioteca entre 3.8 ms y 6.1 ms (muy inferior a 33 ms).
- **Evidencias visuales**: 6 capturas generadas en `/tmp/`: `pl2-biblio-light.png`, `pl2-biblio-dark.png`, `pl2-viewer-chrome-light.png`, `pl2-viewer-chrome-dark.png`, `pl2-sheet-light.png`, `pl2-biblio-sepia.png`.

## 2026-08-24 — Restyling visual de PDFLector (Android) al look&feel de Readest

Reemplazo completo del sistema visual en `crates/pdf_android` adoptando los principios de diseño, paletas y jerarquía tipográfica de Readest (https://github.com/readest/readest).

- **Sistema de Temas (`lib.rs` `mod theme`)**: Implementados 4 temas completos derivados de la fórmula Readest (`base-100`, `base-200`, `base-300`, `base-content`, `neutral`, `neutral-content`, `primary`, `primary-content`): `Default-Light`, `Sepia-Light`, `Default-Dark`, `Sepia-Dark`. Los 4 temas son ciclables de forma interactiva y persistentes en `ViewerState`. Se eliminaron todos los literales de color hardcodeados fuera de `mod theme` (0 ocurrencias de `0x[0-9A-Fa-f]{8}` fuera de `mod theme`).
- **Chrome del Visor y Navegación (`reader.rs`, `draw.rs`, `input.rs`)**:
  - Taps por tercios horizontales: tercio izquierdo = página anterior, tercio derecho = página siguiente, tercio central = alternar chrome.
  - Barra superior fina: botón estilo píldora `← Biblioteca`, título del documento truncado con elipsis y botón de tema actual (`Default-Light`, `Sepia-Light`, etc.).
  - Barra inferior de progreso: pista estilizada de 3px con relleno de color `primary` y etiqueta `"Página N de M · P %"` en tipografía 12sp `neutral-content`. Auto-ocultado automático tras ≤2.5s de inactividad.
  - Indicador badge mínimo (`N / M`) visible únicamente cuando el chrome está oculto.
- **Sheet de Ajustes (`draw.rs` `render_sheet`)**: Rediseñado con tokens Readest, esquinas redondeadas (16px), título de sección en 11sp mayúsculas (`FONT_LABEL_CAPS`) y control de tema activo destacado en color `primary`.
- **Biblioteca y Componentes (`draw.rs`)**: Biblioteca personal (cabecera, barra de búsqueda, sección "Leyendo ahora", carrusel, chips de ordenación y filtrado, rejilla "Tu Colección", empty state), toasts, FAB y menús adaptados a los tokens y jerarquía tipográfica Readest (`FONT_DISPLAY` 24sp, `FONT_TITLE` 17sp, `FONT_BODY` 14sp, `FONT_CAPTION` 12sp, `FONT_LABEL_CAPS` 11sp).
- **Archivos editados**: `crates/pdf_android/src/{lib.rs, draw.rs, input.rs, reader.rs, persist.rs}`.
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` (0 warnings), `cargo clippy -p pdf_android --target aarch64-linux-android --release -- -D warnings` (limpio), `cargo fmt --all -- --check` (limpio), `cargo check -p pdf_core -p pdf_app -p pdf_bench` (limpio). APK instalado y probado en TCL 9469X con 5 capturas verificadas en `/tmp/`.

## 2026-08-18 — Restyling visual de la UI de Android: paleta warm-neutral, card de ajustes y portadas en rejilla

- **Paleta de color (`lib.rs` `mod theme`)**: migrada a tonos cálidos/neutros premium (`DARK_BAR_BG` = `0xFF0B0D12`, `DARK_BAR_BORDER` = `0xFF232B3A`, `DARK_BTN_BG` = `0xFF161B26`, `DARK_BTN_BORDER` = `0xFF2A3444`, `DARK_BTN_TEXT` = `0xFFE6EAF0`, `ACCENT` warm gold `0xFFC8A96A`/`0xFFD9BD8B`, `DARK_BADGE_BG` = `0xDD0B0D12` semitransparente, `LIB_BG` = `0xFF0B0D12`, `LIB_ROW_EVEN/ODD` = `0xFF10141C`/`0xFF141922`, `LIB_TEXT_PRIMARY/SECONDARY/MUTED`).
- **Sheet de ajustes (`draw.rs` `render_sheet`)**: panel card deslizante desde el borde superior con esquinas inferiores redondeadas (16px), 1px de borde `bar_border`, etiqueta "SETTINGS" en mayúsculas (11sp `LIB_TEXT_SECONDARY`), y botones estilo píldora (radio capsule $H/2$, 1px borde, `DARK_BTN_BG`/`DARK_BTN_TEXT`; toggle de modo oscuro con relleno `ACCENT` warm gold y texto oscuro en estado activo).
- **Biblioteca en rejilla (`draw.rs` `render_library_grid` / `paste_thumb`)**: portadas en celdas de 3 columnas con esquinas redondeadas 12px, 1px de borde `LIB_ROW_BORDER`, escaladas con *scale-to-fill* (sin letterbox), y título de 14sp a 1 sola línea con puntos suspensivos abajo (`LIB_TEXT_SECONDARY`).
- **Indicador de página (`draw.rs` `render_page_badge`)**: badge estilo píldora en la esquina inferior izquierda con fondo oscuro semitransparente (`DARK_BADGE_BG` `0xDD0B0D12`), 1px de borde y texto 12sp (`DARK_BADGE_TEXT`).
- **Archivos editados**: ÚNICAMENTE `crates/pdf_android/src/lib.rs` y `crates/pdf_android/src/draw.rs`.
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` (0 warnings), `cargo fmt --all -- --check` (limpio), `cargo clippy --all-targets -- -D warnings` (limpio).


## 2026-08-13 — Visor a UNA SOLA HOJA + sheet fluido + estudio legal

- **Modo una sola hoja** (el autor no quería la columna de páginas apiladas): eliminada toda la
  geometría de columna (page_offsets/page_heights/doc_height/scroll_y/layout_dirty/pending_page,
  PAGE_GAP, visible_pages, rebuild_layout, clamp_scroll, update_page_from_scroll, blit_stacked) →
  sustituida por `draw::blit_page`: el visor blitea SOLO la página actual (centrada cover + pan
  anclado, recortada a sus bordes). Se conserva PageCache LRU con prefetch ±1 (no se dibuja) y el
  zoom relativo/anclado.
- **Sheet fluido**: la causa del lag era el re-blit de la página completa (~25-40 ms) por frame de la
  animación/arrastre. Fix: `draw::compose_frame` compone el frame de página UNA vez
  (fondo+página+anotaciones+indicador) y durante el deslizamiento `draw::blit_composed` solo hace
  memcpy (~1-2 ms) + overlay del sheet — la página ya no se re-blitea por paso.
- **Estudio legal preparatorio** en `docs/legal.md` (NO decisión, solo análisis para el futuro):
  punto de partida (LICENSE=AGPL-3.0-only efectivo; MuPDF AGPL-3.0-or-later), las 3 piezas
  (SPDX/REUSE, variante -or-later recomendada, NOTICE de atribución MuPDF/Artifex + terceros),
  cumplimiento AGPL al distribuir, y checklist de 8 pasos para ejecutar cuando haya versión.
- Sin commits (regla AGENTS.md: pendiente de que el autor lo pida).


## 2026-08-13 — Diagnóstico: "Activity no resuelve" en release APK (NO reproducible)

- **Síntoma reportado**: tras las olas Groq/Gemini/zoom, `am start -n com.pdflector.app/android.app.NativeActivity` daba "Error type 3: Activity class does not exist", `cmd package resolve-activity --brief com.pdflector.app` daba "No activity found", y ActivityTaskManager devolvía -92, pese a un AndroidManifest.xml correcto (aapt2 dump xmltree) y el .so en lib/arm64-v8a.
- **Bisección realizada (worktree 681b618, cargo-apk 0.10, NDK r28, aapt v1)**: el ÚNICO diff de manifest entre el commit conocido-bueno 681b618 y HEAD es el permiso INTERNET (`uses_permission android.permission.INTERNET`). Todo lo demás idéntico: hasCode=false, debuggable=false, exported=true, meta-data android.app.lib_name="pdf_android", intent-filters VIEW/application/pdf + MAIN/LAUNCHER.
- **Verificado en la tablet TCL 9469X (Android 15), USB**:
  - APK 681b618: `adb install -r` + `am start` → PID 8523, logcat "opened: 12 pages / restored / Resume / InitWindow".
  - APK HEAD con INTERNET (recompilado desde cero del Cargo.toml actual): → PID 8279, "opened: 328 pages / restored".
  - APK HEAD sin INTERNET (el instalado en target/release, byte-idéntico al que reportaba el fallo): uninstall+install limpio → PID 8116, arranca.
  - `cmd package resolve-activity` (paquete, MAIN/LAUNCHER y VIEW con content-URI) resuelve siempre; `monkey -p com.pdflector.app 1` → PID 8687.
- **Conclusión**: el fallo reportado NO se reproduce con el APK actual ni con el commit anterior; la activity se resuelve y arranca en todas las configuraciones probadas (con/sin INTERNET, install limpio). Sin cambio de metadatos necesario: `crates/pdf_android/Cargo.toml` intacto. El -92 (START_APP_STILL_STARTING) es un código de arranque diferido, no de actividad inexistente.
- **Verificación final**: `cargo build -p pdf_android --target aarch64-linux-android --release` 0 warnings, `cargo fmt --all -- --check` limpio. APK HEAD reinstalado en la tablet y arrancando (PID 8758). Sin commits (regla AGENTS.md).


## 2026-08-18 — Biblioteca personal premium: Continue Reading + My Library + progreso por libro + rect de selección rápido

Rediseño de la pantalla **Library** (`UiMode::Library`) como biblioteca PERSONAL (estilo Apple Books/Kindle pero propio): las PORTADAS mandan; se eliminó el aspecto de file manager (sin rutas completas visibles, sin iconos PDF genéricos: placeholder elegante con el título mientras carga la portada). Solo se tocó `crates/pdf_android/` (lib/draw/input/reader/persist); pdf_core/pdf_app/pdf_bench intactos. Sin commits (regla AGENTS.md).

- **Modelo de datos de progreso por libro (`persist.rs`)**: `internal/library.json` = `Vec<BookProgress>` con `{path, page, page_count, last_read_unix, added_unix}` (clave = ruta local absoluta, la misma de `state.json`/`recents.json`). `Reader::save_state` (eager, cada cambio de página/apertura/zoom/dark) lo actualiza con `touch_progress`/`save_progress`/`load_progress`/`progress_for` (funciones puras con tests `#[cfg(test)]`; no ejecutables en host: `ndk-sys` solo compila para Android — `cargo check --tests --target aarch64-linux-android` valida la compilación). Derivados SIN abrir el PDF: `title` = nombre sin extensión (`title_from_name`), `author` = primer segmento de RELATIVE_PATH o "PDF" (`entry_author`), `progress% = (page+1)/page_count` (`pct`), `status` = Unread/Reading/Finished (`book_status`, `is_finished` = última página alcanzada). `open_pdf_at(path, start_page)` reanuda en la página guardada (Continue Reading y rejilla); `open_library_entry` y el carousel pasan la página del registro.
- **Cabecera editorial**: título "Library" grande (hasta 44 px, negrita), botón **"＋ Add book"** (rescan MediaStore + toast "Add PDFs to Downloads, then open with PDFLector" dibujado como overlay con un segundo lock+present vía `draw::blit_overlay`, solo ~1,5 s), y **campo de búsqueda** (píldora) que al tocarla abre el panel de chips letra A-Z/# + carpeta (la búsqueda SIN teclado de siempre, ahora tras un campo; ✕ limpia filtros y cierra). El botón "Search" del sheet abre la biblioteca con el panel desplegado.
- **Continue Reading** (sección destacada, oculta si no hay libros): carousel horizontal de tarjetas (fondo `LIB_CARD_BG`, portada 2:3 grande con sombra, título, autor, barra de progreso dorada, "Page X of Y · Z%", botón "Read"); tocar la tarjeta abre el libro en su página guardada. Construido desde `recents.json` + `library.json` (`lib_continue_reading`); respeta el filtro de letra y, trivialmente, el de estado (All/Reading muestran la sección; Finished/Unread la ocultan).
- **My Library** (rejilla principal 3×3): portada 2:3 dominante + título (primario) + autor (muted) + barra fina de progreso si el libro está empezado. **Organización** discreta bajo el título: chips de SORT (Recently Added / Recently Read / Title / Author) y FILTER (All / Reading / Finished / Unread), que reordenan/filtran `lib_filtered` (`refresh_lib_filtered` con sort precomputado). **Empty state** si no hay PDFs: ilustración de libro (rects Canvas) + "Your library is empty" + "Add your first PDF to start reading." + botón "Add PDF" (o "Grant access" si falta el permiso MANAGE_EXTERNAL_STORAGE; `rescan_library` ya no cae al picker cuando no hay PDFs en ningún sitio).
- **Constantes de tema nuevas (`lib.rs` `mod theme`)**: `LIB_ACCENT`/`LIB_ACCENT_DARK` (dorado + texto oscuro), `LIB_CARD_BG`/`LIB_CARD_BORDER`, `LIB_SEARCH_BG`/`LIB_SEARCH_BORDER`, `LIB_PROGRESS_TRACK`. Reutiliza `LIB_COVER_SHADOW`/`LIB_COVER_PLACEHOLDER`/`LIB_TEXT_*`/`ACCENT_AMBER_*` existentes.
- **Rect de selección MÁS RÁPIDO** (petición del autor; medible con el log de `Reader::blit` durante el arrastre): `draw::fill_rect_bordered` usa ahora un camino bpp 4 con alfa-blend por LUT (`fill_rect_lut`, 2 tablas de 256 construidas una vez por llamada — sin división ni llamada por píxel, relleno por filas, borde como 4 rectas) en vez del `stamp` por píxel con división por 255; y `draw::blit_page_scaled` tiene un camino 1:1 (zoom == 1.0) que copia la página FILA a FILA (memcpy) sin la tabla x ni el bucle de vecino-más-cercano — el arrastre de selección re-blitea la página en cada Move. Sin regresión visual (el blend LUT se desvía ±1 por canal en overlays translúcidos).
- **Verificación**: `cargo build -p pdf_android --target aarch64-linux-android --release` 0 warnings (NDK r28 + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`), `cargo clippy -p pdf_android --target aarch64-linux-android --release` limpio, `cargo fmt --all -- --check` limpio, `cargo check -p pdf_android --target aarch64-linux-android --release --tests` OK, host check de pdf_core/pdf_app/pdf_bench OK. Pendiente: medición en tablet del frame time del rect de selección y del render de la biblioteca (hook: log `blit ... ms` de `Reader::blit`).


## 2026-08-22 — Fase 3.5: anotaciones con lápiz (resaltado + boli) y auditoría del pipeline de render en tablet

Trabajo completo en el worktree de la rama `marakihau` (sin commits hasta esta sesión, en la que el autor pidió commitear). Ver `docs/api-anotaciones-fase3.md` (API de motor), `docs/api-anotaciones-ui.md` (UI Android, con prueba manual pendiente) y `docs/benchmark-results.md` (auditoría completa).

- **`pdf_core` — API de anotaciones (lógica pura, sin UI, en coords de página)**:
  - `selection.rs` (nuevo): resaltador con detección de texto — `highlight_under_gesture(spans, gesture, color)` selecciona las líneas (`TextSpan`) cuyo bbox intersecta el gesto (puntos o marquee) y recorta el rect al tramo horizontal del trazo.
  - `overlay.rs` (nuevo): composición de la capa de anotaciones sobre la página.
  - `annotations.rs`: `smooth_polyline` (Catmull-Rom, función pura, pasa por los vértices, preserve endpoints) + tests.
  - `prefetch.rs` + tests ampliados (`prefetch.rs`, `annotations_pipeline.rs`).
- **`pdf_android` — UI de anotación y rendimiento de biblioteca**:
  - Barra de herramientas flotante (píldora centrada arriba): Resaltar / Boli / ↶ deshacer / ● color / → volver a navegación; botón flotante ✎/✕; ocultar la barra vuelve a modo NAVEGACIÓN (decisión documentada).
  - Gestos de herramienta: un dedo o lápiz dibuja en vez de navegar (`begin/update/end_tool_gesture`), umbral `TOOL_MIN_PX = 6` descarta toques sin intención; segundo dedo cancela el gesto; long-press de selección de texto desactivado con herramienta activa.
  - Rendimiento de la biblioteca: scroll por bandas cacheadas (`lib_header` + `lib_band`, blit por filas ~1-3 ms) en vez de re-renderizar toda la pantalla por Canvas+JNI (~20-60 ms → el lag/parpadeo reportado); transición `lib_fade` (0,18 s) al abrir libro; `TOOL_MIN_PX`.
- **Benchmarks (`pdf_bench`)**: benches nuevos `blit.rs` (espejo de las primitivas CPU de `draw.rs`; pdf_android no compila en host, hay que mantenerlo en sync) y `prefetch.rs`. Auditoría completa en `docs/benchmark-results.md`.
- **Mediciones (2026-08-22)**: hardware AMD Ryzen 7 5800H (16 hilos), Rust 1.97.1, release, carga ambiental ALTA (load avg ~5-7 → varianza ±10-20 % en rutas sin tocar). Corpus regenerado el mismo día 18:24 (absolutos NO comparables con 2026-08-05/13; comparaciones dentro del mismo día). Baseline del inventario documentado (open 38-67 µs, render 1x 4,1-6,8 ms, cache_scroll 95,9-131,9 ms…); las rutas optimizadas ganan **31-89 %** (blit/prefetch) por encima del ruido. Sweep de humo: PEAK_RSS 30 072 KB.
- **Verificación**: `cargo test -p pdf_core` todo en verde (44+21+8+7+9+7+5+5+2+4 tests según binario) y `cargo check -p pdf_core` OK. `pdf_android` NO compila en host (android-activity) → **pendiente: prueba manual en la tablet** (barra de herramientas, gestos de boli/resaltado, undo, scroll de biblioteca sin parpadeo) y medición de frame time en dispositivo.


---
[Historial completo](docs/log/memory-2026-08.md)
