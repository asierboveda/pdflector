# PDFLector

Lector de PDFs rápido y ligero para tablet Android con lápiz. Gratis, sin anuncios, sin pagos. Proyecto personal de aprendizaje.

## Prioridades (lo que más valora el autor)

1. **Rapidez y fluidez total** — que no se trabe, que el scrolling sea de 120fps.
2. **Consumo mínimo de RAM** — optimizado, sin desperdicio.
3. Gratis y sin anuncios.
4. Aprendizaje: primer proyecto real en Rust.

## Contexto

- **Tablet objetivo**: Lenovo Idea Tab (Android, ~200€), con lápiz activo.
- Plataforma final: **Android**. Desarrollo inicial en escritorio Linux (Omarchy).
- Stack ya instalado: Rust 1.97.1, cargo, Python 3.14.6, uv.

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

### Motor de renderizado PDF (por decidir)

| Motor | Licencia | Velocidad/RAM | Pros | Contras |
|-------|----------|---------------|------|---------|
| **PDFium** | Apache-2.0 | Media-alta | El de Chrome, muy probado, crate `pdfium-render` fácil | Un poco más pesado que MuPDF |
| **MuPDF** | AGPL | La más ligera y rápida en bajo rendimiento | Mínimo consumo RAM/CPU, ideal para tablet barata | AGPL (restrictivo si se distribuye modificado; OK para uso personal) |
| **poppler** | LGPL | Media | Muy usado en Linux | Más pesado, enlazado C/C++ más incómodo en Rust |

> **Nota para tus prioridades**: MuPDF es el más ligero en RAM y CPU (perfecto para tablet de 200€), PDFium el más equilibrado en licencia. Pendiente de validar con un benchmark real en la Fase 1.

### UI (por decidir)

| Opción | Velocidad/RAM | Pros | Contras |
|--------|---------------|------|---------|
| **egui/eframe** (prototipo) | Muy buena, pocos MB | Iteración rapidísima, 100% Rust, ideal para aprender | Android experimental, lápiz sin resolver → solo para fase escritorio |
| **Slint** | Muy buena, Skia | Un solo stack desktop+Android, declarativo | Lápiz: presión no expuesta; ecosistema pequeño |
| **Qt Quick (C++)** | Buena | Más maduro para táctil/lápiz | Curva dura, Qt pesado, setup Android laborioso |
| **Tauri v2** (Rust+web) | Variable | Lápiz nativo del navegador (presión/inclinación), 1 código | WebView consume más RAM y es menos predecible |

### Almacenamiento
- **SQLite** (`rusqlite`): anotaciones, progreso, biblioteca. Ligero, un solo archivo.

## Arquitectura

```
pdf_core/   # Biblioteca Rust: abrir PDF, renderizar páginas, anotaciones, caché. Sin UI.
pdf_app/    # UI (egui ahora; Tauri/Slint en el futuro). Reutiliza pdf_core.
```

Separar núcleo y UI = poder cambiar de framework sin reescribir la lógica.

## Optimización de RAM y velocidad (plan concreto)

- Renderizar páginas a la resolución de pantalla, **no** a resolución máxima.
- **Caché LRU** de páginas: solo las visibles + colindantes; expulsar al hacer scroll.
- No conservar todas las páginas como texturas en memoria.
- Extracción de texto perezosa (solo cuando se necesita).
- Medir con `top`/`/proc` el RSS y con herramientas de profiling; umbral objetivo: < 150 MB en tablet.

## Hoja de ruta

- **Fase 0**: andamiaje cargo (`pdf_core` + `pdf_app`), abrir PDF y mostrar página 1.
- **Fase 1**: paginado, zoom, scrolling fluido con caché — validar aquí el criterio de "fluido".
- **Fase 2**: modo oscuro.
- **Fase 3**: anotaciones + exportar (lápiz).
- **Fase 4**: sincronización (Syncthing, gratis).
- **Fase 5**: consulta a IA (Ollama, local).
- **Fase 6**: personalización y aterrizaje en Android (elegir UI final; spike de 1-2 días con Tauri/Slint).

## Licencias (compatibles con "gratis y sin anuncios")

PDFium (Apache-2.0) · egui/Slint (MIT/LGPL/Royalty-free) · SQLite (dominio público) · Ollama (MIT) · Tauri (Apache/MIT). Cuidado con MuPDF (AGPL).

## Decisiones pendientes

1. Motor PDF: PDFium vs MuPDF (validar con benchmark).
2. UI final para Android: Tauri vs Slint vs Qt (según lápiz y RAM).
3. Nivel de presión del lápiz: imprescindible o aceptable sin él.
4. Formato de exportación de notas (Markdown/JSON/incrustado en PDF).
