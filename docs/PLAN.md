# PLAN DE IMPLEMENTACIÓN — PDFLector

> Generado el 2026-08-05 a partir de [[PROYECTO]] y la sesión de preguntas con el autor.
> Ritmo asumido: **20+ h/semana**. Duración total estimada: **~17 semanas**.
>
> ▶ **El proyecto empieza por §2 (PRIMER PASO)**: puesta a punto del entorno e
> instalación de skills. **No se escribe código hasta completarla.** El código
> empieza en la Fase 0 (§5).

## 1. Decisiones cerradas (sesión de aclaración)

| # | Decisión | Resultado |
|---|----------|-----------|
| 1 | Motor PDF | **MuPDF (ADR-001, Fase 0.5)** — elegido por benchmark (render 2,7-4× más rápido, RSS pico -21%); repo licenciado AGPL-3.0 |
| 2 | Presión del lápiz | **No necesaria.** Requisito real: anotaciones en márgenes (subrayado, dibujo tipo mapa mental) que se rendericen **nítidas y sin penalizar rendimiento** |
| 3 | Distribución | **Código público en GitHub.** AGPL no impide publicar en GitHub; si MuPDF gana el benchmark, el proyecto se licencia AGPL. Si PDFium gana → MIT/Apache-2.0 |
| 4 | Exportar notas | **Markdown** (citas con nº de página, ideal Obsidian) **+ PDF con anotaciones incrustadas** (estándar PDF, legible en cualquier lector) |
| 5 | Modo oscuro | **UI oscura + páginas del PDF invertidas** (página negra, texto claro) |
| 6 | Sincronización | **Automática tablet ↔ PC ↔ móvil, gratuita** → Syncthing sincronizando la carpeta de datos; diseño de almacenamiento anti-conflicto |
| 7 | Repositorio | `~/Projects/pdflector/`, git desde el día 1. **Una sola carpeta**: el vault de Obsidian vive en `docs/` dentro del repo (actualizado 2026-08-05) |
| 8 | Validación de fluidez | **Hardware real vía USB**: tablet conectada por USB-C con `adb` desde Fase 1 + métricas en escritorio |
| 9 | Testing | **Tests unitarios en `pdf_core` + benchmarks automatizados** (criterion). La UI se prueba a mano |

## 2. PRIMER PASO — puesta a punto del entorno e instalación de skills

> **Este es el primer paso del proyecto.** Estimación: 1 sesión (2-3 h) + descargas.
> Estado actual: `~/Projects/pdflector/` contiene `AGENTS.md` y `docs/` (vault de
> Obsidian con PROYECTO.md y PLAN.md). El código aún no existe; se escribe por
> primera vez en la Fase 0.

### 2.1 Instalar skills del agente (lo primero de todo)

**A) Skills existentes del ecosistema abierto** (estándar agentskills.io):

```bash
npx skills add anthropics/skills    # pdf (generar corpus de pruebas) + skill-creator
npx skills add obra/superpowers     # tdd, systematic-debugging, verification-before-completion
```

**B) Skills propios del proyecto** — se crean en `.opencode/skills/` (versionado)
al arrancar su fase (un skill documenta un procedimiento que ya existe; no antes):

| Skill | Se crea en |
|-------|------------|
| `pdflector-rendimiento` | Fase 0.5-1 |
| `android-tablet-adb` | Fase 0.5 |
| `benchmark-motores` | Fase 0.5 (temporal; archivable tras ADR-001) |
| `exportar-anotaciones` | Fase 3 |
| `syncthing-sync` | Fase 4 |

### 2.2 Herramientas del sistema

Verificado 2026-08-05: Rust 1.97.1 instalado vía pacman (**sin rustup**), **sin `adb`**, sin SDK/NDK de Android.
Estado 2026-08-05 (actualización §2): rustup + target Android instalados; `adb` y JDK 17 (pacman); SDK en `~/Android/Sdk` (cmdline-tools, NDK r28, platform 35, build-tools 35.0.0); corpus en `corpus/` dentro del repo (movido desde `~/Projects/pdflector-corpus/` el 2026-08-05: una sola carpeta por proyecto en `~/Projects/`).

