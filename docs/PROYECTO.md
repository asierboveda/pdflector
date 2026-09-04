# PDFLector

Lector de PDFs rápido y ligero para tablet Android con lápiz. Gratis, sin anuncios, sin pagos. Proyecto personal de aprendizaje.

## Prioridades (lo que más valora el autor)

1. **Rapidez y fluidez total** — que no se trabe, que el scrolling sea de 120fps.
2. **Consumo mínimo de RAM** — optimizado, sin desperdicio. Métrica de producto: PSS vía `dumpsys meminfo`.
3. Gratis y sin anuncios.
4. Aprendizaje: primer proyecto real en Rust.

## Contexto

- **Tablet objetivo**: TCL NXTPaper 11 Plus (modelo 9469X, Android 15, MediaTek
  MT8781 8× Cortex-A55, pantalla 1440×2200 @ 320 dpi, con lápiz). Hardware real
  desde la Fase 1 (spike 2026-08-12, ver `docs/benchmark-results.md`).
- Plataforma final: **Android nativo (`pdf_android`)** — decisión vigente ADR-005 (sustituye a ADR-004/Slint).
  Desarrollo inicial en escritorio Linux (Omarchy).
- Stack ya instalado: Rust 1.97.1 (rustup), cargo, Python 3.14.6, uv; toolchain
  Android: adb, JDK 17, Android SDK en `~/Android/Sdk` (NDK r28, platform 35).

## Funciones

**Primera versión útil (v1, decidida):** APK para la TCL con:

- Biblioteca local (añadir PDFs; nunca borrado automático: solo el usuario borra)
- Lectura fluida (paginado, scroll con caché)
- Zoom
- Lápiz y subrayador persistentes (anotaciones vectoriales guardadas)

**Después de v1 (explícitamente fuera):**

- Consulta a IA sobre el PDF (Ollama/Groq; requiere configuración de claves, sin claves en Git ni en APK distribuible)
- Sincronización entre dispositivos (congelada)

**Ya existentes / en curso (no bloquean v1):**

- Anotaciones (subrayar, dibujar, notas) con el lápiz
- Exportar notas (Markdown + PDF con anotaciones)
- Modo oscuro
- Personalización (temas, ajustes) y gestos táctiles específicos de tablet

## Stack — decisiones con pros y contras

### Lenguaje: Rust (decidido)

- **Pros**: rendimiento nativo, bajo consumo de RAM, sin GC (control total de memoria), gestión de proyectos con cargo (muy fácil), ya instalado.
- **Contras**: curva inicial; UI de escritorio menos "wysiwyg" que web.

### Motor de renderizado PDF (decidido: **MuPDF**, AGPL-3.0 — ADR-001)

> **Decisión (2026-08-05, ADR-001)**: MuPDF es el motor único y por defecto, y el
> repositorio está licenciado **AGPL-3.0** (LICENSE). El benchmark de la Fase 0.5
> confirmó las ventajas esperadas: render 2,7-4× más rápido y RSS pico -21% frente
> a PDFium (detalle en `docs/benchmark-results.md`); el backend PDFium se eliminó.

La tabla siguiente es la comparativa que motivó la decisión (contexto histórico):

| Motor | Licencia | Velocidad/RAM | Pros | Contras |
|-------|----------|---------------|------|---------|
| **MuPDF** (elegido) | AGPL-3.0 | La más ligera y rápida en bajo rendimiento | Mínimo consumo RAM/CPU, ideal para tablet barata | AGPL (copyleft; OK para proyecto público y gratuito) |
| PDFium | Apache-2.0 | Media-alta | El de Chrome, muy probado, crate `pdfium-render` fácil | Más pesado que MuPDF; descartado en ADR-001 |
| poppler | LGPL | Media | Muy usado en Linux | Más pesado, enlazado C/C++ más incómodo en Rust |

### UI (decidida: **`pdf_android` nativa** — ADR-005)

> **Decisión vigente (2026-08-23, ADR-005)**: la plataforma final es `pdf_android`
> (NativeActivity + JNI + render propio a `ANativeWindow`). ADR-004 (Slint) queda
> **Superseded**. Slint/Tauri/Qt **no** son alternativas abiertas.

