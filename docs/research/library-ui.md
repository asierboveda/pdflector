# Investigación — Bibliotecas / estanterías de documentos en lectores

> Investigación de referencia para PDFLector. Objetivo: estudiar cómo construyen
> los lectores la **biblioteca/estantería** (rejilla de portadas, thumbnails,
> recientes, escaneo de archivos, búsqueda) en móvil/Android y escritorio, para
> mejorar la rejilla 3×3 ya existente en `pdf_android` (ver §6).
>
> Fecha: 2026-08-19. Modo: solo lectura (sin tocar código del repo).
> Repos clonados (shallow) en `/tmp/research/` durante esta sesión; fechas y
> líneas citadas corresponden a `main`/`master` de esa fecha.

## 0. Resumen ejecutivo (qué copiar tal cual)

Las 7 técnicas con mejor ratio valor/esfuerzo para PDFLector:

1. **Progreso de lectura sobre la portada** — barra de progreso + marca de
   esquina (leyendo / terminado / en pausa). KOReader (`mosaicmenu.lua`) y
   Foliate (`library.js`) lo dibujan sobre la propia celda; el dato ya existe
   en nuestra arquitectura (sidecar + `library.db`).
2. **Thumbnails persistidos a disco**, no solo en RAM: KOReader guarda la
   portada como **BLOB zstd del bitmap** en SQLite (`bookinfomanager.lua`);
   Calibre usa una **ThumbnailCache en disco (PPM)** con invalidación por
   mtime (`caches.py`); Foliate guarda `<identificador>.png` en caché y no
   regenera si existe. Nuestro `ThumbCache` (RAM, 9 MiB) regenera todo en cada
   arranque.
3. **Limitar tamaño de portada a ~512 px y re-encodear JPEG q85 solo cuando
   se escala** (Readest, `parser_common.rs`), con **re-extracción si la rejilla
   pide más resolución** (KOReader `isCachedCoverInvalid`).
4. **Lazy + prioridad de scroll**: renderizar solo celdas visibles (ya lo
   hacemos, 3/tick) y servir las peticiones más recientes primero (Calibre usa
   `LifoQueue` en el hilo de thumbnails).
5. **Fallback de portada = texto sobre color derivado del título** (Calibre
   `generate_cover`, Thorium `RandomCustomCovers`, Foliate): elimina el
   placeholder "…" sin coste de I/O.
6. **Estado por libro + "continuar leyendo"**: status `new/reading/abandoned/
   complete` calculado desde el sidecar (KOReader `BookList.getBookStatus`) y
   reapertura de los últimos N libros no terminados al arrancar (Readest
   `handleOpenLastBooks`).
7. **Resiliencia del escaneo**: marca `in_progress`/`unsupported` con tope de
   reintentos (3) para no re-procesar libros corruptos en cada arranque
   (KOReader `bookinfomanager.lua`), y merge-floor al persistir la biblioteca
   para no perder entradas con escrituras concurrentes (Readest
   `libraryService.ts` — muy relevante para Syncthing).

## 1. Método y búsquedas

- `gh search repos "pdf library grid"`, `"book library ui android"`,
  `"ebook reader"`: la API de búsqueda de GitHub (10 req/min autenticada)
  rate-limitó las primeras consultas; los repos de referencia se eligieron por
  criterio conocido (KOReader, Readest, Calibre, Foliate, Thorium, Librum,
  Librera) y se clonaron shallow.
- `gh search code "thumbnail" --language Rust --limit 10`: devuelve crates de
  generación genérica de thumbnails (`thumbo-core`, `lazy-image`,
  `imager-wasm`…), **ninguno** de biblioteca de ebooks en Rust. Conclusión:
  la capa de biblioteca vive en TS/Kotlin/Lua/Python/C++; el único Rust nativo
  relevante encontrado es el parser de portadas de Readest (Tauri).
- `gh search code "takePersistableUriPermission" --language Kotlin`: confirma
  el patrón SAF (permiso persistente de URI) en apps Android de media
  (Auxio, mpv-android, Seal…); ver §4.3.

## 2. Repos estudiados