- [x] Instalar `adb` → `sudo pacman -S android-tools`
- [x] Instalar **rustup** + toolchain estable + `rustup target add aarch64-linux-android` (el Rust de pacman no gestiona targets Android)
- [x] Android SDK (cmdline-tools) + NDK → necesarios en Fase 0.5/1; instalar ya para no bloquear
- [ ] Tablet Lenovo Idea Tab: activar opciones de desarrollador + depuración USB
- [x] Reunir **corpus de PDFs de prueba**: `corpus/` dentro del repo → `dense_textbook.pdf` (93 pág.), `scanned_pages.pdf` (30 pág., imágenes), `scientific_paper.pdf` (12 pág., gráficos vectoriales), `large_document.pdf` (500 pág.). Regenerable con `tools/generate_corpus.py` (uv + reportlab + pillow)
- [ ] (Fase 4) Syncthing en tablet, PC y móvil
- [ ] (Fase 5) Ollama instalado en el PC

### 2.3 Verificación de «entorno listo» → puerta de entrada a Fase 0

- [ ] `adb devices` muestra la tablet  ← pendiente: **tablet aún no comprada** (anotado 2026-08-05). No bloquea Fase 0 ni el benchmark de escritorio de Fase 0.5 (compilar para Android solo requiere SDK/NDK, ya instalados); solo bloquea las métricas en hardware real (harness de Fase 1 y validación de Fase 6)
- [x] `rustup target list --installed` incluye `aarch64-linux-android`
- [x] Skills del grupo A visibles para el agente (se cargan al iniciar una nueva sesión de opencode)
- [x] Corpus de PDFs en `corpus/` dentro del repo (añadir a `.gitignore` en Fase 0; regenerable con `tools/generate_corpus.py`)

## 3. Arquitectura

### 3.1 Estructura del workspace

```
~/Projects/pdflector/
├── AGENTS.md               # Reglas del agente (ya creado)
├── Cargo.toml              # workspace
├── .agents/skills/         # skills propios del proyecto (sección 2.1B)
├── corpus/                 # PDFs de prueba para benchmarks (gitignored)
├── tools/                  # Scripts auxiliares (generate_corpus.py, ...)
├── crates/
│   ├── pdf_core/           # Biblioteca: TODA la lógica. Sin UI, sin egui.
│   ├── pdf_app/            # Binario egui/eframe. SOLO prototipo de escritorio.
│   └── pdf_bench/          # Binario de benchmarks (escritorio y Android).
└── docs/                   # Vault de Obsidian dentro del repo (ya existe)
    ├── PROYECTO.md         # Visión del proyecto
    ├── PLAN.md             # Este documento
    └── adr/                # ADR-001 motor PDF, ADR-002 licencia, ...
```

### 3.2 Reglas de dependencia (innegociables)

```
pdf_app ──────▶ pdf_core ◀────── pdf_bench
  (UI)                            (benchmarks)
```

1. **`pdf_core` nunca depende de la UI.** Compila solo, sin entorno gráfico.
   Cambiar egui por Slint/Tauri en Fase 6 no reescribe ni una línea de lógica.
2. La UI nunca implementa lógica: pide a `pdf_core` y pinta.
3. El motor PDF va detrás de un `trait RenderEngine`, con backends
   intercambiables por feature flags:

```rust
trait RenderEngine {
    fn open(path: &Path) -> Result<Document>;
    fn page_count(&self) -> u32;
    fn render_page(&self, page: u32, scale: f32) -> Result<Bitmap>; // RGBA, resolución de pantalla
    fn text(&self, page: u32) -> Result<PageText>;                  // extracción perezosa
}
```

Backends durante Fase 0.5: `PdfiumEngine` y `MupdfEngine`. Tras ADR-001 queda
uno solo (**MuPDF**); el perdedor se eliminó sin deuda (Fase 0.5).

### 3.3 Módulos internos de `pdf_core`

| Módulo | Responsabilidad |
|--------|-----------------|
| `engine` | `trait RenderEngine` + backends tras feature flags |
| `cache` | LRU de páginas renderizadas, **limitado por bytes**; expulsa al hacer scroll |
| `render` | Cola de render en hilos de fondo (rayon), prioridad a páginas visibles |
| `annotations` | Modelo vectorial (`Stroke`, `Highlight`, `TextNote`) en **coordenadas de página** |
| `store` | SQLite (rusqlite), sidecar por PDF |
| `export` | Markdown + PDF con anotaciones incrustadas |
| `sync` | (Fase 4) Formato sync-friendly y detección de cambios (`notify`) |

