# PDFLector

Lector de PDFs rápido y ligero para tablet Android con lápiz. Gratis, sin anuncios, sin pagos. Proyecto personal de aprendizaje.

## Prioridades (lo que más valora el autor)

1. **Rapidez y fluidez total** — que no se trabe en ningún gesto. Objetivo declarado: mínimo sostenido de 60 fps (tiempo de frame p95 < 16,6 ms) y meta de 120 fps para aprovechar la pantalla de 120 Hz de la TCL (tiempo de frame p95 < 8,33 ms).
2. **Consumo mínimo de RAM** — optimizado, sin desperdicio. Métrica de producto: PSS vía `dumpsys meminfo` (objetivo <150 MB en tablet).
3. Gratis y sin anuncios.
4. Aprendizaje: primer proyecto real en Rust.

## Contexto

- **Tablet objetivo**: TCL NXTPaper 11 Plus (modelo 9469X, Android 15, MediaTek
  MT8781 8× Cortex-A55, pantalla 1440×2200 @ 320 dpi, 120 Hz, con lápiz). Hardware real
  desde la Fase 1 (spike 2026-08-12, ver `docs/benchmark-results.md`).
- Plataforma final: **Android nativo (`crates/pdf_android`)** — decisión vigente ADR-005 (sustituye a ADR-004/Slint).
  El escritorio `egui` (`crates/pdf_app`) es únicamente un banco de pruebas del core para iteración rápida en Linux; NO es producto ni plataforma soportada.
- Stack ya instalado: Rust 1.97.1 (rustup), cargo, Python 3.14.6, uv; toolchain
  Android: adb, JDK 17, Android SDK en `~/Android/Sdk` (NDK r28, platform 35).

## Funciones

### Alcance de producto (la app Android nativa `crates/pdf_android`)

- **Biblioteca curada local**: añadir PDFs mediante selector de carpetas (SAF / MediaStore); persistencia en SQLite; nunca borra automáticamente (solo el usuario borra).
- **Lectura fluida**: paginado, pan continuo y zoom con swap EGL/GLES y caché LRU de texturas (evita re-renderizar la página en cada frame de desplazamiento).
- **Zoom continuo**: pinch-to-zoom con preservación del foco y límites de escala.
- **Lápiz y anotaciones vectoriales**:
  - Subrayador con detección de texto y alineación en orden de lectura.
  - Tinta libre con baja latencia y predicción de trazo (Kalman y masa-resorte).
  - Goma de borrar vectorial que elimina trazos completos.
  - Persistencia asíncrona a SQLite en hilo de fondo.
- **Discover / arXiv**: búsqueda integrada de papers en arXiv, visualización de metadatos y resúmenes, descarga en segundo plano y handoff directo al visor.
- **Asistente de IA integrado**: panel interactivo en la app con backend híbrido:
  - Groq (`llama-3.3-70b-versatile`) para explicación contextual del texto seleccionado.
  - Google Gemini (`gemini-flash-latest`) para análisis visual multimodal de ecuaciones y figuras recortadas.
  - Utiliza claves de API del usuario. La APK compila con claves placeholder y no se incluyen secretos en Git.
- **Modo oscuro y temas**: 4 temas disponibles (DefaultLight, SepiaLight, DefaultDark, SepiaDark).
- **Búsqueda en el documento**: búsqueda de texto con teclado IME nativo de Android.

### Banco de pruebas de escritorio (`crates/pdf_app` — fuera de producto)

- Prototipo inicial en `egui` para desarrollo y validación del core en Linux.
- Soporta exportación de notas (Markdown y PDF con anotaciones vectoriales incrustadas).
- No es producto ni plataforma final; no tiene hoja de ruta independiente ni soporte de distribución.

### Explícitamente fuera de alcance / congelado

- **Sincronización entre dispositivos**: congelada.

## Stack — decisiones con pros y contras

### Lenguaje: Rust (decidido)

- **Pros**: rendimiento nativo, bajo consumo de RAM, sin GC (control total de memoria), gestión de proyectos con cargo (muy fácil), ya instalado.
- **Contras**: curva inicial; UI de escritorio menos "wysiwyg" que web.

### Motor de renderizado PDF (decidido: **MuPDF**, AGPL-3.0-or-later — ADR-001)

> **Decisión (2026-08-05, ADR-001)**: MuPDF es el motor único y por defecto, y el
> repositorio está licenciado **AGPL-3.0-or-later** (LICENSE). El benchmark de la Fase 0.5
> confirmó las ventajas esperadas: render 2,7–4× más rápido y RSS pico -21% frente
> a PDFium (detalle en `docs/benchmark-results.md`); el backend PDFium se eliminó.

La tabla siguiente es la comparativa que motivó la decisión (contexto histórico):

| Motor | Licencia | Velocidad/RAM | Pros | Contras |
|-------|----------|---------------|------|---------|
| **MuPDF** (elegido) | AGPL-3.0-or-later | La más ligera y rápida en bajo rendimiento | Mínimo consumo RAM/CPU, ideal para tablet barata | AGPL (copyleft; adecuado para proyecto público y gratuito) |
| PDFium | Apache-2.0 | Media-alta | El de Chrome, muy probado, crate `pdfium-render` fácil | Más pesado que MuPDF; descartado en ADR-001 |
| poppler | LGPL | Media | Muy usado en Linux | Más pesado, enlazado C/C++ más incómodo en Rust |

### UI (decidida: **`pdf_android` nativa** — ADR-005)