Contexto histórico (opciones evaluadas en su día, ya cerradas):

| Opción | Velocidad/RAM | Pros | Contras |
|--------|---------------|------|---------|
| **egui/eframe** (prototipo) | Muy buena, pocos MB | Iteración rapidísima, 100% Rust, ideal para aprender | Android experimental, lápiz sin resolver → solo para fase escritorio |
| **Slint** (descartado, ADR-005) | Muy buena, Skia | Un solo stack desktop+Android, declarativo | Lápiz real sin validar; riesgo no-repaint en Android; reescribir ~12k líneas sin beneficio PSS |
| **Qt Quick (C++)** (descartado) | Buena | Más maduro para táctil/lápiz | Curva dura, Qt pesado, setup Android laborioso |
| **Tauri v2** (descartado) | Variable | Lápiz nativo del navegador (presión/inclinación), 1 código | WebView consume más RAM y es menos predecible |

### Almacenamiento

- **SQLite** (`rusqlite`): anotaciones, progreso, biblioteca. Ligero, un solo archivo.

## Arquitectura

```
pdf_core/     # Biblioteca Rust: abrir PDF, renderizar páginas, anotaciones, caché. Sin UI.
pdf_android/  # Plataforma final Android nativa (ADR-005). Reutiliza pdf_core.
pdf_app/      # UI egui: prototipo desktop, no plataforma final.
pdf_bench/    # Benchmarks (criterion) y barridos de rendimiento; escritorio y Android.
```

Separar núcleo y UI = poder cambiar de framework sin reescribir la lógica.

## Optimización de RAM y velocidad (plan concreto)

- Renderizar páginas a la resolución de pantalla, **no** a resolución máxima.
- **Caché LRU** de páginas: solo las visibles + colindantes; expulsar al hacer scroll.
- No conservar todas las páginas como texturas en memoria.
- Extracción de texto perezosa (solo cuando se necesita).
- Métrica de producto Android: **PSS** vía `dumpsys meminfo` (objetivo <150 MB en tablet).
  RSS/VmHWM (`top`/`/proc`, `PEAK_RSS_KB` del sweep) queda como diagnóstico de host.
- Procedimiento repetible: `.opencode/skills/pdflector-rendimiento/SKILL.md`.
  Ninguna afirmación de rendimiento sin fecha + flujo medido + hardware + métrica.

## Hoja de ruta

- Roadmap activo: **`docs/plan/NEXT-PLAN.md` (fases A–E)**. Primera versión: A → B → C → E
  (instrumentación → subrayador → lápiz → biblioteca). IA (D) y sincronización quedan después de v1.
- Histórico: Fases 0–6 (andamiaje, motor MuPDF, lectura, oscuro, anotaciones, sync, IA, Android).
  Detalle en `docs/PLAN.md` (índice histórico, no editar).

## Licencias

El proyecto está licenciado **AGPL-3.0** (LICENSE), ligado a la elección de
MuPDF como motor (ADR-001). Compatible con "gratis y sin anuncios" y con la
publicación pública del código en GitHub. Dependencias: egui/Slint
(MIT/LGPL/Royalty-free) · SQLite (dominio público) · Ollama (MIT) · Tauri
(Apache/MIT) · lru (MIT).

## Decisiones pendientes

1. Ubicación de Ollama / claves IA: PC por red local vs otra opción (después de v1; ninguna clave en Git ni en APK distribuible; la APK compila e instala sin claves).
2. Migración de claves embebidas (`include_str!` en `pdf_android`): tarea separada pendiente, sin cambio de código en esta reestructuración.
3. Eliminar cualquier borrado automático de biblioteca (p. ej. límite 50 PDFs): tarea futura específica en Fase E; la biblioteca nunca borra sin acción del usuario.

**Resueltas** (ADR-001 / ADR-005): motor PDF = MuPDF (AGPL-3.0); presión del
lápiz = no necesaria; exportación = Markdown + PDF con anotaciones incrustadas;
plataforma final = `pdf_android` nativa (ADR-005 sustituye ADR-004).