### 3.4 Flujo de renderizado (el principio de fluidez)

```
Scroll → calcula páginas visibles → consulta `cache`
  ├─ HIT  → textura lista, se pinta (camino rápido, < 16 ms)
  └─ MISS → placeholder + encola render en `render` (rayon, NUNCA en hilo UI)
            → al terminar entra en caché y se repinta
Zoom        → escala el bitmap existente al instante + re-render nítido asíncrono
Anotaciones → capa vectorial SOBRE la textura: nítidas a cualquier zoom,
              coste proporcional a trazos visibles (no a páginas)
```

**Prohibido**: render a resolución máxima, mantener todas las páginas en memoria,
bloquear el hilo de UI. Este diseño responde directamente a la prioridad nº1
(fluidez) y a la preocupación del autor: los mapas mentales en márgenes se
renderizan nítidos **sin bajar la velocidad**.

### 3.5 Almacenamiento en disco (diseñado para Syncthing)

```
BibliotecaPDF/                  # carpeta que Syncthing sincroniza
├── documento.pdf
├── annotations/
│   └── <id-documento>.db       # SQLite sidecar: un fichero de anotaciones por PDF
└── library.db                  # índice de biblioteca y progreso de lectura
```

Un sidecar por PDF → los conflictos de sincronización se limitan a un solo
documento, y el versionado de Syncthing (`.stversions`) actúa como red de
seguridad.

## 4. Criterios de aceptación globales (medibles)

| Métrica | Objetivo | Dónde se mide |
|---------|----------|---------------|
| Frame time p95 en scroll | < 16,6 ms (60 fps sostenidos) | Escritorio Fase 1; tablet Fase 6 |
| Render de página a resolución nativa | < 25 ms en tablet (ajustar con datos de Fase 1) | Harness adb |
| RSS con PDF de 500 páginas | < 150 MB en tablet / < 300 MB en escritorio | `dumpsys meminfo` / `/proc` |
| 200 trazos de anotación visibles | Sin degradar frame time | Test de estrés Fase 3 |
| Sync tablet → PC | < 1 min sin intervención | Fase 4 |

## 5. Fases

### Fase 0 — Andamiaje (semana 1, ~20 h)

> Requiere haber completado el **Primer paso** (§2, verificación 2.3).

> ✅ **Completada el 2026-08-05** (salvo `git init`, pendiente de confirmación del autor).
> Verificación: `cargo build --workspace` OK · `cargo test -p pdf_core` 4/4 OK · clippy limpio · fmt limpio · `pdf_app corpus/scientific_paper.pdf` lanza ventana (proceso vivo 6 s, log sin panic/error). Detalle en `memory.md`.

- `git init` en `~/Projects/pdflector/` (ya contiene `AGENTS.md`), README, `.gitignore`, workspace con 3 crates.
- CI en GitHub Actions: `fmt`, `clippy`, `test` (gratis en repo público).
- `pdf_core`: abrir PDF con `pdfium-render`, `page_count`, `render_page()` → bitmap RGBA.
- `pdf_app`: egui/eframe, diálogo de apertura, mostrar página 1.
- Tests: apertura, `page_count` correcto, dimensiones del bitmap.

**Entregable**: `cargo run -p pdf_app` abre un PDF y muestra la página 1.

### Fase 0.5 — Benchmark de motores (semana 2, ~20 h)

> ✅ **Completada el 2026-08-05** con ADR-001.
> Resultados: **MuPDF elegido** (benchmark en `docs/benchmark-results.md`:
> render 2,7-4× más rápido — dense 1x 3,53 vs 9,69 ms; large 2x 10,19 vs
> 35,10 ms — y RSS pico -21%, 25 572 vs 32 520 KB). Repo licenciado
> **AGPL-3.0** (LICENSE). Backend **PDFium eliminado** por completo
> (pdfium.rs, dep `pdfium-render`, feature `pdfium`, selector CLI);
> MuPDF es motor único y default. Cross-compile Android aarch64 validado
> (17 s + env var `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android`).
> Verificación: `cargo build --workspace` OK · `cargo test -p pdf_core` 5/5 OK ·
> clippy limpio · fmt limpio · cross-compile aarch64 OK. Detalle en
> `docs/adr/ADR-001-motor-pdf.md` y `memory.md`.

