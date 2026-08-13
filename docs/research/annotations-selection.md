# Investigación — Selección de texto y anotaciones en lectores PDF

> Investigación de referencia para PDFLector (Fase 3: anotaciones). Estudia cómo
> los lectores reales implementan (a) la **selección de texto** —de dos puntos
> del puntero a texto + rects de subrayado— y (b) las **anotaciones**
> (subrayado, tinta/dibujo, notas) y cómo las **persisten** (PDF nativo vs
> sidecar).
>
> **Fuentes** (clonados en `/tmp/pdfresearch/` durante esta sesión):
>
> | Repo | Lenguaje | Motor | Licencia | Rol para nosotros |
> |------|----------|-------|----------|-------------------|
> | [koreader/koreader](https://github.com/koreader/koreader) | Lua + FFI | MuPDF | AGPL-3.0 | Lector de e-ink con lápiz; selección por palabras + subrayado + guardado dual (sidecar + PDF). El más parecido a PDFLector. |
> | [ArtifexSoftware/mupdf](https://github.com/ArtifexSoftware/mupdf) | C | — | AGPL-3.0 | Motor de PDFLector (crate `mupdf` 0.8). Aquí viven los algoritmos: `fz_highlight_selection`, `fz_snap_selection`, anotaciones Ink/Highlight/Text. |
> | crate `mupdf` 0.8 (messense) | Rust | MuPDF | AGPL-3.0 | El crate que ya usa `pdf_core`. Confirma qué API expone (selección y anotaciones ya disponibles). |
> | [invent.kde.org/graphics/okular](https://invent.kde.org/graphics/okular) | C++/Qt | Poppler | GPL-2.0+ | Visor KDE: selección por palabras (`TextPage`), anotaciones nativas PDF, guardado a temp + rename. |
> | [mozilla/pdf.js](https://github.com/mozilla/pdf.js) | JS | propio | Apache-2.0 | Selección vía capa DOM + editor de anotaciones (highlight/ink) con `QuadPoints`. |
> | [ajrcarey/pdfium-render](https://github.com/ajrcarey/pdfium-render) | Rust | PDFium | MIT | Backend histórico (Fase 0, eliminado en ADR-001). Referencia de API Rust para anotaciones. |
> | [barteksc/AndroidPdfViewer](https://github.com/barteksc/AndroidPdfViewer) | Java | PDFium | Apache-2.0 | Contrapunto: renderiza anotaciones **dentro del bitmap** (`annotationRendering`), sin selección ni creación. |
> | Evince (ya estudiado en `evince-architecture.md`) | C/GTK | Poppler | GPL-2.0+ | Solo lectura: selección de texto por palabras, sin anotaciones. |

**Búsquedas GitHub ejecutadas** (2026-08-13): `gh search repos "pdf reader annotations"`,
`"pdf reader rust"`, `"mupdf rust"`, `gh search code "Annotation" --language Rust --limit 10`
(el código search da ruido: "Annotation" es genérico en Rust; los resultados útiles
fueron los repos concretos clonados). El panorama de lectores con anotaciones en
Rust es casi vacío: los reales son C/C++/Lua/JS; el camino Rust pasa por el crate
`mupdf` (ya elegido en ADR-001).

---

## 1. Resumen ejecutivo (qué copiar tal cual)

1. **Selección = dos puntos → índice de carácter en orden de lectura → rects por línea.**
   MuPDF ya implementa esto completo: `fz_highlight_selection(stext, a, b, quads)`
   devuelve un quad **por línea seleccionada** (fusionando caracteres contiguos con
   tolerancia), y `fz_copy_selection` el texto en orden de lectura. El crate
   `mupdf` expone `TextPage::highlight_selection` **tal cual**. Es la forma de
   arreglar el subrayado-caja de PDFLector sin inventar nada.
2. **El subrayado se guarda como quads/rects por línea, nunca como un rect de
   selección.** KOReader (`getTextFromBoxes`), Okular (`TextPage::textArea`) y
   pdf.js (`HighlightOutliner`) producen: primera línea = desde la palabra inicial
   hasta final de línea, últimas = líneas completas, última = desde inicio de
   línea hasta la palabra final. PDFLector ya tiene `Highlight { rects: Vec<Rect> }`
   con varios rects por anotación: solo falta llenarlos con el algoritmo.
3. **Doble almacenamiento: sidecar como fuente de verdad + escritura en el PDF
   (opcional o en exportación).** KOReader guarda en el sidecar `.sdr/settings.reader.lua`
   y, si `highlight_write_into_pdf`, escribe anotaciones PDF reales al cerrar
   (MuPDF). Okular escribe solo en el PDF (Poppler). pdf.js guarda en memoria
   (`AnnotationStorage`) y las vuelca al PDF al exportar. PDFLector ya tiene
   sidecar SQLite (Fase 3) y `export_pdf_annotated` (Fase 3): la decisión abierta
   es si además escribir en el PDF de forma incremental.
4. **La tinta es polilíneas en coordenadas de página** con grosor y color — el
   formato PDF Ink (`pdf_set_annot_ink_list`) y el `Stroke` de PDFLector coinciden
   1:1. Lo que falta es suavizado/decimación y (en lápiz) presión.
5. **Capa vectorial sobre el bitmap, sí; pero MuPDF puede rasterizar anotaciones
   con un flag.** `fz_run_page` = contenido + anotaciones; el crate expone
   `Page::to_display_list(annotations: bool)` y `run_annotations(device, ctm)`.
   AndroidPdfViewer rasteriza anotaciones en el bitmap (PDFium); KOReader dibuja
   su propio overlay raster sobre el tile cache **y además** escribe anotaciones
   PDF reales que MuPDF renderiza al reabrir. Nuestra capa vectorial (AGENTS.md §4.3)
   es la opción correcta para no invalidar la caché; conviene saber que el
   alternativo existe.

---

## 2. Selección de texto

### 2.1 MuPDF: el algoritmo de referencia (lo que ya tenemos debajo)

MuPDF extrae el texto como **stext page**: bloques → líneas → caracteres, cada
carácter con su `quad` (caja con 4 esquinas, puede estar rotada), `origin` y
`size`; cada línea con `bbox`, `wmode` (0 horizontal / 1 vertical) y `dir`
(vector de la línea base) — `include/mupdf/fitz/structured-text.h:467`.

La selección es un único pase:

- `fz_snap_selection(stext, &a, &b, mode)` (`source/fitz/stext-search.c`) — dados
  dos puntos, encuentra el **carácter más cercano** a cada uno e **"chupa"** los
  puntos de anclaje al origen/quad del carácter. Modos `FZ_SELECT_CHARS` /
  `FZ_SELECT_WORDS` (el ancla salta al inicio de palabra, y el final al espacio
  que la cierra) / `FZ_SELECT_LINES` (ancla al inicio de línea).
- `fz_enumerate_selection` — indexa todos los caracteres en **orden de lectura**
  (recorre bloques → líneas → caracteres tal como los dejó el análisis stext;
  el analizador ya ordena las líneas por posición, no por orden de pintado),
  encuentra el índice de los dos anclajes, los ordena (`start > end` → swap) y
  recorre del primero al último invocando `on_char`/`on_line`.
- `fz_highlight_selection` (`source/fitz/stext-search.c:453`) — sobre ese
  recorrido, **fusiona caracteres contiguos en un quad por línea**: si el
  siguiente carácter está cerca del extremo actual del quad (`hfuzz = 0.5 × size`
  horizontal, `vfuzz = 0.1 × size` vertical) lo estira; si no, abre un quad nuevo.
  Resultado: N quads, típicamente uno por línea (o por fragmento de línea).
- `fz_copy_selection` — mismo recorrido, produce el texto plano en orden de
  lectura (espacios y saltos de línea, con opción CRLF).

**Cita clave** (`source/fitz/stext-search.c`):

```c
/* on_highlight_char: estira el quad actual si el siguiente carácter está cerca */
static int is_near(float hfuzz, float vfuzz, fz_point hdir, fz_point end,
                   fz_point p1, fz_point p2) { ... }
```

El crate `mupdf` (el nuestro) expone exactamente esto:

- `TextPage::highlight_selection(&mut self, a: Point, b: Point, quads: &[Quad]) -> Result<i32>`
  (`src/text_page.rs:476`) — buffer de salida de quads, devuelve el número usado.
- `TextPage::search(needle) -> Vec<Quad>` y `search_cb` — el mismo mecanismo para
  buscar (los hits son quads reutilizables como rects de subrayado).
- `TextPage::structured()` → `StructuredTextBlock`/`StructuredTextLine`
  (`bounds`, `wmode`, `text`, `chars`) y `TextPage::words()` → `TextWord`
  (`text`, `bounds`, `block`, `line`, `word`) — lo que usa hoy `pdf_core` para
  `PageText.spans` (`engine/mupdf.rs:123`), solo que a nivel de línea.

**No expone** `fz_snap_selection` ni `fz_copy_selection` directamente; el
"snapping" hay que hacerlo sobre `structured()`/`words()` (elegir la palabra
más cercana al punto) o vivir sin él (los anclajes se quedan donde tocó el dedo,
y `highlight_selection` recorta por caracteres).

### 2.2 KOReader: selección por palabras sobre stext

`frontend/document/pdfdocument.lua:239` — `getPageTextBoxes` cachea por página
(`DocCache`, clave `"textbox|<file>|<pageno>"`) el resultado de
`page:getPageText()` (stext de MuPDF): **cada página se extrae una vez y se
reutiliza para selección, buscar y subrayado** (PDFLector ya hace lazy, pero sin
caché del stext — se re-extrae en cada `text(page)`).

`frontend/document/koptinterface.lua`:

- `getWordBoxIndices(boxes, pos)` (línea 996) — la palabra más cercana al punto
  por distancia al bbox.
- `getTextFromBoxes(boxes, pos0, pos1)` (línea 1042) — el algoritmo de selección:
  1. Índices `(i,j)` de palabra inicial y final; si están invertidos, swap.
  2. Recorre líneas `i_start..i_stop`; para la **primera** línea el rect va de la
     palabra inicial al **final de línea** (`x = wb.x0`, `w = lb.x1 - wb.x0`), para
     las **intermedias** el rect es la **línea completa**, y para la **última** va
     del **inicio de línea** a la palabra final.
  3. El texto se une con espacios entre palabras, **quitando el guion final de
     línea** (`line_text:sub(-1) == "-"`) y el guion blando (`\u{00AD}`), con
     heurística CJK (sin espacio entre dos caracteres CJK adyacentes cuyo hueco
     es < 80 % de la altura de línea).

Devuelve `{ text, boxes }` donde `boxes` son **rects por línea en coordenadas de
página** — exactamente el input para `saveHighlight`.

### 2.3 Okular: TextPage (palabras) y selección geométrica

`core/textpage.cpp` — `TextPage` es una lista de `TextEntity` (palabras con
`NormalizedRect` en coords normalizadas 0..1 de página, no puntos PDF).
`TextPage::textArea(const TextSelection &sel)` (línea 277) — el algoritmo
documentado en el propio código: busca la palabra bajo cada cursor; si no la
hay, la primera a la derecha en la misma línea base, si no, la primera bajo el
cursor; construye la selección como **tres rects**: `(inicio → fin de línea 1) +
(líneas completas intermedias) + (inicio de última línea → fin)`. El mismo
resultado que KOReader y que `fz_highlight_selection`. `TextPage::text(area)`
extrae el texto del área, con manejo de guiones (`stringLengthAdaptedWithHyphen`,
línea 682).

**Lección Okular para selección**: trabajar en coordenadas **normalizadas de
página** (0..1) desacopla la selección del zoom/rotación. PDFLector trabaja en
puntos PDF (decision ya tomada en `annotations.rs`; ambas válidas, la nuestra es
la misma que MuPDF/KOReader).

### 2.4 pdf.js: selección "gratis" del navegador sobre una capa DOM

`src/display/text_layer.js` — pdf.js pinta cada ítem de texto (palabra/fragmento)
como un `<span>` posicionado con `transform` CSS sobre el canvas
(`#appendText`, línea 322). La **selección la hace el navegador** sobre esos
spans (DOM selection), no pdf.js. El resaltado de búsqueda (`web/text_highlighter.js`)
marca los spans con clases `highlight begin|middle|end` (línea 255-267).

El editor de resaltado (`src/display/editor/highlight.js`) convierte la selección
DOM en quads: `window.getSelection()` → geometría de los spans seleccionados →
`HighlightOutliner` (`src/display/editor/drawers/highlight.js:20`) calcula el
contorno y los quads por línea.

**Lección pdf.js**: si un día hay UI web, la selección DOM da gratis orden de
lectura, doble-tap de palabra y selección visual; el coste es mantener la capa de
texto alineada al render (lo que ya hace con `transform`). No aplica a Android
nativo, pero el patrón "capa de texto por encima del bitmap para selección" sí.

---

## 3. Subrayado (highlight)

### 3.1 Geometría: quads/rects por línea

Todos los lectores convergen en lo mismo:

| Lector | Estructura | Cita |
|--------|-----------|------|
| MuPDF | `fz_quad` por línea (fusión de caracteres) | `source/fitz/stext-search.c:453` `fz_highlight_selection` |
| KOReader | `pboxes`: rect por línea en coords de página | `koptinterface.lua:1042` `getTextFromBoxes` |
| Okular | `HighlightAnnotation::highlightQuads()` — lista de `Quad` (del PDF) | `core/annotations.cpp:2118` |
| pdf.js | quads → path SVG por quad | `src/display/annotation_layer.js:652` |
| PDFLector (actual) | `Highlight { rects: Vec<Rect> }` — **un solo rect** (caja de selección) | `crates/pdf_android/src/reader.rs` `highlight_sel` |

El formato PDF nativo guarda el subrayado como **`QuadPoints`** (8 floats por
quad: 4 esquinas), y MuPDF/KOReader convierten rect → quad con un orden concreto
de esquinas. **Cita del detalle** (`frontend/document/pdfdocument.lua:75`
`_quadpointsFromPboxes`): *"The order must be left bottom, right bottom, left
top, right top"* (bug de MuPDF 695130). Si exportamos a PDF (ya lo hace
`export_pdf_annotated`), respetar ese orden.

### 3.2 Mezcla (blend) y estilo

- KOReader dibuja su highlight con **blend MUL** (multiplicativo, oscurece poco
  el texto) o "lighten", con un factor configurable y altura de línea ajustable
  (`highlight_height_pct`); nota en `pdfdocument.lua:258`: *"we do a MUL blend,
  MuPDF currently appears to do an OVER blend"* — o sea, el render de MuPDF de la
  anotación PDF no se ve igual que el overlay propio.
- MuPDF guarda el color con `pdf_set_annot_color` (RGB) y la opacidad con
  `pdf_set_annot_opacity`; al renderizar la anotación la pinta como contenido
  PDF sobre la página (`pdf_process_annot`, `source/pdf/pdf-run.c:27`).
- PDFLector ya dibuja highlights translúcidos debajo de los trazos
  (`draw.rs:483` — "los highlights (rellenos translúcidos) se dibujan PRIMERO").

### 3.3 Creación en el PDF (escritura)

- KOReader: `saveHighlight` (`pdfdocument.lua:239`) → `page:addMarkupAnnotation(
  quadpoints, n, PDF_ANNOT_HIGHLIGHT|UNDERLINE|STRIKE_OUT, color)`; borrar y
  editar notas por los mismos quads (`getMarkupAnnotation`). Al cerrar, si
  `is_edited`, `writeDocument` (`pdfdocument.lua:395`).
- MuPDF C: `pdf_create_annot(page, PDF_ANNOT_HIGHLIGHT)` +
  `pdf_set_annot_quad_points(n, qv)` + `pdf_set_annot_color` +
  `pdf_set_annot_contents` (`include/mupdf/pdf/annot.h:395,715,688,800`).
- crate `mupdf` (nuestro): `Page::create_annotation(PdfAnnotationType::Highlight)`
  (`src/pdf/page.rs:212`), `PdfAnnotation::set_quad_points` (`src/pdf/annotation.rs:596`),
  `set_color`, `set_opacity`, `set_contents`.

---

## 4. Tinta / dibujo

- **Formato**: polilíneas en coordenadas de página, con grosor y color.
  - MuPDF: `pdf_set_annot_ink_list(annot, n, count[], v[])` — varios trazos
    empaquetados, cada uno con su recuento de puntos (`include/mupdf/pdf/annot.h:734`);
    crate `mupdf`: `PdfAnnotation::set_ink_list<I, S>(strokes)` (`src/pdf/annotation.rs:1028`).
  - Okular: `InkAnnotation::inkPaths()` = `QList<QList<NormalizedPoint>>`
    (`core/annotations.cpp:2571`) — **varios trazos por anotación**, puntos
    normalizados 0..1.
  - pdf.js: `InkEditor` (`src/display/editor/ink.js`) → `serialize()` guarda
    `paths: { points: inkLists }` (línea 289-301); el dibujo se suaviza con el
    outliner (`src/display/editor/drawers/inkdraw.js`, "smoothing/thinning").
  - PDFLector: `Stroke { points: Vec<(f32,f32)>, width, color }` en coords de
    página (`annotations.rs:90`) — **formato ya compatible con PDF Ink**.
- **Extras que tienen otros y nosotros no**:
  - Suavizado/decimación de puntos (pdf.js `freedraw.js`/`inkdraw.js`; KOReader no
    tiene dibujo en el reader principal).
  - Presión del lápiz → grosor variable (AndroidPdfViewer no dibuja; en el mundo
    Android lo hacen apps de tinta; MuPDF no lo modela — el ancho es uniforme).
  - Múltiples trazos por anotación agrupados (Okular lo permite; PDFLector
    guarda un `Stroke` por anotación — se pueden agrupar en la exportación con
    un solo `pdf_set_annot_ink_list`).

---

## 5. Notas (texto asociado)

- PDF nativo: la nota es el campo `Contents` de la anotación (popup). KOReader
  adjunta la nota al highlight (`item.note`) y la escribe con
  `updateHighlightContents` (`pdfdocument.lua:279`); Okular `TextAnnotation` es
  una anotación independiente con `contents()`; pdf.js `HighlightEditor` +
  `AnnotationEditor.addComment` (comentario ligado al highlight, `highlight.js`).
- El crate `mupdf` expone `PdfAnnotation::set_contents(&str)` /
  `contents() -> Option<&str>` (`src/pdf/annotation.rs:723,733`).
- PDFLector: el enum `Annotation` (`annotations.rs:137`) tiene `Highlight` y
  `Stroke`, **sin tipo nota**; KOReader demuestra que lo natural es **nota
  adjunta al highlight** (un campo `note: Option<String>`), no una anotación
  separada: coincide con cómo se exporta al PDF (Contents de la misma anotación).

---

## 6. Almacenamiento y persistencia

Dos paradigmas, y los lectores los mezclan:

### 6.1 PDF nativo (anotaciones dentro del fichero PDF)

- **Okular**: todo dentro del PDF vía Poppler (`generators/poppler/annots.cpp:700`
  `ppl_page->addAnnotation(ppl_ann)`); al guardar escribe el PDF completo a un
  **fichero temporal y lo renombra** (`PDFGenerator::save`,
  `generators/poppler/generator_pdf.cpp:2097` — *"poppler doesn't like
  overwriting in-place"*). Okular además persiste bookmarks/settings en un XML
  aparte, pero las anotaciones viven en el PDF.
- **KOReader**: opción `highlight_write_into_pdf` (por defecto activa): escribe
  anotaciones PDF reales **y además** las mantiene en el sidecar. Al reabrir,
  `importEmbeddedAnnotations` (`readerhighlight.lua:2766`) las lee del PDF
  (`getEmbeddedAnnotations`, `pdfdocument.lua:350`). El PDF es el formato de
  intercambio; el sidecar es la copia de trabajo.
- **pdf.js**: `AnnotationStorage` en memoria; al "download" se escribe en el PDF.
- **MuPDF**: `fz_save_document`; **escritura incremental** disponible
  (`pdf_can_be_saved_incrementally`); crate `mupdf`:
  `PdfSaveOptions::set_incremental(true)` (`src/pdf/document.rs:66`) + `Document::save`
  (`src/pdf/document.rs:846`).

### 6.2 Sidecar (fichero adjunto al PDF)

- **KOReader**: sidecar en `docs/<pdf>.sdr/settings.reader.lua`
  (`docsettings.lua:113` `DocSettings.getSidecarDir` — carpeta `<nombre>.sdr`
  junto al PDF). Las anotaciones viven en la clave `annotations` de ese Lua
  (`readerannotation.lua:113` `onReadSettings` — `config:readSetting("annotations")`),
  con migración de formato y backup de incompatibles.
- **PDFLector**: sidecar SQLite por PDF (`store.rs`, `annotations/<stem>.db`,
  sin WAL, un solo fichero, pensado para Syncthing). Es la versión robusta del
  patrón KOReader (SQLite+serde en vez de Lua); el formato es decisión de Fase 3-4.

### 6.3 Decisión pendiente (PLAN.md Fase 3-4)

Los tres lectores convergen en: **el PDF nativo es el formato de intercambio** y
el sidecar es el estado de trabajo. Para PDFLector hay dos caminos coherentes:

1. **Sidecar como fuente de verdad + exportación a PDF** (lo que ya hay:
   `store.rs` + `export_pdf_annotated`). Cero riesgo de corromper el PDF,
   Syncthing-friendly, pero un lector externo no ve las anotaciones hasta exportar.
2. **Dual-write incremental** (estilo KOReader): al cerrar/guardar, escribir las
   anotaciones en el PDF con save **incremental** (solo el delta → diffs pequeños
   para Syncthing). Interoperable con Okular/KOReader/evince. Requiere que el PDF
   sea escribible y decidir qué pasa con los conflictos de sync en el PDF mismo.

Cualquiera de los dos: si algún día se escribe el PDF, seguir el patrón Okular
(temp + rename) y el orden de esquinas de `_quadpointsFromPboxes`.

---

## 7. Render de la capa de anotaciones sobre el bitmap

| Enfoque | Quién | Coste |
|---------|-------|-------|
| **Overlay vectorial propio** sobre el bitmap cacheado | PDFLector (AGENTS.md §4.3), Okular (pinta `HighlightAnnotation`/`InkAnnotation` como vector sobre la página) | Sin invalidar caché al editar; hay que tesselar nosotros; render ∝ nº de anotaciones visibles. |
| **Overlay raster propio** sobre el tile cache | KOReader (`readerview.lua` `drawSavedHighlight` — pinta rects con blend MUL sobre el blitbuffer cacheado) | Rápido, pero hay que re-pintar el overlay al cambiar zoom/scroll; el cache de tiles no se invalida. |
| **Rasterizado en el bitmap por el motor** | AndroidPdfViewer (`PdfFile.renderPageBitmap(..., annotationRendering)` → PDFium `FPDF_RENDER_ANNOT`); MuPDF con `fz_run_page` (incluye `fz_run_page_annots`, `source/fitz/document.c:1117`) | Cero código propio, pero **editar una anotación invalida el render de la página** (re-render). El crate expone `Page::to_display_list(annotations: bool)` y `run_annotations`. |

**Conclusión**: la decisión de AGENTS.md §4.3 (capa vectorial propia, nunca en el
bitmap de página) coincide con Okular y evita el re-render; KOReader demuestra
que un overlay **raster** sobre la caché también sirve y es más barato por frame
si los trazos son muchos (test de estrés de Fase 3: 200 trazos). El dato nuevo:
MuPDF puede renderizar las anotaciones por separado (`run_annotations`) — útil
para un modo "previsualizar cómo quedará exportado" sin tocar la caché.

---

## 8. Aplicable a PDFLector (priorizado)

Referencias a ficheros actuales: `crates/pdf_core/src/engine.rs` (`TextSpan`),
`crates/pdf_core/src/engine/mupdf.rs` (`text()`), `crates/pdf_core/src/annotations.rs`,
`crates/pdf_android/src/reader.rs` (`sel_text`, `highlight_sel`), `crates/pdf_android/src/draw.rs`.

### P1 — Subrayado por líneas en vez de caja única
**Problema**: `highlight_sel` guarda el rect completo de la selección
(`reader.rs:1584`) — pinta márgenes y huecos. `Highlight.rects` ya es `Vec<Rect>`.
**Técnica**: usar `TextPage::highlight_selection(a, b, quads)` del crate mupdf
(1 llamada: orden de lectura + fusión por línea) y convertir cada `Quad` en
`Rect`; o implementar `getTextFromBoxes` de KOReader sobre `StructuredTextLine`
para control fino (primera línea parcial + líneas completas + última parcial).
**Medición**: visual + test unitario con corpus (nº rects por selección
multilínea; cobertura del texto). *(cita: `mupdf-0.8.0/src/text_page.rs:476`,
`koptinterface.lua:1042`, `okular/core/textpage.cpp:277`)*

### P1 — Selección por palabra/índice de lectura, no por intersección de bbox
**Problema**: `sel_text` (reader.rs:1439) filtra spans por intersección de rect y
ordena por (y, x) — en páginas a dos columnas el orden (y,x) rompe la lectura
(una línea de la columna izquierda se mezcla con la de la derecha si sus y
coinciden); un span tocado 1 px se incluye entero.
**Técnica**: anclar los dos puntos a la palabra más cercana (estilo
`getWordBoxIndices` KOReader) y recorrer en el orden de los spans de MuPDF
(que ya es orden de lectura del análisis stext, `fz_enumerate_selection`);
recortar la primera/última línea por la posición del ancla. Para texto vertical
(CJK) tener en cuenta `wmode`/`dir` de la línea.
**Medición**: test unitario con página de 2 columnas; comparar salida con
"copiar en Okular" del mismo PDF. *(cita: `koptinterface.lua:996,1042`,
`stext-search.c` `fz_enumerate_selection`)*

### P1 — Calidad del texto extraído: guiones y espacios
**Problema**: hoy se concatenan `line.text` sin limpiar guiones finales de línea.
**Técnica**: regla KOReader — al unir líneas, si la anterior termina en `-`
(o guion blando U+00AD) se elimina; si no, se añade un espacio; heurística CJK
(sin espacio entre caracteres CJK con hueco < 80 % de altura). Alternativa: stext
de MuPDF con flag `FZ_STEXT_DEHYPHENATE` (el crate lo expone vía
`TextPageFlags`).
**Medición**: test unitario con corpus bilingüe; diff con `pdftotext -layout`.
*(cita: `koptinterface.lua:1070-1100`)*

### P1 — Caché del stext por página
**Problema**: `mupdf.rs:text()` re-ejecuta `to_text_page` en cada llamada;
`sel_text` y el subrayado la llaman por gesto.
**Técnica**: KOReader cachea las textboxes por página en el `DocCache`
(`getPageTextBoxes`). En PDFLector, caché LRU del stext (por bytes, como la de
bitmaps) o reutilizar el `PageText` ya devuelto por `text(page)` en el gesto.
**Medición**: `cargo bench` — `text(page)` repetido sin cambio de página.
*(cita: `pdfdocument.lua:239`)*

### P2 — Persistencia: decidir y documentar el dual-write (PENDIENTE §6)
**Técnica**: si se escribe en el PDF: save incremental del crate
(`PdfSaveOptions::incremental`) + patrón temp+rename de Okular + orden de
esquinas ll/lr/ul/ur de KOReader; interoperable con KOReader/Okular.
**Es decisión abierta** (PLAN.md, Fase 3-4): preguntar al autor antes de codificar.
*(cita: `mupdf-0.8.0/src/pdf/document.rs:66`, `generator_pdf.cpp:2097`,
`pdfdocument.lua:75`)*

### P2 — Notas adjuntas al highlight
**Técnica**: añadir `note: Option<String>` a `Highlight` (modelo KOReader) y
mapear a `PdfAnnotation::set_contents` en la exportación. El enum `Annotation`
actual no tiene nota.
**Medición**: round-trip serde + exportación a PDF (abrir en Okular y ver el
popup). *(cita: `pdfdocument.lua:279`, `mupdf-0.8.0/src/pdf/annotation.rs:723`)*

### P2 — Tinta: agrupar trazos y suavizar
**Técnica**: `Stroke` ya es PDF-Ink-compatible; al exportar, agrupar N trazos en
un solo `set_ink_list` (como `inkPaths` de Okular). Suavizado opcional
(Catmull-Rom / decimación) antes de guardar para reducir puntos — mismo trade-off
que pdf.js (`inkdraw.js`).
**Medición**: nº de puntos por trazo antes/después; tamaño del sidecar.
*(cita: `mupdf-0.8.0/src/pdf/annotation.rs:1028`, `okular/core/annotations.cpp:2571`)*

### P3 — Modo "vista previa de exportación" con render de anotaciones de MuPDF
**Técnica**: `Page::run_annotations(device, ctm)` / `to_display_list(annotations=true)`
para mostrar cómo quedaría el PDF exportado sin tocar la caché de páginas.
*(cita: `mupdf-0.8.0/src/page.rs:231,132`)*

### P3 — Soporte de texto rotado/vertical
`Quad` de MuPDF puede no ser axis-aligned (línea rotada): nuestro `Rect`
(`x,y,w,h`) no lo representa. Si el corpus incluye PDFs con texto rotado, subir a
`Quad` propio en `annotations.rs` o aceptar la pérdida (highlight de caja).
*(cita: `mupdf/fitz/structured-text.h:467` — `wmode`/`dir`)*

---

## 9. Fuentes

Rutas locales (clonados/descargados en esta sesión en `/tmp/pdfresearch/`):

- `koreader/frontend/document/koptinterface.lua` — selección, `getTextFromBoxes`
- `koreader/frontend/document/pdfdocument.lua` — `saveHighlight`, quadpoints, escritura PDF
- `koreader/frontend/apps/reader/modules/readerhighlight.lua` — flujo de subrayado
- `koreader/frontend/apps/reader/modules/readerannotation.lua` — sidecar `annotations`
- `koreader/frontend/docsettings.lua` — sidecar `.sdr`
- `mupdf/source/fitz/stext-search.c` — `fz_highlight_selection`, `fz_snap_selection`
- `mupdf/source/fitz/document.c:1117` — `fz_run_page_annots`
- `mupdf/include/mupdf/pdf/annot.h` — API de anotaciones (quad points, ink list, contents)
- `mupdf/source/pdf/pdf-run.c:27` — render de anotaciones (`pdf_process_annot`)
- `~/.cargo/registry/src/index.crates.io-*/mupdf-0.8.0/src/text_page.rs:476` — `highlight_selection`
- `~/.cargo/registry/src/index.crates.io-*/mupdf-0.8.0/src/pdf/annotation.rs` — `set_ink_list`, `set_quad_points`, `set_contents`
- `~/.cargo/registry/src/index.crates.io-*/mupdf-0.8.0/src/pdf/document.rs:66,846` — save incremental
- `okular/core/textpage.cpp:277` — `TextPage::textArea`
- `okular/core/annotations.cpp` — `HighlightAnnotation`, `InkAnnotation`, `TextAnnotation`
- `okular/generators/poppler/generator_pdf.cpp:2097` — `PDFGenerator::save` (temp+rename)
- `pdf.js/src/display/text_layer.js` — capa de texto DOM
- `pdf.js/src/display/editor/highlight.js`, `drawers/highlight.js` — selección → quads
- `pdf.js/src/display/editor/ink.js` — editor de tinta
- `pdf.js/src/core/annotation.js` — parsing de QuadPoints
- `AndroidPdfViewer/.../PdfFile.java:293` — `renderPageBitmap(..., annotationRendering)`
- `pdfium-render/src/pdf/document/page/annotation/*.rs` — API Rust (histórico, Fase 0)

URLs: https://github.com/koreader/koreader · https://github.com/ArtifexSoftware/mupdf ·
https://invent.kde.org/graphics/okular · https://github.com/mozilla/pdf.js ·
https://github.com/ajrcarey/pdfium-render · https://github.com/barteksc/AndroidPdfViewer ·
crate `mupdf` 0.8: https://crates.io/crates/mupdf

> Nota de licencias (AGENTS.md §3): KOReader y MuPDF son AGPL-3.0 — leer código
> como referencia está cubierto por la decisión ADR-001 (nuestro repo ya es
> AGPL-3.0); no se copia código, solo técnicas. Okular es GPL-2.0+ (copyleft):
> solo lectura de patrones, nada que derive en nuestro código.