| Repo | Stack | Licencia | Qué aporta a este estudio |
|------|-------|----------|---------------------------|
| [koreader/koreader](https://github.com/koreader/koreader) | Lua (e-reader) | AGPL-3.0 | El mosaico 3×3 más pulido: portadas, estados, caché SQLite, extracción en subproceso |
| [readest/readest](https://github.com/readest/readest) | TS + Tauri/Rust | AGPL-3.0 | Grid responsive, thumbnails 512px en Rust, búsqueda en worker, merge-floor, "open last books" |
| [kovidgoyal/calibre](https://github.com/kovidgoyal/calibre) | Python/Qt | GPL-3.0 | Referencia de portadas: generadas, caché disco+RAM, hilo de thumbnails con LifoQueue |
| [johnfactotum/foliate](https://github.com/johnfactotum/foliate) | JS/GTK | GPL-3.0 | Caché de portada por identificador, lazy loading incremental, progreso en celda |
| [edrlab/thorium-reader](https://github.com/edrlab/thorium-reader) | Electron/TS | BSD-3-Clause | Fallback de portada con degradado + título; covers "thumbnail" vs "cover" |
| [Librum-Reader/Librum](https://github.com/Librum-Reader/Librum) | C++/Qt | AGPL-3.0 | Portada redimensionada a tamaño fijo al importar (188×238) |
| [foobnix/LibreraReader](https://github.com/foobnix/LibreraReader) | Kotlin KMP (rewrite) | GPL-3.0 | `coverPDF()`: thumbnail 200×250 PNG del render de página 1 |

> Nota de licencias: KOReader y Readest son AGPL; solo se estudian patrones
> (no se copia código), lo que es compatible con nuestra elección MuPDF/AGPL
> (ADR-001). Calibre/Foliate/Librera son GPL: mismo criterio, solo patrones.

## 3. Técnicas — rejilla de portadas (grid)

### 3.1 Mosaico 3×3 de KOReader (el referente móvil/e-ink)

`plugins/coverbrowser.koplugin/` — el plugin "CoverBrowser" sustituye el
filemanager clásico por una rejilla:

- **Modos de vista configurables**: clásico (solo nombre), `mosaic_image`
  (rejilla con portadas), `mosaic_text` (rejilla con "text covers"),
  listas con/sin imagen. Por defecto: filemanager en lista con imagen,
  **historial y colecciones en mosaico 3×3** (`main.lua: init()`:
  `filemanager_display_mode = list_image_meta`, `history_display_mode =
  mosaic_image`).
- **Celdas uniformes**, columnas/filas configurables (por defecto
  **3×3 en vertical**, `mosaicmenu.lua` `_recalculateDimen()`:
  `nb_cols_portrait = 3`, `nb_rows_portrait = 3`, rango 2..8). Tamaño de
  celda: `(pantalla − márgenes)/(cols|rows)` con `item_margin = scaleBySize(10)`.
- **La portada no llena la celda**: la imagen se escala a ajustarse dentro de
  `celda − borde`, centrada (`CenterContainer`) y con **borde fino**
  (`border_size = Size.border.thin`) "porque algunas portadas son blancas y
  sin borde se pierden" (`MosaicMenuItem:update()`). Los **directorios** se
  dibujan distintos: más borde, esquinas redondeadas, nombre centrado y nº de
  items abajo.
- **"Text cover" cuando no hay imagen** (`FakeCover`): título + autores
  centrados, tamaño de fuente autocalculado (bucle que baja el tamaño hasta
  caber), ancho reducido a 7/8 de la celda "para que parezca más un libro que
  un cuadrado". Normaliza el nombre de fichero como título provisional
  (`title = filename` y limpia `_ - |`).
- **Indicadores sobre la portada**: marcas de esquina (dog-ear) de estado
  **leyendo / abandonado / completo / colección** y **barra de progreso**
  (`progress_widget`, ancho 60 % de la celda) cuando `show_progress_in_mosaic`
  está activo (`mosaicmenu.lua`).
- **Carga perezosa visible-celda-a-celda**: los items aún no indexados se
  dibujan con placeholder "…"; al final del `updateItems()` se lanza la
  extracción de portadas en background al `nextTick` y al terminar se refresca
  (`covermenu.lua` `updateItems()`). Garbage-collection programada cada N
  dibujos (memoria estable 15–25 MB navegando, según comentario del propio
  código).

### 3.2 Readest — grid responsive y dos modos de encaje

`apps/readest-app/` (Next.js + Tauri):

- `utils/grid.ts` `getGridTemplate(count, aspectRatio)`: layout adaptativo —
  1 libro = 1 celda, 2 libros = 2×1 (según ratio), 3-4 = 2×2, ≥5 = **3×3**.
- `components/BookCover.tsx`: dos políticas de encaje configurables:
  `coverFit='crop'` (`object-cover`, celdas uniformes, se recorta) o
  `'fit'` (preserva ratio, con `shadow-md`). Fallback si la imagen falla:
  **portada de texto** (título + autor, serif, `line-clamp-3`) sobre fondo
  plano. Memoización por `coverImageUrl` para no re-renderizar al hacer scroll.
- `components/CachedImage.tsx`: **Mapa en memoria URL→url cacheada** +
  **dedupe de promesas en vuelo** (un solo fetch por portada aunque varias
  celdas la pidan a la vez durante el scroll).

### 3.3 Foliate — lazy loading incremental

`src/library.js`: el `BookList` extiende `Gio.ListStore` y expone
`loadMore(n)` (scroll infinito por lotes), ordenado por `modified` desc.
La celda (`BookItem`) muestra portada + **título + porcentaje**; la fila
(`BookRow`) añade barra de progreso y un indicador "book size" escalonado
(10 pasos). Placeholder global: pixbuf gris 256×384 con título encima.

## 4. Técnicas — thumbnails (generación, caché, invalidación)

### 4.1 Render de página 1 vs portada embebida

- **PDF**: se renderiza la **página 1** a resolución de pantalla. KOReader
  `BookInfo:getCoverImage()` (`filemanagerbookinfo.lua:461`) delega en
  `document:getCoverPageImage()` (MuPDF/KOReader renderiza la primera página
  para PDF; en EPUB usa la portada embebida). Librera KMP hace lo mismo:
  `coverPDF()` (`shared/src/jvmMain/.../pdf.kt`) renderiza la página 1 a
  **200×250 (4:5)** y la codifica como PNG.
- **EPUB/MOBI**: se extrae la imagen de portada del contenedor (Readest
  `parser_common.rs`, Foliate `saveCover` en `data.js`).

### 4.2 Tamaño, formato y filtro de reescalado

- **Readest** (`apps/readest-app/src-tauri/src/parser_common.rs`): constante
  `COVER_MAX_LONG_EDGE = 512` ("sized for the library grid ~250-300px @2x").
  Solo re-encodea si el lado largo supera 512: **JPEG q85**, filtro
  **Triangle** en vez de Lanczos3 porque "a escala de 512 px la diferencia es
  imperceptible y Triangle es 5-8× más rápido" (debug). Si no escala, guarda
  los bytes originales tal cual.
- **KOReader** (`bookinfomanager.lua`): el cover se escala al tamaño máximo
  de la celda actual (`cover_specs.max_cover_w/h`) y se guarda **redimensionado
  ya** en la DB; `getCachedCoverSize()` = ajuste proporcional a la caja.
- **Foliate** (`data.js` `saveCover`): ancho objetivo configurable
  (`cover-size`, default 256), `scale_simple` bilinear preservando ratio, PNG.
  **No regenera si el fichero ya existe** (`query_exists`).
- **Librum** (`book.hpp`): `maxCoverWidth 188 / maxCoverHeight 238`
  (aspecto ≈ 0,79, similar a un libro/A4) al importar.
- **Calibre** (`ebooks/covers.py`): portadas generadas por defecto a
  **1200×1600 px** (`cover_width/height`), con estilos (Blocks…) + temas de
  color aleatorios y bloques de texto (título/subtítulo/pie) — ver §5.4.

### 4.3 Caché (RAM y disco) e invalidación

- **Calibre** (`gui2/library/caches.py`) — el pipeline más completo:
  - `ThumbnailCache` en **disco** (formato **PPM** = encode/decode barato y
    lossless; `min_disk_cache=100`, `max_size=1024`, `thumbnail_size=(100,100)`
    por defecto), agrupada por `library_id`.
  - `RAMCache`: **LRU de QPixmaps, límite 100**, thread-safe con *staging*
    (los pixmaps desalojados desde hilos no-GUI se liberan en el hilo GUI).
  - `ThumbnailRenderer`: hilo worker con **`LifoQueue`** — la petición más
    reciente se sirve primero, que es la que quiere el scroll. Señal
    `rendered(book_id, QPixmap)` al hilo GUI; descarta resultados si el tamaño
    de thumbnail cambió o el library_id ya no es el actual.
  - `fetch_cover_from_cache()`: disco → si miss, lee el cover de la DB del
    libro, hace thumbnail (`resize_to_fit`) e inserta en disco. La clave
    incluye el **timestamp (mtime) del cover** para invalidar.
- **KOReader**: el BLOB de portada (bitmap crudo **comprimido con zstd**,
    con `w/h/type/stride` en columnas) vive en la tabla SQLite `bookinfo`
    (`bookinfomanager.lua`, `BOOKINFO_COLS_SET`). Invalidación doble:
    - `isCachedCoverInvalid(bookinfo, cover_specs)`: si la celda actual pide
      más resolución que el thumbnail guardado → se **re-extrae con el nuevo
      tamaño** (evita portadas borrosas al cambiar de modo/vista).
    - `removeNonExistantEntries()`: prune de entradas cuyo fichero ya no
      existe (`lfs.attributes ~= file`).
- **Foliate**: fichero `<cache>/<encodeURIComponent(identifier)>.png`,
  `readCover = utils.memoize(...)` (carga una vez por sesión).
- **Readest**: `Books/<hash>/cover.<ext>` junto a los metadatos del libro;
  el hash es un **`partialMD5`** del fichero (solo 11 muestras de 1 KiB) que
  sirve de identidad estable y dedup.

### 4.4 Extracción en segundo plano y resiliencia (KOReader)

`extractInBackground()` lanza **subprocesos** (fork) con:
- tope de reintentos `max_extract_tries = 3`; antes de procesar inserta la
  fila con `in_progress = N` (flag anti-reintento de libros que crashean) y
  marca `unsupported` + `cover_fetched='Y'` para no volver a intentar;
- gestión de CPU: `Device:enableCPUCores(2)` mientras extrae, vuelta a 1 al
  terminar (e-ink, ahorro de batería);
- timeout global que mata subprocesos colgados y cancelación al cambiar de
  página/directorio;
- escaneo `findFilesInDir()` **iterativo** (BFS con pila explícita, no
  recursión), salta carpetas ocultas (salvo `show_hidden`), ignora forks de
  macOS (`._*`) y filtra por extensión vía `DocumentRegistry:hasProvider()`.

## 5. Técnicas — escaneo, recientes, búsqueda

### 5.1 Escaneo de documentos en Android: MediaStore vs SAF

- **PDFLector ya resuelve el caso Android** (memoria, 2026-08-13): query
  `MediaStore.Files` con `mime_type='application/pdf'`, proyección
  `[_ID, DISPLAY_NAME, RELATIVE_PATH, _SIZE]`, orden por ruta/nombre, y
  apertura vía `ContentResolver.openFileDescriptor` (content:// → fd → MuPDF).
  Permiso: Android 13+ requiere **`MANAGE_EXTERNAL_STORAGE`** (appop "All
  files access", sin `READ_MEDIA_*` para documentos); Android ≤12
  `READ_EXTERNAL_STORAGE`. Hallazgo: ficheros `adb push` quedan `is_pending=1`
  e invisibles hasta `content call scan_volume`.
- **KOReader no usa SAF**: su APK es un wrapper nativo que accede al
  filesystem directamente (`/storage/emulated/...`); es la vía "F-Droid".
- **Patrón SAF persistente** (verificado con `gh search code
  takePersistableUriPermission` en apps Android de media: Auxio, mpv-android,
  Seal…): `ACTION_OPEN_DOCUMENT_TREE` + `takePersistableUriPermission()` para
  guardar el acceso a una carpeta elegida por el usuario; útil como alternativa
  "carpeta propia" sin permiso global (relevante si más adelante no queremos
  `MANAGE_EXTERNAL_STORAGE`).
- **Watch / rescaneo**: PDFLector ya tiene `notify` + debounce 150 ms para
  anotaciones (Fase 4). KOReader relee el historial si el mtime del fichero de
  historial cambió (`readhistory.lua` `_read()`), pensado para sync.

### 5.2 Recientes y posición de lectura persistida

- **KOReader** (`readhistory.lua`): historial como lista `{time, file}` en
  fichero Lua, cap `history_size` (default **500**), actualizado por mtime;
  reapertura del último libro al arrancar (`ensureLastFile()` +
  `G_reader_settings "lastfile"`). El estado de lectura vive en el **sidecar
  por libro** (`settings.reader.lua` → `summary.status`: reading/abandoned/
  complete + `percent_finished`), y `BookList.getBookStatus(file)` lo resume
  para la rejilla (estado "new" si no hay sidecar). Historial con items
  "deleted" marcados (no se borran, se atenúan).
- **Readest** (`app/library/page.tsx` `handleOpenLastBooks`): al arrancar
  reabre los **últimos N libros no terminados** (`readingStatus !== 'finished'`)
  solo si el fichero sigue disponible (`isBookAvailable`). Además filtra los
  borrados con **tombstones** (`deletedAt`) en lugar de eliminarlos del índice.
- **Foliate**: el porcentaje viaja en los metadatos JSON por libro y se
  muestra en celda/fila.

### 5.3 Búsqueda / filtro

- **Readest**: la búsqueda de biblioteca corre en un **Web Worker**
  (`librarySearchService.ts` + `librarySearchWorker.ts`), mantiene un LRU de
  bases de índice abiertas (`MAX_OPEN_INDEX_DBS`) y versiona el índice
  (`SEARCH_INDEX_VERSION`) para reconstruirlo al cambiar la extracción.
  Búsqueda *in-book* con **SQLite FTS-like** (`search.db` por libro con texto
  plegado: caseless + sin diacríticos, `librarySearchIndex.ts`).
- **KOReader**: `filemanagerfilesearcher.lua` — búsqueda de ficheros por
  nombre con resaltado y colas de resultados; el historial se puede filtrar
  por estado (all/new/reading/abandoned/complete/deleted) y por texto.
- **Calibre**: búsqueda FTS sobre el catálogo (tabla `books` con FTS5 +
  búsqueda por campos) — sobre-dimensionado para nosotros.

### 5.4 Portadas generadas (fallback estético de bajo coste)

- **Calibre** (`ebooks/covers.py` `generate_cover`): pinta bloques de texto
  (título/subtítulo/pie) sobre un **tema de color elegido al azar** y un
  estilo geométrico (Blocks…), a 1200×1600.
- **Thorium** (`common/components/Cover.tsx` + `custom-cover.ts`): si no hay
  imagen → **degradado lineal** de una paleta de 3 colores fijos (o el
  `customCover` del libro) + título y autores centrados; capa `gradient` al
  pie de la portada real.
- **KOReader/Foliate**: el "text cover" (título centrado, fuente autoescalada,
  ancho 7/8 de la celda) — mismo espíritu sin color.

## 6. Aplicable a PDFLector (estado actual → mejoras, con prioridad)

Estado actual de nuestra biblioteca (`crates/pdf_android/`, ver
`docs/ux-rediseño-estructura.md` y `memory.md`): rejilla de **3 columnas**,
celda uniforme con área de portada A4 (`grid_cover_w/h`) + título 1 línea
14sp elipsis; portada = página 1 renderizada **perezosamente** (solo celdas
visibles, ≤3/tick, placeholder "…"); `ThumbCache` LRU en **RAM** (36 entradas
/ 9 MiB / 200 px); escaneo MediaStore al arranque con flujo de permiso
`MANAGE_EXTERNAL_STORAGE`; **sin** búsqueda, sin orden/filtros, sin recientes
en Android, sin progreso ni estado sobre la portada.

| # | Mejora (fuente) | Prioridad | Coste / riesgo |
|---|---|---|---|
| 1 | **Progreso + estado sobre la portada**: barra de % al pie de la celda y marca de esquina leyendo/terminado (KOReader `mosaicmenu.lua`; Foliate). El % ya está en `library.db` (Fase 4) | **P1** | Bajo. Solo draw + lectura de `library.db`; no toca el motor |
| 2 | **Persistir thumbnails en disco** (BLOB zstd en `library.db` al estilo KOReader, o `thumbs/<hash-uri>.png` + mtime al estilo Foliate) para no regenerar 200 portadas en cada arranque | **P1** | Medio. Añade columna/tabla en `library.db`; invalidación por mtime + `isCachedCoverInvalid` |
| 3 | **Bloquear libros corruptos**: flag `unsupported` con tope de reintentos (KOReader `max_extract_tries=3`), y caché de "ya intentado" para que el rescan no re-intente PDFs que no abren | **P1** | Muy bajo. Estado en memoria o en `library.db` |
| 4 | **Text-cover como placeholder** (título centrado, fuente autoescalada, color derivado de un hash del nombre) en vez de "…" (KOReader `FakeCover`, Thorium gradientes, Calibre) | **P2** | Bajo. Cero I/O; elimina el "parpadeo" de portadas cargando |
| 5 | **Re-extraer a mayor resolución si cambia la densidad/columnas** (`isCachedCoverInvalid`); mantener 200 px a 3 columnas, ~150 px a 4 | **P2** | Bajo. Ya tenemos `getCachedCoverSize` análogo en el pipeline de escala |
| 6 | **Búsqueda/filtro en memoria** por nombre de fichero (y después por metadatos de `library.db`), con worker si hace falta (Readest) | **P2–P3** | Medio. Solo filtro de la lista ya cargada; FTS5 solo si indexamos títulos |
| 7 | **Recientes en la biblioteca Android + "continuar leyendo"**: tabla `recents(uri, ts)` en `library.db`; al arrancar reabrir el último libro no terminado (Readest `handleOpenLastBooks`, KOReader `lastfile`) | **P3** | Medio. El egui ya tiene `recent_pdfs` (5) como referencia |
| 8 | **Dedup por hash parcial + merge-floor** al persistir (Readest `partialMD5`, `updateBooks`) para importaciones y escrituras concurrentes con Syncthing | **P3** | Medio. Alineado con Fase 4 |
| 9 | **Borde fino alrededor de la portada** (KOReader) — ya tenemos 1 px `LIB_ROW_BORDER`, pero para portadas claras el borde debe rodear la imagen, no solo la celda | **P3** | Trivial |
| 10 | **No copiar**: ya abrimos por fd sin copiar el PDF (mejor que KOReader/Readest, que copian al importar); mantener | — | — |

Prioridad sugerida al planificar Fase 4 (biblioteca): **1 → 2 → 3** primero
(progreso visible + portadas que no se regeneran + escaneo resiliente); 4-5
junto al restyling; 6-8 cuando haya `library.db` madura.

## 7. Referencias (rutas citadas)

- KOReader: `plugins/coverbrowser.koplugin/main.lua`, `mosaicmenu.lua`,
  `covermenu.lua`, `bookinfomanager.lua`; `frontend/readhistory.lua`;
  `frontend/apps/filemanager/filemanagerbookinfo.lua`;
  `frontend/ui/widget/booklist.lua`.
- Readest: `apps/readest-app/src-tauri/src/parser_common.rs`;
  `apps/readest-app/src/utils/grid.ts`;
  `apps/readest-app/src/components/BookCover.tsx`, `CachedImage.tsx`;
  `apps/readest-app/src/store/libraryStore.ts`;
  `apps/readest-app/src/services/libraryService.ts`, `librarySearchIndex.ts`,
  `librarySearchWorker.ts`; `apps/readest-app/src/app/library/page.tsx`.
- Calibre: `src/calibre/gui2/library/caches.py`; `src/calibre/ebooks/covers.py`;
  `src/calibre/db/cache.py` (`has_book`, `add_cover_cache`).
- Foliate: `src/library.js`; `src/data.js`.
- Thorium: `src/renderer/common/components/Cover.tsx`;
  `src/common/models/custom-cover.ts`.
- Librum: `src/domain/entities/book.hpp`; `src/application/services/library_service.cpp`.
- Librera: `shared/src/jvmMain/kotlin/mobi/librera5/pdf.kt`.