- `trait RenderEngine` + backend MuPDF tras feature flag (crate `mupdf`; si está abandonado, bindgen propio — esfuerzo acotado, forma parte del aprendizaje).
- Benchmarks con **criterion** sobre el corpus: apertura, render/página a 1x y 2x, RSS pico, tamaño de binario.
- **Spike crítico de build Android** de ambos backends: pdfium necesita `libpdfium.so` precompilada (bblanchon/pdfium-binaries); MuPDF compila estático. La facilidad de este build es criterio de decisión tanto como la velocidad.
- **ADR-001** documentando la elección y la licencia resultante (MuPDF → AGPL; PDFium → MIT/Apache-2.0).

**Criterio de salida**: motor elegido con datos + compilación Android validada. Se elimina el backend perdedor. ✅

### Fase 1 — Lectura fluida (semanas 3–5, ~60 h)

- Scroll continuo **virtualizado**: solo páginas visibles + N colindantes.
- Caché LRU limitado por bytes; render a resolución de pantalla; nada de texturas de todas las páginas.
- Prefetch en hilos de fondo con cola prioritaria (visibles primero).
- Zoom: escalado rápido del bitmap existente (respuesta inmediata) + re-render nítido asíncrono; invalidación selectiva de caché.
- Modo paginado (salto página a página) además de continuo.
- **Harness Android mínimo** (`android-activity` + surface): renderiza páginas en bucle y vuelca timings y memoria. Despliegue por `adb` en la tablet real.
- Overlay de debug opcional: frame time, RSS, estado de caché.

**Criterio de aceptación**: scroll a 60 fps sostenidos en escritorio con PDF de 500 páginas; en tablet, render/página < 25 ms y RSS < 150 MB (umbral revisable con la primera medición real).

> **Progreso (2026-08-05)**: B1 (scroll virtualizado + caché LRU) ✅ — ver `docs/benchmark-results.md` (reducción 5× RAM: 105 MB → 20,6 MB en escritorio, cumple objetivo < 150 MB con margen) y `memory.md` Ola 5. Pendientes de Fase 1: B2 (prefetch en hilos de fondo), zoom, modo paginado, harness android-activity y overlay debug.

### Fase 2 — Modo oscuro (semana 6, ~20 h)

- Tema oscuro de UI (visuals de egui).
- **Inversión de páginas** en el blit (shader/filtro de color en GPU, sin re-render del motor); preferencia independiente de la UI (combinables).
- Persistencia de la preferencia; tests de la transformación de color.

**Criterio**: conmutar al instante sin recargar el documento; caché coherente (bitmaps invertidos no se mezclan con los normales).

### Fase 3 — Anotaciones y exportación (semanas 7–10, ~80 h)

- Modelo vectorial en coordenadas de página: `Stroke` (polilínea + grosor/color fijo), `Highlight` (rectángulos por línea de texto), `TextNote` (ancla + texto, para márgenes).
- Extracción de texto **perezosa** para subrayado preciso por selección.
- Dibujo como **capa vectorial** sobre el bitmap (painter de egui); captura de puntos con suavizado/interpolación.
- **Test de estrés**: 200+ trazos visibles sin bajar de 60 fps; si falla, tessellation cacheada por página.
- Persistencia en SQLite (`rusqlite`) con diseño **sidecar por PDF** (un fichero de anotaciones por documento) pensando ya en Syncthing.
- Exportación:
  - **Markdown**: citas con nº de página, notas, estructura por documento (verificación en Obsidian).
  - **PDF con anotaciones incrustadas** (anotaciones estándar: highlight, ink, text) legible en cualquier lector.
- Tests: modelo, serialización, round-trip exportar→importar.

**Criterio**: mapa mental en un margen nítido a zoom 3x y scroll sin degradación; el Markdown se abre bien en Obsidian y el PDF anotado en un lector externo.

### Fase 4 — Sincronización (semanas 11–12, ~40 h)