> **Decisión vigente (2026-08-23, ADR-005)**: la plataforma final es `pdf_android`
> (NativeActivity + JNI + render propio a `ANativeWindow` con EGL/GLES). Sustituye la
> vía anterior de Slint (ADR-004, Superseded). Opciones descartadas: Slint, Tauri y Qt.

Contexto histórico (opciones evaluadas en su día, ya cerradas):

| Opción | Velocidad/RAM | Pros | Contras |
|--------|---------------|------|---------|
| **egui/eframe** (banco de pruebas) | Muy buena, pocos MB | Iteración rapidísima, 100% Rust, ideal para validar el core | Android experimental, lápiz sin resolver → solo para prototipo en escritorio |
| **Slint** (descartado, ADR-005) | Muy buena, Skia | Un solo stack desktop+Android, declarativo | Lápiz real sin validar; riesgo no-repaint en Android; reescritura sin beneficio PSS |
| **Qt Quick (C++)** (descartado) | Buena | Más maduro para táctil/lápiz | Curva dura, Qt pesado, setup Android laborioso |
| **Tauri v2** (descartado) | Variable | Lápiz nativo del navegador (presión/inclinación), 1 código | WebView consume más RAM y es menos predecible |

### Almacenamiento

- **SQLite** (`rusqlite`): anotaciones, progreso, biblioteca. Ligero, un solo archivo.

## Arquitectura

```
crates/pdf_core/     # Biblioteca Rust: motor MuPDF, renderizado, anotaciones, caché, arXiv, clientes IA. Sin UI.
crates/pdf_android/  # Plataforma y producto final Android nativo (ADR-005, NativeActivity + EGL/GLES).
crates/pdf_spike/    # Spike de latencia y presentación de trazo stylus en Android.
crates/pdf_app/      # Banco de pruebas egui: prototipo desktop para iteración del core, NO producto.
crates/pdf_bench/    # Benchmarks (criterion) y barridos de rendimiento en escritorio y tablet.
```

Separar núcleo y UI = poder mantener y optimizar el motor y la lógica de negocio independientemente del frontend.

## Optimización de RAM y velocidad (plan concreto)

- Renderizar páginas a la resolución de pantalla, **no** a resolución máxima.
- **Caché LRU** de páginas: solo las visibles + colindantes; expulsar al hacer scroll.
- No conservar todas las páginas como texturas en memoria.
- Extracción de texto perezosa (solo cuando se necesita).
- Métrica de producto Android: **PSS** vía `dumpsys meminfo` (objetivo <150 MB en tablet).
- Procedimiento repetible: `.opencode/skills/pdflector-rendimiento/SKILL.md`.
  Ninguna afirmación de rendimiento sin fecha + flujo medido + hardware + métrica.

## Hoja de ruta

El roadmap vigente es **`docs/plan/NEXT-PLAN.md` (fases A–F)**:
- **Fase A**: Latencia de interacción y render (presupuesto de frame y profiling).
- **Fase B**: Subrayador con alineación de texto y orden de lectura.
- **Fase C**: Pintado y lápiz con predicción de baja latencia.
- **Fase D**: Asistente de IA y contexto (panel híbrido Groq / Gemini).
- **Fase E**: Biblioteca curada y gestión de documentos.
- **Fase F**: Discover / arXiv (búsqueda, descarga y apertura fluida de papers).
- **Deuda transversal**: `docs/plan/DEUDA.md` (asuntos técnicos pendientes y optimizaciones abiertas).

## Licencias

El proyecto está licenciado **AGPL-3.0-or-later** (ver `LICENSE` y `NOTICE`), ligado a la elección de
MuPDF como motor (ADR-001). Compatible con "gratis y sin anuncios" y con la
publicación del código en GitHub.

Dependencias principales y licencias (verificadas con `cargo metadata`, ver `NOTICE`):
- **MuPDF** (`mupdf` / `mupdf-sys`): AGPL-3.0-or-later (Artifex Software).
- **Android runtime**: `android-activity`, `ndk`, `jni`, `android_logger`, `log` (MIT o Apache-2.0).
- **Almacenamiento y utilidades**: `rusqlite` (MIT), `lru` (MIT), `serde` / `serde_json` (MIT o Apache-2.0), `notify` (CC0-1.0).
- **Banco de pruebas desktop**: `eframe` / `egui`, `rfd` (MIT o Apache-2.0).
- **Harness de benchmarks**: `criterion` (Apache-2.0 o MIT).

Todas las dependencias de terceros emplean licencias permisivas totalmente compatibles con AGPL-3.0-or-later.

## Decisiones pendientes

1. **Migración de claves embebidas**: actualmente `crates/pdf_android` utiliza `include_str!` hacia ficheros locales (`groq_key.txt` y `google_key.txt`). La compilación funciona en limpio con claves placeholder (ver `CONTRIBUTING.md`), pero la solución definitiva para distribución debe mover la configuración de claves a un diálogo de ajustes en la app o almacenamiento seguro en tiempo de ejecución.
2. **Pantalla de créditos/licencias en la app**: incluir vista "Acerca de" que exponga el texto AGPL y `NOTICE` en la interfaz de Android para cumplir los requisitos de distribución de binarios.

**Resueltas** (ADR-001 / ADR-005): motor PDF = MuPDF (AGPL-3.0-or-later); presión del
lápiz = no necesaria; exportación = Markdown + PDF con anotaciones incrustadas (en prototipo desktop);
plataforma final = `pdf_android` nativa (ADR-005 sustituye a ADR-004).
