# PDFLector

Lector de PDFs rápido y ligero para tablet Android con lápiz. Gratis, sin anuncios, sin pagos. Proyecto personal de aprendizaje.

## Prioridades (lo que más valora el autor)

1. **Rapidez y fluidez total** — que no se trabe, que el scrolling sea de 120fps.
2. **Consumo mínimo de RAM** — optimizado, sin desperdicio.
3. Gratis y sin anuncios.
4. Aprendizaje: primer proyecto real en Rust.

## Contexto

- **Tablet objetivo**: TCL NXTPaper 11 Plus (modelo 9469X, Android 15, MediaTek
  MT8781 8× Cortex-A55, pantalla 1440×2200 @ 320 dpi, con lápiz). Hardware real
  desde la Fase 1 (spike 2026-08-12, ver `docs/benchmark-results.md`).
- Plataforma final: **Android**. Desarrollo inicial en escritorio Linux (Omarchy).
- Stack ya instalado: Rust 1.97.1 (rustup), cargo, Python 3.14.6, uv; toolchain
  Android: adb, JDK 17, Android SDK en `~/Android/Sdk` (NDK r28, platform 35).

## Funciones

**Imprescindibles:**
- Lectura fluida (paginado, zoom, scroll con caché)
- Anotaciones (subrayar, dibujar, notas) con el lápiz
- Exportar notas
- Modo oscuro
- Sincronización entre dispositivos

**Futuras:**
- Personalización (temas, ajustes)
- Consulta a IA sobre el PDF (Ollama, local y gratis)
- Gestos táctiles específicos de tablet

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

### UI (por decidir)

| Opción | Velocidad/RAM | Pros | Contras |
|--------|---------------|------|---------|
| **egui/eframe** (prototipo) | Muy buena, pocos MB | Iteración rapidísima, 100% Rust, ideal para aprender | Android experimental, lápiz sin resolver → solo para fase escritorio |
| **Slint** | Muy buena, Skia | Un solo stack desktop+Android, declarativo | Lápiz: presión no expuesta; ecosistema pequeño |
| **Qt Quick (C++)** | Buena | Más maduro para táctil/lápiz | Curva dura, Qt pesado, setup Android laborioso |
| **Tauri v2** (Rust+web) | Variable | Lápiz nativo del navegador (presión/inclinación), 1 código | WebView consume más RAM y es menos predecible |

> **Estado**: la decisión se resuelve en el spike de la **Fase 6** (Slint vs
> Tauri v2; Qt queda fuera del spike, ver PLAN.md). Como el lápiz **no requiere
> presión** (PLAN.md §1, decisión 2), el contra de Slint ("presión no expuesta")
> deja de pesar y vuelve a ser candidato fuerte.

### Almacenamiento
- **SQLite** (`rusqlite`): anotaciones, progreso, biblioteca. Ligero, un solo archivo.

## Arquitectura

```
pdf_core/   # Biblioteca Rust: abrir PDF, renderizar páginas, anotaciones, caché. Sin UI.
pdf_app/    # UI (egui ahora; Tauri/Slint en el futuro). Reutiliza pdf_core.
pdf_bench/  # Benchmarks (criterion) y barridos de rendimiento; escritorio y Android.
```

Separar núcleo y UI = poder cambiar de framework sin reescribir la lógica.

## Optimización de RAM y velocidad (plan concreto)

- Renderizar páginas a la resolución de pantalla, **no** a resolución máxima.
- **Caché LRU** de páginas: solo las visibles + colindantes; expulsar al hacer scroll.
- No conservar todas las páginas como texturas en memoria.
- Extracción de texto perezosa (solo cuando se necesita).
- Medir con `top`/`/proc` el RSS y con herramientas de profiling; umbral objetivo: < 150 MB en tablet.

## Hoja de ruta

- **Fase 0** — ✅ completada (2026-08-05): andamiaje cargo (`pdf_core` + `pdf_app`), abrir PDF y mostrar página 1.
- **Fase 0.5** — ✅ completada (2026-08-05): benchmark de motores y decisión **MuPDF / AGPL-3.0** (ADR-001); backend PDFium eliminado.
- **Fase 1** — en curso: scroll virtualizado + caché LRU (B1) ✅, prefetch en hilos de fondo (B2) ✅, spike en la tablet TCL NXTPaper 11 Plus ✅ (2026-08-12). Pendientes: zoom (B3), modo paginado, harness android-activity, overlay debug.
- **Fase 2**: modo oscuro.
- **Fase 3**: anotaciones + exportar (lápiz).
- **Fase 4**: sincronización (Syncthing, gratis).
- **Fase 5**: consulta a IA (Ollama, local).
- **Fase 6**: personalización y aterrizaje en Android (elegir UI final; spike de 1-2 días con Slint/Tauri).

## Licencias

El proyecto está licenciado **AGPL-3.0** (LICENSE), ligado a la elección de
MuPDF como motor (ADR-001). Compatible con "gratis y sin anuncios" y con la
publicación pública del código en GitHub. Dependencias: egui/Slint
(MIT/LGPL/Royalty-free) · SQLite (dominio público) · Ollama (MIT) · Tauri
(Apache/MIT) · lru (MIT).

## Decisiones pendientes

1. UI final para Android: Slint vs Tauri v2 (spike en Fase 6; Qt descartado).
2. Ubicación de Ollama: PC por red local vs otra opción (al inicio de Fase 5).

**Resueltas** (PLAN.md §1 / ADR-001): motor PDF = MuPDF (AGPL-3.0); presión del
lápiz = no necesaria; exportación = Markdown + PDF con anotaciones incrustadas;
sincronización = Syncthing.