- Estructura sincronizable: `PDFs/` + `annotations/<id-pdf>.db` (sidecar) + `library.db`.
- Syncthing en tablet, PC y móvil compartiendo la carpeta. Gratis, sin servidor, sin código de red en la app.
- Detección de cambios en disco (crate `notify`) para recargar anotaciones actualizadas en caliente.
- Política de conflictos: escritura en un dispositivo a la vez (uso personal), last-writer-wins + versionado de Syncthing (`.stversions`) como red de seguridad.

**Criterio**: anotar en la tablet → visible en el PC en < 1 min sin tocar nada; conflicto simulado no corrompe datos.

### Fase 5 — Consulta a IA con Ollama (semanas 13–14, ~40 h)

> **Supuesto**: Ollama corre en el PC (la tablet de 200 € no tiene RAM para modelos). La app consulta por HTTP en red local; en escritorio, localhost. Confirmar al llegar a esta fase.

- Extracción de texto por chunks con límites de contexto (páginas relevantes).
- Panel de chat en la UI con streaming de respuesta.

**Criterio**: pregunta sobre un paper responde con contenido real del documento.

### Fase 6 — Aterrizaje Android (semanas 15–17, ~60 h)

- **Spike de UI final (1–2 días)**: Slint vs Tauri v2. Al no necesitarse presión de lápiz, **Slint vuelve a ser candidato fuerte** (un solo stack, Skia, bajo consumo). Criterios: fluidez de input táctil/lápiz, RAM, integración con `pdf_core`.
- Port completo a la UI elegida reutilizando `pdf_core` intacto.
- Gestos táctiles de tablet: pellizco para zoom, swipe, zonas de tap.
- Medición final en la Lenovo Idea Tab: RSS < 150 MB, 60 fps, consumo de batería razonable.

**Criterio**: una semana de uso real diario en la tablet sin cuelgues ni tirones.

## 6. Calendario resumen

| Semanas | Fase | Hito |
|---------|------|------|
| 0 | **Primer paso (§2)** | Entorno, skills y corpus listos |
| 1 | 0 | App abre PDF, página 1 |
| 2 | 0.5 | **Motor decidido con benchmark** + build Android |
| 3–5 | 1 | Scroll fluido 60 fps; métricas en tablet real vía adb |
| 6 | 2 | Modo oscuro UI + páginas |
| 7–10 | 3 | Anotaciones + exportar MD/PDF |
| 11–12 | 4 | Sync automática tablet ↔ PC ↔ móvil |
| 13–14 | 5 | Chat IA con Ollama |
| 15–17 | 6 | App Android final en uso diario |

## 7. Riesgos y mitigaciones

| Riesgo | Prob. | Mitigación |
|--------|-------|------------|
| `pdfium-render` difícil de compilar para Android | Media | Spike en Fase 0.5; binarios precompilados de pdfium-binaries |
| Crate `mupdf` abandonado | Media | Bindgen propio (acotado) o descartar MuPDF — el benchmark decide |
| egui en Android experimental | Alta | egui es **solo** prototipo de escritorio; UI final se decide en spike de Fase 6; `pdf_core` jamás depende de egui |
| Conflictos de Syncthing sobre SQLite binario | Baja | Sidecar por PDF + un dispositivo escribiendo a la vez + `.stversions` |
| Ollama inviable en la tablet | Alta (seguro) | Ollama en el PC, app consulta por red local |
| Anotaciones degradan FPS con muchos trazos | Media | Capa vectorial + tessellation cacheada por página; test de estrés en Fase 3 |

## 8. Decisiones diferidas (con su fase de resolución)

- UI final Android (Slint vs Tauri) → spike en Fase 6.
- Ollama en PC vs en tablet → confirmar al inicio de Fase 5.
- Gestión de biblioteca (colección de PDFs, metadatos) → surgirá naturalmente en Fase 4.

## 9. Herramientas de medición

- **criterion**: benchmarks automatizados de render y apertura.
- **Overlay de debug** en la app: frame time p95, RSS, estado de caché.
- **`/proc/self/status`**: RSS en escritorio.
- **`adb shell dumpsys meminfo`**: memoria en la tablet.
- **`adb install` / `adb logcat`**: despliegue y logs del harness Android.
